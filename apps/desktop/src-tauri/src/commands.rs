use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aeria_core::{ReviewState, SourceBinding, TranslationUnitId};
use aeria_projects::{ProjectMetadata, ProjectRegistry, REGISTRY_FILE_NAME, RegistryEntry};
use aeria_source::GameSource;
use aeria_workspace::{ProjectSession, ProjectSessionError, WorkspaceStore};
use aeria_workspace::{TranslationReadError, TranslationRowCursor, TranslationRowView};

use tauri::{Manager, State};

use crate::dto::{
    DetachedUnitDto, GameOpenResultDto, OtherLanguageTextDto, ProjectOpenResultDto,
    ProjectSummaryDto, RecentProjectDto, ReviewStateDto, SheetProgressDto, SourceBindingDto,
    SourceUpdateReportDto, TranslationOverlayDto, TranslationRowCursorDto, TranslationRowPageDto,
};
use crate::error::CommandError;
use crate::paths::AeriaPaths;
use crate::scene::{QuestNameDto, SheetDialogueDto};
use crate::source::{load_catalog_with_cache, open_game};
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

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

fn cache_root(app: &tauri::AppHandle) -> CommandResult<PathBuf> {
    app.aeria_cache_dir()
        .map_err(|error| CommandError::new("cachePath", error.to_string()))
}

fn remember_project(
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

/// Opens the configured game in the project's source language.
fn project_game(app: &tauri::AppHandle, repository_root: &Path) -> CommandResult<Arc<GameSource>> {
    let metadata = WorkspaceStore::new(repository_root)
        .read_metadata()
        .map_err(CommandError::from)?;
    open_game(app, metadata.source_language())
}

/// Opens a project against `source`. When it needs a source update, nothing
/// is written: with `accept_source_update` the update is applied, otherwise
/// the plan is returned for confirmation.
pub(crate) fn open_with_game(
    state: &DesktopState,
    repository_root: &Path,
    source: Arc<GameSource>,
    cache_root: &Path,
    accept_source_update: bool,
) -> CommandResult<GameOpenOutcome> {
    load_catalog_with_cache(cache_root, &source)?;
    if accept_source_update {
        let (session, report) = ProjectSession::open_with_source_update(repository_root, source)
            .map_err(CommandError::from)?;
        let report = report.as_ref().map(SourceUpdateReportDto::from);
        return Ok(GameOpenOutcome::Opened {
            project: replace_project(state, session)?,
            source_update: report,
        });
    }
    match ProjectSession::open(repository_root, Arc::clone(&source)) {
        Ok(session) => Ok(GameOpenOutcome::Opened {
            project: replace_project(state, session)?,
            source_update: None,
        }),
        Err(ProjectSessionError::SourceUpdateRequired { .. }) => {
            let report = ProjectSession::preview_source_update(repository_root, &source)
                .map_err(CommandError::from)?;
            Ok(GameOpenOutcome::SourceUpdateRequired {
                report: SourceUpdateReportDto::from(&report),
            })
        }
        Err(error) => Err(CommandError::from(error)),
    }
}

pub(crate) enum GameOpenOutcome {
    Opened {
        project: ProjectSummaryDto,
        source_update: Option<SourceUpdateReportDto>,
    },
    SourceUpdateRequired {
        report: SourceUpdateReportDto,
    },
}

fn remembered_outcome(
    state: &DesktopState,
    outcome: GameOpenOutcome,
    registry_path: Result<PathBuf, String>,
) -> GameOpenResultDto {
    match outcome {
        GameOpenOutcome::Opened {
            project,
            source_update,
        } => GameOpenResultDto::Opened {
            result: Box::new(remember_project(
                state,
                project,
                registry_path,
                source_update,
            )),
        },
        GameOpenOutcome::SourceUpdateRequired { report } => {
            GameOpenResultDto::SourceUpdateRequired { report }
        }
    }
}

/// Opens an existing project with the configured game installation.
///
/// The project's source language is read from its workspace manifest. When
/// the project needs a source update, nothing is written and the plan is
/// returned for confirmation; [`update_project_from_game`] applies it.
///
/// # Errors
///
/// Returns a typed error when the workspace cannot be read, no game
/// installation is available, the game is older than the project, or the
/// project cannot be opened.
#[tauri::command(rename_all = "camelCase")]
pub async fn open_project_from_game(
    app: tauri::AppHandle,
    repository_root: String,
) -> CommandResult<GameOpenResultDto> {
    let cache_root = cache_root(&app)?;
    let registry_path = app_registry_path(&app);
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let root = PathBuf::from(&repository_root);
        let source = project_game(&app, &root)?;
        let outcome = open_with_game(&state, &root, source, &cache_root, false)?;
        Ok(remembered_outcome(&state, outcome, registry_path))
    })
    .await
}

/// Opens an existing project with the configured game installation and
/// applies the deterministic source update when it is required.
///
/// # Errors
///
/// Returns a typed error when the workspace cannot be read, no game
/// installation is available, the game is older than the project, or the
/// source update fails.
#[tauri::command(rename_all = "camelCase")]
pub async fn update_project_from_game(
    app: tauri::AppHandle,
    repository_root: String,
) -> CommandResult<ProjectOpenResultDto> {
    let cache_root = cache_root(&app)?;
    let registry_path = app_registry_path(&app);
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let root = PathBuf::from(&repository_root);
        let source = project_game(&app, &root)?;
        match open_with_game(&state, &root, source, &cache_root, true)? {
            GameOpenOutcome::Opened {
                project,
                source_update,
            } => Ok(remember_project(
                &state,
                project,
                registry_path,
                source_update,
            )),
            GameOpenOutcome::SourceUpdateRequired { .. } => Err(CommandError::internal_state(
                "an accepted source update was not applied",
            )),
        }
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Plans the source update that opening a project with the configured game
/// would apply, without writing anything or changing the active project.
///
/// # Errors
///
/// Returns a typed command error when the workspace or game cannot be read,
/// the source language differs, the game is older than the project, or the
/// plan cannot be built.
pub async fn preview_source_update(
    app: tauri::AppHandle,
    repository_root: String,
) -> CommandResult<SourceUpdateReportDto> {
    run_blocking(move || {
        let root = PathBuf::from(&repository_root);
        let source = project_game(&app, &root)?;
        ProjectSession::preview_source_update(&root, &source)
            .map(|report| SourceUpdateReportDto::from(&report))
            .map_err(CommandError::from)
    })
    .await
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

/// Creates a project for the configured game installation, in a new or empty
/// folder, and makes it the active project.
///
/// # Errors
///
/// Returns a typed error when the folder cannot be created, no game
/// installation is available, the source language is unknown, or workspace
/// initialization fails.
#[tauri::command(rename_all = "camelCase")]
pub async fn initialize_project_from_game(
    app: tauri::AppHandle,
    repository_root: String,
    source_language: String,
    target_language: String,
) -> CommandResult<ProjectOpenResultDto> {
    let cache_root = cache_root(&app)?;
    let registry_path = app_registry_path(&app);
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let root = PathBuf::from(&repository_root);
        let created = create_project_directory(&root)?;
        let result = open_game(&app, &source_language).and_then(|source| {
            initialize_with_game(&state, &root, source, &cache_root, target_language)
        });
        if result.is_err() && created {
            // Only an empty folder is removed; anything written into it stays.
            let _ = fs::remove_dir(&root);
        }
        result.map(|project| remember_project(&state, project, registry_path, None))
    })
    .await
}

pub(crate) fn initialize_with_game(
    state: &DesktopState,
    repository_root: &Path,
    source: Arc<GameSource>,
    cache_root: &Path,
    target_language: String,
) -> CommandResult<ProjectSummaryDto> {
    if !aeria_core::is_target_language(&target_language) {
        return Err(ProjectSessionError::InvalidTargetLanguage {
            tag: target_language,
        }
        .into());
    }
    load_catalog_with_cache(cache_root, &source)?;
    let session = ProjectSession::initialize(repository_root, source, target_language)
        .map_err(CommandError::from)?;
    replace_project(state, session)
}

/// Sets the open project's target language, the language it translates
/// into, and records it for Recent projects.
///
/// # Errors
///
/// Returns `invalidTargetLanguage` for a value that is not a BCP 47 language
/// tag or is `und`, `noProjectOpen` without a project, and a store error
/// when the manifest cannot be written. A failed Recent projects update is
/// returned as the result's warning.
#[tauri::command(rename_all = "camelCase")]
pub async fn set_project_target_language(
    app: tauri::AppHandle,
    target_language: String,
) -> CommandResult<ProjectOpenResultDto> {
    let registry_path = app_registry_path(&app);
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let summary = {
            let mut project = state.lock_project()?;
            let project = project.as_mut().ok_or_else(CommandError::no_project)?;
            project
                .set_target_language(target_language.trim())
                .map_err(CommandError::from)?;
            ProjectSummaryDto::from_session(project)
        };
        Ok(remember_project(&state, summary, registry_path, None))
    })
    .await
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
/// Opens one exact local recent-project entry with the configured game and
/// refreshes its cached metadata.
///
/// # Errors
///
/// Returns a typed error when the registry entry is missing, the repository
/// is unavailable, no game installation is available, or the project cannot
/// be opened.
pub async fn open_recent_project(
    app: tauri::AppHandle,
    project_id: String,
    accept_source_update: Option<bool>,
) -> CommandResult<GameOpenResultDto> {
    let cache_root = cache_root(&app)?;
    let registry_path = app_registry_path(&app)
        .map_err(|message| CommandError::new("projectRegistryRead", message))?;
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let entry = recent_entry(&state, &registry_path, &project_id)?;
        let root = recent_repository(&entry)?;
        let source = open_game(&app, &entry.source_language)?;
        let outcome = open_with_game(
            &state,
            &root,
            source,
            &cache_root,
            accept_source_update.unwrap_or(false),
        )?;
        Ok(remembered_outcome(&state, outcome, Ok(registry_path)))
    })
    .await
}

fn recent_entry(
    state: &DesktopState,
    registry_path: &Path,
    project_id: &str,
) -> CommandResult<RegistryEntry> {
    let _lock = state.lock_registry()?;
    ProjectRegistry::new(registry_path)
        .load()
        .map_err(|error| CommandError::registry_read(&error))?
        .into_iter()
        .find(|project| project.id == project_id)
        .ok_or_else(|| {
            CommandError::recent_project(
                "recentProjectNotFound",
                format!("recent project {project_id:?} was not found"),
            )
        })
}

fn recent_repository(entry: &RegistryEntry) -> CommandResult<PathBuf> {
    let root = PathBuf::from(&entry.repository_root);
    if root.is_dir() {
        Ok(root)
    } else {
        Err(CommandError::recent_project(
            "recentProjectRepositoryMissing",
            format!(
                "recent project repository is missing: {}",
                entry.repository_root
            ),
        ))
    }
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

fn replace_project(
    state: &DesktopState,
    replacement: ProjectSession,
) -> CommandResult<ProjectSummaryDto> {
    let summary = ProjectSummaryDto::from_session(&replacement);
    let root = replacement.repository_root().to_owned();
    let mut project = state.lock_project()?;
    *project = Some(replacement);
    state.reset_stamp(Some(&root));
    drop(project);
    crate::git::refresh_merge_driver(state, &root);
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
    state.reset_stamp(None);
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
/// Reads one source cell in the game's other client languages, so a
/// translator can compare its text and tags. Nothing is recorded.
///
/// # Errors
///
/// Returns a typed command error when no project is open or a game file
/// cannot be read.
pub async fn source_in_other_languages(
    app: tauri::AppHandle,
    source_binding: SourceBindingDto,
) -> CommandResult<Vec<OtherLanguageTextDto>> {
    run_blocking(move || {
        let source = {
            let state = app.state::<DesktopState>();
            let project = state.lock_project()?;
            project
                .as_ref()
                .ok_or_else(CommandError::no_project)?
                .source_handle()
        };
        let texts = source.cell_in_other_languages(
            &source_binding.sheet_name,
            source_binding.row_id,
            source_binding.subrow_id,
            source_binding.column_index,
        )?;
        Ok(texts
            .into_iter()
            .map(|(language, text)| OtherLanguageTextDto {
                language: language.code().to_owned(),
                text,
            })
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Reads the dialogue structure of a quest or cutscene sheet for the scene
/// view: the role and speaker label of each row with text, the quest's
/// name and its other versions, and the scenes traced from the quest's
/// script. `None` for any other sheet, or one without row keys. The structure
/// is context only and nothing is recorded.
///
/// # Errors
///
/// Returns a typed command error when no project is open or a game file
/// cannot be read.
pub async fn sheet_dialogue(
    app: tauri::AppHandle,
    sheet_name: String,
) -> CommandResult<Option<SheetDialogueDto>> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        sheet_dialogue_with_state(&state, &sheet_name)
    })
    .await
}

pub(crate) fn sheet_dialogue_with_state(
    state: &DesktopState,
    sheet_name: &str,
) -> CommandResult<Option<SheetDialogueDto>> {
    let source = {
        let project = state.lock_project()?;
        project
            .as_ref()
            .ok_or_else(CommandError::no_project)?
            .source_handle()
    };
    let Some(dialogue) = source.dialogue(sheet_name)? else {
        return Ok(None);
    };
    let quest = match source.quest_row(sheet_name)? {
        Some((row_id, subrow_id)) => {
            let project = state.lock_project()?;
            let session = project.as_ref().ok_or_else(CommandError::no_project)?;
            translation_row(session, QUEST_SHEET, row_id, subrow_id)?
                .and_then(|row| row.cells.into_iter().next())
                .map(|cell| QuestNameDto {
                    source_binding: SourceBindingDto::from(&cell.source_binding),
                    source_macro: cell.source_macro,
                    target_macro: cell.translation.map(|overlay| overlay.target_macro),
                })
        }
        None => None,
    };
    // The script is context: when it cannot be read, the scene view shows
    // the sheet's rows and says why.
    let script = match source.quest_script(sheet_name) {
        Ok(script) => Ok(script),
        Err(
            error @ (aeria_source::SourceError::Script { .. }
            | aeria_source::SourceError::Cutscene { .. }),
        ) => Err(error.to_string()),
        Err(error) => return Err(error.into()),
    };
    let versions = source.quest_versions(sheet_name)?;
    // Every cutscene file is read once per source, on up to four threads.
    let threads = std::thread::available_parallelism().map_or(1, |threads| threads.get().min(4));
    let cutscenes = source.cutscenes_naming(sheet_name, threads)?;
    let rows: Vec<u32> = cutscenes.iter().map(|cutscene| cutscene.row).collect();
    let plays = source.cutscene_plays(&rows)?;
    Ok(Some(SheetDialogueDto::new(
        sheet_name, dialogue, quest, versions, script, cutscenes, plays,
    )))
}

/// The sheet whose rows name quests.
const QUEST_SHEET: &str = "Quest";

/// One logical row with its workspace overlays; `None` when the row has no
/// translatable string.
///
/// # Errors
///
/// Returns an error when the sheet cannot be read.
pub(crate) fn translation_row(
    session: &ProjectSession,
    sheet: &str,
    row_id: u32,
    subrow_id: u16,
) -> Result<Option<TranslationRowView>, TranslationReadError> {
    // The page cursor is exclusive, so start just before the row.
    let cursor = match (row_id, subrow_id) {
        (0, 0) => None,
        (row, 0) => Some(TranslationRowCursor::new(sheet, row - 1, u16::MAX)),
        (row, subrow) => Some(TranslationRowCursor::new(sheet, row, subrow - 1)),
    };
    let page = session.page_translation_rows(sheet, cursor.as_ref(), 1)?;
    Ok(page
        .rows
        .into_iter()
        .find(|view| view.row_id == row_id && view.subrow_id == subrow_id))
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
    state.write_project(|project| {
        project
            .set_target(&source_binding, target_macro)
            .map_err(CommandError::from)
            .and_then(|id| translation_overlay(project, id))
    })
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
    state.write_project(|project| {
        let translation_unit_id = parse_translation_unit_id(translation_unit_id)?;
        project
            .set_note(translation_unit_id, note)
            .map_err(CommandError::from)
            .and_then(|()| translation_overlay(project, translation_unit_id))
    })
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
    state.write_project(|project| {
        let translation_unit_id = parse_translation_unit_id(translation_unit_id)?;
        let review_state: ReviewState = review_state.into();
        project
            .set_review_state(translation_unit_id, review_state)
            .map_err(CommandError::from)
            .and_then(|()| translation_overlay(project, translation_unit_id))
    })
}

pub(crate) fn parse_translation_unit_id(value: &str) -> CommandResult<TranslationUnitId> {
    TranslationUnitId::from_str(value).map_err(CommandError::from)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use aeria_sqpack::testing::{FakeGame, TextSheet};

    use super::*;
    use crate::dto::{ProjectSheetDto, ReviewStateDto, SourceBindingDto};
    use crate::scene::{DialogueKindDto, DialogueRoleDto};
    use crate::test_support::{GAME_VERSION, TestGame, open, test_game};

    fn binding() -> SourceBindingDto {
        SourceBindingDto {
            sheet_name: "Synthetic".to_owned(),
            row_id: 42,
            subrow_id: 0,
            column_index: 0,
        }
    }

    fn initialize(state: &DesktopState, root: &Path, game: &TestGame) -> ProjectSummaryDto {
        initialize_with_game(
            state,
            root,
            open(game.path()),
            &root.join("../cache"),
            "fr".to_owned(),
        )
        .expect("initialize project")
    }

    fn open_project(state: &DesktopState, root: &Path, game_path: &Path) -> GameOpenOutcome {
        open_with_game(state, root, open(game_path), &root.join("../cache"), false)
            .expect("open project")
    }

    fn registry_path(directory: &Path) -> PathBuf {
        directory.join("app-data").join(REGISTRY_FILE_NAME)
    }

    #[test]
    fn a_quest_sheet_reads_as_a_scene_with_its_quest_name() {
        let directory = tempfile::tempdir().expect("directory");
        let game_folder = directory.path().join("game");
        FakeGame::new(GAME_VERSION)
            .with_text(
                "quest/001/ManFst004_00124",
                &TextSheet::new(2, &[0, 1])
                    .keyed(0)
                    .row(0, &[(0, "TEXT_MANFST004_00124_SEQ_00"), (1, "Journal")])
                    .row(1, &[(0, "TEXT_MANFST004_00124_TODO_00"), (1, "Speak")])
                    .row(
                        2,
                        &[(0, "TEXT_MANFST004_00124_MIOUNNE_000_1"), (1, "Welcome")],
                    )
                    .row(3, &[(0, "TEXT_MANFST004_00124_POP_MESSAGE"), (1, "Hint")]),
            )
            .with_text(
                "Quest",
                &TextSheet::new(2, &[0, 1])
                    .keyed(1)
                    .row(7, &[(0, "Close to Home"), (1, "ManFst004_00124")])
                    .row(8, &[(0, "Other"), (1, "Other_00001")]),
            )
            .with_text(
                "Synthetic",
                &TextSheet::new(1, &[0]).row(1, &[(0, "Plain")]),
            )
            .with_file(
                "game_script/quest/001/ManFst004_00124.luab",
                b"not a script".to_vec(),
            )
            .write(&game_folder)
            .expect("write game");
        let root = directory.path().join("repository");
        fs::create_dir_all(&root).expect("repository");
        let state = DesktopState::new();
        initialize_with_game(
            &state,
            &root,
            open(&game_folder),
            &directory.path().join("cache"),
            "fr".to_owned(),
        )
        .expect("initialize project");
        let quest_name = SourceBindingDto {
            sheet_name: "Quest".to_owned(),
            row_id: 7,
            subrow_id: 0,
            column_index: 0,
        };
        set_translation_target_with_state(&state, quest_name.clone(), "Comme à la maison")
            .expect("target");

        let scene = sheet_dialogue_with_state(&state, "quest/001/ManFst004_00124")
            .expect("readable")
            .expect("dialogue");
        assert_eq!(scene.kind, DialogueKindDto::Quest);
        let quest = scene.quest.expect("quest name");
        assert_eq!(quest.source_binding, quest_name);
        assert_eq!(quest.source_macro, "Close to Home");
        assert_eq!(quest.target_macro.as_deref(), Some("Comme à la maison"));
        let lines: Vec<(u32, DialogueRoleDto, Option<&str>, &str)> = scene
            .lines
            .iter()
            .map(|line| {
                (
                    line.source_binding.row_id,
                    line.role,
                    line.speaker.as_deref(),
                    line.key.as_str(),
                )
            })
            .collect();
        assert_eq!(
            lines,
            [
                (0, DialogueRoleDto::Journal, None, "SEQ_00"),
                (1, DialogueRoleDto::Objective, None, "TODO_00"),
                (2, DialogueRoleDto::Speech, Some("MIOUNNE"), "MIOUNNE_000_1"),
                (3, DialogueRoleDto::Other, None, "POP_MESSAGE"),
            ]
        );
        assert_eq!(scene.lines[2].source_binding.column_index, 1);
        assert_eq!(scene.lines[2].source_macro, "Welcome");
        assert_eq!(scene.scenes, None);
        assert!(
            scene
                .script_error
                .as_deref()
                .is_some_and(|error| error.contains("quest/001/ManFst004_00124")),
            "an unreadable script is reported, not hidden: {:?}",
            scene.script_error
        );
        assert_eq!(
            sheet_dialogue_with_state(&state, "Synthetic").expect("readable"),
            None
        );
    }

    #[test]
    fn a_new_state_has_no_project_and_closing_is_idempotent() {
        let state = DesktopState::new();
        assert_eq!(current_project_with_state(&state).expect("state"), None);
        assert_eq!(
            page_translation_rows_with_state(&state, "Synthetic", None, 1).expect_err("no project"),
            CommandError::no_project()
        );
        close_project_with_state(&state).expect("first close");
        close_project_with_state(&state).expect("second close");
    }

    #[test]
    fn the_desktop_lifecycle_reads_writes_and_reopens_a_project() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path().join("project");
        fs::create_dir_all(&root).expect("root");
        let game = test_game();
        let state = DesktopState::new();

        let summary = initialize(&state, &root, &game);
        assert_eq!(summary.source_language, "en");
        assert_eq!(summary.target_language, "fr");
        assert_eq!(summary.game_version, GAME_VERSION);
        assert!(
            summary.sheets.contains(&ProjectSheetDto {
                name: "Synthetic".to_owned(),
                row_count: 2,
                translatable_cell_count: 2,
                unavailable: false,
            }),
            "{:?}",
            summary.sheets
        );
        assert_eq!(
            current_project_with_state(&state).expect("current"),
            Some(summary)
        );

        let page = page_translation_rows_with_state(&state, "Synthetic", None, 10).expect("page");
        assert_eq!(page.rows[1].cells[0].source_macro, "Hello there");
        assert!(page.rows[1].cells[0].translation.is_none());
        assert_eq!(
            set_translation_note_with_state(&state, "not-an-id", None)
                .expect_err("invalid ID")
                .code,
            "invalidTranslationUnitId"
        );

        let id = set_translation_target_with_state(&state, binding(), "Bonjour")
            .expect("target")
            .translation_unit_id;
        set_translation_target_with_state(&state, binding(), "Salut").expect("edit");
        let noted =
            set_translation_note_with_state(&state, &id, Some("checked".to_owned())).expect("note");
        assert_eq!(noted.translator_note.as_deref(), Some("checked"));
        let reviewed =
            set_translation_review_state_with_state(&state, &id, ReviewStateDto::Reviewed)
                .expect("review");
        assert_eq!(reviewed.review_state, ReviewStateDto::Reviewed);
        assert_eq!(
            translation_progress_with_state(&state).expect("progress"),
            vec![SheetProgressDto {
                sheet_name: "Synthetic".to_owned(),
                translated: 1,
                reviewed: 1,
                needs_review: 0,
            }]
        );

        close_project_with_state(&state).expect("close");
        assert!(matches!(
            open_project(&state, &root, game.path()),
            GameOpenOutcome::Opened {
                source_update: None,
                ..
            }
        ));
        let page = page_translation_rows_with_state(&state, "Synthetic", None, 10).expect("page");
        let overlay = page.rows[1].cells[0].translation.as_ref().expect("overlay");
        assert_eq!(overlay.translation_unit_id, id);
        assert_eq!(overlay.target_macro, "Salut");
        assert_eq!(overlay.review_state, ReviewStateDto::Reviewed);
        assert_eq!(overlay.translator_note.as_deref(), Some("checked"));
    }

    #[test]
    fn a_newer_game_plans_the_update_without_writing_and_an_older_game_is_refused() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path().join("project");
        fs::create_dir_all(&root).expect("root");
        let game = test_game();
        let state = DesktopState::new();
        initialize(&state, &root, &game);
        set_translation_target_with_state(&state, binding(), "Bonjour").expect("target");
        close_project_with_state(&state).expect("close");
        let manifest = fs::read(root.join(".aeria/manifest.json")).expect("manifest");

        let newer = tempfile::tempdir().expect("newer game");
        FakeGame::new("2026.10.01.0000.0000")
            .with_text(
                "Synthetic",
                &TextSheet::new(2, &[0]).row(42, &[(0, "Hello there, friend")]),
            )
            .write(newer.path())
            .expect("write");
        let GameOpenOutcome::SourceUpdateRequired { report } =
            open_project(&state, &root, newer.path())
        else {
            panic!("an update is required");
        };
        assert_eq!(report.previous_game_version, GAME_VERSION);
        assert_eq!(report.source_changed, 1);
        assert_eq!(
            fs::read(root.join(".aeria/manifest.json")).expect("manifest"),
            manifest
        );
        assert_eq!(current_project_with_state(&state).expect("state"), None);

        let updated = open_with_game(
            &state,
            &root,
            open(newer.path()),
            &directory.path().join("cache"),
            true,
        )
        .expect("update");
        assert!(matches!(
            updated,
            GameOpenOutcome::Opened {
                source_update: Some(_),
                ..
            }
        ));
        close_project_with_state(&state).expect("close");

        let error = open_with_game(
            &state,
            &root,
            open(game.path()),
            &directory.path().join("cache"),
            false,
        )
        .err()
        .expect("an older game is refused");
        assert_eq!(error.code, "gameOutdated");
    }

    #[test]
    fn recent_projects_are_remembered_opened_and_forgotten() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path().join("project");
        fs::create_dir_all(&root).expect("root");
        let game = test_game();
        let state = DesktopState::new();
        let summary = initialize(&state, &root, &game);
        let path = registry_path(directory.path());
        let result = remember_project(&state, summary, Ok(path.clone()), None);
        assert!(result.warning.is_none());

        let id = ProjectRegistry::new(&path).load().expect("load")[0]
            .id
            .clone();
        let entry = recent_entry(&state, &path, &id).expect("entry");
        assert_eq!(entry.game_version, GAME_VERSION);
        assert_eq!(
            recent_repository(&entry).expect("repository"),
            PathBuf::from(&entry.repository_root)
        );
        assert_eq!(
            recent_entry(&state, &path, "missing")
                .expect_err("unknown")
                .code,
            "recentProjectNotFound"
        );
        let mut moved = entry.clone();
        moved.repository_root = directory
            .path()
            .join("moved")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            recent_repository(&moved).expect_err("missing").code,
            "recentProjectRepositoryMissing"
        );

        forget_recent_project_from_registry(&state, &path, &entry.id).expect("forget");
        assert!(ProjectRegistry::new(&path).load().expect("load").is_empty());
        assert!(current_project_with_state(&state).expect("state").is_some());
        assert!(root.join(".aeria").is_dir());
    }

    #[test]
    fn a_registry_write_failure_keeps_the_project_open_with_a_warning() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path().join("project");
        fs::create_dir_all(&root).expect("root");
        let game = test_game();
        let state = DesktopState::new();
        let summary = initialize(&state, &root, &game);
        let blocked = directory.path().join("not-a-directory");
        fs::write(&blocked, b"file").expect("blocker");
        let result = remember_project(
            &state,
            summary.clone(),
            Ok(blocked.join(REGISTRY_FILE_NAME)),
            None,
        );
        assert_eq!(result.project, summary);
        assert_eq!(
            result.warning.expect("warning").code,
            "projectRegistryWrite"
        );
        assert!(current_project_with_state(&state).expect("state").is_some());
    }

    #[test]
    fn new_project_folders_are_created_with_their_parents() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path().join("Проекты Aeria").join("новый");
        assert!(create_project_directory(&root).expect("create"));
        assert!(root.is_dir());
        assert!(!create_project_directory(&root).expect("existing"));
        assert_eq!(
            create_project_directory(Path::new(""))
                .expect_err("empty")
                .code,
            "invalidInput"
        );
    }

    #[test]
    fn mutation_commands_check_for_a_project_after_parsing_ids() {
        let state = DesktopState::new();
        assert_eq!(
            set_translation_note_with_state(&state, "not-an-id", None)
                .expect_err("no project")
                .code,
            "noProjectOpen"
        );
        assert_eq!(
            set_translation_review_state_with_state(&state, "x", ReviewStateDto::Reviewed)
                .expect_err("no project")
                .code,
            "noProjectOpen"
        );
        assert_eq!(
            set_translation_target_with_state(&state, binding(), "t")
                .expect_err("no project")
                .code,
            "noProjectOpen"
        );
        assert_eq!(
            parse_translation_unit_id("not-an-id")
                .expect_err("invalid")
                .code,
            "invalidTranslationUnitId"
        );
    }
}
