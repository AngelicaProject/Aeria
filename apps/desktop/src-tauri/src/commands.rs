use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aeria_git::GitRepository;
use aeria_po::{OpenError, Session};
use aeria_projects::{ProjectMetadata, ProjectRegistry, REGISTRY_FILE_NAME, RegistryEntry};
use aeria_source::GameSource;

use tauri::{Manager, State};

use crate::dto::{
    GameOpenResultDto, OtherLanguageTextDto, ProjectOpenResultDto, ProjectSummaryDto,
    RecentProjectDto, SheetProgressDto, SourceBindingDto, SourceUpdateNeededDto,
    SourceUpdateReportDto, TranslationOverlayDto, TranslationRowCursorDto, TranslationRowPageDto,
};
use crate::error::CommandError;
use crate::paths::AeriaPaths;
use crate::scene::{QuestNameDto, SheetDialogueDto};
use crate::source::{load_catalog_with_cache, open_game};
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// The most rows one page request reads.
pub(crate) const MAX_TRANSLATION_PAGE_SIZE: u32 = 256;

pub(crate) async fn run_blocking<T, F>(operation: F) -> CommandResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CommandResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| CommandError::internal_state(format!("desktop worker failed: {error}")))?
}

/// Threads that read the game when a project is made or updated.
fn threads() -> usize {
    std::thread::available_parallelism().map_or(2, std::num::NonZero::get)
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
    let settings = aeria_po::read_settings(repository_root)?;
    open_game(app, &settings.source_language)
}

/// Opens a project against `source`. Nothing is written: a project whose
/// files are for an older game version is reported for an update.
pub(crate) fn open_with_game(
    state: &DesktopState,
    repository_root: &Path,
    source: Arc<GameSource>,
    cache_root: &Path,
) -> CommandResult<GameOpenOutcome> {
    load_catalog_with_cache(cache_root, &source)?;
    match Session::open(repository_root, source) {
        Ok(session) => Ok(GameOpenOutcome::Opened {
            project: replace_project(state, session)?,
        }),
        Err(OpenError::UpdateRequired { project, game }) => {
            Ok(GameOpenOutcome::SourceUpdateRequired {
                update: SourceUpdateNeededDto {
                    previous_game_version: project,
                    game_version: game,
                },
            })
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) enum GameOpenOutcome {
    Opened { project: ProjectSummaryDto },
    SourceUpdateRequired { update: SourceUpdateNeededDto },
}

fn remembered_outcome(
    state: &DesktopState,
    outcome: GameOpenOutcome,
    registry_path: Result<PathBuf, String>,
) -> GameOpenResultDto {
    match outcome {
        GameOpenOutcome::Opened { project } => GameOpenResultDto::Opened {
            result: Box::new(remember_project(state, project, registry_path, None)),
        },
        GameOpenOutcome::SourceUpdateRequired { update } => {
            GameOpenResultDto::SourceUpdateRequired { update }
        }
    }
}

/// Opens an existing project with the configured game installation.
///
/// The project's source language is read from `aeria.json`. When the
/// project's files are for an older game version, nothing is written and the
/// versions are returned for confirmation; [`update_project_from_game`]
/// updates it.
///
/// # Errors
///
/// Returns a typed error when the settings cannot be read, no game
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
        let outcome = open_with_game(&state, &root, source, &cache_root)?;
        Ok(remembered_outcome(&state, outcome, registry_path))
    })
    .await
}

/// Brings a project's files to the installed game (see
/// `docs/architecture/po-project.md`) and records the update as one commit,
/// then opens the project. A project in a Git repository must have no
/// uncommitted changes in `po/` or `aeria.json` first.
pub(crate) fn update_with_game(
    state: &DesktopState,
    repository_root: &Path,
    source: Arc<GameSource>,
    cache_root: &Path,
) -> CommandResult<(ProjectSummaryDto, SourceUpdateReportDto)> {
    load_catalog_with_cache(cache_root, &source)?;
    let previous = aeria_po::read_settings(repository_root)?.game_version;
    let repository = GitRepository::discover(repository_root, state.git())?;
    if let Some(repository) = &repository {
        let status = repository.status()?;
        if status
            .files
            .iter()
            .any(|file| file.is_translation_data() || file.path == aeria_po::SETTINGS_FILE)
        {
            return Err(CommandError::new(
                "gitUncommittedTranslations",
                "commit the project's changes before updating it to the game",
            ));
        }
    }
    let game_version = source.version().to_string();
    let updated = aeria_po::update(repository_root, &source, threads())?;
    let commit = match &repository {
        Some(repository) if updated.files > 0 => repository
            .checkpoint(Some(&format!("Update to game version {game_version}")))
            .ok()
            .map(|outcome| outcome.commit.id),
        _ => None,
    };
    let session = Session::open(repository_root, source)?;
    let project = replace_project(state, session)?;
    Ok((
        project,
        SourceUpdateReportDto::new(previous, game_version, updated, commit),
    ))
}

/// Updates an existing project to the configured game installation and
/// opens it; see [`update_with_game`].
///
/// # Errors
///
/// Returns a typed error when the settings cannot be read, no game
/// installation is available, the project has uncommitted changes, or the
/// update fails.
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
        let (project, report) = update_with_game(&state, &root, source, &cache_root)?;
        Ok(remember_project(
            &state,
            project,
            registry_path,
            Some(report),
        ))
    })
    .await
}

/// Creates a project for the configured game installation, in a new or empty
/// folder, and makes it the active project.
///
/// # Errors
///
/// Returns a typed error when the folder cannot be created, no game
/// installation is available, the source language is unknown, or the
/// project cannot be written.
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
            initialize_with_game(&state, &root, source, &cache_root, &target_language)
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
    target_language: &str,
) -> CommandResult<ProjectSummaryDto> {
    if !aeria_core::is_target_language(target_language) {
        return Err(CommandError::new(
            "invalidTargetLanguage",
            format!("{target_language:?} is not a language tag of a target language"),
        ));
    }
    load_catalog_with_cache(cache_root, &source)?;
    let session = aeria_po::session::create(repository_root, source, target_language, threads())?;
    aeria_knowledge::create_empty(repository_root)
        .map_err(|message| CommandError::new("projectStore", message))?;
    replace_project(state, session)
}

/// Sets the open project's target language, the language it translates
/// into, and records it for Recent projects.
///
/// # Errors
///
/// Returns `invalidTargetLanguage` for a value that is not a BCP 47 language
/// tag or is `und`, `noProjectOpen` without a project, and a store error
/// when `aeria.json` cannot be written. A failed Recent projects update is
/// returned as the result's warning.
#[tauri::command(rename_all = "camelCase")]
pub async fn set_project_target_language(
    app: tauri::AppHandle,
    target_language: String,
) -> CommandResult<ProjectOpenResultDto> {
    let registry_path = app_registry_path(&app);
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let tag = target_language.trim();
        if !aeria_core::is_target_language(tag) {
            return Err(CommandError::new(
                "invalidTargetLanguage",
                format!("{tag:?} is not a language tag of a target language"),
            ));
        }
        let session = state.session()?;
        session.set_target_language(tag)?;
        let summary = ProjectSummaryDto::from_session(&session);
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
/// refreshes its cached metadata; with `accept_source_update` a project for
/// an older game version is updated first.
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
        if accept_source_update.unwrap_or(false) {
            let (project, report) = update_with_game(&state, &root, source, &cache_root)?;
            return Ok(GameOpenResultDto::Opened {
                result: Box::new(remember_project(
                    &state,
                    project,
                    Ok(registry_path),
                    Some(report),
                )),
            });
        }
        let outcome = open_with_game(&state, &root, source, &cache_root)?;
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

fn replace_project(state: &DesktopState, replacement: Session) -> CommandResult<ProjectSummaryDto> {
    let summary = ProjectSummaryDto::from_session(&replacement);
    *state.lock_project()? = Some(Arc::new(replacement));
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
    Ok(project
        .as_ref()
        .map(|session| ProjectSummaryDto::from_session(session)))
}

#[tauri::command(rename_all = "camelCase")]
/// Returns how much of each sheet of the active project is translated. The
/// first call reads every file of the project; later calls read only the
/// files that changed.
///
/// # Errors
///
/// Returns a typed command error when no project is open or a file cannot
/// be read.
pub async fn translation_progress(app: tauri::AppHandle) -> CommandResult<Vec<SheetProgressDto>> {
    run_blocking(move || translation_progress_with_state(&app.state::<DesktopState>())).await
}

pub(crate) fn translation_progress_with_state(
    state: &DesktopState,
) -> CommandResult<Vec<SheetProgressDto>> {
    Ok(state
        .session()?
        .progress()?
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
    *state.lock_project()? = None;
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Reads one bounded page of a sheet's rows with their translations.
///
/// # Errors
///
/// Returns a typed command error when no project is open, the page request is
/// invalid, or the game or a file cannot be read.
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
    if !(1..=MAX_TRANSLATION_PAGE_SIZE).contains(&limit) {
        return Err(CommandError::new(
            "translationRead",
            format!(
                "translation page limit {limit} must be between 1 and {MAX_TRANSLATION_PAGE_SIZE}"
            ),
        ));
    }
    if let Some(after) = &after
        && after.sheet_name != sheet_name
    {
        return Err(CommandError::new(
            "translationRead",
            format!(
                "the cursor belongs to sheet {:?}, not {sheet_name:?}",
                after.sheet_name
            ),
        ));
    }
    let session = state.session()?;
    let page = session.page(
        sheet_name,
        after.map(|after| (after.row_id, after.subrow_id)),
        limit as usize,
    )?;
    Ok(TranslationRowPageDto::new(sheet_name, page))
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
        let source = app.state::<DesktopState>().session()?.source_handle();
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
    let session = state.session()?;
    let source = session.source_handle();
    let Some(dialogue) = source.dialogue(sheet_name)? else {
        return Ok(None);
    };
    let quest = match source.quest_row(sheet_name)? {
        Some((row_id, subrow_id)) => {
            let page = session.page(
                QUEST_SHEET,
                match (row_id, subrow_id) {
                    (0, 0) => None,
                    (row, 0) => Some((row - 1, u16::MAX)),
                    (row, subrow) => Some((row, subrow - 1)),
                },
                1,
            )?;
            page.rows
                .into_iter()
                .find(|row| row.row == row_id && row.subrow == subrow_id)
                .and_then(|row| row.cells.into_iter().next())
                .map(|cell| QuestNameDto {
                    source_binding: SourceBindingDto {
                        sheet_name: QUEST_SHEET.to_owned(),
                        row_id,
                        subrow_id,
                        column_index: cell.column,
                    },
                    source_macro: cell.source,
                    target_macro: cell
                        .translation
                        .map(|translation| translation.text)
                        .filter(|text| !text.is_empty()),
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

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Sets the translation of one string; an empty text leaves it
/// untranslated. Returns what its entry holds afterwards.
///
/// # Errors
///
/// Returns `translationInvalid` with the problems when the checks refuse the
/// translation, or another typed command error when no project is open or
/// the string or its file cannot be written.
pub async fn set_translation_target(
    app: tauri::AppHandle,
    source_binding: SourceBindingDto,
    target_macro: String,
) -> CommandResult<Option<TranslationOverlayDto>> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        set_translation_target_with_state(&state, &source_binding, &target_macro)
    })
    .await
}

pub(crate) fn set_translation_target_with_state(
    state: &DesktopState,
    binding: &SourceBindingDto,
    target_macro: &str,
) -> CommandResult<Option<TranslationOverlayDto>> {
    Ok(state
        .session()?
        .set_translation(
            &binding.sheet_name,
            binding.row_id,
            binding.subrow_id,
            binding.column_index,
            target_macro,
        )?
        .map(Into::into))
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Replaces or clears the translator's note of one string.
///
/// # Errors
///
/// Returns a typed command error when no project is open or the string or
/// its file cannot be written.
pub async fn set_translation_note(
    app: tauri::AppHandle,
    source_binding: SourceBindingDto,
    note: Option<String>,
) -> CommandResult<Option<TranslationOverlayDto>> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        set_translation_note_with_state(&state, &source_binding, note.as_deref())
    })
    .await
}

pub(crate) fn set_translation_note_with_state(
    state: &DesktopState,
    binding: &SourceBindingDto,
    note: Option<&str>,
) -> CommandResult<Option<TranslationOverlayDto>> {
    Ok(state
        .session()?
        .set_note(
            &binding.sheet_name,
            binding.row_id,
            binding.subrow_id,
            binding.column_index,
            note,
        )?
        .map(Into::into))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use aeria_sqpack::testing::{FakeGame, TextSheet};

    use super::*;
    use crate::dto::ProjectSheetDto;
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
        initialize_with_game(state, root, open(game.path()), &root.join("../cache"), "fr")
            .expect("initialize project")
    }

    fn open_project(state: &DesktopState, root: &Path, game_path: &Path) -> GameOpenOutcome {
        open_with_game(state, root, open(game_path), &root.join("../cache")).expect("open project")
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
            "fr",
        )
        .expect("initialize project");
        let quest_name = SourceBindingDto {
            sheet_name: "Quest".to_owned(),
            row_id: 7,
            subrow_id: 0,
            column_index: 0,
        };
        set_translation_target_with_state(&state, &quest_name, "Comme à la maison")
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
        assert_eq!(
            set_translation_target_with_state(&state, &binding(), "t")
                .expect_err("no project")
                .code,
            "noProjectOpen"
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
        assert!(root.join("po/Synthetic.po").is_file());
        assert!(root.join("aeria-knowledge/terms.csv").is_file());
        assert_eq!(
            current_project_with_state(&state).expect("current"),
            Some(summary)
        );

        let page = page_translation_rows_with_state(&state, "Synthetic", None, 10).expect("page");
        assert_eq!(page.rows[1].cells[0].source_macro, "Hello there");
        assert!(page.rows[1].cells[0].translation.is_none());

        set_translation_target_with_state(&state, &binding(), "Bonjour").expect("target");
        set_translation_target_with_state(&state, &binding(), "Salut").expect("edit");
        let noted = set_translation_note_with_state(&state, &binding(), Some("checked"))
            .expect("note")
            .expect("overlay");
        assert_eq!(noted.translator_note.as_deref(), Some("checked"));
        assert_eq!(
            set_translation_target_with_state(&state, &binding(), "Salut <If(")
                .expect_err("invalid")
                .code,
            "translationInvalid"
        );
        assert_eq!(
            translation_progress_with_state(&state).expect("progress"),
            vec![
                SheetProgressDto {
                    sheet_name: "Addon".to_owned(),
                    strings: 2,
                    translated: 0,
                    fuzzy: 0,
                },
                SheetProgressDto {
                    sheet_name: "Synthetic".to_owned(),
                    strings: 2,
                    translated: 1,
                    fuzzy: 0,
                }
            ]
        );

        close_project_with_state(&state).expect("close");
        assert!(matches!(
            open_project(&state, &root, game.path()),
            GameOpenOutcome::Opened { .. }
        ));
        let page = page_translation_rows_with_state(&state, "Synthetic", None, 10).expect("page");
        let overlay = page.rows[1].cells[0].translation.as_ref().expect("overlay");
        assert_eq!(overlay.target_macro, "Salut");
        assert!(!overlay.fuzzy);
        assert_eq!(overlay.translator_note.as_deref(), Some("checked"));
    }

    #[test]
    fn a_newer_game_asks_for_an_update_and_an_older_game_is_refused() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path().join("project");
        fs::create_dir_all(&root).expect("root");
        let game = test_game();
        let state = DesktopState::new();
        initialize(&state, &root, &game);
        set_translation_target_with_state(&state, &binding(), "Bonjour").expect("target");
        close_project_with_state(&state).expect("close");
        let settings = fs::read(root.join("aeria.json")).expect("settings");

        let newer = tempfile::tempdir().expect("newer game");
        FakeGame::new("2026.10.01.0000.0000")
            .with_text(
                "Synthetic",
                &TextSheet::new(2, &[0]).row(42, &[(0, "Hello there, friend")]),
            )
            .write(newer.path())
            .expect("write");
        let GameOpenOutcome::SourceUpdateRequired { update } =
            open_project(&state, &root, newer.path())
        else {
            panic!("an update is required");
        };
        assert_eq!(update.previous_game_version, GAME_VERSION);
        assert_eq!(
            fs::read(root.join("aeria.json")).expect("settings"),
            settings
        );
        assert_eq!(current_project_with_state(&state).expect("state"), None);

        let (project, report) = update_with_game(
            &state,
            &root,
            open(newer.path()),
            &directory.path().join("cache"),
        )
        .expect("update");
        assert_eq!(project.game_version, "2026.10.01.0000.0000");
        assert_eq!(report.fuzzy, 1);
        let page = page_translation_rows_with_state(&state, "Synthetic", None, 10).expect("page");
        let overlay = page.rows[0].cells[0].translation.as_ref().expect("kept");
        assert_eq!(overlay.target_macro, "Bonjour");
        assert!(overlay.fuzzy);
        close_project_with_state(&state).expect("close");

        let error = open_with_game(
            &state,
            &root,
            open(game.path()),
            &directory.path().join("cache"),
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
        assert!(root.join("po").is_dir());
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
}
