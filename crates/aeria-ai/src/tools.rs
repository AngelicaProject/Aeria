//! Angelica's Aeria-specific tools.
//!
//! Tools are typed and bounded. `aeria-ai` owns their schemas, argument
//! validation, and result shaping; the desktop implements [`ProjectReader`]
//! over the active project session. Invalid calls return an error result to
//! the model instead of failing the conversation.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::chat::ToolDefinition;
use crate::dialogue::{
    SheetDialogue, dialogue_context, dialogue_tool_definitions, get_voices, list_speakers,
    propose_voice_profile, speaker_lines, voice_change_definition,
};
use crate::guidance::{
    GLOSSARY_FILE, Glossary, GlossaryEntry, ProjectFile, ProjectGuide, change_glossary,
};
use crate::images::MAX_JOB_IMAGES;
use crate::jobs::{
    JobEstimate, JobEvent, JobFilter, JobQuality, JobScope, JobStatus, JobSummary, JobUnit,
    UnitStatus,
};
use crate::search::{ProjectSearch, run_search_tool};

/// Longest source, target, or note text returned for one cell.
pub const MAX_CELL_TEXT_CHARS: usize = 2000;
/// Longest source returned for a translatable cell. A source is never cut
/// below this, since a partial source cannot be translated.
pub const MAX_SOURCE_CHARS: usize = 8000;
/// Most translations one `propose_translation` call accepts.
pub const MAX_PROPOSALS_PER_CALL: usize = 20;
/// Longest serialized tool result.
pub const MAX_RESULT_CHARS: usize = 24_000;
/// Most source row groups one `read_rows` call scans.
pub const MAX_READ_ROWS: u32 = 50;

/// A failure reported back to the model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolError(pub String);

impl ToolError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Facts about the open project.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFacts {
    pub source_language: String,
    /// `None` while the project has no real target language yet.
    pub target_language: Option<String>,
    pub game_version: String,
    pub sheet_count: usize,
    pub translatable_strings: u64,
    pub translated: u64,
    pub reviewed: u64,
    pub needs_review: u64,
    pub detached_units: usize,
}

/// Coverage of one sheet.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetSummary {
    pub name: String,
    pub translatable: u64,
    pub translated: u64,
    pub reviewed: u64,
    pub needs_review: u64,
}

/// A review state as the model sees it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewLabel {
    Draft,
    NeedsReview,
    Reviewed,
}

/// One translatable string occurrence and its translation, if any.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellSnapshot {
    pub column: u32,
    pub source: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub formatting_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_state: Option<ReviewLabel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_id: Option<String>,
    /// What each macro of the source does and what a translation may do
    /// with it. Filled in by the tools, not by the reader.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub constructs: Vec<String>,
    /// The source is malformed and cannot be translated with assistance.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub malformed: bool,
    /// Glossary entries whose terms occur in the source.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<GlossaryEntry>,
}

/// One source row with its translatable cells and read-only context.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowSnapshot {
    pub row: u32,
    pub subrow: u16,
    pub cells: Vec<CellSnapshot>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<ContextCell>,
}

/// A read-only source cell shown for context.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCell {
    pub column: u32,
    pub source: String,
}

/// One bounded page of rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowsPage {
    pub rows: Vec<RowSnapshot>,
    /// Cursor for the next page, when more source rows exist.
    pub next_after: Option<(u32, u16)>,
}

/// A string occurrence address.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitLocation {
    pub sheet: String,
    pub row: u32,
    pub subrow: u16,
    pub column: Option<u32>,
}

/// Read access to the open project, implemented by the desktop.
pub trait ProjectReader: Send + Sync {
    /// The repository root whose project knowledge Angelica reads and
    /// writes; `None` where knowledge is not available.
    fn knowledge_root(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// # Errors
    /// Returns an error when no project is open or it cannot be read.
    fn facts(&self) -> Result<ProjectFacts, ToolError>;

    /// # Errors
    /// Returns an error when no project is open or it cannot be read.
    fn sheets(&self) -> Result<Vec<SheetSummary>, ToolError>;

    /// Reads up to `limit` source row groups after the exclusive cursor.
    ///
    /// # Errors
    /// Returns an error for an unknown sheet or a read failure.
    fn rows(
        &self,
        sheet: &str,
        after: Option<(u32, u16)>,
        limit: u32,
    ) -> Result<RowsPage, ToolError>;

    /// Reads one source row, or `None` when it has no translatable string.
    ///
    /// # Errors
    /// Returns an error for an unknown sheet or a read failure.
    fn row(&self, sheet: &str, row: u32, subrow: u16) -> Result<Option<RowSnapshot>, ToolError>;

    /// One source cell's macro text in the game's other client languages,
    /// as `(language code, text)` pairs; the text is `None` where a
    /// language lacks the cell.
    ///
    /// # Errors
    /// Returns an error when no project is open or the game cannot be read.
    fn other_languages(
        &self,
        sheet: &str,
        row: u32,
        subrow: u16,
        column: u32,
    ) -> Result<Vec<(String, Option<String>)>, ToolError>;

    /// Uncommitted translation changes as JSON.
    ///
    /// # Errors
    /// Returns an error when Git is unavailable or fails.
    fn pending_changes(&self) -> Result<Value, ToolError>;

    /// Committed history of one unit's translation as JSON.
    ///
    /// # Errors
    /// Returns an error for an unknown unit or a Git failure.
    fn unit_history(&self, unit_id: &str, limit: u32) -> Result<Value, ToolError>;

    /// Opens an occurrence in the editor. Presentation only.
    ///
    /// # Errors
    /// Returns an error when the editor cannot be reached.
    fn navigate(&self, location: &UnitLocation) -> Result<(), ToolError>;

    /// Reads a project-shared file as text; `None` when it does not exist.
    ///
    /// # Errors
    /// Returns an error when the file exists but cannot be read.
    fn project_file(&self, file: ProjectFile) -> Result<Option<String>, ToolError>;

    /// The dialogue structure of a quest or cutscene sheet; `None` for any
    /// other sheet.
    ///
    /// # Errors
    /// Returns an error when no project is open or the game cannot be read.
    fn dialogue(&self, sheet: &str) -> Result<Option<SheetDialogue>, ToolError>;

    /// Speaker labels of quest and cutscene speech that contain `query`,
    /// ignoring case, with their number of lines.
    ///
    /// # Errors
    /// Returns an error when no project is open or the game cannot be read.
    fn speakers(&self, query: &str) -> Result<Vec<(String, usize)>, ToolError>;

    /// One speaker label's lines in sheet-name and row order: the total and
    /// up to `limit` lines from `offset`, without a column.
    ///
    /// # Errors
    /// Returns an error when no project is open or the game cannot be read.
    fn speaker_lines(
        &self,
        speaker: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(usize, Vec<UnitLocation>), ToolError>;
}

/// Most glossary entries attached to one string.
pub const MAX_GLOSSARY_PER_CELL: usize = 20;
/// Most glossary entries one `get_guidance` call returns.
pub const MAX_GLOSSARY_LOOKUP: usize = 200;
/// Most glossary additions or removals in one proposal.
pub const MAX_GLOSSARY_CHANGES: usize = 100;

/// A proposed replacement of a project-shared file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileChange {
    pub file: ProjectFile,
    /// The file as read when the change was made; `None` when absent.
    pub before: Option<String>,
    pub after: String,
    /// A pending proposal of the same conversation this change was built
    /// on and replaces.
    pub replaces: Option<String>,
}

/// A pending change to a project-shared file in the same conversation,
/// made against the file as it is now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingFileChange {
    pub proposal_id: String,
    /// The file content the change would write.
    pub after: String,
}

/// The current state of one string, as a translation's expectation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitState {
    pub target: Option<String>,
    pub review_state: Option<ReviewLabel>,
}

/// A translatable string: its verified source and current state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslatableUnit {
    pub source: String,
    pub state: UnitState,
}

/// A validated translation, ready to apply or to show for approval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    pub location: UnitLocation,
    pub source: String,
    /// The target macro string, as the model wrote it.
    pub target: String,
    /// The state the translation was produced against.
    pub expected: UnitState,
}

/// What happened to a submitted proposal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalOutcome {
    /// Written as a draft.
    Applied,
    /// Waiting for the user's approval.
    Pending {
        proposal_id: String,
    },
    /// The string changed; nothing was written.
    Conflict {
        message: String,
    },
    Failed {
        message: String,
    },
}

/// Most strings in one suggested approval.
pub const MAX_REVIEW_ITEMS: usize = 200;

/// A translation Angelica suggests approving, as it was when suggested.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewItem {
    pub location: UnitLocation,
    pub source: String,
    pub target: String,
}

/// Translations Angelica suggests marking reviewed, with her reason.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewBatch {
    pub reason: String,
    pub items: Vec<ReviewItem>,
}

/// Write access for Angelica, implemented by the desktop. Whether a
/// proposal is applied at once or waits for approval is the desktop's
/// decision, following the conversation's mode.
pub trait ProjectWriter: Send + Sync {
    /// Reads the verified source and current state of one string.
    ///
    /// # Errors
    /// Returns an error for a location that is not a translatable string.
    fn translatable_unit(&self, location: &UnitLocation) -> Result<TranslatableUnit, ToolError>;

    /// Applies or records validated proposals, one outcome per proposal.
    ///
    /// # Errors
    /// Returns an error when nothing could be submitted.
    fn submit(&self, proposals: Vec<Proposal>) -> Result<Vec<ProposalOutcome>, ToolError>;

    /// Records a change to a project-shared file for the user's approval.
    ///
    /// # Errors
    /// Returns an error when the proposal cannot be recorded.
    fn propose_file_change(&self, change: FileChange) -> Result<ProposalOutcome, ToolError>;

    /// The latest pending change to `file` in this conversation that was
    /// made against the file as it is now, so a new change can build on it.
    ///
    /// # Errors
    /// Returns an error when the proposals cannot be read.
    fn pending_file_change(
        &self,
        file: ProjectFile,
    ) -> Result<Option<PendingFileChange>, ToolError> {
        let _ = file;
        Ok(None)
    }

    /// Records translations to mark reviewed. It always waits for the
    /// user's approval, in every mode.
    ///
    /// # Errors
    /// Returns an error when the suggestion cannot be recorded.
    fn propose_review(&self, batch: ReviewBatch) -> Result<ProposalOutcome, ToolError>;
}

/// A control action on a job.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobAction {
    Pause,
    Resume,
    Cancel,
}

/// What a revision job covers: the strings that contain a term, or the
/// lines of a speaker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RevisionTarget {
    /// The strings that contain any of these terms.
    Terms(Vec<String>),
    Speaker(String),
}

/// Translation-job access for Angelica, implemented by the desktop.
pub trait JobControl: Send + Sync {
    /// Records a job for the user to start that translates again the
    /// strings of `target` no person has settled, with `reason` as its
    /// instructions.
    ///
    /// # Errors
    /// Returns an error when the target has no such strings or the proposal
    /// cannot be recorded.
    fn propose_revision(
        &self,
        target: RevisionTarget,
        reason: String,
        quality: JobQuality,
    ) -> Result<ProposalOutcome, ToolError> {
        let _ = (target, reason, quality);
        Err(ToolError::new("revision jobs are not available here"))
    }

    /// Counts the strings a job over `scope` would cover.
    ///
    /// # Errors
    /// Returns an error for an unknown sheet or a read failure.
    fn estimate(&self, scope: &JobScope) -> Result<JobEstimate, ToolError>;

    /// Records a job for the user to start. `concurrency` is Angelica's
    /// choice of workers, if she made one; `images` are IDs of images in
    /// the conversation for every worker; `quality` is how much work the
    /// localizer spends on each unit.
    ///
    /// # Errors
    /// Returns an error for an image the conversation does not have, or
    /// when the proposal cannot be recorded.
    fn propose(
        &self,
        scope: JobScope,
        instructions: String,
        concurrency: Option<u8>,
        images: &[String],
        quality: JobQuality,
    ) -> Result<ProposalOutcome, ToolError>;

    /// Lists the project's jobs, newest first.
    ///
    /// # Errors
    /// Returns an error when jobs cannot be read.
    fn jobs(&self) -> Result<Vec<JobSummary>, ToolError>;

    /// Events after `after`.
    ///
    /// # Errors
    /// Returns an error for an unknown job.
    fn events(&self, job_id: &str, after: u64) -> Result<Vec<JobEvent>, ToolError>;

    /// Strings with the given statuses.
    ///
    /// # Errors
    /// Returns an error for an unknown job.
    fn units(&self, job_id: &str, statuses: &[UnitStatus]) -> Result<Vec<JobUnit>, ToolError>;

    /// Adds instructions for chunks that start afterwards.
    ///
    /// # Errors
    /// Returns an error for an unknown job.
    fn amend(&self, job_id: &str, instructions: &str) -> Result<(), ToolError>;

    /// Requeues strings with the given final statuses; returns how many.
    ///
    /// # Errors
    /// Returns an error for an unknown job.
    fn retry(&self, job_id: &str, statuses: &[UnitStatus]) -> Result<u64, ToolError>;

    /// Changes how many workers a job runs.
    ///
    /// # Errors
    /// Returns an error for an unknown or cancelled job.
    fn set_concurrency(&self, job_id: &str, concurrency: u8) -> Result<u8, ToolError>;

    /// Records a higher token limit for a job, for the user to approve.
    ///
    /// # Errors
    /// Returns an error for an unknown or cancelled job, a limit not above
    /// the tokens used, or a failure to record the proposal.
    fn propose_limit(&self, job_id: &str, token_limit: u64) -> Result<ProposalOutcome, ToolError>;

    /// Pauses, resumes, or cancels a job and returns its status.
    ///
    /// # Errors
    /// Returns an error for an unknown job or an impossible transition.
    fn control(&self, job_id: &str, action: JobAction) -> Result<JobStatus, ToolError>;

    /// The decisions that wait for a person: uncalibrated kinds of text,
    /// made-up names with their options, strings left for review, and
    /// findings against the knowledge.
    ///
    /// # Errors
    /// Returns an error when they cannot be read.
    fn decisions(&self) -> Result<Value, ToolError> {
        Err(ToolError::new("decisions are not available here"))
    }

    /// Settles a made-up name with a rendering; returns whether the
    /// rendering changed.
    ///
    /// # Errors
    /// Returns an error for a term the agent knowledge does not have.
    fn settle_name(&self, term: &str, rendering: &str) -> Result<bool, ToolError> {
        let _ = (term, rendering);
        Err(ToolError::new("names cannot be settled here"))
    }

    /// Withdraws one of this conversation's pending proposals.
    ///
    /// # Errors
    /// Returns an error for an unknown or settled proposal.
    fn withdraw(&self, proposal_id: &str) -> Result<(), ToolError> {
        let _ = proposal_id;
        Err(ToolError::new("proposals cannot be withdrawn here"))
    }
}

/// Most strings `job_status` lists per status.
const MAX_JOB_UNITS_LISTED: usize = 50;

/// Definitions of the job tools. Reading tools are offered in every mode;
/// the others only where Angelica may change the project.
#[must_use]
pub fn job_tool_definitions(write: bool) -> Vec<ToolDefinition> {
    let scope = json!({
        "sheets": { "type": "array", "items": { "type": "string" }, "description": "Exact sheet names. With no sheets and no patterns, every sheet of the project." },
        "patterns": { "type": "array", "items": { "type": "string" }, "description": "Sheet name patterns where * stands for any text, ignoring case, such as quest/* (every quest), quest/*/Man* (quests whose ID starts with Man), cut_scene/*, or *Name." },
        "exclude": { "type": "array", "items": { "type": "string" }, "description": "Patterns of sheets to leave out, such as quest/*/Cls* when class quests get a job of their own." },
        "careful": { "type": "array", "items": { "type": "string" }, "description": "Patterns of sheets localized with quality careful inside the job, such as quest/*/Man* for the first main story quests; other sheets use the job's quality." },
        "filter": { "type": "string", "enum": ["untranslated", "needsReview", "untranslatedAndDrafts"], "description": "Which strings: untranslated ones (default), ones needing review, or untranslated ones and existing drafts. Reviewed translations are never included." },
    });
    let job_id = json!({ "type": "object", "properties": { "job_id": { "type": "string" } }, "required": ["job_id"], "additionalProperties": false });
    let mut estimate_properties = scope.as_object().cloned().unwrap_or_default();
    estimate_properties.insert("quality".to_owned(), json!({ "type": "string", "enum": ["fast", "careful"], "description": "fast (the default): parts of each unit written in parallel, fast enough for the whole game. careful: a high effort for every step and two full rechecks; about twice the tokens and slower, for main story quests or when the user asks." }));
    let mut tools = vec![
        ToolDefinition {
            name: "list_decisions",
            description: "The decisions that wait for a person in the localization panel: kinds of text whose style no person chose, made-up names with the study's rendering and its options, strings the critics left for review per job, and findings against the project knowledge.",
            parameters: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDefinition {
            name: "estimate_job",
            description: "Counts the strings, chunks, and approximate tokens a translation job over a scope would use at a quality. Changes nothing.",
            parameters: json!({ "type": "object", "properties": estimate_properties, "additionalProperties": false }),
        },
        ToolDefinition {
            name: "job_status",
            description: "Status, progress, and token use of the project's translation jobs, or of one job with the strings that were rejected, failed, or skipped.",
            parameters: json!({ "type": "object", "properties": { "job_id": { "type": "string" } }, "additionalProperties": false }),
        },
        ToolDefinition {
            name: "job_events",
            description: "Issues workers reported and other events of a job, after an event number.",
            parameters: json!({
                "type": "object",
                "properties": { "job_id": { "type": "string" }, "after": { "type": "integer", "minimum": 0 } },
                "required": ["job_id"],
                "additionalProperties": false,
            }),
        },
    ];
    if write {
        let mut start_properties = scope.as_object().cloned().unwrap_or_default();
        start_properties.insert("instructions".to_owned(), json!({ "type": "string", "description": "Instructions for every chunk of the job: style, terminology, anything the user asked for." }));
        start_properties.insert("images".to_owned(), json!({ "type": "array", "items": { "type": "string" }, "maxItems": MAX_JOB_IMAGES, "description": "IDs of images in this conversation that every chunk should see, such as a screenshot showing where the strings appear. Each image is sent with every chunk." }));
        start_properties.insert("quality".to_owned(), json!({ "type": "string", "enum": ["fast", "careful"], "description": "fast (the default): parts of each unit written in parallel, fast enough for the whole game. careful: a high effort for every step and two full rechecks; about twice the tokens and slower, for main story quests or when the user asks." }));
        tools.extend([
            ToolDefinition {
                name: "start_job",
                description: "Proposes a translation job: the localizer translates the scope's strings, each quest or cutscene as one scene, and writes final translations, or ones needing review where its critics left a finding open. The user sees the estimate and starts it; nothing runs before that.",
                parameters: json!({ "type": "object", "properties": start_properties, "additionalProperties": false }),
            },
            ToolDefinition {
                name: "amend_job",
                description: "Adds instructions for the chunks of a job that have not started yet.",
                parameters: json!({
                    "type": "object",
                    "properties": { "job_id": { "type": "string" }, "instructions": { "type": "string" } },
                    "required": ["job_id", "instructions"],
                    "additionalProperties": false,
                }),
            },
            ToolDefinition {
                name: "retry_units",
                description: "Requeues a job's rejected, failed, or skipped strings, optionally with instructions for the retry, and resumes the job.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "job_id": { "type": "string" },
                        "statuses": { "type": "array", "items": { "type": "string", "enum": ["rejected", "failed", "conflict"] }, "description": "Defaults to rejected and failed." },
                        "instructions": { "type": "string" },
                    },
                    "required": ["job_id"],
                    "additionalProperties": false,
                }),
            },
            ToolDefinition { name: "pause_job", description: "Pauses a running job after its current chunks.", parameters: job_id.clone() },
            ToolDefinition { name: "resume_job", description: "Resumes a paused job.", parameters: job_id.clone() },
            ToolDefinition { name: "cancel_job", description: "Cancels a job. Translations already written stay.", parameters: job_id },
            ToolDefinition {
                name: "settle_names",
                description: "Settles made-up names from list_decisions: each term takes the rendering you choose (the study's choice, one of its options, or your own), the other options become forbidden, and the name leaves the decisions. Returns which renderings changed; strings written with an old rendering can then be revised with propose_revision.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "names": { "type": "array", "minItems": 1, "maxItems": 200, "items": { "type": "object", "properties": {
                            "term": { "type": "string" },
                            "rendering": { "type": "string" },
                        }, "required": ["term", "rendering"], "additionalProperties": false } },
                    },
                    "required": ["names"],
                    "additionalProperties": false,
                }),
            },
            ToolDefinition {
                name: "withdraw_proposal",
                description: "Withdraws one of your pending proposals in this conversation, such as a job or glossary change that is no longer right, so you can propose a better one.",
                parameters: json!({
                    "type": "object",
                    "properties": { "proposal_id": { "type": "string" } },
                    "required": ["proposal_id"],
                    "additionalProperties": false,
                }),
            },
        ]);
        tools.push(revision_definition());
    }
    tools
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeArgs {
    #[serde(default)]
    sheets: Vec<String>,
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    careful: Vec<String>,
    filter: Option<JobFilter>,
    quality: Option<JobQuality>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartJobArgs {
    #[serde(default)]
    sheets: Vec<String>,
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    careful: Vec<String>,
    filter: Option<JobFilter>,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    quality: JobQuality,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevisionArgs {
    term: Option<String>,
    #[serde(default)]
    terms: Vec<String>,
    speaker: Option<String>,
    reason: String,
    quality: Option<JobQuality>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobIdArgs {
    job_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettleName {
    term: String,
    rendering: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettleNamesArgs {
    names: Vec<SettleName>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WithdrawArgs {
    proposal_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobStatusArgs {
    job_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobEventsArgs {
    job_id: String,
    #[serde(default)]
    after: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AmendArgs {
    job_id: String,
    instructions: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetryArgs {
    job_id: String,
    statuses: Option<Vec<UnitStatus>>,
    instructions: Option<String>,
}

fn start_job(jobs: &dyn JobControl, args: StartJobArgs) -> Result<Value, ToolError> {
    let scope = JobScope {
        patterns: args.patterns,
        exclude: args.exclude,
        careful: args.careful,
        ..JobScope::sheets(args.sheets, args.filter.unwrap_or(JobFilter::Untranslated))
    };
    let mut seen = std::collections::HashSet::new();
    let images: Vec<String> = args
        .images
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect();
    if images.len() > MAX_JOB_IMAGES {
        return Err(ToolError::new(format!(
            "a job can pass at most {MAX_JOB_IMAGES} images to its workers"
        )));
    }
    let outcome = jobs.propose(
        scope,
        args.instructions.trim().to_owned(),
        None,
        &images,
        args.quality,
    )?;
    Ok(match outcome {
        ProposalOutcome::Pending { proposal_id } => json!({
            "status": "awaitingApproval",
            "proposalId": proposal_id,
            "note": "The user sees the estimate and starts the job. You will get an automatic message when it finishes or pauses.",
        }),
        ProposalOutcome::Applied => json!({ "status": "started" }),
        ProposalOutcome::Conflict { message } | ProposalOutcome::Failed { message } => {
            json!({ "status": "failed", "errors": [message] })
        }
    })
}

fn retry_units(jobs: &dyn JobControl, args: RetryArgs) -> Result<Value, ToolError> {
    if let Some(instructions) = args.instructions.filter(|text| !text.trim().is_empty()) {
        jobs.amend(&args.job_id, &instructions)?;
    }
    let statuses = args
        .statuses
        .unwrap_or_else(|| vec![UnitStatus::Rejected, UnitStatus::Failed]);
    let requeued = jobs.retry(&args.job_id, &statuses)?;
    let status = if requeued > 0 {
        Some(jobs.control(&args.job_id, JobAction::Resume)?)
    } else {
        None
    };
    Ok(json!({ "requeued": requeued, "status": status }))
}

fn propose_revision(jobs: &dyn JobControl, args: RevisionArgs) -> Result<Value, ToolError> {
    let terms: Vec<String> = args
        .term
        .into_iter()
        .chain(args.terms)
        .map(|term| term.trim().to_owned())
        .filter(|term| !term.is_empty())
        .collect();
    let (target, default_quality) = match (terms.is_empty(), args.speaker) {
        // A changed rendering only needs the old one edited out.
        (false, None) => (RevisionTarget::Terms(terms), JobQuality::Edit),
        (true, Some(speaker)) if !speaker.trim().is_empty() => (
            RevisionTarget::Speaker(speaker.trim().to_ascii_uppercase()),
            JobQuality::Fast,
        ),
        _ => return Err(ToolError::new("give terms or a speaker, not both")),
    };
    let reason = args.reason.trim().to_owned();
    if reason.is_empty() {
        return Err(ToolError::new("give the reason for the revision"));
    }
    Ok(
        match jobs.propose_revision(target, reason, args.quality.unwrap_or(default_quality))? {
            ProposalOutcome::Pending { proposal_id } => json!({
                "status": "awaitingApproval",
                "proposalId": proposal_id,
                "note": "The user sees the estimate and starts the job.",
            }),
            ProposalOutcome::Applied => json!({ "status": "started" }),
            ProposalOutcome::Conflict { message } | ProposalOutcome::Failed { message } => {
                json!({ "status": "failed", "errors": [message] })
            }
        },
    )
}

fn run_job_tool(
    jobs: &dyn JobControl,
    write: bool,
    name: &str,
    arguments: &str,
) -> Result<Value, ToolError> {
    let needs_write = !matches!(
        name,
        "estimate_job" | "job_status" | "job_events" | "list_decisions"
    );
    if needs_write && !write {
        return Err(ToolError::new(
            "changing jobs is not available in Chat mode",
        ));
    }
    match name {
        "list_decisions" => {
            let _: EmptyArgs = parse(arguments)?;
            jobs.decisions()
        }
        "settle_names" => {
            let args: SettleNamesArgs = parse(arguments)?;
            let mut changed = Vec::new();
            let mut failed = Vec::new();
            for name in args.names {
                match jobs.settle_name(&name.term, &name.rendering) {
                    Ok(true) => changed.push(name.term),
                    Ok(false) => {}
                    Err(error) => failed.push(json!({ "term": name.term, "error": error.0 })),
                }
            }
            Ok(json!({ "changed": changed, "failed": failed }))
        }
        "withdraw_proposal" => {
            let args: WithdrawArgs = parse(arguments)?;
            jobs.withdraw(&args.proposal_id)?;
            Ok(json!({ "withdrawn": true }))
        }
        "estimate_job" => {
            let args: ScopeArgs = parse(arguments)?;
            let scope = JobScope {
                patterns: args.patterns,
                exclude: args.exclude,
                careful: args.careful,
                ..JobScope::sheets(args.sheets, args.filter.unwrap_or(JobFilter::Untranslated))
            };
            to_value(
                &jobs
                    .estimate(&scope)?
                    .with_quality(args.quality.unwrap_or(JobQuality::Fast)),
            )
        }
        "start_job" => start_job(jobs, parse(arguments)?),
        "propose_revision" => propose_revision(jobs, parse(arguments)?),
        "job_status" => {
            let args: JobStatusArgs = parse(arguments)?;
            let all = jobs.jobs()?;
            match args.job_id {
                None => to_value(&all.into_iter().take(20).collect::<Vec<_>>()),
                Some(id) => {
                    let job = all
                        .into_iter()
                        .find(|job| job.id == id)
                        .ok_or_else(|| ToolError::new(format!("job {id:?} was not found")))?;
                    let problems = jobs.units(
                        &id,
                        &[
                            UnitStatus::Rejected,
                            UnitStatus::Failed,
                            UnitStatus::Conflict,
                        ],
                    )?;
                    Ok(json!({
                        "job": job,
                        "problemUnits": problems.iter().take(MAX_JOB_UNITS_LISTED).collect::<Vec<_>>(),
                        "problemUnitsTotal": problems.len(),
                    }))
                }
            }
        }
        "job_events" => {
            let args: JobEventsArgs = parse(arguments)?;
            to_value(&jobs.events(&args.job_id, args.after)?)
        }
        "amend_job" => {
            let args: AmendArgs = parse(arguments)?;
            if args.instructions.trim().is_empty() {
                return Err(ToolError::new("the instructions are empty"));
            }
            jobs.amend(&args.job_id, &args.instructions)?;
            Ok(json!({ "amended": true }))
        }
        "retry_units" => retry_units(jobs, parse(arguments)?),
        "pause_job" | "resume_job" | "cancel_job" => {
            let args: JobIdArgs = parse(arguments)?;
            let action = match name {
                "pause_job" => JobAction::Pause,
                "resume_job" => JobAction::Resume,
                _ => JobAction::Cancel,
            };
            Ok(json!({ "status": jobs.control(&args.job_id, action)? }))
        }
        other => Err(ToolError::new(format!("unknown tool {other:?}"))),
    }
}

/// The definition of `propose_revision`.
fn revision_definition() -> ToolDefinition {
    ToolDefinition {
        name: "propose_revision",
        description: "Revises the translated strings that contain any of the given terms, or the lines of a speaker, which no person has settled: drafts, strings needing review, and reviewed ones whose translation is still the one a job wrote. Use it after the renderings of names or game terms, or a character's profile, changed, with the change as the reason; never for ordinary words or style, which the next work follows anyway. Give every changed term in one call: one revision, not one per term. Terms are edited in place (quality edit: the old wording replaced and agreement fixed, one cheap request per part); a speaker's lines are translated again. At most 1,000 strings.",
        parameters: json!({
            "type": "object",
            "properties": {
                "terms": { "type": "array", "items": { "type": "string" }, "description": "Source-language terms whose rendering changed." },
                "term": { "type": "string", "description": "One source-language term; terms takes several." },
                "speaker": { "type": "string", "description": "A speaker label, such as URIANGER." },
                "reason": { "type": "string", "description": "What changed and what the translation must do now, such as each term's old and new rendering; every chunk gets it as instructions." },
                "quality": { "type": "string", "enum": ["edit", "fast", "careful"], "description": "Defaults to edit for terms and fast for a speaker." },
            },
            "required": ["reason"],
            "additionalProperties": false,
        }),
    }
}

/// Definitions of the tools that propose changes, offered in Ask and
/// Auto-draft modes.
#[must_use]
pub fn write_tool_definitions() -> Vec<ToolDefinition> {
    let translation = json!({
        "type": "object",
        "properties": {
            "sheet": { "type": "string" },
            "row": { "type": "integer", "minimum": 0 },
            "subrow": { "type": "integer", "minimum": 0 },
            "column": { "type": "integer", "minimum": 0 },
            "target": { "type": "string", "description": "The translation as macro text: the game data of the source kept, conditions and formatting as the target language needs them, \\< \\{ \\\\ for literal characters." },
        },
        "required": ["sheet", "row", "subrow", "column", "target"],
        "additionalProperties": false,
    });
    let mut tools = vec![
        ToolDefinition {
            name: "validate_target",
            description: "Checks a translation of one string against its source without writing anything, and returns what to fix, if anything.",
            parameters: translation.clone(),
        },
        ToolDefinition {
            name: "propose_translation",
            description: "Proposes translations for up to 20 strings. Each is validated; valid ones are written as drafts or shown to the user for approval, depending on the mode. Returns one result per translation.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "translations": { "type": "array", "minItems": 1, "maxItems": MAX_PROPOSALS_PER_CALL, "items": translation },
                },
                "required": ["translations"],
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "propose_glossary_change",
            description: "Proposes adding, replacing, or removing glossary entries. The user always approves glossary changes. Use it when the user asks or a term clearly needs a fixed translation.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "add": {
                        "type": "array",
                        "maxItems": MAX_GLOSSARY_CHANGES,
                        "items": {
                            "type": "object",
                            "properties": {
                                "term": { "type": "string", "description": "The source-language term." },
                                "translation": { "type": "string" },
                                "note": { "type": "string" },
                                "forbidden": { "type": "array", "items": { "type": "string" }, "description": "Translations that must not be used." },
                            },
                            "required": ["term", "translation"],
                            "additionalProperties": false,
                        },
                    },
                    "remove": { "type": "array", "maxItems": MAX_GLOSSARY_CHANGES, "items": { "type": "string" }, "description": "Terms to remove." },
                },
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "propose_review",
            description: "Suggests marking up to 200 translated strings as reviewed, with a short reason. The user sees the list and approves or rejects it; nothing is marked reviewed until then, and strings changed meanwhile are skipped. Check each translation before suggesting it.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "strings": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": MAX_REVIEW_ITEMS,
                        "items": {
                            "type": "object",
                            "properties": {
                                "sheet": { "type": "string" },
                                "row": { "type": "integer", "minimum": 0 },
                                "subrow": { "type": "integer", "minimum": 0 },
                                "column": { "type": "integer", "minimum": 0 },
                            },
                            "required": ["sheet", "row", "column"],
                            "additionalProperties": false,
                        },
                    },
                    "reason": { "type": "string", "description": "What you checked, for the user." },
                },
                "required": ["strings", "reason"],
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "propose_guidance_change",
            description: "Proposes a new full text for the project's translation guidance. The user always approves guidance changes. Read the current guidance with get_guidance first.",
            parameters: json!({
                "type": "object",
                "properties": { "text": { "type": "string", "description": "The complete new guidance in Markdown." } },
                "required": ["text"],
                "additionalProperties": false,
            }),
        },
    ];
    tools.push(voice_change_definition());
    tools.extend(crate::knowledge_tools::write_definitions());
    tools
}

/// Definitions of the read-only tools offered in Chat mode.
#[must_use]
pub fn read_tool_definitions() -> Vec<ToolDefinition> {
    let mut tools = project_read_definitions();
    tools.extend(dialogue_tool_definitions());
    tools.extend(crate::knowledge_tools::read_definitions());
    tools.push(crate::ask::definition());
    tools
}

fn list_sheets_definition() -> ToolDefinition {
    ToolDefinition {
        name: "list_sheets",
        description: "Sheets that contain translatable strings, with per-sheet progress. Filter by a name substring; results are paged. With group_by, totals per group of sheets instead, for an overview of a large part of the game in one call.",
        parameters: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Case-insensitive substring of the sheet name." },
                "untranslated_only": { "type": "boolean", "description": "Only sheets with untranslated strings." },
                "group_by": { "type": "string", "enum": ["folder", "prefix"], "description": "folder: totals per folder, such as quest/001; sheets without a folder are one group. prefix: per folder and the first three letters of the sheet's own name, such as quest/001/Man." },
                "offset": { "type": "integer", "minimum": 0 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Defaults to 50." },
            },
            "additionalProperties": false,
        }),
    }
}

/// The read tools over the project's strings and files.
fn project_read_definitions() -> Vec<ToolDefinition> {
    let location = |description: &str| {
        json!({
            "type": "object",
            "properties": {
                "sheet": { "type": "string", "description": "Exact sheet name, as listed by list_sheets." },
                "row": { "type": "integer", "minimum": 0 },
                "subrow": { "type": "integer", "minimum": 0, "description": "Defaults to 0." },
                "column": { "type": "integer", "minimum": 0, "description": description },
            },
            "required": ["sheet", "row"],
            "additionalProperties": false,
        })
    };
    vec![
        ToolDefinition {
            name: "project_overview",
            description: "Languages, game version, sheet count, and overall translation progress of the open project.",
            parameters: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        list_sheets_definition(),
        ToolDefinition {
            name: "read_rows",
            description: "Reads source rows of one sheet in order with their translations. `limit` counts scanned source rows, so fewer rows can come back; continue from `nextAfter`.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "sheet": { "type": "string" },
                    "after_row": { "type": "integer", "minimum": 0, "description": "Exclusive cursor row from nextAfter." },
                    "after_subrow": { "type": "integer", "minimum": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_READ_ROWS, "description": "Defaults to 20." },
                    "state": {
                        "type": "string",
                        "enum": ["all", "untranslated", "translated", "draft", "needsReview", "reviewed"],
                        "description": "Keep only cells in this state. Defaults to all.",
                    },
                },
                "required": ["sheet"],
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "get_unit",
            description: "One source row with every translatable string, its translation, review state, note, and read-only context cells.",
            parameters: location("Only this column; omit for all cells of the row."),
        },
        ToolDefinition {
            name: "other_languages",
            description: "The source text of a row's translatable strings as the game's other client languages (Japanese, the original, and the English, German, and French localizations) write it, to see what the writers meant and how each language words a string and places its tags. Context only: translate from the project's source language and keep its tags; null means that language has no such string.",
            parameters: location("Only this column; omit for all cells of the row."),
        },
        ToolDefinition {
            name: "get_guidance",
            description: "The project's translation guidance and glossary: entries matching the given terms, or the start of the glossary, with any problems in those files.",
            parameters: json!({
                "type": "object",
                "properties": { "terms": { "type": "array", "maxItems": 50, "items": { "type": "string" }, "description": "Terms or parts of terms to look up." } },
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "pending_changes",
            description: "Uncommitted translation changes in the project's Git working tree.",
            parameters: json!({
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Defaults to 50." } },
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "unit_history",
            description: "Committed history of one translation unit, by the unitId from get_unit or read_rows.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "unit_id": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "description": "Defaults to 10." },
                },
                "required": ["unit_id"],
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "navigate_to",
            description: "Opens a string in the user's editor so they can see it. Changes nothing in the project.",
            parameters: location("The string's column; omit for the row's first string."),
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListSheetsArgs {
    query: Option<String>,
    #[serde(default)]
    untranslated_only: bool,
    group_by: Option<SheetGroup>,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
enum SheetGroup {
    Folder,
    Prefix,
}

impl SheetGroup {
    /// The group of a sheet: its folder, or its folder and the first three
    /// letters of its own name.
    fn of(self, sheet: &str) -> String {
        let (folder, name) = sheet.rsplit_once('/').unwrap_or(("(no folder)", sheet));
        match self {
            Self::Folder => folder.to_owned(),
            Self::Prefix => format!("{folder}/{}", name.chars().take(3).collect::<String>()),
        }
    }
}

/// Totals of a group of sheets.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SheetGroupSummary {
    group: String,
    sheets: u64,
    translatable: u64,
    translated: u64,
    reviewed: u64,
    needs_review: u64,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
enum StateFilter {
    All,
    Untranslated,
    Translated,
    Draft,
    NeedsReview,
    Reviewed,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRowsArgs {
    sheet: String,
    after_row: Option<u32>,
    after_subrow: Option<u16>,
    limit: Option<u32>,
    state: Option<StateFilter>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocationArgs {
    sheet: String,
    row: u32,
    subrow: Option<u16>,
    column: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingArgs {
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryArgs {
    unit_id: String,
    limit: Option<u32>,
}

/// The outcome of one tool call as it is sent back to the model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

/// Executes the read-only tools against a [`ProjectReader`].
pub struct ReadTools<'a> {
    reader: &'a dyn ProjectReader,
    writer: Option<&'a dyn ProjectWriter>,
    jobs: Option<&'a dyn JobControl>,
    search: Option<&'a dyn ProjectSearch>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TranslationArgs {
    sheet: String,
    row: u32,
    subrow: u16,
    column: u32,
    target: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposeArgs {
    translations: Vec<TranslationArgs>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuidanceArgs {
    #[serde(default)]
    terms: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GlossaryEntryArgs {
    term: String,
    translation: String,
    note: Option<String>,
    #[serde(default)]
    forbidden: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GlossaryChangeArgs {
    #[serde(default)]
    add: Vec<GlossaryEntryArgs>,
    #[serde(default)]
    remove: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewStringArgs {
    sheet: String,
    row: u32,
    subrow: Option<u16>,
    column: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewArgs {
    strings: Vec<ReviewStringArgs>,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuidanceChangeArgs {
    text: String,
}

impl<'a> ReadTools<'a> {
    #[must_use]
    pub fn new(reader: &'a dyn ProjectReader) -> Self {
        Self {
            reader,
            writer: None,
            jobs: None,
            search: None,
        }
    }

    /// Adds the search and translation-memory tools.
    #[must_use]
    pub fn with_search(mut self, search: &'a dyn ProjectSearch) -> Self {
        self.search = Some(search);
        self
    }

    /// Adds the job tools; changing jobs also needs a writer.
    #[must_use]
    pub fn with_jobs(mut self, jobs: &'a dyn JobControl) -> Self {
        self.jobs = Some(jobs);
        self
    }

    /// Adds the tools that propose changes.
    #[must_use]
    pub fn with_writer(reader: &'a dyn ProjectReader, writer: &'a dyn ProjectWriter) -> Self {
        Self {
            reader,
            writer: Some(writer),
            jobs: None,
            search: None,
        }
    }

    /// Runs one tool call. Unknown tools, invalid arguments, and reader
    /// failures become error outputs for the model.
    #[must_use]
    pub fn execute(&self, name: &str, arguments: &str) -> ToolOutput {
        match self.dispatch(name, arguments) {
            Ok(value) => ToolOutput {
                content: bounded_json(&value),
                is_error: false,
            },
            Err(error) => ToolOutput {
                content: json!({ "error": error.0 }).to_string(),
                is_error: true,
            },
        }
    }

    /// Adds the kinds of text of a job's scope whose style no person chose,
    /// so Angelica can offer a calibration first.
    fn add_style_hint(&self, value: &mut Value, arguments: &str) {
        let Some(root) = self.reader.knowledge_root() else {
            return;
        };
        let parsed: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
        let list = |field: &str| -> Vec<String> {
            parsed
                .get(field)
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default()
        };
        // A pattern stands for the sheets its fixed start names.
        let mut sheets = list("sheets");
        sheets.extend(
            list("patterns")
                .into_iter()
                .map(|pattern| pattern.split('*').next().unwrap_or_default().to_owned()),
        );
        let mut domains: Vec<crate::knowledge::Domain> = Vec::new();
        let dialogue = [
            crate::knowledge::Domain::Journal,
            crate::knowledge::Domain::Objective,
            crate::knowledge::Domain::Dialogue,
        ];
        if sheets.is_empty() {
            domains.extend(dialogue);
        }
        for sheet in &sheets {
            let domain = crate::knowledge::sheet_domain(sheet);
            let found: &[crate::knowledge::Domain] = if domain == crate::knowledge::Domain::Dialogue
            {
                &dialogue
            } else {
                std::slice::from_ref(&domain)
            };
            for domain in found {
                if !domains.contains(domain) {
                    domains.push(*domain);
                }
            }
        }
        let missing = crate::knowledge::Knowledge::load(&root).uncalibrated(&domains);
        if let Some(object) = value.as_object_mut()
            && !missing.is_empty()
        {
            object.insert(
                "uncalibratedStyle".to_owned(),
                json!(
                    missing
                        .iter()
                        .map(|domain| domain.as_str())
                        .collect::<Vec<_>>()
                ),
            );
        }
    }

    /// Runs `get_knowledge`, or `set_knowledge` where changes are allowed.
    fn knowledge_tool(&self, name: &str, arguments: &str) -> Result<Value, ToolError> {
        if name == "set_knowledge" && self.writer.is_none() {
            return Err(ToolError::new(
                "changing the project is not available in Chat mode",
            ));
        }
        let root = self
            .reader
            .knowledge_root()
            .ok_or_else(|| ToolError::new("project knowledge is not available here"))?;
        if name == "set_knowledge" {
            crate::knowledge_tools::set_knowledge(&root, arguments)
        } else {
            crate::knowledge_tools::get_knowledge(&root, arguments)
        }
    }

    fn dispatch(&self, name: &str, arguments: &str) -> Result<Value, ToolError> {
        match name {
            "project_overview" => {
                parse::<Empty>(arguments)?;
                to_value(&self.reader.facts()?)
            }
            "list_sheets" => self.list_sheets(parse(arguments)?),
            "read_rows" => self.read_rows(&parse(arguments)?),
            "get_unit" => self.get_unit(&parse(arguments)?),
            "other_languages" => self.other_languages(&parse(arguments)?),
            "pending_changes" => {
                let args: PendingArgs = parse(arguments)?;
                let limit = args.limit.unwrap_or(50).clamp(1, 200);
                let changes = self.reader.pending_changes()?;
                Ok(match changes {
                    Value::Array(items) => {
                        let total = items.len();
                        json!({ "total": total, "changes": items.into_iter().take(limit).collect::<Vec<_>>() })
                    }
                    other => other,
                })
            }
            "unit_history" => {
                let args: HistoryArgs = parse(arguments)?;
                self.reader
                    .unit_history(args.unit_id.trim(), args.limit.unwrap_or(10).clamp(1, 50))
            }
            "navigate_to" => {
                let args: LocationArgs = parse(arguments)?;
                let location = UnitLocation {
                    sheet: args.sheet,
                    row: args.row,
                    subrow: args.subrow.unwrap_or(0),
                    column: args.column,
                };
                self.reader.navigate(&location)?;
                Ok(json!({ "opened": location }))
            }
            "get_guidance" => Ok(self.get_guidance(&parse(arguments)?)),
            "get_knowledge" | "set_knowledge" => self.knowledge_tool(name, arguments),
            "ask_choice" => crate::ask::ask_choice(arguments),
            "dialogue_context" => dialogue_context(self.reader, &self.guide(), &parse(arguments)?),
            "speaker_lines" => speaker_lines(self.reader, &self.guide(), &parse(arguments)?),
            "list_speakers" => list_speakers(self.reader, &self.guide(), &parse(arguments)?),
            "get_voices" => Ok(get_voices(&self.guide(), &parse(arguments)?)),
            "search_source"
            | "search_translations"
            | "similar_translations"
            | "glossary_candidates" => {
                let Some(search) = self.search else {
                    return Err(ToolError::new("search is not available here"));
                };
                run_search_tool(search, self.reader, name, arguments)
            }
            "estimate_job" | "start_job" | "job_status" | "job_events" | "amend_job"
            | "retry_units" | "pause_job" | "resume_job" | "cancel_job" | "propose_revision"
            | "list_decisions" | "settle_names" | "withdraw_proposal" => {
                let Some(jobs) = self.jobs else {
                    return Err(ToolError::new("translation jobs are not available here"));
                };
                let mut value = run_job_tool(jobs, self.writer.is_some(), name, arguments)?;
                if matches!(name, "estimate_job" | "start_job") {
                    self.add_style_hint(&mut value, arguments);
                }
                Ok(value)
            }
            "validate_target"
            | "propose_translation"
            | "propose_glossary_change"
            | "propose_review"
            | "propose_voice_profile"
            | "propose_guidance_change" => {
                let Some(writer) = self.writer else {
                    return Err(ToolError::new(
                        "changing the project is not available in Chat mode",
                    ));
                };
                match name {
                    "validate_target" => Ok(Self::validate_target(writer, parse(arguments)?)),
                    "propose_translation" => {
                        let guide = self.guide();
                        Self::propose(
                            writer,
                            parse::<ProposeArgs>(arguments)?.translations,
                            guide.glossary.as_ref(),
                        )
                    }
                    "propose_glossary_change" => {
                        self.propose_glossary_change(writer, parse(arguments)?)
                    }
                    "propose_review" => Self::propose_review(writer, parse(arguments)?),
                    "propose_voice_profile" => {
                        propose_voice_profile(self.reader, writer, &parse(arguments)?)
                    }
                    _ => self.propose_guidance_change(writer, &parse(arguments)?),
                }
            }
            other => Err(ToolError::new(format!("unknown tool {other:?}"))),
        }
    }

    /// Checks each string has a translation that is not reviewed yet and
    /// records the rest for the user's approval.
    fn propose_review(writer: &dyn ProjectWriter, args: ReviewArgs) -> Result<Value, ToolError> {
        let reason = args.reason.trim().to_owned();
        if reason.is_empty() {
            return Err(ToolError::new("give a reason the user can check"));
        }
        if args.strings.len() > MAX_REVIEW_ITEMS {
            return Err(ToolError::new(format!(
                "suggest at most {MAX_REVIEW_ITEMS} strings at once"
            )));
        }
        let mut items: Vec<ReviewItem> = Vec::new();
        let mut skipped = Vec::new();
        for string in args.strings {
            let location = UnitLocation {
                sheet: string.sheet,
                row: string.row,
                subrow: string.subrow.unwrap_or(0),
                column: Some(string.column),
            };
            if items.iter().any(|item| item.location == location) {
                continue;
            }
            let problem = match writer.translatable_unit(&location) {
                Ok(unit) => match (unit.state.target, unit.state.review_state) {
                    (_, Some(ReviewLabel::Reviewed)) => Some("already reviewed".to_owned()),
                    (Some(target), _) if !target.trim().is_empty() => {
                        items.push(ReviewItem {
                            location: location.clone(),
                            source: unit.source,
                            target,
                        });
                        None
                    }
                    _ => Some("not translated".to_owned()),
                },
                Err(error) => Some(error.0),
            };
            if let Some(problem) = problem {
                skipped.push(json!({ "location": location, "problem": problem }));
            }
        }
        if items.is_empty() {
            return Ok(json!({ "status": "nothingToApprove", "skipped": skipped }));
        }
        let count = items.len();
        let outcome = writer.propose_review(ReviewBatch { reason, items })?;
        let mut result = match outcome {
            ProposalOutcome::Pending { proposal_id } => json!({
                "status": "awaitingApproval",
                "proposalId": proposal_id,
                "note": "The user approves or rejects the batch; strings changed meanwhile are skipped.",
            }),
            ProposalOutcome::Applied => json!({ "status": "applied" }),
            ProposalOutcome::Conflict { message } | ProposalOutcome::Failed { message } => {
                json!({ "status": "failed", "errors": [message] })
            }
        };
        result["strings"] = json!(count);
        if !skipped.is_empty() {
            result["skipped"] = json!(skipped);
        }
        Ok(result)
    }

    fn list_sheets(&self, args: ListSheetsArgs) -> Result<Value, ToolError> {
        let query = args.query.map(|query| query.trim().to_lowercase());
        let matching: Vec<SheetSummary> = self
            .reader
            .sheets()?
            .into_iter()
            .filter(|sheet| sheet.translatable > 0)
            .filter(|sheet| {
                query
                    .as_deref()
                    .is_none_or(|query| sheet.name.to_lowercase().contains(query))
            })
            .filter(|sheet| !args.untranslated_only || sheet.translated < sheet.translatable)
            .collect();
        let limit = args.limit.unwrap_or(50).clamp(1, 200);
        if let Some(group_by) = args.group_by {
            let mut groups: Vec<SheetGroupSummary> = Vec::new();
            let mut index: HashMap<String, usize> = HashMap::new();
            for sheet in &matching {
                let key = group_by.of(&sheet.name);
                let at = *index.entry(key.clone()).or_insert_with(|| {
                    groups.push(SheetGroupSummary {
                        group: key,
                        sheets: 0,
                        translatable: 0,
                        translated: 0,
                        reviewed: 0,
                        needs_review: 0,
                    });
                    groups.len() - 1
                });
                let group = &mut groups[at];
                group.sheets += 1;
                group.translatable += sheet.translatable;
                group.translated += sheet.translated;
                group.reviewed += sheet.reviewed;
                group.needs_review += sheet.needs_review;
            }
            groups.sort_by(|a, b| a.group.cmp(&b.group));
            let total = groups.len();
            let page: Vec<_> = groups.into_iter().skip(args.offset).take(limit).collect();
            let next_offset =
                (args.offset + page.len() < total).then_some(args.offset + page.len());
            return Ok(json!({ "total": total, "groups": page, "nextOffset": next_offset }));
        }
        let total = matching.len();
        let page: Vec<_> = matching.into_iter().skip(args.offset).take(limit).collect();
        let next_offset = (args.offset + page.len() < total).then_some(args.offset + page.len());
        Ok(json!({ "total": total, "sheets": page, "nextOffset": next_offset }))
    }

    /// Reads the project's guidance and glossary; read failures become
    /// problems rather than errors, so reads never fail because of them.
    fn guide(&self) -> ProjectGuide {
        let read = |file| {
            self.reader
                .project_file(file)
                .map_err(|error| format!("{}: {}", ProjectFile::file_name(file), error.0))
        };
        ProjectGuide::from_files(read(ProjectFile::Guidance), read(ProjectFile::Glossary))
            .with_voices(read(ProjectFile::Voices))
    }

    fn get_guidance(&self, args: &GuidanceArgs) -> Value {
        let guide = self.guide();
        let glossary = guide.glossary.as_ref();
        let entries: Vec<&GlossaryEntry> = match glossary {
            Some(glossary) if args.terms.is_empty() => glossary.entries.iter().collect(),
            Some(glossary) => glossary.lookup(&args.terms),
            None => Vec::new(),
        };
        json!({
            "guidance": guide.guidance,
            "glossary": {
                "total": glossary.map_or(0, |glossary| glossary.entries.len()),
                "entries": entries.iter().take(MAX_GLOSSARY_LOOKUP).collect::<Vec<_>>(),
                "invalidRows": glossary.map(|glossary| glossary.diagnostics.iter().take(50).collect::<Vec<_>>()),
            },
            "problems": guide.problems,
        })
    }

    fn propose_glossary_change(
        &self,
        writer: &dyn ProjectWriter,
        args: GlossaryChangeArgs,
    ) -> Result<Value, ToolError> {
        if args.add.len() > MAX_GLOSSARY_CHANGES || args.remove.len() > MAX_GLOSSARY_CHANGES {
            return Err(ToolError::new(format!(
                "change at most {MAX_GLOSSARY_CHANGES} entries at a time"
            )));
        }
        let before = self.reader.project_file(ProjectFile::Glossary)?;
        let pending = writer.pending_file_change(ProjectFile::Glossary)?;
        let base = pending
            .as_ref()
            .map_or(before.as_deref(), |pending| Some(pending.after.as_str()));
        let add = args
            .add
            .into_iter()
            .map(|entry| GlossaryEntry {
                term: entry.term,
                translation: entry.translation,
                note: entry.note.filter(|note| !note.trim().is_empty()),
                forbidden: entry.forbidden,
            })
            .collect();
        let after = change_glossary(base, add, &args.remove).map_err(ToolError::new)?;
        if base == Some(after.as_str()) {
            return Err(ToolError::new(format!("{GLOSSARY_FILE} would not change")));
        }
        Self::submit_file_change(
            writer,
            ProjectFile::Glossary,
            before,
            after,
            pending.map(|pending| pending.proposal_id),
        )
    }

    fn propose_guidance_change(
        &self,
        writer: &dyn ProjectWriter,
        args: &GuidanceChangeArgs,
    ) -> Result<Value, ToolError> {
        let text = args.text.trim();
        if text.is_empty() {
            return Err(ToolError::new("the guidance must not be empty"));
        }
        let after = format!("{text}\n");
        if after.len() as u64 > ProjectFile::Guidance.max_bytes() {
            return Err(ToolError::new("the guidance is too long"));
        }
        let before = self.reader.project_file(ProjectFile::Guidance)?;
        Self::submit_file_change(writer, ProjectFile::Guidance, before, after, None)
    }

    fn submit_file_change(
        writer: &dyn ProjectWriter,
        file: ProjectFile,
        before: Option<String>,
        after: String,
        replaces: Option<String>,
    ) -> Result<Value, ToolError> {
        // Glossary changes build on a pending change of the conversation;
        // guidance is replaced as a whole text.
        let note = if file == ProjectFile::Guidance {
            "Wait until the user applies or rejects this change before proposing another guidance change; a change made before then would conflict with it."
        } else {
            "You may propose more changes to this file before the user decides: a new change starts from this one, includes it, and replaces it, so the user approves a single change."
        };
        let replaced_proposal = replaces.clone();
        if before.as_deref() == Some(after.as_str()) {
            return Err(ToolError::new(format!(
                "{} would not change",
                file.file_name()
            )));
        }
        Ok(
            match writer.propose_file_change(FileChange {
                file,
                before,
                after,
                replaces,
            })? {
                ProposalOutcome::Pending { proposal_id } => {
                    json!({
                        "status": "awaitingApproval",
                        "proposalId": proposal_id,
                        "file": file.file_name(),
                        "replaced": replaced_proposal,
                        "note": note,
                    })
                }
                ProposalOutcome::Applied => {
                    json!({ "status": "applied", "file": file.file_name() })
                }
                ProposalOutcome::Conflict { message } | ProposalOutcome::Failed { message } => {
                    json!({ "status": "failed", "errors": [message] })
                }
            },
        )
    }

    fn read_rows(&self, args: &ReadRowsArgs) -> Result<Value, ToolError> {
        let after = match (args.after_row, args.after_subrow) {
            (Some(row), subrow) => Some((row, subrow.unwrap_or(0))),
            (None, None) => None,
            (None, Some(_)) => {
                return Err(ToolError::new("after_subrow requires after_row"));
            }
        };
        let limit = args.limit.unwrap_or(20).clamp(1, MAX_READ_ROWS);
        let filter = args.state.unwrap_or(StateFilter::All);
        let page = self.reader.rows(&args.sheet, after, limit)?;
        let guide = self.guide();
        let rows: Vec<RowSnapshot> = page
            .rows
            .into_iter()
            .filter_map(|mut row| {
                row.cells.retain(|cell| matches_filter(cell, filter));
                (!row.cells.is_empty()).then(|| bound_row(row, guide.glossary.as_ref()))
            })
            .collect();
        Ok(json!({
            "sheet": args.sheet,
            "rows": rows,
            "nextAfter": page.next_after.map(|(row, subrow)| json!({ "row": row, "subrow": subrow })),
        }))
    }

    fn get_unit(&self, args: &LocationArgs) -> Result<Value, ToolError> {
        let subrow = args.subrow.unwrap_or(0);
        let Some(mut row) = self.reader.row(&args.sheet, args.row, subrow)? else {
            return Err(ToolError::new(format!(
                "{}:{}:{} has no translatable string",
                args.sheet, args.row, subrow
            )));
        };
        if let Some(column) = args.column {
            row.cells.retain(|cell| cell.column == column);
            if row.cells.is_empty() {
                return Err(ToolError::new(format!(
                    "column {column} of {}:{}:{subrow} is not a translatable string",
                    args.sheet, args.row
                )));
            }
        }
        let guide = self.guide();
        Ok(json!({ "sheet": args.sheet, "row": bound_row(row, guide.glossary.as_ref()) }))
    }

    fn other_languages(&self, args: &LocationArgs) -> Result<Value, ToolError> {
        let subrow = args.subrow.unwrap_or(0);
        let Some(row) = self.reader.row(&args.sheet, args.row, subrow)? else {
            return Err(ToolError::new(format!(
                "{}:{}:{} has no translatable string",
                args.sheet, args.row, subrow
            )));
        };
        let mut cells = Vec::new();
        for cell in row
            .cells
            .into_iter()
            .filter(|cell| args.column.is_none_or(|column| column == cell.column))
        {
            let languages: serde_json::Map<String, Value> = self
                .reader
                .other_languages(&args.sheet, args.row, subrow, cell.column)?
                .into_iter()
                .map(|(language, text)| {
                    let text = text.map_or(Value::Null, |mut text| {
                        bound_text(&mut text);
                        Value::String(text)
                    });
                    (language, text)
                })
                .collect();
            let mut source = cell.source;
            bound_text(&mut source);
            cells.push(json!({ "column": cell.column, "source": source, "languages": languages }));
        }
        if cells.is_empty() {
            return Err(ToolError::new(format!(
                "column {} of {}:{}:{subrow} is not a translatable string",
                args.column.unwrap_or_default(),
                args.sheet,
                args.row
            )));
        }
        Ok(json!({ "sheet": args.sheet, "row": args.row, "subrow": subrow, "cells": cells }))
    }
}

impl ReadTools<'_> {
    fn prepare(
        writer: &dyn ProjectWriter,
        args: TranslationArgs,
    ) -> Result<Proposal, (UnitLocation, Vec<String>)> {
        let location = UnitLocation {
            sheet: args.sheet,
            row: args.row,
            subrow: args.subrow,
            column: Some(args.column),
        };
        let unit = match writer.translatable_unit(&location) {
            Ok(unit) => unit,
            Err(error) => return Err((location, vec![error.0])),
        };
        match aeria_se::check_assisted_structure(&unit.source, &args.target) {
            Ok(()) => Ok(Proposal {
                location,
                source: unit.source,
                target: args.target,
                expected: unit.state,
            }),
            Err(errors) => Err((
                location,
                errors.into_iter().map(|error| error.message).collect(),
            )),
        }
    }

    fn validate_target(writer: &dyn ProjectWriter, args: TranslationArgs) -> Value {
        match Self::prepare(writer, args) {
            Ok(_) => json!({ "valid": true }),
            Err((_, errors)) => json!({ "valid": false, "errors": errors }),
        }
    }

    fn propose(
        writer: &dyn ProjectWriter,
        translations: Vec<TranslationArgs>,
        glossary: Option<&Glossary>,
    ) -> Result<Value, ToolError> {
        if translations.is_empty() || translations.len() > MAX_PROPOSALS_PER_CALL {
            return Err(ToolError::new(format!(
                "propose between 1 and {MAX_PROPOSALS_PER_CALL} translations per call"
            )));
        }
        let mut results: Vec<Value> = Vec::with_capacity(translations.len());
        let mut valid = Vec::new();
        let mut slots = Vec::new();
        for args in translations {
            match Self::prepare(writer, args) {
                Ok(proposal) => {
                    slots.push(results.len());
                    let mut entry = json!({ "location": proposal.location });
                    let warnings = glossary
                        .map(|glossary| glossary.check(&proposal.source, &proposal.target))
                        .unwrap_or_default();
                    if !warnings.is_empty() {
                        entry["glossaryWarnings"] = json!(warnings);
                    }
                    results.push(entry);
                    valid.push(proposal);
                }
                Err((location, errors)) => {
                    results.push(
                        json!({ "location": location, "status": "rejected", "errors": errors }),
                    );
                }
            }
        }
        if !valid.is_empty() {
            let outcomes = writer.submit(valid)?;
            for (slot, outcome) in slots.into_iter().zip(outcomes) {
                let entry = &mut results[slot];
                match outcome {
                    ProposalOutcome::Applied => entry["status"] = json!("applied"),
                    ProposalOutcome::Pending { proposal_id } => {
                        entry["status"] = json!("awaitingApproval");
                        entry["proposalId"] = json!(proposal_id);
                    }
                    ProposalOutcome::Conflict { message } => {
                        entry["status"] = json!("conflict");
                        entry["errors"] = json!([message]);
                    }
                    ProposalOutcome::Failed { message } => {
                        entry["status"] = json!("failed");
                        entry["errors"] = json!([message]);
                    }
                }
            }
        }
        let count = |status: &str| {
            results
                .iter()
                .filter(|entry| entry["status"] == status)
                .count()
        };
        Ok(json!({
            "applied": count("applied"),
            "awaitingApproval": count("awaitingApproval"),
            "rejected": count("rejected"),
            "results": results,
        }))
    }
}

fn matches_filter(cell: &CellSnapshot, filter: StateFilter) -> bool {
    match filter {
        StateFilter::All => true,
        StateFilter::Untranslated => cell.target.is_none(),
        StateFilter::Translated => cell.target.is_some(),
        StateFilter::Draft => cell.review_state == Some(ReviewLabel::Draft),
        StateFilter::NeedsReview => cell.review_state == Some(ReviewLabel::NeedsReview),
        StateFilter::Reviewed => cell.review_state == Some(ReviewLabel::Reviewed),
    }
}

fn bound_row(mut row: RowSnapshot, glossary: Option<&Glossary>) -> RowSnapshot {
    for cell in &mut row.cells {
        if let Some(glossary) = glossary {
            cell.glossary = glossary
                .matches(&cell.source)
                .into_iter()
                .take(MAX_GLOSSARY_PER_CELL)
                .cloned()
                .collect();
        }
        match aeria_se::constructs(&cell.source) {
            Ok(constructs) => {
                cell.constructs = constructs.iter().map(aeria_se::Construct::legend).collect();
            }
            Err(_) => cell.malformed = true,
        }
        if cell.source.chars().count() > MAX_SOURCE_CHARS {
            cell.source = cell
                .source
                .chars()
                .take(MAX_SOURCE_CHARS)
                .collect::<String>()
                + "…[truncated]";
        }
        if let Some(target) = &mut cell.target {
            bound_text(target);
        }
        if let Some(note) = &mut cell.note {
            bound_text(note);
        }
    }
    for cell in &mut row.context {
        bound_text(&mut cell.source);
    }
    row
}

pub(crate) fn bound_text(text: &mut String) {
    if text.chars().count() > MAX_CELL_TEXT_CHARS {
        *text = text.chars().take(MAX_CELL_TEXT_CHARS).collect::<String>() + "…[truncated]";
    }
}

pub(crate) fn parse<T: for<'de> Deserialize<'de>>(arguments: &str) -> Result<T, ToolError> {
    let arguments = if arguments.trim().is_empty() {
        "{}"
    } else {
        arguments
    };
    serde_json::from_str(arguments)
        .map_err(|error| ToolError::new(format!("invalid arguments: {error}")))
}

pub(crate) fn to_value<T: Serialize>(value: &T) -> Result<Value, ToolError> {
    serde_json::to_value(value).map_err(|error| ToolError::new(error.to_string()))
}

/// Serializes a result, cutting it at [`MAX_RESULT_CHARS`] with a notice so
/// one call cannot flood the context.
#[must_use]
pub fn bounded_json(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() <= MAX_RESULT_CHARS {
        return text;
    }
    let cut: String = text.chars().take(MAX_RESULT_CHARS).collect();
    format!("{cut}\n[result truncated at {MAX_RESULT_CHARS} characters; request a smaller page]")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct FakeReader {
        navigated: Mutex<Vec<UnitLocation>>,
    }

    fn cell(column: u32, target: Option<&str>, state: Option<ReviewLabel>) -> CellSnapshot {
        CellSnapshot {
            column,
            source: format!("Source {column}"),
            formatting_only: false,
            target: target.map(str::to_owned),
            review_state: state,
            note: None,
            unit_id: target.map(|_| format!("unit-{column}")),
            constructs: Vec::new(),
            malformed: false,
            glossary: Vec::new(),
        }
    }

    impl ProjectReader for FakeReader {
        fn facts(&self) -> Result<ProjectFacts, ToolError> {
            Ok(ProjectFacts {
                source_language: "en".to_owned(),
                target_language: Some("ru".to_owned()),
                game_version: "2026.09.01".to_owned(),
                sheet_count: 3,
                translatable_strings: 30,
                translated: 5,
                reviewed: 1,
                needs_review: 0,
                detached_units: 0,
            })
        }

        fn sheets(&self) -> Result<Vec<SheetSummary>, ToolError> {
            Ok(["Addon", "Item", "Quest/Main"]
                .iter()
                .enumerate()
                .map(|(index, name)| SheetSummary {
                    name: (*name).to_owned(),
                    translatable: if index == 0 { 0 } else { 10 },
                    translated: if index == 1 { 10 } else { 0 },
                    reviewed: 0,
                    needs_review: 0,
                })
                .collect())
        }

        fn rows(
            &self,
            sheet: &str,
            after: Option<(u32, u16)>,
            limit: u32,
        ) -> Result<RowsPage, ToolError> {
            if sheet != "Item" {
                return Err(ToolError::new(format!("unknown sheet {sheet:?}")));
            }
            assert_eq!(after, Some((4, 0)));
            assert_eq!(limit, MAX_READ_ROWS);
            Ok(RowsPage {
                rows: vec![RowSnapshot {
                    row: 5,
                    subrow: 0,
                    cells: vec![
                        cell(0, Some("Меч"), Some(ReviewLabel::Draft)),
                        cell(1, None, None),
                    ],
                    context: Vec::new(),
                }],
                next_after: Some((5, 0)),
            })
        }

        fn row(
            &self,
            _sheet: &str,
            row: u32,
            subrow: u16,
        ) -> Result<Option<RowSnapshot>, ToolError> {
            Ok((row == 5).then(|| RowSnapshot {
                row,
                subrow,
                cells: vec![
                    cell(0, Some(&"я".repeat(3000)), Some(ReviewLabel::Reviewed)),
                    cell(1, None, None),
                ],
                context: vec![ContextCell {
                    column: 2,
                    source: "context".to_owned(),
                }],
            }))
        }

        fn other_languages(
            &self,
            _sheet: &str,
            _row: u32,
            _subrow: u16,
            column: u32,
        ) -> Result<Vec<(String, Option<String>)>, ToolError> {
            Ok(vec![
                ("ja".to_owned(), Some(format!("ソース {column}"))),
                ("de".to_owned(), Some("q".repeat(3000))),
                ("fr".to_owned(), None),
            ])
        }

        fn pending_changes(&self) -> Result<Value, ToolError> {
            Ok(json!([{ "id": 1 }, { "id": 2 }, { "id": 3 }]))
        }

        fn unit_history(&self, unit_id: &str, limit: u32) -> Result<Value, ToolError> {
            Ok(json!({ "unitId": unit_id, "limit": limit }))
        }

        fn navigate(&self, location: &UnitLocation) -> Result<(), ToolError> {
            self.navigated.lock().expect("lock").push(location.clone());
            Ok(())
        }

        fn project_file(&self, file: ProjectFile) -> Result<Option<String>, ToolError> {
            Ok(match file {
                ProjectFile::Guidance => Some("Use informal address.".to_owned()),
                ProjectFile::Glossary => {
                    Some("term,translation,forbidden\nSource 0,Исходник,Сорс\n".to_owned())
                }
                ProjectFile::Voices => None,
            })
        }

        fn dialogue(&self, _: &str) -> Result<Option<crate::dialogue::SheetDialogue>, ToolError> {
            Ok(None)
        }

        fn speakers(&self, _: &str) -> Result<Vec<(String, usize)>, ToolError> {
            Ok(Vec::new())
        }

        fn speaker_lines(
            &self,
            _: &str,
            _: usize,
            _: usize,
        ) -> Result<(usize, Vec<UnitLocation>), ToolError> {
            Ok((0, Vec::new()))
        }
    }

    fn reader() -> FakeReader {
        FakeReader {
            navigated: Mutex::new(Vec::new()),
        }
    }

    fn run(reader: &FakeReader, name: &str, arguments: &str) -> (Value, bool) {
        let output = ReadTools::new(reader).execute(name, arguments);
        (
            serde_json::from_str(&output.content).expect("json output"),
            output.is_error,
        )
    }

    #[test]
    fn every_definition_has_an_object_schema_and_a_handler() {
        let reader = reader();
        for tool in read_tool_definitions() {
            assert_eq!(tool.parameters["type"], "object", "{}", tool.name);
            let output = ReadTools::new(&reader).execute(tool.name, "{}");
            assert!(
                !output.content.contains("unknown tool"),
                "{} has no handler",
                tool.name
            );
        }
    }

    #[test]
    fn list_sheets_hides_empty_sheets_and_filters_by_name_and_progress() {
        let reader = reader();
        let (value, _) = run(&reader, "list_sheets", "{}");
        assert_eq!(value["total"], 2);
        let (value, _) = run(&reader, "list_sheets", r#"{"query":"quest"}"#);
        assert_eq!(value["sheets"][0]["name"], "Quest/Main");
        let (value, _) = run(&reader, "list_sheets", r#"{"untranslated_only":true}"#);
        assert_eq!(value["total"], 1);
        let (value, _) = run(&reader, "list_sheets", r#"{"limit":1}"#);
        assert_eq!(value["nextOffset"], 1);
        let (value, _) = run(&reader, "list_sheets", r#"{"group_by":"folder"}"#);
        assert_eq!(value["total"], 2);
        assert_eq!(value["groups"][0]["group"], "(no folder)");
        assert_eq!(value["groups"][1]["group"], "Quest");
        let (value, _) = run(&reader, "list_sheets", r#"{"group_by":"prefix"}"#);
        assert_eq!(value["groups"][1]["group"], "Quest/Mai");
        assert_eq!(value["groups"][1]["sheets"], 1);
    }

    #[test]
    fn read_rows_clamps_the_limit_and_filters_cells() {
        let reader = reader();
        let (value, error) = run(
            &reader,
            "read_rows",
            r#"{"sheet":"Item","after_row":4,"limit":999,"state":"untranslated"}"#,
        );
        assert!(!error);
        let cells = value["rows"][0]["cells"].as_array().expect("cells");
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0]["column"], 1);
        assert!(cells[0].get("target").is_none());
        assert_eq!(value["nextAfter"], json!({ "row": 5, "subrow": 0 }));
    }

    #[test]
    fn other_languages_lists_each_cell_in_the_other_client_languages() {
        let reader = reader();
        let (value, error) = run(&reader, "other_languages", r#"{"sheet":"Item","row":5}"#);
        assert!(!error);
        let cells = value["cells"].as_array().expect("cells");
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[1]["column"], 1);
        assert_eq!(cells[1]["languages"]["ja"], "ソース 1");
        assert!(cells[1]["languages"]["fr"].is_null());
        assert!(
            cells[0]["languages"]["de"]
                .as_str()
                .expect("text")
                .ends_with("…[truncated]")
        );
        let (value, _) = run(
            &reader,
            "other_languages",
            r#"{"sheet":"Item","row":5,"column":1}"#,
        );
        assert_eq!(value["cells"].as_array().expect("cells").len(), 1);
        assert!(
            run(
                &reader,
                "other_languages",
                r#"{"sheet":"Item","row":5,"column":9}"#
            )
            .1
        );
        assert!(run(&reader, "other_languages", r#"{"sheet":"Item","row":6}"#).1);
    }

    #[test]
    fn get_unit_bounds_long_text_and_selects_a_column() {
        let reader = reader();
        let (value, _) = run(
            &reader,
            "get_unit",
            r#"{"sheet":"Item","row":5,"column":0}"#,
        );
        let target = value["row"]["cells"][0]["target"].as_str().expect("target");
        assert!(target.ends_with("…[truncated]"));
        assert_eq!(
            target.chars().count(),
            MAX_CELL_TEXT_CHARS + "…[truncated]".chars().count()
        );
        assert_eq!(value["row"]["cells"].as_array().expect("cells").len(), 1);
        assert_eq!(value["row"]["cells"][0]["reviewState"], "reviewed");

        let (value, error) = run(&reader, "get_unit", r#"{"sheet":"Item","row":6}"#);
        assert!(error);
        assert!(
            value["error"]
                .as_str()
                .expect("error")
                .contains("no translatable string")
        );
    }

    #[test]
    fn invalid_arguments_and_unknown_tools_become_error_results() {
        let reader = reader();
        for (name, arguments) in [
            ("get_unit", r#"{"sheet":"Item"}"#),
            ("get_unit", r#"{"sheet":"Item","row":-1}"#),
            ("list_sheets", r#"{"unexpected":1}"#),
            ("read_rows", r#"{"sheet":"Item","after_subrow":1}"#),
            ("read_rows", r#"{"sheet":"Missing"}"#),
            ("write_everything", "{}"),
            ("project_overview", "not json"),
        ] {
            let (value, error) = run(&reader, name, arguments);
            assert!(error, "{name} {arguments}");
            assert!(value["error"].is_string());
        }
    }

    #[test]
    fn pending_changes_are_limited_and_navigation_is_forwarded() {
        let reader = reader();
        let (value, _) = run(&reader, "pending_changes", r#"{"limit":2}"#);
        assert_eq!(value["total"], 3);
        assert_eq!(value["changes"].as_array().expect("changes").len(), 2);

        let (value, error) = run(&reader, "navigate_to", r#"{"sheet":"Item","row":5}"#);
        assert!(!error);
        assert_eq!(value["opened"]["subrow"], 0);
        assert_eq!(reader.navigated.lock().expect("lock").len(), 1);
    }

    #[derive(Default)]
    struct FakeWriter {
        submitted: Mutex<Vec<Proposal>>,
        files: Mutex<Vec<FileChange>>,
        reviews: Mutex<Vec<ReviewBatch>>,
    }

    #[test]
    fn review_suggestions_keep_translated_unreviewed_strings() {
        let reader = reader();
        let writer = FakeWriter::default();
        let tools = ReadTools::with_writer(&reader, &writer);
        let output = tools.execute(
            "propose_review",
            r#"{"reason":" Checked terms. ","strings":[{"sheet":"Item","row":2,"column":0},{"sheet":"Item","row":2,"column":0},{"sheet":"Item","row":1,"column":0},{"sheet":"Item","row":9,"column":0}]}"#,
        );
        assert!(!output.is_error, "{}", output.content);
        assert!(output.content.contains("awaitingApproval"));
        assert!(output.content.contains("\"strings\":1"));
        assert!(output.content.contains("not translated"));
        let reviews = writer.reviews.lock().expect("lock");
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].reason, "Checked terms.");
        assert_eq!(reviews[0].items[0].target, "Пока");
        drop(reviews);

        let nothing = tools.execute(
            "propose_review",
            r#"{"reason":"x","strings":[{"sheet":"Item","row":1,"column":0}]}"#,
        );
        assert!(nothing.content.contains("nothingToApprove"));
        assert!(
            tools
                .execute(
                    "propose_review",
                    r#"{"reason":" ","strings":[{"sheet":"Item","row":2,"column":0}]}"#
                )
                .is_error
        );
        assert!(
            ReadTools::new(&reader)
                .execute("propose_review", r#"{"reason":"x","strings":[]}"#)
                .is_error
        );
    }

    impl ProjectWriter for FakeWriter {
        fn translatable_unit(
            &self,
            location: &UnitLocation,
        ) -> Result<TranslatableUnit, ToolError> {
            match location.row {
                1 => Ok(TranslatableUnit {
                    source: "Hi <player-name $n1>!".to_owned(),
                    state: UnitState {
                        target: None,
                        review_state: None,
                    },
                }),
                2 => Ok(TranslatableUnit {
                    source: "Bye".to_owned(),
                    state: UnitState {
                        target: Some("Пока".to_owned()),
                        review_state: Some(ReviewLabel::Draft),
                    },
                }),
                _ => Err(ToolError::new("not a translatable string")),
            }
        }

        fn propose_file_change(&self, change: FileChange) -> Result<ProposalOutcome, ToolError> {
            let mut files = self.files.lock().expect("lock");
            files.push(change);
            Ok(ProposalOutcome::Pending {
                proposal_id: format!("f-{}", files.len()),
            })
        }

        /// Every recorded glossary change stays pending.
        fn pending_file_change(
            &self,
            file: ProjectFile,
        ) -> Result<Option<PendingFileChange>, ToolError> {
            let files = self.files.lock().expect("lock");
            Ok(files
                .iter()
                .enumerate()
                .rev()
                .find(|(_, change)| change.file == file && file == ProjectFile::Glossary)
                .map(|(index, change)| PendingFileChange {
                    proposal_id: format!("f-{}", index + 1),
                    after: change.after.clone(),
                }))
        }

        fn propose_review(&self, batch: ReviewBatch) -> Result<ProposalOutcome, ToolError> {
            self.reviews.lock().expect("lock").push(batch);
            Ok(ProposalOutcome::Pending {
                proposal_id: "r-1".to_owned(),
            })
        }

        fn submit(&self, proposals: Vec<Proposal>) -> Result<Vec<ProposalOutcome>, ToolError> {
            let outcomes = proposals
                .iter()
                .map(|proposal| {
                    if proposal.expected.target.is_none() {
                        ProposalOutcome::Applied
                    } else {
                        ProposalOutcome::Pending {
                            proposal_id: "p-1".to_owned(),
                        }
                    }
                })
                .collect();
            self.submitted.lock().expect("lock").extend(proposals);
            Ok(outcomes)
        }
    }

    #[test]
    fn proposals_are_validated_rebuilt_and_submitted() {
        let reader = reader();
        let writer = FakeWriter {
            submitted: Mutex::new(Vec::new()),
            files: Mutex::new(Vec::new()),
            reviews: Mutex::new(Vec::new()),
        };
        let tools = ReadTools::with_writer(&reader, &writer);
        let output = tools.execute(
            "propose_translation",
            r#"{"translations":[
                {"sheet":"Item","row":1,"subrow":0,"column":0,"target":"Привет, <player-name $n1>!"},
                {"sheet":"Item","row":2,"subrow":0,"column":0,"target":"До встречи"},
                {"sheet":"Item","row":1,"subrow":0,"column":0,"target":"Привет!"},
                {"sheet":"Item","row":9,"subrow":0,"column":0,"target":"x"}
            ]}"#,
        );
        assert!(!output.is_error, "{}", output.content);
        let value: Value = serde_json::from_str(&output.content).expect("json");
        assert_eq!(value["applied"], 1);
        assert_eq!(value["awaitingApproval"], 1);
        assert_eq!(value["rejected"], 2);
        assert_eq!(value["results"][1]["proposalId"], "p-1");
        assert!(
            value["results"][2]["errors"][0]
                .as_str()
                .expect("error")
                .contains("<player-name $n1> of the source is missing")
        );
        let submitted = writer.submitted.lock().expect("lock");
        assert_eq!(submitted[0].target, "Привет, <player-name $n1>!");
        assert_eq!(submitted[1].expected.target.as_deref(), Some("Пока"));
    }

    #[test]
    fn write_tools_need_a_writer_and_validation_writes_nothing() {
        let reader = reader();
        let output = ReadTools::new(&reader).execute(
            "validate_target",
            r#"{"sheet":"Item","row":1,"subrow":0,"column":0,"target":"x"}"#,
        );
        assert!(output.is_error);
        assert!(output.content.contains("Chat mode"));

        let writer = FakeWriter {
            submitted: Mutex::new(Vec::new()),
            files: Mutex::new(Vec::new()),
            reviews: Mutex::new(Vec::new()),
        };
        let output = ReadTools::with_writer(&reader, &writer).execute(
            "validate_target",
            r#"{"sheet":"Item","row":1,"subrow":0,"column":0,"target":"Привет, <player-name $n1>"}"#,
        );
        let value: Value = serde_json::from_str(&output.content).expect("json");
        assert_eq!(value["valid"], true);
        assert!(writer.submitted.lock().expect("lock").is_empty());
    }

    #[test]
    fn reads_explain_the_macros_of_their_sources() {
        let row = bound_row(
            RowSnapshot {
                row: 1,
                subrow: 0,
                cells: vec![
                    CellSnapshot {
                        source: "Hi <player-name $n1>".to_owned(),
                        ..cell(0, None, None)
                    },
                    CellSnapshot {
                        source: "Plain".to_owned(),
                        ..cell(1, None, None)
                    },
                    CellSnapshot {
                        source: "<if $n1>".to_owned(),
                        ..cell(2, None, None)
                    },
                ],
                context: Vec::new(),
            },
            None,
        );
        assert_eq!(row.cells[0].source, "Hi <player-name $n1>");
        assert!(row.cells[0].constructs[0].starts_with("<player-name $n1> — "));
        assert!(row.cells[0].constructs[0].contains("game data"));
        assert!(row.cells[1].constructs.is_empty());
        assert!(row.cells[2].malformed);
    }

    #[test]
    fn guidance_glossary_matches_and_file_proposals() {
        let reader = reader();
        let (value, _) = run(&reader, "get_guidance", r#"{"terms":["source"]}"#);
        assert_eq!(value["guidance"], "Use informal address.");
        assert_eq!(value["glossary"]["entries"][0]["translation"], "Исходник");

        let (value, _) = run(
            &reader,
            "get_unit",
            r#"{"sheet":"Item","row":5,"column":1}"#,
        );
        assert!(value["row"]["cells"][0]["glossary"].as_array().is_none());
        let (value, _) = run(
            &reader,
            "get_unit",
            r#"{"sheet":"Item","row":5,"column":0}"#,
        );
        assert_eq!(value["row"]["cells"][0]["glossary"][0]["term"], "Source 0");

        let writer = FakeWriter {
            submitted: Mutex::new(Vec::new()),
            files: Mutex::new(Vec::new()),
            reviews: Mutex::new(Vec::new()),
        };
        let tools = ReadTools::with_writer(&reader, &writer);
        let output = tools.execute(
            "propose_glossary_change",
            r#"{"add":[{"term":"Aether","translation":"Эфир"}],"remove":["source 0"]}"#,
        );
        assert!(!output.is_error, "{}", output.content);
        let output = tools.execute(
            "propose_guidance_change",
            r#"{"text":"  Use formal address. "}"#,
        );
        assert!(!output.is_error, "{}", output.content);
        assert!(output.content.contains("Wait until the user applies"));
        let output = tools.execute(
            "propose_glossary_change",
            r#"{"add":[{"term":"Ishgard","translation":"Ишгард"}]}"#,
        );
        assert!(!output.is_error, "{}", output.content);
        assert!(output.content.contains("\"replaced\":\"f-1\""));
        let files = writer.files.lock().expect("lock");
        assert_eq!(
            files[0].after,
            "term,translation,note,forbidden\nAether,Эфир,,\n"
        );
        assert_eq!(files[1].before.as_deref(), Some("Use informal address."));
        assert_eq!(files[1].after, "Use formal address.\n");
        assert_eq!(
            files[2].after, "term,translation,note,forbidden\nAether,Эфир,,\nIshgard,Ишгард,,\n",
            "a new glossary change builds on the pending one"
        );
        assert_eq!(files[2].before, files[0].before);
        assert_eq!(files[2].replaces.as_deref(), Some("f-1"));
    }

    struct FakeJobs {
        calls: Mutex<Vec<String>>,
    }

    impl JobControl for FakeJobs {
        fn estimate(&self, scope: &JobScope) -> Result<JobEstimate, ToolError> {
            self.calls
                .lock()
                .expect("lock")
                .push(format!("estimate {:?}", scope.filter));
            Ok(JobEstimate {
                units: 30,
                chunks: 2,
                estimated_tokens: 12_000,
                history_chunk_tokens: None,
            })
        }
        fn propose(
            &self,
            scope: JobScope,
            instructions: String,
            concurrency: Option<u8>,
            images: &[String],
            quality: JobQuality,
        ) -> Result<ProposalOutcome, ToolError> {
            let careful = if quality == JobQuality::Careful {
                " careful"
            } else {
                ""
            };
            self.calls.lock().expect("lock").push(format!(
                "propose {:?} {instructions} {concurrency:?} {images:?}{careful}",
                scope.sheets
            ));
            Ok(ProposalOutcome::Pending {
                proposal_id: "job-proposal".to_owned(),
            })
        }
        fn jobs(&self) -> Result<Vec<JobSummary>, ToolError> {
            Ok(Vec::new())
        }
        fn events(&self, _: &str, _: u64) -> Result<Vec<JobEvent>, ToolError> {
            Ok(Vec::new())
        }
        fn units(&self, _: &str, _: &[UnitStatus]) -> Result<Vec<JobUnit>, ToolError> {
            Ok(Vec::new())
        }
        fn amend(&self, _: &str, instructions: &str) -> Result<(), ToolError> {
            self.calls
                .lock()
                .expect("lock")
                .push(format!("amend {instructions}"));
            Ok(())
        }
        fn retry(&self, _: &str, statuses: &[UnitStatus]) -> Result<u64, ToolError> {
            self.calls
                .lock()
                .expect("lock")
                .push(format!("retry {statuses:?}"));
            Ok(4)
        }
        fn set_concurrency(&self, _: &str, concurrency: u8) -> Result<u8, ToolError> {
            self.calls
                .lock()
                .expect("lock")
                .push(format!("workers {concurrency}"));
            Ok(concurrency)
        }
        fn propose_limit(&self, _: &str, token_limit: u64) -> Result<ProposalOutcome, ToolError> {
            self.calls
                .lock()
                .expect("lock")
                .push(format!("limit {token_limit}"));
            Ok(ProposalOutcome::Pending {
                proposal_id: "limit-proposal".to_owned(),
            })
        }
        fn control(&self, _: &str, action: JobAction) -> Result<JobStatus, ToolError> {
            self.calls
                .lock()
                .expect("lock")
                .push(format!("control {action:?}"));
            Ok(JobStatus::Running)
        }
    }

    #[test]
    fn job_tools_estimate_propose_and_retry_only_where_allowed() {
        let reader = reader();
        let jobs = FakeJobs {
            calls: Mutex::new(Vec::new()),
        };
        let chat = ReadTools::new(&reader).with_jobs(&jobs);
        let estimate = chat.execute("estimate_job", r#"{"sheets":["Item"]}"#);
        assert!(!estimate.is_error, "{}", estimate.content);
        assert!(estimate.content.contains("12000"));
        let careful = chat.execute("estimate_job", r#"{"sheets":["Item"],"quality":"careful"}"#);
        assert!(careful.content.contains("24000"), "{}", careful.content);
        let refused = chat.execute("start_job", r#"{"sheets":["Item"]}"#);
        assert!(refused.is_error);
        assert!(refused.content.contains("Chat mode"));

        let writer = FakeWriter {
            submitted: Mutex::new(Vec::new()),
            files: Mutex::new(Vec::new()),
            reviews: Mutex::new(Vec::new()),
        };
        let tools = ReadTools::with_writer(&reader, &writer).with_jobs(&jobs);
        let started = tools.execute(
            "start_job",
            r#"{"sheets":["Item"],"instructions":" Formal. "}"#,
        );
        assert!(
            started.content.contains("awaitingApproval"),
            "{}",
            started.content
        );
        let too_many = tools.execute(
            "start_job",
            r#"{"sheets":["Item"],"images":["a","b","c","d","e"]}"#,
        );
        assert!(too_many.is_error, "{}", too_many.content);
        let deduplicated = tools.execute("start_job", r#"{"sheets":["Item"],"images":["a","a"]}"#);
        assert!(!deduplicated.is_error, "{}", deduplicated.content);
        let retried = tools.execute(
            "retry_units",
            r#"{"job_id":"j1","instructions":"Keep names in Latin."}"#,
        );
        assert!(
            retried.content.contains("\"requeued\":4"),
            "{}",
            retried.content
        );
        assert_eq!(
            jobs.calls.lock().expect("lock").as_slice(),
            [
                "estimate Untranslated",
                "estimate Untranslated",
                "propose [\"Item\"] Formal. None []",
                "propose [\"Item\"]  None [\"a\"]",
                "amend Keep names in Latin.",
                "retry [Rejected, Failed]",
                "control Resume",
            ]
        );
        assert!(ReadTools::new(&reader).execute("job_status", "{}").is_error);
        assert!(
            tools
                .execute("set_job_workers", r#"{"job_id":"j1","concurrency":9}"#)
                .is_error
        );
    }

    #[test]
    fn oversized_results_are_cut_with_a_notice() {
        let text = bounded_json(&json!({ "text": "x".repeat(MAX_RESULT_CHARS * 2) }));
        assert!(text.ends_with("request a smaller page]"));
        assert!(text.chars().count() < MAX_RESULT_CHARS + 100);
    }
}
