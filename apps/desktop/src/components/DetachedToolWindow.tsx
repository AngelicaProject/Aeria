import { useEffect, useState } from "react";
import { currentProject, normalizeCommandError } from "../ipc";
import type { CommandError, ProjectSummaryDto } from "../types";
import { BottomPanel, type BottomPanelTab } from "./BottomPanel";
import { ErrorBanner } from "./ErrorBanner";
import { WindowChrome } from "./WindowChrome";
import { WorkbenchToolDock, toolTitle, type GitPresentationMode, type WorkbenchTool } from "./WorkbenchToolDock";
import { displayPathName } from "../pathDisplay";

export type DetachedPanel = "search" | "ai" | "git" | "tasks" | "gitChanges" | "diagnostics";

export function isDetachedPanel(value: string | null): value is DetachedPanel {
  return value === "search" || value === "ai" || value === "git" || value === "tasks" || value === "gitChanges" || value === "diagnostics";
}

export function detachedPanelTitle(panel: DetachedPanel): string {
  if (panel === "search" || panel === "ai" || panel === "git") return toolTitle(panel);
  return panel === "gitChanges" ? "Git changes" : panel === "diagnostics" ? "Diagnostics" : "Tasks";
}

export function DetachedToolWindow({ panel }: { panel: DetachedPanel }) {
  const [project, setProject] = useState<ProjectSummaryDto | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [gitMode, setGitMode] = useState<GitPresentationMode>("collaboration");
  const [bottomTab, setBottomTab] = useState<BottomPanelTab>(panel === "gitChanges" ? "gitChanges" : panel === "diagnostics" ? "diagnostics" : "tasks");

  useEffect(() => {
    void currentProject().then(setProject).catch((caughtError: unknown) => setError(normalizeCommandError(caughtError)));
  }, []);

  const tool = panel === "search" || panel === "ai" || panel === "git";

  return (
    <main className="detached-shell">
        <WindowChrome mode="detached" title={detachedPanelTitle(panel)} subtitle={project ? displayPathName(project.repositoryRoot) : null} />
        {error ? <div className="notices"><ErrorBanner title="Tool window unavailable" error={error} onDismiss={() => setError(null)} /></div> : null}
        <div className="detached-body">
          {tool ? (
            <section className="panel">
              <div className="panel-body">
                <WorkbenchToolDock activeTool={panel as WorkbenchTool} gitMode={gitMode} selectedBinding={null} onGitModeChange={setGitMode} />
              </div>
            </section>
          ) : <BottomPanel activeTab={bottomTab} onTabChange={setBottomTab} />}
        </div>
    </main>
  );
}
