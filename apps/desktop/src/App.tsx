import { useEffect, useState } from "react";
import { Tooltip } from "radix-ui";
import { currentProject, normalizeCommandError } from "./ipc";
import { EditorShell } from "./components/EditorShell";
import { ProjectLauncher } from "./components/ProjectLauncher";
import type { CommandError, ProjectOpenResultDto, ProjectSummaryDto, SourceUpdateReportDto } from "./types";
import { ThemeProvider } from "./ui/theme/theme";
import { PreferencesProvider } from "./ui/preferences";
import { I18nProvider, useI18n } from "./ui/i18n";
import { TitleTooltips } from "./ui/primitives/TitleTooltips";
import { DetachedToolWindow, isDetachedPanel } from "./components/DetachedToolWindow";
import { SourceUpdateDialog } from "./components/SourceUpdateDialog";

type StartupState = "starting" | "launcher";

function MainWindow() {
  const { t } = useI18n();
  const [project, setProject] = useState<ProjectSummaryDto | null>(null);
  const [startupState, setStartupState] = useState<StartupState>("starting");
  const [startupError, setStartupError] = useState<CommandError | null>(null);
  const [projectWarning, setProjectWarning] = useState<CommandError | null>(null);
  /** An applied source update to summarize, or `{ report: null }` for the detached list only. */
  const [sourceUpdateView, setSourceUpdateView] = useState<{ report: SourceUpdateReportDto | null } | null>(null);

  function handleProjectReady(result: ProjectOpenResultDto) {
    setProject(result.project);
    setProjectWarning(result.warning);
    setSourceUpdateView(result.sourceUpdate ? { report: result.sourceUpdate } : null);
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
        <span className="visually-hidden">{t("app.checkingProject")}</span>
      </main>
    );
  }

  if (!project) {
    return <ProjectLauncher initialError={startupError} onProjectReady={handleProjectReady} />;
  }

  return (
    <>
      <EditorShell
        project={project}
        applicationWarning={projectWarning}
        onDismissApplicationWarning={() => setProjectWarning(null)}
        onShowDetachedUnits={() => setSourceUpdateView({ report: null })}
        onClosed={() => {
          setProject(null);
          setProjectWarning(null);
          setSourceUpdateView(null);
        }}
      />
      <SourceUpdateDialog
        open={sourceUpdateView !== null}
        mode="summary"
        report={sourceUpdateView?.report ?? null}
        onClose={() => setSourceUpdateView(null)}
      />
    </>
  );
}

const detachedPanel = new URLSearchParams(window.location.search).get("detached");

export function App() {
  return (
    <ThemeProvider>
      <PreferencesProvider>
        <I18nProvider>
          <Tooltip.Provider delayDuration={500} skipDelayDuration={200}>
            {isDetachedPanel(detachedPanel) ? <DetachedToolWindow panel={detachedPanel} /> : <MainWindow />}
            <TitleTooltips />
          </Tooltip.Provider>
        </I18nProvider>
      </PreferencesProvider>
    </ThemeProvider>
  );
}
