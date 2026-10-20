import { memo, useCallback, useRef } from "react";
import { diffWords } from "../textDiff";
import type { SourceBinding, TranslationCorrected } from "../types";
import { useI18n } from "../ui/i18n";

type TranslationCorrectionsProps = {
  corrections: readonly TranslationCorrected[];
  /** Every translation the run wrote; more than listed when the list was cut. */
  total: number;
  onReveal?: ((binding: SourceBinding) => void) | undefined;
};

type CorrectionProps = {
  corrected: TranslationCorrected;
  onReveal: (binding: SourceBinding) => void;
};

function sameBinding(a: SourceBinding | null, b: SourceBinding | null) {
  if (a === null || b === null) return a === b;
  return a.sheetName === b.sheetName && a.rowId === b.rowId && a.subrowId === b.subrowId && a.columnIndex === b.columnIndex;
}

// The status is read again every second with the whole list, so a correction
// already shown is drawn again only when it differs.
const Correction = memo(function Correction({ corrected, onReveal }: CorrectionProps) {
  const { t } = useI18n();
  const binding = corrected.binding;
  return (
    <li className="translate-rejection">
      <button className="link-button mono" type="button" disabled={!binding} onClick={() => { if (binding) onReveal(binding); }}>
        {binding ? `${binding.sheetName} ${binding.rowId}:${binding.subrowId} · ${binding.columnIndex}` : corrected.context}
      </button>
      {corrected.reason ? <p className="translate-correction-reason">{corrected.reason}</p> : null}
      <div className="git-diff">
        {diffWords(corrected.before, corrected.after).map((part, index) => part.kind === "same"
          ? <span key={index}>{part.text}</span>
          : part.kind === "added" ? <ins key={index}>{part.text}</ins> : <del key={index}>{part.text}</del>)}
      </div>
    </li>
  );
}, (previous, next) => previous.onReveal === next.onReveal
  && previous.corrected.context === next.corrected.context
  && previous.corrected.before === next.corrected.before
  && previous.corrected.after === next.corrected.after
  && previous.corrected.reason === next.corrected.reason
  && sameBinding(previous.corrected.binding, next.corrected.binding));

/** The translations a correction run wrote: each before and after with the changed words marked, and why it changed. */
export function TranslationCorrections({ corrections, total, onReveal }: TranslationCorrectionsProps) {
  const { t } = useI18n();
  const revealRef = useRef(onReveal);
  revealRef.current = onReveal;
  const reveal = useCallback((binding: SourceBinding) => revealRef.current?.(binding), []);
  return (
    <details className="translate-rejections translate-corrections">
      <summary>{t("translate.corrections", { count: total })}</summary>
      <ul>
        {corrections.map((corrected) => <Correction key={corrected.context} corrected={corrected} onReveal={reveal} />)}
      </ul>
      {total > corrections.length ? <p className="field-hint">{t("translate.correctionsCut", { shown: corrections.length, count: total })}</p> : null}
    </details>
  );
}
