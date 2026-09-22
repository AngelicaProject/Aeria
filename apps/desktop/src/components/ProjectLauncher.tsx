import { useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { listen } from "@tauri-apps/api/event";
import {
  cancelSourcePackage,
  forgetRecentProject,
  initializeProjectFromGame,
  listRecentProjects,
  normalizeCommandError,
  openProject,
  openRecentProject,
  startSourcePackage,
} from "../ipc";
import type {
  AtlasEvent,
  CommandError,
  ProjectOpenResultDto,
  RecentProjectAvailability,
  RecentProjectDto,
  SourcePackageEventPayload,
} from "../types";
import { ErrorBanner } from "./ErrorBanner";
import { WindowChrome } from "./WindowChrome";
import { displayPath, displayPathName } from "../pathDisplay";
import {
  initialRecentProjectsState,
  reduceRecentProjectsState,
  type RecentProjectsState,
} from "../recentProjectsState";
import {
  launcherErrorTitle,
  sourcePackageListenerError,
  type LauncherError,
} from "../launcherErrorState";

type LauncherMode = "recent" | "open" | "create";

type ProjectLauncherProps = {
  initialError: CommandError | null;
  onProjectReady: (result: ProjectOpenResultDto) => void;
};

const sourceLanguages = ["en", "ja", "de", "fr"] as const;

function phaseLabel(event: AtlasEvent | null): string {
  if (!event) return "Preparing source package";
  if (event.type === "started") return "Starting Harmonia Atlas";
  if (event.type === "completed") return "Source package complete";
  if (event.type === "failed") return "Source package failed";
  const labels: Record<string, string> = {
    inspectInstallation: "Inspecting game installation",
    extractSource: "Building source snapshot",
    verifySource: "Verifying source snapshot",
    scanEvidence: "Comparing official languages",
    writePackage: "Building source package",
    buildPackage: "Building source package",
    validatePackage: "Validating source package",
  };
  return labels[event.phase] ?? event.phase;
}

function progressDetails(event: AtlasEvent | null): { sheet: string | null; language: string | null; rows: number | null } {
  if (!event || event.type !== "progress") return { sheet: null, language: null, rows: null };
  return { sheet: event.sheet, language: event.language, rows: event.rowsProcessed };
}

function repositoryName(path: string): string {
  return displayPathName(path);
}

function availabilityLabel(availability: RecentProjectAvailability): string {
  switch (availability) {
    case "ready": return "Ready";
    case "repositoryMissing": return "Repository missing";
    case "sourcePackageMissing": return "Source package missing";
    case "repositoryAndSourceMissing": return "Repository and source package missing";
  }
}

function lastOpenedLabel(timestamp: number): string {
  return new Date(timestamp).toLocaleString();
}

export function ProjectLauncher({ initialError, onProjectReady }: ProjectLauncherProps) {
  const [mode, setMode] = useState<LauncherMode>("recent");
  const [repositoryRoot, setRepositoryRoot] = useState("");
  const [sourcePackagePath, setSourcePackagePath] = useState("");
  const [gamePath, setGamePath] = useState("");
  const [sourceLanguage, setSourceLanguage] = useState<(typeof sourceLanguages)[number]>("en");
  const [targetLanguage, setTargetLanguage] = useState("");
  const [busy, setBusy] = useState<Exclude<LauncherMode, "recent"> | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const [cancelRequested, setCancelRequested] = useState(false);
  const [sourcePackageEventsReady, setSourcePackageEventsReady] = useState(false);
  const [progress, setProgress] = useState<AtlasEvent | null>(null);
  const [error, setError] = useState<LauncherError | null>(
    initialError ? { operation: "open", error: initialError } : null,
  );
  const [recentState, setRecentState] = useState<RecentProjectsState>(initialRecentProjectsState);
  const [recentActionError, setRecentActionError] = useState<CommandError | null>(null);
  const [recentBusyId, setRecentBusyId] = useState<string | null>(null);
  const busyRef = useRef<Exclude<LauncherMode, "recent"> | null>(null);
  const jobIdRef = useRef<string | null>(null);

  useEffect(() => {
    setError(initialError ? { operation: "open", error: initialError } : null);
  }, [initialError]);

  useEffect(() => {
    let disposed = false;
    void listRecentProjects()
      .then((projects) => {
        if (!disposed) {
          setRecentState({ status: "loaded", projects });
        }
      })
      .catch((caughtError: unknown) => {
        if (!disposed) {
          setRecentState({ status: "failed", error: normalizeCommandError(caughtError) });
        }
      });
    return () => {
      disposed = true;
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<SourcePackageEventPayload>("source-package-event", ({ payload }) => {
      if (busyRef.current !== "create" || (jobIdRef.current !== null && payload.jobId !== jobIdRef.current)) return;
      jobIdRef.current = payload.jobId;
      setJobId(payload.jobId);
      setProgress(payload.event);
    }).then((cleanup) => {
      if (disposed) cleanup();
      else {
        unlisten = cleanup;
        setSourcePackageEventsReady(true);
      }
    }).catch((caughtError: unknown) => {
      if (disposed) return;
      if (import.meta.env.DEV) {
        console.error("failed to register source-package-event listener", caughtError);
      }
      setSourcePackageEventsReady(false);
      setError({ operation: "create", error: sourcePackageListenerError() });
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (mode === "recent") return;
    if (mode === "create" && !sourcePackageEventsReady) {
      setError({ operation: "create", error: sourcePackageListenerError() });
      return;
    }
    busyRef.current = mode;
    jobIdRef.current = null;
    flushSync(() => {
      setBusy(mode);
      setError(null);
      setProgress(null);
      setJobId(null);
      setCancelRequested(false);
    });
    try {
      const result = mode === "open"
        ? await openProject(repositoryRoot, sourcePackagePath)
        : await (async () => {
          const started = await startSourcePackage();
          jobIdRef.current = started.jobId;
          setJobId(started.jobId);
          return initializeProjectFromGame(
            started.jobId,
            repositoryRoot,
            gamePath,
            sourceLanguage,
            targetLanguage,
          );
        })();
      onProjectReady(result);
    } catch (caughtError) {
      setError({
        operation: mode,
        error: normalizeCommandError(caughtError),
      });
    } finally {
      busyRef.current = null;
      jobIdRef.current = null;
      setBusy(null);
      setCancelRequested(false);
    }
  }

  async function handleRecentOpen(project: RecentProjectDto) {
    if (project.availability !== "ready" || busy !== null || recentBusyId !== null) return;
    flushSync(() => {
      setRecentBusyId(project.id);
      setError(null);
    });
    try {
      const result = await openRecentProject(project.id);
      onProjectReady(result);
    } catch (caughtError) {
      setError({
        operation: "recentOpen",
        error: normalizeCommandError(caughtError),
      });
    } finally {
      setRecentBusyId(null);
    }
  }

  async function handleRecentRemove(project: RecentProjectDto) {
    if (busy !== null || recentBusyId !== null) return;
    flushSync(() => {
      setRecentBusyId(project.id);
      setRecentActionError(null);
    });
    try {
      await forgetRecentProject(project.id);
      setRecentState((current) => reduceRecentProjectsState(current, {
        type: "removed",
        projectId: project.id,
      }));
    } catch (caughtError) {
      setRecentActionError(normalizeCommandError(caughtError));
    } finally {
      setRecentBusyId(null);
    }
  }

  async function handleCancel() {
    if (!jobId || cancelRequested) return;
    flushSync(() => setCancelRequested(true));
    try {
      await cancelSourcePackage(jobId);
    } catch (caughtError) {
      setCancelRequested(false);
      setError({
        operation: "create",
        error: normalizeCommandError(caughtError),
      });
    }
  }

  const details = progressDetails(progress);
  const launcherDisabled = busy !== null || recentBusyId !== null;
  const modeLabel = mode === "recent" ? "Recent projects" : mode === "open" ? "Open project" : "Create project";

  return (
    <main className="launcher-shell">
      <WindowChrome context="Project launcher" detail={modeLabel} mode="launcher" launcherMode={mode} />
      <section className="launcher-panel" aria-labelledby="launcher-title">
        <div className="launcher-heading">
          <h1 id="launcher-title">{modeLabel}</h1>
        </div>

        {error ? (
          <ErrorBanner
            title={launcherErrorTitle(error.operation)}
            error={error.error}
            onDismiss={() => setError(null)}
          />
        ) : null}

        <nav className="launcher-tabs" role="tablist" aria-label="Project action">
          {(["recent", "open", "create"] as const).map((tab) => (
            <button
              className={mode === tab ? "tab-button active" : "tab-button"}
              type="button"
              role="tab"
              key={tab}
              aria-selected={mode === tab}
              disabled={launcherDisabled}
              onClick={() => setMode(tab)}
            >
              {tab === "recent" ? "Recent" : tab === "open" ? "Open" : "Create"}
            </button>
          ))}
        </nav>

        {mode === "recent" ? (
          <section className="recent-projects" aria-labelledby="recent-projects-title">
            <div className="section-heading">
              <div>
                <h2 id="recent-projects-title">Workspaces</h2>
                <p>Workspaces opened on this machine.</p>
              </div>
              {recentState.status === "loaded" ? <span className="pane-count">{recentState.projects.length}</span> : null}
            </div>
            {recentState.status === "failed" && recentState.error ? (
              <ErrorBanner
                title="Recent projects unavailable"
                error={recentState.error}
                onDismiss={() => setRecentState((current) => reduceRecentProjectsState(current, { type: "dismissError" }))}
              />
            ) : null}
            {recentActionError ? (
              <ErrorBanner
                title="Could not remove recent project"
                error={recentActionError}
                onDismiss={() => setRecentActionError(null)}
              />
            ) : null}
            {recentState.status === "loading" ? (
              <div className="launcher-list-state" aria-live="polite">
                <span className="spinner" aria-hidden="true" />
                Loading recent projects…
              </div>
            ) : null}
            {recentState.status === "failed" ? <p className="recent-empty">Recent projects unavailable.</p> : null}
            {recentState.status === "loaded" && recentState.projects.length === 0 ? (
              <p className="recent-empty">Projects you successfully open or create will appear here.</p>
            ) : null}
            {recentState.status === "loaded" && recentState.projects.length > 0 ? (
              <div className="recent-project-list">
                {recentState.projects.map((project) => (
                  <article className="recent-project-row" key={project.id}>
                    <div className="recent-project-identity">
                      <strong>{repositoryName(project.repositoryRoot)}</strong>
                      <code title={displayPath(project.repositoryRoot)}>{displayPath(project.repositoryRoot)}</code>
                    </div>
                    <span className="recent-project-language">{project.sourceLanguage} → {project.targetLanguage}</span>
                    <span className="recent-project-game">{project.gameVersion || "Unknown game version"}</span>
                    <span className="recent-project-opened">{lastOpenedLabel(project.lastOpenedAtUnixMs)}</span>
                    <span className={project.availability === "ready" ? "availability ready" : "availability"}>
                      {availabilityLabel(project.availability)}
                    </span>
                    <div className="recent-project-actions">
                      {project.availability === "ready" ? (
                        <button
                          className="primary-button compact-button"
                          type="button"
                          onClick={() => void handleRecentOpen(project)}
                          disabled={launcherDisabled}
                        >
                          {recentBusyId === project.id ? "Opening…" : "Open"}
                        </button>
                      ) : null}
                      <button
                        className="text-button"
                        type="button"
                        onClick={() => void handleRecentRemove(project)}
                        disabled={launcherDisabled}
                      >
                        {recentBusyId === project.id ? "Working…" : "Remove"}
                      </button>
                    </div>
                  </article>
                ))}
              </div>
            ) : null}
          </section>
        ) : (
          <form className="launcher-form" onSubmit={handleSubmit}>
            <label htmlFor="repository-root">Repository root</label>
            <input id="repository-root" value={repositoryRoot}
              onChange={(event) => setRepositoryRoot(event.target.value)}
              placeholder="C:\\Projects\\my-translation" autoComplete="off" disabled={launcherDisabled} required />

            {mode === "open" ? (
              <>
                <label htmlFor="source-package-path">HSP source package path</label>
                <input id="source-package-path" value={sourcePackagePath}
                  onChange={(event) => setSourcePackagePath(event.target.value)}
                  placeholder="C:\\Sources\\source-en.hsp" autoComplete="off" disabled={launcherDisabled} required />
              </>
            ) : (
              <>
                <label htmlFor="game-path">Game installation path</label>
                <input id="game-path" value={gamePath}
                  onChange={(event) => setGamePath(event.target.value)}
                  placeholder="C:\\Games\\FINAL FANTASY XIV" autoComplete="off" disabled={launcherDisabled} required />
                <div className="form-columns">
                  <div>
                    <label htmlFor="source-language">Source language</label>
                    <select id="source-language" value={sourceLanguage}
                      onChange={(event) => setSourceLanguage(event.target.value as (typeof sourceLanguages)[number])}
                      disabled={launcherDisabled}>
                      {sourceLanguages.map((language) => <option key={language} value={language}>{language}</option>)}
                    </select>
                  </div>
                  <div>
                    <label htmlFor="target-language">Target language</label>
                    <input id="target-language" value={targetLanguage}
                      onChange={(event) => setTargetLanguage(event.target.value)}
                      placeholder="fr, ru, es" autoComplete="off" disabled={launcherDisabled} required />
                  </div>
                </div>
              </>
            )}

            <button
              className="primary-button launcher-submit"
              type="submit"
              disabled={launcherDisabled || (mode === "create" && !sourcePackageEventsReady)}
            >
              {busy === "open" ? "Opening…" : busy === "create" ? "Creating…" : mode === "open" ? "Open project" : "Create project"}
            </button>
          </form>
        )}

        {busy === "create" ? (
          <section className="atlas-progress" aria-live="polite" aria-label="Source package progress">
            <div className="atlas-progress-heading">
              <span className="spinner" aria-hidden="true" />
              <strong>{phaseLabel(progress)}</strong>
            </div>
            <div className="progress-facts">
              <span>Sheet <strong>{details.sheet ?? "—"}</strong></span>
              <span>Language <strong>{details.language ?? "—"}</strong></span>
              <span>Rows <strong>{details.rows === null ? "—" : details.rows.toLocaleString()}</strong></span>
            </div>
            <button className="secondary-button cancel-button" type="button"
              onClick={() => void handleCancel()} disabled={jobId === null || cancelRequested}>
              {cancelRequested ? "Cancelling…" : "Cancel"}
            </button>
          </section>
        ) : null}

      </section>
    </main>
  );
}
