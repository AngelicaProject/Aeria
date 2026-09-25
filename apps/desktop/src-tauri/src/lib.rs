mod ai;
mod angelica;
mod commands;
mod dto;
mod error;
mod export;
mod fonts;
mod games;
mod git;
mod guide;
mod jobs;
mod paths;
mod project_changes;
mod search;
mod source_store;
mod state;
mod updates;
mod web;

use serde::Serialize;
use tauri::Manager;

pub use ai::{
    AiConnectionCheckDto, AiProviderDto, AiProviderInputDto, AiProviderPresetDto, AiSettingsDto,
    ApiKeyStateDto, ChatGptLoginDto, ChatGptLoginEventDto, ai_chatgpt_login_cancel,
    ai_chatgpt_login_start, ai_clear_api_key, ai_list_remote_models, ai_remove_provider,
    ai_save_provider, ai_set_agent_model, ai_set_api_key, ai_set_web_domains, ai_set_worker_model,
    ai_settings, ai_test_connection,
};
pub use angelica::{
    AngelicaDraftDto, AngelicaEventDto, ConversationDto, ConversationSummaryDto,
    TranslationAppliedDto, angelica_apply_proposal, angelica_cancel, angelica_conversation,
    angelica_conversations, angelica_delete_conversation, angelica_draft, angelica_proposals,
    angelica_reject_proposal, angelica_send,
};
pub use commands::{
    cancel_source_package, close_project, current_project, default_projects_directory_path,
    forget_recent_project, initialize_project, initialize_project_from_game, list_detached_units,
    list_recent_projects, open_project, open_project_from_game, open_recent_project,
    page_translation_rows, preview_source_update, set_translation_note,
    set_translation_review_state, set_translation_target, start_source_package,
    translation_progress, update_project_from_game,
};
pub use dto::{
    DetachReasonDto, DetachedUnitDto, GameOpenResultDto, ProjectOpenResultDto, ProjectSheetDto,
    ProjectSummaryDto, RecentProjectAvailability, RecentProjectDto, ReviewStateDto,
    SheetProgressDto, SheetSchemaUpdateDto, SourceBindingDto, SourcePackageJobDto,
    SourceUpdateReportDto, TranslationCellDto, TranslationContextCellDto, TranslationOverlayDto,
    TranslationRowCursorDto, TranslationRowDto, TranslationRowPageDto, TranslationUnitIdDto,
};
pub use error::CommandError;
pub use export::{
    ExportOverviewDto, LocalExportDto, PackSettingsDto, PublishedReleaseDto, ReleaseInputDto,
    export_backup_key, export_generate_key, export_import_key, export_install_workflow,
    export_overview, export_pack, export_publish, export_remove_key, export_save_settings,
};
pub use fonts::{
    FontsOverviewDto, fonts_import_file, fonts_overview, fonts_preview, fonts_save,
    fonts_use_recommended,
};
pub use games::{
    GameInstallationDto, GameOriginDto, GameSettingsDto, game_settings, set_game_path,
};
pub use git::{
    git_branches, git_checkpoint, git_clone_repository, git_commit_changes, git_contributors,
    git_create_branch, git_delete_branch, git_fetch_main, git_finish_contribution, git_initialize,
    git_log, git_merge_contribution, git_overview, git_pending_changes, git_project_changes,
    git_remote_branches, git_remove_remote, git_set_identity, git_set_main_branch, git_set_remote,
    git_set_upstream, git_state_stamp, git_switch_branch, git_sync, git_unit_attribution,
    git_unit_history,
};
pub use guide::{
    GlossaryEntryInput, ProjectGuideDto, project_guide, save_project_glossary,
    save_project_guidance,
};
pub use jobs::{
    angelica_job_control, angelica_job_events, angelica_job_retry, angelica_job_units,
    angelica_jobs,
};
pub use source_store::{
    SourceAvailabilityDto, SourcePackageEntryDto, delete_source_package, list_source_packages,
    reveal_source_packages, source_availability,
};
pub use state::{Activity, DesktopState};
pub use updates::{
    AvailableUpdateDto, UpdateChannel, UpdateDownloadDto, UpdateStatusDto, Updates, update_check,
    update_download, update_install, update_open_release, update_set_channel, update_status,
};

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
#[allow(clippy::too_many_lines)] // one registration list
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(DesktopState::new())
        .manage(Updates::default())
        .setup(|app| {
            paths::migrate_legacy_directories(app.handle());
            app.state::<DesktopState>()
                .set_git(git::resolve_git(app.handle()));
            updates::start_background_checks(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            update_status,
            update_check,
            update_set_channel,
            update_download,
            update_install,
            update_open_release,
            open_project,
            open_project_from_game,
            game_settings,
            set_game_path,
            list_source_packages,
            reveal_source_packages,
            delete_source_package,
            source_availability,
            default_projects_directory_path,
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
            git_set_main_branch,
            git_state_stamp,
            git_merge_contribution,
            git_delete_branch,
            git_project_changes,
            git_remove_remote,
            git_fetch_main,
            git_remote_branches,
            git_set_upstream,
            git_finish_contribution,
            ai_settings,
            ai_save_provider,
            ai_remove_provider,
            ai_set_api_key,
            ai_clear_api_key,
            ai_set_agent_model,
            ai_set_worker_model,
            ai_set_web_domains,
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
            angelica_draft,
            angelica_jobs,
            angelica_job_units,
            angelica_job_events,
            angelica_job_control,
            angelica_job_retry,
            project_guide,
            save_project_guidance,
            save_project_glossary,
            export_overview,
            export_save_settings,
            export_generate_key,
            export_import_key,
            export_backup_key,
            export_remove_key,
            export_install_workflow,
            export_pack,
            export_publish,
            fonts_overview,
            fonts_use_recommended,
            fonts_save,
            fonts_import_file,
            fonts_preview
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Aeria desktop application");
}
