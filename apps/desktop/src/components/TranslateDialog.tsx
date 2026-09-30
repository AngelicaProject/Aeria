import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { Dialog } from "radix-ui";
import { normalizeCommandError, translationCount, translationStart, translationStatus, translationStop } from "../ipc";
import type { CommandError, TranslationCountDto, TranslationStatus, TranslationStop } from "../types";
import type { MessageKey } from "../i18n/translate";
import { ErrorBanner } from "./ErrorBanner";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n, type Translate } from "../ui/i18n";
import { usePreferences } from "../ui/preferences";

type Scope = "sheet" | "folder" | "project";

type TranslateDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The sheet the editor shows, the default scope. */
  sheetName: string | null;
  onOpenSettings: () => void;
};

/** How often a running translation's progress is read. */
const POLL_MS = 1000;

/** The folder of a sheet name, such as `quest/001/` for `quest/001/X`. */
function folderOf(sheetName: string | null): string | null {
  const index = sheetName?.lastIndexOf("/") ?? -1;
  return sheetName && index > 0 ? sheetName.slice(0, index + 1) : null;
}

const stopLabels: Readonly<Record<TranslationStop["reason"], MessageKey>> = {
  finished: "translate.stop.finished",
  cancelled: "translate.stop.cancelled",
  usageLimit: "translate.stop.usageLimit",
  signInRequired: "translate.stop.signInRequired",
  failed: "translate.stop.failed",
};

function describeStop(stop: TranslationStop, t: Translate, locale: string): string {
  if (stop.reason === "usageLimit" && stop.resetsAt !== null) {
    return t("translate.stop.usageLimitUntil", { time: new Date(stop.resetsAt * 1000).toLocaleString(locale) });
  }
  if (stop.reason === "failed") return t("translate.stop.failedWith", { message: stop.message });
  return t(stopLabels[stop.reason]);
}

/** Machine translation of a sheet, a folder of sheets, or the project, and its progress. */
export const TranslateDialog = memo(function TranslateDialog({ open, onOpenChange, sheetName, onOpenSettings }: TranslateDialogProps) {
  const { t, locale, formatNumber } = useI18n();
  const { preferences } = usePreferences();
  const folder = folderOf(sheetName);
  const [scope, setScope] = useState<Scope>(sheetName ? "sheet" : "project");
  const [fuzzy, setFuzzy] = useState(false);
  const [count, setCount] = useState<TranslationCountDto | null>(null);
  const [status, setStatus] = useState<TranslationStatus | null>(null);
  const [error, setError] = useState<CommandError | null>(null);

  const scopePaths = useMemo(() => {
    if (scope === "sheet" && sheetName) return [sheetName];
    if (scope === "folder" && folder) return [folder];
    return [];
  }, [folder, scope, sheetName]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    setCount(null);
    translationCount(scopePaths, fuzzy)
      .then((next) => { if (active) setCount(next); })
      .catch((reason: unknown) => { if (active) setError(normalizeCommandError(reason)); });
    return () => { active = false; };
  }, [fuzzy, open, scopePaths]);

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

  async function start() {
    setError(null);
    try {
      await translationStart(scopePaths, fuzzy, preferences.translationModel, preferences.translationEffort || null);
      await readStatus();
    } catch (reason) {
      setError(normalizeCommandError(reason));
    }
  }

  const running = status?.running ?? false;
  const noModel = preferences.translationModel === "";
  const cachedShare = status && status.inputTokens > 0 ? status.cachedTokens / status.inputTokens : null;
  const scopeOptions = [
    ...(sheetName ? [{ value: "sheet" as const, label: t("translate.scope.sheet") }] : []),
    ...(folder ? [{ value: "folder" as const, label: t("translate.scope.folder", { folder }) }] : []),
    { value: "project" as const, label: t("translate.scope.project") },
  ];

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog translate-dialog" aria-describedby="translate-description">
          <Dialog.Title className="dialog-title">{t("translate.title")}</Dialog.Title>
          <p id="translate-description" className="dialog-description">{t("translate.description")}</p>
          {error ? <ErrorBanner title={t("translate.error")} error={error} onDismiss={() => setError(null)} /> : null}

          {!running ? (
            <>
              <div className="export-block">
                <span className="field-label">{t("translate.scope")}</span>
                <Segmented<Scope> label={t("translate.scope")} value={scope} onChange={setScope} options={scopeOptions} />
                {scope === "sheet" && sheetName ? <span className="field-hint mono">{sheetName}</span> : null}
              </div>
              <label className="field field-inline">
                <input type="checkbox" checked={fuzzy} onChange={(event) => setFuzzy(event.target.checked)} />
                <span>{t("translate.fuzzy")}</span>
              </label>
              <p className="field-hint">
                {count === null ? t("common.loading") : t("translate.count", { strings: formatNumber(count.strings), files: formatNumber(count.files) })}
              </p>
              {noModel ? (
                <p className="export-warning">
                  <UiIcon icon="circleAlert" size="xs" />{t("translate.noModel")}{" "}
                  <button className="link-button" type="button" onClick={onOpenSettings}>{t("translate.openSettings")}</button>
                </p>
              ) : <p className="field-hint">{t("translate.model", { model: preferences.translationModel })}</p>}
            </>
          ) : null}

          {status && (running || status.stop) ? (
            <div className="translate-progress" aria-live="polite">
              <div className="meter" aria-hidden="true">
                <span className="meter-translated" style={{ width: `${status.strings > 0 ? Math.min(1, (status.written + status.rejected) / status.strings) * 100 : 0}%` }} />
              </div>
              <p className="export-facts">
                <span>{t("translate.progress", { written: formatNumber(status.written), strings: formatNumber(status.strings) })}</span>
                {status.rejected > 0 ? <span>{t("translate.rejected", { count: status.rejected })}</span> : null}
                <span>{t("translate.tokens", { input: formatNumber(status.inputTokens), output: formatNumber(status.outputTokens) })}</span>
                {cachedShare !== null ? <span>{t("translate.cached", { percent: Math.round(cachedShare * 100) })}</span> : null}
                {running ? <span>{t("translate.pace", { pace: status.pace })}</span> : null}
              </p>
              {status.message ? <p className="field-hint">{status.message}</p> : null}
              {status.stop ? <p className="field-hint"><strong>{describeStop(status.stop, t, locale)}</strong></p> : null}
              {status.rejections.length > 0 ? (
                <details className="guide-diagnostics">
                  <summary>{t("translate.rejections", { count: status.rejections.length })}</summary>
                  <ul>
                    {status.rejections.slice(0, 50).map((entry) => (
                      <li key={entry.context}><span className="mono">{entry.context}</span>: {entry.problems.join("; ")}</li>
                    ))}
                  </ul>
                </details>
              ) : null}
            </div>
          ) : null}

          <div className="dialog-actions">
            <button className="button button-ghost" type="button" onClick={() => onOpenChange(false)}>{t(running ? "translate.hide" : "common.close")}</button>
            {running ? (
              <button className="button button-secondary" type="button" onClick={() => void translationStop()}>{t("translate.stopRun")}</button>
            ) : (
              <button className="button button-primary" type="button" disabled={noModel || count === null || count.strings === 0} onClick={() => void start()}>
                {t(status?.stop && status.stop.reason !== "finished" ? "translate.continue" : "translate.start")}
              </button>
            )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
});
