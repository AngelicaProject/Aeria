import { useEffect, useState } from "react";
import { normalizeCommandError, termUnappliedStrings, translationStop } from "../ipc";
import type { SourceBinding, TermKey, TranslationStatus, UnappliedStringDto } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";
import { describeStop } from "./TranslateDialog";
import { TranslationCorrections } from "./TranslationCorrections";
import { TranslationRejections } from "./TranslationRejections";

/** Most strings of a term listed in its fields; the run takes them all. */
const LISTED = 200;

/** Why a correction cannot start now, or null when it can. */
export type FixBlock = "noModel" | "running" | "unsaved" | null;

type FixControlsProps = {
  label: string;
  block: FixBlock;
  busy: boolean;
  onFix: (request: string | null) => void;
  onOpenSettings: () => void;
};

/** The button that corrects strings with AI, with an optional request for every translation. */
export function FixControls({ label, block, busy, onFix, onOpenSettings }: FixControlsProps) {
  const { t } = useI18n();
  const [asking, setAsking] = useState(false);
  const [request, setRequest] = useState("");
  return (
    <div className="glossary-fix-controls">
      {asking ? (
        <textarea className="input glossary-fix-request" value={request} rows={2} spellCheck placeholder={t("glossary.fix.requestPlaceholder")} aria-label={t("glossary.fix.request")} onChange={(event) => setRequest(event.target.value)} />
      ) : null}
      <div className="glossary-fix-actions">
        {block === "noModel" ? (
          <span className="glossary-fix-blocked">{t("glossary.fix.noModel")} <button className="link-button" type="button" onClick={onOpenSettings}>{t("glossary.fix.chooseModel")}</button></span>
        ) : block ? <span className="glossary-fix-blocked">{t(block === "running" ? "glossary.fix.busy" : "glossary.fix.unsaved")}</span> : null}
        {asking ? null : <button className="link-button" type="button" onClick={() => setAsking(true)}>{t("glossary.fix.request")}</button>}
        <button className="button button-primary" type="button" disabled={block !== null || busy} onClick={() => onFix(request.trim() || null)}>
          <UiIcon icon="sparkles" size="sm" />{label}
        </button>
      </div>
    </div>
  );
}

type UnappliedProps = {
  /** The meaning of the term. */
  term: TermKey;
  count: number;
  /** Changes when the strings may have changed. */
  revision: number;
  block: FixBlock;
  busy: boolean;
  onFix: (request: string | null) => void;
  onOpen: (context: string) => void;
  onOpenSettings: () => void;
};

/**
 * The translations that do not use a term, as the checks find: each opens
 * in the editor, and a correction run with AI decides them all.
 */
export function UnappliedStrings({ term, count, revision, block, busy, onFix, onOpen, onOpenSettings }: UnappliedProps) {
  const { t } = useI18n();
  const [strings, setStrings] = useState<UnappliedStringDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setStrings(null);
    setError(null);
    if (count === 0) return;
    let current = true;
    void termUnappliedStrings([term])
      .then((found) => { if (current) setStrings(found); })
      .catch((reason: unknown) => { if (current) setError(normalizeCommandError(reason).message); });
    return () => { current = false; };
    // The meaning, not the object it comes in, decides the strings.
  }, [count, revision, term.term, term.category]);
  if (count === 0) return null;
  return (
    <div className="glossary-field glossary-unapplied">
      <span className="glossary-field-label">
        {t("glossary.fix.title")}
        <span className="glossary-strings-count warning">{count.toLocaleString()}</span>
      </span>
      {error ? <p className="glossary-problem"><UiIcon icon="circleAlert" size="xs" />{error}</p> : null}
      {strings === null && !error ? <span className="muted glossary-strings-status">{t("common.loading")}</span> : null}
      {strings && strings.length > 0 ? (
        <ul className="glossary-strings">
          {strings.slice(0, LISTED).map((string) => (
            <li key={`${string.path}:${string.context}`}>
              <button type="button" className="glossary-string" title={string.context} onClick={() => onOpen(string.context)}>
                <span className="glossary-string-sheet">{string.context.split(":")[0]}</span>
                <span className="glossary-string-source">{string.source}</span>
                <span className="glossary-string-translation">{string.translation}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      <FixControls label={t("glossary.fix.run")} block={block} busy={busy} onFix={onFix} onOpenSettings={onOpenSettings} />
    </div>
  );
}

type RunPanelProps = {
  status: TranslationStatus;
  /** The terms the run decides, by name. */
  terms: readonly string[];
  onReveal: (binding: SourceBinding) => void;
  onClose: () => void;
};

/** A correction run the glossary started: its progress, then what it wrote, decided, and refused. */
export function RunPanel({ status, terms, onReveal, onClose }: RunPanelProps) {
  const { t, locale, formatNumber } = useI18n();
  const running = status.running;
  const done = status.written + status.unchanged + status.rejected;
  return (
    <section className="glossary-run" aria-live="polite" aria-label={t("glossary.fix.panel")}>
      <div className="glossary-run-head">
        <UiIcon icon="sparkles" size="sm" />
        <span className="glossary-run-title">{t("glossary.fix.panel")}</span>
        <span className="muted glossary-run-terms">{terms.length > 0 ? terms.join(", ") : t("glossary.fix.allTerms")}</span>
        {running
          ? <button className="button button-secondary" type="button" onClick={() => void translationStop()}>{t("translate.stopRun")}</button>
          : <button className="button button-ghost" type="button" onClick={onClose}>{t("glossary.fix.close")}</button>}
      </div>
      <div className="meter" aria-hidden="true">
        <span className="meter-translated" style={{ width: `${status.strings > 0 ? Math.min(1, done / status.strings) * 100 : running ? 0 : 100}%` }} />
      </div>
      <p className="export-facts">
        <span>{t("translate.progressFix", { written: formatNumber(status.written), strings: formatNumber(status.strings) })}</span>
        {status.unchanged > 0 ? <span>{t("translate.unchanged", { count: status.unchanged })}</span> : null}
        {status.decided > 0 ? <span>{t("translate.decided", { count: status.decided })}</span> : null}
        {status.rejected > 0 ? <span>{t("translate.rejected", { count: status.rejected })}</span> : null}
      </p>
      {status.message ? <p className="field-hint">{status.message}</p> : null}
      {status.stop ? <p className="field-hint"><strong>{describeStop(status.stop, t, locale)}</strong></p> : null}
      {status.corrections.length > 0 ? <TranslationCorrections corrections={status.corrections} total={status.written} onReveal={onReveal} /> : null}
      {status.rejections.length > 0 ? <TranslationRejections rejections={status.rejections} total={status.rejected} running={running} onReveal={onReveal} onRetry={undefined} /> : null}
    </section>
  );
}
