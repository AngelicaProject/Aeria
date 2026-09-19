import { useEffect, useState } from "react";
import { initializeProject, normalizeCommandError, openProject } from "../ipc";
import type { CommandError, ProjectSummaryDto } from "../types";
import { ErrorBanner } from "./ErrorBanner";

type LauncherMode = "open" | "create";

type ProjectLauncherProps = {
  initialError: CommandError | null;
  onProjectReady: (project: ProjectSummaryDto) => void;
};

export function ProjectLauncher({ initialError, onProjectReady }: ProjectLauncherProps) {
  const [mode, setMode] = useState<LauncherMode>("open");
  const [repositoryRoot, setRepositoryRoot] = useState("");
  const [sourcePath, setSourcePath] = useState("");
  const [targetLanguage, setTargetLanguage] = useState("");
  const [busy, setBusy] = useState<LauncherMode | null>(null);
  const [error, setError] = useState<CommandError | null>(initialError);

  useEffect(() => {
    setError(initialError);
  }, [initialError]);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setBusy(mode);
    setError(null);

    try {
      const project =
        mode === "open"
          ? await openProject(repositoryRoot, sourcePath)
          : await initializeProject(repositoryRoot, sourcePath, targetLanguage);
      onProjectReady(project);
    } catch (caughtError) {
      setError(normalizeCommandError(caughtError));
    } finally {
      setBusy(null);
    }
  }

  return (
    <main className="launcher-shell">
      <section className="launcher-panel" aria-labelledby="launcher-title">
        <div className="launcher-heading">
          <p className="eyebrow">Desktop translation editor</p>
          <h1 id="launcher-title">Aeria</h1>
          <p>Open an existing translation workspace or initialize one from a verified HXS source.</p>
        </div>

        {error ? <ErrorBanner title="Could not open project" error={error} onDismiss={() => setError(null)} /> : null}

        <div className="mode-tabs" role="tablist" aria-label="Project action">
          <button
            className={mode === "open" ? "tab-button active" : "tab-button"}
            type="button"
            role="tab"
            aria-selected={mode === "open"}
            disabled={busy !== null}
            onClick={() => setMode("open")}
          >
            Open project
          </button>
          <button
            className={mode === "create" ? "tab-button active" : "tab-button"}
            type="button"
            role="tab"
            aria-selected={mode === "create"}
            disabled={busy !== null}
            onClick={() => setMode("create")}
          >
            Create project
          </button>
        </div>

        <form className="launcher-form" onSubmit={handleSubmit}>
          <label htmlFor="repository-root">Repository root</label>
          <input
            id="repository-root"
            value={repositoryRoot}
            onChange={(event) => setRepositoryRoot(event.target.value)}
            placeholder="C:\\Projects\\my-translation"
            autoComplete="off"
            disabled={busy !== null}
          />

          <label htmlFor="source-path">HXS source path</label>
          <input
            id="source-path"
            value={sourcePath}
            onChange={(event) => setSourcePath(event.target.value)}
            placeholder="C:\\Sources\\game.hxs"
            autoComplete="off"
            disabled={busy !== null}
          />

          {mode === "create" ? (
            <>
              <label htmlFor="target-language">Target language</label>
              <input
                id="target-language"
                value={targetLanguage}
                onChange={(event) => setTargetLanguage(event.target.value)}
                placeholder="fr, ru, es"
                autoComplete="off"
                disabled={busy !== null}
              />
            </>
          ) : null}

          <button className="primary-button launcher-submit" type="submit" disabled={busy !== null}>
            {busy === "open" ? "Opening…" : busy === "create" ? "Creating…" : mode === "open" ? "Open project" : "Create project"}
          </button>
        </form>

        <p className="launcher-footnote">Paths are entered directly for now. A file picker will come later.</p>
      </section>
    </main>
  );
}
