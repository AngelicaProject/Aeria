use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use aeria_atlas::{
    AtlasEvent, AtlasPackageRequest, AtlasPackageResult, AtlasPackageRunner, CancellationToken,
};
use aeria_core::{ReviewState, SourceBinding, TranslationUnitId};
use aeria_hsp::SourcePackage;
use aeria_workspace::ProjectSession;
use aeria_workspace::TranslationRowCursor;
use serde::Serialize;

use tauri::{Emitter, Manager, State};

use crate::dto::{
    ProjectSummaryDto, ReviewStateDto, SourceBindingDto, TranslationRowCursorDto,
    TranslationRowPageDto, TranslationUnitIdDto,
};
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

const SOURCE_PACKAGES_DIRECTORY: &str = "source-packages";
const STAGING_DIRECTORY: &str = "staging";
const STAGING_FILE: &str = "source.hsp";

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
) -> CommandResult<ProjectSummaryDto> {
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
    open_project_with_state(&state, repository_root, source_package_path, cache_root)
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
) -> CommandResult<ProjectSummaryDto> {
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
    initialize_project_with_state(
        &state,
        repository_root,
        source_package_path,
        cache_root,
        target_language,
    )
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

/// Generates a source package from a local game installation and initializes
/// the project from the already validated package.
///
/// # Errors
///
/// Returns a typed error when another Atlas job is active, Atlas cannot run,
/// package publication or validation fails, or workspace initialization fails.
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub async fn initialize_project_from_game(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    repository_root: String,
    game_path: String,
    source_language: String,
    target_language: String,
) -> CommandResult<ProjectSummaryDto> {
    let started = state.start_atlas_job()?;
    let job_id = started.id.clone();
    let token = started.token;
    let worker_job_id = job_id.clone();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        initialize_project_from_game_inner(
            &app,
            &worker_job_id,
            &token,
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
    state.finish_atlas_job(&job_id)?;
    result.and_then(|replacement| replace_project(&state, replacement))
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
) -> Result<ProjectSession, CommandError> {
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
                let _ = app_handle.emit("source-package-event", payload);
            },
            cancellation,
        )
        .map_err(CommandError::from)?;

    let source_package =
        publish_and_open_package(&result, &staging_path, &packages_root, &cache_root)?;
    if source_package.package_id() != result.package_id {
        let _ = fs::remove_file(source_package.package_path());
        return Err(CommandError::new(
            "atlasPackage",
            format!(
                "completed packageId {} does not match validated HSP packageId {}",
                result.package_id,
                source_package.package_id()
            ),
        ));
    }

    let replacement = ProjectSession::initialize_from_source_package(
        repository_root,
        source_package,
        target_language,
    )
    .map_err(CommandError::from)?;
    Ok(replacement)
}

fn resolve_atlas_executable(app: &tauri::AppHandle) -> CommandResult<PathBuf> {
    if let Ok(value) = env::var("AERIA_ATLAS_PATH") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
        return Err(CommandError::new(
            "atlasNotFound",
            format!(
                "AERIA_ATLAS_PATH does not point to a file: {}",
                path.display()
            ),
        ));
    }

    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|error| CommandError::new("atlasNotFound", error.to_string()))?;
    let target = if cfg!(target_os = "windows") {
        "x86_64-pc-windows-msvc"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let extension = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    let candidates = [
        resource_dir
            .join("binaries")
            .join(format!("harmonia-atlas-{target}{extension}")),
        resource_dir
            .join("binaries")
            .join(format!("harmonia-atlas{extension}")),
        resource_dir.join(format!("harmonia-atlas-{target}{extension}")),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            CommandError::new(
                "atlasNotFound",
                format!(
                    "the bundled Harmonia Atlas executable was not found under {}",
                    resource_dir.display()
                ),
            )
        })
}

fn publish_and_open_package(
    result: &AtlasPackageResult,
    staging_path: &Path,
    packages_root: &Path,
    cache_root: &Path,
) -> CommandResult<SourcePackage> {
    let package_hex = result
        .package_id
        .strip_prefix("sha256:")
        .ok_or_else(|| CommandError::new("atlasPackage", "Atlas packageId is not canonical"))?;
    let final_path = packages_root.join(format!("{package_hex}.hsp"));

    if final_path.exists() {
        if let Ok(existing) = SourcePackage::open(&final_path, cache_root)
            && existing.package_id() == result.package_id
        {
            fs::remove_file(staging_path)
                .map_err(|error| storage_error("remove reused staging package", &error))?;
            return Ok(existing);
        }
        replace_invalid_final(staging_path, &final_path)?;
    } else {
        fs::rename(staging_path, &final_path)
            .map_err(|error| storage_error("atomically publish source package", &error))?;
    }

    match SourcePackage::open(&final_path, cache_root) {
        Ok(source_package) if source_package.package_id() == result.package_id => {
            Ok(source_package)
        }
        Ok(source_package) => {
            let package_id = source_package.package_id().to_owned();
            let _ = fs::remove_file(&final_path);
            Err(CommandError::new(
                "atlasPackage",
                format!(
                    "validated HSP packageId {package_id} does not match Atlas packageId {}",
                    result.package_id
                ),
            ))
        }
        Err(error) => {
            let _ = fs::remove_file(&final_path);
            Err(CommandError::new("atlasPackage", error.to_string()))
        }
    }
}

fn replace_invalid_final(staging_path: &Path, final_path: &Path) -> CommandResult<()> {
    let backup_path = final_path.with_extension("hsp.invalid");
    if backup_path.exists() {
        fs::remove_file(&backup_path)
            .map_err(|error| storage_error("remove stale invalid package backup", &error))?;
    }
    fs::rename(final_path, &backup_path)
        .map_err(|error| storage_error("stage invalid package replacement", &error))?;
    match fs::rename(staging_path, final_path) {
        Ok(()) => {
            let _ = fs::remove_file(backup_path);
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup_path, final_path);
            Err(storage_error("publish replacement source package", &error))
        }
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
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::dto::{ProjectSheetDto, ReviewStateDto, SourceBindingDto};

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
        let reused = publish_and_open_package(&result, &staging, &existing, &cache).expect("reuse");
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
        let replacement =
            publish_and_open_package(&result, &staging, &existing, &cache).expect("replace");
        assert_eq!(replacement.package_id(), result.package_id);
        assert!(!staging.exists());
        assert!(final_path.exists());
        assert!(!final_path.with_extension("hsp.invalid").exists());
    }
}
