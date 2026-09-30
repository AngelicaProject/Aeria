//! Changes made to the open project's files outside the editor, by Git or by
//! hand: the editor reads a file again when it changed, and the renderer is
//! told which sheets it shows changed so it reloads them.

use tauri::{Emitter, Manager};

/// Renderer event with the names of sheets whose files changed outside the
/// editor.
pub const FILES_CHANGED_EVENT: &str = "project://files-changed";
/// How often the desktop looks for changed files.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);

/// Watches the files of the sheets the editor has shown and tells the
/// renderer when one changed.
pub(crate) fn start_file_watcher(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(WATCH_INTERVAL);
            let state = app.state::<crate::state::DesktopState>();
            let Ok(session) = state.session() else {
                continue;
            };
            let sheets = session.changed_sheets();
            if !sheets.is_empty() {
                let _ = app.emit(FILES_CHANGED_EVENT, sheets);
            }
        }
    });
}
