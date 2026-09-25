import { memo, useEffect, useState } from "react";
import { gitUnitHistory, normalizeCommandError } from "../ipc";
import type { CommandError, RecordVersionDto, UnitHistoryDto } from "../types";
import { formatRelativeTime } from "../timeDisplay";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";
import { changeLabel, kindLabel } from "./GitShared";

const HISTORY_LIMIT = 50;

type StringHistoryProps = {
  /** Translation unit of the selected field; null before it is translated. */
  unitId: string | null;
  /** Bumps when the string or the repository may have changed. */
  revision: number;
  /** Puts a historical text into the editor as an unsaved draft. */
  onUseText?: ((target: string) => void) | undefined;
};

function versionTarget(version: RecordVersionDto): string | null {
  return version.state === "valid" ? version.unit.targetMacro : null;
}

/** Who translated and reviewed a string, and every committed change to it. */
export const StringHistory = memo(function StringHistory({ unitId, revision, onUseText }: StringHistoryProps) {
  const { t, locale } = useI18n();
  const [history, setHistory] = useState<UnitHistoryDto | null>(null);
  const [error, setError] = useState<CommandError | null>(null);

  useEffect(() => {
    let cancelled = false;
    setHistory(null);
    setError(null);
    if (!unitId) return;
    gitUnitHistory(unitId, HISTORY_LIMIT)
      .then((next) => { if (!cancelled) setHistory(next); })
      .catch((caught: unknown) => { if (!cancelled) setError(normalizeCommandError(caught)); });
    return () => { cancelled = true; };
  }, [unitId, revision]);

  if (!unitId) return <p className="string-history-empty muted">{t("history.untranslated")}</p>;
  if (error) return <p className="string-history-empty muted">{error.code === "gitNotRepository" ? t("history.noRepository") : error.message}</p>;
  if (!history) return <p className="string-history-empty muted">{t("common.loading")}</p>;

  const relative = (seconds: number) => formatRelativeTime(seconds * 1000, Date.now(), locale, t("time.justNow"));
  return (
    <div className="string-history">
      {history.translatedBy || history.reviewedBy ? (
        <p className="git-attribution">
          {history.translatedBy ? <span><UiIcon icon="user" size="xs" />{t("git.translatedBy")} <strong>{history.translatedBy.authorName}</strong></span> : null}
          {history.reviewedBy ? <span><UiIcon icon="circleCheck" size="xs" />{t("git.reviewedBy")} <strong>{history.reviewedBy.authorName}</strong></span> : null}
        </p>
      ) : null}
      <ol className="git-timeline">
        {history.pending ? <li className="git-timeline-item pending"><div className="git-item-head"><strong>{t("git.uncommitted")}</strong><span className="chip">{changeLabel(history.pending, t)}</span></div></li> : null}
        {history.revisions.map((revisionEntry) => {
          const target = versionTarget(revisionEntry.after);
          return (
            <li className="git-timeline-item" key={revisionEntry.commit.id}>
              <div className="git-item-head">
                <strong>{revisionEntry.commit.authorName}</strong>
                <span className="muted" title={new Date(revisionEntry.commit.authoredAt * 1000).toLocaleString(locale)}>{relative(revisionEntry.commit.authoredAt)}</span>
                <span className="spacer" />
                <span className="chip">{t(kindLabel[revisionEntry.kind])}</span>
              </div>
              <span className="git-item-subject">{revisionEntry.commit.subject}</span>
              {revisionEntry.after.state === "invalid" ? <span className="git-feedback error">{t("git.invalidRecord", { message: revisionEntry.after.message })}</span> : null}
              {target !== null ? <div className="git-diff"><ins>{target || t("common.empty")}</ins></div> : null}
              {target !== null && onUseText ? <button className="link-button" type="button" onClick={() => onUseText(target)}><UiIcon icon="undo" size="xs" />{t("history.useText")}</button> : null}
            </li>
          );
        })}
        {history.revisions.length === 0 && !history.pending ? <li className="muted">{t("git.noHistory")}</li> : null}
        {history.truncated ? <li className="muted">{t("git.historyTruncated")}</li> : null}
      </ol>
    </div>
  );
});
