mod commands;
mod dto;
mod error;
mod state;

use serde::Serialize;

pub use commands::{
    close_project, current_project, initialize_project, open_project, page_translation_entries,
    set_translation_note, set_translation_review_state, set_translation_target,
};
pub use dto::{
    ProjectSheetDto, ProjectSummaryDto, ReviewStateDto, SourceBindingDto, TranslationEntryDto,
    TranslationEntryPageDto, TranslationOverlayDto, TranslationUnitIdDto,
};
pub use error::CommandError;
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
        .manage(DesktopState::new())
        .invoke_handler(tauri::generate_handler![
            app_info,
            open_project,
            initialize_project,
            current_project,
            close_project,
            page_translation_entries,
            set_translation_target,
            set_translation_note,
            set_translation_review_state
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Aeria desktop application");
}
