import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Dialog } from "radix-ui";
import { normalizeCommandError, projectKnowledge, projectTermCandidates, saveKnowledgeStyle, saveKnowledgeTerms } from "../ipc";
import { filterRows, inputsFromRows, openCandidates, rowFromCandidate, rowProblems, rowsChanged, rowsFromEntries, type GlossaryRow, type RowProblem } from "../projectGuide";
import type { CommandError, ProjectKnowledgeDto, TermCandidateDto } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ConfirmDialog } from "./ConfirmDialog";
import { ErrorBanner } from "./ErrorBanner";

export type ProjectGuideTab = "terms" | "candidates" | "style";

/** Where the candidates a person skipped are kept on this computer. */
const SKIPPED_KEY = "aeria.guide.skippedCandidates";

function readSkipped(): Set<string> {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(SKIPPED_KEY) ?? "[]");
    return new Set(Array.isArray(stored) ? stored.filter((item): item is string => typeof item === "string") : []);
  } catch {
    return new Set();
  }
}

function writeSkipped(skipped: ReadonlySet<string>) {
  try {
    window.localStorage.setItem(SKIPPED_KEY, JSON.stringify([...skipped]));
  } catch {
    // Skipping is a convenience of this computer; the list still works.
  }
}

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

/** Rows rendered at once; the filter narrows larger term lists. */
const ROWS_SHOWN = 300;
/** The largest knowledge file Aeria reads. */
const FILE_LIMIT = 8 * 1024 * 1024;

/**
 * The project's knowledge in `aeria-knowledge/`: terms and style.
 * Memoized so the closed dialog does not re-render with the workbench.
 */
export const ProjectGuideDialog = memo(function ProjectGuideDialog({ open, initialTab, onOpenChange }: ProjectGuideDialogProps) {
  const { t } = useI18n();
  const [tab, setTab] = useState<ProjectGuideTab>(initialTab);
  const [saved, setSaved] = useState<ProjectKnowledgeDto | null>(null);
  const [rows, setRows] = useState<GlossaryRow[]>([]);
  const [style, setStyle] = useState("");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);
  const [confirm, setConfirm] = useState<{ message: string; confirmLabel?: string; run: () => void } | null>(null);
  const nextKey = useRef(0);
  const [candidates, setCandidates] = useState<TermCandidateDto[] | null>(null);
  const [finding, setFinding] = useState(false);
  const [chosen, setChosen] = useState<ReadonlyMap<string, number>>(new Map());
  const [skipped, setSkipped] = useState<ReadonlySet<string>>(readSkipped);

  const show = useCallback((knowledge: ProjectKnowledgeDto) => {
    setSaved(knowledge);
    const next = rowsFromEntries(knowledge.entries);
    nextKey.current = next.length;
    setRows(next);
    setStyle(knowledge.style ?? "");
  }, []);

  const load = useCallback(() => {
    setError(null);
    void projectKnowledge().then(show).catch((reason: unknown) => setError(normalizeCommandError(reason)));
  }, [show]);

  useEffect(() => {
    if (!open) return;
    setTab(initialTab);
    setQuery("");
    load();
  }, [initialTab, load, open]);

  const problems = useMemo(() => rowProblems(rows), [rows]);
  const termsDirty = saved !== null && rowsChanged(rows, saved.entries);
  const styleDirty = saved !== null && style !== (saved.style ?? "");
  const styleBytes = useMemo(() => new TextEncoder().encode(style).length, [style]);
  const filtered = useMemo(() => filterRows(rows, query), [query, rows]);

  const run = async (operation: () => Promise<ProjectKnowledgeDto>) => {
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

  const saveTerms = () => {
    if (!saved) return;
    const write = () => void run(() => saveKnowledgeTerms(saved.termsText, inputsFromRows(rows)));
    if (saved.termsError && saved.termsText !== null) {
      setConfirm({ message: t("guide.glossary.replaceBroken"), confirmLabel: t("guide.save"), run: write });
    } else if (saved.diagnostics.length > 0) {
      setConfirm({ message: t("guide.glossary.dropExcluded", { count: saved.diagnostics.length }), confirmLabel: t("guide.save"), run: write });
    } else {
      write();
    }
  };

  const saveStyle = () => {
    if (!saved) return;
    void run(() => saveKnowledgeStyle(saved.style, style));
  };

  // A term a person edits is one a person decided.
  const update = (key: number, field: "term" | "translation" | "note" | "forbidden", value: string) => {
    setRows((current) => current.map((row) => row.key === key ? { ...row, [field]: value, settled: true } : row));
  };

  const setSettled = (key: number, settled: boolean) => {
    setRows((current) => current.map((row) => row.key === key ? { ...row, settled } : row));
  };

  const setMatchCase = (key: number, matchCase: boolean) => {
    setRows((current) => current.map((row) => row.key === key ? { ...row, matchCase, settled: true } : row));
  };

  const addRow = () => {
    const key = nextKey.current++;
    setQuery("");
    setRows((current) => [...current, { key, term: "", translation: "", note: "", forbidden: "", settled: true, matchCase: false }]);
  };

  const findCandidates = () => {
    setFinding(true);
    setError(null);
    void projectTermCandidates()
      .then((found) => { setCandidates(found); setChosen(new Map()); })
      .catch((reason: unknown) => setError(normalizeCommandError(reason)))
      .finally(() => setFinding(false));
  };

  const skip = (phrase: string | null) => {
    const next = new Set(phrase === null ? [] : [...skipped, phrase]);
    setSkipped(next);
    writeSkipped(next);
  };

  // The candidate becomes a term row to review and save with the others.
  const addCandidate = (candidate: TermCandidateDto) => {
    const key = nextKey.current++;
    setRows((current) => [...current, rowFromCandidate(candidate, chosen.get(candidate.phrase) ?? 0, key)]);
    setQuery(candidate.phrase);
    setTab("terms");
  };

  const remaining = useMemo(() => openCandidates(candidates ?? [], rows, skipped), [candidates, rows, skipped]);

  const close = (next: boolean) => {
    if (next || !(termsDirty || styleDirty)) { onOpenChange(next); return; }
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
                { value: "terms", label: termsDirty ? `${t("guide.tab.terms")} •` : t("guide.tab.terms") },
                { value: "candidates", label: t("guide.tab.candidates") },
                { value: "style", label: styleDirty ? `${t("guide.tab.style")} •` : t("guide.tab.style") },
              ]}
            />
            <Dialog.Close className="icon-button icon-button-ghost" aria-label={t("settings.closeLabel")}><UiIcon icon="x" size="sm" /></Dialog.Close>
          </header>

          {error ? <ErrorBanner title={t("guide.error")} error={error} onDismiss={() => setError(null)} /> : null}

          {saved === null ? (
            error ? null : <p className="muted">{t("common.loading")}</p>
          ) : tab === "terms" ? (
            <section className="guide-body">
              <p className="field-hint">{t("guide.terms.hint")}</p>
              {saved.termsError ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{saved.termsError}</p> : null}
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
              <div className="guide-table" role="table" aria-label={t("guide.tab.terms")}>
                <div className="guide-row guide-head" role="row">
                  <span role="columnheader">{t("guide.glossary.term")}</span>
                  <span role="columnheader">{t("guide.glossary.translation")}</span>
                  <span role="columnheader">{t("guide.glossary.note")}</span>
                  <span role="columnheader">{t("guide.glossary.forbidden")}</span>
                  <span role="columnheader" title={t("guide.terms.matchCaseHint")}>{t("guide.terms.matchCase")}</span>
                  <span role="columnheader" title={t("guide.terms.settledHint")}>{t("guide.terms.settled")}</span>
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
                      <input type="checkbox" checked={row.matchCase} aria-label={t("guide.terms.matchCase")} title={t("guide.terms.matchCaseHint")} onChange={(event) => setMatchCase(row.key, event.target.checked)} />
                      <input type="checkbox" checked={row.settled} aria-label={t("guide.terms.settled")} title={t("guide.terms.settledHint")} onChange={(event) => setSettled(row.key, event.target.checked)} />
                      <button className="icon-button icon-button-ghost" type="button" aria-label={t("guide.glossary.remove")} title={t("guide.glossary.remove")} onClick={() => setRows((current) => current.filter((candidate) => candidate.key !== row.key))}><UiIcon icon="trash" size="sm" /></button>
                    </div>
                  );
                })}
                {filtered.length > ROWS_SHOWN ? <p className="field-hint">{t("guide.glossary.more", { shown: ROWS_SHOWN, count: filtered.length })}</p> : null}
                {rows.length === 0 ? <p className="field-hint">{t("guide.glossary.empty")}</p> : null}
              </div>
              <div className="dialog-actions">
                {problems.size > 0 ? <span className="guide-problems">{t("guide.glossary.problems", { count: problems.size })}</span> : null}
                <button className="button button-ghost" type="button" disabled={busy || !termsDirty} onClick={() => show(saved)}>{t("guide.revert")}</button>
                <button className="button button-primary" type="button" disabled={busy || !termsDirty || problems.size > 0} onClick={saveTerms}>{t("guide.save")}</button>
              </div>
            </section>
          ) : tab === "candidates" ? (
            <section className="guide-body">
              <p className="field-hint">{t("guide.candidates.hint")}</p>
              <div className="guide-toolbar">
                <button className="button button-secondary" type="button" disabled={finding} onClick={findCandidates}>
                  {finding ? <span className="spinner" /> : <UiIcon icon="search" size="sm" />}{candidates === null ? t("guide.candidates.find") : t("guide.candidates.findAgain")}
                </button>
                {candidates !== null ? <span className="muted">{t("guide.candidates.count", { count: remaining.length })}</span> : null}
                {skipped.size > 0 ? <button className="link-button" type="button" onClick={() => skip(null)}>{t("guide.candidates.restore", { count: skipped.size })}</button> : null}
              </div>
              {finding ? <p className="muted">{t("guide.candidates.finding")}</p> : null}
              {candidates !== null && !finding && remaining.length === 0 ? <p className="field-hint">{t("guide.candidates.none")}</p> : null}
              <div className="guide-candidates">
                {remaining.slice(0, ROWS_SHOWN).map((candidate) => {
                  const picked = chosen.get(candidate.phrase) ?? 0;
                  return (
                    <article key={candidate.phrase} className="guide-candidate">
                      <header className="guide-candidate-head">
                        <strong>{candidate.phrase}</strong>
                        <span className="muted">{t("guide.candidates.strings", { count: candidate.translated })} · {candidate.sheets.map((sheet) => sheet.sheet).join(", ")}</span>
                      </header>
                      <div className="search-chips" role="radiogroup" aria-label={t("guide.candidates.renderings")}>
                        {candidate.renderings.map((rendering, index) => (
                          <button key={index} type="button" role="radio" aria-checked={index === picked} className={index === picked ? "chip-toggle on" : "chip-toggle"} title={t("guide.candidates.choose")} onClick={() => setChosen((current) => new Map(current).set(candidate.phrase, index))}>
                            {rendering.words.join(" ")}<span className="search-issue-count">{rendering.strings}</span>
                          </button>
                        ))}
                      </div>
                      <details className="guide-candidate-examples">
                        <summary>{t("guide.candidates.examples")}</summary>
                        {candidate.renderings.map((rendering, index) => (
                          <ul key={index}>
                            {rendering.examples.map((example) => (
                              <li key={`${example.path}|${example.context}`}>
                                <span className="guide-candidate-source">{example.source}</span>
                                <span>{example.translation}</span>
                              </li>
                            ))}
                          </ul>
                        ))}
                      </details>
                      <div className="guide-candidate-actions">
                        <button className="button button-secondary" type="button" onClick={() => addCandidate(candidate)}><UiIcon icon="plus" size="sm" />{t("guide.candidates.add")}</button>
                        <button className="button button-ghost" type="button" onClick={() => skip(candidate.phrase)}>{t("guide.candidates.skip")}</button>
                      </div>
                    </article>
                  );
                })}
              </div>
            </section>
          ) : (
            <section className="guide-body">
              <p className="field-hint">{t("guide.style.hint")}</p>
              {saved.styleError ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{saved.styleError}</p> : null}
              <textarea className="input guide-guidance" value={style} spellCheck placeholder={t("guide.style.placeholder")} aria-label={t("guide.tab.style")} onChange={(event) => setStyle(event.target.value)} />
              <div className="dialog-actions">
                <span className={styleBytes > FILE_LIMIT ? "guide-problems" : "muted"}>{t("guide.size", { kib: (styleBytes / 1024).toFixed(1) })}</span>
                <button className="button button-ghost" type="button" disabled={busy || !styleDirty} onClick={() => setStyle(saved.style ?? "")}>{t("guide.revert")}</button>
                <button className="button button-primary" type="button" disabled={busy || !styleDirty || styleBytes > FILE_LIMIT} onClick={saveStyle}>{t("guide.save")}</button>
              </div>
            </section>
          )}
          <ConfirmDialog open={confirm !== null} message={confirm?.message ?? ""} {...(confirm?.confirmLabel ? { confirmLabel: confirm.confirmLabel, cancelLabel: t("guide.cancel") } : {})} onKeepEditing={() => setConfirm(null)} onDiscard={() => { const action = confirm?.run; setConfirm(null); action?.(); }} />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
});
