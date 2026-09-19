import { useEffect, useState } from "react";
import { currentProject, normalizeCommandError } from "./ipc";
import { EditorShell } from "./components/EditorShell";
import { ProjectLauncher } from "./components/ProjectLauncher";
import type { CommandError, ProjectSummaryDto } from "./types";

type StartupState = "starting" | "launcher";

export function App() {
  const [project, setProject] = useState<ProjectSummaryDto | null>(null);
  const [startupState, setStartupState] = useState<StartupState>("starting");
  const [startupError, setStartupError] = useState<CommandError | null>(null);

  useEffect(() => {
    let active = true;

    void currentProject()
      .then((current) => {
        if (!active) {
          return;
        }
        setProject(current);
        setStartupState("launcher");
      })
      .catch((error: unknown) => {
        if (!active) {
          return;
        }
        setStartupError(normalizeCommandError(error));
        setStartupState("launcher");
      });

    return () => {
      active = false;
    };
  }, []);

  if (startupState === "starting") {
    return (
      <main className="status-shell">
        <div className="status-card">
          <span className="spinner" aria-hidden="true" />
          <p>Checking the active project…</p>
        </div>
      </main>
    );
  }

  if (!project) {
    return <ProjectLauncher initialError={startupError} onProjectReady={setProject} />;
  }

  return <EditorShell project={project} onClosed={() => setProject(null)} />;
}
