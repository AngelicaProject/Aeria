mod ai;
mod angelica;
mod commands;
mod dto;
mod error;
mod git;
mod state;

use serde::Serialize;
use tauri::Manager;

pub use ai::{
    AiConnectionCheckDto, AiProviderDto, AiProviderInputDto, AiProviderPresetDto, AiSettingsDto,
    ApiKeyStateDto, ChatGptLoginDto, ChatGptLoginEventDto, ai_chatgpt_login_cancel,
    ai_chatgpt_login_start, ai_clear_api_key, ai_list_remote_models, ai_remove_provider,
    ai_save_provider, ai_set_agent_model, ai_set_api_key, ai_settings, ai_test_connection,
};
pub use angelica::{
    AngelicaDraftDto, AngelicaEventDto, ConversationDto, ConversationSummaryDto,
    TranslationAppliedDto, angelica_apply_proposal, angelica_cancel, angelica_conversation,
    angelica_conversations, angelica_delete_conversation, angelica_draft, angelica_proposals,
    angelica_reject_proposal, angelica_send,
};
pub use commands::{
    cancel_source_package, close_project, current_project, forget_recent_project,
    initialize_project, initialize_project_from_game, list_detached_units, list_recent_projects,
    open_project, open_recent_project, page_translation_rows, preview_source_update,
    set_translation_note, set_translation_review_state, set_translation_target,
    start_source_package, translation_progress, update_project_from_game,
};
pub use dto::{
    DetachReasonDto, DetachedUnitDto, ProjectOpenResultDto, ProjectSheetDto, ProjectSummaryDto,
    RecentProjectAvailability, RecentProjectDto, ReviewStateDto, SheetProgressDto,
    SheetSchemaUpdateDto, SourceBindingDto, SourcePackageJobDto, SourceUpdateReportDto,
    TranslationCellDto, TranslationContextCellDto, TranslationOverlayDto, TranslationRowCursorDto,
    TranslationRowDto, TranslationRowPageDto, TranslationUnitIdDto,
};
pub use error::CommandError;
pub use git::{
    git_branches, git_checkpoint, git_clone_repository, git_commit_changes, git_contributors,
    git_create_branch, git_finish_contribution, git_initialize, git_log, git_overview,
    git_pending_changes, git_set_collaboration, git_set_identity, git_set_remote,
    git_switch_branch, git_sync, git_unit_attribution, git_unit_history,
};
pub use state::DesktopState;

#[derive(Serialize)]
struct AppInfo {
    name: &'static str,
    version: &'static str,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "Aeria",
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Starts the Aeria desktop application.
///
/// # Panics
///
/// Panics if Tauri cannot initialize the application runtime or load its
/// generated configuration.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(DesktopState::new())
        .setup(|app| {
            app.state::<DesktopState>()
                .set_git(git::resolve_git(app.handle()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            open_project,
            initialize_project,
            start_source_package,
            initialize_project_from_game,
            update_project_from_game,
            preview_source_update,
            list_detached_units,
            cancel_source_package,
            current_project,
            close_project,
            list_recent_projects,
            open_recent_project,
            forget_recent_project,
            page_translation_rows,
            set_translation_target,
            set_translation_note,
            set_translation_review_state,
            translation_progress,
            git_overview,
            git_initialize,
            git_set_identity,
            git_set_remote,
            git_pending_changes,
            git_checkpoint,
            git_log,
            git_commit_changes,
            git_unit_history,
            git_contributors,
            git_sync,
            git_clone_repository,
            git_unit_attribution,
            git_branches,
            git_create_branch,
            git_switch_branch,
            git_set_collaboration,
            git_finish_contribution,
            ai_settings,
            ai_save_provider,
            ai_remove_provider,
            ai_set_api_key,
            ai_clear_api_key,
            ai_set_agent_model,
            ai_list_remote_models,
            ai_test_connection,
            ai_chatgpt_login_start,
            ai_chatgpt_login_cancel,
            angelica_conversations,
            angelica_conversation,
            angelica_delete_conversation,
            angelica_cancel,
            angelica_send,
            angelica_proposals,
            angelica_apply_proposal,
            angelica_reject_proposal,
            angelica_draft
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Aeria desktop application");
}
