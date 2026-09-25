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
use crate::settings::ModelSelection;
use crate::tools::{ReviewLabel, UnitLocation, UnitState};

/// Most strings in one chunk.
pub const CHUNK_UNITS: usize = 30;
/// Most source characters in one chunk.
pub const CHUNK_SOURCE_CHARS: usize = 12_000;
/// Most workers one job runs at once.
pub const MAX_CONCURRENCY: u8 = 16;
/// Workers a job runs when Angelica does not choose.
pub const DEFAULT_CONCURRENCY: u8 = 8;
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
}

/// The strings a job covers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobScope {
    /// Sheets to cover; empty means every sheet with translatable strings.
    pub sheets: Vec<String>,
    pub filter: JobFilter,
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
    Drafted,
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
            Self::Rejected => "rejected",
            Self::Failed => "failed",
            Self::Conflict => "conflict",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "drafted" => Self::Drafted,
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
    pub rejected: u64,
    pub failed: u64,
    pub conflict: u64,
}

impl JobCounts {
    /// Strings with a final outcome.
    #[must_use]
    pub const fn processed(&self) -> u64 {
        self.drafted + self.rejected + self.failed + self.conflict
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

/// A string selected for a job, with its source size for chunking.
#[derive(Clone, Debug)]
pub struct ScopedUnit {
    pub location: UnitLocation,
    pub expected: UnitState,
    pub source_chars: usize,
}

/// Chunk numbers for units in order: a new chunk starts at a new sheet or
/// when a chunk would exceed [`CHUNK_UNITS`] strings or
/// [`CHUNK_SOURCE_CHARS`] source characters.
#[must_use]
pub fn assign_chunks(units: &[ScopedUnit]) -> Vec<u64> {
    let mut chunks = Vec::with_capacity(units.len());
    let mut chunk = 0_u64;
    let (mut chunk_units, mut chunk_chars) = (0_usize, 0_usize);
    let mut previous_sheet: Option<&str> = None;
    for unit in units {
        let new_sheet = previous_sheet.is_some_and(|sheet| sheet != unit.location.sheet);
        if chunk_units > 0
            && (new_sheet
                || chunk_units >= CHUNK_UNITS
                || chunk_chars + unit.source_chars > CHUNK_SOURCE_CHARS)
        {
            chunk += 1;
            chunk_units = 0;
            chunk_chars = 0;
        }
        chunk_units += 1;
        chunk_chars += unit.source_chars;
        previous_sheet = Some(&unit.location.sheet);
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
    /// A rough token estimate: per-chunk instructions and tools plus the
    /// source text read and written. Providers count differently.
    pub estimated_tokens: u64,
}

/// Per-chunk overhead in the token estimate: instructions, tools, and turns.
const CHUNK_OVERHEAD_TOKENS: u64 = 6000;

impl JobEstimate {
    #[must_use]
    pub fn for_units(units: &[ScopedUnit]) -> Self {
        let chunks = assign_chunks(units).last().map_or(0, |last| last + 1);
        let characters: u64 = units.iter().map(|unit| unit.source_chars as u64).sum();
        Self {
            units: units.len() as u64,
            chunks,
            estimated_tokens: chunks * CHUNK_OVERHEAD_TOKENS + characters * 2,
        }
    }
}

/// A job Angelica proposed and the user has not started yet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobProposal {
    pub scope: JobScope,
    pub instructions: String,
    pub concurrency: u8,
    pub estimate: JobEstimate,
    pub token_limit: u64,
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
            usage: Usage {
                prompt_tokens: to_u64(row.5),
                completion_tokens: to_u64(row.6),
            },
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
                    UnitStatus::Pending | UnitStatus::Running | UnitStatus::Drafted
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
            scope: JobScope {
                sheets: vec!["Item".to_owned()],
                filter: JobFilter::Untranslated,
            },
            instructions: "Be brief.".to_owned(),
            model: ModelSelection {
                provider_id: "p".to_owned(),
                model_id: "m".to_owned(),
                effort: None,
            },
            token_limit: 1000,
            concurrency: 2,
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
    fn estimates_count_units_chunks_and_tokens() {
        let count = u32::try_from(CHUNK_UNITS).expect("chunk size") + 1;
        let units: Vec<ScopedUnit> = (0..count).map(|row| unit("Item", row, 30)).collect();
        let estimate = JobEstimate::for_units(&units);
        assert_eq!(estimate.units, u64::from(count));
        assert_eq!(estimate.chunks, 2);
        assert_eq!(
            estimate.estimated_tokens,
            2 * CHUNK_OVERHEAD_TOKENS + u64::from(count) * 30 * 2
        );
        assert_eq!(JobEstimate::for_units(&[]).chunks, 0);
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
