import { memo } from "react";
import type { MessageKey } from "../i18n/translate";
import type { EditorContextDto, SourceBinding, UnitChangeDto } from "../types";
import { useI18n } from "../ui/i18n";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";
import { AngelicaPanel } from "./AngelicaPanel";
import { GitPanel } from "./GitPanel";
import type { GitCommitDto } from "../types";

export type WorkbenchTool = "search" | "ai" | "git";

type WorkbenchToolDockProps = {
  activeTool: WorkbenchTool;
  selectedBinding: SourceBinding | null;
  /** Opens one commit in a document tab; absent in detached windows. */
  onOpenCommit?: ((commit: GitCommitDto) => void) | undefined;
  selectedCommitId?: string | null;
  /** Opens Settings on the repository section. */
  onOpenRepositorySettings?: (() => void) | undefined;
  projectRevision?: number;
  selectedUnitId?: string | null;
  workspaceRevision?: number;
  onWorkspaceChanged?: () => void;
  onRestoreTarget?: (targetMacro: string) => void;
  pending?: { changes: UnitChangeDto[] | null; refresh: () => Promise<void> };
  onRevealBinding?: (binding: SourceBinding) => void;
  editorContext?: EditorContextDto | null;
  onOpenSettings?: () => void;
  onOpenGuide?: (tab: "glossary" | "guidance") => void;
};

export function toolTitle(tool: WorkbenchTool): MessageKey {
  return tool === "ai" ? "workbench.tool.ai" : tool === "git" ? "workbench.tool.git" : "workbench.tool.search";
}

/** Memoized: Angelica and Git transcripts are costly to re-render on unrelated workbench updates. */
export const WorkbenchToolDock = memo(function WorkbenchToolDock({ activeTool, selectedBinding, onOpenCommit, selectedCommitId, onOpenRepositorySettings, projectRevision, selectedUnitId, workspaceRevision, onWorkspaceChanged, onRestoreTarget, pending, onRevealBinding, editorContext, onOpenSettings, onOpenGuide }: WorkbenchToolDockProps) {
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
    return <AngelicaPanel editorContext={editorContext ?? null} onOpenSettings={onOpenSettings} onOpenGuide={onOpenGuide} onReveal={onRevealBinding} />;
  }

  return (
    <section className="tool-content git-tool" aria-label={t("workbench.tool.git")}>
      <GitPanel onOpenCommit={onOpenCommit} selectedCommitId={selectedCommitId} onOpenSettings={onOpenRepositorySettings} projectRevision={projectRevision} selectedUnitId={selectedUnitId ?? null} workspaceRevision={workspaceRevision ?? 0} onWorkspaceChanged={onWorkspaceChanged} pending={pending} onRevealBinding={onRevealBinding} />
    </section>
  );
});
