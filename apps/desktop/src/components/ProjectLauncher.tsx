import { useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { listen } from "@tauri-apps/api/event";
import { open as openNativeDialog } from "@tauri-apps/plugin-dialog";
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
import { Icon } from "../ui/primitives/Icon";
import { SelectMenu } from "../ui/primitives/SelectMenu";
import { useTheme } from "../ui/theme/theme";
import { themeRegistry } from "../ui/theme/registry";

type LauncherMode = "recent" | "open" | "create" | "settings";

type ProjectLauncherProps = {
  initialError: CommandError | null;
  onProjectReady: (result: ProjectOpenResultDto) => void;
};

const sourceLanguages = [
  { value: "en", label: "English" },
  { value: "ja", label: "Japanese" },
  { value: "de", label: "German" },
  { value: "fr", label: "French" },
] as const;

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

function availabilityLabel(availability: RecentProjectAvailability): string {
  switch (availability) {
    case "ready": return "Ready";
    case "repositoryMissing": return "Repository missing";
    case "sourcePackageMissing": return "Source package missing";
    case "repositoryAndSourceMissing": return "Repository and source package missing";
  }
}

function languageLabel(value: string): string {
  return sourceLanguages.find((language) => language.value === value)?.label ?? value;
}

type BrowseFieldProps = {
  id: string;
  label: string;
  value: string;
  placeholder: string;
  hint?: string;
  directory?: boolean;
  disabled: boolean;
  onChange: (value: string) => void;
  onError: (message: string) => void;
};

function BrowseField({ id, label, value, placeholder, hint, directory = false, disabled, onChange, onError }: BrowseFieldProps) {
  async function handleBrowse() {
    try {
      const options = directory
        ? { directory: true, multiple: false }
        : { directory: false, multiple: false, filters: [{ name: "Harmonia source package", extensions: ["hsp"] }] };
      const selection = await openNativeDialog(options);
      if (typeof selection === "string") onChange(selection);
    } catch (error) {
      onError(error instanceof Error ? error.message : "The native path picker failed.");
    }
  }

  return (
    <div className="launcher-field">
      <label htmlFor={id}>{label}</label>
      <div className="path-picker">
        <input
          className="path-input"
          id={id}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          placeholder={placeholder}
          autoComplete="off"
          disabled={disabled}
          required
        />
        <button className="icon-button picker-button" type="button" aria-label={`Choose ${label.toLowerCase()}`} disabled={disabled} onClick={() => void handleBrowse()}>
          <Icon name="folder" size={14} />
        </button>
      </div>
      {hint ? <small className="field-hint">{hint}</small> : null}
    </div>
  );
}

type RecentProjectActionsProps = {
  project: RecentProjectDto;
  disabled: boolean;
  onOpen: () => void;
  onRemove: () => void;
};

function RecentProjectActions({ project, disabled, onOpen, onRemove }: RecentProjectActionsProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handlePointerDown(event: PointerEvent) {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    }
    window.addEventListener("pointerdown", handlePointerDown);
    return () => window.removeEventListener("pointerdown", handlePointerDown);
  }, []);

  return (
    <div className="project-actions-menu" ref={rootRef}>
      <button className="button icon-button" type="button" aria-label={`Actions for ${displayPathName(project.repositoryRoot)}`} aria-haspopup="menu" aria-expanded={open} disabled={disabled} onClick={() => setOpen((current) => !current)}><Icon name="more" size={15} /></button>
      {open ? <div className="project-actions-popup" role="menu"><button type="button" role="menuitem" disabled={project.availability !== "ready"} onClick={() => { setOpen(false); onOpen(); }}>Open</button><button type="button" role="menuitem" onClick={() => { setOpen(false); onRemove(); }}>Remove from Recent Projects…</button></div> : null}
    </div>
  );
}

export function ProjectLauncher({ initialError, onProjectReady }: ProjectLauncherProps) {
  const [mode, setMode] = useState<LauncherMode>("recent");
  const [repositoryRoot, setRepositoryRoot] = useState("");
  const [sourcePackagePath, setSourcePackagePath] = useState("");
  const [gamePath, setGamePath] = useState("");
  const [sourceLanguage, setSourceLanguage] = useState("en");
  const [busy, setBusy] = useState<Exclude<LauncherMode, "recent" | "settings"> | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const [cancelRequested, setCancelRequested] = useState(false);
  const [progress, setProgress] = useState<AtlasEvent | null>(null);
  const [error, setError] = useState<LauncherError | null>(initialError ? { operation: "open", error: initialError } : null);
  const [recentState, setRecentState] = useState<RecentProjectsState>(initialRecentProjectsState);
  const [recentActionError, setRecentActionError] = useState<CommandError | null>(null);
  const [recentBusyId, setRecentBusyId] = useState<string | null>(null);
  const busyRef = useRef<Exclude<LauncherMode, "recent" | "settings"> | null>(null);
  const jobIdRef = useRef<string | null>(null);
  const { theme, setThemeId, accentOverride, setAccentOverride } = useTheme();

  useEffect(() => setError(initialError ? { operation: "open", error: initialError } : null), [initialError]);

  useEffect(() => {
    let disposed = false;
    void listRecentProjects()
      .then((projects) => { if (!disposed) setRecentState({ status: "loaded", projects }); })
      .catch((caughtError: unknown) => { if (!disposed) setRecentState({ status: "failed", error: normalizeCommandError(caughtError) }); });
    return () => { disposed = true; };
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
      else { unlisten = cleanup; }
    }).catch((caughtError: unknown) => {
      if (disposed) return;
      if (import.meta.env.DEV) console.error("failed to register source-package-event listener", caughtError);
      setError({ operation: "create", error: sourcePackageListenerError() });
    });
    return () => { disposed = true; unlisten?.(); };
  }, []);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (mode !== "open" && mode !== "create") return;
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
            return initializeProjectFromGame(started.jobId, repositoryRoot, gamePath, sourceLanguage, "und");
          })();
      onProjectReady(result);
    } catch (caughtError) {
      setError({ operation: mode, error: normalizeCommandError(caughtError) });
    } finally {
      busyRef.current = null;
      jobIdRef.current = null;
      setBusy(null);
      setCancelRequested(false);
    }
  }

  async function handleRecentOpen(project: RecentProjectDto) {
    if (project.availability !== "ready" || busy !== null || recentBusyId !== null) return;
    flushSync(() => { setRecentBusyId(project.id); setError(null); });
    try { onProjectReady(await openRecentProject(project.id)); }
    catch (caughtError) { setError({ operation: "recentOpen", error: normalizeCommandError(caughtError) }); }
    finally { setRecentBusyId(null); }
  }

  async function handleRecentRemove(project: RecentProjectDto) {
    if (busy !== null || recentBusyId !== null) return;
    flushSync(() => { setRecentBusyId(project.id); setRecentActionError(null); });
    try {
      await forgetRecentProject(project.id);
      setRecentState((current) => reduceRecentProjectsState(current, { type: "removed", projectId: project.id }));
    } catch (caughtError) { setRecentActionError(normalizeCommandError(caughtError)); }
    finally { setRecentBusyId(null); }
  }

  async function handleCancel() {
    if (!jobId || cancelRequested) return;
    flushSync(() => setCancelRequested(true));
    try { await cancelSourcePackage(jobId); }
    catch (caughtError) {
      setCancelRequested(false);
      setError({ operation: "create", error: normalizeCommandError(caughtError) });
    }
  }

  const details = progressDetails(progress);
  const launcherDisabled = busy !== null || recentBusyId !== null;
  const themeOptions = themeRegistry.map((entry) => ({ value: entry.id, label: `${entry.family ?? "Themes"} · ${entry.displayName}` }));
  const accentOptions = [
    { value: "", label: "Use theme default" },
    { value: "#c6a0f6", label: "Violet" },
    { value: "#8aadf4", label: "Blue" },
    { value: "#a6da95", label: "Green" },
    { value: "#f5a97f", label: "Peach" },
  ];
  const handlePickerError = (message: string) => setError({ operation: mode === "open" ? "open" : "create", error: { code: "pathPicker", message } });

  return (
    <main className="launcher-shell">
      <WindowChrome context="Aeria" mode="launcher" />
      <div className="launcher-layout">
        <aside className="launcher-sidebar">
          <div className="launcher-sidebar-head">
            <strong>Projects</strong>
            <span>Open or create a local Aeria workspace.</span>
          </div>
          <nav className="launcher-nav" aria-label="Project launcher">
            {(["recent", "open", "create", "settings"] as const).map((item) => (
              <button className={mode === item ? "launcher-nav-button active" : "launcher-nav-button"} type="button" key={item} onClick={() => setMode(item)}>
                {item === "settings" ? <Icon name="settings" size={14} /> : null}
                <span>{item.slice(0, 1).toUpperCase() + item.slice(1)}</span>
              </button>
            ))}
          </nav>
          <div className="launcher-sidebar-foot">Aeria desktop</div>
        </aside>

        <section className="launcher-content" aria-live="polite">
          {error ? <ErrorBanner title={launcherErrorTitle(error.operation)} error={error.error} onDismiss={() => setError(null)} /> : null}
          {mode === "recent" ? (
            <section className="launcher-view" aria-labelledby="recent-title">
              <div className="launcher-view-head">
                <div><h1 id="recent-title">Recent projects</h1><p>Continue working in a local project.</p></div>
                <span className="count-badge">{recentState.status === "loaded" ? recentState.projects.length : "—"}</span>
              </div>
              {recentState.status === "failed" && recentState.error ? <ErrorBanner title="Recent projects unavailable" error={recentState.error} onDismiss={() => setRecentState((current) => reduceRecentProjectsState(current, { type: "dismissError" }))} /> : null}
              {recentActionError ? <ErrorBanner title="Could not remove recent project" error={recentActionError} onDismiss={() => setRecentActionError(null)} /> : null}
              {recentState.status === "loading" ? <div className="launcher-list-state"><span className="spinner" /> Loading recent projects…</div> : null}
              {recentState.status === "failed" ? <p className="recent-empty">Recent projects unavailable.</p> : null}
              {recentState.status === "loaded" && recentState.projects.length === 0 ? <p className="recent-empty">Projects you successfully open or create will appear here.</p> : null}
              {recentState.status === "loaded" && recentState.projects.length > 0 ? (
                <div className="project-list">
                  {recentState.projects.map((project) => (
                    <article className="project-plate" key={project.id}>
                      <button className="project-open-area" type="button" disabled={launcherDisabled || project.availability !== "ready"} onClick={() => void handleRecentOpen(project)}>
                        <div className="project-main">
                        <strong className="project-name">{displayPathName(project.repositoryRoot)}</strong>
                        <code className="project-path" title={displayPath(project.repositoryRoot)}>{displayPath(project.repositoryRoot)}</code>
                        <span className={project.availability === "ready" ? "project-meta" : "project-meta issue"}>{project.availability === "ready" ? `${languageLabel(project.sourceLanguage)} source` : availabilityLabel(project.availability)}</span>
                        </div>
                      </button>
                      <div className="project-actions">
                        {project.availability === "ready" ? <button className="button primary-button compact-button" type="button" disabled={launcherDisabled} onClick={() => void handleRecentOpen(project)}>{recentBusyId === project.id ? "Opening…" : "Open"}</button> : null}
                        <RecentProjectActions project={project} disabled={launcherDisabled} onOpen={() => void handleRecentOpen(project)} onRemove={() => void handleRecentRemove(project)} />
                      </div>
                    </article>
                  ))}
                </div>
              ) : null}
            </section>
          ) : null}

          {mode === "open" || mode === "create" ? (
            <section className="launcher-view" aria-labelledby={`${mode}-title`}>
              <div className="launcher-view-head"><div><h1 id={`${mode}-title`}>{mode === "open" ? "Open project" : "Create project"}</h1><p>{mode === "open" ? "Open an existing local Aeria repository." : "Create a local project from an installed FFXIV source."}</p></div></div>
              <form className="launcher-form" onSubmit={handleSubmit}>
                <BrowseField id="repository-root" label="Repository root" value={repositoryRoot} onChange={setRepositoryRoot} onError={handlePickerError} placeholder="C:\\Projects\\my-translation" directory disabled={launcherDisabled} />
                {mode === "open" ? <BrowseField id="source-package-path" label="HSP source package" value={sourcePackagePath} onChange={setSourcePackagePath} onError={handlePickerError} placeholder="C:\\Sources\\source-en.hsp" hint="Exported .hsp package this repository was built from." disabled={launcherDisabled} /> : (
                  <>
                    <BrowseField id="game-path" label="Game installation" value={gamePath} onChange={setGamePath} onError={handlePickerError} placeholder="C:\\Games\\FINAL FANTASY XIV" directory disabled={launcherDisabled} />
                    <div className="launcher-field"><label htmlFor="source-language">Source language</label><SelectMenu value={sourceLanguage} options={sourceLanguages} onChange={setSourceLanguage} label="Source language" disabled={launcherDisabled} /></div>
                  </>
                )}
                <button className="button primary-button launcher-submit" type="submit" disabled={launcherDisabled}>{busy === "open" ? "Opening…" : busy === "create" ? "Creating…" : mode === "open" ? "Open project" : "Create project"}</button>
              </form>
              {busy === "create" ? <section className="atlas-progress" aria-label="Source package progress"><div className="atlas-progress-heading"><span className="spinner" /><strong>{phaseLabel(progress)}</strong></div><div className="progress-facts"><span>Sheet<strong>{details.sheet ?? "—"}</strong></span><span>Language<strong>{details.language ?? "—"}</strong></span><span>Rows<strong>{details.rows === null ? "—" : details.rows.toLocaleString()}</strong></span></div><button className="button secondary-button cancel-button" type="button" onClick={() => void handleCancel()} disabled={jobId === null || cancelRequested}>{cancelRequested ? "Cancelling…" : "Cancel"}</button></section> : null}
            </section>
          ) : null}

          {mode === "settings" ? (
            <section className="launcher-view" aria-labelledby="settings-title">
              <div className="launcher-view-head"><div><h1 id="settings-title">Settings</h1><p>Appearance defaults for Aeria.</p></div></div>
              <div className="settings-panel">
                <section className="settings-section"><h2>Appearance</h2><div className="setting-row"><span className="setting-label">Theme</span><SelectMenu value={theme.id} options={themeOptions} onChange={setThemeId} label="Theme" /></div><div className="setting-row"><span className="setting-label">Accent override</span><SelectMenu value={accentOverride ?? ""} options={accentOptions} onChange={(value) => setAccentOverride(value || null)} label="Accent override" /></div></section>
                <section className="settings-section"><h2>Typography</h2><div className="setting-row"><span className="setting-label">Interface</span><span className="setting-value">Segoe UI Variable <small>12 px</small></span></div><div className="setting-row"><span className="setting-label">Content</span><span className="setting-value">Segoe UI Variable <small>12 / 13 px</small></span></div><div className="setting-row"><span className="setting-label">Monospace</span><span className="setting-value">Cascadia Mono <small>10.5 / 11.5 px</small></span></div></section>
              </div>
            </section>
          ) : null}
        </section>
      </div>
    </main>
  );
}
