import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  cancelSourcePackage,
  initializeProjectFromGame,
  normalizeCommandError,
  openProject,
} from "../ipc";
import type { AtlasEvent, CommandError, ProjectSummaryDto, SourcePackageEventPayload } from "../types";
import { ErrorBanner } from "./ErrorBanner";

type LauncherMode = "open" | "create";

type ProjectLauncherProps = {
  initialError: CommandError | null;
  onProjectReady: (project: ProjectSummaryDto) => void;
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

export function ProjectLauncher({ initialError, onProjectReady }: ProjectLauncherProps) {
  const [mode, setMode] = useState<LauncherMode>("open");
  const [repositoryRoot, setRepositoryRoot] = useState("");
  const [sourcePackagePath, setSourcePackagePath] = useState("");
  const [gamePath, setGamePath] = useState("");
  const [sourceLanguage, setSourceLanguage] = useState<(typeof sourceLanguages)[number]>("en");
  const [targetLanguage, setTargetLanguage] = useState("");
  const [busy, setBusy] = useState<LauncherMode | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const [cancelRequested, setCancelRequested] = useState(false);
  const [progress, setProgress] = useState<AtlasEvent | null>(null);
  const [error, setError] = useState<CommandError | null>(initialError);
  const busyRef = useRef<LauncherMode | null>(null);
  const jobIdRef = useRef<string | null>(null);

  useEffect(() => setError(initialError), [initialError]);

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
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    busyRef.current = mode;
    jobIdRef.current = null;
    setBusy(mode);
    setError(null);
    setProgress(null);
    setJobId(null);
    setCancelRequested(false);
    try {
      const project = mode === "open"
        ? await openProject(repositoryRoot, sourcePackagePath)
        : await initializeProjectFromGame(repositoryRoot, gamePath, sourceLanguage, targetLanguage);
      onProjectReady(project);
    } catch (caughtError) {
      setError(normalizeCommandError(caughtError));
    } finally {
      busyRef.current = null;
      jobIdRef.current = null;
      setBusy(null);
      setCancelRequested(false);
    }
  }

  async function handleCancel() {
    if (!jobId || cancelRequested) return;
    setCancelRequested(true);
    try {
      await cancelSourcePackage(jobId);
    } catch (caughtError) {
      setCancelRequested(false);
      setError(normalizeCommandError(caughtError));
    }
  }

  const details = progressDetails(progress);

  return (
    <main className="launcher-shell">
      <section className="launcher-panel" aria-labelledby="launcher-title">
        <div className="launcher-heading">
          <p className="eyebrow">Desktop translation editor</p>
          <h1 id="launcher-title">Aeria</h1>
          <p>Open an existing translation workspace or create one directly from a verified game installation.</p>
        </div>
        {error ? (
          <ErrorBanner
            title={mode === "create" ? "Could not create project" : "Could not open project"}
            error={error}
            onDismiss={() => setError(null)}
          />
        ) : null}

        <div className="mode-tabs" role="tablist" aria-label="Project action">
          <button className={mode === "open" ? "tab-button active" : "tab-button"} type="button" role="tab"
            aria-selected={mode === "open"} disabled={busy !== null} onClick={() => setMode("open")}>
            Open project
          </button>
          <button className={mode === "create" ? "tab-button active" : "tab-button"} type="button" role="tab"
            aria-selected={mode === "create"} disabled={busy !== null} onClick={() => setMode("create")}>
            Create project
          </button>
        </div>

        <form className="launcher-form" onSubmit={handleSubmit}>
          <label htmlFor="repository-root">Repository root</label>
          <input id="repository-root" value={repositoryRoot}
            onChange={(event) => setRepositoryRoot(event.target.value)}
            placeholder="C:\\Projects\\my-translation" autoComplete="off" disabled={busy !== null} required />

          {mode === "open" ? (
            <>
              <label htmlFor="source-package-path">HSP source package path</label>
              <input id="source-package-path" value={sourcePackagePath}
                onChange={(event) => setSourcePackagePath(event.target.value)}
                placeholder="C:\\Sources\\source-en.hsp" autoComplete="off" disabled={busy !== null} required />
            </>
          ) : (
            <>
              <label htmlFor="game-path">Game installation path</label>
              <input id="game-path" value={gamePath}
                onChange={(event) => setGamePath(event.target.value)}
                placeholder="C:\\Games\\FINAL FANTASY XIV" autoComplete="off" disabled={busy !== null} required />
              <label htmlFor="source-language">Source language</label>
              <select id="source-language" value={sourceLanguage}
                onChange={(event) => setSourceLanguage(event.target.value as (typeof sourceLanguages)[number])}
                disabled={busy !== null}>
                {sourceLanguages.map((language) => <option key={language} value={language}>{language}</option>)}
              </select>
              <label htmlFor="target-language">Target language</label>
              <input id="target-language" value={targetLanguage}
                onChange={(event) => setTargetLanguage(event.target.value)}
                placeholder="fr, ru, es" autoComplete="off" disabled={busy !== null} required />
            </>
          )}

          <button className="primary-button launcher-submit" type="submit" disabled={busy !== null}>
            {busy === "open" ? "Opening…" : busy === "create" ? "Creating…" : mode === "open" ? "Open project" : "Create project"}
          </button>
        </form>

        {busy === "create" ? (
          <section className="atlas-progress" aria-live="polite" aria-label="Source package progress">
            <div className="atlas-progress-heading"><span className="spinner" aria-hidden="true" /><strong>{phaseLabel(progress)}</strong></div>
            {details.sheet ? <p>Sheet: <span>{details.sheet}</span></p> : null}
            {details.language ? <p>Evidence language: <span>{details.language}</span></p> : null}
            {details.rows !== null ? <p>Rows processed: <span>{details.rows}</span></p> : null}
            <button className="secondary-button cancel-button" type="button"
              onClick={() => void handleCancel()} disabled={jobId === null || cancelRequested}>
              {cancelRequested ? "Cancelling…" : "Cancel"}
            </button>
          </section>
        ) : null}

        <p className="launcher-footnote">
          {mode === "create"
            ? "Atlas builds and validates the source package in app data. It is not stored in the translation repository."
            : "Paths are entered directly for now. A file picker will come later."}
        </p>
      </section>
    </main>
  );
}
