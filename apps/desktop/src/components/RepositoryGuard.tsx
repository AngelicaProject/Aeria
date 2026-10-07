import { useCallback, useEffect, useState } from "react";
import { save as saveNativeDialog } from "@tauri-apps/plugin-dialog";
import { gitInstallGuardWorkflow, gitOpenRulesSettings, gitRepositoryProtection, gitSaveRuleset, normalizeCommandError } from "../ipc";
import type { BranchRule, CommandError, GuardWorkflowDto, ProtectionDto, RuleStateDto, TagRule } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { IconButton } from "../ui/primitives/IconButton";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useStickyState } from "./GitShared";

const branchRuleLabels: Record<BranchRule, MessageKey> = {
  deletion: "git.guard.rule.deletion",
  forcePush: "git.guard.rule.forcePush",
  pullRequest: "git.guard.rule.pullRequest",
  guardCheck: "git.guard.rule.guardCheck",
};
const tagRuleLabels: Record<TagRule, MessageKey> = {
  creation: "git.guard.rule.creation",
  update: "git.guard.rule.update",
  deletion: "git.guard.rule.tagDeletion",
};

type RepositoryGuardProps = {
  /** The workflow as the panel last read it; `null` before. */
  workflow: GuardWorkflowDto | null;
  onWorkflow: (workflow: GuardWorkflowDto) => void;
  busy: boolean;
  /** Runs an operation with the panel's busy state and feedback. */
  run: (label: string, action: () => Promise<string | null>) => void;
  onError: (error: CommandError) => void;
};

/**
 * How the repository guards its main branch on GitHub: the Aeria Guard
 * workflow, the rules of the main branch, and the rules of pack release
 * tags. Shown while something is missing; the rules are imported on GitHub
 * from the files Aeria saves.
 */
export function RepositoryGuard({ workflow, onWorkflow, busy, run, onError }: RepositoryGuardProps) {
  const { t } = useI18n();
  const [protection, setProtection] = useState<ProtectionDto | null>(null);
  const [checking, setChecking] = useState(false);
  const [dismissed, setDismissed] = useStickyState("git.guardDismissed", false);

  const readProtection = useCallback(async (refresh: boolean) => {
    setChecking(true);
    try {
      setProtection(await gitRepositoryProtection(refresh));
    } catch {
      setProtection(null);
    } finally {
      setChecking(false);
    }
  }, []);
  const github = workflow?.github ?? false;
  useEffect(() => {
    if (github) void readProtection(false);
  }, [github, readProtection]);

  if (!workflow?.github || dismissed) return null;
  const workflowOk = workflow.state === "current";
  const branchOk = protection?.branch.state === "protected";
  const tagsOk = protection?.tags.state === "protected";
  if (workflowOk && branchOk && tagsOk) return null;
  const done = [workflowOk, branchOk, tagsOk].filter(Boolean).length;

  const saveRuleset = async (kind: "branch" | "tags") => {
    const path = await saveNativeDialog({
      defaultPath: kind === "branch" ? "aeria-main-branch.json" : "aeria-pack-releases.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    }).catch(() => null);
    if (!path) return;
    run("ruleset", async () => {
      await gitSaveRuleset(kind, path);
      return t("git.guard.rulesSaved");
    });
  };
  const openRules = () => void gitOpenRulesSettings().catch((caught: unknown) => onError(normalizeCommandError(caught)));

  const rulesRow = <Rule extends string>(label: string, state: RuleStateDto<Rule> | undefined, labels: Record<Rule, MessageKey>, kind: "branch" | "tags") => {
    const ok = state?.state === "protected";
    const detail = checking ? t("git.guard.checking")
      : !state ? t("git.guard.unknown")
        : state.state === "partial" ? t("git.guard.missingRules", { rules: state.missing.map((rule) => t(labels[rule])).join(", ") })
        : state.state === "missing" ? t("git.guard.noRules")
          : state.state === "unknown" ? t("git.guard.unknown")
            : t("git.guard.protected");
    return (
      <li className={ok ? "git-guard-row ok" : "git-guard-row"}>
        <UiIcon icon={ok ? "circleCheck" : !state || state.state === "unknown" ? "circleHelp" : "circleAlert"} size="sm" />
        <span className="git-guard-text"><strong>{label}</strong><span>{detail}</span></span>
        {ok ? null : (
          <span className="git-guard-actions">
            <button className="button button-secondary" type="button" disabled={busy} onClick={() => void saveRuleset(kind)}>{t("git.guard.saveRules")}</button>
            <button className="button button-ghost" type="button" onClick={openRules}>{t("git.guard.openRules")}</button>
          </span>
        )}
      </li>
    );
  };

  return (
    <details className="git-offer git-guard">
      <summary>
        <UiIcon icon="shieldAlert" size="sm" />
        <span className="git-offer-title">{t("git.guard.title")}</span>
        <span className="git-guard-count">{t("git.guard.count", { done: String(done), total: "3" })}</span>
        <IconButton icon="refreshCw" size="xs" label={t("git.guard.refresh")} disabled={checking} onClick={(event) => { event.preventDefault(); void readProtection(true); }} />
        <IconButton icon="x" size="xs" label={t("git.guard.dismiss")} onClick={(event) => { event.preventDefault(); setDismissed(true); }} />
      </summary>
      <ul className="git-guard-rows">
        <li className={workflowOk ? "git-guard-row ok" : "git-guard-row"}>
          <UiIcon icon={workflowOk ? "circleCheck" : "circleAlert"} size="sm" />
          <span className="git-guard-text">
            <strong>{t("git.guard.workflow")}</strong>
            <span>{t(!workflow.topLevel ? "git.guard.workflowSubfolder" : workflowOk ? "git.guard.workflowCurrent" : workflow.state === "different" ? "git.guard.workflowOutdated" : "git.guard.workflowMissing")}</span>
          </span>
          {workflow.topLevel && !workflowOk ? (
            <span className="git-guard-actions">
              <button className="button button-secondary" type="button" disabled={busy} onClick={() => run("guardWorkflow", async () => { onWorkflow(await gitInstallGuardWorkflow()); return t("git.guard.workflowWritten"); })}>
                {t(workflow.state === "different" ? "git.guard.update" : "git.guard.add")}
              </button>
            </span>
          ) : null}
        </li>
        {rulesRow(t("git.guard.branch", { branch: workflow.branch }), protection?.branch, branchRuleLabels, "branch")}
        {rulesRow(t("git.guard.tags"), protection?.tags, tagRuleLabels, "tags")}
      </ul>
    </details>
  );
}
