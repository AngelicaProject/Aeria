import type { SourceBinding } from "../types";

export type WorkbenchTool = "search" | "ai" | "git";
export type GitPresentationMode = "collaboration" | "advanced";

type WorkbenchToolDockProps = {
  activeTool: WorkbenchTool;
  gitMode: GitPresentationMode;
  selectedBinding: SourceBinding | null;
  onGitModeChange: (mode: GitPresentationMode) => void;
};

export function WorkbenchToolDock({ activeTool, gitMode, selectedBinding, onGitModeChange }: WorkbenchToolDockProps) {
  if (activeTool === "search") {
    return <section className="tool-dock-content" aria-label="Project search"><div className="tool-dock-empty"><strong>Project search not available in this build</strong><p>Search will use the project backend when it is available. Sheet Quick Find remains local to the Sheets tool.</p></div></section>;
  }
  if (activeTool === "ai") {
    return <section className="tool-dock-content" aria-label="AI tool"><div className="tool-dock-empty"><strong>AI not configured</strong><p>Configure translation assistance to use this workbench slot.</p>{selectedBinding ? <code>{selectedBinding.sheetName} · {selectedBinding.rowId}:{selectedBinding.subrowId} · col {selectedBinding.columnIndex}</code> : null}</div></section>;
  }

  return (
    <section className="tool-dock-content git-tool" aria-label="Git tool">
      <div className="git-mode-switch" role="tablist" aria-label="Git presentation mode">
        <button className={gitMode === "collaboration" ? "git-mode-button active" : "git-mode-button"} type="button" role="tab" aria-selected={gitMode === "collaboration"} onClick={() => onGitModeChange("collaboration")}>Collaboration</button>
        <button className={gitMode === "advanced" ? "git-mode-button active" : "git-mode-button"} type="button" role="tab" aria-selected={gitMode === "advanced"} onClick={() => onGitModeChange("advanced")}>Advanced Git</button>
      </div>
      <div className="tool-dock-empty"><strong>Git not available</strong><p>{gitMode === "collaboration" ? "Collaboration changes will appear here when Git is connected." : "Advanced Git presentation is ready for the shared Git state."}</p></div>
    </section>
  );
}
