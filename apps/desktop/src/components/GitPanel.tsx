import { useCallback, useEffect, useRef, useState } from "react";
import {
  gitBranches,
  gitChangedFiles,
  gitCommit,
  gitCreateBranch,
  gitDiscard,
  gitFetch,
  gitInitialize,
  gitGuardWorkflow,
  gitOpenPullRequest,
  gitOverview,
  gitPendingFileChanges,
  gitPull,
  gitPush,
  gitSetIdentity,
  gitStage,
  gitStateStamp,
  gitSwitchBranch,
  gitUndoLastCommit,
  gitUnstage,
  normalizeCommandError,
} from "../ipc";
import type {
  SourceBinding,
  CommandError,
  ConflictResolution,
  GitBranchDto,
  GitCommitDto,
  GitPullDto,
  GuardWorkflowDto,
  EntryConflictDto,
  FileChangeDto,
  GitOverviewDto,
  PendingChangesDto,
  WorkingChangesDto,
} from "../types";
import { IconButton } from "../ui/primitives/IconButton";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";
import { ConfirmDialog } from "./ConfirmDialog";
import { GitBranchDelete } from "./GitBranchDelete";
import { GitBranchPicker } from "./GitBranchPicker";
import { useCommitActions } from "./GitCommitActions";
import { GitHistoryList } from "./GitHistory";
import { FileChangeList, hasCommittable, type FileActions } from "./GitFileChanges";
import { Section, bindingLabel, useStickyState } from "./GitShared";
import { RepositoryGuard } from "./RepositoryGuard";

/** How often the dock checks whether the repository changed. */
const POLL_MS = 2000;
/** How long typing in the history search waits before searching. */
const SEARCH_DELAY_MS = 250;

const NO_CHANGES: WorkingChangesDto = { staged: [], changes: [] };

type GitTab = "changes" | "history";

type GitPanelProps = {
  /** The key of the selected string (see `changeKey`), when one is selected. */
  selectedKey: string | null;
  /** Changes whenever the editor persisted a translation. */
  workspaceRevision: number;
  /** Changes whenever project settings, glossary, or guidance may have been saved. */
  projectRevision?: number | undefined;
  onWorkspaceChanged?: (() => void) | undefined;
  /** Pending changes owned by the workbench; detached windows fetch their own. */
  pending?: { summary: PendingChangesDto | null; refresh: () => Promise<void> } | undefined;
  /** Opens a string in the editor; absent in detached windows. */
  onRevealBinding?: ((binding: SourceBinding) => void) | undefined;
  /** Opens one commit in a document tab; absent in detached windows. */
  onOpenCommit?: ((commit: GitCommitDto) => void) | undefined;
  selectedCommitId?: string | null | undefined;
  /** Opens Settings on the repository section. */
  onOpenSettings?: (() => void) | undefined;
};

const pullLabels: Record<GitPullDto["integration"], MessageKey> = {
  upToDate: "git.pull.upToDate",
  fastForward: "git.pull.fastForward",
  merged: "git.pull.merged",
};

const blockLabels: Record<NonNullable<GitBranchDto["blocked"]>, MessageKey> = {
  noProject: "git.blocked.noProject",
  olderFormat: "git.blocked.olderFormat",
  otherSource: "git.blocked.otherSource",
};

/** Branch and sync above, then the uncommitted changes with the commit composer, or the history. It follows the repository on its own. */
export function GitPanel({ selectedKey, workspaceRevision, projectRevision, onWorkspaceChanged, pending: externalPending, onRevealBinding, onOpenCommit, selectedCommitId, onOpenSettings }: GitPanelProps) {
  const { t } = useI18n();
  const [overview, setOverview] = useState<GitOverviewDto | null>(null);
  const [branches, setBranches] = useState<GitBranchDto[]>([]);
  const [working, setWorking] = useState<WorkingChangesDto>(NO_CHANGES);
  // Changes with every refresh, so open files read their strings again.
  const [filesRevision, setFilesRevision] = useState(0);
  const [historyRevision, setHistoryRevision] = useState(0);
  // Kept while the window lives, so reopening the Git tab keeps it as it was left.
  const [tab, setTab] = useStickyState<GitTab>("git.tab", "changes");
  const [historyQuery, setHistoryQuery] = useStickyState("git.historyQuery", "");
  const [searched, setSearched] = useState(historyQuery);
  const [historyPath, setHistoryPath] = useStickyState<string | null>("git.historyPath", null);
  const [discarding, setDiscarding] = useState<readonly FileChangeDto[] | null>(null);
  // The next commit replaces the last one (`git commit --amend`).
  const [amend, setAmend] = useState(false);
  const [message, setMessage] = useState("");
  const [identityDraft, setIdentityDraft] = useState<{ name: string; email: string; global: boolean } | null>(null);
  const [conflicts, setConflicts] = useState<EntryConflictDto[]>([]);
  const [choices, setChoices] = useState<Record<string, ConflictResolution>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<GitBranchDto | null>(null);
  const [guardWorkflow, setGuardWorkflow] = useState<GuardWorkflowDto | null>(null);
  const stamp = useRef<string | null | undefined>(undefined);

  const identity = overview?.identity ?? null;
  // Git requires only a name; the email is optional (see aeria-git identity rules).
  const identityComplete = Boolean(identity?.name);

  const refresh = useCallback(async () => {
    try {
      const [next, nextStamp] = await Promise.all([gitOverview(), gitStateStamp()]);
      stamp.current = nextStamp;
      setOverview(next);
      if (next.repository) {
        if (externalPending) await externalPending.refresh();
        setWorking(await gitChangedFiles());
        setFilesRevision((current) => current + 1);
        setBranches(await gitBranches());
        // Optional: a failure only hides the guard and pull requests.
        setGuardWorkflow(await gitGuardWorkflow().catch(() => null));
      } else {
        setWorking(NO_CHANGES);
      }
      setHistoryRevision((current) => current + 1);
      setError(null);
    } catch (caught) {
      setError(normalizeCommandError(caught));
    }
    // Depend on the stable refresh callback, not the pending object that changes with every fetch.
  }, [externalPending?.refresh]);

  useEffect(() => {
    void refresh();
  }, [refresh, workspaceRevision, projectRevision]);

  // Follow the repository without a refresh button: poll a cheap fingerprint
  // of `git status` while the window is visible, and when it gains focus.
  useEffect(() => {
    let cancelled = false;
    const check = async () => {
      if (document.visibilityState !== "visible" || busy !== null) return;
      try {
        const next = await gitStateStamp();
        if (!cancelled && stamp.current !== undefined && next !== stamp.current) await refresh();
      } catch {
        // A transient failure is reported by the next full refresh.
      }
    };
    const timer = window.setInterval(() => void check(), POLL_MS);
    const onFocus = () => void check();
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.removeEventListener("focus", onFocus);
    };
  }, [busy, refresh]);

  const run = useCallback(async (label: string, action: () => Promise<string | null | void>) => {
    setBusy(label);
    setError(null);
    setNotice(null);
    try {
      const result = await action();
      if (typeof result === "string") setNotice(result);
      await refresh();
    } catch (caught) {
      setError(normalizeCommandError(caught));
    } finally {
      setBusy(null);
    }
  }, [refresh]);

  useEffect(() => {
    const timer = window.setTimeout(() => setSearched(historyQuery), SEARCH_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [historyQuery]);

  const commitActions = useCommitActions(() => { onWorkspaceChanged?.(); void refresh(); });

  const describePull = (result: GitPullDto): string => {
    if (result.conflicts.length > 0) {
      setConflicts(result.conflicts);
      setChoices({});
      return t("git.conflicts", { count: result.conflicts.length });
    }
    setConflicts([]);
    if (result.workspaceChanged) onWorkspaceChanged?.();
    return t(pullLabels[result.integration]);
  };

  const feedback = <>
    {error ? <div className="git-feedback error" role="alert"><UiIcon icon="circleAlert" size="sm" /><span>{error.code === "gitBranchProtected" && overview?.repository?.branch
      ? t("git.branchProtected", { remote: overview.repository.upstream?.split("/")[0] ?? "origin", branch: overview.repository.branch })
      : error.message}</span></div> : null}
    {notice ? <div className="git-feedback" role="status"><UiIcon icon="info" size="sm" /><span>{notice}</span></div> : null}
  </>;

  if (!overview) {
    return <div className="git-panel">{error ? feedback : <div className="panel-state"><span className="spinner" />{t("git.loading")}</div>}</div>;
  }

  if (!overview.repository) {
    return (
      <div className="git-panel">
        <div className="empty-state git-empty">
          <UiIcon icon="gitBranch" size="xl" />
          <strong>{t("git.noRepository")}</strong>
          <p>{t("git.noRepositoryHint")}</p>
          {overview.runtime.version ? null : <p className="git-feedback error">{t("git.unavailable")}</p>}
          <button className="button button-primary" type="button" disabled={busy !== null} onClick={() => void run("init", async () => { await gitInitialize(); return t("git.initialized"); })}>{t("git.initialize")}</button>
          {feedback}
        </div>
      </div>
    );
  }

  const status = overview.repository;
  const hasRemote = overview.remotes.length > 0;
  const { staged, changes } = working;
  const committable = hasCommittable(changes, staged);
  const changeCount = new Set([...staged, ...changes].map((file) => file.path)).size;
  const paths = (files: readonly FileChangeDto[]) => files.map((file) => file.path);
  const fileActions: FileActions = {
    disabled: busy !== null,
    stage: (files) => void run("stage", async () => { await gitStage(paths(files)); }),
    unstage: (files) => void run("unstage", async () => { await gitUnstage(paths(files)); }),
    discard: (files) => setDiscarding(files),
  };
  const discardMessage = (files: readonly FileChangeDto[]): string => {
    const untracked = files.filter((file) => file.kind === "untracked").length;
    if (files.length === 1) return t(untracked ? "git.discard.deleteOne" : "git.discard.one", { file: files[0]!.path });
    return untracked > 0
      ? t("git.discard.manyWithDeleted", { count: files.length, deleted: untracked })
      : t("git.discard.many", { count: files.length });
  };
  // The remote the branch syncs with, as Pull and Push choose it.
  const upstreamRemote = status.upstream?.split("/")[0] ?? null;
  const syncRemote = overview.remotes.find((remote) => remote.name === upstreamRemote)?.name
    ?? overview.remotes.find((remote) => remote.name === "origin")?.name
    ?? (overview.remotes.length === 1 ? overview.remotes[0]!.name : null);
  // The last commit can be amended or undone only while origin does not have it.
  const lastPublished = status.upstream !== null && status.ahead === 0;
  const amendable = status.head !== null && !lastPublished;
  const canCommit = identityComplete && busy === null && (amend ? amendable && (committable || message.trim() !== "") : committable);
  const commit = () => {
    const text = message.trim();
    const amending = amend;
    void run("commit", async () => {
      const result = await gitCommit(text === "" ? null : text, amending);
      setMessage("");
      setAmend(false);
      return t(amending ? "git.amended" : "git.checkpointCreated", { id: result.commit.id.slice(0, 8), subject: result.commit.subject });
    });
  };
  const undoLastCommit = () => void run("undo", async () => {
    const undone = await gitUndoLastCommit();
    // The commit's message comes back to the composer, as Git tools do.
    setMessage((current) => current.trim() === "" ? undone.subject : current);
    return t("git.undone", { subject: undone.subject });
  });
  const showHistory = (path: string) => { setHistoryPath(path); setTab("history"); };
  const syncState = status.upstream
    ? status.ahead === 0 && status.behind === 0 ? t("git.inSync") : [status.ahead > 0 ? t("git.toSend", { count: status.ahead }) : null, status.behind > 0 ? t("git.toReceive", { count: status.behind }) : null].filter(Boolean).join(" · ")
    : t(hasRemote ? "git.notPublished" : "git.noRemote");

  return (
    <div className="git-panel">
      <div className="git-summary">
        <div className="git-summary-branch">
          <span className="git-summary-icon"><UiIcon icon="gitBranch" size="md" /></span>
          <div className="git-summary-text">
            <GitBranchPicker
              branches={branches}
              current={status.branch}
              switchBlocked={status.hasTranslationChanges ? t("git.switchBlocked") : null}
              disabled={busy !== null}
              blockLabels={blockLabels}
              onSwitch={(name) => void run("switch", async () => { await gitSwitchBranch(name); onWorkspaceChanged?.(); return t("git.switched", { name }); })}
              onCreate={(name) => void run("branch", async () => { await gitCreateBranch(name); return t("git.branch.created", { name }); })}
              onDelete={setDeleting}
            />
            <span className="muted">{syncState}{status.upstream ? <> · <span className="mono">{status.upstream}</span></> : null}</span>
          </div>
          {onOpenSettings ? <IconButton className="git-summary-settings" icon="settings" label={t("git.openRepository")} onClick={onOpenSettings} /> : null}
        </div>
        {hasRemote ? (
          <div className="git-ops" role="toolbar" aria-label={t("git.operations")}>
            <button className="button button-ghost" type="button" disabled={busy !== null} title={t("git.fetchTitle")} onClick={() => void run("fetch", async () => { await gitFetch(); return t("git.fetched"); })}>
              {busy === "fetch" ? <span className="spinner" /> : <UiIcon icon="refreshCw" size="sm" />}{t("git.fetch")}
            </button>
            <button className="button button-ghost" type="button" disabled={busy !== null || status.hasTranslationChanges} title={t(status.hasTranslationChanges ? "git.pullBlocked" : "git.pullTitle")} onClick={() => void run("pull", async () => describePull(await gitPull()))}>
              {busy === "pull" ? <span className="spinner" /> : <UiIcon icon="arrowDown" size="sm" />}{t("git.pull")}{status.behind > 0 ? <span className="git-ops-count">{status.behind}</span> : null}
            </button>
            <button className="button button-ghost" type="button" disabled={busy !== null || status.head === null || (status.upstream !== null && status.behind > 0)} title={t(status.upstream !== null && status.behind > 0 ? "git.pushBehind" : status.upstream === null ? "git.pushPublish" : "git.pushTitle")} onClick={() => void run("push", async () => t(await gitPush() ? "git.pushed" : "git.nothingToPush"))}>
              {busy === "push" ? <span className="spinner" /> : <UiIcon icon="arrowUp" size="sm" />}{t("git.push")}{status.ahead > 0 ? <span className="git-ops-count">{status.ahead}</span> : null}
            </button>
            {/* A pushed branch other than the one the remote guards reaches it through a pull request. */}
            {guardWorkflow?.github && status.branch && status.branch !== guardWorkflow.branch && status.upstream !== null ? (
              <button className="button button-ghost" type="button" title={t("git.pullRequestTitle", { branch: guardWorkflow.branch })} onClick={() => void gitOpenPullRequest().catch((caught: unknown) => setError(normalizeCommandError(caught)))}>
                <UiIcon icon="gitPullRequest" size="sm" />{t("git.pullRequest")}
              </button>
            ) : null}
          </div>
        ) : null}
        {!hasRemote && onOpenSettings ? <button className="link-button git-summary-link" type="button" onClick={onOpenSettings}>{t("git.connectRemote")}</button> : null}
      </div>

      {feedback}
      <GitBranchDelete branch={deleting} onCancel={() => setDeleting(null)} run={(label, action) => void run(label, action)} />

      {conflicts.length > 0 ? (
        <Section title={t("git.chooseVersions")} icon="gitMerge" meta={conflicts.length}>
          <ul className="git-list">
            {conflicts.map((conflict) => (
              <li className="git-item" key={conflict.context}>
                <span className="git-item-title mono">{bindingLabel(conflict.sourceBinding, t)}</span>
                <span className="git-item-subject">{conflict.sourceMacro}</span>
                <div className="git-choice" role="radiogroup" aria-label={t("git.versionFor", { unit: bindingLabel(conflict.sourceBinding, t) })}>
                  {(["ours", "theirs"] as const).map((side) => {
                    const version = side === "ours" ? conflict.ours : conflict.theirs;
                    const checked = choices[conflict.context] === side;
                    return (
                      <label className={checked ? "git-choice-option checked" : "git-choice-option"} key={side}>
                        <input type="radio" name={conflict.context} checked={checked} onChange={() => setChoices({ ...choices, [conflict.context]: side })} />
                        <span className="git-choice-side">{t(side === "ours" ? "git.mine" : "git.server")}</span>
                        <span className="git-choice-text">{version.targetMacro || <em>{t("git.deleted")}</em>}</span>
                      </label>
                    );
                  })}
                </div>
              </li>
            ))}
          </ul>
          <button className="button button-primary button-block" type="button" disabled={busy !== null || conflicts.some((conflict) => !choices[conflict.context])} onClick={() => void run("pull", async () => {
            const resolutions = conflicts.map((conflict) => ({ context: conflict.context, resolution: choices[conflict.context] ?? "ours" }));
            return describePull(await gitPull(resolutions));
          })}>{t("git.pullWithChoices")}</button>
        </Section>
      ) : null}

      <RepositoryGuard workflow={guardWorkflow} onWorkflow={setGuardWorkflow} busy={busy !== null} run={(label, action) => void run(label, action)} onError={setError} />

      <div className="git-tabs">
      <Segmented<GitTab>
        label={t("git.views")}
        value={tab}
        onChange={setTab}
        options={[
          { value: "changes", label: <>{t("git.changes")}{changeCount > 0 ? <span className="git-count">{changeCount}</span> : null}</> },
          { value: "history", label: t("git.history") },
        ]}
      />
      </div>

      {tab === "changes" ? (
        <section className="git-tab-body" aria-label={t("git.changes")}>
          <form className="git-composer" onSubmit={(event) => { event.preventDefault(); if (canCommit) commit(); }}>
            <textarea
              className="input"
              aria-label={t("git.checkpointMessage")}
              rows={2}
              placeholder={t(staged.length > 0 ? "git.messageStaged" : "git.checkpointPlaceholder")}
              value={message}
              onChange={(event) => setMessage(event.target.value)}
              onKeyDown={(event) => { if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) { event.preventDefault(); if (canCommit) commit(); } }}
            />
            <label className="checkbox git-amend" title={t(amendable ? "git.amendHint" : "git.amendPublished", { remote: syncRemote ?? "origin" })}>
              <input type="checkbox" checked={amend && amendable} disabled={!amendable || busy !== null} onChange={(event) => setAmend(event.target.checked)} />
              {t("git.amend")}
            </label>
            <div className="git-composer-foot">
              {identityDraft ? null : identityComplete ? (
                <button className="git-identity" type="button" title={t("git.changeIdentity")} onClick={() => setIdentityDraft({ name: identity?.name ?? "", email: identity?.email ?? "", global: identity?.nameScope === "global" })}>
                  <UiIcon icon="user" size="xs" />{identity?.name}
                </button>
              ) : (
                <button className="git-identity warn" type="button" onClick={() => setIdentityDraft({ name: identity?.name ?? "", email: identity?.email ?? "", global: identity?.nameScope === "global" })}>
                  <UiIcon icon="user" size="xs" />{t("git.setName")}
                </button>
              )}
              <span className="spacer" />
              <button
                className="button button-primary"
                type="submit"
                disabled={!canCommit}
                title={t(!identityComplete ? "git.setNameFirst" : amend ? "git.amendHint" : !committable ? "git.nothingToSave" : staged.length > 0 ? "git.commitStaged" : "git.saveCheckpoint", { remote: syncRemote ?? "origin" })}
              >
                <UiIcon icon="gitCommit" size="sm" />{t(busy === "commit" ? "common.saving" : amend ? "git.commitAmend" : "git.checkpoint")}
              </button>
            </div>
            {!identityComplete ? <p className="git-hint">{t("git.identityHint")}</p> : null}
          </form>
          {identityDraft ? (
            <form className="git-identity-form" onSubmit={(event) => { event.preventDefault(); const draft = identityDraft; void run("identity", async () => { await gitSetIdentity(draft.name, draft.email.trim() === "" ? null : draft.email, draft.global); setIdentityDraft(null); return null; }); }}>
              <strong>{t("git.identity")}</strong>
              <input className="input" aria-label={t("git.identityName")} placeholder={t("git.name")} value={identityDraft.name} onChange={(event) => setIdentityDraft({ ...identityDraft, name: event.target.value })} />
              <input className="input" aria-label={t("git.identityEmail")} placeholder={t("git.email")} value={identityDraft.email} onChange={(event) => setIdentityDraft({ ...identityDraft, email: event.target.value })} />
              <label className="checkbox"><input type="checkbox" checked={identityDraft.global} onChange={(event) => setIdentityDraft({ ...identityDraft, global: event.target.checked })} />{t("git.identityGlobal")}</label>
              <div className="git-form-actions"><button className="button button-ghost" type="button" onClick={() => setIdentityDraft(null)}>{t("common.cancel")}</button><button className="button button-primary" type="submit" disabled={busy !== null}>{t("common.save")}</button></div>
            </form>
          ) : null}
          {changeCount === 0 ? <p className="muted">{t("git.noChanges")}</p>
            : <FileChangeList files={changes} staged={staged} stringsOf={gitPendingFileChanges} actions={fileActions} onHistory={showHistory} revision={filesRevision} selectedKey={selectedKey} onRevealBinding={onRevealBinding} viewKey="pending" />}
          <ConfirmDialog
            open={discarding !== null}
            title={t("git.discard.title")}
            message={discarding ? discardMessage(discarding) : ""}
            confirmLabel={t(discarding?.every((file) => file.kind === "untracked") ? "git.action.delete" : "git.action.discard")}
            cancelLabel={t("common.cancel")}
            onKeepEditing={() => setDiscarding(null)}
            onDiscard={() => {
              const files = discarding ?? [];
              setDiscarding(null);
              void run("discard", async () => { await gitDiscard(paths(files)); onWorkspaceChanged?.(); });
            }}
          />
        </section>
      ) : (
        <section className="git-tab-body git-history-section" aria-label={t("git.history")}>
          <div className="git-history-filters">
            <label className="git-change-search">
              <UiIcon icon="search" size="xs" />
              <input className="input" type="search" value={historyQuery} placeholder={t("git.history.search")} aria-label={t("git.history.search")} onChange={(event) => setHistoryQuery(event.target.value)} />
            </label>
            {historyPath ? (
              <span className="chip git-history-path" title={historyPath}>
                <UiIcon icon="fileDiff" size="xs" /><span>{historyPath.split("/").at(-1)}</span>
                <button type="button" className="git-history-path-clear" aria-label={t("git.history.clearPath")} onClick={() => setHistoryPath(null)}><UiIcon icon="x" size="xs" /></button>
              </span>
            ) : null}
          </div>
          {commitActions.feedback}
          <GitHistoryList
            revision={historyRevision}
            selectedCommitId={selectedCommitId ?? null}
            onOpenCommit={(commit) => onOpenCommit?.(commit)}
            query={searched}
            path={historyPath}
            remote={syncRemote}
            actionsFor={(commit) => commitActions.actionsFor(commit, {
              remote: syncRemote,
              // Only the last commit, while origin does not have it.
              undo: commit.unpublished && commit.parents.length <= 1 && commit.refs.some((ref) => ref === "HEAD" || ref.startsWith("HEAD -> ")) ? undoLastCommit : undefined,
            })}
          />
          {commitActions.dialogs}
        </section>
      )}
    </div>
  );
}
