import { useCallback, useEffect, useRef, useState } from "react";
import {
  gitBranches,
  gitCheckpoint,
  gitCheckWorkflow,
  gitCreateBranch,
  gitFetch,
  gitFetchMain,
  gitFinishContribution,
  gitInitialize,
  gitInstallCheckWorkflow,
  gitMergeContribution,
  gitOpenBranchSettings,
  gitOverview,
  gitPendingChanges,
  gitProjectChanges,
  gitPull,
  gitPush,
  gitSetIdentity,
  gitStateStamp,
  gitSwitchBranch,
  gitSync,
  normalizeCommandError,
} from "../ipc";
import type {
  SourceBinding,
  CheckWorkflowDto,
  CommandError,
  ConflictResolution,
  GitBranchDto,
  GitCommitDto,
  GitSyncDto,
  UnitConflictDto,
  GitOverviewDto,
  ProjectChangeDto,
  UnitChangeDto,
} from "../types";
import { IconButton } from "../ui/primitives/IconButton";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";
import { ConfirmDialog } from "./ConfirmDialog";
import { GitBranchPicker } from "./GitBranchPicker";
import { GitHistoryList } from "./GitHistory";
import { ProjectChangeList, Section, TranslationChangeGroups, unitLabel, useStickyState } from "./GitShared";

/** How often the dock checks whether the repository changed. */
const POLL_MS = 2000;
/** How often, and at most how often on focus, a contribution branch checks
 * whether the remote main branch moved. */
const MAIN_CHECK_MS = 5 * 60_000;
const MAIN_CHECK_MIN_MS = 60_000;

type GitPanelProps = {
  /** Translation unit of the selected cell, when it has one. */
  selectedUnitId: string | null;
  /** Changes whenever the editor persisted a translation. */
  workspaceRevision: number;
  /** Changes whenever project settings, glossary, or guidance may have been saved. */
  projectRevision?: number | undefined;
  onWorkspaceChanged?: (() => void) | undefined;
  /** Pending changes owned by the workbench; detached windows fetch their own. */
  pending?: { changes: UnitChangeDto[] | null; refresh: () => Promise<void> } | undefined;
  /** Opens a string in the editor; absent in detached windows. */
  onRevealBinding?: ((binding: SourceBinding) => void) | undefined;
  /** Opens one commit in a document tab; absent in detached windows. */
  onOpenCommit?: ((commit: GitCommitDto) => void) | undefined;
  selectedCommitId?: string | null | undefined;
  /** Opens Settings on the repository section. */
  onOpenSettings?: (() => void) | undefined;
};

const syncLabels: Record<GitSyncDto["integration"], { plain: MessageKey; pushed: MessageKey }> = {
  upToDate: { plain: "git.sync.upToDate", pushed: "git.sync.upToDatePushed" },
  fastForward: { plain: "git.sync.fastForward", pushed: "git.sync.fastForwardPushed" },
  merged: { plain: "git.sync.merged", pushed: "git.sync.mergedPushed" },
};

const blockLabels: Record<NonNullable<GitBranchDto["blocked"]>, MessageKey> = {
  noProject: "git.blocked.noProject",
  olderFormat: "git.blocked.olderFormat",
  otherSource: "git.blocked.otherSource",
};

/** Branch, sync, uncommitted changes with the checkpoint, and project history. It follows the repository on its own. */
export function GitPanel({ selectedUnitId, workspaceRevision, projectRevision, onWorkspaceChanged, pending: externalPending, onRevealBinding, onOpenCommit, selectedCommitId, onOpenSettings }: GitPanelProps) {
  const { t } = useI18n();
  const [overview, setOverview] = useState<GitOverviewDto | null>(null);
  const [branches, setBranches] = useState<GitBranchDto[]>([]);
  const [ownPending, setOwnPending] = useState<UnitChangeDto[]>([]);
  const [projectChanges, setProjectChanges] = useState<ProjectChangeDto[]>([]);
  const [historyRevision, setHistoryRevision] = useState(0);
  // Kept while the window lives, so reopening the Git tab keeps it as it was left.
  const [changesOpen, setChangesOpen] = useStickyState("git.changesOpen", true);
  const [message, setMessage] = useState("");
  const [identityDraft, setIdentityDraft] = useState<{ name: string; email: string; global: boolean } | null>(null);
  const [conflicts, setConflicts] = useState<UnitConflictDto[]>([]);
  const [choices, setChoices] = useState<Record<string, ConflictResolution>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmMerge, setConfirmMerge] = useState(false);
  // The operation that reported same-unit conflicts, repeated with the choices.
  const [conflictAction, setConflictAction] = useState<"sync" | "pull">("sync");
  const [checkWorkflow, setCheckWorkflow] = useState<CheckWorkflowDto | null>(null);
  const [checkDismissed, setCheckDismissed] = useStickyState("git.checkWorkflowDismissed", false);
  const [checkInstalled, setCheckInstalled] = useState(false);
  const stamp = useRef<string | null | undefined>(undefined);
  const lastMainCheck = useRef(0);

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
        else setOwnPending(await gitPendingChanges());
        setProjectChanges(await gitProjectChanges());
        setBranches(await gitBranches());
        // Optional: a failure only hides the offer.
        setCheckWorkflow(await gitCheckWorkflow().catch(() => null));
      } else {
        setOwnPending([]);
        setProjectChanges([]);
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

  // On a contribution branch, notice when the remote main branch moves (for
  // example when another pull request was merged), so the translator can
  // sync before the pull request conflicts. The fetch never asks to sign in.
  const watchMain = Boolean(overview?.contribution?.branch && !overview.contribution.local);
  useEffect(() => {
    if (!watchMain) return;
    let cancelled = false;
    const check = async () => {
      if (document.visibilityState !== "visible" || busy !== null || Date.now() - lastMainCheck.current < MAIN_CHECK_MIN_MS) return;
      lastMainCheck.current = Date.now();
      try {
        if (await gitFetchMain() && !cancelled) await refresh();
      } catch {
        // Offline or signed out: the next sync reports it.
      }
    };
    void check();
    const timer = window.setInterval(() => void check(), MAIN_CHECK_MS);
    const onFocus = () => void check();
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.removeEventListener("focus", onFocus);
    };
  }, [watchMain, busy, refresh]);

  const pending = externalPending ? externalPending.changes ?? [] : ownPending;

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

  const describeSync = (result: GitSyncDto, action: "sync" | "pull" = "sync"): string => {
    if (result.conflicts.length > 0) {
      setConflictAction(action);
      setConflicts(result.conflicts);
      setChoices({});
      return t("git.conflicts", { count: result.conflicts.length });
    }
    setConflicts([]);
    if (result.workspaceChanged) onWorkspaceChanged?.();
    const labels = syncLabels[result.integration];
    const text = t(result.pushed ? labels.pushed : labels.plain);
    return result.reconciled ? `${text} ${t("git.sync.reconciled")}` : text;
  };

  const feedback = <>
    {error ? <div className="git-feedback error" role="alert"><UiIcon icon="circleAlert" size="sm" /><span>{error.message}</span></div> : null}
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
  const changeCount = pending.length + projectChanges.length;
  const mainBranch = overview.collaboration?.mainBranch ?? null;
  const onMain = mainBranch !== null && status.branch === mainBranch && status.head !== null;
  const localBranches = branches.filter((branch) => !branch.remote);
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
              mainBranch={mainBranch}
              switchBlocked={status.hasTranslationChanges ? t("git.switchBlocked") : null}
              disabled={busy !== null}
              blockLabels={blockLabels}
              onSwitch={(name) => void run("switch", async () => { await gitSwitchBranch(name); onWorkspaceChanged?.(); return t("git.switched", { name }); })}
              onCreate={(name) => void run("branch", async () => { await gitCreateBranch(name); return t("git.branch.created", { name }); })}
            />
            <span className="muted">{syncState}{status.upstream ? <> · <span className="mono">{status.upstream}</span></> : null}</span>
          </div>
          {onOpenSettings ? <IconButton className="git-summary-settings" icon="settings" label={t("git.openRepository")} onClick={onOpenSettings} /> : null}
        </div>
        {hasRemote ? (
          <div className="git-ops" role="toolbar" aria-label={t("git.operations")}>
            <button className="button button-ghost" type="button" disabled={busy !== null} title={t("git.fetchTitle")} onClick={() => void run("fetch", async () => { await gitFetch(); return t("git.fetched"); })}>
              <UiIcon icon="cloud" size="sm" className={busy === "fetch" ? "spin" : undefined} />{t("git.fetch")}
            </button>
            <button className="button button-ghost" type="button" disabled={busy !== null || status.hasTranslationChanges} title={t(status.hasTranslationChanges ? "git.pullBlocked" : "git.pullTitle")} onClick={() => void run("pull", async () => describeSync(await gitPull(), "pull"))}>
              <UiIcon icon="arrowDown" size="sm" />{t("git.pull")}{status.behind > 0 ? <span className="git-ops-count">{status.behind}</span> : null}
            </button>
            <button className="button button-ghost" type="button" disabled={busy !== null || status.head === null || (status.upstream !== null && status.behind > 0) || onMain} title={t(onMain ? "git.pushMain" : status.upstream !== null && status.behind > 0 ? "git.pushBehind" : status.upstream === null ? "git.pushPublish" : "git.pushTitle")} onClick={() => void run("push", async () => t(await gitPush() ? "git.pushed" : "git.nothingToPush"))}>
              <UiIcon icon="arrowUp" size="sm" />{t("git.push")}{status.ahead > 0 ? <span className="git-ops-count">{status.ahead}</span> : null}
            </button>
            <button className="button button-secondary" type="button" disabled={busy !== null || status.hasTranslationChanges} title={t(status.hasTranslationChanges ? "git.syncBlocked" : "git.syncTitle")} onClick={() => void run("sync", async () => describeSync(await gitSync()))}>
              <UiIcon icon="refreshCw" size="sm" className={busy === "sync" ? "spin" : undefined} />{t(busy === "sync" ? "git.syncing" : "git.sync")}
            </button>
          </div>
        ) : null}
        {!hasRemote && onOpenSettings ? <button className="link-button git-summary-link" type="button" onClick={onOpenSettings}>{t("git.connectRemote")}</button> : null}
        {overview.collaboration?.error ? <p className="git-hint warn">{overview.collaboration.error}</p> : null}
        {onMain ? <p className="git-hint">{t(hasRemote ? "git.onMainHint" : "git.onMainLocalHint", { branch: mainBranch })}</p> : null}
        {(() => {
          const main = localBranches.find((branch) => branch.name === mainBranch && branch.blocked !== null);
          return main ? <p className="git-hint">{t(hasRemote ? "git.mainBehind" : "git.mainBehindLocal", { branch: main.name })}</p> : null;
        })()}
      </div>

      {feedback}

      {conflicts.length > 0 ? (
        <Section title={t("git.chooseVersions")} icon="gitMerge" meta={conflicts.length}>
          <ul className="git-list">
            {conflicts.map((conflict) => (
              <li className="git-item" key={conflict.translationUnitId}>
                <span className="git-item-title mono">{unitLabel(conflict.ours ?? conflict.theirs, t)}</span>
                <div className="git-choice" role="radiogroup" aria-label={t("git.versionFor", { unit: unitLabel(conflict.ours ?? conflict.theirs, t) })}>
                  {(["ours", "theirs"] as const).map((side) => {
                    const version = side === "ours" ? conflict.ours : conflict.theirs;
                    const checked = choices[conflict.translationUnitId] === side;
                    return (
                      <label className={checked ? "git-choice-option checked" : "git-choice-option"} key={side}>
                        <input type="radio" name={conflict.translationUnitId} checked={checked} onChange={() => setChoices({ ...choices, [conflict.translationUnitId]: side })} />
                        <span className="git-choice-side">{t(side === "ours" ? "git.mine" : "git.server")}</span>
                        <span className="git-choice-text">{version ? version.targetMacro || t("common.empty") : <em>{t("git.deleted")}</em>}</span>
                      </label>
                    );
                  })}
                </div>
              </li>
            ))}
          </ul>
          <button className="button button-primary button-block" type="button" disabled={busy !== null || conflicts.some((conflict) => !choices[conflict.translationUnitId])} onClick={() => void run(conflictAction, async () => {
            const resolutions = conflicts.map((conflict) => ({ translationUnitId: conflict.translationUnitId, resolution: choices[conflict.translationUnitId] ?? "ours" }));
            return conflictAction === "pull" ? describeSync(await gitPull(resolutions), "pull") : describeSync(await gitSync(resolutions));
          })}>{t(conflictAction === "pull" ? "git.pullWithChoices" : "git.syncWithChoices")}</button>
        </Section>
      ) : null}

      {checkWorkflow?.github && checkWorkflow.state !== "current" && !checkDismissed ? (
        <Section
          title={t("git.check.title")}
          icon="circleCheck"
          action={<IconButton icon="x" label={t("git.check.dismiss")} onClick={() => setCheckDismissed(true)} />}
        >
          <p className="muted">{t(checkWorkflow.state === "different" ? "git.check.outdated" : "git.check.offer")}</p>
          <ul className="git-check-stages">
            <li>{t("git.check.stage.integrity")}</li>
            <li>{t("git.check.stage.translations")}</li>
            <li>{t("git.check.stage.merge")}</li>
          </ul>
          {!checkWorkflow.topLevel ? <p className="git-hint">{t("git.check.subfolder")}</p>
            : !checkWorkflow.available ? <p className="git-hint">{t("git.check.releaseOnly")}</p> : null}
          <div className="git-form-actions">
            <button
              className="button button-primary"
              type="button"
              disabled={busy !== null || !checkWorkflow.available || !checkWorkflow.topLevel}
              onClick={() => void run("checkWorkflow", async () => { setCheckWorkflow(await gitInstallCheckWorkflow()); setCheckInstalled(true); return null; })}
            >
              <UiIcon icon="plus" size="sm" />{t(checkWorkflow.state === "different" ? "git.check.update" : "git.check.add")}
            </button>
          </div>
        </Section>
      ) : null}
      {checkWorkflow?.github && checkWorkflow.state === "current" && checkInstalled ? (
        <div className="git-feedback" role="status">
          <UiIcon icon="info" size="sm" />
          <span>{t("git.check.installed")} {t("git.check.required")} <button className="link-button" type="button" onClick={() => void gitOpenBranchSettings().catch((caught: unknown) => setError(normalizeCommandError(caught)))}>{t("git.check.openSettings")}</button></span>
        </div>
      ) : null}

      {overview.contribution?.branch && overview.contribution.local ? (
        <Section title={t("git.contribution")} icon="gitPullRequest" meta={t("git.contributionInto", { branch: overview.contribution.mainBranch })}>
          <p className="muted">{t("git.localMergeHint", { branch: overview.contribution.mainBranch })}</p>
          <button className="button button-secondary" type="button" disabled={busy !== null || status.hasTranslationChanges} title={status.hasTranslationChanges ? t("git.switchBlocked") : undefined} onClick={() => setConfirmMerge(true)}>
            <UiIcon icon="gitMerge" size="sm" />{t(busy === "merge" ? "git.merging" : "git.localMerge", { branch: overview.contribution.mainBranch })}
          </button>
          <ConfirmDialog
            open={confirmMerge}
            message={t("git.localMergeConfirm", { source: overview.contribution.branch, branch: overview.contribution.mainBranch })}
            confirmLabel={t("git.localMerge", { branch: overview.contribution.mainBranch })}
            cancelLabel={t("common.cancel")}
            onKeepEditing={() => setConfirmMerge(false)}
            onDiscard={() => { setConfirmMerge(false); void run("merge", async () => { const result = await gitMergeContribution(); onWorkspaceChanged?.(); return t("git.localMerged", { branch: overview.contribution?.mainBranch ?? "" }) + (result.deletedBranch ? ` ${t("git.localMergedDeleted", { source: result.deletedBranch })}` : ""); }); }}
          />
        </Section>
      ) : overview.contribution?.branch ? (
        <Section title={t("git.contribution")} icon="gitPullRequest" meta={t("git.contributionInto", { branch: overview.contribution.mainBranch })}>
          <p className="muted">{t(overview.contribution.published ? "git.contributionSent" : "git.contributionNotSent")} · {overview.contribution.unmergedCommits === 0 ? t("git.contributionAccepted") : t("git.contributionWaiting", { count: overview.contribution.unmergedCommits })}</p>
          {overview.contribution.mainAhead > 0 && overview.contribution.unmergedCommits > 0 ? (
            <div className="git-feedback warning" role="status"><UiIcon icon="circleAlert" size="sm" /><span>{t("git.mainAhead", { count: overview.contribution.mainAhead, branch: overview.contribution.mainBranch })}</span></div>
          ) : null}
          {overview.contribution.unmergedCommits === 0 && overview.contribution.published ? <button className="button button-secondary" type="button" disabled={busy !== null || status.hasTranslationChanges} onClick={() => void run("finish", async () => { const result = await gitFinishContribution(); onWorkspaceChanged?.(); return t(result.deletedBranch ? "git.contributionFinished" : "git.contributionKept"); })}>{t("git.finishContribution", { branch: overview.contribution.mainBranch })}</button> : null}
        </Section>
      ) : null}

      <section className="git-section" aria-label={t("git.changes")}>
        <button className="git-section-head git-section-toggle" type="button" aria-expanded={changesOpen} onClick={() => setChangesOpen(!changesOpen)}>
          <UiIcon icon={changesOpen ? "chevronDown" : "chevronRight"} size="xs" />
          <UiIcon icon="fileDiff" size="sm" />
          <strong>{t("git.changes")}</strong>
          <span className="git-count">{changeCount}</span>
        </button>
        {changesOpen ? <>
          <form className="git-composer" onSubmit={(event) => { event.preventDefault(); const text = message.trim(); void run("checkpoint", async () => { const result = await gitCheckpoint(text === "" ? null : text); setMessage(""); return result.branchCreated ? t("git.checkpointOnBranch", { branch: result.branchCreated, subject: result.commit.subject }) : t("git.checkpointCreated", { id: result.commit.id.slice(0, 8), subject: result.commit.subject }); }); }}>
            <textarea className="input" aria-label={t("git.checkpointMessage")} rows={2} placeholder={t("git.checkpointPlaceholder")} value={message} onChange={(event) => setMessage(event.target.value)} />
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
              <button className="button button-primary" type="submit" disabled={busy !== null || changeCount === 0 || !identityComplete} title={t(!identityComplete ? "git.setNameFirst" : changeCount === 0 ? "git.nothingToSave" : "git.saveCheckpoint")}>
                <UiIcon icon="gitCommit" size="sm" />{t(busy === "checkpoint" ? "common.saving" : "git.checkpoint")}
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
          {changeCount === 0 ? <p className="muted">{t("git.noChanges")}</p> : <>
            {pending.length > 0 ? <>
              <h4 className="git-subhead">{t("git.changes.translations", { count: pending.length })}</h4>
              <TranslationChangeGroups changes={pending} selectedUnitId={selectedUnitId} onRevealBinding={onRevealBinding} viewKey="pending" />
            </> : null}
            {projectChanges.length > 0 ? <>
              <h4 className="git-subhead">{t("git.changes.project")}</h4>
              <ProjectChangeList changes={projectChanges} />
            </> : null}
          </>}
        </> : null}
      </section>

      <section className="git-section git-history-section" aria-label={t("git.history")}>
        <header className="git-section-head">
          <UiIcon icon="history" size="sm" />
          <strong>{t("git.history")}</strong>
        </header>
        <GitHistoryList revision={historyRevision} selectedCommitId={selectedCommitId ?? null} onOpenCommit={(commit) => onOpenCommit?.(commit)} />
      </section>
    </div>
  );
}
