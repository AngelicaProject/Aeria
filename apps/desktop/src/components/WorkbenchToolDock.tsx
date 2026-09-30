import { memo } from "react";
import type { MessageKey } from "../i18n/translate";
import type { EntryChangeDto, SourceBinding } from "../types";
import { useI18n } from "../ui/i18n";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";
import { GitPanel } from "./GitPanel";
import type { GitCommitDto } from "../types";

export type WorkbenchTool = "search" | "git";

type WorkbenchToolDockProps = {
  activeTool: WorkbenchTool;
  selectedBinding: SourceBinding | null;
  /** Opens one commit in a document tab; absent in detached windows. */
  onOpenCommit?: ((commit: GitCommitDto) => void) | undefined;
  selectedCommitId?: string | null;
  /** Opens Settings on the repository section. */
  onOpenRepositorySettings?: (() => void) | undefined;
  projectRevision?: number;
  selectedKey?: string | null;
  workspaceRevision?: number;
  onWorkspaceChanged?: () => void;
  onRestoreTarget?: (targetMacro: string) => void;
  pending?: { changes: EntryChangeDto[] | null; refresh: () => Promise<void> };
  onRevealBinding?: (binding: SourceBinding) => void;
};

export function toolTitle(tool: WorkbenchTool): MessageKey {
  return tool === "git" ? "workbench.tool.git" : "workbench.tool.search";
}

/** Memoized: the Git panel is costly to re-render on unrelated workbench updates. */
export const WorkbenchToolDock = memo(function WorkbenchToolDock({ activeTool, selectedBinding, onOpenCommit, selectedCommitId, onOpenRepositorySettings, projectRevision, selectedKey, workspaceRevision, onWorkspaceChanged, onRestoreTarget, pending, onRevealBinding }: WorkbenchToolDockProps) {
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
  return (
    <section className="tool-content git-tool" aria-label={t("workbench.tool.git")}>
      <GitPanel onOpenCommit={onOpenCommit} selectedCommitId={selectedCommitId} onOpenSettings={onOpenRepositorySettings} projectRevision={projectRevision} selectedKey={selectedKey ?? null} workspaceRevision={workspaceRevision ?? 0} onWorkspaceChanged={onWorkspaceChanged} pending={pending} onRevealBinding={onRevealBinding} />
    </section>
  );
});
