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
import { TextContextMenu } from "./ui/TextContextMenu";
import { DetachedToolWindow, isDetachedPanel } from "./components/DetachedToolWindow";
import { SourceUpdateDialog } from "./components/SourceUpdateDialog";
import { UpdateNotice } from "./components/UpdateNotice";
import { loadGameGlyphs } from "./ui/gameGlyphs";

type StartupState = "starting" | "launcher";

function MainWindow() {
  const { t } = useI18n();
  const [project, setProject] = useState<ProjectSummaryDto | null>(null);
  const [startupState, setStartupState] = useState<StartupState>("starting");
  const [startupError, setStartupError] = useState<CommandError | null>(null);
  const [projectWarning, setProjectWarning] = useState<CommandError | null>(null);
  /** An update to a new game version that just ran, to summarize. */
  const [sourceUpdateView, setSourceUpdateView] = useState<SourceUpdateReportDto | null>(null);

  function handleProjectReady(result: ProjectOpenResultDto) {
    setProject(result.project);
    setProjectWarning(result.warning);
    setSourceUpdateView(result.sourceUpdate);
  }

  // The game's symbols and icons follow the open project's game.
  const gamePath = project?.gamePath ?? null;
  useEffect(() => {
    void loadGameGlyphs(gamePath);
  }, [gamePath]);

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
    return (
      <>
        <ProjectLauncher initialError={startupError} onProjectReady={handleProjectReady} />
        <UpdateNotice />
      </>
    );
  }

  return (
    <>
      <EditorShell
        project={project}
        applicationWarning={projectWarning}
        onDismissApplicationWarning={() => setProjectWarning(null)}
        onProjectChanged={setProject}
        onClosed={() => {
          setProject(null);
          setProjectWarning(null);
          setSourceUpdateView(null);
        }}
      />
      <SourceUpdateDialog
        open={sourceUpdateView !== null}
        mode="summary"
        report={sourceUpdateView}
        onClose={() => setSourceUpdateView(null)}
      />
      <UpdateNotice />
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
            <TextContextMenu>
              {isDetachedPanel(detachedPanel) ? <DetachedToolWindow panel={detachedPanel} /> : <MainWindow />}
            </TextContextMenu>
            <TitleTooltips />
          </Tooltip.Provider>
        </I18nProvider>
      </PreferencesProvider>
    </ThemeProvider>
  );
}
