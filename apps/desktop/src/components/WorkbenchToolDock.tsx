import type { MessageKey } from "../i18n/translate";
import type { SourceBinding, UnitChangeDto } from "../types";
import { useI18n } from "../ui/i18n";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";
import { GitPanel } from "./GitPanel";

export type WorkbenchTool = "search" | "ai" | "git";
export type GitPresentationMode = "collaboration" | "advanced";

type WorkbenchToolDockProps = {
  activeTool: WorkbenchTool;
  gitMode: GitPresentationMode;
  selectedBinding: SourceBinding | null;
  onGitModeChange: (mode: GitPresentationMode) => void;
  selectedUnitId?: string | null;
  workspaceRevision?: number;
  onWorkspaceChanged?: () => void;
  onRestoreTarget?: (targetMacro: string) => void;
  pending?: { changes: UnitChangeDto[] | null; refresh: () => Promise<void> };
  onRevealBinding?: (binding: SourceBinding) => void;
};

export function toolTitle(tool: WorkbenchTool): MessageKey {
  return tool === "ai" ? "workbench.tool.ai" : tool === "git" ? "workbench.tool.git" : "workbench.tool.search";
}

export function WorkbenchToolDock({ activeTool, gitMode, selectedBinding, onGitModeChange, selectedUnitId, workspaceRevision, onWorkspaceChanged, onRestoreTarget, pending, onRevealBinding }: WorkbenchToolDockProps) {
  const { t } = useI18n();
  if (activeTool === "search") {
    return (
      <section className="tool-content" aria-label={t("tool.searchLabel")}>
        <div className="empty-state">
          <UiIcon icon="search" size="xl" />
          <strong>{t("tool.searchUnavailable")}</strong>
          <p>{t("tool.searchHint")}</p>
        </div>
      </section>
    );
  }
  if (activeTool === "ai") {
    return (
      <section className="tool-content" aria-label={t("workbench.tool.ai")}>
        <div className="empty-state">
          <UiIcon icon="sparkles" size="xl" />
          <strong>{t("tool.aiUnavailable")}</strong>
          <p>{t("tool.aiHint")}</p>
          {selectedBinding ? <code className="chip mono">{t("common.cellLocation", { sheet: selectedBinding.sheetName, row: String(selectedBinding.rowId), subrow: String(selectedBinding.subrowId), column: String(selectedBinding.columnIndex) })}</code> : null}
        </div>
      </section>
    );
  }

  return (
    <section className="tool-content git-tool" aria-label={t("workbench.tool.git")}>
      <div className="git-mode">
        <Segmented
          label={t("tool.gitView")}
          value={gitMode}
          onChange={onGitModeChange}
          options={[{ value: "collaboration", label: t("tool.gitCollaboration") }, { value: "advanced", label: t("tool.gitAdvanced") }]}
        />
      </div>
      <GitPanel mode={gitMode} selectedUnitId={selectedUnitId ?? null} workspaceRevision={workspaceRevision ?? 0} onWorkspaceChanged={onWorkspaceChanged} onRestoreTarget={onRestoreTarget} pending={pending} onRevealBinding={onRevealBinding} />
    </section>
  );
}
