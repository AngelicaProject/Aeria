import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  gitBranches,
  gitCheckpoint,
  gitCommitChanges,
  gitContributors,
  gitCreateBranch,
  gitFinishContribution,
  gitInitialize,
  gitLog,
  gitOverview,
  gitPendingChanges,
  gitSetCollaboration,
  gitSetIdentity,
  gitSetRemote,
  gitSwitchBranch,
  gitSync,
  gitUnitHistory,
  normalizeCommandError,
} from "../ipc";
import type {
  SourceBinding,
  CommandError,
  ConflictResolution,
  ContributorDto,
  GitBranchDto,
  GitSyncDto,
  UnitConflictDto,
  GitCommitChangesDto,
  GitCommitDto,
  GitFileKind,
  GitOverviewDto,
  RecordVersionDto,
  UnitChangeDto,
  UnitHistoryDto,
  UnitVersionDto,
} from "../types";
import { formatRelativeTime } from "../timeDisplay";
import { IconButton } from "../ui/primitives/IconButton";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import type { GitPresentationMode } from "./WorkbenchToolDock";
import { useI18n, type Translate } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";

const HISTORY_LIMIT = 50;

type GitPanelProps = {
  mode: GitPresentationMode;
  /** Translation unit of the selected cell, when it has one. */
  selectedUnitId: string | null;
  /** Changes whenever the editor persisted a translation. */
  workspaceRevision: number;
  onWorkspaceChanged?: (() => void) | undefined;
  /** Absent in detached windows that cannot edit. */
  onRestoreTarget?: ((targetMacro: string) => void) | undefined;
  /** Pending changes owned by the workbench; detached windows fetch their own. */
  pending?: { changes: UnitChangeDto[] | null; refresh: () => Promise<void> } | undefined;
  /** Opens a string in the editor; absent in detached windows. */
  onRevealBinding?: ((binding: SourceBinding) => void) | undefined;
};

type ChangeGroup = { sheetName: string; changes: UnitChangeDto[] };

function changeBinding(change: UnitChangeDto): SourceBinding | null {
  return (change.after ?? change.before)?.sourceBinding ?? null;
}

/** Groups changes by sheet, ordered by sheet name then row coordinate. */
function groupChanges(changes: readonly UnitChangeDto[], unknownSheet: string): ChangeGroup[] {
  const groups = new Map<string, UnitChangeDto[]>();
  for (const change of changes) {
    const sheetName = changeBinding(change)?.sheetName ?? unknownSheet;
    groups.set(sheetName, [...(groups.get(sheetName) ?? []), change]);
  }
  const order = (change: UnitChangeDto) => {
    const binding = changeBinding(change);
    return binding ? [binding.rowId, binding.subrowId, binding.columnIndex] : [0, 0, 0];
  };
  return [...groups].sort(([left], [right]) => left.localeCompare(right)).map(([sheetName, entries]) => ({
    sheetName,
    changes: entries.sort((left, right) => {
      const [a, b] = [order(left), order(right)];
      return a[0]! - b[0]! || a[1]! - b[1]! || a[2]! - b[2]!;
    }),
  }));
}

const kindLetter: Record<UnitChangeDto["kind"], string> = { added: "A", modified: "M", removed: "D" };
const kindLabel: Record<UnitChangeDto["kind"], MessageKey> = { added: "git.kind.added", modified: "git.kind.modified", removed: "git.kind.removed" };
const fileKindLabel: Record<GitFileKind, MessageKey> = {
  added: "git.file.added",
  modified: "git.file.modified",
  deleted: "git.file.deleted",
  renamed: "git.file.renamed",
  copied: "git.file.copied",
  typeChanged: "git.file.typeChanged",
  untracked: "git.file.untracked",
  conflicted: "git.file.conflicted",
};

type ChangeRowProps = {
  change: UnitChangeDto;
  selected: boolean;
  onOpen?: (() => void) | undefined;
};

function ChangeRow({ change, selected, onOpen }: ChangeRowProps) {
  const { t } = useI18n();
  const binding = changeBinding(change);
  const text = change.after?.targetMacro ?? change.before?.targetMacro ?? "";
  const content = <>
    <span className={`git-kind git-kind-${change.kind}`} aria-label={t(kindLabel[change.kind])}>{kindLetter[change.kind]}</span>
    <span className="git-change-coord mono">{binding ? `${binding.rowId}:${binding.subrowId}` : "?"}{binding ? <small> {t("common.column", { column: String(binding.columnIndex) })}</small> : null}</span>
    <span className={change.kind === "removed" ? "git-change-text removed" : "git-change-text"}>{text || <em>{t("git.emptyText")}</em>}</span>
    <span className="git-change-meta">{changeLabel(change, t)}</span>
  </>;
  return onOpen
    ? <li><button type="button" className={selected ? "git-change selected" : "git-change"} onClick={onOpen} title={binding ? t("git.openUnit", { unit: unitLabel(change.after ?? change.before, t) }) : t("git.openString")}>{content}</button></li>
    : <li><div className={selected ? "git-change selected" : "git-change"}>{content}</div></li>;
}

function unitLabel(unit: UnitVersionDto | null, t: Translate): string {
  if (!unit) return t("git.unknownUnit");
  const binding = unit.sourceBinding;
  return t("common.cellLocation", { sheet: binding.sheetName, row: String(binding.rowId), subrow: String(binding.subrowId), column: String(binding.columnIndex) });
}

function changeLabel(change: UnitChangeDto, t: Translate): string {
  if (change.kind === "added") return t("git.change.added");
  if (change.kind === "removed") return t("git.change.removed");
  const parts = [];
  if (change.targetChanged) parts.push(t("git.change.text"));
  if (change.reviewChanged) parts.push(t("git.change.review"));
  if (change.noteChanged) parts.push(t("git.change.note"));
  return parts.join(", ") || t("git.change.changed");
}

const syncLabels: Record<GitSyncDto["integration"], { plain: MessageKey; pushed: MessageKey }> = {
  upToDate: { plain: "git.sync.upToDate", pushed: "git.sync.upToDatePushed" },
  fastForward: { plain: "git.sync.fastForward", pushed: "git.sync.fastForwardPushed" },
  merged: { plain: "git.sync.merged", pushed: "git.sync.mergedPushed" },
};

function versionTarget(version: RecordVersionDto): string | null {
  return version.state === "valid" ? version.unit.targetMacro : null;
}

function Section({ title, icon, meta, action, children }: { title: string; icon: UiIconName; meta?: ReactNode; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="git-section" aria-label={title}>
      <header className="git-section-head">
        <UiIcon icon={icon} size="sm" />
        <strong>{title}</strong>
        {meta !== undefined ? <span className="git-count">{meta}</span> : null}
        <span className="spacer" />
        {action}
      </header>
      {children}
    </section>
  );
}

export function GitPanel({ mode, selectedUnitId, workspaceRevision, onWorkspaceChanged, onRestoreTarget, pending: externalPending, onRevealBinding }: GitPanelProps) {
  const { t, locale } = useI18n();
  const formatTime = (seconds: number) => new Date(seconds * 1000).toLocaleString(locale);
  const relativeTime = (seconds: number) => formatRelativeTime(seconds * 1000, Date.now(), locale, t("time.justNow"));
  const [overview, setOverview] = useState<GitOverviewDto | null>(null);
  const [ownPending, setOwnPending] = useState<UnitChangeDto[]>([]);
  const [collapsedSheets, setCollapsedSheets] = useState<ReadonlySet<string>>(() => new Set());
  const [unitHistory, setUnitHistory] = useState<UnitHistoryDto | null>(null);
  const [log, setLog] = useState<GitCommitDto[]>([]);
  const [openCommit, setOpenCommit] = useState<GitCommitChangesDto | null>(null);
  const [contributors, setContributors] = useState<ContributorDto[] | null>(null);
  const [message, setMessage] = useState("");
  const [identityDraft, setIdentityDraft] = useState<{ name: string; email: string; global: boolean } | null>(null);
  const [remoteUrl, setRemoteUrl] = useState("");
  const [conflicts, setConflicts] = useState<UnitConflictDto[]>([]);
  const [choices, setChoices] = useState<Record<string, ConflictResolution>>({});
  const [branches, setBranches] = useState<GitBranchDto[]>([]);
  const [newBranch, setNewBranch] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const isRepository = overview?.repository != null;
  const identity = overview?.identity ?? null;
  // Git requires only a name; the email is optional (see aeria-git identity rules).
  const identityComplete = Boolean(identity?.name);

  const refresh = useCallback(async () => {
    try {
      const next = await gitOverview();
      setOverview(next);
      if (next.repository) {
        if (externalPending) await externalPending.refresh();
        else setOwnPending(await gitPendingChanges());
        if (mode === "advanced") {
          setLog(await gitLog(0, HISTORY_LIMIT));
          setBranches(await gitBranches());
        }
      } else {
        setOwnPending([]);
        setLog([]);
      }
      setError(null);
    } catch (caught) {
      setError(normalizeCommandError(caught));
    }
    // Depend on the stable refresh callback, not the pending object that changes with every fetch.
  }, [mode, externalPending?.refresh]);

  useEffect(() => {
    void refresh();
  }, [refresh, workspaceRevision]);

  const pending = externalPending ? externalPending.changes ?? [] : ownPending;

  useEffect(() => {
    let cancelled = false;
    if (!selectedUnitId || !isRepository) {
      setUnitHistory(null);
      return;
    }
    gitUnitHistory(selectedUnitId, HISTORY_LIMIT)
      .then((history) => { if (!cancelled) setUnitHistory(history); })
      .catch((caught) => { if (!cancelled) setError(normalizeCommandError(caught)); });
    return () => { cancelled = true; };
  }, [selectedUnitId, isRepository, workspaceRevision, overview?.repository?.head]);

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

  const describeSync = (result: GitSyncDto): string => {
    if (result.conflicts.length > 0) {
      setConflicts(result.conflicts);
      setChoices({});
      return t("git.conflicts", { count: result.conflicts.length });
    }
    setConflicts([]);
    if (result.workspaceChanged) onWorkspaceChanged?.();
    const labels = syncLabels[result.integration];
    return t(result.pushed ? labels.pushed : labels.plain);
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
  const syncState = status.upstream
    ? status.ahead === 0 && status.behind === 0 ? t("git.inSync") : [status.ahead > 0 ? t("git.toSend", { count: status.ahead }) : null, status.behind > 0 ? t("git.toReceive", { count: status.behind }) : null].filter(Boolean).join(" · ")
    : t(hasRemote ? "git.notPublished" : "git.noRemote");

  return (
    <div className="git-panel">
      <div className="git-summary">
        <div className="git-summary-branch">
          <span className="git-summary-icon"><UiIcon icon="gitBranch" size="md" /></span>
          <div>
            <strong>{status.branch ?? t("git.detachedHead")}</strong>
            <span className="muted">{syncState}{status.upstream ? <> · <span className="mono">{status.upstream}</span></> : null}</span>
          </div>
        </div>
        <div className="git-summary-actions">
          {hasRemote ? (
            <button className="button button-secondary" type="button" disabled={busy !== null || status.hasTranslationChanges} title={t(status.hasTranslationChanges ? "git.syncBlocked" : "git.syncTitle")} onClick={() => void run("sync", async () => describeSync(await gitSync()))}>
              <UiIcon icon="refreshCw" size="sm" className={busy === "sync" ? "spin" : undefined} />{t(busy === "sync" ? "git.syncing" : "git.sync")}
            </button>
          ) : null}
          <IconButton icon="refreshCw" label={t("git.refresh")} disabled={busy !== null} onClick={() => void run("refresh", async () => null)} />
        </div>
        {!hasRemote ? (
          <form className="git-inline-form" onSubmit={(event) => { event.preventDefault(); const url = remoteUrl; void run("remote", async () => { await gitSetRemote("origin", url); setRemoteUrl(""); return t("git.remoteAdded"); }); }}>
            <input className="input" aria-label={t("git.remoteUrl")} placeholder={t("git.remotePlaceholder")} value={remoteUrl} onChange={(event) => setRemoteUrl(event.target.value)} spellCheck={false} />
            <button className="button button-secondary" type="submit" disabled={busy !== null || remoteUrl.trim() === ""}>{t("git.addRemote")}</button>
          </form>
        ) : null}
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
          <button className="button button-primary button-block" type="button" disabled={busy !== null || conflicts.some((conflict) => !choices[conflict.translationUnitId])} onClick={() => void run("sync", async () => describeSync(await gitSync(conflicts.map((conflict) => ({ translationUnitId: conflict.translationUnitId, resolution: choices[conflict.translationUnitId] ?? "ours" })))))}>{t("git.syncWithChoices")}</button>
        </Section>
      ) : null}

      {overview.contribution ? (
        <Section title={t("git.contribution")} icon="gitPullRequest" meta={t("git.contributionInto", { branch: overview.contribution.mainBranch })}>
          {overview.contribution.branch === null ? <p className="muted">{t("git.contributionNext")}</p> : <>
            <p className="muted"><span className="mono">{overview.contribution.branch}</span> · {t(overview.contribution.published ? "git.contributionSent" : "git.contributionNotSent")} · {overview.contribution.unmergedCommits === 0 ? t("git.contributionAccepted") : t("git.contributionWaiting", { count: overview.contribution.unmergedCommits })}</p>
            {overview.contribution.unmergedCommits === 0 && overview.contribution.published ? <button className="button button-secondary" type="button" disabled={busy !== null || status.hasTranslationChanges} onClick={() => void run("finish", async () => { const result = await gitFinishContribution(); onWorkspaceChanged?.(); return t(result.deletedBranch ? "git.contributionFinished" : "git.contributionKept"); })}>{t("git.finishContribution")}</button> : null}
          </>}
        </Section>
      ) : null}

      <Section title={t("git.changes")} icon="fileDiff" meta={pending.length}>
        {pending.length === 0 ? <p className="muted">{t("git.noChanges")}</p> : (
          <div className="git-change-groups">
            {groupChanges(pending, t("git.unknownSheet")).map((group) => {
              const collapsed = collapsedSheets.has(group.sheetName);
              return (
                <div className="git-change-group" key={group.sheetName}>
                  <button type="button" className="git-change-group-head" aria-expanded={!collapsed} onClick={() => setCollapsedSheets((current) => {
                    const next = new Set(current);
                    if (next.has(group.sheetName)) next.delete(group.sheetName);
                    else next.add(group.sheetName);
                    return next;
                  })}>
                    <UiIcon icon={collapsed ? "chevronRight" : "chevronDown"} size="xs" />
                    <UiIcon icon="table2" size="sm" />
                    <span className="git-change-group-name">{group.sheetName}</span>
                    <span className="git-count">{group.changes.length}</span>
                  </button>
                  {collapsed ? null : (
                    <ul className="git-change-list">
                      {group.changes.map((change) => {
                        const binding = changeBinding(change);
                        return <ChangeRow key={change.translationUnitId} change={change} selected={change.translationUnitId === selectedUnitId} onOpen={binding && onRevealBinding && change.kind !== "removed" ? () => onRevealBinding(binding) : undefined} />;
                      })}
                    </ul>
                  )}
                </div>
              );
            })}
          </div>
        )}
        <form className="git-composer" onSubmit={(event) => { event.preventDefault(); const text = message.trim(); void run("checkpoint", async () => { const result = await gitCheckpoint(text === "" ? null : text); setMessage(""); return t("git.checkpointCreated", { id: result.commit.id.slice(0, 8), subject: result.commit.subject }); }); }}>
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
            <button className="button button-primary" type="submit" disabled={busy !== null || !status.hasTranslationChanges || !identityComplete} title={t(!identityComplete ? "git.setNameFirst" : !status.hasTranslationChanges ? "git.nothingToSave" : "git.saveCheckpoint")}>
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
      </Section>

      <Section title={t("git.stringHistory")} icon="history">
        {unitHistory && (unitHistory.translatedBy || unitHistory.reviewedBy) ? (
          <p className="git-attribution">
            {unitHistory.translatedBy ? <span><UiIcon icon="user" size="xs" />{t("git.translatedBy")} <strong>{unitHistory.translatedBy.authorName}</strong></span> : null}
            {unitHistory.reviewedBy ? <span><UiIcon icon="circleCheck" size="xs" />{t("git.reviewedBy")} <strong>{unitHistory.reviewedBy.authorName}</strong></span> : null}
          </p>
        ) : null}
        {!selectedUnitId ? <p className="muted">{t("git.selectString")}</p> : !unitHistory ? <p className="muted">{t("common.loading")}</p> : (
          <ol className="git-timeline">
            {unitHistory.pending ? <li className="git-timeline-item pending"><div className="git-item-head"><strong>{t("git.uncommitted")}</strong><span className="chip">{changeLabel(unitHistory.pending, t)}</span></div></li> : null}
            {unitHistory.revisions.map((revision) => {
              const target = versionTarget(revision.after);
              return (
                <li className="git-timeline-item" key={revision.commit.id}>
                  <div className="git-item-head">
                    <strong>{revision.commit.authorName}</strong>
                    <span className="muted" title={formatTime(revision.commit.authoredAt)}>{relativeTime(revision.commit.authoredAt)}</span>
                    <span className="spacer" />
                    <span className="chip">{t(kindLabel[revision.kind])}</span>
                  </div>
                  <span className="git-item-subject">{revision.commit.subject}</span>
                  {revision.after.state === "invalid" ? <span className="git-feedback error">{t("git.invalidRecord", { message: revision.after.message })}</span> : null}
                  {target !== null ? <div className="git-diff"><ins>{target || t("common.empty")}</ins></div> : null}
                  {target !== null && onRestoreTarget ? <button className="link-button" type="button" onClick={() => onRestoreTarget(target)}><UiIcon icon="undo" size="xs" />{t("git.restore")}</button> : null}
                </li>
              );
            })}
            {unitHistory.revisions.length === 0 && !unitHistory.pending ? <li className="muted">{t("git.noHistory")}</li> : null}
            {unitHistory.truncated ? <li className="muted">{t("git.historyTruncated")}</li> : null}
          </ol>
        )}
      </Section>

      {mode === "advanced" ? <>
        <Section title={t("git.history")} icon="gitCommit" meta={log.length}>
          <ul className="git-list">
            {log.map((commit) => {
              const expanded = openCommit?.commit.id === commit.id;
              return (
                <li className="git-item" key={commit.id}>
                  <button className="git-commit" type="button" aria-expanded={expanded} onClick={() => void run("commit", async () => { setOpenCommit(expanded ? null : await gitCommitChanges(commit.id)); return null; })}>
                    <UiIcon icon={expanded ? "chevronDown" : "chevronRight"} size="xs" />
                    <span className="git-item-title">{commit.subject}</span>
                  </button>
                  <span className="git-item-meta"><span className="mono">{commit.id.slice(0, 8)}</span> · {commit.authorName} · <span title={formatTime(commit.authoredAt)}>{relativeTime(commit.authoredAt)}</span></span>
                  {expanded ? (
                    <ul className="git-list nested">
                      {openCommit.changes.length === 0 ? <li className="muted">{t("git.noTranslationChanges")}</li> : openCommit.changes.map((change) => {
                        const binding = changeBinding(change);
                        return <ChangeRow key={change.translationUnitId} change={change} selected={change.translationUnitId === selectedUnitId} onOpen={binding && onRevealBinding && change.kind !== "removed" ? () => onRevealBinding(binding) : undefined} />;
                      })}
                    </ul>
                  ) : null}
                </li>
              );
            })}
            {log.length === 0 ? <li className="muted">{t("git.noCommits")}</li> : null}
          </ul>
        </Section>

        <Section title={t("git.branches")} icon="gitBranch" meta={overview.runtime.version ?? t("git.runtimeUnavailable")}>
          <ul className="git-list">
            {branches.filter((branch) => !branch.remote).map((branch) => (
              <li className="git-item git-branch" key={branch.name}>
                {branch.current ? <span className="git-item-title"><UiIcon icon="check" size="xs" />{branch.name}</span> : <button className="link-button git-item-title" type="button" disabled={busy !== null || status.hasTranslationChanges} onClick={() => void run("switch", async () => { await gitSwitchBranch(branch.name); onWorkspaceChanged?.(); return t("git.switched", { name: branch.name }); })}>{branch.name}</button>}
                {branch.upstream ? <span className="git-item-meta mono">{branch.upstream}</span> : null}
              </li>
            ))}
          </ul>
          <form className="git-inline-form" onSubmit={(event) => { event.preventDefault(); const name = newBranch.trim(); void run("branch", async () => { await gitCreateBranch(name); setNewBranch(""); return t("git.branchCreated", { name }); }); }}>
            <input className="input" aria-label={t("git.newBranchName")} placeholder={t("git.newBranch")} value={newBranch} onChange={(event) => setNewBranch(event.target.value)} spellCheck={false} />
            <button className="button button-secondary" type="submit" disabled={busy !== null || newBranch.trim() === ""}>{t("common.create")}</button>
          </form>
          <label className="checkbox">
            <input type="checkbox" checked={overview.collaboration?.policy === "pullRequest"} disabled={busy !== null || !identityComplete} onChange={(event) => { const pullRequest = event.target.checked; void run("policy", async () => { await gitSetCollaboration(pullRequest ? "pullRequest" : "direct", pullRequest ? (status.branch ?? "main") : null); return pullRequest ? t("git.reviewPolicyOn", { branch: status.branch ?? "main" }) : t("git.reviewPolicyOff"); }); }} />
            {t("git.reviewPolicy", { branch: overview.collaboration?.mainBranch ?? status.branch ?? "main" })}
          </label>
          <p className="muted small">{t("git.runtime", { origin: overview.runtime.origin })}</p>
        </Section>

        <Section title={t("git.contributors")} icon="users" action={<button className="link-button" type="button" disabled={busy !== null} onClick={() => void run("contributors", async () => { setContributors(await gitContributors()); return null; })}>{t(contributors ? "git.recount" : "git.count")}</button>}>
          {contributors ? (
            <ul className="git-list">
              {contributors.map((contributor) => (
                <li className="git-item" key={`${contributor.name}<${contributor.email}>`}>
                  <div className="git-item-head"><strong>{contributor.name}</strong><span className="spacer" /><span className="muted" title={formatTime(contributor.lastAuthoredAt)}>{relativeTime(contributor.lastAuthoredAt)}</span></div>
                  <span className="git-item-meta">{t("git.contributorStats", { translated: contributor.translated, reviewed: contributor.reviewed })}</span>
                </li>
              ))}
              {contributors.length === 0 ? <li className="muted">{t("git.noContributors")}</li> : null}
            </ul>
          ) : <p className="muted">{t("git.contributorsHint")}</p>}
        </Section>

        <Section title={t("git.workingTree")} icon="folder" meta={status.files.length}>
          <ul className="git-list">
            {status.files.map((file) => <li className="git-item-head" key={file.path}><code className="git-file">{file.path}</code><span className="chip">{file.staged ? t("git.file.staged", { kind: t(fileKindLabel[file.kind]) }) : t(fileKindLabel[file.kind])}</span></li>)}
            {status.files.length === 0 ? <li className="muted">{t("git.workingTreeClean")}</li> : null}
          </ul>
        </Section>
      </> : null}
    </div>
  );
}
