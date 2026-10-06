//! The project as PO files: the format, the identity of an entry, where each
//! string's file is, files made from the installed game, and the join that
//! carries translations to a new game version.
//!
//! See `docs/architecture/po-project.md`.

pub mod candidates;
pub mod check;
pub mod generate;
pub mod identity;
pub mod length;
pub mod merge;
pub mod po;
pub mod project;
pub mod search;
pub mod session;

pub use candidates::{Candidate, Example, Rendering, term_candidates};
pub use check::{Finding, Issue, Verdict, check_file, check_translation};
pub use generate::{GenerateError, Languages, identity_keys, identity_of, is_entry, sheet_files};
pub use identity::{Identity, ROWS_PER_FILE, RowName, SheetPaths, is_scene, splits};
pub use merge::{merge, merge_files};
pub use po::{
    Entry, Header, PoFile, Problem, REVIEWED_FLAG, TERM_EXCEPTION_FLAG, can_be_exception,
    fingerprint, quote, unquote,
};
pub use project::{
    FORMAT, FileProblems, GAME_VERSION_FIELD, PO_DIR, ProjectError, READ_FORMATS, README_PATH,
    SETTINGS_FILE, Settings, Updated, create, list, make, read, read_settings, update, versions,
    write, write_settings,
};
pub use search::{
    Change, CheckFilter, Corpus, Field, FieldMatch, Fields, FileHits, Found, Hit, IssueCount,
    MAX_HITS, MatchKind, Matcher, Pattern, Query, Replacement, SearchError, State, path_selected,
    replace_in_text, text_ranges,
};
pub use session::{
    CellView, EditDone, EditError, EditKind, EditSkipped, EditsApplied, EntryEdit, EntryState,
    OpenError, Page, RowView, Session, SheetProgress, SkipReason, Translation, set_term_exception,
    write_atomically,
};
