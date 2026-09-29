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
#[cfg(test)]
mod test_support;
mod updates;

use serde::Serialize;
use tauri::Manager;

pub use check_workflow::{
    CheckWorkflowDto, git_check_workflow, git_install_check_workflow, git_open_branch_settings,
};
pub use commands::{
    close_project, current_project, default_projects_directory_path, forget_recent_project,
    initialize_project_from_game, list_detached_units, list_recent_projects,
    open_project_from_game, open_recent_project, page_translation_rows, preview_source_update,
    set_project_target_language, set_translation_note, set_translation_review_state,
    set_translation_target, sheet_dialogue, source_in_other_languages, translation_progress,
    update_project_from_game,
};
pub use dto::{
    DetachReasonDto, DetachedUnitDto, GameOpenResultDto, OtherLanguageTextDto,
    ProjectOpenResultDto, ProjectSheetDto, ProjectSummaryDto, RecentProjectAvailability,
    RecentProjectDto, ReviewStateDto, SheetLayoutUpdateDto, SheetProgressDto, SourceBindingDto,
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
    git_create_branch, git_delete_branch, git_fetch, git_fetch_main, git_finish_contribution,
    git_initialize, git_log, git_merge_contribution, git_merge_driver, git_overview,
    git_pending_changes, git_project_changes, git_pull, git_push, git_remote_branches,
    git_remove_remote, git_set_identity, git_set_main_branch, git_set_merge_driver, git_set_remote,
    git_set_upstream, git_state_stamp, git_switch_branch, git_sync, git_unit_attribution,
    git_unit_history,
};
pub use guide::{
    ProjectKnowledgeDto, TermInput, project_knowledge, save_knowledge_characters,
    save_knowledge_style, save_knowledge_terms,
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

/// Handles the command-line modes that run without a window and returns
/// their exit code, or `None` to start the application.
///
/// `merge-driver BASE OURS THEIRS PATH` is Git's merge driver for unit
/// shards (`%O %A %B %P`): it merges per translation unit and exits 0 when
/// clean, 1 when units conflict (they are marked in the file), and 2 when a
/// version is not a valid shard, which leaves the local version for Git.
#[must_use]
pub fn run_command_line() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("merge-driver") {
        return None;
    }
    let [base, ours, theirs, path] = &args[1..] else {
        eprintln!("usage: aeria merge-driver BASE OURS THEIRS PATH");
        return Some(2);
    };
    Some(
        match aeria_git::run_merge_driver(base.as_ref(), ours.as_ref(), theirs.as_ref(), path) {
            Ok(0) => 0,
            Ok(conflicts) => {
                eprintln!(
                    "aeria: {conflicts} translation unit(s) in {path} changed differently on both sides"
                );
                1
            }
            Err(error) => {
                eprintln!("aeria: cannot merge {path} per unit: {error}");
                2
            }
        },
    )
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
            open_project_from_game,
            game_settings,
            set_game_path,
            default_projects_directory_path,
            initialize_project_from_game,
            update_project_from_game,
            preview_source_update,
            list_detached_units,
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
            set_translation_review_state,
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
            git_checkpoint,
            git_log,
            git_commit_changes,
            git_unit_history,
            git_contributors,
            git_sync,
            git_fetch,
            git_pull,
            git_push,
            git_merge_driver,
            git_set_merge_driver,
            git_check_workflow,
            git_install_check_workflow,
            git_open_branch_settings,
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
            project_knowledge,
            save_knowledge_style,
            save_knowledge_terms,
            save_knowledge_characters,
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
