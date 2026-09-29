//! Translation jobs: local, resumable state for translating many strings.
//!
//! A job fixes its list of strings when it starts, groups them into chunks,
//! and records each string's outcome. Worker subagents translate one chunk
//! at a time; the desktop schedules them. The state is machine-local SQLite
//! in application data and is never written to a project repository.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::chat::Usage;
use crate::images::ImageRef;
use crate::settings::ModelSelection;
use crate::tools::{ReviewLabel, UnitLocation, UnitState};

/// Most strings in one chunk.
pub const CHUNK_UNITS: usize = 30;
/// Most strings in one chunk of a quest or cutscene sheet: the localizer
/// translates a whole scene at once, in parallel parts.
pub const DIALOGUE_CHUNK_UNITS: usize = crate::localizer::MAX_UNIT_LINES;
/// Most source characters in one chunk of a sheet that is not dialogue.
pub const CHUNK_SOURCE_CHARS: usize = 12_000;
/// Most workers one job runs at once.
pub const MAX_CONCURRENCY: u8 = 16;
/// Workers a job runs when Angelica does not choose.
pub const DEFAULT_CONCURRENCY: u8 = 8;
/// Chunks from which a job runs [`MAX_CONCURRENCY`] workers by default.
pub const LARGE_JOB_CHUNKS: u64 = 100;

/// Workers for a job of `chunks` chunks: `requested` or, without it,
/// [`MAX_CONCURRENCY`] for large jobs and [`DEFAULT_CONCURRENCY`] otherwise;
/// never more than the chunks, since each worker translates one at a time.
#[must_use]
pub fn job_concurrency(requested: Option<u8>, chunks: u64) -> u8 {
    let default = if chunks >= LARGE_JOB_CHUNKS {
        MAX_CONCURRENCY
    } else {
        DEFAULT_CONCURRENCY
    };
    let wanted = requested.unwrap_or(default).clamp(1, MAX_CONCURRENCY);
    u8::try_from(chunks.clamp(1, u64::from(wanted))).unwrap_or(wanted)
}
/// Most strings one job may cover.
pub const MAX_JOB_UNITS: usize = 1_000_000;

/// Which strings of the scope a job translates.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobFilter {
    /// Strings without a translation.
    Untranslated,
    /// Translations marked as needing review, retranslated.
    NeedsReview,
    /// Untranslated strings and existing drafts, which are retranslated.
    UntranslatedAndDrafts,
    /// The strings of the scope's list that no person has settled: those
    /// without a reviewed translation, and reviewed ones whose translation
    /// is still the one a job wrote. They are translated again, for example
    /// after a term or a character's profile changed.
    Revise,
}

/// The strings a job covers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobScope {
    /// Sheets to cover; empty means every sheet with translatable strings.
    pub sheets: Vec<String>,
    pub filter: JobFilter,
    /// Only these strings, when the list is not empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<UnitLocation>,
    /// Sheets whose names match one of these patterns (`*` for any text,
    /// ignoring case), besides `sheets`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    /// Sheets whose names match one of these patterns are left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

impl JobScope {
    /// A scope of sheets and a filter, without patterns or a list.
    #[must_use]
    pub fn sheets(sheets: Vec<String>, filter: JobFilter) -> Self {
        Self {
            sheets,
            filter,
            units: Vec::new(),
            patterns: Vec::new(),
            exclude: Vec::new(),
        }
    }

    /// Whether the scope names no sheets and no patterns: every sheet.
    #[must_use]
    pub fn covers_everything(&self) -> bool {
        self.sheets.is_empty() && self.patterns.is_empty()
    }

    /// Whether a sheet of the project belongs to the scope by name.
    #[must_use]
    pub fn includes_sheet(&self, sheet: &str) -> bool {
        let named = self.covers_everything()
            || self.sheets.iter().any(|known| known == sheet)
            || self
                .patterns
                .iter()
                .any(|pattern| sheet_matches(pattern, sheet));
        named
            && !self
                .exclude
                .iter()
                .any(|pattern| sheet_matches(pattern, sheet))
    }
}

/// Whether a sheet name matches a pattern where `*` stands for any text,
/// ignoring case; a pattern without `*` must be the whole name.
#[must_use]
pub fn sheet_matches(pattern: &str, sheet: &str) -> bool {
    let pattern = pattern.trim().to_lowercase();
    let sheet = sheet.to_lowercase();
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return sheet == pattern;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !sheet.starts_with(first) || sheet.len() < first.len() + last.len() || !sheet.ends_with(last)
    {
        return false;
    }
    let mut rest = &sheet[first.len()..sheet.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(found) => rest = &rest[found + part.len()..],
            None => return false,
        }
    }
    true
}

/// What the user approved when starting a job.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobSpec {
    pub scope: JobScope,
    /// Instructions from Angelica and the user for every chunk.
    pub instructions: String,
    /// The worker model.
    pub model: ModelSelection,
    /// The job pauses when its tokens exceed this.
    pub token_limit: u64,
    /// Chunks translated at the same time.
    pub concurrency: u8,
    /// Images of the job's conversation sent to every worker.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageRef>,
    /// How much work the localizer spends on each unit.
    #[serde(default)]
    pub quality: JobQuality,
}

/// How much work the localizer spends on each unit of a job.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobQuality {
    /// Parts written in parallel and one recheck of what changed: the
    /// default, fast enough for the whole game.
    #[default]
    Fast,
    /// One writer for the whole unit, critics at the highest effort, and two
    /// full rechecks: for story quests whose scenes must hold together.
    Careful,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobStatus {
    Running,
    Paused,
    Completed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UnitStatus {
    Pending,
    /// Claimed by a running worker.
    Running,
    /// Written as a draft by an earlier version of Aeria.
    Drafted,
    /// Written as a final translation: the critics left nothing open.
    Finished,
    /// Written, but a critic's major finding is still open; the string
    /// needs review and the message says why.
    Flagged,
    /// The worker's translation broke the structure after its corrections.
    Rejected,
    /// The worker did not translate the string or a provider request failed.
    Failed,
    /// The string changed after the job read it; nothing was written.
    Conflict,
}

impl UnitStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Drafted => "drafted",
            Self::Finished => "finished",
            Self::Flagged => "flagged",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
            Self::Conflict => "conflict",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "drafted" => Self::Drafted,
            "finished" => Self::Finished,
            "flagged" => Self::Flagged,
            "rejected" => Self::Rejected,
            "failed" => Self::Failed,
            "conflict" => Self::Conflict,
            _ => Self::Pending,
        }
    }
}

impl JobStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            _ => Self::Paused,
        }
    }
}

/// Counts of a job's strings by status.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobCounts {
    pub total: u64,
    pub pending: u64,
    pub running: u64,
    pub drafted: u64,
    #[serde(default)]
    pub finished: u64,
    #[serde(default)]
    pub flagged: u64,
    pub rejected: u64,
    pub failed: u64,
    pub conflict: u64,
}

impl JobCounts {
    /// Strings with a final outcome.
    #[must_use]
    pub const fn processed(&self) -> u64 {
        self.drafted + self.finished + self.flagged + self.rejected + self.failed + self.conflict
    }
}

/// A job as listed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSummary {
    pub id: String,
    pub conversation_id: String,
    pub status: JobStatus,
    /// Why a paused job paused.
    pub reason: Option<String>,
    pub spec: JobSpec,
    pub created_at_unix_ms: u64,
    pub counts: JobCounts,
    /// Chunks being translated right now, one per busy worker.
    pub active_workers: u64,
    pub usage: Usage,
    /// Chunks of the job.
    pub chunks: u64,
    /// Chunks with no pending or running strings.
    pub finished_chunks: u64,
    /// Tokens the whole job will likely use: the tokens used so far plus
    /// the average per finished chunk for each unfinished one. `None` until
    /// a chunk finished.
    pub projected_tokens: Option<u64>,
}

impl JobSummary {
    #[must_use]
    pub const fn used_tokens(&self) -> u64 {
        self.usage.prompt_tokens + self.usage.completion_tokens
    }
}

/// Tokens a job will likely use, from its average per finished chunk.
#[must_use]
pub fn project_tokens(used: u64, chunks: u64, finished_chunks: u64) -> Option<u64> {
    if finished_chunks == 0 {
        return None;
    }
    let remaining = chunks.saturating_sub(finished_chunks);
    Some(used.saturating_add(used / finished_chunks * remaining))
}

/// One string of a job.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobUnit {
    pub seq: u64,
    pub chunk: u64,
    pub location: UnitLocation,
    pub status: UnitStatus,
    pub attempts: u32,
    pub message: Option<String>,
    /// The string's state when the job started; writes are compare-and-set
    /// against it.
    pub expected: UnitState,
}

/// Something a worker or the orchestrator reports for Angelica.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub seq: u64,
    pub created_at_unix_ms: u64,
    pub kind: String,
    pub message: String,
    pub location: Option<UnitLocation>,
}

/// A critic's finding on a line a job translated, kept for learning.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// The critic that raised it, such as `blind` or `player`.
    pub role: String,
    pub major: bool,
    pub problem: String,
    pub location: Option<UnitLocation>,
    pub source: String,
    /// The final translation of the line.
    pub target: String,
}

/// A translation a job wrote, the latest one per string.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Written {
    pub location: UnitLocation,
    pub job_id: String,
    pub created_at_unix_ms: u64,
    pub source: String,
    pub target: String,
}

/// A string selected for a job, with its source size for chunking.
#[derive(Clone, Debug)]
pub struct ScopedUnit {
    pub location: UnitLocation,
    pub expected: UnitState,
    pub source_chars: usize,
}

fn is_dialogue(sheet: &str) -> bool {
    sheet.starts_with("quest/") || sheet.starts_with("cut_scene/")
}

/// The most strings a chunk of `sheet` may hold when the job has `count` of
/// the sheet's strings in a row. A quest or cutscene sheet is one scene, so
/// it is one chunk up to [`DIALOGUE_CHUNK_UNITS`] strings; a larger one is
/// split evenly, so no chunk is left with a few stray lines.
fn chunk_unit_limit(sheet: &str, count: usize) -> usize {
    if is_dialogue(sheet) {
        count.div_ceil(count.div_ceil(DIALOGUE_CHUNK_UNITS).max(1))
    } else {
        CHUNK_UNITS
    }
}

/// Chunk numbers for units in order: a new chunk starts at a new sheet or
/// when a chunk would exceed its string limit (see [`chunk_unit_limit`]) or,
/// outside quest and cutscene sheets, [`CHUNK_SOURCE_CHARS`] source
/// characters.
#[must_use]
pub fn assign_chunks(units: &[ScopedUnit]) -> Vec<u64> {
    let mut chunks = Vec::with_capacity(units.len());
    let mut chunk = 0_u64;
    let (mut chunk_units, mut chunk_chars) = (0_usize, 0_usize);
    let mut limit = CHUNK_UNITS;
    for (index, unit) in units.iter().enumerate() {
        let sheet = &unit.location.sheet;
        let new_sheet = index == 0 || units[index - 1].location.sheet != *sheet;
        if new_sheet {
            let count = units[index..]
                .iter()
                .take_while(|next| next.location.sheet == *sheet)
                .count();
            limit = chunk_unit_limit(sheet, count);
        }
        if chunk_units > 0
            && (new_sheet
                || chunk_units >= limit
                || (!is_dialogue(sheet) && chunk_chars + unit.source_chars > CHUNK_SOURCE_CHARS))
        {
            chunk += 1;
            chunk_units = 0;
            chunk_chars = 0;
        }
        chunk_units += 1;
        chunk_chars += unit.source_chars;
        chunks.push(chunk);
    }
    chunks
}

/// Approximate size of a job, shown before it starts.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobEstimate {
    pub units: u64,
    pub chunks: u64,
    /// A rough token estimate. Providers count differently.
    pub estimated_tokens: u64,
    /// The average tokens per chunk of earlier jobs with the same model,
    /// when the estimate is based on them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_chunk_tokens: Option<u64>,
}

/// Tokens a chunk costs besides its strings in the estimate: the
/// localizer's instructions and project knowledge, sent with each of its
/// requests. Measured on quests with the localizer.
const CHUNK_OVERHEAD_TOKENS: u64 = 50_000;
/// Tokens per source character in the estimate: the script with the other
/// languages, read by the contract, the writers, and the critics, and the
/// translation written and fixed. Measured on quests with the localizer.
const TOKENS_PER_SOURCE_CHAR: u64 = 13;
/// Finished chunks of earlier jobs needed before their average replaces
/// the formula.
pub const HISTORY_MIN_CHUNKS: u64 = 3;
/// Earlier jobs the history average reads.
const HISTORY_JOBS: usize = 20;

impl JobEstimate {
    /// Estimates a job over `units`. `history_chunk_tokens` is the average
    /// per chunk of earlier jobs with the same model; without it the
    /// estimate is [`CHUNK_OVERHEAD_TOKENS`] per chunk plus
    /// [`TOKENS_PER_SOURCE_CHAR`] per source character.
    #[must_use]
    pub fn for_units(units: &[ScopedUnit], history_chunk_tokens: Option<u64>) -> Self {
        let chunks = assign_chunks(units).last().map_or(0, |last| last + 1);
        let characters: u64 = units.iter().map(|unit| unit.source_chars as u64).sum();
        let formula = chunks * CHUNK_OVERHEAD_TOKENS + characters * TOKENS_PER_SOURCE_CHAR;
        Self {
            units: units.len() as u64,
            chunks,
            estimated_tokens: history_chunk_tokens.map_or(formula, |per_chunk| chunks * per_chunk),
            history_chunk_tokens,
        }
    }

    /// The estimate for a job of `quality`: a careful job writes each unit
    /// in one piece and rechecks it twice with stronger critics, about twice
    /// the tokens.
    #[must_use]
    pub fn with_quality(mut self, quality: JobQuality) -> Self {
        if quality == JobQuality::Careful {
            self.estimated_tokens = self.estimated_tokens.saturating_mul(2);
        }
        self
    }

    /// The token limit a job over this estimate starts with.
    #[must_use]
    pub fn token_limit(&self) -> u64 {
        (self.estimated_tokens * 2).max(200_000)
    }
}

/// A job Angelica proposed and the user has not started yet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobProposal {
    pub scope: JobScope,
    pub instructions: String,
    /// Images of the conversation for every worker.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageRef>,
    pub concurrency: u8,
    pub estimate: JobEstimate,
    pub token_limit: u64,
    #[serde(default)]
    pub quality: JobQuality,
}

/// A higher token limit for a job, proposed by Angelica.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobLimitProposal {
    pub job_id: String,
    pub token_limit: u64,
    /// The job's limit, tokens used, and projection when proposed.
    pub previous_limit: u64,
    pub used_tokens: u64,
    pub projected_tokens: Option<u64>,
}

/// Job storage errors.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("job storage failed: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("job {0:?} was not found")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn review_str(label: Option<ReviewLabel>) -> Option<&'static str> {
    label.map(|label| match label {
        ReviewLabel::Draft => "draft",
        ReviewLabel::NeedsReview => "needsReview",
        ReviewLabel::Reviewed => "reviewed",
    })
}

fn review_parse(value: Option<&str>) -> Option<ReviewLabel> {
    match value {
        Some("draft") => Some(ReviewLabel::Draft),
        Some("needsReview") => Some(ReviewLabel::NeedsReview),
        Some("reviewed") => Some(ReviewLabel::Reviewed),
        _ => None,
    }
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    status TEXT NOT NULL,
    reason TEXT,
    spec TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS job_units (
    job_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    chunk INTEGER NOT NULL,
    sheet TEXT NOT NULL,
    row_id INTEGER NOT NULL,
    subrow_id INTEGER NOT NULL,
    column_index INTEGER NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    message TEXT,
    expected_target TEXT,
    expected_review TEXT,
    PRIMARY KEY (job_id, seq)
);
CREATE INDEX IF NOT EXISTS job_units_by_status ON job_units (job_id, status, chunk);
CREATE TABLE IF NOT EXISTS job_events (
    job_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    created_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    message TEXT NOT NULL,
    location TEXT,
    PRIMARY KEY (job_id, seq)
);
CREATE TABLE IF NOT EXISTS job_findings (
    job_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    created_ms INTEGER NOT NULL,
    role TEXT NOT NULL,
    major INTEGER NOT NULL,
    problem TEXT NOT NULL,
    location TEXT,
    source TEXT NOT NULL,
    target TEXT NOT NULL,
    PRIMARY KEY (job_id, seq)
);
CREATE TABLE IF NOT EXISTS job_written (
    location TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    source TEXT NOT NULL,
    target TEXT NOT NULL
);
";

/// The jobs of one project: one SQLite file in application data.
#[derive(Clone, Debug)]
pub struct JobStore {
    path: PathBuf,
}

impl JobStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn open(&self) -> Result<Connection, JobError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                JobError::Invalid(format!("cannot create {}: {error}", parent.display()))
            })?;
        }
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(std::time::Duration::from_secs(10))?;
        connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        connection.execute_batch(SCHEMA)?;
        Ok(connection)
    }

    /// Creates a running job over `units`, grouped into chunks by sheet and
    /// order, at most [`CHUNK_UNITS`] strings and [`CHUNK_SOURCE_CHARS`]
    /// source characters each.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or oversized job or a storage failure.
    pub fn create(
        &self,
        conversation_id: &str,
        spec: &JobSpec,
        units: &[ScopedUnit],
    ) -> Result<JobSummary, JobError> {
        if units.is_empty() {
            return Err(JobError::Invalid(
                "the job has no strings to translate".to_owned(),
            ));
        }
        if units.len() > MAX_JOB_UNITS {
            return Err(JobError::Invalid(format!(
                "a job may cover at most {MAX_JOB_UNITS} strings"
            )));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut connection = self.open()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO jobs (id, conversation_id, status, spec, created_ms) VALUES (?1, ?2, 'running', ?3, ?4)",
            params![
                id,
                conversation_id,
                serde_json::to_string(spec).map_err(|error| JobError::Invalid(error.to_string()))?,
                to_i64(now_unix_ms())
            ],
        )?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO job_units (job_id, seq, chunk, sheet, row_id, subrow_id, column_index, status, expected_target, expected_review) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?9)",
            )?;
            for (seq, (unit, chunk)) in units.iter().zip(assign_chunks(units)).enumerate() {
                insert.execute(params![
                    id,
                    to_i64(seq as u64),
                    to_i64(chunk),
                    unit.location.sheet,
                    unit.location.row,
                    unit.location.subrow,
                    unit.location.column.unwrap_or(0),
                    unit.expected.target,
                    review_str(unit.expected.review_state),
                ])?;
            }
        }
        transaction.commit()?;
        self.summary(&id)
    }

    /// Lists jobs, newest first.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn list(&self) -> Result<Vec<JobSummary>, JobError> {
        let connection = self.open()?;
        let ids: Vec<String> = connection
            .prepare("SELECT id FROM jobs ORDER BY created_ms DESC LIMIT 100")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        ids.iter().map(|id| self.summary(id)).collect()
    }

    /// Returns one job with its counts.
    ///
    /// # Errors
    ///
    /// Returns [`JobError::NotFound`] or a storage error.
    pub fn summary(&self, id: &str) -> Result<JobSummary, JobError> {
        let connection = self.open()?;
        let row = connection
            .query_row(
                "SELECT conversation_id, status, reason, spec, created_ms, prompt_tokens, completion_tokens FROM jobs WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| JobError::NotFound(id.to_owned()))?;
        let mut counts = JobCounts::default();
        let mut statement = connection
            .prepare("SELECT status, COUNT(*) FROM job_units WHERE job_id = ?1 GROUP BY status")?;
        for entry in statement.query_map(params![id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })? {
            let (status, count) = entry?;
            let count = to_u64(count);
            counts.total += count;
            match UnitStatus::parse(&status) {
                UnitStatus::Pending => counts.pending += count,
                UnitStatus::Running => counts.running += count,
                UnitStatus::Drafted => counts.drafted += count,
                UnitStatus::Finished => counts.finished += count,
                UnitStatus::Flagged => counts.flagged += count,
                UnitStatus::Rejected => counts.rejected += count,
                UnitStatus::Failed => counts.failed += count,
                UnitStatus::Conflict => counts.conflict += count,
            }
        }
        let active_workers: i64 = connection.query_row(
            "SELECT COUNT(DISTINCT chunk) FROM job_units WHERE job_id = ?1 AND status = 'running'",
            params![id],
            |row| row.get(0),
        )?;
        let (chunks, unfinished_chunks): (i64, i64) = connection.query_row(
            "SELECT COUNT(DISTINCT chunk), COUNT(DISTINCT CASE WHEN status IN ('pending', 'running') THEN chunk END) FROM job_units WHERE job_id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let chunks = to_u64(chunks);
        let finished_chunks = chunks.saturating_sub(to_u64(unfinished_chunks));
        let usage = Usage {
            prompt_tokens: to_u64(row.5),
            completion_tokens: to_u64(row.6),
        };
        Ok(JobSummary {
            active_workers: to_u64(active_workers),
            id: id.to_owned(),
            conversation_id: row.0,
            status: JobStatus::parse(&row.1),
            reason: row.2,
            spec: serde_json::from_str(&row.3)
                .map_err(|error| JobError::Invalid(error.to_string()))?,
            created_at_unix_ms: to_u64(row.4),
            counts,
            projected_tokens: project_tokens(
                usage.prompt_tokens + usage.completion_tokens,
                chunks,
                finished_chunks,
            ),
            usage,
            chunks,
            finished_chunks,
        })
    }

    /// Claims the next chunk with pending strings, marking them running.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn claim_chunk(&self, id: &str) -> Result<Option<Vec<JobUnit>>, JobError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let chunk: Option<i64> = transaction.query_row(
            "SELECT MIN(chunk) FROM job_units WHERE job_id = ?1 AND status = 'pending'",
            params![id],
            |row| row.get(0),
        )?;
        let Some(chunk) = chunk else {
            return Ok(None);
        };
        transaction.execute(
            "UPDATE job_units SET status = 'running', attempts = attempts + 1 WHERE job_id = ?1 AND chunk = ?2 AND status = 'pending'",
            params![id, chunk],
        )?;
        let units = Self::read_units(
            &transaction,
            id,
            "AND chunk = ?2 AND status = 'running'",
            chunk,
            CHUNK_UNITS as u64 * 4,
        )?;
        transaction.commit()?;
        Ok(Some(units))
    }

    fn read_units(
        connection: &Connection,
        id: &str,
        condition: &str,
        value: i64,
        limit: u64,
    ) -> Result<Vec<JobUnit>, JobError> {
        let mut statement = connection.prepare(&format!(
            "SELECT seq, chunk, sheet, row_id, subrow_id, column_index, status, attempts, message, expected_target, expected_review \
             FROM job_units WHERE job_id = ?1 {condition} ORDER BY seq LIMIT {limit}"
        ))?;
        let units = statement
            .query_map(params![id, value], |row| {
                Ok(JobUnit {
                    seq: to_u64(row.get(0)?),
                    chunk: to_u64(row.get(1)?),
                    location: UnitLocation {
                        sheet: row.get(2)?,
                        row: row.get(3)?,
                        subrow: row.get(4)?,
                        column: Some(row.get(5)?),
                    },
                    status: UnitStatus::parse(&row.get::<_, String>(6)?),
                    attempts: row.get(7)?,
                    message: row.get(8)?,
                    expected: UnitState {
                        target: row.get(9)?,
                        review_state: review_parse(row.get::<_, Option<String>>(10)?.as_deref()),
                    },
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(units)
    }

    /// Lists a job's strings with one of `statuses`, in order.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn units(
        &self,
        id: &str,
        statuses: &[UnitStatus],
        offset: u64,
        limit: u64,
    ) -> Result<Vec<JobUnit>, JobError> {
        let connection = self.open()?;
        let list = statuses
            .iter()
            .map(|status| format!("'{}'", status.as_str()))
            .collect::<Vec<_>>()
            .join(",");
        let condition = format!("AND status IN ({list}) AND seq >= ?2");
        Self::read_units(
            &connection,
            id,
            &condition,
            to_i64(offset),
            limit.min(10_000),
        )
    }

    /// Records the outcome of strings.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn finish_units(
        &self,
        id: &str,
        outcomes: &[(u64, UnitStatus, Option<String>)],
    ) -> Result<(), JobError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut update = transaction.prepare(
                "UPDATE job_units SET status = ?3, message = ?4 WHERE job_id = ?1 AND seq = ?2",
            )?;
            for (seq, status, message) in outcomes {
                update.execute(params![id, to_i64(*seq), status.as_str(), message])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Adds provider usage to a job.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn add_usage(&self, id: &str, usage: Usage) -> Result<(), JobError> {
        self.open()?.execute(
            "UPDATE jobs SET prompt_tokens = prompt_tokens + ?2, completion_tokens = completion_tokens + ?3 WHERE id = ?1",
            params![id, to_i64(usage.prompt_tokens), to_i64(usage.completion_tokens)],
        )?;
        Ok(())
    }

    /// Changes a job's status. Pausing, completing, or cancelling returns
    /// claimed strings to pending.
    ///
    /// # Errors
    ///
    /// Returns [`JobError::NotFound`] or a storage error.
    pub fn set_status(
        &self,
        id: &str,
        status: JobStatus,
        reason: Option<&str>,
    ) -> Result<(), JobError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE jobs SET status = ?2, reason = ?3 WHERE id = ?1",
            params![id, status.as_str(), reason],
        )?;
        if changed == 0 {
            return Err(JobError::NotFound(id.to_owned()));
        }
        if status != JobStatus::Running {
            transaction.execute(
                "UPDATE job_units SET status = 'pending' WHERE job_id = ?1 AND status = 'running'",
                params![id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Changes how many workers a job that was not cancelled runs. A running
    /// job's runner follows the change: extra workers stop after their
    /// current chunk, and new ones start.
    ///
    /// # Errors
    ///
    /// Returns [`JobError::NotFound`], [`JobError::Invalid`] for a cancelled
    /// job, or a storage error.
    pub fn set_concurrency(&self, id: &str, concurrency: u8) -> Result<JobSpec, JobError> {
        let job = self.summary(id)?;
        if job.status == JobStatus::Cancelled {
            return Err(JobError::Invalid("the job was cancelled".to_owned()));
        }
        let mut spec = job.spec;
        spec.concurrency = concurrency.clamp(1, MAX_CONCURRENCY);
        self.open()?.execute(
            "UPDATE jobs SET spec = ?2 WHERE id = ?1",
            params![
                id,
                serde_json::to_string(&spec)
                    .map_err(|error| JobError::Invalid(error.to_string()))?
            ],
        )?;
        Ok(spec)
    }

    /// Appends instructions used by chunks that start afterwards.
    ///
    /// # Errors
    ///
    /// Returns [`JobError::NotFound`] or a storage error.
    pub fn amend(&self, id: &str, instructions: &str) -> Result<JobSpec, JobError> {
        let mut spec = self.summary(id)?.spec;
        if !spec.instructions.is_empty() {
            spec.instructions.push_str("\n\n");
        }
        spec.instructions.push_str(instructions.trim());
        self.open()?.execute(
            "UPDATE jobs SET spec = ?2 WHERE id = ?1",
            params![
                id,
                serde_json::to_string(&spec)
                    .map_err(|error| JobError::Invalid(error.to_string()))?
            ],
        )?;
        Ok(spec)
    }

    /// Changes the token limit of a job that was not cancelled.
    ///
    /// # Errors
    ///
    /// Returns [`JobError::NotFound`], [`JobError::Invalid`] for a cancelled
    /// job or a limit not above the tokens already used, or a storage error.
    pub fn set_token_limit(&self, id: &str, token_limit: u64) -> Result<JobSpec, JobError> {
        let job = self.summary(id)?;
        if job.status == JobStatus::Cancelled {
            return Err(JobError::Invalid("the job was cancelled".to_owned()));
        }
        if token_limit <= job.used_tokens() {
            return Err(JobError::Invalid(format!(
                "the limit must be above the {} tokens already used",
                job.used_tokens()
            )));
        }
        let mut spec = job.spec;
        spec.token_limit = token_limit;
        self.open()?.execute(
            "UPDATE jobs SET spec = ?2 WHERE id = ?1",
            params![
                id,
                serde_json::to_string(&spec)
                    .map_err(|error| JobError::Invalid(error.to_string()))?
            ],
        )?;
        Ok(spec)
    }

    /// The average tokens per finished chunk of the latest jobs with the
    /// same model and effort, once they finished at least
    /// [`HISTORY_MIN_CHUNKS`] chunks together.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn history_chunk_tokens(&self, model: &ModelSelection) -> Result<Option<u64>, JobError> {
        let (tokens, chunks) = self
            .list()?
            .iter()
            .filter(|job| job.spec.model == *model && job.finished_chunks > 0)
            .take(HISTORY_JOBS)
            .fold((0_u64, 0_u64), |(tokens, chunks), job| {
                (tokens + job.used_tokens(), chunks + job.finished_chunks)
            });
        Ok((chunks >= HISTORY_MIN_CHUNKS).then(|| tokens / chunks))
    }

    /// Returns strings with the given final statuses to pending and returns
    /// how many were requeued.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn requeue(&self, id: &str, statuses: &[UnitStatus]) -> Result<u64, JobError> {
        let list = statuses
            .iter()
            .filter(|status| {
                !matches!(
                    status,
                    UnitStatus::Pending
                        | UnitStatus::Running
                        | UnitStatus::Drafted
                        | UnitStatus::Finished
                        | UnitStatus::Flagged
                )
            })
            .map(|status| format!("'{}'", status.as_str()))
            .collect::<Vec<_>>()
            .join(",");
        if list.is_empty() {
            return Ok(0);
        }
        let changed = self.open()?.execute(
            &format!("UPDATE job_units SET status = 'pending', message = NULL WHERE job_id = ?1 AND status IN ({list})"),
            params![id],
        )?;
        Ok(changed as u64)
    }

    /// The sheets of a job's strings, in the order of their first string.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn sheets(&self, id: &str) -> Result<Vec<String>, JobError> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT sheet FROM job_units WHERE job_id = ?1 GROUP BY sheet ORDER BY MIN(seq)",
        )?;
        let sheets = statement
            .query_map(params![id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(sheets)
    }

    /// Whether a job has an event of a kind.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn has_event(&self, id: &str, kind: &str) -> Result<bool, JobError> {
        let found: i64 = self.open()?.query_row(
            "SELECT COUNT(*) FROM job_events WHERE job_id = ?1 AND kind = ?2",
            params![id, kind],
            |row| row.get(0),
        )?;
        Ok(found > 0)
    }

    /// Records an event for Angelica.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn add_event(
        &self,
        id: &str,
        kind: &str,
        message: &str,
        location: Option<&UnitLocation>,
    ) -> Result<(), JobError> {
        // One statement, so concurrent workers never take the same number.
        self.open()?.execute(
            "INSERT INTO job_events (job_id, seq, created_ms, kind, message, location)              SELECT ?1, COALESCE(MAX(seq), 0) + 1, ?2, ?3, ?4, ?5 FROM job_events WHERE job_id = ?1",
            params![
                id,
                to_i64(now_unix_ms()),
                kind,
                message,
                location.map(|location| serde_json::to_string(location).unwrap_or_default()),
            ],
        )?;
        Ok(())
    }

    /// Records critics' findings on a job's lines.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn add_findings(&self, id: &str, findings: &[Finding]) -> Result<(), JobError> {
        if findings.is_empty() {
            return Ok(());
        }
        let mut connection = self.open()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let next: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM job_findings WHERE job_id = ?1",
                params![id],
                |row| row.get(0),
            )?;
            let mut insert = transaction.prepare(
                "INSERT INTO job_findings (job_id, seq, created_ms, role, major, problem, location, source, target) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            let now = to_i64(now_unix_ms());
            for (offset, finding) in findings.iter().enumerate() {
                insert.execute(params![
                    id,
                    next + i64::try_from(offset).unwrap_or(i64::MAX) + 1,
                    now,
                    finding.role,
                    finding.major,
                    finding.problem,
                    finding
                        .location
                        .as_ref()
                        .map(|location| serde_json::to_string(location).unwrap_or_default()),
                    finding.source,
                    finding.target,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// A job's findings, in the order they were recorded.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn findings(&self, id: &str) -> Result<Vec<Finding>, JobError> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT role, major, problem, location, source, target FROM job_findings WHERE job_id = ?1 ORDER BY seq",
        )?;
        let findings = statement
            .query_map(params![id], |row| {
                Ok(Finding {
                    role: row.get(0)?,
                    major: row.get(1)?,
                    problem: row.get(2)?,
                    location: row
                        .get::<_, Option<String>>(3)?
                        .and_then(|text| serde_json::from_str(&text).ok()),
                    source: row.get(4)?,
                    target: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(findings)
    }

    /// Records translations a job wrote, replacing earlier records of the
    /// same strings.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn add_written(
        &self,
        id: &str,
        written: &[(UnitLocation, String, String)],
    ) -> Result<(), JobError> {
        if written.is_empty() {
            return Ok(());
        }
        let mut connection = self.open()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut upsert = transaction.prepare(
                "INSERT INTO job_written (location, job_id, created_ms, source, target) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(location) DO UPDATE SET job_id = excluded.job_id, created_ms = excluded.created_ms, source = excluded.source, target = excluded.target",
            )?;
            let now = to_i64(now_unix_ms());
            for (location, source, target) in written {
                upsert.execute(params![
                    serde_json::to_string(location).unwrap_or_default(),
                    id,
                    now,
                    source,
                    target,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// The translations jobs wrote, newest first, at most `limit`.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn written(&self, limit: usize) -> Result<Vec<Written>, JobError> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT location, job_id, created_ms, source, target FROM job_written ORDER BY created_ms DESC LIMIT ?1",
        )?;
        let written = statement
            .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter_map(|(location, job_id, created, source, target)| {
                Some(Written {
                    location: serde_json::from_str(&location).ok()?,
                    job_id,
                    created_at_unix_ms: to_u64(created),
                    source,
                    target,
                })
            })
            .collect();
        Ok(written)
    }

    /// The translation a job last wrote for a string, if any.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn written_target(&self, location: &UnitLocation) -> Result<Option<String>, JobError> {
        Ok(self
            .open()?
            .query_row(
                "SELECT target FROM job_written WHERE location = ?1",
                params![serde_json::to_string(location).unwrap_or_default()],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Lists events after `after_seq`, at most 200.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn events(&self, id: &str, after_seq: u64) -> Result<Vec<JobEvent>, JobError> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT seq, created_ms, kind, message, location FROM job_events WHERE job_id = ?1 AND seq > ?2 ORDER BY seq LIMIT 200",
        )?;
        let events = statement
            .query_map(params![id, to_i64(after_seq)], |row| {
                Ok(JobEvent {
                    seq: to_u64(row.get(0)?),
                    created_at_unix_ms: to_u64(row.get(1)?),
                    kind: row.get(2)?,
                    message: row.get(3)?,
                    location: row
                        .get::<_, Option<String>>(4)?
                        .and_then(|text| serde_json::from_str(&text).ok()),
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(events)
    }

    /// Marks jobs left running by a previous run of Aeria as paused, since
    /// their workers are gone. Returns their IDs.
    ///
    /// # Errors
    ///
    /// Returns a storage error.
    pub fn pause_interrupted(&self) -> Result<Vec<String>, JobError> {
        let connection = self.open()?;
        let ids: Vec<String> = connection
            .prepare("SELECT id FROM jobs WHERE status = 'running'")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        for id in &ids {
            self.set_status(
                id,
                JobStatus::Paused,
                Some("Aeria was closed while the job ran"),
            )?;
        }
        Ok(ids)
    }

    /// Removes a job that no longer runs, with its strings and events.
    /// Drafts it wrote stay in the project.
    ///
    /// # Errors
    ///
    /// Returns [`JobError::NotFound`], [`JobError::Invalid`] for a running
    /// job, or a storage error.
    pub fn remove(&self, id: &str) -> Result<(), JobError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let status: String = transaction
            .query_row(
                "SELECT status FROM jobs WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| JobError::NotFound(id.to_owned()))?;
        if JobStatus::parse(&status) == JobStatus::Running {
            return Err(JobError::Invalid(
                "pause or cancel the job first".to_owned(),
            ));
        }
        transaction.execute("DELETE FROM job_units WHERE job_id = ?1", params![id])?;
        transaction.execute("DELETE FROM job_events WHERE job_id = ?1", params![id])?;
        // What the job wrote stays: it tells later jobs which translations
        // agents wrote and which a person changed since.
        transaction.execute("DELETE FROM job_findings WHERE job_id = ?1", params![id])?;
        transaction.execute("DELETE FROM jobs WHERE id = ?1", params![id])?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(sheet: &str, row: u32, chars: usize) -> ScopedUnit {
        ScopedUnit {
            location: UnitLocation {
                sheet: sheet.to_owned(),
                row,
                subrow: 0,
                column: Some(0),
            },
            expected: UnitState {
                target: None,
                review_state: None,
            },
            source_chars: chars,
        }
    }

    fn spec() -> JobSpec {
        JobSpec {
            scope: JobScope::sheets(vec!["Item".to_owned()], JobFilter::Untranslated),
            instructions: "Be brief.".to_owned(),
            model: ModelSelection {
                provider_id: "p".to_owned(),
                model_id: "m".to_owned(),
                effort: None,
            },
            token_limit: 1000,
            concurrency: 2,
            images: Vec::new(),
            quality: JobQuality::Fast,
        }
    }

    fn store() -> (tempfile::TempDir, JobStore) {
        let directory = tempfile::tempdir().expect("directory");
        let store = JobStore::new(directory.path().join("jobs").join("project.sqlite3"));
        (directory, store)
    }

    #[test]
    fn parallel_workers_claim_each_chunk_once() {
        let (_directory, store) = store();
        let units: Vec<_> = (0..400).map(|row| unit("Item", row, 10)).collect();
        let chunks = assign_chunks(&units).into_iter().max().expect("chunks") + 1;
        let job = store.create("c", &spec(), &units).expect("job");
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let store = store.clone();
                let id = job.id.clone();
                std::thread::spawn(move || {
                    let mut claimed = Vec::new();
                    while let Some(units) = store.claim_chunk(&id).expect("claim") {
                        let chunk = units[0].chunk;
                        let outcomes: Vec<_> = units
                            .iter()
                            .map(|unit| (unit.seq, UnitStatus::Drafted, None))
                            .collect();
                        store.finish_units(&id, &outcomes).expect("finish");
                        store.add_event(&id, "issue", "note", None).expect("event");
                        claimed.push(chunk);
                    }
                    claimed
                })
            })
            .collect();
        let mut claimed: Vec<u64> = workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("worker"))
            .collect();
        claimed.sort_unstable();
        assert_eq!(claimed, (0..chunks).collect::<Vec<_>>());
        let summary = store.summary(&job.id).expect("summary");
        assert_eq!(summary.counts.drafted, 400);
        assert_eq!(
            store.events(&job.id, 0).expect("events").len(),
            claimed.len()
        );
    }

    #[test]
    fn only_a_job_that_no_longer_runs_is_removed() {
        let (_directory, store) = store();
        let units: Vec<_> = (0..3).map(|row| unit("Item", row, 10)).collect();
        let job = store.create("c", &spec(), &units).expect("job");
        store
            .add_event(&job.id, "issue", "note", None)
            .expect("event");
        assert!(matches!(store.remove(&job.id), Err(JobError::Invalid(_))));

        store
            .set_status(&job.id, JobStatus::Paused, Some("paused"))
            .expect("pause");
        store.remove(&job.id).expect("remove");
        assert!(matches!(store.summary(&job.id), Err(JobError::NotFound(_))));
        assert!(store.list().expect("list").is_empty());
        assert!(store.events(&job.id, 0).expect("events").is_empty());
        assert!(matches!(store.remove(&job.id), Err(JobError::NotFound(_))));
    }

    #[test]
    fn chunks_split_by_sheet_size_and_count() {
        let (_directory, store) = store();
        let items = u32::try_from(CHUNK_UNITS).expect("chunk size") + 5;
        let mut units: Vec<ScopedUnit> = (0..items).map(|row| unit("Item", row, 10)).collect();
        units.push(unit("Action", 1, 10));
        units.push(unit("Action", 2, CHUNK_SOURCE_CHARS));
        let job = store.create("c1", &spec(), &units).expect("job");
        assert_eq!(job.counts.total, u64::from(items) + 2);
        assert_eq!(job.status, JobStatus::Running);

        let first = store.claim_chunk(&job.id).expect("claim").expect("chunk");
        assert_eq!(first.len(), CHUNK_UNITS);
        assert!(
            first
                .iter()
                .all(|unit| unit.status == UnitStatus::Running && unit.attempts == 1)
        );
        let second = store.claim_chunk(&job.id).expect("claim").expect("chunk");
        assert_eq!(second.len(), 5);
        let third = store.claim_chunk(&job.id).expect("claim").expect("chunk");
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].location.sheet, "Action");
        let fourth = store.claim_chunk(&job.id).expect("claim").expect("chunk");
        assert_eq!(fourth[0].location.row, 2);
        assert!(store.claim_chunk(&job.id).expect("claim").is_none());
    }

    #[test]
    fn scopes_take_sheets_by_name_pattern_and_leave_out_exclusions() {
        assert!(sheet_matches("quest/*", "quest/000/ManFst004_00124"));
        assert!(sheet_matches("quest/*/man*", "quest/000/ManFst004_00124"));
        assert!(!sheet_matches("quest/*/man*", "quest/002/SubSea923_00246"));
        assert!(sheet_matches("*Name", "BNpcName"));
        assert!(!sheet_matches("Item", "ItemUICategory"));
        assert!(sheet_matches("item*", "ItemUICategory"));
        let scope = JobScope {
            patterns: vec!["quest/*".to_owned()],
            exclude: vec!["quest/*/Cls*".to_owned()],
            ..JobScope::sheets(vec!["Addon".to_owned()], JobFilter::Untranslated)
        };
        assert!(scope.includes_sheet("quest/002/SubSea923_00246"));
        assert!(!scope.includes_sheet("quest/002/ClsCul021_00254"));
        assert!(scope.includes_sheet("Addon"));
        assert!(!scope.includes_sheet("Item"));
        assert!(JobScope::sheets(Vec::new(), JobFilter::Untranslated).includes_sheet("Item"));
    }

    #[test]
    fn findings_and_written_translations_are_kept_for_learning() {
        let directory = tempfile::tempdir().expect("temp");
        let store = JobStore::new(directory.path().join("jobs.sqlite3"));
        let job = store
            .create("c1", &spec(), &[unit("Item", 1, 10)])
            .expect("job");
        let location = UnitLocation {
            sheet: "Item".to_owned(),
            row: 1,
            subrow: 0,
            column: Some(0),
        };
        let finding = Finding {
            role: "blind".to_owned(),
            major: true,
            problem: "Reads as a translation.".to_owned(),
            location: Some(location.clone()),
            source: "Potion".to_owned(),
            target: "Зелье".to_owned(),
        };
        store
            .add_findings(&job.id, &[finding.clone(), finding.clone()])
            .expect("findings");
        assert_eq!(store.findings(&job.id).expect("read").len(), 2);

        store
            .add_written(
                &job.id,
                &[(location.clone(), "Potion".to_owned(), "Зелье".to_owned())],
            )
            .expect("written");
        store
            .add_written(
                &job.id,
                &[(location.clone(), "Potion".to_owned(), "Эликсир".to_owned())],
            )
            .expect("rewritten");
        assert_eq!(
            store.written_target(&location).expect("target").as_deref(),
            Some("Эликсир")
        );
        assert_eq!(store.written(10).expect("all").len(), 1);

        store
            .set_status(&job.id, JobStatus::Cancelled, None)
            .expect("cancel");
        store.remove(&job.id).expect("remove");
        assert!(store.findings(&job.id).expect("gone").is_empty());
        assert_eq!(store.written(10).expect("kept").len(), 1);
    }

    #[test]
    fn dialogue_sheets_are_whole_chunks_and_long_ones_split_evenly() {
        let quest = "quest/001/ManFst004_00124";
        // Long source lines do not split a scene.
        let mut units: Vec<ScopedUnit> = (0..70).map(|row| unit(quest, row, 1_000)).collect();
        units.extend((0..300).map(|row| unit("cut_scene/024/VoiceMan_02400", row, 10)));
        let chunks = assign_chunks(&units);
        let sizes: Vec<usize> = (0..=chunks[chunks.len() - 1])
            .map(|chunk| chunks.iter().filter(|&&c| c == chunk).count())
            .collect();
        assert_eq!(sizes, [70, 150, 150]);
    }

    #[test]
    fn estimates_count_units_chunks_and_tokens() {
        let count = u32::try_from(CHUNK_UNITS).expect("chunk size") + 1;
        let units: Vec<ScopedUnit> = (0..count).map(|row| unit("Item", row, 30)).collect();
        let estimate = JobEstimate::for_units(&units, None);
        assert_eq!(estimate.units, u64::from(count));
        assert_eq!(estimate.chunks, 2);
        assert_eq!(
            estimate.estimated_tokens,
            2 * CHUNK_OVERHEAD_TOKENS + u64::from(count) * 30 * TOKENS_PER_SOURCE_CHAR
        );
        assert_eq!(estimate.history_chunk_tokens, None);
        assert_eq!(JobEstimate::for_units(&[], None).chunks, 0);

        let history = JobEstimate::for_units(&units, Some(25_000));
        assert_eq!(history.estimated_tokens, 50_000);
        assert_eq!(history.history_chunk_tokens, Some(25_000));
        assert_eq!(history.token_limit(), 200_000, "the limit has a floor");
    }

    #[test]
    fn projection_and_history_follow_finished_chunks() {
        assert_eq!(project_tokens(1000, 10, 0), None);
        assert_eq!(project_tokens(1000, 10, 2), Some(5000));
        assert_eq!(project_tokens(1000, 2, 2), Some(1000));

        let (_directory, store) = store();
        let rows = u32::try_from(CHUNK_UNITS * 4).expect("rows");
        let units: Vec<_> = (0..rows).map(|row| unit("Item", row, 10)).collect();
        let job = store.create("c", &spec(), &units).expect("job");
        assert_eq!(
            store.history_chunk_tokens(&spec().model).expect("history"),
            None
        );
        for _ in 0..2 {
            let claimed = store.claim_chunk(&job.id).expect("claim").expect("chunk");
            let outcomes: Vec<_> = claimed
                .iter()
                .map(|unit| (unit.seq, UnitStatus::Drafted, None))
                .collect();
            store.finish_units(&job.id, &outcomes).expect("finish");
        }
        store
            .add_usage(
                &job.id,
                Usage {
                    prompt_tokens: 50_000,
                    completion_tokens: 10_000,
                },
            )
            .expect("usage");
        let summary = store.summary(&job.id).expect("summary");
        assert_eq!((summary.chunks, summary.finished_chunks), (4, 2));
        assert_eq!(summary.projected_tokens, Some(120_000));
        assert_eq!(
            store.history_chunk_tokens(&spec().model).expect("history"),
            None,
            "two chunks are too few"
        );

        let claimed = store.claim_chunk(&job.id).expect("claim").expect("chunk");
        let outcomes: Vec<_> = claimed
            .iter()
            .map(|unit| (unit.seq, UnitStatus::Failed, None))
            .collect();
        store.finish_units(&job.id, &outcomes).expect("finish");
        assert_eq!(
            store.history_chunk_tokens(&spec().model).expect("history"),
            Some(20_000)
        );
        let mut other = spec().model;
        other.effort = Some(crate::provider::ReasoningEffort::High);
        assert_eq!(store.history_chunk_tokens(&other).expect("history"), None);
    }

    #[test]
    fn concurrency_follows_the_job_size() {
        assert_eq!(job_concurrency(None, 1), 1);
        assert_eq!(job_concurrency(None, 26), DEFAULT_CONCURRENCY);
        assert_eq!(job_concurrency(None, LARGE_JOB_CHUNKS), MAX_CONCURRENCY);
        assert_eq!(
            job_concurrency(Some(12), 5),
            5,
            "never more than the chunks"
        );
        assert_eq!(job_concurrency(Some(40), 500), MAX_CONCURRENCY);
        assert_eq!(job_concurrency(Some(0), 500), 1);

        let (_directory, store) = store();
        let units: Vec<_> = (0..3).map(|row| unit("Item", row, 10)).collect();
        let job = store.create("c", &spec(), &units).expect("job");
        assert_eq!(
            store.set_concurrency(&job.id, 40).expect("set").concurrency,
            MAX_CONCURRENCY
        );
        assert_eq!(
            store.summary(&job.id).expect("summary").spec.concurrency,
            MAX_CONCURRENCY
        );
        store
            .set_status(&job.id, JobStatus::Cancelled, None)
            .expect("cancel");
        assert!(matches!(
            store.set_concurrency(&job.id, 4),
            Err(JobError::Invalid(_))
        ));
    }

    #[test]
    fn the_token_limit_changes_above_the_tokens_used() {
        let (_directory, store) = store();
        let units: Vec<_> = (0..3).map(|row| unit("Item", row, 10)).collect();
        let job = store.create("c", &spec(), &units).expect("job");
        store
            .add_usage(
                &job.id,
                Usage {
                    prompt_tokens: 900,
                    completion_tokens: 100,
                },
            )
            .expect("usage");
        assert!(matches!(
            store.set_token_limit(&job.id, 1000),
            Err(JobError::Invalid(_))
        ));
        assert_eq!(
            store
                .set_token_limit(&job.id, 5000)
                .expect("limit")
                .token_limit,
            5000
        );
        assert_eq!(
            store.summary(&job.id).expect("summary").spec.token_limit,
            5000
        );
        store
            .set_status(&job.id, JobStatus::Cancelled, None)
            .expect("cancel");
        assert!(matches!(
            store.set_token_limit(&job.id, 9000),
            Err(JobError::Invalid(_))
        ));
    }

    #[test]
    fn outcomes_counts_requeue_and_interruption() {
        let (_directory, store) = store();
        let units: Vec<ScopedUnit> = (0..4).map(|row| unit("Item", row, 10)).collect();
        let job = store.create("c1", &spec(), &units).expect("job");
        let chunk = store.claim_chunk(&job.id).expect("claim").expect("chunk");
        store
            .finish_units(
                &job.id,
                &[
                    (chunk[0].seq, UnitStatus::Drafted, None),
                    (
                        chunk[1].seq,
                        UnitStatus::Rejected,
                        Some("tag 1 is missing".to_owned()),
                    ),
                    (chunk[2].seq, UnitStatus::Failed, None),
                ],
            )
            .expect("finish");
        store
            .add_usage(
                &job.id,
                Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                },
            )
            .expect("usage");
        let summary = store.summary(&job.id).expect("summary");
        assert_eq!(summary.counts.drafted, 1);
        assert_eq!(summary.counts.rejected, 1);
        assert_eq!(summary.counts.running, 1);
        assert_eq!(summary.counts.processed(), 3);
        assert_eq!(summary.usage.prompt_tokens, 10);
        let rejected = store
            .units(&job.id, &[UnitStatus::Rejected], 0, 10)
            .expect("units");
        assert_eq!(rejected[0].message.as_deref(), Some("tag 1 is missing"));

        assert_eq!(
            store.pause_interrupted().expect("pause"),
            vec![job.id.clone()]
        );
        let paused = store.summary(&job.id).expect("summary");
        assert_eq!(paused.status, JobStatus::Paused);
        assert_eq!(paused.counts.running, 0);
        assert_eq!(paused.counts.pending, 1);

        assert_eq!(
            store
                .requeue(
                    &job.id,
                    &[
                        UnitStatus::Rejected,
                        UnitStatus::Failed,
                        UnitStatus::Drafted
                    ]
                )
                .expect("requeue"),
            2
        );
        assert_eq!(store.summary(&job.id).expect("summary").counts.pending, 3);
    }

    #[test]
    fn amendments_and_events_are_kept() {
        let (_directory, store) = store();
        let job = store
            .create("c1", &spec(), &[unit("Item", 1, 5)])
            .expect("job");
        let spec = store
            .amend(&job.id, " Use formal address. ")
            .expect("amend");
        assert_eq!(spec.instructions, "Be brief.\n\nUse formal address.");
        store
            .add_event(
                &job.id,
                "issue",
                "ambiguous term",
                Some(&unit("Item", 1, 0).location),
            )
            .expect("event");
        store
            .add_event(&job.id, "completed", "done", None)
            .expect("event");
        let events = store.events(&job.id, 0).expect("events");
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].location.as_ref().map(|location| location.row),
            Some(1)
        );
        assert_eq!(store.events(&job.id, 1).expect("events").len(), 1);
        assert!(matches!(
            store.summary("missing"),
            Err(JobError::NotFound(_))
        ));
        assert!(store.create("c1", &spec, &[]).is_err());
    }
}
