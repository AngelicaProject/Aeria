import { useCallback, useEffect, useMemo, useState } from "react";
import { deleteSkill, normalizeCommandError, runSkill, saveSkill, skillCount, skills, translationStatus, translationStop } from "../ipc";
import type { CommandError, ProjectSheetDto, ScopeCountDto, ScopeDto, SheetProgressDto, SkillDto, TranslationStatus } from "../types";
import { useI18n } from "../ui/i18n";
import { usePreferences } from "../ui/preferences";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ConfirmDialog } from "./ConfirmDialog";
import { ErrorBanner } from "./ErrorBanner";
import { SheetChooser, useSheetTree } from "./SheetChooser";
import { ContinueButton, ReportButton, RunProgress, describeStop } from "./TranslateDialog";

/** How often the run is read while one goes. */
const POLL_MS = 1000;
/** How long the scope stays unchanged before its strings are counted. */
const COUNT_DELAY_MS = 500;

/** The scope as the form edits it: lists as text, a line each. */
type ScopeForm = { paths: string; exclude: string; source: string; translation: string; other: string };

const emptyForm: ScopeForm = { paths: "", exclude: "", source: "", translation: "", other: "" };

function formOf(scope: ScopeDto): ScopeForm {
  return { paths: scope.paths.join("\n"), exclude: scope.exclude.join("\n"), source: scope.source ?? "", translation: scope.translation ?? "", other: scope.other ?? "" };
}

const lines = (text: string) => text.split("\n").map((line) => line.trim()).filter((line) => line !== "");

function scopeOfForm(form: ScopeForm): ScopeDto {
  const pattern = (text: string) => text.trim() === "" ? null : text.trim();
  return { paths: lines(form.paths), exclude: lines(form.exclude), source: pattern(form.source), translation: pattern(form.translation), other: pattern(form.other) };
}

/** The skill in the editor: its file as saved, `null` for a new one, and as edited. */
type Draft = { savedName: string | null; savedText: string | null; saved: { name: string; body: string; form: ScopeForm }; name: string; body: string; form: ScopeForm };

const emptyDraft: Draft = { savedName: null, savedText: null, saved: { name: "", body: "", form: emptyForm }, name: "", body: "", form: emptyForm };

function draftOf(skill: SkillDto): Draft {
  const form = formOf(skill.scope);
  return { savedName: skill.name, savedText: skill.text, saved: { name: skill.name, body: skill.body, form }, name: skill.name, body: skill.body, form };
}

const sameForm = (a: ScopeForm, b: ScopeForm) => a.paths === b.paths && a.exclude === b.exclude && a.source === b.source && a.translation === b.translation && a.other === b.other;

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
 * Editing: the project's skills for AI runs, each a Markdown file of
 * `aeria-knowledge/skills/` with where it applies, and a run of one over
 * the translations of its scope.
 */
export function EditingView({ active, sheets, progress, onDirtyChange, onFilesChanged, onOpenSettings, onOpenReport }: EditingViewProps) {
  const { t, locale, formatNumber } = useI18n();
  const { preferences } = usePreferences();
  const [list, setList] = useState<SkillDto[] | null>(null);
  const [problems, setProblems] = useState<string[]>([]);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ title: string; error: CommandError } | null>(null);
  const [confirm, setConfirm] = useState<{ message: string; label: string; run: () => void } | null>(null);
  const [picking, setPicking] = useState(false);
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  const [count, setCount] = useState<{ key: string; value: ScopeCountDto | null; error: string | null } | null>(null);
  const [status, setStatus] = useState<TranslationStatus | null>(null);
  const { scopeOf } = useSheetTree(sheets);

  const dirty = draft.name !== draft.saved.name || draft.body !== draft.saved.body || !sameForm(draft.form, draft.saved.form);
  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange(false), [onDirtyChange]);

  const show = useCallback((found: { skills: SkillDto[]; problems: string[] }) => {
    setList(found.skills);
    setProblems(found.problems);
  }, []);
  useEffect(() => {
    if (!active) return;
    void skills()
      .then(show)
      .catch((reason: unknown) => setError({ title: t("editing.loadError"), error: normalizeCommandError(reason) }));
  }, [active, show, t]);

  // The strings of the scope are counted once it stays unchanged a moment.
  const scope = useMemo(() => scopeOfForm(draft.form), [draft.form]);
  const scopeKey = JSON.stringify(scope);
  useEffect(() => {
    if (!active) return;
    let current = true;
    const timer = window.setTimeout(() => {
      void skillCount(scope)
        .then((value) => { if (current) setCount({ key: scopeKey, value, error: null }); })
        .catch((reason: unknown) => { if (current) setCount({ key: scopeKey, value: null, error: normalizeCommandError(reason).message }); });
    }, COUNT_DELAY_MS);
    return () => { current = false; window.clearTimeout(timer); };
    // The scope's text, not the object it comes in, decides the count.
  }, [active, scopeKey]);
  const counted = count?.key === scopeKey ? count : null;

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
  const open = (skill: SkillDto) => leaving(() => setDraft(draftOf(skill)));
  const create = () => leaving(() => setDraft(emptyDraft));
  const setForm = (patch: Partial<ScopeForm>) => setDraft((current) => ({ ...current, form: { ...current.form, ...patch } }));

  const save = () => {
    setBusy(true);
    setError(null);
    const name = draft.name.trim();
    void saveSkill(name, draft.savedName, draft.savedText, scope, draft.body)
      .then((found) => {
        show(found);
        const saved = found.skills.find((skill) => skill.name === name);
        if (saved) setDraft(draftOf(saved));
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
        void deleteSkill(savedName, savedText)
          .then((found) => { show(found); setDraft(emptyDraft); })
          .catch((reason: unknown) => setError({ title: t("editing.saveError"), error: normalizeCommandError(reason) }))
          .finally(() => setBusy(false));
      },
    });
  };

  const togglePicked = useCallback((names: readonly string[], on: boolean) => {
    setPicked((current) => {
      const next = new Set(current);
      for (const name of names) {
        if (on) next.add(name);
        else next.delete(name);
      }
      return next;
    });
  }, []);
  const addPicked = () => {
    const known = new Set(lines(draft.form.paths));
    const added = scopeOf(picked).filter((path) => !known.has(path));
    if (added.length > 0) setForm({ paths: [...lines(draft.form.paths), ...added].join("\n") });
    setPicked(new Set());
    setPicking(false);
  };
  const translatedOf = useCallback((name: string): number | null => progress.get(name)?.translated ?? null, [progress]);
  const countLabel = (value: number | null) => value === null
    ? <span className="translate-remaining muted">{t("common.loading")}</span>
    : <span className="translate-remaining">{formatNumber(value)}</span>;

  const noModel = preferences.translationModel === "";
  const current = list?.find((skill) => skill.name === draft.savedName) ?? null;
  const block = noModel ? null
    : running ? t("editing.busy")
    : draft.savedName === null ? t("editing.saveFirst")
    : dirty ? t("editing.unsaved")
    : current?.problem ? current.problem
    : counted?.error ? counted.error
    : counted?.value?.total === 0 ? t("editing.nothing")
    : null;
  const start = () => {
    if (block !== null || noModel || draft.savedName === null) return;
    setError(null);
    void runSkill(draft.savedName, preferences.translationModel, preferences.translationEffort || null)
      .then(readStatus)
      .catch((reason: unknown) => setError({ title: t("editing.runError"), error: normalizeCommandError(reason) }));
  };
  const pathLines = lines(draft.form.paths);

  return (
    <div className="glossary-view editing-view">
      {error ? <ErrorBanner title={error.title} error={error.error} onDismiss={() => setError(null)} /> : null}
      <div className="glossary-panes editing-panes">
        <nav className="glossary-facets" aria-label={t("editing.skills")}>
          <div className="glossary-facet-head">{t("editing.skills")}</div>
          <div className="glossary-facet-group">
            {(list ?? []).map((skill) => (
              <button key={skill.name} type="button" className={`glossary-facet${draft.savedName === skill.name ? " active" : ""}${skill.problem ? " danger" : ""}`} aria-pressed={draft.savedName === skill.name} title={skill.problem ?? skill.name} onClick={() => open(skill)}>
                <UiIcon icon={skill.problem ? "circleAlert" : "wand"} size="xs" />
                <span className="glossary-facet-label">{skill.name}</span>
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

        <section className="editing-editor" aria-label={t("editing.skill")}>
          <label className="field">
            <span className="field-label">{t("editing.name")}</span>
            <input className="input" value={draft.name} spellCheck={false} placeholder={t("editing.namePlaceholder")} disabled={busy} onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
          </label>
          {current?.problem ? <p className="glossary-problem"><UiIcon icon="circleAlert" size="xs" />{current.problem}</p> : null}
          <textarea className="input editing-text" value={draft.body} spellCheck placeholder={t("editing.textPlaceholder")} aria-label={t("editing.skill")} disabled={busy} onChange={(event) => setDraft({ ...draft, body: event.target.value })} />
          <div className="editing-actions">
            {draft.savedName !== null ? <button className="button button-ghost" type="button" disabled={busy} onClick={remove}>{t("editing.delete")}</button> : null}
            <span className="spacer" />
            <button className="button button-ghost" type="button" disabled={busy || !dirty} onClick={() => setDraft({ ...draft, name: draft.saved.name, body: draft.saved.body, form: draft.saved.form })}>{t("editing.revert")}</button>
            <button className="button button-primary" type="button" disabled={busy || !dirty || draft.name.trim() === "" || draft.body.trim() === ""} onClick={save}>{t("editing.save")}</button>
          </div>
        </section>

        <aside className="editing-run" aria-label={t("editing.scope")}>
          <div className="glossary-facet-head">{t("editing.scope")}</div>
          <label className="field">
            <span className="field-label">{t("editing.paths")}</span>
            <textarea className="input editing-paths" rows={6} value={draft.form.paths} spellCheck={false} placeholder={t("editing.pathsPlaceholder")} disabled={busy} onChange={(event) => setForm({ paths: event.target.value })} />
          </label>
          {pathLines.length > 0 && counted?.value && counted.value.paths.length === pathLines.length ? (
            <ul className="editing-path-counts">
              {pathLines.map((path, index) => (
                <li key={`${path}:${index}`}><span className="mono">{path}</span><span className="muted">{formatNumber(counted.value?.paths[index] ?? 0)}</span></li>
              ))}
            </ul>
          ) : null}
          <div className="editing-actions">
            <button className="button button-ghost" type="button" disabled={busy || running} onClick={() => setPicking((value) => !value)}>
              <UiIcon icon={picking ? "chevronDown" : "chevronRight"} size="xs" />{t("editing.pick")}
            </button>
          </div>
          {picking ? (
            <>
              <SheetChooser sheets={sheets} selected={picked} onToggle={togglePicked} countOf={translatedOf} renderCount={countLabel} disabled={busy} label={t("editing.pick")} />
              <div className="editing-actions">
                <button className="button button-secondary" type="button" disabled={picked.size === 0} onClick={addPicked}><UiIcon icon="plus" size="sm" />{t("editing.addPicked")}</button>
              </div>
            </>
          ) : null}
          <label className="field">
            <span className="field-label">{t("editing.exclude")}</span>
            <textarea className="input editing-paths" rows={2} value={draft.form.exclude} spellCheck={false} disabled={busy} onChange={(event) => setForm({ exclude: event.target.value })} />
          </label>
          <label className="field">
            <span className="field-label">{t("editing.matchSource")}</span>
            <input className="input mono" value={draft.form.source} spellCheck={false} disabled={busy} onChange={(event) => setForm({ source: event.target.value })} />
          </label>
          <label className="field">
            <span className="field-label">{t("editing.matchTranslation")}</span>
            <input className="input mono" value={draft.form.translation} spellCheck={false} disabled={busy} onChange={(event) => setForm({ translation: event.target.value })} />
          </label>
          <label className="field">
            <span className="field-label">{t("editing.matchOther")}</span>
            <input className="input mono" value={draft.form.other} spellCheck={false} disabled={busy} onChange={(event) => setForm({ other: event.target.value })} />
          </label>
          <p className="translate-summary">
            {counted?.value ? t("editing.count", { strings: formatNumber(counted.value.total) }) : counted?.error ? <span className="glossary-problem">{counted.error}</span> : t("editing.counting")}
          </p>
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
