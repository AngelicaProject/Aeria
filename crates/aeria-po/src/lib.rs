//! The project as PO files: the format, the identity of an entry, where each
//! string's file is, files made from the installed game, and the join that
//! carries translations to a new game version.
//!
//! See `docs/architecture/po-project.md`.

pub mod generate;
pub mod identity;
pub mod merge;
pub mod po;
pub mod project;

pub use generate::{GenerateError, Languages, sheet_files};
pub use identity::{Identity, ROWS_PER_FILE, RowName, SheetPaths, is_scene, splits};
pub use merge::{merge, merge_files};
pub use po::{Entry, Header, PoFile, Problem, quote, unquote};
pub use project::{
    FORMAT, FileProblems, GAME_VERSION_FIELD, PO_DIR, ProjectError, README_PATH, SETTINGS_FILE,
    Settings, Updated, create, list, make, read, read_settings, update, versions, write,
    write_settings,
};
