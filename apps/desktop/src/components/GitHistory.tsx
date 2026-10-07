import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { gitCommitChanges, gitLog, gitUndoLastCommit, normalizeCommandError } from "../ipc";
import type { CommandError, GitCommitChangesDto, GitCommitDto, SourceBinding } from "../types";
import { layoutGraph, type GraphRow } from "../commitGraph";
import { formatRelativeTime } from "../timeDisplay";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";
import { CommitMenu, useCommitActions, type CommitAction } from "./GitCommitActions";
import { FileChangeList } from "./GitFileChanges";

const PAGE = 100;
const ROW_HEIGHT = 26;
const LANE_WIDTH = 12;
const LANE_COLORS = 6;

/** One row of the commit graph, drawn on a canvas in the theme's lane colors. */
function GraphCell({ row }: { row: GraphRow }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const width = Math.max(row.width, 1) * LANE_WIDTH;
  useLayoutEffect(() => {
    const element = canvas.current;
    const context = element?.getContext("2d");
    if (!element || !context) return;
    const scale = window.devicePixelRatio || 1;
    element.width = width * scale;
    element.height = ROW_HEIGHT * scale;
    context.setTransform(scale, 0, 0, scale, 0, 0);
    const style = getComputedStyle(element);
    const color = (lane: number) => style.getPropertyValue(`--graph-lane-${lane % LANE_COLORS}`).trim() || style.color;
    const x = (lane: number) => lane * LANE_WIDTH + LANE_WIDTH / 2;
    const mid = ROW_HEIGHT / 2;
    context.lineWidth = 1.5;
    const stroke = (lane: number, draw: () => void) => {
      context.strokeStyle = color(lane);
      context.beginPath();
      draw();
      context.stroke();
    };
    for (const lane of row.through) stroke(lane, () => { context.moveTo(x(lane), 0); context.lineTo(x(lane), ROW_HEIGHT); });
    for (const lane of row.incoming) stroke(lane, () => { context.moveTo(x(lane), 0); context.bezierCurveTo(x(lane), mid, x(row.column), mid / 2, x(row.column), mid); });
    for (const lane of row.outgoing) stroke(lane, () => { context.moveTo(x(row.column), mid); context.bezierCurveTo(x(row.column), mid * 1.5, x(lane), mid, x(lane), ROW_HEIGHT); });
    context.fillStyle = color(row.column);
    context.beginPath();
    context.arc(x(row.column), mid, 3.5, 0, Math.PI * 2);
    context.fill();
  }, [row, width]);
  return <canvas ref={canvas} className="graph-cell" style={{ width, height: ROW_HEIGHT }} aria-hidden="true" />;
}

export function RefChips({ refs }: { refs: readonly string[] }) {
  return <>
    {refs.map((name) => {
      const tag = name.startsWith("tag: ");
      const head = name.startsWith("HEAD -> ") || name === "HEAD";
      const label = tag ? name.slice(5) : head ? name.replace("HEAD -> ", "") : name;
      return <span key={name} className={`graph-ref${tag ? " tag" : head ? " head" : name.includes("/") ? " remote" : ""}`}>{tag ? <UiIcon icon="circleDot" size="xs" /> : <UiIcon icon="gitBranch" size="xs" />}{label}</span>;
    })}
  </>;
}

type GitHistoryListProps = {
  /** Bumps when history may have changed. */
  revision: number;
  selectedCommitId: string | null;
  onOpenCommit: (commit: GitCommitDto) => void;
  /** Narrows history to commits whose subject, author, or ID contains it. */
  query?: string | undefined;
  /** Narrows history to commits that changed this project path. */
  path?: string | null | undefined;
  /** The remote the branch syncs with, which commits not pushed yet are missing from. */
  remote?: string | null | undefined;
  /** The right-click actions of a commit. */
  actionsFor?: ((commit: GitCommitDto) => CommitAction[]) | undefined;
};

/** The project's commits with a lane graph, loading older pages while scrolling. */
export const GitHistoryList = memo(function GitHistoryList({ revision, selectedCommitId, onOpenCommit, query = "", path = null, remote = null, actionsFor }: GitHistoryListProps) {
  const { t, locale } = useI18n();
  const [commits, setCommits] = useState<GitCommitDto[]>([]);
  const [complete, setComplete] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);
  const fetching = useRef(false);
  const count = useRef(0);

  const load = useCallback(async (reset: boolean) => {
    if (fetching.current) return;
    fetching.current = true;
    setLoading(true);
    try {
      // A reload keeps as many commits as were shown, so the scroll holds.
      const limit = reset ? Math.max(PAGE, count.current) : PAGE;
      const page = await gitLog(reset ? 0 : count.current, limit, query.trim() || null, path);
      count.current = reset ? page.length : count.current + page.length;
      setCommits((current) => (reset ? page : [...current, ...page]));
      setComplete(page.length < limit);
      setError(null);
    } catch (caught) {
      setError(normalizeCommandError(caught));
    } finally {
      fetching.current = false;
      setLoading(false);
    }
  }, [query, path]);

  useEffect(() => { count.current = 0; }, [query, path]);
  useEffect(() => { void load(true); }, [load, revision]);

  // A narrowed history leaves out commits, so its graph would not connect.
  const narrowed = query.trim() !== "" || path !== null;
  const rows = useMemo(() => layoutGraph(commits), [commits]);
  const relative = (seconds: number) => formatRelativeTime(seconds * 1000, Date.now(), locale, t("time.justNow"));
  return (
    <div
      className="history-list"
      role="listbox"
      aria-label={t("git.history")}
      onScroll={(event) => {
        const element = event.currentTarget;
        if (!complete && element.scrollTop + element.clientHeight > element.scrollHeight - ROW_HEIGHT * 10) void load(false);
      }}
    >
      {error ? <div className="git-feedback error" role="alert"><UiIcon icon="circleAlert" size="sm" /><span>{error.message}</span></div> : null}
      {commits.map((commit, index) => {
        const row = (
          <button
            key={commit.id}
            type="button"
            role="option"
            aria-selected={selectedCommitId === commit.id}
            className={`history-row${selectedCommitId === commit.id ? " selected" : ""}${narrowed ? " flat" : ""}`}
            title={`${commit.subject}\n${commit.authorName} · ${new Date(commit.authoredAt * 1000).toLocaleString(locale)}\n${commit.id.slice(0, 8)}`}
            onClick={() => onOpenCommit(commit)}
          >
            {narrowed ? <span className="history-id mono">{commit.id.slice(0, 7)}</span> : <GraphCell row={rows[index]!} />}
            <span className="history-subject">
              {commit.unpublished ? <span className="history-unpublished" title={t("git.history.unpublished", { remote: remote ?? "" })} aria-label={t("git.history.unpublished", { remote: remote ?? "" })}><UiIcon icon="arrowUp" size="xs" /></span> : null}
              <RefChips refs={commit.refs} />{commit.subject}
            </span>
            <span className="history-time">{relative(commit.authoredAt)}</span>
          </button>
        );
        return actionsFor ? <CommitMenu key={commit.id} actions={actionsFor(commit)}>{row}</CommitMenu> : row;
      })}
      {loading && commits.length === 0 ? <div className="panel-state"><span className="spinner" />{t("common.loading")}</div> : null}
      {!loading && commits.length === 0 && !error ? <p className="muted history-empty">{t(narrowed ? "git.history.noMatches" : "git.noCommits")}</p> : null}
    </div>
  );
});

/** One commit in a document tab: message, author, its actions, and every change. */
export const CommitView = memo(function CommitView({ commitId, onRevealBinding, onWorkspaceChanged }: { commitId: string; onRevealBinding?: ((binding: SourceBinding) => void) | undefined; onWorkspaceChanged?: (() => void) | undefined }) {
  const { t, locale } = useI18n();
  const [commit, setCommit] = useState<GitCommitChangesDto | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const commitActions = useCommitActions(() => onWorkspaceChanged?.());
  const stringsOf = useCallback((path: string) => Promise.resolve(commit?.changes.filter((change) => change.path === path) ?? []), [commit]);
  useEffect(() => {
    let cancelled = false;
    setCommit(null);
    gitCommitChanges(commitId)
      .then((next) => { if (!cancelled) setCommit(next); })
      .catch((caught: unknown) => { if (!cancelled) setError(normalizeCommandError(caught)); });
    return () => { cancelled = true; };
  }, [commitId]);

  if (error) return <div className="document-empty empty-state"><UiIcon icon="circleAlert" size="xl" /><p>{error.message}</p></div>;
  if (!commit) return <div className="document-empty empty-state"><span className="spinner" /></div>;
  const merge = commit.commit.parents.length > 1;
  return (
    <div className="commit-view">
      <div className="history-detail">
        <h3 className="history-detail-title">{commit.commit.subject}</h3>
        <p className="export-facts">
          <span><UiIcon icon="user" size="xs" /> {commit.commit.authorName}{commit.commit.authorEmail ? ` <${commit.commit.authorEmail}>` : ""}</span>
          <span>{new Date(commit.commit.authoredAt * 1000).toLocaleString(locale)}</span>
          <span className="mono export-selectable">{commit.commit.id}</span>
          {merge ? <span>{t("commit.merge")}</span> : null}
          {commit.local && commit.remote ? <span className="history-unpublished-fact"><UiIcon icon="arrowUp" size="xs" /> {t("git.history.unpublished", { remote: commit.remote })}</span> : null}
        </p>
        <div className="history-detail-refs"><RefChips refs={commit.commit.refs} /></div>
        <div className="commit-actions" role="toolbar" aria-label={t("commit.actions")}>
          {commitActions.actionsFor(commit.commit, {
            remote: commit.webUrl !== null ? commit.remote : null,
            undo: commit.local && !merge && commit.commit.refs.some((ref) => ref === "HEAD" || ref.startsWith("HEAD -> "))
              ? () => void gitUndoLastCommit().then(() => onWorkspaceChanged?.()).catch((caught: unknown) => setError(normalizeCommandError(caught)))
              : undefined,
          }).map((action) => (
            <button key={action.id} className={action.danger ? "button button-ghost commit-action danger" : "button button-ghost commit-action"} type="button" disabled={action.disabled ?? false} title={action.id === "open" && commit.webUrl ? commit.webUrl : undefined} onClick={action.run}>
              <UiIcon icon={action.icon} size="sm" />{t(action.label, action.params)}
            </button>
          ))}
        </div>
        {commitActions.feedback}
        {commitActions.dialogs}
        {commit.files.length === 0 ? <p className="muted">{t(merge ? "commit.mergeNoChanges" : "commit.noFiles")}</p>
          : <FileChangeList files={commit.files} stringsOf={stringsOf} selectedKey={null} onRevealBinding={onRevealBinding} viewKey={`commit:${commit.commit.id}`} />}
      </div>
    </div>
  );
});
