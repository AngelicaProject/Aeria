import { diffWords } from "../textDiff";
import type { SourceBinding, TranslationCorrected } from "../types";
import { useI18n } from "../ui/i18n";

type TranslationCorrectionsProps = {
  corrections: readonly TranslationCorrected[];
  /** Every translation the run wrote; more than listed when the list was cut. */
  total: number;
  onReveal?: ((binding: SourceBinding) => void) | undefined;
};

/** The translations a correction run wrote: each before and after with the changed words marked, and why it changed. */
export function TranslationCorrections({ corrections, total, onReveal }: TranslationCorrectionsProps) {
  const { t } = useI18n();
  return (
    <details className="translate-rejections translate-corrections">
      <summary>{t("translate.corrections", { count: total })}</summary>
      <ul>
        {corrections.map((corrected) => (
          <li key={corrected.context} className="translate-rejection">
            <button className="link-button mono" type="button" disabled={!corrected.binding} onClick={() => { if (corrected.binding) onReveal?.(corrected.binding); }}>
              {corrected.binding ? `${corrected.binding.sheetName} ${corrected.binding.rowId}:${corrected.binding.subrowId} · ${corrected.binding.columnIndex}` : corrected.context}
            </button>
            {corrected.reason ? <p className="translate-correction-reason">{corrected.reason}</p> : null}
            <div className="git-diff">
              {diffWords(corrected.before, corrected.after).map((part, index) => part.kind === "same"
                ? <span key={index}>{part.text}</span>
                : part.kind === "added" ? <ins key={index}>{part.text}</ins> : <del key={index}>{part.text}</del>)}
            </div>
          </li>
        ))}
      </ul>
      {total > corrections.length ? <p className="field-hint">{t("translate.correctionsCut", { shown: corrections.length, count: total })}</p> : null}
    </details>
  );
}
