import { useEffect, useMemo, useRef, useState, type CSSProperties, type FormEvent } from "react";
import { flushSync } from "react-dom";
import { listen } from "@tauri-apps/api/event";
import { open as openNativeDialog } from "@tauri-apps/plugin-dialog";
import { DropdownMenu } from "radix-ui";
import {
  appInfo,
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
import { SettingsDialog } from "./SettingsDialog";
import { WindowChrome } from "./WindowChrome";
import { displayPath, displayPathName } from "../pathDisplay";
import { formatRelativeTime } from "../timeDisplay";
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
import { IconButton } from "../ui/primitives/IconButton";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";

type LauncherView = "recent" | "open" | "create";
type LauncherJob = "open" | "create";

type ProjectLauncherProps = {
  initialError: CommandError | null;
  onProjectReady: (result: ProjectOpenResultDto) => void;
};

const sourceLanguages = [
  { value: "en", label: "English", short: "EN" },
  { value: "ja", label: "Japanese", short: "JA" },
  { value: "de", label: "German", short: "DE" },
  { value: "fr", label: "French", short: "FR" },
] as const;

type SourceLanguage = (typeof sourceLanguages)[number]["value"];

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

function progressFraction(event: AtlasEvent | null): number | null {
  if (!event || event.type !== "progress" || event.sheetIndex === null || !event.sheetCount) return null;
  return Math.min(1, Math.max(0, (event.sheetIndex + (event.sheetCompleted ? 1 : 0)) / event.sheetCount));
}

function availabilityLabel(availability: RecentProjectAvailability): string {
  switch (availability) {
    case "ready": return "Ready";
    case "repositoryMissing": return "Repository missing";
    case "sourcePackageMissing": return "Source package missing";
    case "repositoryAndSourceMissing": return "Repository and source package missing";
  }
}

function languageShort(value: string): string {
  return sourceLanguages.find((language) => language.value === value)?.short ?? value.toUpperCase();
}

function initials(name: string): string {
  const words = name.replace(/[^\p{L}\p{N}]+/gu, " ").trim().split(" ").filter(Boolean);
  const letters = words.length > 1 ? `${words[0]![0]}${words[1]![0]}` : (words[0] ?? "?").slice(0, 2);
  return letters.toUpperCase();
}

function avatarHue(value: string): number {
  let hash = 0;
  for (const character of value) hash = (hash * 31 + character.codePointAt(0)!) % 360;
  return hash;
}

type PathFieldProps = {
  id: string;
  label: string;
  value: string;
  placeholder: string;
  hint?: string | undefined;
  directory?: boolean;
  disabled: boolean;
  onChange: (value: string) => void;
  onError: (message: string) => void;
};

function PathField({ id, label, value, placeholder, hint, directory = false, disabled, onChange, onError }: PathFieldProps) {
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
    <div className="field">
      <label className="field-label" htmlFor={id}>{label}</label>
      <div className="path-field">
        <input className="input mono" id={id} value={value} onChange={(event) => onChange(event.target.value)} placeholder={placeholder} autoComplete="off" spellCheck={false} disabled={disabled} required />
        <button className="button button-secondary" type="button" disabled={disabled} onClick={() => void handleBrowse()} aria-label={`Browse for ${label.toLowerCase()}`}>
          <UiIcon icon="folderOpen" size="sm" /> Browse
        </button>
      </div>
      {hint ? <small className="field-hint">{hint}</small> : null}
    </div>
  );
}

type RecentProjectRowProps = {
  project: RecentProjectDto;
  disabled: boolean;
  opening: boolean;
  now: number;
  onOpen: () => void;
  onRemove: () => void;
};

function RecentProjectRow({ project, disabled, opening, now, onOpen, onRemove }: RecentProjectRowProps) {
  const name = displayPathName(project.repositoryRoot);
  const ready = project.availability === "ready";
  return (
    <li className={ready ? "recent-row" : "recent-row unavailable"}>
      <button className="recent-open" type="button" disabled={disabled || !ready} onClick={onOpen} title={ready ? `Open ${name}` : availabilityLabel(project.availability)}>
        <span className="recent-avatar" style={{ "--avatar-hue": avatarHue(name) } as CSSProperties} aria-hidden="true">{initials(name)}</span>
        <span className="recent-text">
          <strong>{name}</strong>
          <span className="recent-path mono">{displayPath(project.repositoryRoot)}</span>
        </span>
        <span className="recent-meta">
          {ready ? <>
            <span className="chip">{languageShort(project.sourceLanguage)}</span>
            {project.gameVersion ? <span className="recent-version mono" title="Game version">{project.gameVersion}</span> : null}
          </> : <span className="chip chip-warn"><UiIcon icon="triangleAlert" size="xs" />{availabilityLabel(project.availability)}</span>}
          <span className="recent-time">{opening ? "Opening…" : formatRelativeTime(project.lastOpenedAtUnixMs, now)}</span>
        </span>
        {ready ? <span className="recent-go" aria-hidden="true"><UiIcon icon="arrowRight" size="sm" /></span> : null}
      </button>
      <DropdownMenu.Root>
        <DropdownMenu.Trigger asChild disabled={disabled}>
          <button className="icon-button icon-button-ghost recent-more" type="button" aria-label={`Actions for ${name}`}><UiIcon icon="ellipsis" size="md" /></button>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content className="menu-content" align="end" sideOffset={4}>
            <DropdownMenu.Item className="menu-item" disabled={!ready} onSelect={onOpen}><span className="menu-item-label">Open</span></DropdownMenu.Item>
            <DropdownMenu.Separator className="menu-separator" />
            <DropdownMenu.Item className="menu-item danger" onSelect={onRemove}><span className="menu-item-label">Remove from recent projects</span></DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
    </li>
  );
}

function LauncherAction({ icon, title, description, active, disabled, onClick }: { icon: UiIconName; title: string; description: string; active: boolean; disabled: boolean; onClick: () => void }) {
  return (
    <button className={active ? "launcher-action active" : "launcher-action"} type="button" aria-pressed={active} disabled={disabled} onClick={onClick}>
      <span className="launcher-action-icon"><UiIcon icon={icon} size="lg" /></span>
      <span className="launcher-action-text"><strong>{title}</strong><small>{description}</small></span>
      <UiIcon icon="chevronRight" size="sm" className="launcher-action-chevron" />
    </button>
  );
}

export function ProjectLauncher({ initialError, onProjectReady }: ProjectLauncherProps) {
  const [view, setView] = useState<LauncherView>("recent");
  const [repositoryRoot, setRepositoryRoot] = useState("");
  const [sourcePackagePath, setSourcePackagePath] = useState("");
  const [gamePath, setGamePath] = useState("");
  const [sourceLanguage, setSourceLanguage] = useState<SourceLanguage>("en");
  const [busy, setBusy] = useState<LauncherJob | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const [cancelRequested, setCancelRequested] = useState(false);
  const [progress, setProgress] = useState<AtlasEvent | null>(null);
  const [error, setError] = useState<LauncherError | null>(initialError ? { operation: "open", error: initialError } : null);
  const [recentState, setRecentState] = useState<RecentProjectsState>(initialRecentProjectsState);
  const [recentActionError, setRecentActionError] = useState<CommandError | null>(null);
  const [recentBusyId, setRecentBusyId] = useState<string | null>(null);
  const [recentQuery, setRecentQuery] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [version, setVersion] = useState<string | null>(null);
  const busyRef = useRef<LauncherJob | null>(null);
  const jobIdRef = useRef<string | null>(null);
  const now = useMemo(() => Date.now(), [recentState]);

  useEffect(() => setError(initialError ? { operation: "open", error: initialError } : null), [initialError]);

  useEffect(() => {
    let disposed = false;
    void listRecentProjects()
      .then((projects) => { if (!disposed) setRecentState({ status: "loaded", projects }); })
      .catch((caughtError: unknown) => { if (!disposed) setRecentState({ status: "failed", error: normalizeCommandError(caughtError) }); });
    void appInfo().then((info) => { if (!disposed) setVersion(info.version); }).catch(() => undefined);
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

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (view === "recent") return;
    const job: LauncherJob = view;
    busyRef.current = job;
    jobIdRef.current = null;
    flushSync(() => {
      setBusy(job);
      setError(null);
      setProgress(null);
      setJobId(null);
      setCancelRequested(false);
    });
    try {
      const result = job === "open"
        ? await openProject(repositoryRoot, sourcePackagePath)
        : await (async () => {
            const started = await startSourcePackage();
            jobIdRef.current = started.jobId;
            setJobId(started.jobId);
            return initializeProjectFromGame(started.jobId, repositoryRoot, gamePath, sourceLanguage, "und");
          })();
      onProjectReady(result);
    } catch (caughtError) {
      setError({ operation: job, error: normalizeCommandError(caughtError) });
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

  const launcherDisabled = busy !== null || recentBusyId !== null;
  const handlePickerError = (message: string) => setError({ operation: view === "open" ? "open" : "create", error: { code: "pathPicker", message } });
  const fraction = progressFraction(progress);
  const recentProjects = recentState.status === "loaded" ? recentState.projects : [];
  const normalizedQuery = recentQuery.trim().toLocaleLowerCase();
  const visibleProjects = normalizedQuery
    ? recentProjects.filter((project) => displayPath(project.repositoryRoot).toLocaleLowerCase().includes(normalizedQuery))
    : recentProjects;
  const showView = (next: LauncherView) => { if (!launcherDisabled) setView((current) => current === next ? "recent" : next); };

  return (
    <main className="launcher">
      <WindowChrome mode="launcher" actions={<IconButton icon="settings" label="Settings" onClick={() => setSettingsOpen(true)} />} />
      <div className="launcher-body">
        <aside className="launcher-hero">
          <div className="launcher-brand">
            <h1>Aeria</h1>
            <p>A translation workbench for Final Fantasy XIV game text.</p>
          </div>
          <nav className="launcher-actions" aria-label="Start">
            <LauncherAction icon="folderOpen" title="Open project" description="Repository and source package" active={view === "open"} disabled={launcherDisabled} onClick={() => showView("open")} />
            <LauncherAction icon="folderPlus" title="New project" description="From your game installation" active={view === "create"} disabled={launcherDisabled} onClick={() => showView("create")} />
          </nav>
          <footer className="launcher-foot">
            <span>{version ? `Version ${version}` : "Aeria desktop"}</span>
            <span className="launcher-foot-dot" aria-hidden="true" />
            <span>Local projects</span>
          </footer>
        </aside>

        <section className="launcher-panel" aria-live="polite">
          {error ? <ErrorBanner title={launcherErrorTitle(error.operation)} error={error.error} onDismiss={() => setError(null)} /> : null}

          {view === "recent" ? (
            <div className="launcher-view" key="recent">
              <header className="launcher-view-head">
                <div>
                  <h2>Recent projects</h2>
                  <p>{recentState.status === "loaded" && recentProjects.length > 0 ? `${recentProjects.length} local ${recentProjects.length === 1 ? "project" : "projects"}` : "Continue where you left off."}</p>
                </div>
                {recentProjects.length > 3 ? (
                  <label className="search-field">
                    <UiIcon icon="search" size="sm" />
                    <input value={recentQuery} onChange={(event) => setRecentQuery(event.target.value)} placeholder="Filter projects" aria-label="Filter recent projects" spellCheck={false} />
                  </label>
                ) : null}
              </header>
              {recentState.status === "failed" && recentState.error ? <ErrorBanner title="Recent projects unavailable" error={recentState.error} onDismiss={() => setRecentState((current) => reduceRecentProjectsState(current, { type: "dismissError" }))} /> : null}
              {recentActionError ? <ErrorBanner title="Could not remove recent project" error={recentActionError} onDismiss={() => setRecentActionError(null)} /> : null}
              {recentState.status === "loading" ? <div className="launcher-state"><span className="spinner" /> Loading recent projects…</div> : null}
              {recentState.status === "failed" ? <div className="launcher-empty"><UiIcon icon="cloudOff" size="xl" /><strong>Recent projects unavailable</strong><p>You can still open or create a project.</p></div> : null}
              {recentState.status === "loaded" && recentProjects.length === 0 ? (
                <div className="launcher-empty">
                  <UiIcon icon="folder" size="xl" />
                  <strong>No recent projects yet</strong>
                  <p>Projects you open or create will appear here.</p>
                </div>
              ) : null}
              {visibleProjects.length > 0 ? (
                <ul className="recent-list" aria-label="Recent projects">
                  {visibleProjects.map((project) => (
                    <RecentProjectRow key={project.id} project={project} now={now} disabled={launcherDisabled} opening={recentBusyId === project.id} onOpen={() => void handleRecentOpen(project)} onRemove={() => void handleRecentRemove(project)} />
                  ))}
                </ul>
              ) : recentProjects.length > 0 ? <div className="launcher-state">No projects match “{recentQuery.trim()}”.</div> : null}
            </div>
          ) : (
            <div className="launcher-view" key={view}>
              <header className="launcher-view-head">
                <div className="launcher-view-title">
                  <IconButton icon="arrowLeft" label="Back to recent projects" disabled={launcherDisabled} onClick={() => setView("recent")} />
                  <div>
                    <h2>{view === "open" ? "Open project" : "New project"}</h2>
                    <p>{view === "open" ? "Open an existing local Aeria repository." : "Build an HSP source package from an installed game."}</p>
                  </div>
                </div>
              </header>
              <form className="launcher-form" onSubmit={(event) => void handleSubmit(event)}>
                <PathField id="repository-root" label={view === "open" ? "Repository" : "Repository folder"} value={repositoryRoot} onChange={setRepositoryRoot} onError={handlePickerError} placeholder="C:\Projects\my-translation" directory disabled={launcherDisabled} hint={view === "create" ? "The folder where Aeria keeps the translation workspace." : undefined} />
                {view === "open" ? (
                  <PathField id="source-package-path" label="Source package" value={sourcePackagePath} onChange={setSourcePackagePath} onError={handlePickerError} placeholder="C:\Sources\source-en.hsp" hint="The .hsp package this repository was built from." disabled={launcherDisabled} />
                ) : (
                  <>
                    <PathField id="game-path" label="Game installation" value={gamePath} onChange={setGamePath} onError={handlePickerError} placeholder="C:\Games\FINAL FANTASY XIV" directory disabled={launcherDisabled} />
                    <div className="field">
                      <span className="field-label" id="source-language-label">Source language</span>
                      <Segmented size="md" label="Source language" value={sourceLanguage} onChange={setSourceLanguage} disabled={launcherDisabled} options={sourceLanguages.map((language) => ({ value: language.value, label: language.label }))} />
                    </div>
                  </>
                )}
                {busy === "create" ? (
                  <section className="job-progress" aria-label="Source package progress">
                    <div className="job-progress-head">
                      <span className="spinner" aria-hidden="true" />
                      <strong>{phaseLabel(progress)}</strong>
                      {fraction !== null ? <span className="job-progress-percent">{Math.round(fraction * 100)}%</span> : null}
                    </div>
                    <div className={fraction === null ? "progress-track indeterminate" : "progress-track"}><span style={{ width: `${(fraction ?? 0.3) * 100}%` }} /></div>
                    {progress?.type === "progress" ? (
                      <div className="job-progress-facts">
                        <span>{progress.sheet ?? "—"}</span>
                        {progress.language ? <span>{progress.language}</span> : null}
                        {progress.rowsProcessed !== null ? <span>{progress.rowsProcessed.toLocaleString()} rows</span> : null}
                      </div>
                    ) : null}
                  </section>
                ) : null}
                <div className="launcher-form-actions">
                  {busy === "create" ? (
                    <button className="button button-secondary" type="button" onClick={() => void handleCancel()} disabled={jobId === null || cancelRequested}>{cancelRequested ? "Cancelling…" : "Cancel"}</button>
                  ) : (
                    <button className="button button-ghost" type="button" disabled={launcherDisabled} onClick={() => setView("recent")}>Back</button>
                  )}
                  <button className="button button-primary" type="submit" disabled={launcherDisabled}>
                    {busy === "open" ? "Opening…" : busy === "create" ? "Creating…" : view === "open" ? "Open project" : "Create project"}
                  </button>
                </div>
              </form>
            </div>
          )}
        </section>
      </div>
      <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
    </main>
  );
}
