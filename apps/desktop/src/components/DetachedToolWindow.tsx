import { useEffect, useState } from "react";
import { currentProject, normalizeCommandError } from "../ipc";
import type { CommandError, ProjectSummaryDto } from "../types";
import type { MessageKey } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { ErrorBanner } from "./ErrorBanner";
import { WindowChrome } from "./WindowChrome";
import { WorkbenchToolDock, toolTitle } from "./WorkbenchToolDock";
import { displayPathName } from "../pathDisplay";
import { mainWindowRequests } from "../workbenchEvents";

export type DetachedPanel = "search" | "git";

export function isDetachedPanel(value: string | null): value is DetachedPanel {
  return value === "search" || value === "git";
}

export function detachedPanelTitle(panel: DetachedPanel): MessageKey {
  return toolTitle(panel);
}

export function DetachedToolWindow({ panel }: { panel: DetachedPanel }) {
  const { t } = useI18n();
  const [project, setProject] = useState<ProjectSummaryDto | null>(null);
  const [error, setError] = useState<CommandError | null>(null);

  useEffect(() => {
    void currentProject().then(setProject).catch((caughtError: unknown) => setError(normalizeCommandError(caughtError)));
  }, []);

  return (
    <main className="detached-shell">
        <WindowChrome mode="detached" title={t(detachedPanelTitle(panel))} subtitle={project ? displayPathName(project.repositoryRoot) : null} />
        {error ? <div className="notices"><ErrorBanner title={t("detached.unavailable")} error={error} onDismiss={() => setError(null)} /></div> : null}
        <div className="detached-body">
          <section className="panel">
            <div className="panel-body">
              <WorkbenchToolDock activeTool={panel} selectedBinding={null} onRevealBinding={mainWindowRequests.revealString} onWorkspaceChanged={mainWindowRequests.workspaceChanged} onOpenTranslate={mainWindowRequests.openTranslate} onOpenCommit={mainWindowRequests.openCommit} />
            </div>
          </section>
        </div>
    </main>
  );
}
