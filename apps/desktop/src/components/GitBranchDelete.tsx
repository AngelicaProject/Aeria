import { useEffect, useState } from "react";
import { AlertDialog } from "radix-ui";
import { gitDeleteBranch, gitDeleteRemoteBranch } from "../ipc";
import type { GitBranchDto } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

type GitBranchDeleteProps = {
  /** The branch to delete; the dialog is closed while null. */
  branch: GitBranchDto | null;
  onCancel: () => void;
  /** Called once the branch was deleted, before the operation reports it. */
  onDeleted?: (() => Promise<void> | void) | undefined;
  /** Runs the deletion as one of the dock's or the settings' operations. */
  run: (label: string, action: () => Promise<string>) => void;
};

/**
 * Confirms deleting a branch: a local branch, optionally with its upstream on
 * the remote, or a remote branch. Commits that exist on no other branch are
 * named, and the deletion then goes ahead only as an explicit "delete anyway".
 */
export function GitBranchDelete({ branch, onCancel, onDeleted, run }: GitBranchDeleteProps) {
  const { t } = useI18n();
  const [withUpstream, setWithUpstream] = useState(false);
  useEffect(() => setWithUpstream(false), [branch?.name]);
  if (!branch) return null;

  const upstream = !branch.remote && branch.lostWithUpstream !== null ? branch.upstream : null;
  const lost = branch.remote ? branch.lostCommits : withUpstream && branch.lostWithUpstream !== null ? branch.lostWithUpstream : branch.lostCommits;
  const message = branch.remote
    ? t("git.branch.deleteRemoteConfirm", { name: branch.name })
    : t("git.branch.deleteConfirm", { name: branch.name });
  const confirm = () => {
    const target = branch;
    const both = upstream !== null && withUpstream;
    onCancel();
    run("deleteBranch", async () => {
      if (target.remote) await gitDeleteRemoteBranch(target.name, lost > 0);
      else await gitDeleteBranch(target.name, both, lost > 0);
      const done = target.remote
        ? t("git.branch.deletedRemote", { name: target.name })
        : both ? t("git.branch.deletedBoth", { name: target.name, upstream: upstream ?? "" }) : t("git.branch.deleted", { name: target.name });
      await onDeleted?.();
      return done;
    });
  };

  return (
    <AlertDialog.Root open onOpenChange={(next) => { if (!next) onCancel(); }}>
      <AlertDialog.Portal>
        <AlertDialog.Overlay className="dialog-overlay" />
        <AlertDialog.Content className="dialog confirm-dialog">
          <AlertDialog.Title className="dialog-title">{t("git.branch.deleteTitle")}</AlertDialog.Title>
          <AlertDialog.Description className="dialog-description">{message}</AlertDialog.Description>
          {upstream ? (
            <label className="checkbox git-branch-delete-option">
              <input type="checkbox" checked={withUpstream} onChange={(event) => setWithUpstream(event.target.checked)} />
              <span>{t("git.branch.deleteUpstream", { upstream })}</span>
            </label>
          ) : null}
          {lost > 0 ? (
            <p className="git-feedback warning" role="status"><UiIcon icon="circleAlert" size="sm" /><span>{t("git.branch.deleteLoses", { count: lost })}</span></p>
          ) : null}
          <div className="dialog-actions">
            <AlertDialog.Cancel className="button button-secondary">{t("common.cancel")}</AlertDialog.Cancel>
            <AlertDialog.Action className="button button-danger" onClick={confirm}>{t(lost > 0 ? "git.branch.deleteAnyway" : "git.branch.delete")}</AlertDialog.Action>
          </div>
        </AlertDialog.Content>
      </AlertDialog.Portal>
    </AlertDialog.Root>
  );
}
