import { useMemo, useState } from "react";
import { Popover } from "radix-ui";
import type { GitBranchDto } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

type GitBranchPickerProps = {
  branches: readonly GitBranchDto[];
  current: string | null;
  mainBranch: string | null;
  /** Why switching is not possible right now, such as uncommitted translations. */
  switchBlocked: string | null;
  disabled: boolean;
  blockLabels: Readonly<Record<NonNullable<GitBranchDto["blocked"]>, MessageKey>>;
  onSwitch: (name: string) => void;
  onCreate: (name: string) => void;
};

/** `origin/feature` → `feature`: the local name a remote branch checks out as. */
function localName(remoteBranch: string): string {
  const slash = remoteBranch.indexOf("/");
  return slash < 0 ? remoteBranch : remoteBranch.slice(slash + 1);
}

/**
 * The branch switcher: local branches (current first, then the main branch,
 * then by name) and remote branches without a local one, which check out as
 * tracking branches. It filters as you type and creates a branch from HEAD.
 */
export function GitBranchPicker({ branches, current, mainBranch, switchBlocked, disabled, blockLabels, onSwitch, onCreate }: GitBranchPickerProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);

  const { local, remote } = useMemo(() => {
    const locals = branches.filter((branch) => !branch.remote);
    const names = new Set(locals.map((branch) => branch.name));
    const rank = (branch: GitBranchDto) => (branch.current ? 0 : branch.name === mainBranch ? 1 : 2);
    return {
      local: [...locals].sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name)),
      remote: branches
        .filter((branch) => branch.remote && !branch.name.endsWith("/HEAD") && !names.has(localName(branch.name)))
        .sort((a, b) => a.name.localeCompare(b.name)),
    };
  }, [branches, mainBranch]);

  const needle = query.trim().toLocaleLowerCase();
  const matches = (branch: GitBranchDto) => needle === "" || branch.name.toLocaleLowerCase().includes(needle);
  const shownLocal = local.filter(matches);
  const shownRemote = remote.filter(matches);
  const newName = query.trim();
  const canCreate = newName !== "" && !branches.some((branch) => branch.name === newName || (branch.remote && localName(branch.name) === newName));

  const choose = (name: string) => {
    setOpen(false);
    setQuery("");
    onSwitch(name);
  };
  const create = () => {
    if (!canCreate) return;
    setOpen(false);
    setQuery("");
    setCreating(false);
    onCreate(newName);
  };

  const item = (branch: GitBranchDto, target: string) => {
    const blocked = branch.blocked ? t(blockLabels[branch.blocked]) : null;
    const unavailable = !branch.current && (blocked !== null || switchBlocked !== null);
    return (
      <li key={branch.name}>
        <button
          type="button"
          className={branch.current ? "git-branch-item current" : "git-branch-item"}
          disabled={disabled || unavailable || branch.current}
          aria-current={branch.current ? "true" : undefined}
          title={blocked ?? (branch.current ? undefined : switchBlocked) ?? branch.name}
          onClick={() => choose(target)}
        >
          <span className="git-branch-check">{branch.current ? <UiIcon icon="check" size="xs" /> : null}</span>
          <span className="git-branch-text">
            <span className="git-branch-name">
              <span className="mono">{branch.name}</span>
              {branch.name === mainBranch ? <span className="git-branch-badge">{t("git.branch.main")}</span> : null}
              {branch.merged ? <span className="git-branch-badge muted">{t("git.branch.merged")}</span> : null}
            </span>
            {blocked ? <span className="git-branch-hint">{blocked}</span> : branch.upstream ? <span className="git-branch-hint mono">{branch.upstream}</span> : null}
          </span>
        </button>
      </li>
    );
  };

  return (
    <Popover.Root open={open} onOpenChange={(next) => { setOpen(next); if (!next) { setQuery(""); setCreating(false); } }}>
      <Popover.Trigger asChild>
        <button className="git-branch-trigger" type="button" disabled={disabled && !open} title={t("git.switchBranch")}>
          <strong>{current ?? t("git.detachedHead")}</strong>
          <UiIcon icon="chevronDown" size="xs" />
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content className="menu-content git-branch-menu" align="start" sideOffset={6} collisionPadding={8}>
          <label className="git-branch-search">
            <UiIcon icon="search" size="xs" />
            <input
              className="input"
              autoFocus
              value={query}
              placeholder={t(creating ? "git.branch.newName" : "git.branch.filter")}
              aria-label={t(creating ? "git.branch.newName" : "git.branch.filter")}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key !== "Enter") return;
                if (creating) create();
                else if (shownLocal.length + shownRemote.length === 1) {
                  const only = shownLocal[0] ?? shownRemote[0];
                  if (only && !only.current) choose(only.remote ? localName(only.name) : only.name);
                }
              }}
            />
          </label>
          {switchBlocked ? <p className="git-branch-note">{switchBlocked}</p> : null}
          {creating ? null : (
            <div className="git-branch-lists">
              {shownLocal.length > 0 ? <>
                <div className="menu-label git-branch-label">{t("git.branch.local")}</div>
                <ul>{shownLocal.map((branch) => item(branch, branch.name))}</ul>
              </> : null}
              {shownRemote.length > 0 ? <>
                <div className="menu-label git-branch-label">{t("git.branch.remote")}</div>
                <ul>{shownRemote.map((branch) => item(branch, localName(branch.name)))}</ul>
              </> : null}
              {shownLocal.length + shownRemote.length === 0 ? <p className="git-branch-note">{t("git.branch.none")}</p> : null}
            </div>
          )}
          <div className="git-branch-foot">
            {creating ? <>
              <span className="git-branch-note">{t("git.branch.createHint", { branch: current ?? "HEAD" })}</span>
              <button className="button button-ghost" type="button" onClick={() => { setCreating(false); setQuery(""); }}>{t("common.cancel")}</button>
              <button className="button button-primary" type="button" disabled={disabled || !canCreate} onClick={create}>{t("git.branch.create")}</button>
            </> : (
              <button className="button button-ghost" type="button" disabled={disabled} onClick={() => (canCreate ? create() : setCreating(true))}>
                <UiIcon icon="plus" size="sm" />{canCreate && newName !== "" ? t("git.branch.createNamed", { name: newName }) : t("git.branch.new")}
              </button>
            )}
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
