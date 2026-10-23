import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Dialog } from "radix-ui";
import { normalizeCommandError, runReports, translationContinue, translationRetry, translationStart, translationStatus, translationStop } from "../ipc";
import type { CommandError, ProjectSheetDto, ReportInfo, RunKind, SheetProgressDto, SourceBinding, TranslationStatus, TranslationStop } from "../types";
import type { MessageKey } from "../i18n/translate";
import { ErrorBanner } from "./ErrorBanner";
import { SheetChooser, useSheetTree } from "./SheetChooser";
import { TranslationCorrections } from "./TranslationCorrections";
import { TranslationRejections } from "./TranslationRejections";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n, type Translate } from "../ui/i18n";
import { usePreferences } from "../ui/preferences";
import { reportLabel, runKind } from "../reportLabel";

type TranslateDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sheets: readonly ProjectSheetDto[];
  progress: ReadonlyMap<string, SheetProgressDto>;
  /** The sheet the editor shows, chosen when the dialog opens with nothing chosen. */
  sheetName: string | null;
  onOpenSettings: () => void;
  /** A run wrote files: sheets and progress should be read again. */
  onFilesChanged: () => void;
  /** Opens a string in the editor. */
  onRevealBinding?: ((binding: SourceBinding) => void) | undefined;
  /** Opens a run's report in a tab of its own. */
  onOpenReport: (reportId: string, label: string) => void;
};

/** How often a running translation's progress is read. */
const POLL_MS = 1000;
/** Requests in flight listed by name; the rest are counted. */
const LISTED_ACTIVE = 3;
/** An answer slower than this gets a hint about the reasoning depth. */
const SLOW_MS = 5 * 60 * 1000;

/** Minutes and seconds, such as 4:07. */
function formatElapsed(milliseconds: number): string {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

const stopLabels: Readonly<Record<TranslationStop["reason"], MessageKey>> = {
  finished: "translate.stop.finished",
  cancelled: "translate.stop.cancelled",
  usageLimit: "translate.stop.usageLimit",
  signInRequired: "translate.stop.signInRequired",
  failed: "translate.stop.failed",
};

/**
 * How far a run is: the strings it settled of those it set out to take, as
 * the meter shows them, then what became of them.
 */
export function RunProgress({ status }: { status: TranslationStatus }) {
  const { t, locale, formatNumber } = useI18n();
  const done = status.written + status.unchanged + status.rejected;
  const running = status.running;
  return (
    <>
      <div className="meter" aria-hidden="true">
        <span className="meter-translated" style={{ width: `${status.strings > 0 ? Math.min(1, done / status.strings) * 100 : running ? 0 : 100}%` }} />
      </div>
      {status.waitingUntil !== null && running ? (
        <p className="field-hint"><strong>{t("translate.waitingLimit", { time: new Date(status.waitingUntil * 1000).toLocaleString(locale, { dateStyle: "short", timeStyle: "short" }) })}</strong></p>
      ) : null}
      <p className="export-facts">
        <span><strong>{t("translate.progressDone", { done: formatNumber(done), strings: formatNumber(status.strings) })}</strong></span>
        {status.written > 0 ? <span>{t(status.fixing ? "translate.corrected" : "translate.translated", { count: status.written })}</span> : null}
        {status.unchanged > 0 ? <span>{t("translate.unchanged", { count: status.unchanged })}</span> : null}
        {status.rejected > 0 ? <span>{t("translate.rejected", { count: status.rejected })}</span> : null}
      </p>
    </>
  );
}

/**
 * A button that continues a run that stopped before it settled every string
 * it planned, or whose strings the checks refused; `onContinued` follows the
 * run that continues it.
 */
export function ContinueButton({ status, onContinued }: { status: TranslationStatus; onContinued?: () => void }) {
  const { t } = useI18n();
  const [error, setError] = useState<CommandError | null>(null);
  const [starting, setStarting] = useState(false);
  const report = status.report;
  if (!report || status.running || !status.stop || (status.stop.reason === "finished" && status.rejected === 0)) return null;
  return (
    <>
      <button className="button button-primary" type="button" disabled={starting} title={t("report.continueTitle")} onClick={() => {
        setStarting(true);
        setError(null);
        void translationContinue(report)
          .then(() => onContinued?.())
          .catch((reason: unknown) => setError(normalizeCommandError(reason)))
          .finally(() => setStarting(false));
      }}>
        <UiIcon icon="sparkles" size="sm" />{t("report.continue")}
      </button>
      {error ? <span className="glossary-problem">{error.message}</span> : null}
    </>
  );
}

/** A button that opens the report of a run, when it has one. */
export function ReportButton({ status, onOpenReport }: { status: TranslationStatus; onOpenReport: (reportId: string, label: string) => void }) {
  const { t, locale } = useI18n();
  const report = status.report;
  if (!report) return null;
  return (
    <button className="button button-secondary" type="button" onClick={() => onOpenReport(report, reportLabel(runKind(status), status.startedAt, t, locale))}>
      <UiIcon icon="clipboard" size="sm" />{t("report.open")}
    </button>
  );
}

/**
 * A button that opens the newest report of a run of `kind` in the data
 * folder, for when no run of this session has one.
 */
export function LastReportButton({ kind, onOpenReport }: { kind: RunKind; onOpenReport: (reportId: string, label: string) => void }) {
  const { t, locale } = useI18n();
  const [last, setLast] = useState<ReportInfo | null>(null);
  useEffect(() => {
    let live = true;
    void runReports()
      .then((reports) => { if (live) setLast(reports.find((report) => report.run.kind === kind) ?? null); })
      .catch(() => { if (live) setLast(null); });
    return () => { live = false; };
  }, [kind]);
  if (!last) return null;
  return (
    <p className="glossary-run-actions">
      <button className="button button-secondary" type="button" onClick={() => onOpenReport(last.id, reportLabel(last.run.kind, last.run.startedAt, t, locale))}>
        <UiIcon icon="clipboard" size="sm" />{t("report.last")}
      </button>
    </p>
  );
}

export function describeStop(stop: TranslationStop, t: Translate, locale: string): string {
  if (stop.reason === "usageLimit" && stop.resetsAt !== null) {
    return t("translate.stop.usageLimitUntil", { time: new Date(stop.resetsAt * 1000).toLocaleString(locale) });
  }
  if (stop.reason === "failed") return t("translate.stop.failedWith", { message: stop.message });
  return t(stopLabels[stop.reason]);
}

/** Machine translation of chosen sheets and folders of sheets, and its progress. */
export const TranslateDialog = memo(function TranslateDialog({ open, onOpenChange, sheets, progress, sheetName, onOpenSettings, onFilesChanged, onRevealBinding, onOpenReport }: TranslateDialogProps) {
  const { t, locale, formatNumber } = useI18n();
  const { preferences } = usePreferences();
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [fuzzy, setFuzzy] = useState(false);
  const [status, setStatus] = useState<TranslationStatus | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const wasRunning = useRef(false);

  const { allNames, scopeOf } = useSheetTree(sheets);

  /** Strings of a sheet a run would translate; null while its progress is unknown. */
  const remainingOf = useCallback((name: string): number | null => {
    const entry = progress.get(name);
    if (!entry) return null;
    return entry.strings - entry.translated + (fuzzy ? entry.fuzzy : 0);
  }, [fuzzy, progress]);
  const remainingOfAll = useCallback((names: readonly string[]) => names.reduce((sum, name) => sum + (remainingOf(name) ?? 0), 0), [remainingOf]);

  useEffect(() => {
    if (!open) return;
    setSelected((current) => current.size > 0 || !sheetName || !(remainingOf(sheetName) ?? 1) ? current : new Set([sheetName]));
    // Only when the dialog opens: the choice stays as the translator left it.
  }, [open]);

  const readStatus = useCallback(async () => {
    try { setStatus(await translationStatus()); }
    catch (reason) { setError(normalizeCommandError(reason)); }
  }, []);

  useEffect(() => {
    if (!open) return;
    void readStatus();
    const timer = window.setInterval(() => void readStatus(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [open, readStatus]);

  const running = status?.running ?? false;
  useEffect(() => {
    if (wasRunning.current && !running) onFilesChanged();
    wasRunning.current = running;
  }, [onFilesChanged, running]);

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
  const choose = useCallback((names: readonly string[]) => toggle(names.filter((name) => (remainingOf(name) ?? 1) > 0), true), [remainingOf, toggle]);
  const selectedNames = useMemo(() => allNames.filter((name) => selected.has(name)), [allNames, selected]);
  const strings = remainingOfAll(selectedNames);
  const noModel = preferences.translationModel === "";
  const cachedShare = status && status.inputTokens > 0 ? status.cachedTokens / status.inputTokens : null;

  async function start() {
    setError(null);
    try {
      await translationStart(scopeOf(selected), fuzzy, preferences.translationModel, preferences.translationEffort || null);
      await readStatus();
    } catch (reason) {
      setError(normalizeCommandError(reason));
    }
  }

  async function retry(contexts: string[]) {
    setError(null);
    try {
      await translationRetry(contexts, preferences.translationModel, preferences.translationEffort || null);
      await readStatus();
    } catch (reason) {
      setError(normalizeCommandError(reason));
    }
  }

  function remainingLabel(count: number | null) {
    if (count === null) return <span className="translate-remaining muted">{t("common.loading")}</span>;
    if (count === 0) return <span className="translate-remaining done">{t("translate.done")}</span>;
    return <span className="translate-remaining">{formatNumber(count)}</span>;
  }

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog translate-dialog" aria-describedby="translate-description">
          <Dialog.Title className="dialog-title">{t("translate.title")}</Dialog.Title>
          <p id="translate-description" className="dialog-description">{t("translate.description")}</p>
          {error ? <ErrorBanner title={t("translate.error")} error={error} onDismiss={() => setError(null)} /> : null}

          <div className="translate-layout">
            <SheetChooser
              sheets={sheets}
              selected={selected}
              onToggle={toggle}
              countOf={remainingOf}
              renderCount={remainingLabel}
              disabled={running}
              label={t("translate.sheets")}
              presets={<>
                <button className="button button-ghost translate-preset" type="button" disabled={running} onClick={() => choose(allNames.filter((name) => name.startsWith("quest/")))}>{t("translate.preset.quests")}</button>
                <button className="button button-ghost translate-preset" type="button" disabled={running} onClick={() => choose(allNames)}>{t("translate.preset.untranslated")}</button>
                <button className="button button-ghost translate-preset" type="button" disabled={running || selected.size === 0} onClick={() => setSelected(new Set())}>{t("translate.preset.none")}</button>
              </>}
            />

            <div className="translate-side">
              <p className="translate-summary">{t("translate.selection", { sheets: formatNumber(selectedNames.length), strings: formatNumber(strings) })}</p>
              <label className="field field-inline">
                <input type="checkbox" checked={fuzzy} disabled={running} onChange={(event) => setFuzzy(event.target.checked)} />
                <span>{t("translate.fuzzy")}</span>
              </label>
              <p className="field-hint">{t("translate.namesFirst")}</p>
              <p className="field-hint">{t("translate.alsoSees")}</p>
              {noModel ? (
                <p className="export-warning">
                  <UiIcon icon="circleAlert" size="xs" />{t("translate.noModel")}{" "}
                  <button className="link-button" type="button" onClick={onOpenSettings}>{t("translate.openSettings")}</button>
                </p>
              ) : <p className="field-hint">{t("translate.model", { model: preferences.translationModel })}</p>}

              {status && (running || status.stop) ? (
                <div className="translate-progress" aria-live="polite">
                  <RunProgress status={status} />
                  <p className="export-facts">
                    <span>{t("translate.tokens", { input: formatNumber(status.inputTokens), output: formatNumber(status.outputTokens) })}</span>
                    {cachedShare !== null ? <span>{t("translate.cached", { percent: Math.round(cachedShare * 100) })}</span> : null}
                    {running ? <span>{t("translate.pace", { pace: status.pace })}</span> : null}
                  </p>
                  {running && status.active.length > 0 ? (
                    <ul className="translate-active">
                      {status.active.slice(0, LISTED_ACTIVE).map((active) => (
                        <li key={`${active.startedAt}:${active.path}:${active.retry}`}>
                          {t("translate.activeRequest", { path: active.path, strings: formatNumber(active.strings), elapsed: formatElapsed(Date.now() - active.startedAt) })}
                          {active.retry > 0 ? t("translate.activeRetry", { retry: active.retry }) : null}
                        </li>
                      ))}
                      {status.active.length > LISTED_ACTIVE ? <li>{t("translate.activeMore", { count: status.active.length - LISTED_ACTIVE })}</li> : null}
                    </ul>
                  ) : null}
                  {running && status.active.some((active) => Date.now() - active.startedAt > SLOW_MS) ? <p className="field-hint">{t("translate.slow")}</p> : null}
                  {status.message ? <p className="field-hint">{status.message}</p> : null}
                  {status.stop ? <p className="field-hint"><strong>{describeStop(status.stop, t, locale)}</strong></p> : null}
                  {status.report ? <p className="glossary-run-actions"><ContinueButton status={status} onContinued={() => void readStatus()} /><ReportButton status={status} onOpenReport={onOpenReport} /></p> : null}
                  {status.corrections.length > 0 ? (
                    <TranslationCorrections corrections={status.corrections} total={status.written} onReveal={onRevealBinding ? (binding) => { onOpenChange(false); onRevealBinding(binding); } : undefined} />
                  ) : null}
                  {status.rejections.length > 0 ? (
                    <TranslationRejections rejections={status.rejections} total={status.rejected} running={running} onReveal={onRevealBinding ? (binding) => { onOpenChange(false); onRevealBinding(binding); } : undefined} onRetry={status.fixing ? undefined : (contexts) => void retry(contexts)} />
                  ) : null}
                </div>
              ) : null}
              {status?.report ? null : <LastReportButton kind="translating" onOpenReport={onOpenReport} />}
            </div>
          </div>

          <div className="dialog-actions">
            <button className="button button-ghost" type="button" onClick={() => onOpenChange(false)}>{t(running ? "translate.hide" : "common.close")}</button>
            {running ? (
              <button className="button button-secondary" type="button" onClick={() => void translationStop()}>{t("translate.stopRun")}</button>
            ) : (
              <button className="button button-primary" type="button" disabled={noModel || strings === 0} onClick={() => void start()}>
                {t(status?.stop && status.stop.reason !== "finished" ? "translate.continue" : "translate.start")}
              </button>
            )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
});
