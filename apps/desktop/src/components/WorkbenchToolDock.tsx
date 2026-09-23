import type { SourceBinding, UnitChangeDto } from "../types";
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

export function toolTitle(tool: WorkbenchTool): string {
  return tool === "ai" ? "AI assist" : tool === "git" ? "Git" : "Search";
}

export function WorkbenchToolDock({ activeTool, gitMode, selectedBinding, onGitModeChange, selectedUnitId, workspaceRevision, onWorkspaceChanged, onRestoreTarget, pending, onRevealBinding }: WorkbenchToolDockProps) {
  if (activeTool === "search") {
    return (
      <section className="tool-content" aria-label="Project search">
        <div className="empty-state">
          <UiIcon icon="search" size="xl" />
          <strong>Project search isn't available yet</strong>
          <p>Search will use the project backend once it exists. To find a sheet by name, use the sheet filter (Ctrl+F).</p>
        </div>
      </section>
    );
  }
  if (activeTool === "ai") {
    return (
      <section className="tool-content" aria-label="AI assist">
        <div className="empty-state">
          <UiIcon icon="sparkles" size="xl" />
          <strong>AI assist isn't configured</strong>
          <p>Translation suggestions will appear here once a provider is set up.</p>
          {selectedBinding ? <code className="chip mono">{selectedBinding.sheetName} {selectedBinding.rowId}:{selectedBinding.subrowId} · col {selectedBinding.columnIndex}</code> : null}
        </div>
      </section>
    );
  }

  return (
    <section className="tool-content git-tool" aria-label="Git">
      <div className="git-mode">
        <Segmented
          label="Git view"
          value={gitMode}
          onChange={onGitModeChange}
          options={[{ value: "collaboration", label: "Collaboration" }, { value: "advanced", label: "Advanced" }]}
        />
      </div>
      <GitPanel mode={gitMode} selectedUnitId={selectedUnitId ?? null} workspaceRevision={workspaceRevision ?? 0} onWorkspaceChanged={onWorkspaceChanged} onRestoreTarget={onRestoreTarget} pending={pending} onRevealBinding={onRevealBinding} />
    </section>
  );
}
