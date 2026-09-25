//! Desktop adapter for Angelica conversations.
//!
//! A turn runs as an async task that streams provider responses and runs
//! read-only tools in blocking workers. Each tool call takes the project lock
//! only for its own read, never across a provider request. Progress is sent
//! to the renderer as `angelica://event` events; conversations are stored per
//! project in application data.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use aeria_ai::ModelSelection;
use aeria_ai::agent::{AgentEvent, ToolExecutor, TurnConfig, TurnOutcome, run_turn};
use aeria_ai::chat::{ChatMessage, ToolCall, Usage};
use aeria_ai::conversation::{
    Conversation, ConversationError, ConversationStore, ConversationSummary,
};
use aeria_ai::conversation::{ProposalRecord, ProposalStatus};
use aeria_ai::guidance::{GlossaryEntry, ProjectFile, ProjectGuide, read_project_file};
use aeria_ai::prompt::{AgentMode, EditorContext, system_prompt};
use aeria_ai::search::search_tool_definitions;
use aeria_ai::tools::{
    CellSnapshot, ContextCell, FileChange, ProjectFacts, ProjectReader, ProjectWriter, Proposal,
    ProposalOutcome, ReadTools, ReviewBatch, ReviewLabel, RowSnapshot, RowsPage, SheetSummary,
    ToolError, ToolOutput, TranslatableUnit, UnitLocation, UnitState, job_tool_definitions,
    read_tool_definitions, write_tool_definitions,
};
use aeria_ai::web::web_tool_definitions;
use aeria_core::ReviewState;
use aeria_core::SourceBinding;
use aeria_workspace::{
    AssistedExpectation, AssistedWriteError, ProjectSession, TranslationRowCursor,
    TranslationRowView,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};

use crate::ai::{resolve_endpoint, settings_store};
use crate::commands::{parse_translation_unit_id, run_blocking};
use crate::dto::{ProjectSummaryDto, SourceBindingDto, TranslationOverlayDto};
use crate::error::CommandError;
use crate::git::{UnitChangeDto, UnitHistoryDto, open_repository};
use crate::jobs::{DesktopJobs, start_proposed_job};
use crate::paths::AeriaPaths;
use crate::search::{DesktopSearch, prepare_source_index};
use crate::state::DesktopState;
use crate::web::{allow_domain, fetch_tool};

type CommandResult<T> = Result<T, CommandError>;

/// Renderer event carrying live turn progress.
pub const EVENT: &str = "angelica://event";
/// Renderer event asking the editor to show an occurrence.
pub const NAVIGATE_EVENT: &str = "angelica://navigate";
/// Renderer event carrying a translation Angelica wrote.
pub const APPLIED_EVENT: &str = "angelica://translation-applied";
/// Renderer event saying a conversation's proposals changed.
pub const PROPOSALS_EVENT: &str = "angelica://proposals";

/// A translation written by Angelica, for patching one editor cell.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationAppliedDto {
    pub source_binding: SourceBindingDto,
    pub overlay: TranslationOverlayDto,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProposalsChangedDto {
    conversation_id: String,
}

const CONVERSATIONS_DIRECTORY: &str = "conversations";
/// Longest accepted user message.
const MAX_MESSAGE_CHARS: usize = 32_000;

/// One live event for a conversation.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AngelicaEventDto {
    pub conversation_id: String,
    pub event: AngelicaEventKind,
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum AngelicaEventKind {
    Agent(AgentEvent),
    Turn(TurnEvent),
}

/// The end of a turn.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum TurnEvent {
    #[serde(rename = "turnFinished")]
    Finished { outcome: TurnOutcome, usage: Usage },
    #[serde(rename = "turnFailed")]
    Failed { code: String, message: String },
    #[serde(rename = "turnCancelled")]
    Cancelled,
}

/// A stored conversation with its live status.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDto {
    pub id: String,
    pub title: String,
    pub model: Option<ModelSelection>,
    pub messages: Vec<ChatMessage>,
    pub usage: Usage,
    pub running: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummaryDto {
    pub id: String,
    pub title: String,
    pub updated_at_unix_ms: u64,
    pub running: bool,
}

pub(crate) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

impl From<ConversationError> for CommandError {
    fn from(error: ConversationError) -> Self {
        let code = match &error {
            ConversationError::NotFound { .. } => "angelicaConversationNotFound",
            ConversationError::Io { .. } | ConversationError::Invalid { .. } => {
                "angelicaConversationStorage"
            }
        };
        Self::new(code, error.to_string())
    }
}

/// A key for per-project application data: a hash of the repository root,
/// so paths never appear in file names.
pub(crate) fn project_key(root: &std::path::Path) -> String {
    let digest = Sha256::digest(root.to_string_lossy().as_bytes());
    digest[..16]
        .iter()
        .fold(String::with_capacity(32), |mut key, byte| {
            let _ = write!(key, "{byte:02x}");
            key
        })
}

/// Returns the conversation store of the active project.
pub(crate) fn conversation_store(app: &tauri::AppHandle) -> CommandResult<ConversationStore> {
    let key = project_key(&repository_root(app)?);
    let data = app.aeria_data_dir().map_err(|error| {
        CommandError::new(
            "angelicaConversationStorage",
            format!("could not resolve the Aeria app-data directory: {error}"),
        )
    })?;
    Ok(ConversationStore::new(
        data.join(CONVERSATIONS_DIRECTORY).join(key),
    ))
}

fn conversation_dto(conversation: Conversation, running: bool) -> ConversationDto {
    ConversationDto {
        id: conversation.id,
        title: conversation.title,
        model: conversation.model,
        messages: conversation.messages,
        usage: conversation.usage,
        running,
    }
}

/// Reads the active project for Angelica's tools.
pub(crate) struct DesktopReader {
    pub(crate) app: tauri::AppHandle,
}

impl DesktopReader {
    pub(crate) fn with_session<T>(
        &self,
        read: impl FnOnce(&ProjectSession) -> Result<T, ToolError>,
    ) -> Result<T, ToolError> {
        let state = self.app.state::<DesktopState>();
        let project = state
            .lock_project()
            .map_err(|error| ToolError::new(error.message))?;
        let session = project
            .as_ref()
            .ok_or_else(|| ToolError::new("no project is open"))?;
        read(session)
    }
}

pub(crate) fn review_label(state: ReviewState) -> ReviewLabel {
    match state {
        ReviewState::Draft => ReviewLabel::Draft,
        ReviewState::NeedsReview => ReviewLabel::NeedsReview,
        ReviewState::Reviewed => ReviewLabel::Reviewed,
    }
}

fn row_snapshot(row: TranslationRowView) -> RowSnapshot {
    RowSnapshot {
        row: row.row_id,
        subrow: row.subrow_id,
        cells: row
            .cells
            .into_iter()
            .map(|cell| {
                let (target, review_state, note, unit_id) = match cell.translation {
                    Some(overlay) => (
                        Some(overlay.target_macro),
                        Some(review_label(overlay.review_state)),
                        overlay.translator_note,
                        Some(overlay.translation_unit_id.to_string()),
                    ),
                    None => (None, None, None, None),
                };
                CellSnapshot {
                    column: cell.source_binding.column_index(),
                    source: cell.source_macro,
                    formatting_only: cell.formatting_only,
                    target,
                    review_state,
                    note,
                    unit_id,
                    tagged: None,
                    tags: Vec::new(),
                    untaggable: false,
                    glossary: Vec::new(),
                }
            })
            .collect(),
        context: row
            .context
            .into_iter()
            .map(|cell| ContextCell {
                column: cell.column_index,
                source: cell.source_macro,
            })
            .collect(),
    }
}

pub(crate) fn known_sheet(session: &ProjectSession, sheet: &str) -> Result<(), ToolError> {
    if session
        .source_package()
        .guidance_index()
        .translatable_cell_count(sheet)
        == 0
    {
        return Err(ToolError::new(format!(
            "sheet {sheet:?} does not exist or has no translatable strings; use list_sheets"
        )));
    }
    Ok(())
}

pub(crate) fn session_facts(session: &ProjectSession) -> ProjectFacts {
    let summary = ProjectSummaryDto::from_session(session);
    let progress = session.translation_progress();
    let sum = |field: fn(&aeria_workspace::SheetTranslationProgress) -> usize| {
        progress.iter().map(field).sum::<usize>() as u64
    };
    ProjectFacts {
        source_language: summary.source_language,
        target_language: (summary.target_language != "und").then_some(summary.target_language),
        game_version: summary.game_version,
        sheet_count: summary
            .sheets
            .iter()
            .filter(|sheet| sheet.translatable_cell_count > 0)
            .count(),
        translatable_strings: summary
            .sheets
            .iter()
            .map(|sheet| sheet.translatable_cell_count as u64)
            .sum(),
        translated: sum(|sheet| sheet.translated),
        reviewed: sum(|sheet| sheet.reviewed),
        needs_review: sum(|sheet| sheet.needs_review),
        detached_units: summary.detached_unit_count,
    }
}

fn session_sheets(session: &ProjectSession) -> Vec<SheetSummary> {
    let summary = ProjectSummaryDto::from_session(session);
    let progress: BTreeMap<String, aeria_workspace::SheetTranslationProgress> = session
        .translation_progress()
        .into_iter()
        .map(|sheet| (sheet.sheet_name.clone(), sheet))
        .collect();
    summary
        .sheets
        .into_iter()
        .map(|sheet| {
            let counts = progress.get(&sheet.name);
            SheetSummary {
                translatable: sheet.translatable_cell_count as u64,
                translated: counts.map_or(0, |counts| counts.translated as u64),
                reviewed: counts.map_or(0, |counts| counts.reviewed as u64),
                needs_review: counts.map_or(0, |counts| counts.needs_review as u64),
                name: sheet.name,
            }
        })
        .collect()
}

fn session_rows(
    session: &ProjectSession,
    sheet: &str,
    after: Option<(u32, u16)>,
    limit: u32,
) -> Result<RowsPage, ToolError> {
    known_sheet(session, sheet)?;
    let cursor = after.map(|(row, subrow)| TranslationRowCursor::new(sheet, row, subrow));
    let page = session
        .page_translation_rows(sheet, cursor.as_ref(), limit)
        .map_err(|error| ToolError::new(error.to_string()))?;
    Ok(RowsPage {
        rows: page.rows.into_iter().map(row_snapshot).collect(),
        next_after: page
            .next_after
            .map(|cursor| (cursor.row_id(), cursor.subrow_id())),
    })
}

pub(crate) fn session_row(
    session: &ProjectSession,
    sheet: &str,
    row: u32,
    subrow: u16,
) -> Result<Option<RowSnapshot>, ToolError> {
    known_sheet(session, sheet)?;
    // The page cursor is exclusive, so start just before the row.
    let cursor = match (row, subrow) {
        (0, 0) => None,
        (row, 0) => Some(TranslationRowCursor::new(sheet, row - 1, u16::MAX)),
        (row, subrow) => Some(TranslationRowCursor::new(sheet, row, subrow - 1)),
    };
    let page = session
        .page_translation_rows(sheet, cursor.as_ref(), 1)
        .map_err(|error| ToolError::new(error.to_string()))?;
    Ok(page
        .rows
        .into_iter()
        .find(|view| view.row_id == row && view.subrow_id == subrow)
        .map(row_snapshot))
}

impl ProjectReader for DesktopReader {
    fn facts(&self) -> Result<ProjectFacts, ToolError> {
        self.with_session(|session| Ok(session_facts(session)))
    }

    fn sheets(&self) -> Result<Vec<SheetSummary>, ToolError> {
        self.with_session(|session| Ok(session_sheets(session)))
    }

    fn rows(
        &self,
        sheet: &str,
        after: Option<(u32, u16)>,
        limit: u32,
    ) -> Result<RowsPage, ToolError> {
        self.with_session(|session| session_rows(session, sheet, after, limit))
    }

    fn row(&self, sheet: &str, row: u32, subrow: u16) -> Result<Option<RowSnapshot>, ToolError> {
        self.with_session(|session| session_row(session, sheet, row, subrow))
    }

    fn pending_changes(&self) -> Result<Value, ToolError> {
        let state = self.app.state::<DesktopState>();
        let repository = open_repository(&state).map_err(|error| ToolError::new(error.message))?;
        let changes: Vec<UnitChangeDto> = repository
            .pending_changes()
            .map_err(|error| ToolError::new(error.to_string()))?
            .iter()
            .map(Into::into)
            .collect();
        serde_json::to_value(changes).map_err(|error| ToolError::new(error.to_string()))
    }

    fn unit_history(&self, unit_id: &str, limit: u32) -> Result<Value, ToolError> {
        let id =
            parse_translation_unit_id(unit_id).map_err(|error| ToolError::new(error.message))?;
        let state = self.app.state::<DesktopState>();
        let repository = open_repository(&state).map_err(|error| ToolError::new(error.message))?;
        let history: UnitHistoryDto = repository
            .unit_history(id, usize::try_from(limit).unwrap_or(10))
            .map_err(|error| ToolError::new(error.to_string()))?
            .into();
        serde_json::to_value(history).map_err(|error| ToolError::new(error.to_string()))
    }

    fn navigate(&self, location: &UnitLocation) -> Result<(), ToolError> {
        let binding = self.with_session(|session| navigation_target(session, location))?;
        self.app
            .emit(NAVIGATE_EVENT, binding)
            .map_err(|error| ToolError::new(format!("the editor could not be reached: {error}")))
    }

    fn project_file(&self, file: ProjectFile) -> Result<Option<String>, ToolError> {
        let root = self.with_session(|session| Ok(session.repository_root().to_owned()))?;
        read_project_file(&root, file).map_err(ToolError::new)
    }
}

pub(crate) fn repository_root(app: &tauri::AppHandle) -> CommandResult<std::path::PathBuf> {
    let state = app.state::<DesktopState>();
    let project = state.lock_project()?;
    let session = project.as_ref().ok_or_else(CommandError::no_project)?;
    Ok(session.repository_root().to_owned())
}

/// Replaces a project-shared file if it still has the content the change was
/// made against, through a temporary file and rename.
pub(crate) fn apply_file_change(
    root: &std::path::Path,
    file: ProjectFile,
    expected: Option<&str>,
    content: &str,
) -> Result<(), (ProposalStatus, String)> {
    let current =
        read_project_file(root, file).map_err(|message| (ProposalStatus::Failed, message))?;
    if current.as_deref() != expected {
        return Err((
            ProposalStatus::Conflict,
            format!("{} changed after the proposal was made", file.file_name()),
        ));
    }
    if file == ProjectFile::Glossary {
        aeria_ai::guidance::parse_glossary(content.as_bytes())
            .map_err(|error| (ProposalStatus::Failed, error.to_string()))?;
    }
    let path = root.join(file.file_name());
    let partial = root.join(format!(".{}.partial", file.file_name()));
    let write = || -> std::io::Result<()> {
        let mut handle = std::fs::File::create(&partial)?;
        std::io::Write::write_all(&mut handle, content.as_bytes())?;
        handle.sync_all()?;
        drop(handle);
        std::fs::rename(&partial, &path)
    };
    write().map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        (
            ProposalStatus::Failed,
            format!("{}: {error}", file.file_name()),
        )
    })
}

/// Resolves a location to one translatable occurrence: the given column, or
/// the row's first string.
fn navigation_target(
    session: &ProjectSession,
    location: &UnitLocation,
) -> Result<SourceBindingDto, ToolError> {
    let row =
        session_row(session, &location.sheet, location.row, location.subrow)?.ok_or_else(|| {
            ToolError::new(format!(
                "{}:{}:{} has no translatable string",
                location.sheet, location.row, location.subrow
            ))
        })?;
    let column = match location.column {
        Some(column) if row.cells.iter().any(|cell| cell.column == column) => column,
        Some(column) => {
            return Err(ToolError::new(format!(
                "column {column} of {}:{}:{} is not a translatable string",
                location.sheet, location.row, location.subrow
            )));
        }
        None => row.cells[0].column,
    };
    Ok(SourceBindingDto {
        sheet_name: location.sheet.clone(),
        row_id: location.row,
        subrow_id: location.subrow,
        column_index: column,
    })
}

fn review_state(label: ReviewLabel) -> ReviewState {
    match label {
        ReviewLabel::Draft => ReviewState::Draft,
        ReviewLabel::NeedsReview => ReviewState::NeedsReview,
        ReviewLabel::Reviewed => ReviewState::Reviewed,
    }
}

fn expectation(state: &UnitState) -> AssistedExpectation {
    AssistedExpectation {
        target: state.target.clone(),
        review_state: state.review_state.map(review_state),
    }
}

pub(crate) fn binding_of(location: &UnitLocation) -> Result<SourceBinding, ToolError> {
    let column = location
        .column
        .ok_or_else(|| ToolError::new("a column is required"))?;
    Ok(SourceBinding::new(
        location.sheet.clone(),
        location.row,
        location.subrow,
        column,
    ))
}

/// Writes one assisted target and tells the editor. Returns the overlay of
/// the written unit.
pub(crate) fn write_assisted(
    app: &tauri::AppHandle,
    session: &mut ProjectSession,
    location: &UnitLocation,
    target: &str,
    expected: &UnitState,
    replace_reviewed: bool,
) -> Result<(), AssistedWriteError> {
    let binding = binding_of(location).map_err(|error| AssistedWriteError::Structure {
        messages: vec![error.0],
    })?;
    let id =
        session.set_assisted_target(&binding, target, &expectation(expected), replace_reviewed)?;
    announce_unit(app, session, &binding, id);
    Ok(())
}

/// Sends a unit's new overlay to the editor.
fn announce_unit(
    app: &tauri::AppHandle,
    session: &ProjectSession,
    binding: &SourceBinding,
    id: aeria_core::TranslationUnitId,
) {
    if let Some(unit) = session.workspace().unit(id) {
        let _ = app.emit(
            APPLIED_EVENT,
            TranslationAppliedDto {
                source_binding: SourceBindingDto::from(binding),
                overlay: TranslationOverlayDto {
                    translation_unit_id: unit.id().to_string(),
                    target_macro: unit.target_macro().to_owned(),
                    review_state: unit.review_state().into(),
                    translator_note: unit.translator_note().map(str::to_owned),
                },
            },
        );
    }
}

/// Marks the batch's strings reviewed where the translation is still the
/// one Angelica suggested. Returns how many were approved and skipped.
fn approve_batch(
    session: &mut ProjectSession,
    batch: &ReviewBatch,
    mut approved_unit: impl FnMut(&ProjectSession, &SourceBinding, aeria_core::TranslationUnitId),
) -> Result<(usize, usize), String> {
    let mut approved = 0;
    let mut skipped = 0;
    for item in &batch.items {
        let binding = binding_of(&item.location).map_err(|error| error.0)?;
        let Some(unit) = session.workspace().unit_by_source_binding(&binding) else {
            skipped += 1;
            continue;
        };
        if unit.target_macro() != item.target {
            skipped += 1;
            continue;
        }
        if unit.review_state() == ReviewState::Reviewed {
            continue;
        }
        let id = unit.id();
        session
            .set_review_state(id, ReviewState::Reviewed)
            .map_err(|error| error.to_string())?;
        approved_unit(session, &binding, id);
        approved += 1;
    }
    Ok((approved, skipped))
}

/// Tells the renderer that a conversation's proposals changed.
pub(crate) fn announce_proposals(app: &tauri::AppHandle, conversation_id: &str) {
    let _ = app.emit(
        PROPOSALS_EVENT,
        ProposalsChangedDto {
            conversation_id: conversation_id.to_owned(),
        },
    );
}

/// Applies or records Angelica's proposals according to the conversation's
/// mode.
struct DesktopWriter {
    app: tauri::AppHandle,
    store: ConversationStore,
    conversation_id: String,
    mode: AgentMode,
    editor: EditorContext,
}

impl DesktopWriter {
    /// Auto-draft writes only new translations, and never one the user is
    /// editing right now.
    fn writes_at_once(&self, proposal: &Proposal) -> bool {
        self.mode == AgentMode::AutoDraft
            && proposal.expected.target.is_none()
            && !(self.editor.unsaved_draft
                && self.editor.selection.as_ref().is_some_and(|selection| {
                    selection.sheet == proposal.location.sheet
                        && selection.row == proposal.location.row
                        && selection.subrow == proposal.location.subrow
                        && selection.column == proposal.location.column
                }))
    }
}

impl ProjectWriter for DesktopWriter {
    fn translatable_unit(&self, location: &UnitLocation) -> Result<TranslatableUnit, ToolError> {
        let binding = binding_of(location)?;
        DesktopReader {
            app: self.app.clone(),
        }
        .with_session(|session| {
            let source = session
                .source_macro(&binding)
                .map_err(|error| ToolError::new(error.to_string()))?;
            let state = session.assisted_state(&binding);
            Ok(TranslatableUnit {
                source,
                state: UnitState {
                    target: state.target,
                    review_state: state.review_state.map(review_label),
                },
            })
        })
    }

    fn submit(&self, proposals: Vec<Proposal>) -> Result<Vec<ProposalOutcome>, ToolError> {
        let state = self.app.state::<DesktopState>();
        let mut outcomes = Vec::with_capacity(proposals.len());
        let mut pending = Vec::new();
        {
            let mut project = state
                .lock_project()
                .map_err(|error| ToolError::new(error.message))?;
            let session = project
                .as_mut()
                .ok_or_else(|| ToolError::new("no project is open"))?;
            for proposal in proposals {
                if !self.writes_at_once(&proposal) {
                    let id = aeria_ai::ProviderConfig::new_id();
                    outcomes.push(ProposalOutcome::Pending {
                        proposal_id: id.clone(),
                    });
                    pending.push(ProposalRecord {
                        id,
                        file: None,
                        job: None,
                        web: None,
                        review: None,
                        location: Some(proposal.location),
                        source: proposal.source,
                        target: proposal.target,
                        expected: proposal.expected,
                        status: ProposalStatus::Pending,
                        message: None,
                        created_at_unix_ms: now_unix_ms(),
                    });
                    continue;
                }
                outcomes.push(
                    match write_assisted(
                        &self.app,
                        session,
                        &proposal.location,
                        &proposal.target,
                        &proposal.expected,
                        false,
                    ) {
                        Ok(()) => ProposalOutcome::Applied,
                        Err(error @ AssistedWriteError::Conflict { .. }) => {
                            ProposalOutcome::Conflict {
                                message: error.to_string(),
                            }
                        }
                        Err(error) => ProposalOutcome::Failed {
                            message: error.to_string(),
                        },
                    },
                );
            }
        }
        if !pending.is_empty() {
            let _guard = state
                .lock_proposals()
                .map_err(|error| ToolError::new(error.message))?;
            let mut records = self
                .store
                .load_proposals(&self.conversation_id)
                .map_err(|error| ToolError::new(error.to_string()))?;
            records.extend(pending);
            self.store
                .save_proposals(&self.conversation_id, &records)
                .map_err(|error| ToolError::new(error.to_string()))?;
            announce_proposals(&self.app, &self.conversation_id);
        }
        Ok(outcomes)
    }

    fn propose_review(&self, batch: ReviewBatch) -> Result<ProposalOutcome, ToolError> {
        let state = self.app.state::<DesktopState>();
        let _guard = state
            .lock_proposals()
            .map_err(|error| ToolError::new(error.message))?;
        let mut records = self
            .store
            .load_proposals(&self.conversation_id)
            .map_err(|error| ToolError::new(error.to_string()))?;
        let id = aeria_ai::ProviderConfig::new_id();
        records.push(ProposalRecord {
            id: id.clone(),
            file: None,
            job: None,
            web: None,
            review: None,
            location: None,
            source: String::new(),
            target: batch.reason.clone(),
            expected: UnitState {
                target: None,
                review_state: None,
            },
            status: ProposalStatus::Pending,
            message: None,
            created_at_unix_ms: now_unix_ms(),
        });
        if let Some(record) = records.last_mut() {
            record.review = Some(batch);
        }
        self.store
            .save_proposals(&self.conversation_id, &records)
            .map_err(|error| ToolError::new(error.to_string()))?;
        announce_proposals(&self.app, &self.conversation_id);
        Ok(ProposalOutcome::Pending { proposal_id: id })
    }

    fn propose_file_change(&self, change: FileChange) -> Result<ProposalOutcome, ToolError> {
        let state = self.app.state::<DesktopState>();
        let _guard = state
            .lock_proposals()
            .map_err(|error| ToolError::new(error.message))?;
        let mut records = self
            .store
            .load_proposals(&self.conversation_id)
            .map_err(|error| ToolError::new(error.to_string()))?;
        let id = aeria_ai::ProviderConfig::new_id();
        records.push(ProposalRecord {
            id: id.clone(),
            file: Some(change.file),
            job: None,
            web: None,
            review: None,
            location: None,
            source: String::new(),
            target: change.after,
            expected: UnitState {
                target: change.before,
                review_state: None,
            },
            status: ProposalStatus::Pending,
            message: None,
            created_at_unix_ms: now_unix_ms(),
        });
        self.store
            .save_proposals(&self.conversation_id, &records)
            .map_err(|error| ToolError::new(error.to_string()))?;
        announce_proposals(&self.app, &self.conversation_id);
        Ok(ProposalOutcome::Pending { proposal_id: id })
    }
}

/// Runs Angelica's tools in blocking workers. Chat mode gets no writer.
struct DesktopTools {
    app: tauri::AppHandle,
    store: ConversationStore,
    conversation_id: String,
    mode: AgentMode,
    editor: EditorContext,
}

impl ToolExecutor for DesktopTools {
    fn execute<'a>(
        &'a self,
        call: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = ToolOutput> + Send + 'a>> {
        if call.name == "fetch_url" {
            return Box::pin(fetch_tool(
                &self.app,
                &self.store,
                &self.conversation_id,
                &call.arguments,
            ));
        }
        let reader = DesktopReader {
            app: self.app.clone(),
        };
        let writer = (self.mode != AgentMode::Chat).then(|| DesktopWriter {
            app: self.app.clone(),
            store: self.store.clone(),
            conversation_id: self.conversation_id.clone(),
            mode: self.mode,
            editor: self.editor.clone(),
        });
        let search = DesktopSearch {
            app: self.app.clone(),
        };
        let jobs = DesktopJobs {
            app: self.app.clone(),
            store: self.store.clone(),
            conversation_id: self.conversation_id.clone(),
        };
        let name = call.name.clone();
        let arguments = call.arguments.clone();
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || match &writer {
                Some(writer) => ReadTools::with_writer(&reader, writer)
                    .with_jobs(&jobs)
                    .with_search(&search)
                    .execute(&name, &arguments),
                None => ReadTools::new(&reader)
                    .with_jobs(&jobs)
                    .with_search(&search)
                    .execute(&name, &arguments),
            })
            .await
            .unwrap_or_else(|error| ToolOutput {
                content: serde_json::json!({ "error": format!("the tool worker failed: {error}") })
                    .to_string(),
                is_error: true,
            })
        })
    }
}

fn emit(app: &tauri::AppHandle, conversation_id: &str, event: AngelicaEventKind) {
    let _ = app.emit(
        EVENT,
        AngelicaEventDto {
            conversation_id: conversation_id.to_owned(),
            event,
        },
    );
}

#[tauri::command(rename_all = "camelCase")]
/// Lists the active project's conversations, newest first.
///
/// # Errors
///
/// Returns `noProjectOpen` or a storage error.
pub async fn angelica_conversations(
    app: tauri::AppHandle,
) -> CommandResult<Vec<ConversationSummaryDto>> {
    run_blocking(move || {
        let store = conversation_store(&app)?;
        let state = app.state::<DesktopState>();
        Ok(store
            .list()?
            .into_iter()
            .map(
                |ConversationSummary {
                     id,
                     title,
                     updated_at_unix_ms,
                 }| ConversationSummaryDto {
                    running: state.angelica_turn_running(&id),
                    id,
                    title,
                    updated_at_unix_ms,
                },
            )
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Loads one conversation of the active project.
///
/// # Errors
///
/// Returns `noProjectOpen`, `angelicaConversationNotFound`, or a storage
/// error.
pub async fn angelica_conversation(
    app: tauri::AppHandle,
    conversation_id: String,
) -> CommandResult<ConversationDto> {
    run_blocking(move || {
        let conversation = conversation_store(&app)?.load(&conversation_id)?;
        let running = app
            .state::<DesktopState>()
            .angelica_turn_running(&conversation_id);
        Ok(conversation_dto(conversation, running))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Deletes a conversation, stopping its turn first.
///
/// # Errors
///
/// Returns `noProjectOpen` or a storage error.
pub async fn angelica_delete_conversation(
    app: tauri::AppHandle,
    conversation_id: String,
) -> CommandResult<()> {
    run_blocking(move || {
        app.state::<DesktopState>()
            .cancel_angelica_turn(&conversation_id);
        conversation_store(&app)?.delete(&conversation_id)?;
        Ok(())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Stops a running turn. Progress saved before the stop is kept.
///
/// # Errors
///
/// Never fails; stopping a finished turn does nothing.
pub async fn angelica_cancel(app: tauri::AppHandle, conversation_id: String) -> CommandResult<()> {
    if app
        .state::<DesktopState>()
        .cancel_angelica_turn(&conversation_id)
    {
        emit(
            &app,
            &conversation_id,
            AngelicaEventKind::Turn(TurnEvent::Cancelled),
        );
    }
    Ok(())
}

/// Everything a turn needs, gathered before it starts.
struct PreparedTurn {
    store: ConversationStore,
    conversation: Conversation,
    endpoint: aeria_ai::ProviderEndpoint,
    model: aeria_ai::ModelConfig,
    effort: Option<aeria_ai::ReasoningEffort>,
    system: String,
    mode: AgentMode,
    editor: EditorContext,
}

struct TurnRequest<'a> {
    conversation_id: Option<&'a str>,
    text: &'a str,
    /// An update from Aeria rather than from the user.
    automatic: bool,
}

fn prepare_turn(
    app: &tauri::AppHandle,
    request: &TurnRequest<'_>,
    selection: ModelSelection,
    editor: &EditorContext,
    mode: AgentMode,
    endpoint: aeria_ai::ProviderEndpoint,
) -> CommandResult<PreparedTurn> {
    if let Some(id) = request.conversation_id
        && app.state::<DesktopState>().angelica_turn_running(id)
    {
        return Err(busy());
    }
    let settings_store = settings_store(app)?;
    let settings = settings_store.load()?;
    let model = settings
        .selected_model(&selection)
        .map_err(|message| CommandError::new("aiInvalidSettings", message))?
        .clone();
    let facts = DesktopReader { app: app.clone() }.facts().ok();
    let guide = ProjectGuide::load(&repository_root(app)?);
    // The first message starts building the search index in the background.
    prepare_source_index(app);
    let system = system_prompt(facts.as_ref(), editor, mode, &guide);

    let store = conversation_store(app)?;
    let now = now_unix_ms();
    let mut conversation = match request.conversation_id {
        Some(id) => store.load(id)?,
        None => Conversation::new(now),
    };
    if request.automatic {
        conversation.push_automatic(request.text, now);
    } else {
        conversation.push_user(request.text, now);
    }
    let effort = selection.effort;
    conversation.model = Some(selection);
    conversation.mode = mode;
    store.save(&conversation)?;
    Ok(PreparedTurn {
        store,
        conversation,
        endpoint,
        model,
        effort,
        system,
        mode,
        editor: editor.clone(),
    })
}

fn busy() -> CommandError {
    CommandError::new(
        "angelicaBusy",
        "Angelica is still answering in this conversation",
    )
}

fn failure(error: CommandError) -> TurnEvent {
    TurnEvent::Failed {
        code: error.code,
        message: error.message,
    }
}

async fn run_prepared_turn(
    app: tauri::AppHandle,
    client: aeria_ai::OpenAiCompatibleClient,
    prepared: PreparedTurn,
) {
    let PreparedTurn {
        store,
        mut conversation,
        endpoint,
        model,
        effort,
        system,
        mode,
        editor,
    } = prepared;
    let id = conversation.id.clone();
    let mut tools = read_tool_definitions();
    if mode != AgentMode::Chat {
        tools.extend(write_tool_definitions());
    }
    tools.extend(search_tool_definitions());
    tools.extend(web_tool_definitions());
    tools.extend(job_tool_definitions(mode != AgentMode::Chat));
    let config = TurnConfig {
        model: &model.id,
        effort,
        system: &system,
        tools: &tools,
        context_tokens: model.context_window,
        session: &id,
        max_rounds: aeria_ai::agent::MAX_ROUNDS_PER_TURN,
    };
    let executor = DesktopTools {
        app: app.clone(),
        store: store.clone(),
        conversation_id: id.clone(),
        mode,
        editor,
    };
    let mut messages = conversation.messages.clone();
    let mut on_event = |event: AgentEvent| emit(&app, &id, AngelicaEventKind::Agent(event));
    let mut persisted = conversation.clone();
    let persist_store = store.clone();
    let mut persist = move |messages: &[ChatMessage]| {
        persisted.messages = messages.to_vec();
        persisted.updated_at_unix_ms = now_unix_ms();
        let _ = persist_store.save(&persisted);
    };
    let result = run_turn(
        &client,
        &endpoint,
        &config,
        &mut messages,
        &executor,
        &mut on_event,
        &mut persist,
    )
    .await;

    conversation.messages = messages;
    conversation.updated_at_unix_ms = now_unix_ms();
    let event = match result {
        Ok(summary) => {
            conversation.usage.add(summary.usage);
            TurnEvent::Finished {
                outcome: summary.outcome,
                usage: conversation.usage,
            }
        }
        Err(error) => failure(error.into()),
    };
    let saved = tauri::async_runtime::spawn_blocking(move || store.save(&conversation)).await;
    app.state::<DesktopState>().finish_angelica_turn(&id);
    let event = match saved {
        Ok(Err(error)) => failure(error.into()),
        _ => event,
    };
    emit(&app, &id, AngelicaEventKind::Turn(event));
}

#[tauri::command(rename_all = "camelCase")]
/// Adds a user message and starts Angelica's turn in the background.
///
/// Without `conversationId` a new conversation is created. The returned
/// conversation already contains the message; progress arrives as
/// `angelica://event` events.
///
/// # Errors
///
/// Returns `noProjectOpen`, `angelicaBusy` while the conversation is
/// running, `aiInvalidSettings` for an unknown model selection, a missing
/// key, or a storage error.
pub async fn angelica_send(
    app: tauri::AppHandle,
    conversation_id: Option<String>,
    text: String,
    model: ModelSelection,
    editor: Option<EditorContext>,
    mode: Option<AgentMode>,
) -> CommandResult<ConversationDto> {
    let text = text.trim().to_owned();
    if text.is_empty() || text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(CommandError::new(
            "angelicaInvalidMessage",
            format!("a message must have 1 to {MAX_MESSAGE_CHARS} characters"),
        ));
    }
    let endpoint = resolve_endpoint(&app, model.provider_id.clone()).await?;
    let prepare_app = app.clone();
    let prepared = run_blocking(move || {
        prepare_turn(
            &prepare_app,
            &TurnRequest {
                conversation_id: conversation_id.as_deref(),
                text: &text,
                automatic: false,
            },
            model,
            &editor.unwrap_or_default(),
            mode.unwrap_or_default(),
            endpoint,
        )
    })
    .await?;
    let client = app.state::<DesktopState>().ai_client()?;
    let response = conversation_dto(prepared.conversation.clone(), true);
    let turn = run_prepared_turn(app.clone(), client, prepared);
    if !app
        .state::<DesktopState>()
        .start_angelica_turn(response.id.clone(), || tauri::async_runtime::spawn(turn))
    {
        return Err(busy());
    }
    Ok(response)
}

/// Starts an automatic turn with an update from Aeria, such as a job report,
/// with the conversation's last model and mode. Nothing starts while a turn
/// runs in the conversation.
pub(crate) async fn wake_angelica(app: &tauri::AppHandle, conversation_id: &str, text: String) {
    if app
        .state::<DesktopState>()
        .angelica_turn_running(conversation_id)
    {
        return;
    }
    let load_app = app.clone();
    let id = conversation_id.to_owned();
    let Ok(conversation) =
        run_blocking(move || Ok(conversation_store(&load_app)?.load(&id)?)).await
    else {
        return;
    };
    let Some(selection) = conversation.model.clone() else {
        return;
    };
    let Ok(endpoint) = resolve_endpoint(app, selection.provider_id.clone()).await else {
        return;
    };
    let prepare_app = app.clone();
    let id = conversation_id.to_owned();
    let mode = conversation.mode;
    let Ok(prepared) = run_blocking(move || {
        prepare_turn(
            &prepare_app,
            &TurnRequest {
                conversation_id: Some(&id),
                text: &text,
                automatic: true,
            },
            selection,
            &EditorContext::default(),
            mode,
            endpoint,
        )
    })
    .await
    else {
        return;
    };
    let Ok(client) = app.state::<DesktopState>().ai_client() else {
        return;
    };
    let turn = run_prepared_turn(app.clone(), client, prepared);
    let _ = app
        .state::<DesktopState>()
        .start_angelica_turn(conversation_id.to_owned(), || {
            tauri::async_runtime::spawn(turn)
        });
}

#[tauri::command(rename_all = "camelCase")]
/// Lists the translations Angelica proposed in a conversation.
///
/// # Errors
///
/// Returns `noProjectOpen` or a storage error.
pub async fn angelica_proposals(
    app: tauri::AppHandle,
    conversation_id: String,
) -> CommandResult<Vec<ProposalRecord>> {
    run_blocking(move || Ok(conversation_store(&app)?.load_proposals(&conversation_id)?)).await
}

/// The status and message of an outcome.
fn settled<E: std::fmt::Display>(
    result: Result<Option<String>, E>,
) -> (ProposalStatus, Option<String>) {
    match result {
        Ok(message) => (ProposalStatus::Applied, message),
        Err(error) => (ProposalStatus::Failed, Some(error.to_string())),
    }
}

/// Applies one pending proposal with the user's approval.
fn apply_record(
    app: &tauri::AppHandle,
    conversation_id: &str,
    record: &ProposalRecord,
) -> CommandResult<(ProposalStatus, Option<String>)> {
    let state = app.state::<DesktopState>();
    if let Some(batch) = &record.review {
        let mut project = state.lock_project()?;
        let session = project.as_mut().ok_or_else(CommandError::no_project)?;
        // Applying is the user's approval of exactly these translations.
        return Ok(settled(
            approve_batch(session, batch, |session, binding, id| {
                announce_unit(app, session, binding, id);
            })
            .map(|(approved, skipped)| {
                Some(format!(
                    "{approved} approved, {skipped} skipped because they changed"
                ))
            }),
        ));
    }
    if let Some(domain) = &record.web {
        return Ok(settled(
            allow_domain(app, domain)
                .map(|()| None)
                .map_err(|error| error.message),
        ));
    }
    if let Some(job) = &record.job {
        return Ok(settled(
            start_proposed_job(app, conversation_id, job)
                .map(Some)
                .map_err(|error| error.message),
        ));
    }
    if let Some(file) = record.file {
        let root = repository_root(app)?;
        return Ok(
            match apply_file_change(
                &root,
                file,
                record.expected.target.as_deref(),
                &record.target,
            ) {
                Ok(()) => (ProposalStatus::Applied, None),
                Err((status, message)) => (status, Some(message)),
            },
        );
    }
    let Some(location) = record.location.clone() else {
        return Err(CommandError::new(
            "angelicaProposalNotFound",
            "the proposal has no string to write",
        ));
    };
    let mut project = state.lock_project()?;
    let session = project.as_mut().ok_or_else(CommandError::no_project)?;
    // Applying is the user's explicit approval, including for a reviewed
    // string.
    Ok(
        match write_assisted(
            app,
            session,
            &location,
            &record.target,
            &record.expected,
            true,
        ) {
            Ok(()) => (ProposalStatus::Applied, None),
            Err(error @ AssistedWriteError::Conflict { .. }) => {
                (ProposalStatus::Conflict, Some(error.to_string()))
            }
            Err(error) => (ProposalStatus::Failed, Some(error.to_string())),
        },
    )
}

/// Settles one pending proposal and returns the updated list.
fn settle_proposal(
    app: &tauri::AppHandle,
    conversation_id: &str,
    proposal_id: &str,
    apply: bool,
) -> CommandResult<Vec<ProposalRecord>> {
    let store = conversation_store(app)?;
    let state = app.state::<DesktopState>();
    let _guard = state.lock_proposals()?;
    let mut records = store.load_proposals(conversation_id)?;
    let record = records
        .iter_mut()
        .find(|record| record.id == proposal_id)
        .ok_or_else(|| {
            CommandError::new(
                "angelicaProposalNotFound",
                format!("proposal {proposal_id:?} was not found"),
            )
        })?;
    if record.status != ProposalStatus::Pending {
        return Err(CommandError::new(
            "angelicaProposalSettled",
            "this proposal was already applied or dismissed",
        ));
    }
    let (status, message) = if apply {
        apply_record(app, conversation_id, record)?
    } else {
        (ProposalStatus::Rejected, None)
    };
    record.status = status;
    record.message = message;
    store.save_proposals(conversation_id, &records)?;
    Ok(records)
}

#[tauri::command(rename_all = "camelCase")]
/// Applies a pending proposal as a draft. A string that changed since the
/// proposal was made is not overwritten; the proposal becomes a conflict.
///
/// # Errors
///
/// Returns `angelicaProposalNotFound`, `angelicaProposalSettled`, or a
/// storage error.
pub async fn angelica_apply_proposal(
    app: tauri::AppHandle,
    conversation_id: String,
    proposal_id: String,
) -> CommandResult<Vec<ProposalRecord>> {
    let settle_app = app.clone();
    let settle_conversation = conversation_id.clone();
    let settle_id = proposal_id.clone();
    let records =
        run_blocking(move || settle_proposal(&settle_app, &settle_conversation, &settle_id, true))
            .await?;
    // Allowing a domain lets Angelica continue with the link she asked for.
    if let Some(record) = records
        .iter()
        .find(|record| record.id == proposal_id && record.status == ProposalStatus::Applied)
        && let Some(domain) = &record.web
    {
        let text = format!(
            "[Aeria] The user allowed reading {domain}. Open {} again and continue.",
            record.target
        );
        let wake_app = app.clone();
        tauri::async_runtime::spawn(async move {
            wake_angelica(&wake_app, &conversation_id, text).await;
        });
    }
    Ok(records)
}

#[tauri::command(rename_all = "camelCase")]
/// Dismisses a pending proposal without writing anything.
///
/// # Errors
///
/// Returns `angelicaProposalNotFound`, `angelicaProposalSettled`, or a
/// storage error.
pub async fn angelica_reject_proposal(
    app: tauri::AppHandle,
    conversation_id: String,
    proposal_id: String,
) -> CommandResult<Vec<ProposalRecord>> {
    run_blocking(move || settle_proposal(&app, &conversation_id, &proposal_id, false)).await
}

/// A draft for one string from "Draft with Angelica".
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AngelicaDraftDto {
    pub target: String,
}

struct DraftInput {
    model: aeria_ai::ModelConfig,
    effort: Option<aeria_ai::ReasoningEffort>,
    facts: Option<ProjectFacts>,
    source: String,
    context: Vec<ContextCell>,
    current_target: Option<String>,
    note: Option<String>,
    guidance: Option<String>,
    glossary: Vec<GlossaryEntry>,
}

fn draft_input(
    app: &tauri::AppHandle,
    binding: &SourceBinding,
    selection: &ModelSelection,
) -> CommandResult<DraftInput> {
    let settings = settings_store(app)?.load()?;
    let model = settings
        .selected_model(selection)
        .map_err(|message| CommandError::new("aiInvalidSettings", message))?
        .clone();
    let state = app.state::<DesktopState>();
    let project = state.lock_project()?;
    let session = project.as_ref().ok_or_else(CommandError::no_project)?;
    let source = session.source_macro(binding).map_err(CommandError::from)?;
    let guide = ProjectGuide::load(session.repository_root());
    let row = session_row(
        session,
        binding.sheet_name(),
        binding.row_id(),
        binding.subrow_id(),
    )
    .map_err(|error| CommandError::new("translationRead", error.0))?;
    let cell = row.as_ref().and_then(|row| {
        row.cells
            .iter()
            .find(|cell| cell.column == binding.column_index())
    });
    let glossary = guide
        .glossary
        .as_ref()
        .map(|glossary| {
            glossary
                .matches(&source)
                .into_iter()
                .take(20)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    Ok(DraftInput {
        model,
        effort: selection.effort,
        facts: Some(session_facts(session)),
        source,
        context: row
            .as_ref()
            .map(|row| row.context.clone())
            .unwrap_or_default(),
        current_target: cell.and_then(|cell| cell.target.clone()),
        note: cell.and_then(|cell| cell.note.clone()),
        guidance: guide.guidance_for_prompt(),
        glossary,
    })
}

#[tauri::command(rename_all = "camelCase")]
/// Drafts a translation of one string with Angelica's default model. The
/// draft is returned for the editor; nothing is saved.
///
/// # Errors
///
/// Returns `aiNoAgentModel` without a default model, `angelicaUntaggable`
/// for a malformed source, `angelicaDraftRejected` when the model kept
/// breaking the structure, or a settings, key, or provider error.
pub async fn angelica_draft(
    app: tauri::AppHandle,
    source_binding: SourceBindingDto,
) -> CommandResult<AngelicaDraftDto> {
    let binding = SourceBinding::from(source_binding);
    let store = settings_store(&app)?;
    let selection = run_blocking(move || Ok(store.load()?.agent_model))
        .await?
        .ok_or_else(|| {
            CommandError::new(
                "aiNoAgentModel",
                "choose Angelica's default model in Settings → AI",
            )
        })?;
    let endpoint = resolve_endpoint(&app, selection.provider_id.clone()).await?;
    let input_app = app.clone();
    let input_binding = binding.clone();
    let input = run_blocking(move || draft_input(&input_app, &input_binding, &selection)).await?;
    let client = app.state::<DesktopState>().ai_client()?;
    let location = format!(
        "{}:{}:{}:{}",
        binding.sheet_name(),
        binding.row_id(),
        binding.subrow_id(),
        binding.column_index()
    );
    let session = aeria_ai::ProviderConfig::new_id();
    let draft = aeria_ai::draft::draft_translation(
        &client,
        &endpoint,
        &aeria_ai::draft::DraftRequest {
            model: &input.model.id,
            effort: input.effort,
            session: &session,
            facts: input.facts.as_ref(),
            location: &location,
            source: &input.source,
            context: &input.context,
            current_target: input.current_target.as_deref(),
            note: input.note.as_deref(),
            guidance: input.guidance.as_deref(),
            glossary: &input.glossary,
        },
    )
    .await
    .map_err(|error| match error {
        aeria_ai::draft::DraftError::Provider(error) => CommandError::from(error),
        aeria_ai::draft::DraftError::Untaggable => {
            CommandError::new("angelicaUntaggable", error.to_string())
        }
        aeria_ai::draft::DraftError::Rejected(_) => {
            CommandError::new("angelicaDraftRejected", error.to_string())
        }
    })?;
    Ok(AngelicaDraftDto {
        target: draft.target,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use aeria_ai::tools::ReviewItem;

    use super::*;

    fn session() -> (tempfile::TempDir, ProjectSession) {
        let directory = tempfile::tempdir().expect("directory");
        std::fs::create_dir(directory.path().join("repository")).expect("repository");
        let package = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../crates/aeria-hsp/tests/fixtures/synthetic.hsp");
        let session = ProjectSession::initialize(
            directory.path().join("repository"),
            package,
            directory.path().join("cache"),
            "ru".to_owned(),
        )
        .expect("session");
        (directory, session)
    }

    #[test]
    fn facts_and_sheets_describe_the_translatable_source() {
        let (_directory, session) = session();
        let facts = session_facts(&session);
        assert_eq!(facts.target_language.as_deref(), Some("ru"));
        assert!(facts.sheet_count > 0);
        assert_eq!(facts.translated, 0);
        let sheets = session_sheets(&session);
        assert_eq!(
            sheets.iter().filter(|sheet| sheet.translatable > 0).count(),
            facts.sheet_count
        );
    }

    #[test]
    fn one_row_is_found_through_the_exclusive_cursor() {
        let (_directory, session) = session();
        for sheet in session_sheets(&session)
            .into_iter()
            .filter(|sheet| sheet.translatable > 0)
        {
            let page = session_rows(&session, &sheet.name, None, 256).expect("rows");
            for expected in &page.rows {
                let found = session_row(&session, &sheet.name, expected.row, expected.subrow)
                    .expect("row")
                    .expect("translatable row");
                assert_eq!(
                    &found, expected,
                    "{}:{}:{}",
                    sheet.name, expected.row, expected.subrow
                );
            }
        }
    }

    #[test]
    fn navigation_resolves_the_first_string_or_checks_the_column() {
        let (_directory, session) = session();
        let sheet = session_sheets(&session)
            .into_iter()
            .find(|sheet| sheet.translatable > 0)
            .expect("translatable sheet");
        let row = session_rows(&session, &sheet.name, None, 256)
            .expect("rows")
            .rows
            .remove(0);
        let mut location = UnitLocation {
            sheet: sheet.name.clone(),
            row: row.row,
            subrow: row.subrow,
            column: None,
        };
        let binding = navigation_target(&session, &location).expect("first string");
        assert_eq!(binding.column_index, row.cells[0].column);
        location.column = Some(u32::MAX);
        assert!(navigation_target(&session, &location).is_err());
    }

    #[test]
    fn file_changes_apply_only_over_the_expected_content() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path();
        apply_file_change(root, ProjectFile::Guidance, None, "Be brief.\n").expect("create");
        assert_eq!(
            read_project_file(root, ProjectFile::Guidance)
                .expect("read")
                .as_deref(),
            Some("Be brief.\n")
        );
        let stale = apply_file_change(root, ProjectFile::Guidance, None, "Other.\n")
            .expect_err("the file exists now");
        assert_eq!(stale.0, ProposalStatus::Conflict);
        apply_file_change(
            root,
            ProjectFile::Guidance,
            Some("Be brief.\n"),
            "Be kind.\n",
        )
        .expect("replace");
        let invalid = apply_file_change(root, ProjectFile::Glossary, None, "term,meaning\n")
            .expect_err("invalid glossary");
        assert_eq!(invalid.0, ProposalStatus::Failed);
        assert!(!root.join(ProjectFile::Glossary.file_name()).exists());
        assert!(!root.join(".aeria-guidance.md.partial").exists());
    }

    #[test]
    fn approval_batches_skip_strings_that_changed() {
        let (_directory, mut session) = session();
        let sheet = session_sheets(&session)
            .into_iter()
            .find(|sheet| sheet.translatable > 0)
            .expect("translatable sheet");
        let row = session_rows(&session, &sheet.name, None, 256)
            .expect("rows")
            .rows
            .remove(0);
        let first = UnitLocation {
            sheet: sheet.name.clone(),
            row: row.row,
            subrow: row.subrow,
            column: Some(row.cells[0].column),
        };
        let binding = binding_of(&first).expect("binding");
        let source = session.source_macro(&binding).expect("source");
        session.set_target(&binding, &source).expect("target");
        let batch = |target: &str| ReviewBatch {
            reason: "checked".to_owned(),
            items: vec![ReviewItem {
                location: first.clone(),
                source: source.clone(),
                target: target.to_owned(),
            }],
        };

        let mut announced = 0;
        let changed = approve_batch(&mut session, &batch("something else"), |_, _, _| {
            announced += 1;
        })
        .expect("batch");
        assert_eq!(changed, (0, 1));
        let approved =
            approve_batch(&mut session, &batch(&source), |_, _, _| announced += 1).expect("batch");
        assert_eq!(approved, (1, 0));
        assert_eq!(announced, 1);
        let unit = session
            .workspace()
            .unit_by_source_binding(&binding)
            .expect("unit");
        assert_eq!(unit.review_state(), ReviewState::Reviewed);
    }

    #[test]
    fn unknown_sheets_are_reported_to_the_model() {
        let (_directory, session) = session();
        let error = session_rows(&session, "NoSuchSheet", None, 10).expect_err("unknown sheet");
        assert!(error.0.contains("list_sheets"));
        assert!(session_row(&session, "NoSuchSheet", 0, 0).is_err());
    }
}
