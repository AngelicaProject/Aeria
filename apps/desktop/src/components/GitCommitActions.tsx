import { useState, type ReactNode } from "react";
import { AlertDialog } from "radix-ui";
import { gitCreateBranchAt, gitOpenCommit, gitRevert, normalizeCommandError } from "../ipc";
import type { CommandError, GitCommitDto } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { RightClickMenu, copyText } from "../ui/primitives/RightClickMenu";
import { ConfirmDialog } from "./ConfirmDialog";

/** One thing to do with a commit, for a menu or a toolbar. */
export type CommitAction = {
  id: string;
  icon: UiIconName;
  label: MessageKey;
  params?: Record<string, string>;
  run: () => void;
  danger?: boolean;
  disabled?: boolean;
};

/**
 * What can be done with a commit, as Git tools offer it: copy its ID or
 * subject, open it on the hosting service, start a branch at it, and revert
 * it. Creating a branch and reverting ask first and report back; `onChanged`
 * runs after either changed the working tree.
 */
export function useCommitActions(onChanged: () => void) {
  const { t } = useI18n();
  const [reverting, setReverting] = useState<GitCommitDto | null>(null);
  const [branching, setBranching] = useState<GitCommitDto | null>(null);
  const [branchName, setBranchName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const run = async (action: () => Promise<string | null>) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      setNotice(await action());
    } catch (caught) {
      setError(normalizeCommandError(caught));
    } finally {
      setBusy(false);
    }
  };

  /**
   * The actions of a commit; `remote` names the sync remote when it has
   * pages for commits, and `undo` undoes the commit when it is the last one
   * and only local.
   */
  const actionsFor = (commit: GitCommitDto, options: { remote: string | null; undo?: (() => void) | undefined }): CommitAction[] => [
    { id: "copyId", icon: "copy", label: "commit.action.copyId", run: () => copyText(commit.id) },
    { id: "copySubject", icon: "copy", label: "commit.action.copySubject", run: () => copyText(commit.subject) },
    ...(options.remote ? [{ id: "open", icon: "externalLink" as const, label: "commit.action.open" as const, params: { remote: options.remote }, run: () => void run(async () => { await gitOpenCommit(commit.id); return null; }) }] : []),
    { id: "branch", icon: "gitBranch", label: "commit.action.branch", disabled: busy, run: () => { setBranchName(""); setBranching(commit); } },
    ...(options.undo ? [{ id: "undo", icon: "undo" as const, label: "git.undoLastCommit" as const, disabled: busy, run: options.undo }] : []),
    { id: "revert", icon: "undo", label: "commit.action.revert", danger: true, disabled: busy || commit.parents.length > 1, run: () => setReverting(commit) },
  ];

  const feedback = <>
    {error ? <div className="git-feedback error" role="alert"><UiIcon icon="circleAlert" size="sm" /><span>{error.message}</span></div> : null}
    {notice ? <div className="git-feedback" role="status"><UiIcon icon="info" size="sm" /><span>{notice}</span></div> : null}
  </>;

  const dialogs = <>
    <ConfirmDialog
      open={reverting !== null}
      title={t("commit.revert.title", { subject: reverting?.subject ?? "" })}
      message={t("commit.revert.message")}
      confirmLabel={t("commit.action.revert")}
      cancelLabel={t("common.cancel")}
      onKeepEditing={() => setReverting(null)}
      onDiscard={() => {
        const commit = reverting;
        setReverting(null);
        if (commit) void run(async () => { const created = await gitRevert(commit.id); onChanged(); return t("commit.revert.done", { id: created.id.slice(0, 8) }); });
      }}
    />
    <AlertDialog.Root open={branching !== null} onOpenChange={(next) => { if (!next) setBranching(null); }}>
      <AlertDialog.Portal>
        <AlertDialog.Overlay className="dialog-overlay" />
        <AlertDialog.Content className="dialog confirm-dialog">
          <AlertDialog.Title className="dialog-title">{t("commit.branch.title")}</AlertDialog.Title>
          <AlertDialog.Description className="dialog-description">{t("commit.branch.message", { subject: branching?.subject ?? "", id: branching?.id.slice(0, 8) ?? "" })}</AlertDialog.Description>
          <form
            className="git-branch-form"
            onSubmit={(event) => {
              event.preventDefault();
              const commit = branching;
              const name = branchName.trim();
              if (!commit || !name) return;
              setBranching(null);
              void run(async () => { await gitCreateBranchAt(name, commit.id); onChanged(); return t("git.switched", { name }); });
            }}
          >
            <input className="input" autoFocus value={branchName} placeholder={t("commit.branch.name")} aria-label={t("commit.branch.name")} onChange={(event) => setBranchName(event.target.value)} />
            <div className="dialog-actions">
              <AlertDialog.Cancel className="button button-secondary">{t("common.cancel")}</AlertDialog.Cancel>
              <button className="button button-primary" type="submit" disabled={branchName.trim() === ""}>{t("commit.branch.create")}</button>
            </div>
          </form>
        </AlertDialog.Content>
      </AlertDialog.Portal>
    </AlertDialog.Root>
  </>;

  return { actionsFor, dialogs, feedback, busy };
}

/** A right-click menu of commit actions around `children`. */
export function CommitMenu({ actions, children }: { actions: readonly CommitAction[]; children: ReactNode }) {
  const { t } = useI18n();
  return (
    <RightClickMenu
      entries={() => actions.flatMap((action) => [
        ...(action.id === "branch" ? [{ id: "separator", separator: true } as const] : []),
        { id: action.id, icon: action.icon, label: t(action.label, action.params), run: action.run, ...(action.danger ? { danger: true } : {}), ...(action.disabled ? { disabled: true } : {}) },
      ])}
    >
      {children}
    </RightClickMenu>
  );
}
