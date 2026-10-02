import { memo, useCallback, useEffect, useState } from "react";
import { normalizeCommandError, setTranslationTermException, translationFindings } from "../ipc";
import { describeIssue, errorText, exceptionTerm } from "../issueText";
import type { CommandError, SourceBinding, TranslationFindingsDto } from "../types";
import { useI18n } from "../ui/i18n";
import { IconButton } from "../ui/primitives/IconButton";
import { UiIcon } from "../ui/primitives/UiIcon";

type StringFindingsProps = {
  /** The selected string; null when none is selected. */
  binding: SourceBinding | null;
  /** Bumps when the string may have changed (save, checkpoint, sync). */
  revision: number;
  /** A term exception was written: the editor reads the string again. */
  onChanged?: (() => void) | undefined;
};

/**
 * What the checks find in the saved translation of a string: problems, which
 * keep it out of the pack, and advice. A term that does not apply to this
 * string can be made an exception, and an exception removed.
 */
export const StringFindings = memo(function StringFindings({ binding, revision, onChanged }: StringFindingsProps) {
  const { t } = useI18n();
  const [findings, setFindings] = useState<TranslationFindingsDto | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [busy, setBusy] = useState(false);
  const [own, setOwn] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setFindings(null);
    setError(null);
    if (!binding) return;
    translationFindings(binding)
      .then((next) => { if (!cancelled) setFindings(next); })
      .catch((caught: unknown) => { if (!cancelled) setError(normalizeCommandError(caught)); });
    return () => { cancelled = true; };
  }, [binding, revision, own]);

  const setException = useCallback(async (term: string, add: boolean) => {
    if (!binding) return;
    setBusy(true);
    setError(null);
    try {
      await setTranslationTermException(binding, term, add);
      setOwn((current) => current + 1);
      onChanged?.();
    } catch (caught) {
      setError(normalizeCommandError(caught));
    } finally {
      setBusy(false);
    }
  }, [binding, onChanged]);

  if (!binding) return null;
  if (error) return <p className="string-findings-empty error">{errorText(error, t)}</p>;
  if (!findings) return <p className="string-findings-empty muted">{t("common.loading")}</p>;

  return (
    <div className="string-findings">
      {findings.issues.length === 0 ? <p className="string-findings-empty muted">{t("findings.none")}</p> : (
        <ul className="string-findings-list">
          {findings.issues.map((issue, index) => {
            const term = exceptionTerm(issue);
            return (
              <li key={`${issue.group}:${index}`} className={issue.advice ? "string-finding advice" : "string-finding"}>
                <UiIcon icon={issue.advice ? "info" : "circleAlert"} size="xs" />
                <span>{describeIssue(issue, t)}</span>
                {term !== null ? (
                  <button className="link-button" type="button" disabled={busy} title={t("findings.exceptTitle", { term })} onClick={() => void setException(term, true)}>
                    <UiIcon icon="bookX" size="xs" />{t("findings.except")}
                  </button>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
      {findings.termExceptions.length > 0 ? (
        <div className="string-findings-exceptions">
          <span className="eyebrow" title={t("findings.exceptionsHint")}>{t("findings.exceptions")}</span>
          <div className="string-exceptions">
            {findings.termExceptions.map((term) => (
              <span key={term} className="chip string-exception">
                {term}
                <IconButton icon="x" size="xs" label={t("findings.removeException", { term })} disabled={busy} onClick={() => void setException(term, false)} />
              </span>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
});
