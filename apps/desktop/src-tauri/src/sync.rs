//! Changes made to the open project's files outside the editor, by Git or by
//! hand: the editor reads a file again when it changed, and the renderer is
//! told which sheets it shows changed so it reloads them.

use tauri::{Emitter, Manager};

/// Renderer event with the names of sheets whose files changed outside the
/// editor.
pub const FILES_CHANGED_EVENT: &str = "project://files-changed";
/// Renderer event when the project's files changed in any way the session
/// knows of: a save, a bulk edit, machine translation, or a Git operation.
/// Views of the whole project, such as search, read them again.
pub const PROJECT_CHANGED_EVENT: &str = "project://changed";
/// How often the desktop looks for changed files.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);

/// Watches the files of the sheets the editor has shown and tells the
/// renderer when one changed.
pub(crate) fn start_file_watcher(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let mut revision = None;
        loop {
            std::thread::sleep(WATCH_INTERVAL);
            let state = app.state::<crate::state::DesktopState>();
            let Ok(session) = state.session() else {
                revision = None;
                continue;
            };
            let sheets = session.changed_sheets();
            let now = session.revision();
            if revision.is_some_and(|seen| seen != now) || !sheets.is_empty() {
                let _ = app.emit(PROJECT_CHANGED_EVENT, ());
            }
            revision = Some(now);
            if !sheets.is_empty() {
                let _ = app.emit(FILES_CHANGED_EVENT, sheets);
            }
        }
    });
}
