use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use aeria_atlas::{
    AtlasError, AtlasEvent, AtlasPackageRequest, AtlasPackageResult, AtlasPackageRunner,
    CancellationToken,
};
use aeria_core::{ReviewState, SourceBinding, TranslationUnitId};
use aeria_hsp::SourcePackage;
use aeria_projects::{ProjectMetadata, ProjectRegistry, REGISTRY_FILE_NAME, RegistryEntry};
use aeria_workspace::TranslationRowCursor;
use aeria_workspace::{ProjectSession, ProjectSessionError, WorkspaceStore};
use serde::Serialize;

use tauri::{Emitter, Manager, State};

use crate::dto::{
    DetachedUnitDto, GameOpenResultDto, ProjectOpenResultDto, ProjectSummaryDto, RecentProjectDto,
    ReviewStateDto, SheetProgressDto, SourceBindingDto, SourcePackageJobDto, SourceUpdateReportDto,
    TranslationOverlayDto, TranslationRowCursorDto, TranslationRowPageDto,
};
use crate::error::CommandError;
use crate::games::resolve_game_path;
use crate::paths::AeriaPaths;
use crate::source_store::{CurrentInputs, find_built_package, write_build_record};
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

pub(crate) const SOURCE_PACKAGES_DIRECTORY: &str = "source-packages";
const STAGING_DIRECTORY: &str = "staging";
const STAGING_FILE: &str = "source.hsp";

pub(crate) async fn run_blocking<T, F>(operation: F) -> CommandResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CommandResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| CommandError::internal_state(format!("desktop worker failed: {error}")))?
}

fn app_registry_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.aeria_data_dir()
        .map(|path| path.join(REGISTRY_FILE_NAME))
        .map_err(|error| format!("could not resolve the Aeria app-data directory: {error}"))
}

fn remember_project(
    state: &DesktopState,
    project: ProjectSummaryDto,
    registry_path: Result<PathBuf, String>,
) -> ProjectOpenResultDto {
    remember_project_with_update(state, project, registry_path, None)
}

fn remember_project_with_update(
    state: &DesktopState,
    project: ProjectSummaryDto,
    registry_path: Result<PathBuf, String>,
    source_update: Option<SourceUpdateReportDto>,
) -> ProjectOpenResultDto {
    let warning = match registry_path {
        Ok(path) => {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|duration| u64::try_from(duration.as_millis()).ok());
            match (timestamp, state.lock_registry()) {
                (Some(timestamp), Ok(_lock)) => {
                    let metadata = ProjectMetadata {
                        repository_root: PathBuf::from(&project.repository_root),
                        source_package_path: PathBuf::from(&project.source_package_path),
                        source_package_id: project.source_package_id.clone(),
                        source_language: project.source_language.clone(),
                        target_language: project.target_language.clone(),
                        game_version: project.game_version.clone(),
                    };
                    ProjectRegistry::new(path)
                        .upsert(&metadata, timestamp)
                        .err()
                        .map(|error| CommandError::registry_write(&error))
                }
                (None, _) => Some(CommandError::new(
                    "projectRegistryWrite",
                    "could not determine the current time for Recent projects",
                )),
                (_, Err(error)) => Some(CommandError::new("projectRegistryWrite", error.message)),
            }
        }
        Err(message) => Some(CommandError::new("projectRegistryWrite", message)),
    };
    ProjectOpenResultDto {
        project,
        warning,
        source_update,
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePackageEventPayload {
    pub job_id: String,
    pub event: AtlasEvent,
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Opens an existing project and makes it the active desktop project.
///
/// When the workspace is not current for the package source, the command
/// fails with `sourceUpdateRequired` unless `accept_source_update` is set, in
/// which case the deterministic source update is applied first.
///
/// # Errors
///
/// Returns a typed command error when source verification, workspace loading,
/// compatibility validation, the source update, or state locking fails.
pub async fn open_project(
    app: tauri::AppHandle,
    repository_root: String,
    source_package_path: String,
    accept_source_update: Option<bool>,
) -> CommandResult<ProjectOpenResultDto> {
    let cache_root = app
        .aeria_cache_dir()
        .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
    let registry_path = app_registry_path(&app);
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let source_package = open_source_package(Path::new(&source_package_path), cache_root)?;
        let (project, source_update) = open_package_with_state(
            &state,
            repository_root,
            source_package,
            accept_source_update.unwrap_or(false),
        )?;
        Ok(remember_project_with_update(
            &state,
            project,
            registry_path,
            source_update,
        ))
    })
    .await
}

#[cfg(test)]
pub(crate) fn open_project_with_state(
    state: &DesktopState,
    repository_root: String,
    source_package_path: String,
    cache_root: PathBuf,
) -> CommandResult<ProjectSummaryDto> {
    let replacement = ProjectSession::open(repository_root, source_package_path, cache_root)
        .map_err(CommandError::from)?;
    replace_project(state, replacement)
}

fn open_source_package(path: &Path, cache_root: PathBuf) -> CommandResult<SourcePackage> {
    SourcePackage::open(path, cache_root).map_err(|source| {
        CommandError::from(ProjectSessionError::Source {
            path: path.to_owned(),
            source,
        })
    })
}

/// Opens a project from a validated package, applying a required source
/// update only when the caller accepted it.
pub(crate) fn open_package_with_state(
    state: &DesktopState,
    repository_root: impl Into<PathBuf>,
    source_package: SourcePackage,
    accept_source_update: bool,
) -> CommandResult<(ProjectSummaryDto, Option<SourceUpdateReportDto>)> {
    if !accept_source_update {
        let replacement = ProjectSession::open_from_source_package(repository_root, source_package)
            .map_err(CommandError::from)?;
        return Ok((replace_project(state, replacement)?, None));
    }
    let (replacement, report) =
        ProjectSession::open_with_source_update(repository_root, source_package)
            .map_err(CommandError::from)?;
    let report = report.as_ref().map(SourceUpdateReportDto::from);
    Ok((replace_project(state, replacement)?, report))
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Plans the source update that opening a project with a package would
/// apply, without writing anything or changing the active project.
///
/// # Errors
///
/// Returns a typed command error when the package or workspace cannot be
/// read, the source language differs, or the plan cannot be built.
pub async fn preview_source_update(
    app: tauri::AppHandle,
    repository_root: String,
    source_package_path: String,
) -> CommandResult<SourceUpdateReportDto> {
    let cache_root = app
        .aeria_cache_dir()
        .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
    run_blocking(move || {
        preview_source_update_with_paths(
            Path::new(&repository_root),
            Path::new(&source_package_path),
            cache_root,
        )
    })
    .await
}

pub(crate) fn preview_source_update_with_paths(
    repository_root: &Path,
    source_package_path: &Path,
    cache_root: PathBuf,
) -> CommandResult<SourceUpdateReportDto> {
    let source_package = open_source_package(source_package_path, cache_root)?;
    ProjectSession::preview_source_update(repository_root, &source_package)
        .map(|report| SourceUpdateReportDto::from(&report))
        .map_err(CommandError::from)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Lists translation units of the active project that are preserved without
/// a current source occurrence.
///
/// # Errors
///
/// Returns a typed command error when no project is open or the desktop state
/// lock cannot be read.
pub fn list_detached_units(state: State<'_, DesktopState>) -> CommandResult<Vec<DetachedUnitDto>> {
    list_detached_units_with_state(&state)
}

pub(crate) fn list_detached_units_with_state(
    state: &DesktopState,
) -> CommandResult<Vec<DetachedUnitDto>> {
    let project = state.lock_project()?;
    let project = project.as_ref().ok_or_else(CommandError::no_project)?;
    Ok(project
        .detached_units()
        .filter_map(DetachedUnitDto::from_unit)
        .collect())
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Initializes a new project and makes it the active desktop project.
///
/// # Errors
///
/// Returns a typed command error when source verification, workspace
/// initialization, or state locking fails.
pub async fn initialize_project(
    app: tauri::AppHandle,
    repository_root: String,
    source_package_path: String,
    target_language: String,
) -> CommandResult<ProjectOpenResultDto> {
    let cache_root = app
        .aeria_cache_dir()
        .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
    let registry_path = app_registry_path(&app);
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let project = initialize_project_with_state(
            &state,
            repository_root,
            source_package_path,
            cache_root,
            target_language,
        )?;
        Ok(remember_project(&state, project, registry_path))
    })
    .await
}

pub(crate) fn initialize_project_with_state(
    state: &DesktopState,
    repository_root: String,
    source_package_path: String,
    cache_root: PathBuf,
    target_language: String,
) -> CommandResult<ProjectSummaryDto> {
    let replacement = ProjectSession::initialize(
        repository_root,
        source_package_path,
        cache_root,
        target_language,
    )
    .map_err(CommandError::from)?;
    replace_project(state, replacement)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Lists local recent projects using only registry data and filesystem presence.
///
/// # Errors
///
/// Returns a typed registry error when app-data cannot be resolved or the
/// registry cannot be loaded.
pub async fn list_recent_projects(app: tauri::AppHandle) -> CommandResult<Vec<RecentProjectDto>> {
    let path = app_registry_path(&app)
        .map_err(|message| CommandError::new("projectRegistryRead", message))?;
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let _lock = state.lock_registry()?;
        ProjectRegistry::new(path)
            .load()
            .map(|projects| {
                projects
                    .into_iter()
                    .map(RecentProjectDto::from_registry_entry)
                    .collect()
            })
            .map_err(|error| CommandError::registry_read(&error))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Opens one exact local recent-project entry and refreshes its cached metadata.
///
/// # Errors
///
/// Returns a typed error when the registry entry is missing, either remembered
/// path is unavailable, project validation fails, or the source package ID no
/// longer matches the remembered association.
pub async fn open_recent_project(
    app: tauri::AppHandle,
    project_id: String,
    accept_source_update: Option<bool>,
) -> CommandResult<ProjectOpenResultDto> {
    let cache_root = app
        .aeria_cache_dir()
        .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
    let registry_path = app_registry_path(&app)
        .map_err(|message| CommandError::new("projectRegistryRead", message))?;
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        open_recent_project_from_registry(
            &state,
            &project_id,
            cache_root,
            registry_path,
            accept_source_update.unwrap_or(false),
        )
    })
    .await
}

fn open_recent_project_from_registry(
    state: &DesktopState,
    project_id: &str,
    cache_root: PathBuf,
    registry_path: PathBuf,
    accept_source_update: bool,
) -> CommandResult<ProjectOpenResultDto> {
    let entry = {
        let _lock = state.lock_registry()?;
        let projects = ProjectRegistry::new(&registry_path)
            .load()
            .map_err(|error| CommandError::registry_read(&error))?;
        projects
            .into_iter()
            .find(|project| project.id == project_id)
            .ok_or_else(|| {
                CommandError::recent_project(
                    "recentProjectNotFound",
                    format!("recent project {project_id:?} was not found"),
                )
            })?
    };

    open_recent_project_with_entry_and_update(
        state,
        &entry,
        cache_root,
        registry_path,
        accept_source_update,
    )
}

#[cfg(test)]
fn open_recent_project_with_entry(
    state: &DesktopState,
    entry: &RegistryEntry,
    cache_root: PathBuf,
    registry_path: PathBuf,
) -> CommandResult<ProjectOpenResultDto> {
    open_recent_project_with_entry_and_update(state, entry, cache_root, registry_path, false)
}

fn open_recent_project_with_entry_and_update(
    state: &DesktopState,
    entry: &RegistryEntry,
    cache_root: PathBuf,
    registry_path: PathBuf,
    accept_source_update: bool,
) -> CommandResult<ProjectOpenResultDto> {
    let repository_root = PathBuf::from(&entry.repository_root);
    if !repository_root.is_dir() {
        return Err(CommandError::recent_project(
            "recentProjectRepositoryMissing",
            format!(
                "recent project repository is missing: {}",
                entry.repository_root
            ),
        ));
    }
    let source_package_path = PathBuf::from(&entry.source_package_path);
    if !source_package_path.is_file() {
        return Err(CommandError::recent_project(
            "recentProjectSourceMissing",
            format!(
                "recent project source package is missing: {}",
                entry.source_package_path
            ),
        ));
    }

    let source_package =
        SourcePackage::open(&source_package_path, cache_root).map_err(|source| {
            CommandError::from(ProjectSessionError::Source {
                path: source_package_path.clone(),
                source,
            })
        })?;
    if source_package.package_id() != entry.source_package_id {
        return Err(CommandError::recent_project(
            "recentProjectSourceMismatch",
            format!(
                "remembered source packageId {} does not match the package at the remembered path ({})",
                entry.source_package_id,
                source_package.package_id()
            ),
        ));
    }

    let (project, source_update) =
        open_package_with_state(state, repository_root, source_package, accept_source_update)?;
    Ok(remember_project_with_update(
        state,
        project,
        Ok(registry_path),
        source_update,
    ))
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Removes one entry from Recent projects without touching project files.
///
/// # Errors
///
/// Returns a typed registry error when app-data cannot be resolved, the entry
/// is absent, or the updated registry cannot be published.
pub async fn forget_recent_project(app: tauri::AppHandle, project_id: String) -> CommandResult<()> {
    let path = app_registry_path(&app)
        .map_err(|message| CommandError::new("projectRegistryWrite", message))?;
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        forget_recent_project_from_registry(&state, &path, &project_id)
    })
    .await
}

fn forget_recent_project_from_registry(
    state: &DesktopState,
    path: &Path,
    project_id: &str,
) -> CommandResult<()> {
    let _lock = state.lock_registry()?;
    ProjectRegistry::new(path)
        .remove(project_id)
        .map_err(|error| CommandError::registry_write(&error))
}

/// Reserves an opaque job identity for source-package generation.
///
/// # Errors
///
/// Returns a typed error when another Atlas job is active or desktop state is
/// unavailable.
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
pub fn start_source_package(state: State<'_, DesktopState>) -> CommandResult<SourcePackageJobDto> {
    let started = state.start_atlas_job()?;
    Ok(SourcePackageJobDto { job_id: started.id })
}

/// Generates a source package from the game installation chosen in Settings
/// (or detected) and initializes the project from the validated package.
///
/// # Errors
///
/// Returns a typed error when the job is no longer active, no game
/// installation is available, Atlas cannot run,
/// package publication or validation fails, or workspace initialization fails.
#[tauri::command(rename_all = "camelCase")]
pub async fn initialize_project_from_game(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    job_id: String,
    repository_root: String,
    source_language: String,
    target_language: String,
) -> CommandResult<ProjectOpenResultDto> {
    let token = state.atlas_job_token(&job_id)?;
    // Created before extraction so an unusable path fails at once.
    let created = match create_project_directory(Path::new(&repository_root)) {
        Ok(created) => created,
        Err(error) => {
            state.finish_atlas_job(&job_id)?;
            return Err(error);
        }
    };
    let created_root = created.then(|| PathBuf::from(&repository_root));
    let worker_job_id = job_id.clone();
    let worker_token = token.clone();
    let worker_app = app.clone();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        initialize_project_from_game_inner(
            &worker_app,
            &worker_job_id,
            &worker_token,
            repository_root,
            source_language,
            target_language,
        )
    });
    let result = match worker.await {
        Ok(result) => result,
        Err(error) => Err(CommandError::internal_state(format!(
            "Atlas creation worker failed: {error}"
        ))),
    };
    let outcome = match result {
        Ok(prepared) => {
            let result = state.with_atlas_publication(&job_id, &token, |_| {
                require_not_cancelled(&token)?;
                let replacement = ProjectSession::initialize_from_source_package(
                    prepared.repository_root,
                    prepared.source_package,
                    prepared.target_language,
                )
                .map_err(CommandError::from)?;
                require_not_cancelled(&token)?;
                replace_project(&state, replacement)
            });
            state.finish_atlas_job(&job_id)?;
            let registry_path = app_registry_path(&app);
            result.map(|project| remember_project(&state, project, registry_path))
        }
        Err(error) => {
            state.finish_atlas_job(&job_id)?;
            Err(error)
        }
    };
    if outcome.is_err()
        && let Some(root) = created_root
    {
        // Only an empty folder is removed; anything written into it stays.
        let _ = fs::remove_dir(root);
    }
    outcome
}

/// Creates a new project's folder, and missing parents, when it does not
/// exist yet. Returns whether the folder was created.
pub(crate) fn create_project_directory(root: &Path) -> CommandResult<bool> {
    if root.as_os_str().is_empty() {
        return Err(CommandError::new(
            "invalidInput",
            "the project folder is empty",
        ));
    }
    if root.is_dir() {
        return Ok(false);
    }
    fs::create_dir_all(root).map_err(|error| {
        CommandError::new(
            "projectFolder",
            format!(
                "could not create the project folder {}: {error}",
                root.display()
            ),
        )
    })?;
    Ok(true)
}

/// Generates a source package from the game installation chosen in Settings
/// (or detected) for an existing project and opens the project with the deterministic source
/// update applied. The source language is read from the project.
///
/// # Errors
///
/// Returns a typed error when the job is no longer active, the project cannot
/// be read, no game installation is available, Atlas cannot run, package publication or validation fails, or the
/// source update fails.
#[tauri::command(rename_all = "camelCase")]
pub async fn update_project_from_game(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    job_id: String,
    repository_root: String,
) -> CommandResult<ProjectOpenResultDto> {
    let token = state.atlas_job_token(&job_id)?;
    let worker_job_id = job_id.clone();
    let worker_token = token.clone();
    let worker_app = app.clone();
    let worker_root = repository_root.clone();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        let source_language = WorkspaceStore::new(&worker_root)
            .read_metadata()
            .map_err(CommandError::from)?
            .source_language()
            .to_owned();
        let game_path = resolve_game_path(&worker_app)?;
        generate_source_package(
            &worker_app,
            &worker_job_id,
            &worker_token,
            game_path,
            source_language,
        )
    });
    let result = match worker.await {
        Ok(result) => result,
        Err(error) => Err(CommandError::internal_state(format!(
            "Atlas update worker failed: {error}"
        ))),
    };
    match result {
        Ok(source_package) => {
            let result = state.with_atlas_publication(&job_id, &token, |_| {
                require_not_cancelled(&token)?;
                open_package_with_state(&state, repository_root, source_package, true)
            });
            state.finish_atlas_job(&job_id)?;
            let registry_path = app_registry_path(&app);
            result.map(|(project, source_update)| {
                remember_project_with_update(&state, project, registry_path, source_update)
            })
        }
        Err(error) => {
            state.finish_atlas_job(&job_id)?;
            Err(error)
        }
    }
}

/// Opens an existing project without a user-chosen source package.
///
/// The project's source language and content ID are read from its workspace
/// manifest. A matching package already published in Aeria's source-package
/// store is used directly; otherwise Harmonia Atlas builds one from the game
/// installation chosen in Settings (or detected), reporting progress under `job_id`. When the package needs a
/// source update, nothing is written and the plan is returned for
/// confirmation.
///
/// # Errors
///
/// Returns a typed error when the workspace cannot be read, no local package
/// matches and no game installation is available, Atlas or package
/// validation fails, or the project cannot be opened.
#[tauri::command(rename_all = "camelCase")]
pub async fn open_project_from_game(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    job_id: String,
    repository_root: String,
) -> CommandResult<GameOpenResultDto> {
    let token = state.atlas_job_token(&job_id)?;
    let worker_job_id = job_id.clone();
    let worker_token = token.clone();
    let worker_app = app.clone();
    let worker_root = repository_root.clone();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        resolve_project_source_package(
            &worker_app,
            &worker_job_id,
            &worker_token,
            Path::new(&worker_root),
        )
    });
    let result = match worker.await {
        Ok(result) => result,
        Err(error) => Err(CommandError::internal_state(format!(
            "source package worker failed: {error}"
        ))),
    };
    let outcome = result.and_then(|(source_package, cache_root)| {
        state.with_atlas_publication(&job_id, &token, |_| {
            require_not_cancelled(&token)?;
            open_or_plan_source_update(&state, &repository_root, source_package, cache_root)
        })
    });
    state.finish_atlas_job(&job_id)?;
    match outcome? {
        GameOpenOutcome::Opened(project) => Ok(GameOpenResultDto::Opened {
            result: Box::new(remember_project(&state, project, app_registry_path(&app))),
        }),
        GameOpenOutcome::SourceUpdateRequired {
            source_package_path,
            report,
        } => Ok(GameOpenResultDto::SourceUpdateRequired {
            source_package_path,
            report,
        }),
    }
}

enum GameOpenOutcome {
    Opened(ProjectSummaryDto),
    SourceUpdateRequired {
        source_package_path: String,
        report: SourceUpdateReportDto,
    },
}

/// Finds or builds the source package for an existing project and returns
/// it with the cache root it was opened with.
fn resolve_project_source_package(
    app: &tauri::AppHandle,
    job_id: &str,
    cancellation: &CancellationToken,
    repository_root: &Path,
) -> CommandResult<(SourcePackage, PathBuf)> {
    let metadata = WorkspaceStore::new(repository_root)
        .read_metadata()
        .map_err(CommandError::from)?;
    let packages_root = app
        .aeria_data_dir()
        .map_err(|error| CommandError::new("atlasStorage", error.to_string()))?
        .join(SOURCE_PACKAGES_DIRECTORY);
    let cache_root = app
        .aeria_cache_dir()
        .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
    if let Some(source_package) = find_local_source_package(
        &packages_root,
        &cache_root,
        metadata.source_language(),
        metadata.source_content_id(),
    ) {
        return Ok((source_package, cache_root));
    }
    let source_package = generate_source_package(
        app,
        job_id,
        cancellation,
        resolve_game_path(app)?,
        metadata.source_language().to_owned(),
    )?;
    Ok((source_package, cache_root))
}

/// Returns a verified package from Aeria's source-package store whose source
/// language and content ID match, preferring the newest game version.
///
/// Manifests are previewed first so only a matching package is verified and
/// materialized. Store files that cannot be read or verified are skipped:
/// the store is a local cache, and the caller then builds a fresh, fully
/// validated package instead.
pub(crate) fn find_local_source_package(
    packages_root: &Path,
    cache_root: &Path,
    source_language: &str,
    content_id: &str,
) -> Option<SourcePackage> {
    let mut candidates = fs::read_dir(packages_root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "hsp") && path.is_file()
        })
        .filter_map(|path| {
            let manifest = aeria_hsp::read_manifest(&path).ok()?;
            (manifest.source.language == source_language
                && manifest.source.content_id == content_id)
                .then_some((manifest.game_version, manifest.package_id, path))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    candidates.into_iter().find_map(|(_, package_id, path)| {
        SourcePackage::open(&path, cache_root)
            .ok()
            .filter(|package| {
                package.package_id() == package_id
                    && package.source_language() == source_language
                    && package.source_content_id() == content_id
            })
    })
}

/// Opens the project when the package is current for it; otherwise plans
/// the required source update without writing anything.
fn open_or_plan_source_update(
    state: &DesktopState,
    repository_root: &str,
    source_package: SourcePackage,
    cache_root: PathBuf,
) -> CommandResult<GameOpenOutcome> {
    let package_path = source_package.package_path().to_owned();
    match ProjectSession::open_from_source_package(repository_root, source_package) {
        Ok(session) => Ok(GameOpenOutcome::Opened(replace_project(state, session)?)),
        Err(ProjectSessionError::SourceUpdateRequired { .. }) => {
            let report = preview_source_update_with_paths(
                Path::new(repository_root),
                &package_path,
                cache_root,
            )?;
            Ok(GameOpenOutcome::SourceUpdateRequired {
                source_package_path: package_path.to_string_lossy().into_owned(),
                report,
            })
        }
        Err(error) => Err(CommandError::from(error)),
    }
}

/// Folder inside the user's Documents directory that receives new and cloned
/// projects when no other folder is chosen.
const DEFAULT_PROJECTS_FOLDER: &str = "Aeria";

pub(crate) fn default_projects_directory(app: &tauri::AppHandle) -> CommandResult<PathBuf> {
    app.path()
        .document_dir()
        .map(|documents| documents.join(DEFAULT_PROJECTS_FOLDER))
        .map_err(|error| {
            CommandError::new(
                "projectsPath",
                format!("could not resolve the Documents folder: {error}"),
            )
        })
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Returns the folder that receives new and cloned projects when no other
/// folder is chosen.
///
/// # Errors
///
/// Returns a typed command error when the Documents folder cannot be resolved.
pub fn default_projects_directory_path(app: tauri::AppHandle) -> CommandResult<String> {
    default_projects_directory(&app).map(|path| path.to_string_lossy().into_owned())
}

/// Cancels the active source-package generation job with the supplied ID.
///
/// # Errors
///
/// Returns a typed error when the job ID is not active or desktop state is
/// unavailable.
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
pub fn cancel_source_package(state: State<'_, DesktopState>, job_id: String) -> CommandResult<()> {
    state.cancel_atlas_job(&job_id)
}

fn initialize_project_from_game_inner(
    app: &tauri::AppHandle,
    job_id: &str,
    cancellation: &CancellationToken,
    repository_root: String,
    source_language: String,
    target_language: String,
) -> Result<PreparedAtlasProject, CommandError> {
    let game_path = resolve_game_path(app)?;
    let source_package =
        generate_source_package(app, job_id, cancellation, game_path, source_language)?;
    Ok(PreparedAtlasProject {
        repository_root,
        source_package,
        target_language,
    })
}

/// Runs Atlas for one installed game and returns the validated, published
/// immutable source package. A package in the store built from the same Atlas
/// executable, source language, and game version files is reused instead.
fn generate_source_package(
    app: &tauri::AppHandle,
    job_id: &str,
    cancellation: &CancellationToken,
    game_path: String,
    source_language: String,
) -> Result<SourcePackage, CommandError> {
    let executable_path = resolve_atlas_executable(app)?;
    let app_data = app
        .aeria_data_dir()
        .map_err(|error| CommandError::new("atlasStorage", error.to_string()))?;
    let cache_root = app
        .aeria_cache_dir()
        .map_err(|error| CommandError::new("atlasStorage", error.to_string()))?;
    let packages_root = app_data.join(SOURCE_PACKAGES_DIRECTORY);
    let staging_root = packages_root.join(STAGING_DIRECTORY);
    fs::create_dir_all(&staging_root)
        .map_err(|error| storage_error("create source-package staging directory", &error))?;
    let staging_path = staging_root.join(STAGING_FILE);

    // Without a complete fingerprint nothing can be reused or recorded.
    let build_record = CurrentInputs::new(Some(&executable_path), Some(Path::new(&game_path)))
        .record(&source_language);
    if let Some(record) = &build_record
        && let Some(existing) = find_built_package(&packages_root, &cache_root, record)
    {
        return Ok(existing);
    }

    let request = AtlasPackageRequest {
        executable_path,
        game_path: PathBuf::from(game_path),
        source_language,
        output_path: staging_path.clone(),
    };
    let app_handle = app.clone();
    let event_job_id = job_id.to_owned();
    let result = AtlasPackageRunner::default()
        .run(
            &request,
            |event| {
                let payload = SourcePackageEventPayload {
                    job_id: event_job_id.clone(),
                    event: event.clone(),
                };
                if let Err(error) = app_handle.emit("source-package-event", payload) {
                    eprintln!(
                        "failed to emit source-package-event for Atlas job {event_job_id}: {error}"
                    );
                }
            },
            cancellation,
        )
        .map_err(|error| atlas_runner_error(error, cancellation, &staging_path))?;

    require_not_cancelled_with_staging(cancellation, &staging_path)?;
    let source_package = validate_and_publish_package(
        &result,
        &staging_path,
        &packages_root,
        &cache_root,
        cancellation,
    )?;
    if let Some(record) = &build_record
        && let Err(error) = write_build_record(source_package.package_path(), record)
    {
        // The package is valid; without its record it is only not reused.
        eprintln!(
            "failed to record how source package {} was built: {error}",
            source_package.package_id()
        );
    }
    require_not_cancelled(cancellation)?;
    Ok(source_package)
}

struct PreparedAtlasProject {
    repository_root: String,
    source_package: SourcePackage,
    target_language: String,
}

pub(crate) fn resolve_atlas_executable(app: &tauri::AppHandle) -> CommandResult<PathBuf> {
    if let Some(path) = atlas_override_path(env::var_os("AERIA_ATLAS_PATH"))? {
        return Ok(path);
    }

    let executable_path = env::current_exe()
        .map_err(|error| CommandError::new("atlasNotFound", error.to_string()))?;
    let resource_dir = app.path().resource_dir().ok();
    let candidates = bundled_atlas_candidates(&executable_path, resource_dir.as_deref());
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            CommandError::new(
                "atlasNotFound",
                format!(
                    "the bundled Harmonia Atlas executable was not found beside {} or in the resource directory",
                    executable_path.display()
                ),
            )
        })
}

fn atlas_override_path(value: Option<std::ffi::OsString>) -> CommandResult<Option<PathBuf>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if path.is_file() {
        Ok(Some(path))
    } else {
        Err(CommandError::new(
            "atlasNotFound",
            format!(
                "AERIA_ATLAS_PATH does not point to a file: {}",
                path.display()
            ),
        ))
    }
}

fn bundled_atlas_candidates(executable_path: &Path, resource_dir: Option<&Path>) -> Vec<PathBuf> {
    // Tauri stages harmonia-atlas-<target-triple>[.exe] at build time, but
    // packages the runtime sidecar as harmonia-atlas[.exe] beside Aeria.
    let mut candidates = Vec::new();
    if let Some(executable_dir) = executable_path.parent() {
        candidates.push(executable_dir.join(atlas_runtime_filename()));
    }

    if let Some(resource_dir) = resource_dir {
        candidates.push(resource_dir.join("binaries").join(atlas_runtime_filename()));
        candidates.push(resource_dir.join(atlas_runtime_filename()));
    }

    candidates
}

fn atlas_runtime_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "harmonia-atlas.exe"
    } else {
        "harmonia-atlas"
    }
}

fn validate_and_publish_package(
    result: &AtlasPackageResult,
    staging_path: &Path,
    packages_root: &Path,
    cache_root: &Path,
    cancellation: &CancellationToken,
) -> CommandResult<SourcePackage> {
    validate_and_publish_package_with_hook(
        result,
        staging_path,
        packages_root,
        cache_root,
        cancellation,
        || {},
    )
}

fn validate_and_publish_package_with_hook<F>(
    result: &AtlasPackageResult,
    staging_path: &Path,
    packages_root: &Path,
    cache_root: &Path,
    cancellation: &CancellationToken,
    on_validated: F,
) -> CommandResult<SourcePackage>
where
    F: FnOnce(),
{
    let package_hex = result
        .package_id
        .strip_prefix("sha256:")
        .ok_or_else(|| CommandError::new("atlasPackage", "Atlas packageId is not canonical"))?;
    let final_path = packages_root.join(format!("{package_hex}.hsp"));

    if final_path.exists()
        && let Ok(existing) = SourcePackage::open(&final_path, cache_root)
        && existing.package_id() == result.package_id
    {
        require_not_cancelled_with_staging(cancellation, staging_path)?;
        remove_staging_package(staging_path, "remove reused staging package")?;
        return Ok(existing);
    }

    require_not_cancelled_with_staging(cancellation, staging_path)?;
    let validated = match SourcePackage::open(staging_path, cache_root) {
        Ok(source_package) => source_package,
        Err(error) => {
            remove_staging_package(staging_path, "remove invalid staging package")?;
            return Err(CommandError::new("atlasPackage", error.to_string()));
        }
    };
    if validated.package_id() != result.package_id {
        let package_id = validated.package_id().to_owned();
        remove_staging_package(staging_path, "remove mismatched staging package")?;
        return Err(CommandError::new(
            "atlasPackage",
            format!(
                "validated HSP packageId {package_id} does not match Atlas packageId {}",
                result.package_id
            ),
        ));
    }
    on_validated();
    require_not_cancelled_with_staging(cancellation, staging_path)?;

    if final_path.exists() {
        let backup_path = final_path.with_extension("hsp.invalid");
        if backup_path.exists() {
            fs::remove_file(&backup_path)
                .map_err(|error| storage_error("remove stale invalid package backup", &error))?;
        }
        fs::rename(&final_path, &backup_path)
            .map_err(|error| storage_error("stage invalid package replacement", &error))?;
        if cancellation.is_cancelled() {
            let restore = fs::rename(&backup_path, &final_path);
            let _ = remove_staging_package(staging_path, "remove cancelled staging package");
            if let Err(error) = restore {
                return Err(storage_error(
                    "restore invalid package after cancellation",
                    &error,
                ));
            }
            return Err(cancelled_error());
        }
        match fs::rename(staging_path, &final_path) {
            Ok(()) => {
                fs::remove_file(&backup_path)
                    .map_err(|error| storage_error("remove invalid package backup", &error))?;
                Ok(validated.relocate_package_path(final_path, cache_root))
            }
            Err(error) => {
                let _ = fs::rename(&backup_path, &final_path);
                Err(storage_error("publish replacement source package", &error))
            }
        }
    } else {
        fs::rename(staging_path, &final_path)
            .map_err(|error| storage_error("atomically publish source package", &error))?;
        Ok(validated.relocate_package_path(final_path, cache_root))
    }
}

fn remove_staging_package(path: &Path, operation: &str) -> CommandResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(storage_error(operation, &error)),
    }
}

fn cancelled_error() -> CommandError {
    CommandError::new("atlasCancelled", "Atlas project creation was cancelled")
}

fn atlas_runner_error(
    error: AtlasError,
    cancellation: &CancellationToken,
    staging_path: &Path,
) -> CommandError {
    if cancellation.is_cancelled() {
        return match remove_staging_package(staging_path, "remove cancelled staging package") {
            Ok(()) => cancelled_error(),
            Err(cleanup_error) => CommandError::new(
                "atlasCancelled",
                format!(
                    "Atlas project creation was cancelled: {}",
                    cleanup_error.message
                ),
            ),
        };
    }
    CommandError::from(error)
}

fn require_not_cancelled(cancellation: &CancellationToken) -> CommandResult<()> {
    if cancellation.is_cancelled() {
        Err(cancelled_error())
    } else {
        Ok(())
    }
}

fn require_not_cancelled_with_staging(
    cancellation: &CancellationToken,
    staging_path: &Path,
) -> CommandResult<()> {
    if cancellation.is_cancelled() {
        remove_staging_package(staging_path, "remove cancelled staging package").map_err(
            |error| {
                CommandError::new(
                    "atlasCancelled",
                    format!("Atlas project creation was cancelled: {}", error.message),
                )
            },
        )?;
        Err(cancelled_error())
    } else {
        Ok(())
    }
}

fn storage_error(operation: &str, error: &std::io::Error) -> CommandError {
    CommandError::new("atlasStorage", format!("failed to {operation}: {error}"))
}

fn replace_project(
    state: &DesktopState,
    replacement: ProjectSession,
) -> CommandResult<ProjectSummaryDto> {
    let summary = ProjectSummaryDto::from_session(&replacement);
    let mut project = state.lock_project()?;
    *project = Some(replacement);
    Ok(summary)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Returns a snapshot of the active project, if one is open.
///
/// # Errors
///
/// Returns a typed command error when the desktop state lock cannot be read.
pub fn current_project(state: State<'_, DesktopState>) -> CommandResult<Option<ProjectSummaryDto>> {
    current_project_with_state(&state)
}

pub(crate) fn current_project_with_state(
    state: &DesktopState,
) -> CommandResult<Option<ProjectSummaryDto>> {
    let project = state.lock_project()?;
    Ok(project.as_ref().map(ProjectSummaryDto::from_session))
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Returns per-sheet Workspace coverage for the active project.
///
/// # Errors
///
/// Returns a typed command error when no project is open or the desktop state
/// lock cannot be read.
pub fn translation_progress(
    state: State<'_, DesktopState>,
) -> CommandResult<Vec<SheetProgressDto>> {
    translation_progress_with_state(&state)
}

pub(crate) fn translation_progress_with_state(
    state: &DesktopState,
) -> CommandResult<Vec<SheetProgressDto>> {
    let project = state.lock_project()?;
    let project = project.as_ref().ok_or_else(CommandError::no_project)?;
    Ok(project
        .translation_progress()
        .into_iter()
        .map(SheetProgressDto::from)
        .collect())
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Closes the active project. Closing an already closed desktop is successful.
///
/// # Errors
///
/// Returns a typed command error when the desktop state lock cannot be acquired.
pub fn close_project(state: State<'_, DesktopState>) -> CommandResult<()> {
    close_project_with_state(&state)
}

pub(crate) fn close_project_with_state(state: &DesktopState) -> CommandResult<()> {
    let mut project = state.lock_project()?;
    *project = None;
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Reads one bounded page of logical source rows and workspace overlays.
///
/// # Errors
///
/// Returns a typed command error when no project is open, the page request is
/// invalid, the source cannot be read, or source integrity fails.
pub async fn page_translation_rows(
    app: tauri::AppHandle,
    sheet_name: String,
    after: Option<TranslationRowCursorDto>,
    limit: u32,
) -> CommandResult<TranslationRowPageDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        page_translation_rows_with_state(&state, &sheet_name, after, limit)
    })
    .await
}

pub(crate) fn page_translation_rows_with_state(
    state: &DesktopState,
    sheet_name: &str,
    after: Option<TranslationRowCursorDto>,
    limit: u32,
) -> CommandResult<TranslationRowPageDto> {
    let after = after.map(TranslationRowCursor::from);
    let project = state.lock_project()?;
    let project = project.as_ref().ok_or_else(CommandError::no_project)?;
    project
        .page_translation_rows(sheet_name, after.as_ref(), limit)
        .map(Into::into)
        .map_err(CommandError::from)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Creates or updates the target for one source occurrence.
///
/// # Errors
///
/// Returns a typed command error when no project is open or the backend rejects
/// the source, target, or persistence operation.
pub async fn set_translation_target(
    app: tauri::AppHandle,
    source_binding: SourceBindingDto,
    target_macro: String,
) -> CommandResult<TranslationOverlayDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        set_translation_target_with_state(&state, source_binding, &target_macro)
    })
    .await
}

pub(crate) fn set_translation_target_with_state(
    state: &DesktopState,
    source_binding: SourceBindingDto,
    target_macro: &str,
) -> CommandResult<TranslationOverlayDto> {
    let source_binding = SourceBinding::from(source_binding);
    let mut project = state.lock_project()?;
    let project = project.as_mut().ok_or_else(CommandError::no_project)?;
    project
        .set_target(&source_binding, target_macro)
        .map_err(CommandError::from)
        .and_then(|id| translation_overlay(project, id))
}

fn translation_overlay(
    project: &ProjectSession,
    translation_unit_id: TranslationUnitId,
) -> CommandResult<TranslationOverlayDto> {
    let unit = project
        .workspace()
        .unit(translation_unit_id)
        .ok_or_else(|| {
            CommandError::new(
                "translationWorkspace",
                format!("translation unit was not found: {translation_unit_id}"),
            )
        })?;
    Ok(TranslationOverlayDto {
        translation_unit_id: unit.id().to_string(),
        target_macro: unit.target_macro().to_owned(),
        review_state: unit.review_state().into(),
        translator_note: unit.translator_note().map(str::to_owned),
    })
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Replaces or clears the note for one translation unit.
///
/// # Errors
///
/// Returns a typed command error when the ID is invalid, no project is open,
/// or the backend rejects the operation.
pub async fn set_translation_note(
    app: tauri::AppHandle,
    translation_unit_id: String,
    note: Option<String>,
) -> CommandResult<TranslationOverlayDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        set_translation_note_with_state(&state, &translation_unit_id, note)
    })
    .await
}

pub(crate) fn set_translation_note_with_state(
    state: &DesktopState,
    translation_unit_id: &str,
    note: Option<String>,
) -> CommandResult<TranslationOverlayDto> {
    let mut project = state.lock_project()?;
    let project = project.as_mut().ok_or_else(CommandError::no_project)?;
    let translation_unit_id = parse_translation_unit_id(translation_unit_id)?;
    project
        .set_note(translation_unit_id, note)
        .map_err(CommandError::from)
        .and_then(|()| translation_overlay(project, translation_unit_id))
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Sets the explicit review state for one translation unit.
///
/// # Errors
///
/// Returns a typed command error when the ID is invalid, no project is open,
/// or the backend rejects the operation.
pub async fn set_translation_review_state(
    app: tauri::AppHandle,
    translation_unit_id: String,
    review_state: ReviewStateDto,
) -> CommandResult<TranslationOverlayDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        set_translation_review_state_with_state(&state, &translation_unit_id, review_state)
    })
    .await
}

pub(crate) fn set_translation_review_state_with_state(
    state: &DesktopState,
    translation_unit_id: &str,
    review_state: ReviewStateDto,
) -> CommandResult<TranslationOverlayDto> {
    let mut project = state.lock_project()?;
    let project = project.as_mut().ok_or_else(CommandError::no_project)?;
    let translation_unit_id = parse_translation_unit_id(translation_unit_id)?;
    let review_state: ReviewState = review_state.into();
    project
        .set_review_state(translation_unit_id, review_state)
        .map_err(CommandError::from)
        .and_then(|()| translation_overlay(project, translation_unit_id))
}

pub(crate) fn parse_translation_unit_id(value: &str) -> CommandResult<TranslationUnitId> {
    TranslationUnitId::from_str(value).map_err(CommandError::from)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::fs;
    use std::io::{Read, Write as _};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::dto::{ProjectSheetDto, ReviewStateDto, SourceBindingDto};
    use aeria_hsp::{
        SourceGuidance, compute_guidance_bundle_id, compute_package_id, compute_source_evidence_id,
    };
    use rusqlite::{Connection, params};
    use sha2::{Digest, Sha256};
    use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

    struct TestRepository {
        path: PathBuf,
    }

    impl TestRepository {
        fn new(label: &str) -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "aeria-desktop-boundary-{label}-{}-{timestamp}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("test repository");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestRepository {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../crates/aeria-hsp/tests/fixtures/synthetic.hsp")
    }

    fn registry_path(repository: &TestRepository) -> PathBuf {
        repository.path().join("app-data").join(REGISTRY_FILE_NAME)
    }

    fn seed_recent_project(label: &str) -> (TestRepository, DesktopState, PathBuf, RegistryEntry) {
        let repository = TestRepository::new(label);
        let source = repository.path().join("source.hsp");
        fs::copy(fixture_path(), &source).expect("source package");
        let cache_root = repository.path().join("cache");
        let state = DesktopState::new();
        let summary = initialize_project_with_state(
            &state,
            repository.path().to_string_lossy().into_owned(),
            source.to_string_lossy().into_owned(),
            cache_root,
            "fr".to_owned(),
        )
        .expect("initialize project");
        let path = registry_path(&repository);
        let result = remember_project(&state, summary, Ok(path.clone()));
        assert!(result.warning.is_none());
        close_project_with_state(&state).expect("close active project");
        let entry = aeria_projects::ProjectRegistry::new(&path)
            .load()
            .expect("load recent project")
            .into_iter()
            .next()
            .expect("seeded entry");
        (repository, state, path, entry)
    }

    fn write_valid_incompatible_package(path: &Path, work_directory: &Path) {
        let mut entries = read_archive_entries(path);
        let manifest_entry = entries
            .iter()
            .find(|(entry_path, _)| entry_path == "manifest.json")
            .expect("manifest entry")
            .1
            .clone();
        let mut manifest: aeria_hsp::HspManifest =
            serde_json::from_slice(&manifest_entry).expect("manifest JSON");

        let source_entry = entries
            .iter()
            .find(|(entry_path, _)| entry_path == "source/source.hxs")
            .expect("source entry")
            .1
            .clone();
        let hxs_path = work_directory.join("replacement.hxs");
        fs::write(&hxs_path, source_entry).expect("replacement HXS");
        let connection = Connection::open(&hxs_path).expect("replacement HXS database");
        let (content_id, source_language): (String, String) = connection
            .query_row(
                "SELECT content_id, language FROM hxs_meta WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("HXS metadata");
        let game_version = "replacement-game";
        let snapshot_id = hxs_snapshot_id(game_version, &source_language, &content_id);
        connection
            .execute(
                "UPDATE hxs_meta SET game_version = ?1, snapshot_id = ?2 WHERE id = 1",
                params![game_version, &snapshot_id],
            )
            .expect("update HXS metadata");
        drop(connection);

        let source_bytes = fs::read(&hxs_path).expect("replacement HXS bytes");
        replace_archive_entry(&mut entries, "source/source.hxs", source_bytes.clone());
        manifest.game_version = game_version.to_owned();
        manifest.source.snapshot_id = snapshot_id.clone();
        update_component(&mut manifest, "sourceHxs", &source_bytes);

        let guidance_entry = entries
            .iter()
            .find(|(entry_path, _)| entry_path == "guidance/source-guidance.json")
            .expect("guidance entry")
            .1
            .clone();
        let mut guidance: SourceGuidance =
            serde_json::from_slice(&guidance_entry).expect("guidance JSON");
        guidance.game_version = game_version.to_owned();
        guidance.source.snapshot_id = snapshot_id.clone();
        let replacement_snapshot =
            aeria_hxs::HxsSnapshot::open(&hxs_path).expect("replacement HXS should validate");
        let source_evidence = guidance
            .evidence_inputs
            .iter_mut()
            .find(|input| input.language == source_language)
            .expect("source evidence input");
        source_evidence.evidence_id =
            compute_source_evidence_id(&replacement_snapshot).expect("source evidence hash");
        guidance.bundle_id = compute_guidance_bundle_id(&guidance).expect("guidance hash");
        let mut guidance_bytes = serde_json::to_vec(&guidance).expect("guidance JSON");
        guidance_bytes.push(b'\n');
        replace_archive_entry(
            &mut entries,
            "guidance/source-guidance.json",
            guidance_bytes.clone(),
        );
        update_component(&mut manifest, "sourceGuidance", &guidance_bytes);
        manifest.package_id = compute_package_id(&manifest).expect("package hash");
        let mut manifest_bytes = serde_json::to_vec(&manifest).expect("manifest JSON");
        manifest_bytes.push(b'\n');
        replace_archive_entry(&mut entries, "manifest.json", manifest_bytes);

        let file = fs::File::create(path).expect("replacement package");
        let mut archive = ZipWriter::new(file);
        for (entry_path, bytes) in entries {
            archive
                .start_file(entry_path, SimpleFileOptions::default())
                .expect("replacement archive entry");
            archive
                .write_all(&bytes)
                .expect("replacement archive bytes");
        }
        archive.finish().expect("replacement archive");
    }

    fn read_archive_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
        let file = fs::File::open(path).expect("source package archive");
        let mut archive = ZipArchive::new(file).expect("source package zip");
        (0..archive.len())
            .map(|index| {
                let mut entry = archive.by_index(index).expect("archive entry");
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).expect("archive bytes");
                (entry.name().to_owned(), bytes)
            })
            .collect()
    }

    fn replace_archive_entry(entries: &mut [(String, Vec<u8>)], path: &str, bytes: Vec<u8>) {
        let entry = entries
            .iter_mut()
            .find(|(entry_path, _)| entry_path == path)
            .expect("archive entry to replace");
        entry.1 = bytes;
    }

    fn update_component(manifest: &mut aeria_hsp::HspManifest, kind: &str, bytes: &[u8]) {
        let component = manifest
            .components
            .iter_mut()
            .find(|component| component.kind == kind)
            .expect("manifest component");
        component.size = i64::try_from(bytes.len()).expect("component size");
        component.sha256 = hash_bytes(bytes);
    }

    fn hxs_snapshot_id(game_version: &str, source_language: &str, content_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"HARMONIA-HXS-SNAPSHOT-v1");
        for value in [game_version, source_language, content_id] {
            hasher.update(
                u32::try_from(value.len())
                    .expect("test value fits HXS framing")
                    .to_le_bytes(),
            );
            hasher.update(value.as_bytes());
        }
        let digest: [u8; 32] = hasher.finalize().into();
        hash_string(digest)
    }

    fn hash_bytes(bytes: &[u8]) -> String {
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        hash_string(digest)
    }

    fn hash_string(digest: [u8; 32]) -> String {
        let mut hex = String::with_capacity(64);
        for byte in digest {
            write!(&mut hex, "{byte:02x}").expect("hex string");
        }
        format!("sha256:{hex}")
    }

    fn binding() -> SourceBindingDto {
        SourceBindingDto {
            sheet_name: "Synthetic".to_owned(),
            row_id: 42,
            subrow_id: 0,
            column_index: 0,
        }
    }

    #[test]
    fn new_state_has_no_current_project() {
        let state = DesktopState::new();

        assert_eq!(
            current_project_with_state(&state).expect("state read"),
            None
        );
        assert_eq!(
            page_translation_rows_with_state(&state, "Synthetic", None, 1).expect_err("no project"),
            CommandError::no_project()
        );
    }

    #[test]
    fn close_is_idempotent_without_an_active_project() {
        let state = DesktopState::new();

        close_project_with_state(&state).expect("first close");
        close_project_with_state(&state).expect("second close");
        assert_eq!(
            current_project_with_state(&state).expect("state read"),
            None
        );
    }

    #[test]
    fn recent_project_open_succeeds_and_refreshes_active_state() {
        let (repository, state, path, entry) = seed_recent_project("recent-open");
        let result = open_recent_project_from_registry(
            &state,
            &entry.id,
            repository.path().join("cache"),
            path,
            false,
        )
        .expect("open recent project");
        assert_eq!(
            PathBuf::from(result.project.repository_root),
            fs::canonicalize(repository.path()).expect("canonical repository")
        );
        assert!(result.warning.is_none());
        assert!(
            current_project_with_state(&state)
                .expect("current project")
                .is_some()
        );
    }

    #[test]
    fn recent_project_unknown_id_is_typed() {
        let (repository, state, path, _entry) = seed_recent_project("recent-unknown");
        let error = open_recent_project_from_registry(
            &state,
            "missing-local-id",
            repository.path().join("cache"),
            path,
            false,
        )
        .expect_err("unknown recent project");
        assert_eq!(error.code, "recentProjectNotFound");
    }

    #[test]
    fn recent_project_missing_paths_are_not_auto_removed() {
        let (repository, state, path, mut entry) = seed_recent_project("recent-missing");
        entry.repository_root = repository
            .path()
            .join("moved")
            .to_string_lossy()
            .into_owned();
        let error = open_recent_project_with_entry(
            &state,
            &entry,
            repository.path().join("cache"),
            path.clone(),
        )
        .expect_err("missing repository");
        assert_eq!(error.code, "recentProjectRepositoryMissing");

        entry.repository_root = repository.path().to_string_lossy().into_owned();
        entry.source_package_path = repository
            .path()
            .join("missing.hsp")
            .to_string_lossy()
            .into_owned();
        let error = open_recent_project_with_entry(
            &state,
            &entry,
            repository.path().join("cache"),
            path.clone(),
        )
        .expect_err("missing source");
        assert_eq!(error.code, "recentProjectSourceMissing");
        assert_eq!(
            aeria_projects::ProjectRegistry::new(path)
                .load()
                .expect("load retained registry")
                .len(),
            1
        );
    }

    #[test]
    fn recent_project_source_package_mismatch_is_explicit() {
        let (repository, state, path, mut entry) = seed_recent_project("recent-mismatch");
        entry.source_package_id =
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned();
        let error =
            open_recent_project_with_entry(&state, &entry, repository.path().join("cache"), path)
                .expect_err("mismatched source package");
        assert_eq!(error.code, "recentProjectSourceMismatch");
        assert!(
            current_project_with_state(&state)
                .expect("current project")
                .is_none()
        );
    }

    #[test]
    fn recent_project_replacement_mismatch_is_reported_even_for_compatible_content() {
        let (repository, state, registry_path, entry) =
            seed_recent_project("recent-replacement-mismatch");
        let remembered_entry = entry.clone();
        let source_path = PathBuf::from(&entry.source_package_path);
        let cache_root = repository.path().join("cache");
        let before = open_project_with_state(
            &state,
            repository.path().to_string_lossy().into_owned(),
            source_path.to_string_lossy().into_owned(),
            cache_root.clone(),
        )
        .expect("active project should open");
        let before_active = current_project_with_state(&state).expect("active project snapshot");

        write_valid_incompatible_package(&source_path, repository.path());
        let replacement =
            SourcePackage::open(&source_path, repository.path().join("replacement-cache"))
                .expect("replacement package should be valid");
        assert_ne!(replacement.package_id(), entry.source_package_id);

        ProjectSession::open(
            repository.path(),
            &source_path,
            repository.path().join("direct-cache"),
        )
        .expect("a replacement with identical content is workspace-compatible");

        let error =
            open_recent_project_with_entry(&state, &entry, cache_root, registry_path.clone())
                .expect_err("recent association mismatch should win");
        assert_eq!(error.code, "recentProjectSourceMismatch");
        assert_eq!(
            current_project_with_state(&state).expect("active project after mismatch"),
            before_active
        );
        assert_eq!(
            aeria_projects::ProjectRegistry::new(registry_path)
                .load()
                .expect("registry after mismatch")
                .into_iter()
                .next()
                .expect("remembered entry"),
            remembered_entry
        );
        assert_eq!(before.source_package_id, entry.source_package_id);
    }

    fn v2_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../crates/aeria-hsp/tests/fixtures/synthetic-v2.hsp")
    }

    #[test]
    fn new_project_folders_are_created_with_their_parents() {
        let repository = TestRepository::new("create-project-folder");
        let root = repository
            .path()
            .join("Документы")
            .join("Aeria")
            .join("Русский перевод");
        assert!(create_project_directory(&root).expect("create"));
        assert!(root.is_dir());
        assert!(!create_project_directory(&root).expect("existing"));
        let file = repository.path().join("file");
        fs::write(&file, b"x").expect("file");
        assert_eq!(
            create_project_directory(&file.join("child"))
                .expect_err("under a file")
                .code,
            "projectFolder"
        );
        assert_eq!(
            create_project_directory(Path::new(""))
                .expect_err("empty")
                .code,
            "invalidInput"
        );
    }

    #[test]
    fn local_source_packages_are_found_by_language_and_content() {
        let repository = TestRepository::new("local-package-lookup");
        let store = repository.path().join("source-packages");
        fs::create_dir_all(store.join(STAGING_DIRECTORY)).expect("store");
        fs::copy(fixture_path(), store.join("one.hsp")).expect("v1 package");
        fs::copy(v2_fixture_path(), store.join("two.hsp")).expect("v2 package");
        fs::write(store.join("broken.hsp"), b"not a zip").expect("broken package");
        let cache_root = repository.path().join("cache");

        for fixture in [fixture_path(), v2_fixture_path()] {
            let expected = SourcePackage::open(&fixture, repository.path().join("expected-cache"))
                .expect("fixture package");
            let found = find_local_source_package(
                &store,
                &cache_root,
                expected.source_language(),
                expected.source_content_id(),
            )
            .expect("matching package");
            assert_eq!(found.package_id(), expected.package_id());
        }
        let expected = SourcePackage::open(fixture_path(), &cache_root).expect("fixture package");
        assert!(
            find_local_source_package(&store, &cache_root, "ja", expected.source_content_id())
                .is_none()
        );
        assert!(
            find_local_source_package(
                &store,
                &cache_root,
                "en",
                "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            )
            .is_none()
        );
        assert!(
            find_local_source_package(&repository.path().join("missing"), &cache_root, "en", "x")
                .is_none()
        );
    }

    #[test]
    fn opening_with_another_source_plans_the_update_without_writing() {
        let repository = TestRepository::new("game-open-plan");
        let cache_root = repository.path().join("cache");
        let state = DesktopState::new();
        initialize_project_with_state(
            &state,
            repository.path().to_string_lossy().into_owned(),
            fixture_path().to_string_lossy().into_owned(),
            cache_root.clone(),
            "fr".to_owned(),
        )
        .expect("initialize project");
        close_project_with_state(&state).expect("close");
        let manifest_path = repository.path().join(".aeria").join("manifest.json");
        let manifest = fs::read(&manifest_path).expect("manifest");
        let root = repository.path().to_string_lossy().into_owned();

        let changed = SourcePackage::open(v2_fixture_path(), &cache_root).expect("v2 package");
        match open_or_plan_source_update(&state, &root, changed, cache_root.clone())
            .expect("planned update")
        {
            GameOpenOutcome::SourceUpdateRequired {
                source_package_path,
                ..
            } => assert_eq!(PathBuf::from(source_package_path), v2_fixture_path()),
            GameOpenOutcome::Opened(_) => panic!("changed content must not open directly"),
        }
        assert_eq!(fs::read(&manifest_path).expect("manifest"), manifest);
        assert!(current_project_with_state(&state).expect("state").is_none());

        let current = SourcePackage::open(fixture_path(), &cache_root).expect("v1 package");
        assert!(matches!(
            open_or_plan_source_update(&state, &root, current, cache_root),
            Ok(GameOpenOutcome::Opened(_))
        ));
        assert!(current_project_with_state(&state).expect("state").is_some());
    }

    #[test]
    fn registry_write_failure_keeps_project_active_and_returns_warning() {
        let repository = TestRepository::new("recent-write-failure");
        let source = fixture_path();
        let cache_root = repository.path().join("cache");
        let state = DesktopState::new();
        let summary = initialize_project_with_state(
            &state,
            repository.path().to_string_lossy().into_owned(),
            source.to_string_lossy().into_owned(),
            cache_root,
            "fr".to_owned(),
        )
        .expect("initialize project");
        let blocked_parent = repository.path().join("not-a-directory");
        fs::write(&blocked_parent, b"owned test failure").expect("blocked parent");
        let result = remember_project(
            &state,
            summary.clone(),
            Ok(blocked_parent.join(REGISTRY_FILE_NAME)),
        );
        assert_eq!(result.project, summary);
        assert_eq!(
            result.warning.expect("warning").code,
            "projectRegistryWrite"
        );
        assert!(
            current_project_with_state(&state)
                .expect("active project")
                .is_some()
        );
    }

    #[test]
    fn forgetting_recent_project_does_not_close_or_delete_the_active_project() {
        let repository = TestRepository::new("recent-forget-active");
        let source = fixture_path();
        let state = DesktopState::new();
        let summary = initialize_project_with_state(
            &state,
            repository.path().to_string_lossy().into_owned(),
            source.to_string_lossy().into_owned(),
            repository.path().join("cache"),
            "fr".to_owned(),
        )
        .expect("initialize project");
        let path = registry_path(&repository);
        let result = remember_project(&state, summary, Ok(path.clone()));
        let id = aeria_projects::ProjectRegistry::new(&path)
            .load()
            .expect("load registry")
            .into_iter()
            .next()
            .expect("recent entry")
            .id;

        forget_recent_project_from_registry(&state, &path, &id).expect("forget recent project");
        assert!(result.warning.is_none());
        assert!(
            current_project_with_state(&state)
                .expect("active project")
                .is_some()
        );
        assert!(repository.path().join(".aeria").is_dir());
        assert!(Path::new(&result.project.source_package_path).is_file());
        assert!(
            aeria_projects::ProjectRegistry::new(path)
                .load()
                .expect("load empty registry")
                .is_empty()
        );
    }

    #[test]
    fn invalid_translation_unit_id_is_a_typed_error() {
        let error = parse_translation_unit_id("not-a-tu").expect_err("invalid ID");

        assert_eq!(error.code, "invalidTranslationUnitId");
    }

    #[test]
    fn mutation_commands_check_for_a_project_after_parsing_ids() {
        let state = DesktopState::new();

        let error =
            set_translation_note_with_state(&state, "not-a-tu", None).expect_err("invalid ID");
        assert_eq!(error.code, "noProjectOpen");

        let error =
            set_translation_review_state_with_state(&state, "not-a-tu", ReviewStateDto::Reviewed)
                .expect_err("no project");
        assert_eq!(error.code, "noProjectOpen");

        let error =
            set_translation_target_with_state(&state, binding(), "target").expect_err("no project");
        assert_eq!(error.code, "noProjectOpen");
    }

    #[test]
    fn atlas_override_path_is_authoritative_when_it_points_to_a_file() {
        let repository = TestRepository::new("atlas-override");
        let override_path = repository.path().join("harmonia-atlas");
        fs::write(&override_path, b"fixture").expect("override executable");

        assert_eq!(
            atlas_override_path(Some(override_path.clone().into_os_string()))
                .expect("override path"),
            Some(override_path)
        );
        assert_eq!(atlas_override_path(None).expect("unset override"), None);
    }

    #[test]
    fn runtime_filename_does_not_expose_a_target_triple() {
        assert!(!atlas_runtime_filename().contains("x86_64"));
        assert_eq!(
            atlas_runtime_filename(),
            if cfg!(target_os = "windows") {
                "harmonia-atlas.exe"
            } else {
                "harmonia-atlas"
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_packaged_atlas_candidate_is_next_to_aeria_executable() {
        let executable = Path::new(r"C:\Program Files\Aeria\aeria.exe");
        let candidates = bundled_atlas_candidates(
            executable,
            Some(Path::new(r"C:\Program Files\Aeria\resources")),
        );

        assert_eq!(
            candidates.first(),
            Some(&PathBuf::from(r"C:\Program Files\Aeria\harmonia-atlas.exe"))
        );
        assert!(
            !candidates
                .first()
                .expect("primary candidate")
                .to_string_lossy()
                .contains("x86_64-pc-windows-msvc")
        );
    }

    #[cfg(unix)]
    #[test]
    fn linux_packaged_atlas_candidate_is_next_to_aeria_executable() {
        let executable = Path::new("/usr/bin/aeria");
        let candidates = bundled_atlas_candidates(executable, Some(Path::new("/usr/lib/aeria")));

        assert_eq!(
            candidates.first(),
            Some(&PathBuf::from("/usr/bin/harmonia-atlas"))
        );
        assert!(
            !candidates
                .first()
                .expect("primary candidate")
                .to_string_lossy()
                .contains("x86_64-unknown-linux-gnu")
        );
    }

    #[test]
    fn bundled_sidecar_prefers_executable_sibling_over_resource_fallbacks() {
        let executable = if cfg!(target_os = "windows") {
            PathBuf::from(r"C:\Program Files\Aeria\aeria.exe")
        } else {
            PathBuf::from("/usr/bin/aeria")
        };
        let resource_dir = if cfg!(target_os = "windows") {
            PathBuf::from(r"C:\Program Files\Aeria\resources")
        } else {
            PathBuf::from("/usr/lib/aeria")
        };
        let candidates = bundled_atlas_candidates(&executable, Some(&resource_dir));

        assert_eq!(
            candidates[0],
            executable
                .parent()
                .expect("executable directory")
                .join(atlas_runtime_filename())
        );
        assert!(
            candidates
                .iter()
                .skip(1)
                .all(|candidate| candidate.starts_with(&resource_dir))
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| !candidate.to_string_lossy().contains("x86_64-"))
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn desktop_boundary_lifecycle_and_read_write_flow_are_persisted() {
        let repository = TestRepository::new("flow");
        let source = fixture_path();
        let cache_root = repository.path().join("cache");
        let state = DesktopState::new();

        let summary = initialize_project_with_state(
            &state,
            repository.path().to_string_lossy().into_owned(),
            source.to_string_lossy().into_owned(),
            cache_root.clone(),
            "fr".to_owned(),
        )
        .expect("initialize project");
        assert_eq!(
            summary.repository_root,
            repository.path().to_string_lossy().into_owned()
        );
        assert_eq!(
            summary.source_package_path,
            source.to_string_lossy().into_owned()
        );
        assert_eq!(summary.source_language, "en");
        assert_eq!(summary.target_language, "fr");
        assert!(summary.source_content_id.starts_with("sha256:"));
        assert!(summary.source_snapshot_id.starts_with("sha256:"));
        assert_eq!(summary.game_version, "test-game");
        assert_eq!(summary.scope, "full");
        assert_eq!(
            summary.sheets,
            vec![ProjectSheetDto {
                name: "Synthetic".to_owned(),
                effective_language: "en".to_owned(),
                row_count: 1,
                translatable_cell_count: 1,
            }]
        );
        assert_eq!(
            current_project_with_state(&state).expect("current"),
            Some(summary.clone())
        );

        let page =
            page_translation_rows_with_state(&state, "Synthetic", None, 1).expect("initial page");
        assert_eq!(page.rows[0].cells[0].source_macro, "one");
        assert!(page.rows[0].cells[0].translation.is_none());
        assert_eq!(
            translation_progress_with_state(&state).expect("empty progress"),
            Vec::new()
        );

        let active_invalid_note =
            set_translation_note_with_state(&state, "not-a-tu", None).expect_err("invalid ID");
        assert_eq!(active_invalid_note.code, "invalidTranslationUnitId");
        let active_invalid_review =
            set_translation_review_state_with_state(&state, "not-a-tu", ReviewStateDto::Reviewed)
                .expect_err("invalid ID");
        assert_eq!(active_invalid_review.code, "invalidTranslationUnitId");

        let binding = binding();
        let first_id = set_translation_target_with_state(&state, binding.clone(), "Bonjour")
            .expect("set target")
            .translation_unit_id;
        let page = page_translation_rows_with_state(&state, "Synthetic", None, 1)
            .expect("translated page");
        let overlay = page.rows[0].cells[0]
            .translation
            .as_ref()
            .expect("translation overlay");
        assert_eq!(overlay.translation_unit_id, first_id);
        assert_eq!(overlay.target_macro, "Bonjour");

        let second_id = set_translation_target_with_state(&state, binding, "Salut")
            .expect("update target")
            .translation_unit_id;
        assert_eq!(second_id, first_id);
        let note_overlay =
            set_translation_note_with_state(&state, &first_id, Some("checked".to_owned()))
                .expect("set note");
        assert_eq!(note_overlay.translation_unit_id, first_id);
        assert_eq!(note_overlay.translator_note.as_deref(), Some("checked"));
        let review_overlay =
            set_translation_review_state_with_state(&state, &first_id, ReviewStateDto::Reviewed)
                .expect("set review state");
        assert_eq!(review_overlay.translation_unit_id, first_id);
        assert_eq!(review_overlay.review_state, ReviewStateDto::Reviewed);
        assert_eq!(
            translation_progress_with_state(&state).expect("progress"),
            vec![SheetProgressDto {
                sheet_name: "Synthetic".to_owned(),
                translated: 1,
                reviewed: 1,
                needs_review: 0,
            }]
        );

        close_project_with_state(&state).expect("close project");
        assert_eq!(
            translation_progress_with_state(&state)
                .expect_err("closed progress")
                .code,
            CommandError::no_project().code
        );
        assert_eq!(current_project_with_state(&state).expect("closed"), None);
        open_project_with_state(
            &state,
            repository.path().to_string_lossy().into_owned(),
            source.to_string_lossy().into_owned(),
            cache_root,
        )
        .expect("reopen project");
        let reopened_page =
            page_translation_rows_with_state(&state, "Synthetic", None, 1).expect("reopened page");
        let reopened_overlay = reopened_page.rows[0].cells[0]
            .translation
            .as_ref()
            .expect("persisted overlay");
        assert_eq!(reopened_overlay.translation_unit_id, first_id);
        assert_eq!(reopened_overlay.target_macro, "Salut");
        assert_eq!(reopened_overlay.review_state, ReviewStateDto::Reviewed);
        assert_eq!(reopened_overlay.translator_note.as_deref(), Some("checked"));

        let before_failed_replacement = current_project_with_state(&state)
            .expect("current before failed replacement")
            .expect("active project");
        let invalid_source = repository.path().join("invalid.hxs");
        fs::write(&invalid_source, b"not an HXS database").expect("invalid source");
        let error = open_project_with_state(
            &state,
            repository.path().to_string_lossy().into_owned(),
            invalid_source.to_string_lossy().into_owned(),
            repository.path().join("cache"),
        )
        .expect_err("invalid replacement");
        assert_eq!(error.code, "projectSource");
        assert_eq!(
            current_project_with_state(&state).expect("current after failure"),
            Some(before_failed_replacement)
        );
    }

    #[test]
    fn valid_immutable_package_collision_reuses_final_and_removes_staging() {
        let temp = TestRepository::new("package-collision");
        let fixture = fixture_path();
        let cache = temp.path().join("cache");
        let existing = temp.path().join("source-packages");
        let staging = existing.join("staging/source.hsp");
        fs::create_dir_all(staging.parent().expect("staging parent")).expect("staging");
        fs::copy(&fixture, &staging).expect("staging package");
        let package = SourcePackage::open(&fixture, &cache).expect("fixture package");
        let final_path = existing.join(format!(
            "{}.hsp",
            package.package_id().strip_prefix("sha256:").expect("hash")
        ));
        fs::copy(&fixture, &final_path).expect("existing package");

        let result = AtlasPackageResult {
            package_id: package.package_id().to_owned(),
            output_path: staging.clone(),
            completed_metadata: std::collections::BTreeMap::new(),
        };
        let reused = validate_and_publish_package(
            &result,
            &staging,
            &existing,
            &cache,
            &CancellationToken::default(),
        )
        .expect("reuse");
        assert_eq!(reused.package_id(), result.package_id);
        assert!(!staging.exists());
        assert!(final_path.exists());
    }

    #[test]
    fn invalid_immutable_package_collision_is_replaced() {
        let temp = TestRepository::new("package-replacement");
        let fixture = fixture_path();
        let cache = temp.path().join("cache");
        let existing = temp.path().join("source-packages");
        let staging = existing.join("staging/source.hsp");
        fs::create_dir_all(staging.parent().expect("staging parent")).expect("staging");
        fs::copy(&fixture, &staging).expect("staging package");
        let package = SourcePackage::open(&fixture, &cache).expect("fixture package");
        let final_path = existing.join(format!(
            "{}.hsp",
            package.package_id().strip_prefix("sha256:").expect("hash")
        ));
        fs::write(&final_path, b"invalid old package").expect("invalid package");

        let result = AtlasPackageResult {
            package_id: package.package_id().to_owned(),
            output_path: staging.clone(),
            completed_metadata: std::collections::BTreeMap::new(),
        };
        let replacement = validate_and_publish_package(
            &result,
            &staging,
            &existing,
            &cache,
            &CancellationToken::default(),
        )
        .expect("replace");
        assert_eq!(replacement.package_id(), result.package_id);
        assert!(!staging.exists());
        assert!(final_path.exists());
        assert!(!final_path.with_extension("hsp.invalid").exists());
    }

    #[test]
    fn new_package_is_validated_before_final_publication() {
        let temp = TestRepository::new("package-new");
        let fixture = fixture_path();
        let cache = temp.path().join("cache");
        let packages = temp.path().join("source-packages");
        let staging = packages.join("staging/source.hsp");
        fs::create_dir_all(staging.parent().expect("staging parent")).expect("staging");
        fs::copy(&fixture, &staging).expect("staging package");
        let package = SourcePackage::open(&fixture, &cache).expect("fixture package");
        let final_path = packages.join(format!(
            "{}.hsp",
            package.package_id().strip_prefix("sha256:").expect("hash")
        ));
        let result = AtlasPackageResult {
            package_id: package.package_id().to_owned(),
            output_path: staging.clone(),
            completed_metadata: std::collections::BTreeMap::new(),
        };
        let published = validate_and_publish_package_with_hook(
            &result,
            &staging,
            &packages,
            &cache,
            &CancellationToken::default(),
            || assert!(!final_path.exists()),
        )
        .expect("publish new package");

        assert_eq!(published.package_path(), final_path);
        assert!(!staging.exists());
        assert!(final_path.is_file());
    }

    #[test]
    fn staging_package_id_mismatch_is_removed_without_creating_final() {
        let temp = TestRepository::new("package-mismatch");
        let fixture = fixture_path();
        let cache = temp.path().join("cache");
        let packages = temp.path().join("source-packages");
        let staging = packages.join("staging/source.hsp");
        fs::create_dir_all(staging.parent().expect("staging parent")).expect("staging");
        fs::copy(&fixture, &staging).expect("staging package");
        let result = AtlasPackageResult {
            package_id: "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                .to_owned(),
            output_path: staging.clone(),
            completed_metadata: std::collections::BTreeMap::new(),
        };

        let Err(error) = validate_and_publish_package(
            &result,
            &staging,
            &packages,
            &cache,
            &CancellationToken::default(),
        ) else {
            panic!("package ID mismatch must fail")
        };
        assert_eq!(error.code, "atlasPackage");
        assert!(!staging.exists());
        assert!(
            !packages
                .join("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff.hsp")
                .exists()
        );
    }

    #[test]
    fn invalid_staging_leaves_invalid_final_untouched() {
        let temp = TestRepository::new("package-invalid-staging");
        let cache = temp.path().join("cache");
        let packages = temp.path().join("source-packages");
        let staging = packages.join("staging/source.hsp");
        fs::create_dir_all(staging.parent().expect("staging parent")).expect("staging");
        fs::write(&staging, b"invalid new package").expect("invalid staging");
        let package_id = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let final_path = packages.join(format!(
            "{}.hsp",
            package_id.strip_prefix("sha256:").expect("hash")
        ));
        let before = b"invalid old package";
        fs::write(&final_path, before).expect("invalid final");
        let result = AtlasPackageResult {
            package_id: package_id.to_owned(),
            output_path: staging.clone(),
            completed_metadata: std::collections::BTreeMap::new(),
        };

        let Err(error) = validate_and_publish_package(
            &result,
            &staging,
            &packages,
            &cache,
            &CancellationToken::default(),
        ) else {
            panic!("invalid staging must fail")
        };
        assert_eq!(error.code, "atlasPackage");
        assert_eq!(fs::read(&final_path).expect("final bytes"), before);
        assert!(!staging.exists());
        assert!(!final_path.with_extension("hsp.invalid").exists());
    }

    #[test]
    fn cancel_after_completed_prevents_workspace_and_active_project_publication() {
        let temp = TestRepository::new("cancel-after-completed");
        let fixture = fixture_path();
        let cache = temp.path().join("cache");
        let packages = temp.path().join("source-packages");
        let staging = packages.join("staging/source.hsp");
        fs::create_dir_all(staging.parent().expect("staging parent")).expect("staging");
        fs::copy(&fixture, &staging).expect("staging package");
        let package = SourcePackage::open(&fixture, &cache).expect("fixture package");
        let result = AtlasPackageResult {
            package_id: package.package_id().to_owned(),
            output_path: staging.clone(),
            completed_metadata: std::collections::BTreeMap::new(),
        };
        let (token, handle) = CancellationToken::new();
        let state = DesktopState::new();
        let started = state.start_atlas_job().expect("Atlas job");
        assert_eq!(started.id, "atlas-0000000000000001");

        let error = state
            .with_atlas_publication(&started.id, &token, |_| {
                let source_package = validate_and_publish_package_with_hook(
                    &result,
                    &staging,
                    &packages,
                    &cache,
                    &token,
                    || handle.cancel(),
                )?;
                require_not_cancelled(&token)?;
                let replacement = ProjectSession::initialize_from_source_package(
                    temp.path().to_owned(),
                    source_package,
                    "fr",
                )
                .map_err(CommandError::from)?;
                require_not_cancelled(&token)?;
                replace_project(&state, replacement)
            })
            .expect_err("cancelled completed flow");
        assert_eq!(error.code, "atlasCancelled");
        assert_eq!(
            current_project_with_state(&state).expect("current project"),
            None
        );
        assert!(!temp.path().join(".aeria").exists());
        state.finish_atlas_job(&started.id).expect("finish job");
    }

    #[test]
    fn cancellation_after_runner_error_removes_partial_staging() {
        let temp = TestRepository::new("cancelled-runner-staging");
        let staging = temp.path().join("staging/source.hsp");
        fs::create_dir_all(staging.parent().expect("staging parent")).expect("staging");
        fs::write(&staging, b"partial Atlas output").expect("partial staging");
        let (token, handle) = CancellationToken::new();
        handle.cancel();

        let error = atlas_runner_error(
            AtlasError::Cancelled {
                stderr_tail: String::new(),
            },
            &token,
            &staging,
        );

        assert_eq!(error.code, "atlasCancelled");
        assert!(!staging.exists());
    }
}
