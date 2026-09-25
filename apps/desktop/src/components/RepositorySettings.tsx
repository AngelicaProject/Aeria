import { useCallback, useEffect, useState } from "react";
import {
  gitBranches,
  gitDeleteBranch,
  gitOverview,
  gitRemoteBranches,
  gitRemoveRemote,
  gitSetIdentity,
  gitSetMainBranch,
  gitSetRemote,
  gitSetUpstream,
  normalizeCommandError,
} from "../ipc";
import type { CommandError, GitBranchDto, GitFileKind, GitOverviewDto } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { Select } from "../ui/primitives/Select";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ConfirmDialog } from "./ConfirmDialog";

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

const PROJECT_FILES = ["aeria-pack.json", "aeria-fonts.json", "aeria-glossary.csv", "aeria-guidance.md", "aeria-collaboration.json", ".gitattributes", ".github/workflows/harmonia-feed.yml"];

/** Files a checkpoint does not commit: neither translations nor project files. */
function isOtherFile(path: string): boolean {
  return !path.startsWith(".aeria/") && !path.startsWith("fonts/") && !PROJECT_FILES.includes(path);
}

/** Loads the Git overview for a settings entry and reloads it after each action. */
function useRepository() {
  const [overview, setOverview] = useState<GitOverviewDto | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const load = useCallback(async () => {
    try {
      setOverview(await gitOverview());
    } catch (caught) {
      setError(normalizeCommandError(caught));
    }
  }, []);
  useEffect(() => { void load(); }, [load]);
  const run = async (label: string, action: () => Promise<string | null | void>) => {
    setBusy(label);
    setError(null);
    setNotice(null);
    try {
      const result = await action();
      if (typeof result === "string") setNotice(result);
      await load();
    } catch (caught) {
      setError(normalizeCommandError(caught));
    } finally {
      setBusy(null);
    }
  };
  const feedback = <>
    {error ? <div className="git-feedback error" role="alert"><UiIcon icon="circleAlert" size="sm" /><span>{error.message}</span></div> : null}
    {notice ? <div className="git-feedback" role="status"><UiIcon icon="info" size="sm" /><span>{notice}</span></div> : null}
  </>;
  return { overview, busy, run, feedback };
}

/** Remote repositories: edit the address, remove, add. */
export function RemotesSetting() {
  const { t } = useI18n();
  const { overview, busy, run, feedback } = useRepository();
  const [urls, setUrls] = useState<Record<string, string>>({});
  const [draft, setDraft] = useState({ name: "", url: "" });
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  if (!overview?.repository) return feedback;
  const locked = busy !== null;
  return (
    <div className="repository-setting">
      {feedback}
      {overview.remotes.length === 0 ? <p className="muted">{t("git.noRemote")}</p> : (
        <ul className="repository-remotes">
          {overview.remotes.map((remote) => {
            const value = urls[remote.name] ?? remote.url;
            return (
              <li key={remote.name} className="repository-remote">
                <span className="mono repository-remote-name">{remote.name}</span>
                <input className="input" value={value} spellCheck={false} disabled={locked} aria-label={t("repository.remoteUrl", { name: remote.name })} onChange={(event) => setUrls((current) => ({ ...current, [remote.name]: event.target.value }))} />
                <button className="button button-secondary" type="button" disabled={locked || value.trim() === "" || value === remote.url} onClick={() => void run("url", async () => { await gitSetRemote(remote.name, value.trim()); return t("repository.remoteSaved", { name: remote.name }); })}>{t("common.save")}</button>
                <button className="button button-ghost" type="button" disabled={locked} onClick={() => setConfirmRemove(remote.name)}>{t("repository.removeRemote")}</button>
              </li>
            );
          })}
        </ul>
      )}
      <form className="repository-remote" onSubmit={(event) => { event.preventDefault(); const next = { name: draft.name.trim() || "origin", url: draft.url.trim() }; void run("add", async () => { await gitSetRemote(next.name, next.url); setDraft({ name: "", url: "" }); return t("repository.remoteAdded", { name: next.name }); }); }}>
        <input className="input repository-remote-name" value={draft.name} placeholder={overview.remotes.length === 0 ? "origin" : t("repository.remoteName")} aria-label={t("repository.remoteName")} spellCheck={false} disabled={locked} onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
        <input className="input" value={draft.url} placeholder={t("git.remotePlaceholder")} aria-label={t("git.remoteUrl")} spellCheck={false} disabled={locked} onChange={(event) => setDraft({ ...draft, url: event.target.value })} />
        <button className="button button-secondary" type="submit" disabled={locked || draft.url.trim() === ""}>{t("git.addRemote")}</button>
      </form>
      <ConfirmDialog
        open={confirmRemove !== null}
        message={t("repository.removeRemoteConfirm", { name: confirmRemove ?? "" })}
        confirmLabel={t("repository.removeRemote")}
        cancelLabel={t("common.cancel")}
        onKeepEditing={() => setConfirmRemove(null)}
        onDiscard={() => { const name = confirmRemove; setConfirmRemove(null); if (name) void run("remove", async () => { await gitRemoveRemote(name); return t("repository.remoteRemoved", { name }); }); }}
      />
    </div>
  );
}

/** The remote branch the current branch syncs with. */
export function UpstreamSetting() {
  const { t } = useI18n();
  const { overview, busy, run, feedback } = useRepository();
  const [branches, setBranches] = useState<string[] | null>(null);
  const [choice, setChoice] = useState("");
  const status = overview?.repository;
  if (!status) return feedback;
  const locked = busy !== null;
  return (
    <div className="repository-setting">
      {feedback}
      <p className="field-hint">{t("repository.upstreamOf", { branch: status.branch ?? t("git.detachedHead") })}</p>
      {status.head === null ? <p className="git-hint">{t("repository.upstreamUnborn", { branch: status.branch ?? "" })}</p>
        : branches !== null && branches.length === 0 ? <p className="git-hint">{t("repository.upstreamEmptyRemote", { branch: status.branch ?? "" })}</p>
        : !status.upstream && branches === null ? <p className="field-hint">{t("repository.upstreamAutomatic")}</p> : null}
      <div className="export-inline">
        {branches === null ? <span className="mono">{status.upstream ?? t("repository.noUpstream")}</span> : (
          <Select<string> value={choice || status.upstream || ""} label={t("repository.upstream")} disabled={locked || branches.length === 0} onChange={setChoice} options={[...(status.upstream ? [] : [{ value: "", label: t("repository.noUpstream") }]), ...branches.map((name) => ({ value: name, label: name }))]} />
        )}
        {branches === null ? (
          <button className="button button-secondary" type="button" disabled={locked || overview.remotes.length === 0 || !status.branch || status.head === null} onClick={() => void run("fetch", async () => { setBranches(await gitRemoteBranches()); return null; })}>{t(busy === "fetch" ? "repository.fetching" : "repository.changeUpstream")}</button>
        ) : (
          <button className="button button-primary" type="button" disabled={locked || branches.length === 0 || choice === "" || choice === status.upstream} onClick={() => void run("upstream", async () => { await gitSetUpstream(choice); setBranches(null); setChoice(""); return t("repository.upstreamSet", { upstream: choice }); })}>{t("repository.useUpstream")}</button>
        )}
      </div>
    </div>
  );
}

/** The branch contributions are reviewed into. */
export function MainBranchSetting() {
  const { t } = useI18n();
  const { overview, busy, run, feedback } = useRepository();
  const [draft, setDraft] = useState<string | null>(null);
  const collaboration = overview?.collaboration;
  if (!overview?.repository || !collaboration) return feedback;
  const value = draft ?? collaboration.configuredMainBranch ?? "";
  return (
    <div className="repository-setting">
      {feedback}
      {collaboration.error ? <p className="git-hint warn">{collaboration.error}</p> : null}
      <p className="field-hint">{collaboration.configuredMainBranch ? t("repository.mainConfigured", { branch: collaboration.configuredMainBranch }) : collaboration.mainBranch ? t("repository.mainDetected", { branch: collaboration.mainBranch }) : t("repository.mainUnknown")}</p>
      <form className="export-inline" onSubmit={(event) => { event.preventDefault(); void run("main", async () => { await gitSetMainBranch(value.trim() === "" ? null : value.trim()); setDraft(null); return t("repository.mainSaved"); }); }}>
        <input className="input" value={value} placeholder={collaboration.mainBranch ?? "main"} aria-label={t("repository.mainBranch")} spellCheck={false} disabled={busy !== null} onChange={(event) => setDraft(event.target.value)} />
        <button className="button button-secondary" type="submit" disabled={busy !== null || draft === null || collaboration.error !== null && value.trim() === ""}>{t("common.save")}</button>
      </form>
    </div>
  );
}

/** The translator name commits are signed with. */
export function IdentitySetting() {
  const { t } = useI18n();
  const { overview, busy, run, feedback } = useRepository();
  const [draft, setDraft] = useState<{ name: string; email: string; global: boolean } | null>(null);
  if (!overview?.repository) return feedback;
  const value = draft ?? { name: overview.identity?.name ?? "", email: overview.identity?.email ?? "", global: overview.identity?.nameScope === "global" };
  return (
    <div className="repository-setting">
      {feedback}
      <form className="repository-identity" onSubmit={(event) => { event.preventDefault(); void run("identity", async () => { await gitSetIdentity(value.name, value.email.trim() === "" ? null : value.email, value.global); setDraft(null); return t("repository.identitySaved"); }); }}>
        <input className="input" value={value.name} placeholder={t("git.name")} aria-label={t("git.identityName")} disabled={busy !== null} onChange={(event) => setDraft({ ...value, name: event.target.value })} />
        <input className="input" value={value.email} placeholder={t("git.email")} aria-label={t("git.identityEmail")} disabled={busy !== null} onChange={(event) => setDraft({ ...value, email: event.target.value })} />
        <label className="checkbox"><input type="checkbox" checked={value.global} disabled={busy !== null} onChange={(event) => setDraft({ ...value, global: event.target.checked })} />{t("git.identityGlobal")}</label>
        <button className="button button-secondary" type="submit" disabled={busy !== null || draft === null || value.name.trim() === ""}>{t("common.save")}</button>
      </form>
    </div>
  );
}

/** Working-tree files outside the project, which Aeria does not commit. */
export function OtherFilesSetting() {
  const { t } = useI18n();
  const { overview, feedback } = useRepository();
  const files = overview?.repository?.files.filter((file) => isOtherFile(file.path)) ?? [];
  return (
    <div className="repository-setting">
      {feedback}
      {files.length === 0 ? <p className="muted">{t("repository.noOtherFiles")}</p> : (
        <ul className="repository-files">
          {files.map((file) => <li key={file.path}><code>{file.path}</code><span className="chip">{file.staged ? t("git.file.staged", { kind: t(fileKindLabel[file.kind]) }) : t(fileKindLabel[file.kind])}</span></li>)}
        </ul>
      )}
      {overview ? <p className="muted small">{t("git.runtime", { origin: overview.runtime.origin })}{overview.runtime.version ? ` · ${overview.runtime.version}` : ""}</p> : null}
    </div>
  );
}

/** Local branches other than the current one, deleted only on request. */
export function BranchesSetting() {
  const { t } = useI18n();
  const { overview, busy, run, feedback } = useRepository();
  const [branches, setBranches] = useState<GitBranchDto[] | null>(null);
  const [confirm, setConfirm] = useState<GitBranchDto | null>(null);
  const reload = useCallback(async () => setBranches(await gitBranches()), []);
  useEffect(() => { void reload().catch(() => setBranches([])); }, [reload]);
  if (!overview?.repository) return feedback;
  const main = overview.collaboration?.mainBranch ?? null;
  const others = (branches ?? []).filter((branch) => !branch.remote && !branch.current && branch.name !== main);
  return (
    <div className="repository-setting">
      {feedback}
      {branches === null ? <p className="muted">{t("common.loading")}</p> : others.length === 0 ? <p className="muted">{t("repository.noOtherBranches")}</p> : (
        <ul className="repository-branch-list">
          {others.map((branch) => (
            <li key={branch.name}>
              <code>{branch.name}</code>
              <span className={branch.merged ? "chip" : "chip chip-warn"}>{t(branch.merged ? "repository.branchMerged" : "repository.branchUnmerged", { branch: main ?? "main" })}</span>
              <button className="button button-ghost" type="button" disabled={busy !== null} onClick={() => setConfirm(branch)}>{t("repository.deleteBranch")}</button>
            </li>
          ))}
        </ul>
      )}
      <ConfirmDialog
        open={confirm !== null}
        message={confirm ? t(confirm.merged ? "repository.deleteMergedConfirm" : "repository.deleteUnmergedConfirm", { name: confirm.name, branch: main ?? "main" }) : ""}
        confirmLabel={t("repository.deleteBranch")}
        cancelLabel={t("common.cancel")}
        onKeepEditing={() => setConfirm(null)}
        onDiscard={() => { const branch = confirm; setConfirm(null); if (branch) void run("delete", async () => { await gitDeleteBranch(branch.name, !branch.merged); await reload(); return t("repository.branchDeleted", { name: branch.name }); }); }}
      />
    </div>
  );
}
