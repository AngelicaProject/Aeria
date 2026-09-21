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
use aeria_workspace::{ProjectSession, ProjectSessionError};
use serde::Serialize;

use tauri::{Emitter, Manager, State};

use crate::dto::{
    ProjectOpenResultDto, ProjectSummaryDto, RecentProjectDto, ReviewStateDto, SourceBindingDto,
    SourcePackageJobDto, TranslationRowCursorDto, TranslationRowPageDto, TranslationUnitIdDto,
};
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

const SOURCE_PACKAGES_DIRECTORY: &str = "source-packages";
const STAGING_DIRECTORY: &str = "staging";
const STAGING_FILE: &str = "source.hsp";

fn app_registry_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join(REGISTRY_FILE_NAME))
        .map_err(|error| format!("could not resolve the Aeria app-data directory: {error}"))
}

fn remember_project(
    state: &DesktopState,
    project: ProjectSummaryDto,
    registry_path: Result<PathBuf, String>,
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
    ProjectOpenResultDto { project, warning }
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
/// # Errors
///
/// Returns a typed command error when source verification, workspace loading,
/// compatibility validation, or state locking fails.
pub fn open_project(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    repository_root: String,
    source_package_path: String,
) -> CommandResult<ProjectOpenResultDto> {
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
    let registry_path = app_registry_path(&app);
    let project =
        open_project_with_state(&state, repository_root, source_package_path, cache_root)?;
    Ok(remember_project(&state, project, registry_path))
}

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

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Initializes a new project and makes it the active desktop project.
///
/// # Errors
///
/// Returns a typed command error when source verification, workspace
/// initialization, or state locking fails.
pub fn initialize_project(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    repository_root: String,
    source_package_path: String,
    target_language: String,
) -> CommandResult<ProjectOpenResultDto> {
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
    let registry_path = app_registry_path(&app);
    let project = initialize_project_with_state(
        &state,
        repository_root,
        source_package_path,
        cache_root,
        target_language,
    )?;
    Ok(remember_project(&state, project, registry_path))
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
pub fn list_recent_projects(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> CommandResult<Vec<RecentProjectDto>> {
    let path = app_registry_path(&app)
        .map_err(|message| CommandError::new("projectRegistryRead", message))?;
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
pub fn open_recent_project(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    project_id: String,
) -> CommandResult<ProjectOpenResultDto> {
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
    let registry_path = app_registry_path(&app)
        .map_err(|message| CommandError::new("projectRegistryRead", message))?;
    open_recent_project_from_registry(&state, &project_id, cache_root, registry_path)
}

fn open_recent_project_from_registry(
    state: &DesktopState,
    project_id: &str,
    cache_root: PathBuf,
    registry_path: PathBuf,
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

    open_recent_project_with_entry(state, &entry, cache_root, registry_path)
}

fn open_recent_project_with_entry(
    state: &DesktopState,
    entry: &RegistryEntry,
    cache_root: PathBuf,
    registry_path: PathBuf,
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

    let replacement = ProjectSession::open_from_source_package(repository_root, source_package)
        .map_err(CommandError::from)?;
    let project = replace_project(state, replacement)?;
    Ok(remember_project(state, project, Ok(registry_path)))
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Removes one entry from Recent projects without touching project files.
///
/// # Errors
///
/// Returns a typed registry error when app-data cannot be resolved, the entry
/// is absent, or the updated registry cannot be published.
pub fn forget_recent_project(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    project_id: String,
) -> CommandResult<()> {
    let path = app_registry_path(&app)
        .map_err(|message| CommandError::new("projectRegistryWrite", message))?;
    forget_recent_project_from_registry(&state, &path, &project_id)
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

/// Generates a source package from a local game installation and initializes
/// the project from the already validated package.
///
/// # Errors
///
/// Returns a typed error when the job is no longer active, Atlas cannot run,
/// package publication or validation fails, or workspace initialization fails.
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub async fn initialize_project_from_game(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    job_id: String,
    repository_root: String,
    game_path: String,
    source_language: String,
    target_language: String,
) -> CommandResult<ProjectOpenResultDto> {
    let token = state.atlas_job_token(&job_id)?;
    let worker_job_id = job_id.clone();
    let worker_token = token.clone();
    let worker_app = app.clone();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        initialize_project_from_game_inner(
            &worker_app,
            &worker_job_id,
            &worker_token,
            repository_root,
            game_path,
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
    match result {
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
    }
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

#[allow(clippy::too_many_arguments)]
fn initialize_project_from_game_inner(
    app: &tauri::AppHandle,
    job_id: &str,
    cancellation: &CancellationToken,
    repository_root: String,
    game_path: String,
    source_language: String,
    target_language: String,
) -> Result<PreparedAtlasProject, CommandError> {
    let executable_path = resolve_atlas_executable(app)?;
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| CommandError::new("atlasStorage", error.to_string()))?;
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandError::new("atlasStorage", error.to_string()))?;
    let packages_root = app_data.join(SOURCE_PACKAGES_DIRECTORY);
    let staging_root = packages_root.join(STAGING_DIRECTORY);
    fs::create_dir_all(&staging_root)
        .map_err(|error| storage_error("create source-package staging directory", &error))?;
    let staging_path = staging_root.join(STAGING_FILE);

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
    require_not_cancelled(cancellation)?;
    Ok(PreparedAtlasProject {
        repository_root,
        source_package,
        target_language,
    })
}

struct PreparedAtlasProject {
    repository_root: String,
    source_package: SourcePackage,
    target_language: String,
}

fn resolve_atlas_executable(app: &tauri::AppHandle) -> CommandResult<PathBuf> {
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
                Ok(validated.relocate_package_path(final_path))
            }
            Err(error) => {
                let _ = fs::rename(&backup_path, &final_path);
                Err(storage_error("publish replacement source package", &error))
            }
        }
    } else {
        fs::rename(staging_path, &final_path)
            .map_err(|error| storage_error("atomically publish source package", &error))?;
        Ok(validated.relocate_package_path(final_path))
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
pub fn page_translation_rows(
    state: State<'_, DesktopState>,
    sheet_name: String,
    after: Option<TranslationRowCursorDto>,
    limit: u32,
) -> CommandResult<TranslationRowPageDto> {
    page_translation_rows_with_state(&state, &sheet_name, after, limit)
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
pub fn set_translation_target(
    state: State<'_, DesktopState>,
    source_binding: SourceBindingDto,
    target_macro: String,
) -> CommandResult<TranslationUnitIdDto> {
    set_translation_target_with_state(&state, source_binding, &target_macro)
}

pub(crate) fn set_translation_target_with_state(
    state: &DesktopState,
    source_binding: SourceBindingDto,
    target_macro: &str,
) -> CommandResult<TranslationUnitIdDto> {
    let source_binding = SourceBinding::from(source_binding);
    let mut project = state.lock_project()?;
    let project = project.as_mut().ok_or_else(CommandError::no_project)?;
    project
        .set_target(&source_binding, target_macro)
        .map(Into::into)
        .map_err(CommandError::from)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Replaces or clears the note for one translation unit.
///
/// # Errors
///
/// Returns a typed command error when the ID is invalid, no project is open,
/// or the backend rejects the operation.
pub fn set_translation_note(
    state: State<'_, DesktopState>,
    translation_unit_id: String,
    note: Option<String>,
) -> CommandResult<()> {
    set_translation_note_with_state(&state, &translation_unit_id, note)
}

pub(crate) fn set_translation_note_with_state(
    state: &DesktopState,
    translation_unit_id: &str,
    note: Option<String>,
) -> CommandResult<()> {
    let mut project = state.lock_project()?;
    let project = project.as_mut().ok_or_else(CommandError::no_project)?;
    let translation_unit_id = parse_translation_unit_id(translation_unit_id)?;
    project
        .set_note(translation_unit_id, note)
        .map_err(CommandError::from)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Sets the explicit review state for one translation unit.
///
/// # Errors
///
/// Returns a typed command error when the ID is invalid, no project is open,
/// or the backend rejects the operation.
pub fn set_translation_review_state(
    state: State<'_, DesktopState>,
    translation_unit_id: String,
    review_state: ReviewStateDto,
) -> CommandResult<()> {
    set_translation_review_state_with_state(&state, &translation_unit_id, review_state)
}

pub(crate) fn set_translation_review_state_with_state(
    state: &DesktopState,
    translation_unit_id: &str,
    review_state: ReviewStateDto,
) -> CommandResult<()> {
    let mut project = state.lock_project()?;
    let project = project.as_mut().ok_or_else(CommandError::no_project)?;
    let translation_unit_id = parse_translation_unit_id(translation_unit_id)?;
    let review_state: ReviewState = review_state.into();
    project
        .set_review_state(translation_unit_id, review_state)
        .map_err(CommandError::from)
}

fn parse_translation_unit_id(value: &str) -> CommandResult<TranslationUnitId> {
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
    fn recent_project_replacement_mismatch_precedes_workspace_compatibility() {
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

        let compatibility_error = match ProjectSession::open(
            repository.path(),
            &source_path,
            repository.path().join("direct-cache"),
        ) {
            Ok(_) => panic!("replacement package should be workspace-incompatible"),
            Err(error) => CommandError::from(error),
        };
        assert_eq!(compatibility_error.code, "projectCompatibility");

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
        set_translation_note_with_state(&state, &first_id, Some("checked".to_owned()))
            .expect("set note");
        set_translation_review_state_with_state(&state, &first_id, ReviewStateDto::Reviewed)
            .expect("set review state");

        close_project_with_state(&state).expect("close project");
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
