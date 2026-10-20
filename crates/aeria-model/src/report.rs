//! What became of every string a run asked: its report, written record by
//! record as each is settled (see `docs/architecture/translate.md#report`).

use serde::Serialize;

use crate::run::Stop;

/// The version of the report's records.
pub const VERSION: u32 = 1;

/// What a run does.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RunKind {
    /// Translates the untranslated strings.
    #[default]
    Translating,
    /// Corrects translations.
    Fixing,
}

/// One answer for a string.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StringAttempt {
    pub translation: String,
    /// Every problem in English, as the model was told.
    pub problems: Vec<String>,
    /// The issues of the checks among `problems`, as data; an interface
    /// words them as it words the editor's.
    #[serde(skip)]
    pub issues: Vec<aeria_po::Issue>,
}

/// What became of a string.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StringOutcome {
    Written,
    /// The answer kept the translation as it was.
    Unchanged,
    /// The translation changed or was reviewed while the run asked, so the
    /// answer was not written.
    Stale,
    /// The last answer still failed the checks.
    Refused,
}

/// What became of one string.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StringRecord {
    /// The file, relative to `po/`.
    pub path: String,
    pub context: String,
    pub source: String,
    pub before: String,
    pub attempts: Vec<StringAttempt>,
    pub outcome: StringOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A record of a report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "record"
)]
pub enum Record {
    Run {
        version: u32,
        kind: RunKind,
        model: String,
        effort: Option<String>,
        /// The terms a correction follows, by headword; empty for every
        /// term or none.
        terms: Vec<String>,
        /// The name of the project's instruction a correction follows.
        instruction: Option<String>,
        /// Unix milliseconds.
        started_at: u64,
    },
    /// What the run was asked to do, second: so a run that stopped can be
    /// continued with the same options.
    Options {
        options: crate::run::Options,
        /// The id of the report of the run this one continues.
        continues: Option<String>,
    },
    String(StringRecord),
    /// A request the service refused; what it asked is left for the next
    /// run.
    Left {
        paths: Vec<String>,
        strings: usize,
        error: String,
    },
    Stop {
        stop: Stop,
    },
}

/// Where a run writes its report.
pub trait Report: Send {
    /// Writes one record.
    ///
    /// # Errors
    ///
    /// Returns the error of a report that cannot be written; the run goes on
    /// without it.
    fn write(&mut self, record: &Record) -> std::io::Result<()>;
}
