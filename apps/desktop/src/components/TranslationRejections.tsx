import { useState } from "react";
import { normalizeCommandError, setTranslationTermException } from "../ipc";
import { describeIssue, errorText, exceptionTerm } from "../issueText";
import type { CommandError, SourceBinding, TranslationRejected } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

type TranslationRejectionsProps = {
  rejections: readonly TranslationRejected[];
  /** Every string rejected by the run; more than listed when the list was cut. */
  total: number;
  running: boolean;
  onReveal?: ((binding: SourceBinding) => void) | undefined;
  /** Translates the listed strings again. */
  onRetry: (contexts: string[]) => void;
};

/**
 * The strings whose translation the checks rejected: each with its problems
 * in the interface language and the model's translation, which was not
 * written. A string opens in the editor; a term problem can become an
 * exception of the string; the strings can be translated again.
 */
export function TranslationRejections({ rejections, total, running, onReveal, onRetry }: TranslationRejectionsProps) {
  const { t } = useI18n();
  const [excepted, setExcepted] = useState<ReadonlySet<string>>(new Set());
  const [error, setError] = useState<CommandError | null>(null);

  const except = async (rejected: TranslationRejected, term: string) => {
    if (!rejected.binding) return;
    setError(null);
    try {
      await setTranslationTermException(rejected.binding, term, true);
      setExcepted((current) => new Set(current).add(`${rejected.context}|${term}`));
    } catch (caught) {
      setError(normalizeCommandError(caught));
    }
  };

  return (
    <details className="translate-rejections" open>
      <summary>{t("translate.rejections", { count: total })}</summary>
      {error ? <p className="translate-rejection-error">{errorText(error, t)}</p> : null}
      <ul>
        {rejections.map((rejected) => (
          <li key={rejected.context} className="translate-rejection">
            <button className="link-button mono" type="button" disabled={!rejected.binding} onClick={() => { if (rejected.binding) onReveal?.(rejected.binding); }}>
              {rejected.binding ? `${rejected.binding.sheetName} ${rejected.binding.rowId}:${rejected.binding.subrowId} · ${rejected.binding.columnIndex}` : rejected.context}
            </button>
            {rejected.problems.map((issue, index) => {
              const term = exceptionTerm(issue);
              const done = term !== null && excepted.has(`${rejected.context}|${term}`);
              return (
                <p key={`${issue.group}:${index}`} className="translate-rejection-problem">
                  <UiIcon icon="circleAlert" size="xs" />
                  <span>{describeIssue(issue, t)}</span>
                  {term !== null && rejected.binding ? (
                    done ? <span className="muted">{t("translate.excepted")}</span> : (
                      <button className="link-button" type="button" title={t("findings.exceptTitle", { term })} onClick={() => void except(rejected, term)}>
                        <UiIcon icon="bookX" size="xs" />{t("findings.except")}
                      </button>
                    )
                  ) : null}
                </p>
              );
            })}
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
      <button className="button button-secondary" type="button" disabled={running || rejections.length === 0} onClick={() => onRetry(rejections.map((rejected) => rejected.context))}>
        <UiIcon icon="sparkles" size="sm" />{t("translate.retryRejected", { count: rejections.length })}
      </button>
    </details>
  );
}
