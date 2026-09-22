import { useEffect, useState } from "react";
import { currentProject, normalizeCommandError } from "./ipc";
import { EditorShell } from "./components/EditorShell";
import { ProjectLauncher } from "./components/ProjectLauncher";
import { WindowChrome } from "./components/WindowChrome";
import type { CommandError, ProjectOpenResultDto, ProjectSummaryDto } from "./types";
import { ThemeProvider } from "./ui/theme/theme";

type StartupState = "starting" | "launcher";

function AppContent() {
  const [project, setProject] = useState<ProjectSummaryDto | null>(null);
  const [startupState, setStartupState] = useState<StartupState>("starting");
  const [startupError, setStartupError] = useState<CommandError | null>(null);
  const [projectWarning, setProjectWarning] = useState<CommandError | null>(null);

  function handleProjectReady(result: ProjectOpenResultDto) {
    setProject(result.project);
    setProjectWarning(result.warning);
  }

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
        <WindowChrome context="Starting" mode="launcher" />
        <div className="status-card" aria-live="polite">
          <span className="spinner" aria-hidden="true" />
          <p>Checking the active project…</p>
        </div>
      </main>
    );
  }

  if (!project) {
    return <ProjectLauncher initialError={startupError} onProjectReady={handleProjectReady} />;
  }

  return (
    <EditorShell
      project={project}
      applicationWarning={projectWarning}
      onDismissApplicationWarning={() => setProjectWarning(null)}
      onClosed={() => {
        setProject(null);
        setProjectWarning(null);
      }}
    />
  );
}

export function App() {
  return (
    <ThemeProvider>
      <AppContent />
    </ThemeProvider>
  );
}
