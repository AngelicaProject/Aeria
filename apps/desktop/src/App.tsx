import { useEffect, useState } from "react";
import { Tooltip } from "radix-ui";
import { currentProject, normalizeCommandError } from "./ipc";
import { EditorShell } from "./components/EditorShell";
import { ProjectLauncher } from "./components/ProjectLauncher";
import type { CommandError, ProjectOpenResultDto, ProjectSummaryDto } from "./types";
import { ThemeProvider } from "./ui/theme/theme";
import { PreferencesProvider } from "./ui/preferences";
import { DetachedToolWindow, isDetachedPanel } from "./components/DetachedToolWindow";

type StartupState = "starting" | "launcher";

function MainWindow() {
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
        if (!active) return;
        setProject(current);
        setStartupState("launcher");
      })
      .catch((error: unknown) => {
        if (!active) return;
        setStartupError(normalizeCommandError(error));
        setStartupState("launcher");
      });

    return () => {
      active = false;
    };
  }, []);

  if (startupState === "starting") {
    return (
      <main className="startup" aria-live="polite">
        <span className="spinner" aria-hidden="true" />
        <span className="visually-hidden">Checking the active project…</span>
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

const detachedPanel = new URLSearchParams(window.location.search).get("detached");

export function App() {
  return (
    <ThemeProvider>
      <PreferencesProvider>
        <Tooltip.Provider delayDuration={500} skipDelayDuration={200}>
          {isDetachedPanel(detachedPanel) ? <DetachedToolWindow panel={detachedPanel} /> : <MainWindow />}
        </Tooltip.Provider>
      </PreferencesProvider>
    </ThemeProvider>
  );
}
