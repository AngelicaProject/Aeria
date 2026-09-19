use std::str::FromStr;

use aeria_core::{ReviewState, SourceBinding, TranslationUnitId};
use aeria_workspace::ProjectSession;
use tauri::State;

use crate::dto::{
    ProjectSummaryDto, ReviewStateDto, SourceBindingDto, TranslationEntryPageDto,
    TranslationUnitIdDto,
};
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Opens an existing project and makes it the active desktop project.
///
/// # Errors
///
/// Returns a typed command error when source verification, workspace loading,
/// compatibility validation, or state locking fails.
pub fn open_project(
    state: State<'_, DesktopState>,
    repository_root: String,
    source_path: String,
) -> CommandResult<ProjectSummaryDto> {
    open_project_with_state(&state, repository_root, source_path)
}

pub(crate) fn open_project_with_state(
    state: &DesktopState,
    repository_root: String,
    source_path: String,
) -> CommandResult<ProjectSummaryDto> {
    let replacement =
        ProjectSession::open(repository_root, source_path).map_err(CommandError::from)?;
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
    state: State<'_, DesktopState>,
    repository_root: String,
    source_path: String,
    target_language: String,
) -> CommandResult<ProjectSummaryDto> {
    initialize_project_with_state(&state, repository_root, source_path, target_language)
}

pub(crate) fn initialize_project_with_state(
    state: &DesktopState,
    repository_root: String,
    source_path: String,
    target_language: String,
) -> CommandResult<ProjectSummaryDto> {
    let replacement = ProjectSession::initialize(repository_root, source_path, target_language)
        .map_err(CommandError::from)?;
    replace_project(state, replacement)
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
/// Reads one bounded page of source occurrences and workspace overlays.
///
/// # Errors
///
/// Returns a typed command error when no project is open, the page request is
/// invalid, the source cannot be read, or source integrity fails.
pub fn page_translation_entries(
    state: State<'_, DesktopState>,
    sheet_name: String,
    after: Option<SourceBindingDto>,
    limit: u32,
) -> CommandResult<TranslationEntryPageDto> {
    page_translation_entries_with_state(&state, &sheet_name, after, limit)
}

pub(crate) fn page_translation_entries_with_state(
    state: &DesktopState,
    sheet_name: &str,
    after: Option<SourceBindingDto>,
    limit: u32,
) -> CommandResult<TranslationEntryPageDto> {
    let after = after.map(SourceBinding::from);
    let project = state.lock_project()?;
    let project = project.as_ref().ok_or_else(CommandError::no_project)?;
    project
        .page_translation_entries(sheet_name, after.as_ref(), limit)
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
    let translation_unit_id = parse_translation_unit_id(translation_unit_id)?;
    let mut project = state.lock_project()?;
    let project = project.as_mut().ok_or_else(CommandError::no_project)?;
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
    let translation_unit_id = parse_translation_unit_id(translation_unit_id)?;
    let review_state: ReviewState = review_state.into();
    let mut project = state.lock_project()?;
    let project = project.as_mut().ok_or_else(CommandError::no_project)?;
    project
        .set_review_state(translation_unit_id, review_state)
        .map_err(CommandError::from)
}

fn parse_translation_unit_id(value: &str) -> CommandResult<TranslationUnitId> {
    TranslationUnitId::from_str(value).map_err(CommandError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{ReviewStateDto, SourceBindingDto};

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
            page_translation_entries_with_state(&state, "Synthetic", None, 1)
                .expect_err("no project"),
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
        assert_eq!(error.code, "invalidTranslationUnitId");

        let error = set_translation_review_state_with_state(
            &state,
            "tu1:0000000000000000000000000000000000000000000000000000000000000000",
            ReviewStateDto::Reviewed,
        )
        .expect_err("no project");
        assert_eq!(error.code, "noProjectOpen");

        let error =
            set_translation_target_with_state(&state, binding(), "target").expect_err("no project");
        assert_eq!(error.code, "noProjectOpen");
    }
}
