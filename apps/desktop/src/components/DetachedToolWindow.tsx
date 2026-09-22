import { useEffect, useState } from "react";
import { currentProject, normalizeCommandError } from "../ipc";
import type { CommandError, ProjectSummaryDto } from "../types";
import { BottomPanel, type BottomPanelTab } from "./BottomPanel";
import { ErrorBanner } from "./ErrorBanner";
import { WindowChrome } from "./WindowChrome";
import { WorkbenchToolDock, type GitPresentationMode, type WorkbenchTool } from "./WorkbenchToolDock";
import { displayPathName } from "../pathDisplay";

type DetachedPanel = "search" | "ai" | "git" | "tasks" | "gitChanges" | "diagnostics";

function panelTitle(panel: DetachedPanel): string {
  return panel === "search" ? "Search" : panel === "ai" ? "AI" : panel === "git" ? "Git" : panel === "gitChanges" ? "Git Changes" : panel === "diagnostics" ? "Diagnostics" : "Tasks";
}

export function DetachedToolWindow({ panel }: { panel: DetachedPanel }) {
  const [project, setProject] = useState<ProjectSummaryDto | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [gitMode, setGitMode] = useState<GitPresentationMode>("collaboration");
  const [bottomTab, setBottomTab] = useState<BottomPanelTab>(panel === "gitChanges" ? "gitChanges" : panel === "diagnostics" ? "diagnostics" : "tasks");

  useEffect(() => {
    void currentProject().then(setProject).catch((caughtError: unknown) => setError(normalizeCommandError(caughtError)));
  }, []);

  return (
    <main className="app-shell editor-shell detached-tool-shell">
      <WindowChrome context={project ? displayPathName(project.repositoryRoot) : "Aeria"} projectName={project ? displayPathName(project.repositoryRoot) : "Aeria"} mode="workbench" />
      {error ? <ErrorBanner title="Detached tool unavailable" error={error} onDismiss={() => setError(null)} /> : null}
      <section className="detached-tool-panel">
        <header className="dock-header"><span className="dock-title">{panelTitle(panel)}</span><span className="dock-meta">shared session</span></header>
        <div className="detached-tool-body">
          {panel === "search" || panel === "ai" || panel === "git" ? <WorkbenchToolDock activeTool={panel as WorkbenchTool} gitMode={gitMode} selectedBinding={null} onGitModeChange={setGitMode} /> : <BottomPanel activeTab={bottomTab} onTabChange={setBottomTab} />}
        </div>
      </section>
    </main>
  );
}
