import { useCallback, useEffect, useMemo, useState } from "react";
import { deleteInstruction, instructions, normalizeCommandError, saveInstruction, translationFix, translationStatus, translationStop } from "../ipc";
import type { CommandError, InstructionDto, ProjectSheetDto, SheetProgressDto, TranslationStatus } from "../types";
import { useI18n } from "../ui/i18n";
import { usePreferences } from "../ui/preferences";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ConfirmDialog } from "./ConfirmDialog";
import { ErrorBanner } from "./ErrorBanner";
import { SheetChooser, useSheetTree } from "./SheetChooser";
import { ContinueButton, ReportButton, RunProgress, describeStop } from "./TranslateDialog";

/** How often the run is read while one goes. */
const POLL_MS = 1000;

/** The instruction in the editor: as saved, `null` for a new one, and as edited. */
type Draft = { savedName: string | null; savedText: string | null; name: string; text: string };

const emptyDraft: Draft = { savedName: null, savedText: null, name: "", text: "" };

type EditingViewProps = {
  active: boolean;
  sheets: readonly ProjectSheetDto[];
  progress: ReadonlyMap<string, SheetProgressDto>;
  onDirtyChange: (dirty: boolean) => void;
  /** A run wrote translations. */
  onFilesChanged: () => void;
  onOpenSettings: () => void;
  onOpenReport: (reportId: string, label: string) => void;
};

/**
 * Editing: the project's instructions for AI runs, each a Markdown file of
 * `aeria-knowledge/instructions/`, and a run of one over the translations of
 * chosen sheets.
 */
export function EditingView({ active, sheets, progress, onDirtyChange, onFilesChanged, onOpenSettings, onOpenReport }: EditingViewProps) {
  const { t, locale, formatNumber } = useI18n();
  const { preferences } = usePreferences();
  const [list, setList] = useState<InstructionDto[] | null>(null);
  const [problems, setProblems] = useState<string[]>([]);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ title: string; error: CommandError } | null>(null);
  const [confirm, setConfirm] = useState<{ message: string; label: string; run: () => void } | null>(null);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [status, setStatus] = useState<TranslationStatus | null>(null);
  const { allNames, scopeOf } = useSheetTree(sheets);

  const dirty = draft.name !== (draft.savedName ?? "") || draft.text !== (draft.savedText ?? "");
  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange(false), [onDirtyChange]);

  const show = useCallback((found: { instructions: InstructionDto[]; problems: string[] }) => {
    setList(found.instructions);
    setProblems(found.problems);
  }, []);
  useEffect(() => {
    if (!active) return;
    void instructions()
      .then(show)
      .catch((reason: unknown) => setError({ title: t("editing.loadError"), error: normalizeCommandError(reason) }));
  }, [active, show, t]);

  // The run is read while the tab is shown, and every second while one goes.
  const readStatus = useCallback(() => void translationStatus().then(setStatus).catch(() => undefined), []);
  const running = status?.running ?? false;
  useEffect(() => {
    if (!active) return;
    readStatus();
    if (!running) return;
    const timer = window.setInterval(readStatus, POLL_MS);
    return () => window.clearInterval(timer);
  }, [active, readStatus, running]);
  const [wasRunning, setWasRunning] = useState(false);
  useEffect(() => {
    if (wasRunning && !running) onFilesChanged();
    setWasRunning(running);
  }, [onFilesChanged, running, wasRunning]);

  /** Asks before unsaved edits are lost, then runs `next`. */
  const leaving = (next: () => void) => {
    if (!dirty) { next(); return; }
    setConfirm({ message: t("editing.discard"), label: t("confirm.discard"), run: next });
  };
  const open = (instruction: InstructionDto) => leaving(() => setDraft({ savedName: instruction.name, savedText: instruction.text, name: instruction.name, text: instruction.text }));
  const create = () => leaving(() => setDraft(emptyDraft));

  const save = () => {
    setBusy(true);
    setError(null);
    const name = draft.name.trim();
    void saveInstruction(name, draft.savedName, draft.savedText, draft.text)
      .then((found) => {
        show(found);
        setDraft({ savedName: name, savedText: draft.text, name, text: draft.text });
      })
      .catch((reason: unknown) => setError({ title: t("editing.saveError"), error: normalizeCommandError(reason) }))
      .finally(() => setBusy(false));
  };
  const remove = () => {
    const { savedName, savedText } = draft;
    if (savedName === null || savedText === null) return;
    setConfirm({
      message: t("editing.deleteConfirm", { name: savedName }),
      label: t("editing.delete"),
      run: () => {
        setBusy(true);
        setError(null);
        void deleteInstruction(savedName, savedText)
          .then((found) => { show(found); setDraft(emptyDraft); })
          .catch((reason: unknown) => setError({ title: t("editing.saveError"), error: normalizeCommandError(reason) }))
          .finally(() => setBusy(false));
      },
    });
  };

  const toggle = useCallback((names: readonly string[], on: boolean) => {
    setSelected((current) => {
      const next = new Set(current);
      for (const name of names) {
        if (on) next.add(name);
        else next.delete(name);
      }
      return next;
    });
  }, []);
  const translatedOf = useCallback((name: string): number | null => progress.get(name)?.translated ?? null, [progress]);
  const strings = useMemo(() => allNames.filter((name) => selected.has(name)).reduce((sum, name) => sum + (translatedOf(name) ?? 0), 0), [allNames, selected, translatedOf]);
  const countLabel = (count: number | null) => count === null
    ? <span className="translate-remaining muted">{t("common.loading")}</span>
    : <span className="translate-remaining">{formatNumber(count)}</span>;

  const noModel = preferences.translationModel === "";
  const saved = draft.savedName !== null;
  const block = noModel ? null : running ? t("editing.busy") : !saved ? t("editing.saveFirst") : dirty ? t("editing.unsaved") : selected.size === 0 ? t("editing.chooseSheets") : null;
  const start = () => {
    if (block !== null || noModel || draft.savedName === null) return;
    setError(null);
    void translationFix(scopeOf(selected), [], { issues: false, groups: [], adapt: false, request: null, instruction: draft.savedName }, preferences.translationModel, preferences.translationEffort || null)
      .then(readStatus)
      .catch((reason: unknown) => setError({ title: t("editing.runError"), error: normalizeCommandError(reason) }));
  };

  return (
    <div className="glossary-view editing-view">
      {error ? <ErrorBanner title={error.title} error={error.error} onDismiss={() => setError(null)} /> : null}
      <div className="glossary-panes editing-panes">
        <nav className="glossary-facets" aria-label={t("editing.instructions")}>
          <div className="glossary-facet-head">{t("editing.instructions")}</div>
          <div className="glossary-facet-group">
            {(list ?? []).map((instruction) => (
              <button key={instruction.name} type="button" className={`glossary-facet${draft.savedName === instruction.name ? " active" : ""}`} aria-pressed={draft.savedName === instruction.name} onClick={() => open(instruction)}>
                <UiIcon icon="wand" size="xs" />
                <span className="glossary-facet-label">{instruction.name}</span>
              </button>
            ))}
            {list === null ? <p className="muted editing-empty">{t("common.loading")}</p> : null}
            {list !== null && list.length === 0 ? <p className="muted editing-empty">{t("editing.none")}</p> : null}
          </div>
          <div className="editing-new">
            <button className="button button-secondary" type="button" disabled={busy} onClick={create}><UiIcon icon="plus" size="sm" />{t("editing.new")}</button>
          </div>
          {problems.map((problem) => <p key={problem} className="glossary-problem"><UiIcon icon="circleAlert" size="xs" />{problem}</p>)}
        </nav>

        <section className="editing-editor" aria-label={t("editing.instruction")}>
          <label className="field">
            <span className="field-label">{t("editing.name")}</span>
            <input className="input" value={draft.name} spellCheck={false} placeholder={t("editing.namePlaceholder")} disabled={busy} onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
          </label>
          <textarea className="input editing-text" value={draft.text} spellCheck placeholder={t("editing.textPlaceholder")} aria-label={t("editing.instruction")} disabled={busy} onChange={(event) => setDraft({ ...draft, text: event.target.value })} />
          <div className="editing-actions">
            {saved ? <button className="button button-ghost" type="button" disabled={busy} onClick={remove}>{t("editing.delete")}</button> : null}
            <span className="spacer" />
            <button className="button button-ghost" type="button" disabled={busy || !dirty} onClick={() => setDraft({ ...draft, name: draft.savedName ?? "", text: draft.savedText ?? "" })}>{t("editing.revert")}</button>
            <button className="button button-primary" type="button" disabled={busy || !dirty || draft.name.trim() === "" || draft.text.trim() === ""} onClick={save}>{t("editing.save")}</button>
          </div>
        </section>

        <aside className="editing-run" aria-label={t("editing.scope")}>
          <div className="glossary-facet-head">{t("editing.scope")}</div>
          <SheetChooser
            sheets={sheets}
            selected={selected}
            onToggle={toggle}
            countOf={translatedOf}
            renderCount={countLabel}
            disabled={running}
            label={t("editing.scope")}
            presets={<>
              <button className="button button-ghost translate-preset" type="button" disabled={running} onClick={() => toggle(allNames, true)}>{t("editing.all")}</button>
              <button className="button button-ghost translate-preset" type="button" disabled={running || selected.size === 0} onClick={() => setSelected(new Set())}>{t("translate.preset.none")}</button>
            </>}
          />
          <p className="translate-summary">{t("editing.selection", { sheets: formatNumber(allNames.filter((name) => selected.has(name)).length), strings: formatNumber(strings) })}</p>
          {noModel ? (
            <p className="export-warning"><UiIcon icon="circleAlert" size="xs" />{t("translate.noModel")}{" "}<button className="link-button" type="button" onClick={onOpenSettings}>{t("translate.openSettings")}</button></p>
          ) : block ? <p className="field-hint">{block}</p> : null}
          <div className="editing-actions">
            {running
              ? <button className="button button-secondary" type="button" onClick={() => void translationStop()}>{t("translate.stopRun")}</button>
              : <button className="button button-primary" type="button" disabled={block !== null || noModel} onClick={start}><UiIcon icon="sparkles" size="sm" />{t("editing.run")}</button>}
          </div>
          {status && (running || status.stop) ? (
            <div className="translate-progress" aria-live="polite">
              <RunProgress status={status} />
              {status.message ? <p className="field-hint">{status.message}</p> : null}
              {status.stop ? <p className="field-hint"><strong>{describeStop(status.stop, t, locale)}</strong></p> : null}
              {status.report ? <p className="glossary-run-actions"><ContinueButton status={status} onContinued={readStatus} /><ReportButton status={status} onOpenReport={onOpenReport} /></p> : null}
            </div>
          ) : null}
        </aside>
      </div>
      <ConfirmDialog
        open={confirm !== null}
        message={confirm?.message ?? ""}
        {...(confirm ? { confirmLabel: confirm.label } : {})}
        onKeepEditing={() => setConfirm(null)}
        onDiscard={() => { const run = confirm?.run; setConfirm(null); run?.(); }}
      />
    </div>
  );
}
