import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Dialog } from "radix-ui";
import { normalizeCommandError, projectGuide, saveProjectGlossary, saveProjectGuidance } from "../ipc";
import { filterRows, inputsFromRows, rowProblems, rowsChanged, rowsFromEntries, type GlossaryRow, type RowProblem } from "../projectGuide";
import type { CommandError, ProjectGuideDto } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ConfirmDialog } from "./ConfirmDialog";
import { ErrorBanner } from "./ErrorBanner";

export type ProjectGuideTab = "glossary" | "guidance";

type ProjectGuideDialogProps = {
  open: boolean;
  initialTab: ProjectGuideTab;
  onOpenChange: (open: boolean) => void;
};

const problemLabels: Readonly<Record<RowProblem, MessageKey>> = {
  emptyTerm: "guide.problem.emptyTerm",
  emptyTranslation: "guide.problem.emptyTranslation",
  duplicateTerm: "guide.problem.duplicateTerm",
};

/** Rows rendered at once; the filter narrows larger glossaries. */
const ROWS_SHOWN = 300;
const GUIDANCE_LIMIT = 64 * 1024;

/** The project's shared glossary and guidance, edited by translators. */
/** Memoized so the closed dialog does not re-render with the workbench. */
export const ProjectGuideDialog = memo(function ProjectGuideDialog({ open, initialTab, onOpenChange }: ProjectGuideDialogProps) {
  const { t } = useI18n();
  const [tab, setTab] = useState<ProjectGuideTab>(initialTab);
  const [saved, setSaved] = useState<ProjectGuideDto | null>(null);
  const [rows, setRows] = useState<GlossaryRow[]>([]);
  const [guidance, setGuidance] = useState("");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);
  const [confirm, setConfirm] = useState<{ message: string; confirmLabel?: string; run: () => void } | null>(null);
  const nextKey = useRef(0);

  const show = useCallback((guide: ProjectGuideDto) => {
    setSaved(guide);
    const next = rowsFromEntries(guide.entries);
    nextKey.current = next.length;
    setRows(next);
    setGuidance(guide.guidance ?? "");
  }, []);

  const load = useCallback(() => {
    setError(null);
    void projectGuide().then(show).catch((reason: unknown) => setError(normalizeCommandError(reason)));
  }, [show]);

  useEffect(() => {
    if (!open) return;
    setTab(initialTab);
    setQuery("");
    load();
  }, [initialTab, load, open]);

  const problems = useMemo(() => rowProblems(rows), [rows]);
  const glossaryDirty = saved !== null && rowsChanged(rows, saved.entries);
  const guidanceDirty = saved !== null && guidance !== (saved.guidance ?? "");
  const guidanceBytes = useMemo(() => new TextEncoder().encode(guidance).length, [guidance]);
  const filtered = useMemo(() => filterRows(rows, query), [query, rows]);

  const run = async (operation: () => Promise<ProjectGuideDto>) => {
    setBusy(true);
    setError(null);
    try {
      show(await operation());
    } catch (reason) {
      setError(normalizeCommandError(reason));
    } finally {
      setBusy(false);
    }
  };

  const saveGlossary = () => {
    if (!saved) return;
    const write = () => void run(() => saveProjectGlossary(saved.glossaryText, inputsFromRows(rows)));
    if (saved.glossaryError && saved.glossaryText !== null) {
      setConfirm({ message: t("guide.glossary.replaceBroken"), confirmLabel: t("guide.save"), run: write });
    } else if (saved.diagnostics.length > 0) {
      setConfirm({ message: t("guide.glossary.dropExcluded", { count: saved.diagnostics.length }), confirmLabel: t("guide.save"), run: write });
    } else {
      write();
    }
  };

  const saveGuidance = () => {
    if (!saved) return;
    void run(() => saveProjectGuidance(saved.guidance, guidance));
  };

  const update = (key: number, field: keyof Omit<GlossaryRow, "key">, value: string) => {
    setRows((current) => current.map((row) => row.key === key ? { ...row, [field]: value } : row));
  };

  const addRow = () => {
    const key = nextKey.current++;
    setQuery("");
    setRows((current) => [...current, { key, term: "", translation: "", note: "", forbidden: "" }]);
  };

  const close = (next: boolean) => {
    if (next || !(glossaryDirty || guidanceDirty)) { onOpenChange(next); return; }
    setConfirm({ message: t("guide.discard"), run: () => onOpenChange(false) });
  };

  return (
    <Dialog.Root open={open} onOpenChange={close}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog guide-dialog" aria-describedby={undefined}>
          <header className="dialog-header">
            <Dialog.Title className="dialog-title">{t("guide.title")}</Dialog.Title>
            <Segmented
              label={t("guide.title")}
              value={tab}
              onChange={setTab}
              options={[
                { value: "glossary", label: glossaryDirty ? `${t("guide.tab.glossary")} •` : t("guide.tab.glossary") },
                { value: "guidance", label: guidanceDirty ? `${t("guide.tab.guidance")} •` : t("guide.tab.guidance") },
              ]}
            />
            <Dialog.Close className="icon-button icon-button-ghost" aria-label={t("settings.closeLabel")}><UiIcon icon="x" size="sm" /></Dialog.Close>
          </header>

          {error ? <ErrorBanner title={t("guide.error")} error={error} onDismiss={() => setError(null)} /> : null}

          {saved === null ? (
            error ? null : <p className="muted">{t("common.loading")}</p>
          ) : tab === "glossary" ? (
            <section className="guide-body">
              <p className="field-hint">{t("guide.glossary.hint")}</p>
              {saved.glossaryError ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{saved.glossaryError}</p> : null}
              {saved.diagnostics.length > 0 ? (
                <details className="guide-diagnostics">
                  <summary>{t("guide.glossary.excluded", { count: saved.diagnostics.length })}</summary>
                  <ul>{saved.diagnostics.map((problem) => <li key={problem.line}>{t("guide.glossary.line", { line: problem.line })}: {problem.message}</li>)}</ul>
                </details>
              ) : null}
              <div className="guide-toolbar">
                <input className="input" type="search" value={query} placeholder={t("guide.glossary.filter")} aria-label={t("guide.glossary.filter")} onChange={(event) => setQuery(event.target.value)} />
                <span className="muted">{t("guide.glossary.count", { count: rows.length })}</span>
                <button className="button button-secondary" type="button" disabled={busy} onClick={addRow}><UiIcon icon="plus" size="sm" />{t("guide.glossary.add")}</button>
              </div>
              <div className="guide-table" role="table" aria-label={t("guide.tab.glossary")}>
                <div className="guide-row guide-head" role="row">
                  <span role="columnheader">{t("guide.glossary.term")}</span>
                  <span role="columnheader">{t("guide.glossary.translation")}</span>
                  <span role="columnheader">{t("guide.glossary.note")}</span>
                  <span role="columnheader">{t("guide.glossary.forbidden")}</span>
                  <span />
                </div>
                {filtered.slice(0, ROWS_SHOWN).map((row) => {
                  const problem = problems.get(row.key);
                  return (
                    <div key={row.key} className={problem ? "guide-row invalid" : "guide-row"} role="row" title={problem ? t(problemLabels[problem]) : undefined}>
                      <input className="input" value={row.term} aria-label={t("guide.glossary.term")} onChange={(event) => update(row.key, "term", event.target.value)} />
                      <input className="input" value={row.translation} aria-label={t("guide.glossary.translation")} onChange={(event) => update(row.key, "translation", event.target.value)} />
                      <input className="input" value={row.note} aria-label={t("guide.glossary.note")} onChange={(event) => update(row.key, "note", event.target.value)} />
                      <input className="input" value={row.forbidden} placeholder={t("guide.glossary.forbiddenHint")} aria-label={t("guide.glossary.forbidden")} onChange={(event) => update(row.key, "forbidden", event.target.value)} />
                      <button className="icon-button icon-button-ghost" type="button" aria-label={t("guide.glossary.remove")} title={t("guide.glossary.remove")} onClick={() => setRows((current) => current.filter((candidate) => candidate.key !== row.key))}><UiIcon icon="trash" size="sm" /></button>
                    </div>
                  );
                })}
                {filtered.length > ROWS_SHOWN ? <p className="field-hint">{t("guide.glossary.more", { shown: ROWS_SHOWN, count: filtered.length })}</p> : null}
                {rows.length === 0 ? <p className="field-hint">{t("guide.glossary.empty")}</p> : null}
              </div>
              <div className="dialog-actions">
                {problems.size > 0 ? <span className="guide-problems">{t("guide.glossary.problems", { count: problems.size })}</span> : null}
                <button className="button button-ghost" type="button" disabled={busy || !glossaryDirty} onClick={() => show(saved)}>{t("guide.revert")}</button>
                <button className="button button-primary" type="button" disabled={busy || !glossaryDirty || problems.size > 0} onClick={saveGlossary}>{t("guide.save")}</button>
              </div>
            </section>
          ) : (
            <section className="guide-body">
              <p className="field-hint">{t("guide.guidance.hint")}</p>
              {saved.guidanceError ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{saved.guidanceError}</p> : null}
              <textarea className="input guide-guidance" value={guidance} spellCheck placeholder={t("guide.guidance.placeholder")} aria-label={t("guide.tab.guidance")} onChange={(event) => setGuidance(event.target.value)} />
              <div className="dialog-actions">
                <span className={guidanceBytes > GUIDANCE_LIMIT ? "guide-problems" : "muted"}>{t("guide.guidance.size", { kib: (guidanceBytes / 1024).toFixed(1) })}</span>
                <button className="button button-ghost" type="button" disabled={busy || !guidanceDirty} onClick={() => setGuidance(saved.guidance ?? "")}>{t("guide.revert")}</button>
                <button className="button button-primary" type="button" disabled={busy || !guidanceDirty || guidanceBytes > GUIDANCE_LIMIT} onClick={saveGuidance}>{t("guide.save")}</button>
              </div>
            </section>
          )}
          <ConfirmDialog open={confirm !== null} message={confirm?.message ?? ""} {...(confirm?.confirmLabel ? { confirmLabel: confirm.confirmLabel, cancelLabel: t("guide.cancel") } : {})} onKeepEditing={() => setConfirm(null)} onDiscard={() => { const action = confirm?.run; setConfirm(null); action?.(); }} />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
});
