mod check_workflow;
mod commands;
mod dto;
mod error;
mod export;
mod fonts;
mod games;
mod git;
mod guide;
mod macros;
mod paths;
mod project_changes;
mod scene;
mod source;
mod state;
mod sync;
#[cfg(test)]
mod test_support;
mod translate;
mod updates;

use serde::Serialize;
use tauri::Manager;

pub use check_workflow::{
    CheckWorkflowDto, git_check_workflow, git_install_check_workflow, git_open_branch_settings,
};
pub use commands::{
    close_project, current_project, default_projects_directory_path, forget_recent_project,
    initialize_project_from_game, list_recent_projects, open_project_from_game,
    open_recent_project, page_translation_rows, set_project_target_language, set_translation_note,
    set_translation_target, sheet_dialogue, source_in_other_languages, translation_progress,
    update_project_from_game,
};
pub use dto::{
    GameOpenResultDto, OtherLanguageTextDto, ProjectOpenResultDto, ProjectSheetDto,
    ProjectSummaryDto, RecentProjectAvailability, RecentProjectDto, SheetProgressDto,
    SourceBindingDto, SourceUpdateNeededDto, SourceUpdateReportDto, TranslationCellDto,
    TranslationContextCellDto, TranslationOverlayDto, TranslationRowCursorDto, TranslationRowDto,
    TranslationRowPageDto,
};
pub use error::CommandError;
pub use export::{
    ExportOverviewDto, LocalExportDto, PackSettingsDto, PublishedReleaseDto, ReleaseInputDto,
    export_backup_key, export_generate_key, export_git_authors, export_import_key,
    export_install_workflow, export_overview, export_pack, export_publish, export_remove_key,
    export_save_settings,
};
pub use fonts::{
    FontsOverviewDto, fonts_import_file, fonts_overview, fonts_preview, fonts_save,
    fonts_use_recommended,
};
pub use games::{
    GameInstallationDto, GameOriginDto, GameSettingsDto, game_settings, set_game_path,
};
pub use git::{
    git_branches, git_checkpoint, git_clone_repository, git_commit_changes, git_create_branch,
    git_delete_branch, git_fetch, git_fetch_main, git_finish_contribution, git_initialize, git_log,
    git_merge_contribution, git_overview, git_pending_changes, git_pending_sheet_changes,
    git_project_changes, git_pull, git_push, git_remote_branches, git_remove_remote,
    git_set_identity, git_set_main_branch, git_set_remote, git_set_upstream, git_state_stamp,
    git_string_history, git_switch_branch, git_sync,
};
pub use guide::{
    ProjectKnowledgeDto, TermInput, project_knowledge, save_knowledge_style, save_knowledge_terms,
};
pub use state::{Activity, DesktopState};
pub use translate::{
    ModelAccountDto, ModelSignInDto, Translation, model_account, model_list,
    model_open_sign_in_page, model_sign_in_poll, model_sign_in_start, model_sign_out,
    translation_name_sheets, translation_start, translation_status, translation_stop,
};
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
        .manage(Translation::default())
        .setup(|app| {
            paths::migrate_legacy_directories(app.handle());
            app.state::<DesktopState>()
                .set_git(git::resolve_git(app.handle()));
            sync::start_file_watcher(app.handle());
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
            open_project_from_game,
            game_settings,
            set_game_path,
            default_projects_directory_path,
            initialize_project_from_game,
            update_project_from_game,
            current_project,
            close_project,
            list_recent_projects,
            open_recent_project,
            forget_recent_project,
            page_translation_rows,
            source_in_other_languages,
            sheet_dialogue,
            set_translation_target,
            set_translation_note,
            translation_progress,
            set_project_target_language,
            macros::macro_view,
            macros::macro_idioms,
            macros::macro_insertions,
            macros::game_glyph_font,
            macros::game_icon,
            git_overview,
            git_initialize,
            git_set_identity,
            git_set_remote,
            git_pending_changes,
            git_pending_sheet_changes,
            git_checkpoint,
            git_log,
            git_commit_changes,
            git_string_history,
            git_sync,
            git_fetch,
            git_pull,
            git_push,
            git_check_workflow,
            git_install_check_workflow,
            git_open_branch_settings,
            git_clone_repository,
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
            project_knowledge,
            save_knowledge_style,
            save_knowledge_terms,
            export_overview,
            export_save_settings,
            export_generate_key,
            export_import_key,
            export_backup_key,
            export_remove_key,
            export_install_workflow,
            export_git_authors,
            export_pack,
            export_publish,
            fonts_overview,
            fonts_use_recommended,
            fonts_save,
            fonts_import_file,
            fonts_preview,
            model_account,
            model_sign_in_start,
            model_open_sign_in_page,
            model_sign_in_poll,
            model_sign_out,
            model_list,
            translation_name_sheets,
            translation_start,
            translation_status,
            translation_stop
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Aeria desktop application");
}
