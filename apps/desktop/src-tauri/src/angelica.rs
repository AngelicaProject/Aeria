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

use aeria_ai::agent::{AgentEvent, ToolExecutor, TurnConfig, TurnOutcome, run_turn};
use aeria_ai::chat::{ChatMessage, ToolCall, Usage};
use aeria_ai::conversation::{
    Conversation, ConversationError, ConversationStore, ConversationSummary,
};
use aeria_ai::prompt::{EditorContext, system_prompt};
use aeria_ai::tools::{
    CellSnapshot, ContextCell, ProjectFacts, ProjectReader, ReadTools, ReviewLabel, RowSnapshot,
    RowsPage, SheetSummary, ToolError, ToolOutput, UnitLocation, read_tool_definitions,
};
use aeria_ai::{KeyringSecretStore, ModelSelection};
use aeria_core::ReviewState;
use aeria_workspace::{ProjectSession, TranslationRowCursor, TranslationRowView};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};

use crate::ai::{provider_endpoint, settings_store};
use crate::commands::{parse_translation_unit_id, run_blocking};
use crate::dto::{ProjectSummaryDto, SourceBindingDto};
use crate::error::CommandError;
use crate::git::{UnitChangeDto, UnitHistoryDto, open_repository};
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// Renderer event carrying live turn progress.
pub const EVENT: &str = "angelica://event";
/// Renderer event asking the editor to show an occurrence.
pub const NAVIGATE_EVENT: &str = "angelica://navigate";

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

fn now_unix_ms() -> u64 {
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

/// Returns the conversation store of the active project, keyed by a hash of
/// its repository root so paths never appear in file names.
fn conversation_store(app: &tauri::AppHandle) -> CommandResult<ConversationStore> {
    let root = {
        let state = app.state::<DesktopState>();
        let project = state.lock_project()?;
        let project = project.as_ref().ok_or_else(CommandError::no_project)?;
        project.repository_root().to_string_lossy().into_owned()
    };
    let digest = Sha256::digest(root.as_bytes());
    let key = digest[..16]
        .iter()
        .fold(String::with_capacity(32), |mut key, byte| {
            let _ = write!(key, "{byte:02x}");
            key
        });
    let data = app.path().app_data_dir().map_err(|error| {
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
struct DesktopReader {
    app: tauri::AppHandle,
}

impl DesktopReader {
    fn with_session<T>(
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

fn review_label(state: ReviewState) -> ReviewLabel {
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

fn known_sheet(session: &ProjectSession, sheet: &str) -> Result<(), ToolError> {
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

fn session_facts(session: &ProjectSession) -> ProjectFacts {
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

fn session_row(
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

/// Runs Angelica's read-only tools in blocking workers.
struct DesktopTools {
    app: tauri::AppHandle,
}

impl ToolExecutor for DesktopTools {
    fn execute<'a>(
        &'a self,
        call: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = ToolOutput> + Send + 'a>> {
        let app = self.app.clone();
        let name = call.name.clone();
        let arguments = call.arguments.clone();
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || {
                ReadTools::new(&DesktopReader { app }).execute(&name, &arguments)
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
}

fn prepare_turn(
    app: &tauri::AppHandle,
    conversation_id: Option<&str>,
    text: &str,
    selection: ModelSelection,
    editor: &EditorContext,
) -> CommandResult<PreparedTurn> {
    if let Some(id) = conversation_id
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
    let endpoint = provider_endpoint(&settings_store, &KeyringSecretStore, &selection.provider_id)?;
    let facts = DesktopReader { app: app.clone() }.facts().ok();
    let system = system_prompt(facts.as_ref(), editor);

    let store = conversation_store(app)?;
    let now = now_unix_ms();
    let mut conversation = match conversation_id {
        Some(id) => store.load(id)?,
        None => Conversation::new(now),
    };
    conversation.push_user(text, now);
    let effort = selection.effort;
    conversation.model = Some(selection);
    store.save(&conversation)?;
    Ok(PreparedTurn {
        store,
        conversation,
        endpoint,
        model,
        effort,
        system,
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
    } = prepared;
    let id = conversation.id.clone();
    let tools = read_tool_definitions();
    let config = TurnConfig {
        model: &model.id,
        effort,
        system: &system,
        tools: &tools,
        context_tokens: model.context_window,
        session: &id,
    };
    let executor = DesktopTools { app: app.clone() };
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
) -> CommandResult<ConversationDto> {
    let text = text.trim().to_owned();
    if text.is_empty() || text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(CommandError::new(
            "angelicaInvalidMessage",
            format!("a message must have 1 to {MAX_MESSAGE_CHARS} characters"),
        ));
    }
    let prepare_app = app.clone();
    let prepared = run_blocking(move || {
        prepare_turn(
            &prepare_app,
            conversation_id.as_deref(),
            &text,
            model,
            &editor.unwrap_or_default(),
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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
    fn unknown_sheets_are_reported_to_the_model() {
        let (_directory, session) = session();
        let error = session_rows(&session, "NoSuchSheet", None, 10).expect_err("unknown sheet");
        assert!(error.0.contains("list_sheets"));
        assert!(session_row(&session, "NoSuchSheet", 0, 0).is_err());
    }
}
