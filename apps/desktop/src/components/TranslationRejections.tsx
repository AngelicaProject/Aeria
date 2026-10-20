import { describeIssue } from "../issueText";
import type { SourceBinding, TranslationRejected } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

type TranslationRejectionsProps = {
  rejections: readonly TranslationRejected[];
  /** Every string rejected by the run; more than listed when the list was cut. */
  total: number;
  running: boolean;
  onReveal?: ((binding: SourceBinding) => void) | undefined;
  /** Translates the listed strings again; a correction's refused strings keep their translation, so it has none. */
  onRetry?: ((contexts: string[]) => void) | undefined;
};

/**
 * The strings whose translation the checks rejected: each with its problems
 * in the interface language and the model's translation, which was not
 * written. A string opens in the editor; the strings can be translated
 * again.
 */
export function TranslationRejections({ rejections, total, running, onReveal, onRetry }: TranslationRejectionsProps) {
  const { t } = useI18n();

  return (
    <details className="translate-rejections" open>
      <summary>{t("translate.rejections", { count: total })}</summary>
      <ul>
        {rejections.map((rejected) => (
          <li key={rejected.context} className="translate-rejection">
            <button className="link-button mono" type="button" disabled={!rejected.binding} onClick={() => { if (rejected.binding) onReveal?.(rejected.binding); }}>
              {rejected.binding ? `${rejected.binding.sheetName} ${rejected.binding.rowId}:${rejected.binding.subrowId} · ${rejected.binding.columnIndex}` : rejected.context}
            </button>
            {rejected.problems.map((issue, index) => (
              <p key={`${issue.group}:${index}`} className="translate-rejection-problem">
                <UiIcon icon="circleAlert" size="xs" />
                <span>{describeIssue(issue, t)}</span>
              </p>
            ))}
            {rejected.translation ? (
              <details className="translate-rejection-text">
                <summary>{t("translate.rejectedText")}</summary>
                <p>{rejected.translation}</p>
              </details>
            ) : null}
          </li>
        ))}
      </ul>
      {total > rejections.length ? <p className="field-hint">{t("translate.rejectionsCut", { shown: rejections.length, count: total })}</p> : null}
      {onRetry ? (
        <button className="button button-secondary" type="button" disabled={running || rejections.length === 0} onClick={() => onRetry(rejections.map((rejected) => rejected.context))}>
          <UiIcon icon="sparkles" size="sm" />{t("translate.retryRejected", { count: rejections.length })}
        </button>
      ) : null}
    </details>
  );
}
