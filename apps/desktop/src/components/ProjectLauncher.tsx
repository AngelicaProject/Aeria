import { useEffect, useMemo, useRef, useState, type CSSProperties, type FormEvent } from "react";
import { flushSync } from "react-dom";
import { listen } from "@tauri-apps/api/event";
import { open as openNativeDialog } from "@tauri-apps/plugin-dialog";
import { DropdownMenu } from "radix-ui";
import {
  appInfo,
  cancelSourcePackage,
  defaultProjectsDirectory,
  forgetRecentProject,
  gameSettings,
  gitCloneRepository,
  initializeProjectFromGame,
  listRecentProjects,
  normalizeCommandError,
  openProject,
  openProjectFromGame,
  openRecentProject,
  previewSourceUpdate,
  sourceAvailability,
  startSourcePackage,
  updateProjectFromGame,
} from "../ipc";
import type {
  AtlasEvent,
  CommandError,
  GameSettingsDto,
  ProjectOpenResultDto,
  RecentProjectAvailability,
  RecentProjectDto,
  SourceAvailability,
  SourcePackageEventPayload,
  SourceUpdateReportDto,
} from "../types";
import { ErrorBanner } from "./ErrorBanner";
import { gameInstallationFacts } from "./GameSettings";
import { SettingsDialog, type SettingsSection } from "./SettingsDialog";
import { SourceUpdateDialog } from "./SourceUpdateDialog";
import { WindowChrome } from "./WindowChrome";
import { displayPath, displayPathName } from "../pathDisplay";
import { projectFolder } from "../projectFolder";
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
  type LauncherErrorOperation,
} from "../launcherErrorState";
import { IconButton } from "../ui/primitives/IconButton";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { useI18n, type Translate } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";

type LauncherView = "recent" | "open" | "clone" | "create" | "update";
type LauncherJob = "open" | "clone" | "create" | "update";

/** A source update that opening requires and the user has not yet accepted. */
type PendingSourceUpdate = {
  report: SourceUpdateReportDto;
  operation: LauncherErrorOperation;
  apply: () => Promise<ProjectOpenResultDto>;
};

/**
 * Jobs that run Harmonia Atlas and report source-package progress. Opening
 * builds a package only when no local one matches; a clone continues as an
 * open once Git finishes.
 */
function runsAtlas(job: LauncherJob | null): boolean {
  return job === "open" || job === "create" || job === "update";
}

type ProjectLauncherProps = {
  initialError: CommandError | null;
  onProjectReady: (result: ProjectOpenResultDto) => void;
};

const sourceLanguages = [
  { value: "en", label: "language.en", short: "EN" },
  { value: "ja", label: "language.ja", short: "JA" },
  { value: "de", label: "language.de", short: "DE" },
  { value: "fr", label: "language.fr", short: "FR" },
] as const satisfies ReadonlyArray<{ value: string; label: MessageKey; short: string }>;

type SourceLanguage = (typeof sourceLanguages)[number]["value"];

const phaseLabels: Readonly<Record<string, MessageKey>> = {
  inspectInstallation: "launcher.phase.inspectInstallation",
  extractSource: "launcher.phase.extractSource",
  verifySource: "launcher.phase.verifySource",
  scanEvidence: "launcher.phase.scanEvidence",
  writePackage: "launcher.phase.buildPackage",
  buildPackage: "launcher.phase.buildPackage",
  validatePackage: "launcher.phase.validatePackage",
};

function phaseLabel(event: AtlasEvent | null, t: Translate): string {
  if (!event) return t("launcher.phase.preparing");
  if (event.type === "started") return t("launcher.phase.started");
  if (event.type === "completed") return t("launcher.phase.completed");
  if (event.type === "failed") return t("launcher.phase.failed");
  const label = phaseLabels[event.phase];
  return label ? t(label) : event.phase;
}

function progressFraction(event: AtlasEvent | null): number | null {
  if (!event || event.type !== "progress" || event.sheetIndex === null || !event.sheetCount) return null;
  return Math.min(1, Math.max(0, (event.sheetIndex + (event.sheetCompleted ? 1 : 0)) / event.sheetCount));
}

function availabilityLabel(availability: RecentProjectAvailability): MessageKey {
  switch (availability) {
    case "ready": return "launcher.availability.ready";
    case "repositoryMissing": return "launcher.availability.repositoryMissing";
    case "sourcePackageMissing": return "launcher.availability.sourcePackageMissing";
    case "repositoryAndSourceMissing": return "launcher.availability.repositoryAndSourceMissing";
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
  required?: boolean;
  disabled: boolean;
  onChange: (value: string) => void;
  onError: (message: string) => void;
};

/** A folder path input with a native folder picker. */
function PathField({ id, label, value, placeholder, hint, required = true, disabled, onChange, onError }: PathFieldProps) {
  const { t, locale } = useI18n();
  async function handleBrowse() {
    try {
      const selection = await openNativeDialog({ directory: true, multiple: false });
      if (typeof selection === "string") onChange(selection);
    } catch (error) {
      onError(error instanceof Error ? error.message : t("launcher.pathPickerFailed"));
    }
  }

  return (
    <div className="field">
      <label className="field-label" htmlFor={id}>{label}</label>
      <div className="path-field">
        <input className="input mono" id={id} value={value} onChange={(event) => onChange(event.target.value)} placeholder={placeholder} autoComplete="off" spellCheck={false} disabled={disabled} required={required} />
        <button className="button button-secondary" type="button" disabled={disabled} onClick={() => void handleBrowse()} aria-label={t("launcher.browseFor", { field: label.toLocaleLowerCase(locale) })}>
          <UiIcon icon="folderOpen" size="sm" /> {t("launcher.browse")}
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
  onUpdate: () => void;
  onRemove: () => void;
};

function RecentProjectRow({ project, disabled, opening, now, onOpen, onUpdate, onRemove }: RecentProjectRowProps) {
  const { t, locale } = useI18n();
  const name = displayPathName(project.repositoryRoot);
  const ready = project.availability === "ready";
  return (
    <li className={ready ? "recent-row" : "recent-row unavailable"}>
      <button className="recent-open" type="button" disabled={disabled || !ready} onClick={onOpen} title={ready ? t("launcher.openNamed", { name }) : t(availabilityLabel(project.availability))}>
        <span className="recent-avatar" style={{ "--avatar-hue": avatarHue(name) } as CSSProperties} aria-hidden="true">{initials(name)}</span>
        <span className="recent-text">
          <strong>{name}</strong>
          <span className="recent-path mono">{displayPath(project.repositoryRoot)}</span>
        </span>
        <span className="recent-meta">
          {ready ? <>
            <span className="chip">{languageShort(project.sourceLanguage)}</span>
            {project.gameVersion ? <span className="recent-version mono" title={t("launcher.gameVersion")}>{project.gameVersion}</span> : null}
          </> : <span className="chip chip-warn"><UiIcon icon="triangleAlert" size="xs" />{t(availabilityLabel(project.availability))}</span>}
          <span className="recent-time">{opening ? t("launcher.opening") : formatRelativeTime(project.lastOpenedAtUnixMs, now, locale, t("time.justNow"))}</span>
        </span>
        {ready ? <span className="recent-go" aria-hidden="true"><UiIcon icon="arrowRight" size="sm" /></span> : null}
      </button>
      <DropdownMenu.Root>
        <DropdownMenu.Trigger asChild disabled={disabled}>
          <button className="icon-button icon-button-ghost recent-more" type="button" aria-label={t("launcher.actionsFor", { name })}><UiIcon icon="ellipsis" size="md" /></button>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content className="menu-content" align="end" sideOffset={4}>
            <DropdownMenu.Item className="menu-item" disabled={!ready} onSelect={onOpen}><span className="menu-item-label">{t("common.open")}</span></DropdownMenu.Item>
            <DropdownMenu.Item className="menu-item" disabled={project.availability === "repositoryMissing" || project.availability === "repositoryAndSourceMissing"} onSelect={onUpdate}><span className="menu-item-label">{t("launcher.updateFromGame")}</span></DropdownMenu.Item>
            <DropdownMenu.Separator className="menu-separator" />
            <DropdownMenu.Item className="menu-item danger" onSelect={onRemove}><span className="menu-item-label">{t("launcher.removeRecent")}</span></DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
    </li>
  );
}

/** The game installation launcher jobs use, with a shortcut to change it. */
function GameStatus({ settings, sources, disabled, onChange }: { settings: GameSettingsDto | null; sources: SourceAvailability | null; disabled: boolean; onChange: () => void }) {
  const { t } = useI18n();
  const active = settings?.active ?? null;
  const detail = settings === null
    ? t("game.loading")
    : active ? gameInstallationFacts(active, t) : t(settings.configuredPath ? "game.invalid" : "game.missing");
  return (
    <div className={settings !== null && !active ? "launcher-game missing" : "launcher-game"}>
      <UiIcon icon={settings !== null && !active ? "triangleAlert" : "gamepad"} size="md" />
      <span className="launcher-game-text">
        <span className="field-label">{t("launcher.gameLabel")}</span>
        {active ? <span className="mono launcher-game-path" title={active.path}>{displayPath(active.path)}</span> : null}
        <small className="launcher-game-facts">
          <span className="launcher-game-detail">{detail}</span>
          {active && sources === "ready" ? <span className="chip chip-added" title={t("launcher.sourcesReadyHint")}>{t("launcher.sourcesReady")}</span> : null}
          {active && sources === "build" ? <span className="chip chip-warn" title={t("launcher.sourcesBuildHint")}>{t("launcher.sourcesBuild")}</span> : null}
        </small>
      </span>
      <button className="button button-secondary" type="button" disabled={disabled} onClick={onChange}>{t(active ? "launcher.gameChange" : "launcher.gameChoose")}</button>
    </div>
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
  const { t } = useI18n();
  const [view, setView] = useState<LauncherView>("recent");
  const [repositoryRoot, setRepositoryRoot] = useState("");
  const [game, setGame] = useState<GameSettingsDto | null>(null);
  const [sources, setSources] = useState<SourceAvailability | null>(null);
  const [cloneUrl, setCloneUrl] = useState("");
  const [cloneParent, setCloneParent] = useState("");
  const [projectName, setProjectName] = useState("");
  const [projectParent, setProjectParent] = useState("");
  const [defaultDirectory, setDefaultDirectory] = useState<string | null>(null);
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
  const [settingsSection, setSettingsSection] = useState<SettingsSection>("appearance");
  const [version, setVersion] = useState<string | null>(null);
  const [pendingUpdate, setPendingUpdate] = useState<PendingSourceUpdate | null>(null);
  const [applyingUpdate, setApplyingUpdate] = useState(false);
  const busyRef = useRef<LauncherJob | null>(null);
  const jobIdRef = useRef<string | null>(null);
  const now = useMemo(() => Date.now(), [recentState]);

  useEffect(() => setError(initialError ? { operation: "open", error: initialError } : null), [initialError]);

  // Tells whether the job in the current form finds a package or builds one.
  // Clones are unknown until the repository exists.
  useEffect(() => {
    setSources(null);
    if (busy !== null || !game?.active || (view !== "open" && view !== "update" && view !== "create")) return;
    const root = repositoryRoot.trim();
    if (view !== "create" && !root) return;
    let active = true;
    const timer = window.setTimeout(() => {
      sourceAvailability(view === "create" ? null : root, view === "create" ? sourceLanguage : null, view === "open")
        .then((next) => { if (active) setSources(next); })
        .catch(() => undefined);
    }, 300);
    return () => { active = false; window.clearTimeout(timer); };
  }, [view, repositoryRoot, sourceLanguage, game, busy]);

  useEffect(() => {
    let disposed = false;
    void listRecentProjects()
      .then((projects) => { if (!disposed) setRecentState({ status: "loaded", projects }); })
      .catch((caughtError: unknown) => { if (!disposed) setRecentState({ status: "failed", error: normalizeCommandError(caughtError) }); });
    void appInfo().then((info) => { if (!disposed) setVersion(info.version); }).catch(() => undefined);
    void defaultProjectsDirectory().then((path) => { if (!disposed) setDefaultDirectory(path); }).catch(() => undefined);
    void refreshGame(() => disposed);
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<SourcePackageEventPayload>("source-package-event", ({ payload }) => {
      if (!runsAtlas(busyRef.current) || (jobIdRef.current !== null && payload.jobId !== jobIdRef.current)) return;
      jobIdRef.current = payload.jobId;
      setJobId(payload.jobId);
      setProgress(payload.event);
    }).then((cleanup) => {
      if (disposed) cleanup();
      else { unlisten = cleanup; }
    }).catch((caughtError: unknown) => {
      if (disposed) return;
      if (import.meta.env.DEV) console.error("failed to register source-package-event listener", caughtError);
      setError({ operation: "create", error: sourcePackageListenerError(t) });
    });
    return () => { disposed = true; unlisten?.(); };
    // The listener registers once; `t` only formats a failure reported at registration.
  }, []);

  /** Re-reads the game setting; a failure shows as a missing installation. */
  async function refreshGame(disposed: () => boolean = () => false) {
    try {
      const next = await gameSettings();
      if (!disposed()) setGame(next);
    } catch {
      if (!disposed()) setGame({ configuredPath: null, active: null, detected: [] });
    }
  }

  function openSettings(section: SettingsSection) {
    setSettingsSection(section);
    setSettingsOpen(true);
  }

  function handleSettingsOpenChange(open: boolean) {
    setSettingsOpen(open);
    if (!open) void refreshGame();
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (view === "recent") return;
    const job: LauncherJob = view;
    const newProjectRoot = job === "create" ? projectFolder(projectParent.trim() || (defaultDirectory ?? ""), projectName) : null;
    if (job === "create" && (newProjectRoot === null || (!projectParent.trim() && defaultDirectory === null))) {
      setError({ operation: "create", error: { code: "invalidInput", message: t("launcher.projectNameInvalid") } });
      return;
    }
    busyRef.current = job;
    jobIdRef.current = null;
    flushSync(() => {
      setBusy(job);
      setError(null);
      setProgress(null);
      setJobId(null);
      setCancelRequested(false);
    });
    let operation: LauncherErrorOperation = job;
    try {
      const startAtlasJob = async () => {
        const started = await startSourcePackage();
        jobIdRef.current = started.jobId;
        setJobId(started.jobId);
        return started.jobId;
      };
      let root = repositoryRoot;
      if (job === "clone") {
        root = await gitCloneRepository(cloneUrl.trim(), cloneParent.trim() || null);
        // The clone exists now; continue as an open so a failure never clones again.
        operation = "open";
        busyRef.current = "open";
        flushSync(() => {
          setRepositoryRoot(root);
          setView("open");
          setBusy("open");
        });
      }
      if (job === "open" || job === "clone") {
        const opened = await openProjectFromGame(await startAtlasJob(), root);
        if (opened.status === "opened") {
          onProjectReady(opened.result);
        } else {
          const packagePath = opened.sourcePackagePath;
          setPendingUpdate({ report: opened.report, operation, apply: () => openProject(root, packagePath, true) });
        }
        return;
      }
      const result = job === "update"
        ? await updateProjectFromGame(await startAtlasJob(), repositoryRoot)
        : await initializeProjectFromGame(await startAtlasJob(), newProjectRoot!, sourceLanguage, "und");
      onProjectReady(result);
    } catch (caughtError) {
      setError({ operation, error: normalizeCommandError(caughtError) });
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
    catch (caughtError) {
      const error = normalizeCommandError(caughtError);
      if (error.code === "sourceUpdateRequired") {
        await offerSourceUpdate(
          "recentOpen",
          () => previewSourceUpdate(project.repositoryRoot, project.sourcePackagePath),
          () => openRecentProject(project.id, true),
        );
      } else {
        setError({ operation: "recentOpen", error });
      }
    }
    finally { setRecentBusyId(null); }
  }

  /** Previews a required source update and asks before applying it. */
  async function offerSourceUpdate(
    operation: LauncherErrorOperation,
    preview: () => Promise<SourceUpdateReportDto>,
    apply: () => Promise<ProjectOpenResultDto>,
  ) {
    try {
      setPendingUpdate({ report: await preview(), operation, apply });
    } catch (caughtError) {
      setError({ operation, error: normalizeCommandError(caughtError) });
    }
  }

  async function handleApplySourceUpdate() {
    if (!pendingUpdate || applyingUpdate) return;
    const { apply, operation } = pendingUpdate;
    flushSync(() => { setApplyingUpdate(true); setError(null); });
    try {
      const result = await apply();
      setPendingUpdate(null);
      onProjectReady(result);
    } catch (caughtError) {
      setPendingUpdate(null);
      setError({ operation, error: normalizeCommandError(caughtError) });
    } finally {
      setApplyingUpdate(false);
    }
  }

  function showUpdateFor(project: RecentProjectDto) {
    if (busy !== null || recentBusyId !== null) return;
    setRepositoryRoot(project.repositoryRoot);
    setError(null);
    setView("update");
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

  const launcherDisabled = busy !== null || recentBusyId !== null || applyingUpdate;
  const pickerOperation: LauncherErrorOperation = view === "open" ? "open" : view === "clone" ? "clone" : view === "update" ? "update" : "create";
  const handlePickerError = (message: string) => setError({ operation: pickerOperation, error: { code: "pathPicker", message } });
  const viewTitle: MessageKey = view === "open" ? "launcher.openProject" : view === "clone" ? "launcher.cloneProject" : view === "update" ? "launcher.updateProject" : "launcher.newProject";
  const viewDescription: MessageKey = view === "open" ? "launcher.openDescription" : view === "clone" ? "launcher.cloneDescription" : view === "update" ? "launcher.updateDescription" : "launcher.createDescription";
  const submitLabel: MessageKey = busy === "open"
    ? "launcher.opening"
    : busy === "clone"
      ? "launcher.cloning"
      : busy === "create"
      ? "launcher.creating"
      : busy === "update"
        ? "launcher.updating"
        : view === "open" ? "launcher.openProject" : view === "clone" ? "launcher.cloneProject" : view === "update" ? "launcher.updateProject" : "launcher.createProject";
  const fraction = progressFraction(progress);
  const recentProjects = recentState.status === "loaded" ? recentState.projects : [];
  const normalizedQuery = recentQuery.trim().toLocaleLowerCase();
  const visibleProjects = normalizedQuery
    ? recentProjects.filter((project) => displayPath(project.repositoryRoot).toLocaleLowerCase().includes(normalizedQuery))
    : recentProjects;
  const gameStatus = <GameStatus settings={game} sources={sources} disabled={launcherDisabled} onChange={() => openSettings("game")} />;
  const showView = (next: LauncherView) => { if (!launcherDisabled) setView((current) => current === next ? "recent" : next); };

  return (
    <main className="launcher">
      <WindowChrome mode="launcher" actions={<IconButton icon="settings" label={t("common.settings")} onClick={() => openSettings("appearance")} />} />
      <div className="launcher-body">
        <aside className="launcher-hero">
          <div className="launcher-brand">
            <h1>Aeria</h1>
            <p>{t("launcher.tagline")}</p>
          </div>
          <nav className="launcher-actions" aria-label={t("launcher.startNavigation")}>
            <LauncherAction icon="folderOpen" title={t("launcher.openProject")} description={t("launcher.openProjectHint")} active={view === "open"} disabled={launcherDisabled} onClick={() => showView("open")} />
            <LauncherAction icon="gitBranch" title={t("launcher.cloneProject")} description={t("launcher.cloneProjectHint")} active={view === "clone"} disabled={launcherDisabled} onClick={() => showView("clone")} />
            <LauncherAction icon="folderPlus" title={t("launcher.newProject")} description={t("launcher.newProjectHint")} active={view === "create"} disabled={launcherDisabled} onClick={() => showView("create")} />
            <LauncherAction icon="refreshCw" title={t("launcher.updateProject")} description={t("launcher.updateProjectHint")} active={view === "update"} disabled={launcherDisabled} onClick={() => showView("update")} />
          </nav>
          <footer className="launcher-foot">
            <span>{version ? t("launcher.version", { version }) : t("launcher.desktop")}</span>
            <span className="launcher-foot-dot" aria-hidden="true" />
            <span>{t("launcher.localProjects")}</span>
          </footer>
        </aside>

        <section className="launcher-panel" aria-live="polite">
          {error ? <ErrorBanner title={t(launcherErrorTitle(error.operation))} error={error.error} onDismiss={() => setError(null)} /> : null}

          {view === "recent" ? (
            <div className="launcher-view" key="recent">
              <header className="launcher-view-head">
                <div>
                  <h2>{t("launcher.recentTitle")}</h2>
                  <p>{recentState.status === "loaded" && recentProjects.length > 0 ? t("launcher.recentCount", { count: recentProjects.length }) : t("launcher.recentSubtitle")}</p>
                </div>
                {recentProjects.length > 3 ? (
                  <label className="search-field">
                    <UiIcon icon="search" size="sm" />
                    <input value={recentQuery} onChange={(event) => setRecentQuery(event.target.value)} placeholder={t("launcher.filterPlaceholder")} aria-label={t("launcher.filterLabel")} spellCheck={false} />
                  </label>
                ) : null}
              </header>
              {recentState.status === "failed" && recentState.error ? <ErrorBanner title={t("launcher.recentUnavailable")} error={recentState.error} onDismiss={() => setRecentState((current) => reduceRecentProjectsState(current, { type: "dismissError" }))} /> : null}
              {recentActionError ? <ErrorBanner title={t("launcher.removeFailed")} error={recentActionError} onDismiss={() => setRecentActionError(null)} /> : null}
              {recentState.status === "loading" ? <div className="launcher-state"><span className="spinner" /> {t("launcher.loadingRecent")}</div> : null}
              {recentState.status === "failed" ? <div className="launcher-empty"><UiIcon icon="cloudOff" size="xl" /><strong>{t("launcher.recentUnavailable")}</strong><p>{t("launcher.recentUnavailableHint")}</p></div> : null}
              {recentState.status === "loaded" && recentProjects.length === 0 ? (
                <div className="launcher-empty">
                  <UiIcon icon="folder" size="xl" />
                  <strong>{t("launcher.noRecent")}</strong>
                  <p>{t("launcher.noRecentHint")}</p>
                </div>
              ) : null}
              {visibleProjects.length > 0 ? (
                <ul className="recent-list" aria-label={t("launcher.recentTitle")}>
                  {visibleProjects.map((project) => (
                    <RecentProjectRow key={project.id} project={project} now={now} disabled={launcherDisabled} opening={recentBusyId === project.id} onOpen={() => void handleRecentOpen(project)} onUpdate={() => showUpdateFor(project)} onRemove={() => void handleRecentRemove(project)} />
                  ))}
                </ul>
              ) : recentProjects.length > 0 ? <div className="launcher-state">{t("launcher.noMatch", { query: recentQuery.trim() })}</div> : null}
            </div>
          ) : (
            <div className="launcher-view" key={view}>
              <header className="launcher-view-head">
                <div className="launcher-view-title">
                  <IconButton icon="arrowLeft" label={t("launcher.backToRecent")} disabled={launcherDisabled} onClick={() => setView("recent")} />
                  <div>
                    <h2>{t(viewTitle)}</h2>
                    <p>{t(viewDescription)}</p>
                  </div>
                </div>
              </header>
              <form className="launcher-form" onSubmit={(event) => void handleSubmit(event)}>
                {view === "clone" ? (
                  <>
                    <div className="field">
                      <label className="field-label" htmlFor="clone-url">{t("launcher.cloneUrl")}</label>
                      <input className="input mono" id="clone-url" value={cloneUrl} onChange={(event) => setCloneUrl(event.target.value)} placeholder="https://github.com/team/translation.git" autoComplete="off" spellCheck={false} disabled={launcherDisabled} required />
                      <small className="field-hint">{t("launcher.cloneUrlHint")}</small>
                    </div>
                    <PathField id="clone-parent" label={t("launcher.cloneParent")} value={cloneParent} onChange={setCloneParent} onError={handlePickerError} placeholder={defaultDirectory ?? "C:\\Projects"} hint={t("launcher.cloneParentHint")} required={false} disabled={launcherDisabled} />
                  </>
                ) : view === "create" ? (
                  <>
                    <div className="field">
                      <label className="field-label" htmlFor="project-name">{t("launcher.projectName")}</label>
                      <input className="input" id="project-name" value={projectName} onChange={(event) => setProjectName(event.target.value)} placeholder="ffxiv-translation" autoComplete="off" spellCheck={false} disabled={launcherDisabled} required />
                    </div>
                    <PathField id="project-parent" label={t("launcher.projectLocation")} value={projectParent} onChange={setProjectParent} onError={handlePickerError} placeholder={defaultDirectory ?? "C:\\Projects"} hint={t("launcher.projectLocationHint")} required={false} disabled={launcherDisabled} />
                    <div className="field">
                      <span className="field-label" id="source-language-label">{t("launcher.sourceLanguage")}</span>
                      <Segmented size="md" label={t("launcher.sourceLanguage")} value={sourceLanguage} onChange={setSourceLanguage} disabled={launcherDisabled} options={sourceLanguages.map((language) => ({ value: language.value, label: t(language.label) }))} />
                    </div>
                  </>
                ) : (
                  <PathField id="repository-root" label={t("launcher.repository")} value={repositoryRoot} onChange={setRepositoryRoot} onError={handlePickerError} placeholder="C:\Projects\my-translation" disabled={launcherDisabled} />
                )}
                {runsAtlas(busy) ? null : gameStatus}
                {runsAtlas(busy) ? (
                  <section className="job-progress" aria-label={t("launcher.progressLabel")}>
                    <div className="job-progress-head">
                      <span className="spinner" aria-hidden="true" />
                      <strong>{phaseLabel(progress, t)}</strong>
                      {fraction !== null ? <span className="job-progress-percent">{Math.round(fraction * 100)}%</span> : null}
                    </div>
                    <div className={fraction === null ? "progress-track indeterminate" : "progress-track"}><span style={{ width: `${(fraction ?? 0.3) * 100}%` }} /></div>
                    {progress?.type === "progress" ? (
                      <div className="job-progress-facts">
                        <span>{progress.sheet ?? "—"}</span>
                        {progress.language ? <span>{progress.language}</span> : null}
                        {progress.rowsProcessed !== null ? <span>{t("launcher.progressRows", { count: progress.rowsProcessed })}</span> : null}
                      </div>
                    ) : null}
                  </section>
                ) : null}
                <div className="launcher-form-actions">
                  {runsAtlas(busy) ? (
                    <button className="button button-secondary" type="button" onClick={() => void handleCancel()} disabled={jobId === null || cancelRequested}>{t(cancelRequested ? "launcher.cancelling" : "common.cancel")}</button>
                  ) : (
                    <button className="button button-ghost" type="button" disabled={launcherDisabled} onClick={() => setView("recent")}>{t("common.back")}</button>
                  )}
                  <button className="button button-primary" type="submit" disabled={launcherDisabled}>
                    {t(submitLabel)}
                  </button>
                </div>
              </form>
            </div>
          )}
        </section>
      </div>
      <SettingsDialog open={settingsOpen} onOpenChange={handleSettingsOpenChange} initialSection={settingsSection} />
      <SourceUpdateDialog
        open={pendingUpdate !== null}
        mode="confirm"
        report={pendingUpdate?.report ?? null}
        busy={applyingUpdate}
        onConfirm={() => void handleApplySourceUpdate()}
        onClose={() => setPendingUpdate(null)}
      />
    </main>
  );
}
