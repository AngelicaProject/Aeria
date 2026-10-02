import { memo, useEffect, useState } from "react";
import { gitStringHistory, normalizeCommandError } from "../ipc";
import type { CommandError, SourceBinding, StringHistoryDto } from "../types";
import { formatRelativeTime } from "../timeDisplay";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";
import { changeLabel } from "./GitShared";

const HISTORY_LIMIT = 50;

type StringHistoryProps = {
  /** The selected string; null when none is selected. */
  binding: SourceBinding | null;
  /** Bumps when the string or the repository may have changed. */
  revision: number;
  /** Puts a historical text into the editor as an unsaved draft. */
  onUseText?: ((target: string) => void) | undefined;
};

/** Every committed change to a string, newest first, and who made it. */
export const StringHistory = memo(function StringHistory({ binding, revision, onUseText }: StringHistoryProps) {
  const { t, locale } = useI18n();
  const [history, setHistory] = useState<StringHistoryDto | null>(null);
  const [error, setError] = useState<CommandError | null>(null);

  useEffect(() => {
    let cancelled = false;
    setHistory(null);
    setError(null);
    if (!binding) return;
    gitStringHistory(binding, HISTORY_LIMIT)
      .then((next) => { if (!cancelled) setHistory(next); })
      .catch((caught: unknown) => { if (!cancelled) setError(normalizeCommandError(caught)); });
    return () => { cancelled = true; };
  }, [binding, revision]);

  if (!binding) return <p className="string-history-empty muted">{t("history.untranslated")}</p>;
  if (error) return <p className="string-history-empty muted">{error.code === "gitNotRepository" ? t("history.noRepository") : error.message}</p>;
  if (!history) return <p className="string-history-empty muted">{t("common.loading")}</p>;

  const relative = (seconds: number) => formatRelativeTime(seconds * 1000, Date.now(), locale, t("time.justNow"));
  return (
    <div className="string-history">
      <ol className="git-timeline">
        {history.pending ? <li className="git-timeline-item pending"><div className="git-item-head"><strong>{t("git.uncommitted")}</strong><span className="chip">{changeLabel(history.pending, t)}</span></div></li> : null}
        {history.revisions.map((revisionEntry) => {
          const target = revisionEntry.after.targetMacro;
          return (
            <li className="git-timeline-item" key={revisionEntry.commit.id}>
              <div className="git-item-head">
                <strong>{revisionEntry.commit.authorName}</strong>
                <span className="muted" title={new Date(revisionEntry.commit.authoredAt * 1000).toLocaleString(locale)}>{relative(revisionEntry.commit.authoredAt)}</span>
                <span className="spacer" />
                <span className="chip">{changeLabel(revisionEntry, t)}</span>
              </div>
              <span className="git-item-subject">{revisionEntry.commit.subject}</span>
              <div className="git-diff"><ins>{target || t("common.empty")}</ins></div>
              {target && onUseText ? <button className="link-button" type="button" onClick={() => onUseText(target)}><UiIcon icon="undo" size="xs" />{t("history.useText")}</button> : null}
            </li>
          );
        })}
        {history.revisions.length === 0 && !history.pending ? <li className="muted">{t("git.noHistory")}</li> : null}
        {history.truncated ? <li className="muted">{t("git.historyTruncated")}</li> : null}
      </ol>
    </div>
  );
});
