//! Desktop orchestration of translation jobs.
//!
//! A job runs as a registered async task with `concurrency` lanes. Each lane
//! claims the next chunk from the job store, runs a worker subagent for it,
//! and records the outcomes. Tools run in blocking workers that take the
//! project lock only for their own reads and writes. A job pauses itself
//! when its token limit is reached, when too many translations are rejected,
//! or when the provider keeps failing, and wakes Angelica in its
//! conversation when it pauses or finishes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use aeria_ai::ProviderError;
use aeria_ai::chat::{ChatMessage, ChatRequest, StreamDelta};
use aeria_ai::conversation::{ConversationStore, ProposalRecord, ProposalStatus};
use aeria_ai::images::{ImagePayloads, ImageRef, message_images};
use aeria_ai::jobs::{
    Finding, JobError, JobEstimate, JobEvent, JobFilter, JobLimitProposal, JobProposal, JobQuality,
    JobScope, JobSpec, JobStatus, JobStore, JobSummary, JobUnit, ScopedUnit, UnitStatus,
};
use aeria_ai::knowledge::{Domain, Knowledge, sheet_domain};
use aeria_ai::localizer::{
    Caller, Finish, LineOutcome, LocalizeOptions, LocalizeResult, Replies,
    Request as LocalizerRequest, Role, Step, effort_for, localize,
};
use aeria_ai::search::ProjectSearch;
use aeria_ai::study::{contract_story, study_terms};
use aeria_ai::tools::{
    JobAction, JobControl, ProjectReader, ProposalOutcome, ReviewLabel, ToolError, UnitLocation,
    UnitState,
};
use aeria_ai::worker::{
    JobHost, PreparedUnit, UnitContext, WriteFailure, prepare_unit, unit_knowledge,
};
use aeria_workspace::{AssistedWriteError, ProjectSession, TranslationRowCursor};
use serde::Serialize;
use tauri::{Emitter, Manager};
use tokio::task::JoinSet;

use crate::ai::{resolve_endpoint, settings_store};
use crate::angelica::{
    DesktopReader, announce_proposals, binding_of, known_sheet, load_image_payloads, now_unix_ms,
    project_conversation_store, project_key, repository_root, review_label, session_row,
    wake_angelica, write_assisted,
};
use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::job_workers::{WorkerActivity, WorkerBoard, WorkerPhase};
use crate::paths::AeriaPaths;
use crate::search::DesktopSearch;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

#[path = "job_learning.rs"]
mod job_learning;
#[path = "job_study.rs"]
mod job_study;

/// Renderer event saying a job changed.
pub const JOB_EVENT: &str = "angelica://job";
const JOBS_DIRECTORY: &str = "jobs";
/// Processed strings before the rejection share is judged.
const REJECTION_SAMPLE: u64 = 40;
/// Provider failures in a row, per lane, before the job pauses.
const MAX_PROVIDER_FAILURES: u32 = 3;
const PROVIDER_BACKOFF: Duration = Duration::from_secs(20);
/// Job store failures in a row, per lane, before the job pauses.
const MAX_STORE_FAILURES: u32 = 5;
const STORE_BACKOFF: Duration = Duration::from_millis(500);
/// Translation-memory matches given to a worker per string.
const JOB_MEMORY_MATCHES: usize = 3;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobChangedDto {
    job_id: String,
}

impl From<JobError> for CommandError {
    fn from(error: JobError) -> Self {
        let code = match &error {
            JobError::NotFound(_) => "angelicaJobNotFound",
            JobError::Invalid(_) => "angelicaJobInvalid",
            JobError::Storage(_) => "angelicaJobStorage",
        };
        Self::new(code, error.to_string())
    }
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::new(error.to_string())
}

/// The active project's job store. Jobs a previous run left running are
/// paused the first time a store is opened in this process.
pub(crate) fn job_store(app: &tauri::AppHandle) -> CommandResult<JobStore> {
    let root = repository_root(app)?;
    let data = app.aeria_data_dir().map_err(|error| {
        CommandError::new(
            "angelicaJobStorage",
            format!("could not resolve the Aeria app-data directory: {error}"),
        )
    })?;
    let store = JobStore::new(
        data.join(JOBS_DIRECTORY)
            .join(format!("{}.sqlite3", project_key(&root))),
    );
    if app
        .state::<DesktopState>()
        .first_job_store_use(store.path())
    {
        store.pause_interrupted()?;
    }
    Ok(store)
}

fn notify(app: &tauri::AppHandle, job_id: &str) {
    let _ = app.emit(
        JOB_EVENT,
        JobChangedDto {
            job_id: job_id.to_owned(),
        },
    );
}

/// Sheets in the order a job translates them: names first, since all other
/// text refers to them, then mechanics, items, interface, lore, and quests
/// and cutscenes last; the given order within each domain.
fn domain_order(mut sheets: Vec<String>) -> Vec<String> {
    let rank = |sheet: &String| match sheet_domain(sheet) {
        Domain::Names => 0,
        Domain::Actions => 1,
        Domain::Items => 2,
        Domain::Interface | Domain::General => 3,
        Domain::Lore => 4,
        Domain::Dialogue | Domain::Journal | Domain::Objective | Domain::System => 5,
    };
    sheets.sort_by_key(rank);
    sheets
}

/// Lists the strings a job over `scope` covers, in sheet and row order.
/// Reviewed translations are never included.
fn enumerate_scope(app: &tauri::AppHandle, scope: &JobScope) -> Result<Vec<ScopedUnit>, ToolError> {
    let reader = DesktopReader { app: app.clone() };
    let sheets: Vec<String> = if scope.sheets.is_empty() {
        reader
            .sheets()?
            .into_iter()
            .filter(|sheet| sheet.translatable > 0)
            .map(|sheet| sheet.name)
            .collect()
    } else {
        scope.sheets.clone()
    };
    let mut units = Vec::new();
    for sheet in domain_order(sheets) {
        let mut after: Option<(u32, u16)> = None;
        loop {
            let page = reader.with_session(|session| {
                page_scope(session, &sheet, after, scope.filter, &mut units)
            })?;
            match page {
                Some(next) => after = Some(next),
                None => break,
            }
            if units.len() > aeria_ai::jobs::MAX_JOB_UNITS {
                return Err(ToolError::new("the scope has too many strings for one job"));
            }
        }
    }
    Ok(units)
}

/// Adds one page of a sheet's matching strings; returns the next cursor.
fn page_scope(
    session: &ProjectSession,
    sheet: &str,
    after: Option<(u32, u16)>,
    filter: JobFilter,
    units: &mut Vec<ScopedUnit>,
) -> Result<Option<(u32, u16)>, ToolError> {
    known_sheet(session, sheet)?;
    let cursor = after.map(|(row, subrow)| TranslationRowCursor::new(sheet, row, subrow));
    let page = session
        .page_translation_rows(
            sheet,
            cursor.as_ref(),
            aeria_workspace::MAX_TRANSLATION_PAGE_SIZE,
        )
        .map_err(tool_error)?;
    for row in page.rows {
        for cell in row.cells {
            let state = cell
                .translation
                .as_ref()
                .map(|overlay| overlay.review_state);
            let included = match filter {
                JobFilter::Untranslated => state.is_none(),
                JobFilter::NeedsReview => state == Some(aeria_core::ReviewState::NeedsReview),
                JobFilter::UntranslatedAndDrafts => {
                    matches!(state, None | Some(aeria_core::ReviewState::Draft))
                }
            };
            if !included {
                continue;
            }
            units.push(ScopedUnit {
                location: UnitLocation {
                    sheet: sheet.to_owned(),
                    row: row.row_id,
                    subrow: row.subrow_id,
                    column: Some(cell.source_binding.column_index()),
                },
                expected: UnitState {
                    target: cell
                        .translation
                        .as_ref()
                        .map(|overlay| overlay.target_macro.clone()),
                    review_state: state.map(review_label),
                },
                source_chars: cell.source_macro.chars().count(),
            });
        }
    }
    Ok(page
        .next_after
        .map(|cursor| (cursor.row_id(), cursor.subrow_id())))
}

fn filter_label(filter: JobFilter) -> &'static str {
    match filter {
        JobFilter::Untranslated => "untranslated strings",
        JobFilter::NeedsReview => "strings needing review",
        JobFilter::UntranslatedAndDrafts => "untranslated strings and drafts",
    }
}

/// Job access for Angelica in one conversation.
pub(crate) struct DesktopJobs {
    pub(crate) app: tauri::AppHandle,
    pub(crate) store: ConversationStore,
    pub(crate) conversation_id: String,
}

impl DesktopJobs {
    /// Resolves image IDs against the images attached in this
    /// conversation. Images are refused when the jobs model does not
    /// accept them, since workers could not see them.
    fn conversation_images(&self, ids: &[String]) -> Result<Vec<ImageRef>, ToolError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conversation = self.store.load(&self.conversation_id).map_err(tool_error)?;
        let known = message_images(&conversation.messages);
        let images = ids
            .iter()
            .map(|id| {
                known
                    .iter()
                    .find(|image| image.id == *id)
                    .map(|image| (*image).clone())
                    .ok_or_else(|| ToolError::new(format!("this conversation has no image {id:?}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let settings = settings_store(&self.app)
            .and_then(|store| Ok(store.load()?))
            .map_err(|error| ToolError::new(error.message))?;
        if let Some(selection) = settings
            .worker_model
            .as_ref()
            .or(settings.agent_model.as_ref())
            && let Ok(model) = settings.selected_model(selection)
            && !model.vision
        {
            return Err(ToolError::new(format!(
                "the jobs model {} does not accept images; propose the job without images, or ask the user to choose a jobs model that accepts images in Settings",
                model.id
            )));
        }
        Ok(images)
    }
}

impl JobControl for DesktopJobs {
    fn estimate(&self, scope: &JobScope) -> Result<JobEstimate, ToolError> {
        let units = enumerate_scope(&self.app, scope)?;
        Ok(JobEstimate::for_units(
            &units,
            history_chunk_tokens(&self.app),
        ))
    }

    fn propose(
        &self,
        scope: JobScope,
        instructions: String,
        concurrency: Option<u8>,
        images: &[String],
        quality: JobQuality,
    ) -> Result<ProposalOutcome, ToolError> {
        let images = self.conversation_images(images)?;
        let mut estimate = self.estimate(&scope)?;
        // A careful job writes each unit in one piece and rechecks it twice
        // with stronger critics: about twice the tokens.
        if quality == JobQuality::Careful {
            estimate.estimated_tokens = estimate.estimated_tokens.saturating_mul(2);
        }
        let concurrency = aeria_ai::jobs::job_concurrency(concurrency, estimate.chunks);
        if estimate.units == 0 {
            return Err(ToolError::new("the scope has no strings to translate"));
        }
        let sheets = if scope.sheets.is_empty() {
            "every sheet".to_owned()
        } else {
            scope.sheets.join(", ")
        };
        let mut summary = format!(
            "Translate {} {} in {sheets}: {} chunks, about {} tokens",
            estimate.units,
            filter_label(scope.filter),
            estimate.chunks,
            estimate.estimated_tokens
        );
        if !images.is_empty() {
            let _ = write!(summary, ", with {} image(s) for every worker", images.len());
        }
        if quality == JobQuality::Careful {
            summary.push_str(", careful");
        }
        let proposal = JobProposal {
            token_limit: estimate.token_limit(),
            scope,
            instructions,
            images,
            concurrency,
            estimate,
            quality,
        };
        let state = self.app.state::<DesktopState>();
        let _guard = state
            .lock_proposals()
            .map_err(|error| ToolError::new(error.message))?;
        let mut records = self
            .store
            .load_proposals(&self.conversation_id)
            .map_err(tool_error)?;
        let id = aeria_ai::ProviderConfig::new_id();
        records.push(ProposalRecord {
            id: id.clone(),
            file: None,
            job: Some(proposal),
            web: None,
            review: None,
            job_limit: None,
            location: None,
            source: String::new(),
            target: summary,
            expected: UnitState {
                target: None,
                review_state: None,
            },
            status: ProposalStatus::Pending,
            message: None,
            created_at_unix_ms: now_unix_ms(),
        });
        self.store
            .save_proposals(&self.conversation_id, &records)
            .map_err(tool_error)?;
        announce_proposals(&self.app, &self.conversation_id);
        Ok(ProposalOutcome::Pending { proposal_id: id })
    }

    fn jobs(&self) -> Result<Vec<JobSummary>, ToolError> {
        job_store(&self.app)
            .map_err(|error| ToolError::new(error.message))?
            .list()
            .map_err(tool_error)
    }

    fn events(&self, job_id: &str, after: u64) -> Result<Vec<JobEvent>, ToolError> {
        job_store(&self.app)
            .map_err(|error| ToolError::new(error.message))?
            .events(job_id, after)
            .map_err(tool_error)
    }

    fn units(&self, job_id: &str, statuses: &[UnitStatus]) -> Result<Vec<JobUnit>, ToolError> {
        job_store(&self.app)
            .map_err(|error| ToolError::new(error.message))?
            .units(job_id, statuses, 0, 500)
            .map_err(tool_error)
    }

    fn amend(&self, job_id: &str, instructions: &str) -> Result<(), ToolError> {
        job_store(&self.app)
            .map_err(|error| ToolError::new(error.message))?
            .amend(job_id, instructions)
            .map(|_| ())
            .map_err(tool_error)
    }

    fn retry(&self, job_id: &str, statuses: &[UnitStatus]) -> Result<u64, ToolError> {
        let store = job_store(&self.app).map_err(|error| ToolError::new(error.message))?;
        let requeued = store.requeue(job_id, statuses).map_err(tool_error)?;
        notify(&self.app, job_id);
        Ok(requeued)
    }

    fn set_concurrency(&self, job_id: &str, concurrency: u8) -> Result<u8, ToolError> {
        set_job_concurrency(&self.app, job_id, concurrency)
            .map(|job| job.spec.concurrency)
            .map_err(|error| ToolError::new(error.message))
    }

    fn propose_limit(&self, job_id: &str, token_limit: u64) -> Result<ProposalOutcome, ToolError> {
        let job = job_store(&self.app)
            .map_err(|error| ToolError::new(error.message))?
            .summary(job_id)
            .map_err(tool_error)?;
        if job.status == JobStatus::Cancelled {
            return Err(ToolError::new("the job was cancelled"));
        }
        if token_limit <= job.used_tokens() {
            return Err(ToolError::new(format!(
                "the limit must be above the {} tokens already used",
                job.used_tokens()
            )));
        }
        let summary = format!(
            "Raise the token limit of job {job_id} from {} to {token_limit}",
            job.spec.token_limit
        );
        let state = self.app.state::<DesktopState>();
        let _guard = state
            .lock_proposals()
            .map_err(|error| ToolError::new(error.message))?;
        let mut records = self
            .store
            .load_proposals(&self.conversation_id)
            .map_err(tool_error)?;
        let id = aeria_ai::ProviderConfig::new_id();
        records.push(ProposalRecord {
            id: id.clone(),
            file: None,
            job: None,
            web: None,
            review: None,
            job_limit: Some(JobLimitProposal {
                job_id: job_id.to_owned(),
                token_limit,
                previous_limit: job.spec.token_limit,
                used_tokens: job.used_tokens(),
                projected_tokens: job.projected_tokens,
            }),
            location: None,
            source: String::new(),
            target: summary,
            expected: UnitState {
                target: None,
                review_state: None,
            },
            status: ProposalStatus::Pending,
            message: None,
            created_at_unix_ms: now_unix_ms(),
        });
        self.store
            .save_proposals(&self.conversation_id, &records)
            .map_err(tool_error)?;
        announce_proposals(&self.app, &self.conversation_id);
        Ok(ProposalOutcome::Pending { proposal_id: id })
    }

    fn control(&self, job_id: &str, action: JobAction) -> Result<JobStatus, ToolError> {
        control_job(&self.app, job_id, action)
            .map(|job| job.status)
            .map_err(|error| ToolError::new(error.message))
    }
}

/// The model job workers use: the jobs model, or Angelica's.
fn jobs_model(app: &tauri::AppHandle) -> CommandResult<Option<aeria_ai::ModelSelection>> {
    let settings = settings_store(app)?.load()?;
    Ok(settings.worker_model.or(settings.agent_model))
}

/// The average tokens per chunk of earlier jobs with the jobs model, if
/// there are enough of them.
fn history_chunk_tokens(app: &tauri::AppHandle) -> Option<u64> {
    let model = jobs_model(app).ok().flatten()?;
    job_store(app)
        .ok()?
        .history_chunk_tokens(&model)
        .ok()
        .flatten()
}

/// Changes a job's token limit and, with `resume`, resumes it when paused.
pub(crate) fn set_job_limit(
    app: &tauri::AppHandle,
    job_id: &str,
    token_limit: u64,
    resume: bool,
) -> CommandResult<JobSummary> {
    let store = job_store(app)?;
    store.set_token_limit(job_id, token_limit)?;
    if resume && store.summary(job_id)?.status == JobStatus::Paused {
        return control_job(app, job_id, JobAction::Resume);
    }
    notify(app, job_id);
    Ok(store.summary(job_id)?)
}

/// Changes how many workers a job runs; a running job follows at once.
pub(crate) fn set_job_concurrency(
    app: &tauri::AppHandle,
    job_id: &str,
    concurrency: u8,
) -> CommandResult<JobSummary> {
    let store = job_store(app)?;
    store.set_concurrency(job_id, concurrency)?;
    notify(app, job_id);
    Ok(store.summary(job_id)?)
}

/// Applies an approved limit proposal; a job paused at its old limit
/// resumes.
pub(crate) fn apply_limit_proposal(
    app: &tauri::AppHandle,
    proposal: &JobLimitProposal,
) -> CommandResult<JobSummary> {
    let job = job_store(app)?.summary(&proposal.job_id)?;
    let at_limit = job.status == JobStatus::Paused && job.used_tokens() >= job.spec.token_limit;
    set_job_limit(app, &proposal.job_id, proposal.token_limit, at_limit)
}

/// Pauses, resumes, or cancels a job.
pub(crate) fn control_job(
    app: &tauri::AppHandle,
    job_id: &str,
    action: JobAction,
) -> CommandResult<JobSummary> {
    let store = job_store(app)?;
    let job = store.summary(job_id)?;
    match (action, job.status) {
        (_, JobStatus::Cancelled) => {
            return Err(CommandError::new(
                "angelicaJobInvalid",
                "the job was cancelled",
            ));
        }
        (JobAction::Pause, JobStatus::Running) => {
            store.set_status(job_id, JobStatus::Paused, Some("paused by the user"))?;
        }
        (JobAction::Resume, JobStatus::Paused | JobStatus::Completed) => {
            store.set_status(job_id, JobStatus::Running, None)?;
            spawn_runner(app, job_id);
        }
        (JobAction::Cancel, _) => {
            app.state::<DesktopState>().stop_job_runner(job_id);
            store.set_status(job_id, JobStatus::Cancelled, Some("cancelled"))?;
        }
        _ => {}
    }
    notify(app, job_id);
    Ok(store.summary(job_id)?)
}

/// Creates and starts the job of an approved proposal. Returns its ID.
pub(crate) fn start_proposed_job(
    app: &tauri::AppHandle,
    conversation_id: &str,
    proposal: &JobProposal,
) -> CommandResult<String> {
    let settings = settings_store(app)?.load()?;
    let model = settings
        .worker_model
        .clone()
        .or_else(|| settings.agent_model.clone())
        .ok_or_else(|| {
            CommandError::new(
                "aiNoAgentModel",
                "choose a model for Angelica or for jobs in Settings → AI",
            )
        })?;
    settings
        .selected_model(&model)
        .map_err(|message| CommandError::new("aiInvalidSettings", message))?;
    let units = enumerate_scope(app, &proposal.scope)
        .map_err(|error| CommandError::new("angelicaJobInvalid", error.0))?;
    let spec = JobSpec {
        scope: proposal.scope.clone(),
        instructions: proposal.instructions.clone(),
        model,
        token_limit: proposal.token_limit,
        concurrency: proposal
            .concurrency
            .clamp(1, aeria_ai::jobs::MAX_CONCURRENCY),
        images: proposal.images.clone(),
        quality: proposal.quality,
    };
    let job = job_store(app)?.create(conversation_id, &spec, &units)?;
    spawn_runner(app, &job.id);
    notify(app, &job.id);
    Ok(job.id)
}

/// Requests one job sends at once across all its lanes. Each lane's chunk
/// sends its parts and critics in parallel; this keeps a job with many
/// lanes within what a provider accepts.
const JOB_PARALLEL_REQUESTS: usize = 24;
/// How often a request waiting for a free slot looks again.
const GATE_POLL: Duration = Duration::from_millis(50);

/// A limit of requests in flight, shared by a job's lanes.
#[derive(Debug)]
struct RequestGate {
    free: std::sync::Mutex<usize>,
}

/// A request's slot; dropping it frees the slot.
struct GateSlot<'a> {
    gate: &'a RequestGate,
}

impl RequestGate {
    const fn new(slots: usize) -> Self {
        Self {
            free: std::sync::Mutex::new(slots),
        }
    }

    /// Waits for a free slot and takes it.
    async fn enter(&self) -> GateSlot<'_> {
        loop {
            {
                let mut free = self
                    .free
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if *free > 0 {
                    *free -= 1;
                    return GateSlot { gate: self };
                }
            }
            tokio::time::sleep(GATE_POLL).await;
        }
    }
}

impl Drop for GateSlot<'_> {
    fn drop(&mut self) {
        let mut free = self
            .gate
            .free
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *free += 1;
    }
}

/// One job's runner context. The store and root are fixed when the runner
/// starts, so a job never reads or writes another project opened later.
#[derive(Clone)]
struct JobRun {
    app: tauri::AppHandle,
    job_id: String,
    store: JobStore,
    root: PathBuf,
    workers: Arc<WorkerBoard>,
    gate: Arc<RequestGate>,
}

impl JobRun {
    /// Runs a blocking store operation.
    async fn with_store<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&JobStore, &str) -> Result<T, JobError> + Send + 'static,
    ) -> CommandResult<T> {
        let store = self.store.clone();
        let id = self.job_id.clone();
        run_blocking(move || Ok(operation(&store, &id)?)).await
    }

    /// Whether the job's project is still the open one.
    fn project_open(&self) -> bool {
        repository_root(&self.app).is_ok_and(|root| root == self.root)
    }
}

/// Starts a job's lanes in the active project unless they already run.
pub(crate) fn spawn_runner(app: &tauri::AppHandle, job_id: &str) {
    let (Ok(store), Ok(root)) = (job_store(app), repository_root(app)) else {
        return;
    };
    let workers = Arc::new(WorkerBoard::default());
    let run = JobRun {
        app: app.clone(),
        job_id: job_id.to_owned(),
        store,
        root,
        workers: Arc::clone(&workers),
        gate: Arc::new(RequestGate::new(JOB_PARALLEL_REQUESTS)),
    };
    app.state::<DesktopState>()
        .start_job_runner(job_id.to_owned(), workers, move || {
            tauri::async_runtime::spawn(run_job(run))
        });
}

/// How often a runner checks whether the job's worker count changed.
const CONCURRENCY_CHECK: Duration = Duration::from_secs(2);

/// The job's worker count and whether it still runs.
async fn runner_state(run: &JobRun) -> Option<(u8, bool)> {
    run.with_store(|store, id| {
        let job = store.summary(id)?;
        Ok((
            job.spec.concurrency.max(1),
            job.status == JobStatus::Running,
        ))
    })
    .await
    .ok()
}

/// Runs a job's lanes until none has work. Lanes run in a join set, so
/// aborting the runner aborts them too. When the job's worker count grows,
/// the missing lanes start; lanes above a smaller count stop by themselves
/// after their current chunk.
/// Learns from a job that is about to complete, so its report says what it
/// learned; `None` when the job does not complete now.
async fn learn_before_completion(run: &JobRun) -> Option<String> {
    let completing = run
        .with_store(|store, id| {
            let job = store.summary(id)?;
            Ok(job.status == JobStatus::Running
                && job.counts.running == 0
                && job.counts.pending == 0)
        })
        .await
        .unwrap_or(false);
    if completing && run.project_open() {
        job_learning::learn(run).await
    } else {
        None
    }
}

async fn run_job(run: JobRun) {
    let mut concurrency = runner_state(&run).await.map_or(1, |(count, _)| count);
    run.workers.reset(u32::from(concurrency));
    // The scope is studied once, before the first chunk.
    if let Err(reason) = job_study::study_scope(&run).await {
        pause_with_reason(&run, reason).await;
        run.app
            .state::<DesktopState>()
            .finish_job_runner(&run.job_id);
        notify(&run.app, &run.job_id);
        return;
    }
    let mut lanes = JoinSet::new();
    let mut live = std::collections::BTreeSet::new();
    for lane in 0..usize::from(concurrency) {
        live.insert(lane);
        let run = run.clone();
        lanes.spawn(async move {
            run_lane(run, lane).await;
            lane
        });
    }
    while !live.is_empty() {
        match tokio::time::timeout(CONCURRENCY_CHECK, lanes.join_next()).await {
            Ok(Some(Ok(lane))) => {
                live.remove(&lane);
            }
            // A lane that panicked or was aborted is gone too.
            Ok(Some(Err(_)) | None) => live.clear(),
            Err(_) => {}
        }
        let Some((count, running)) = runner_state(&run).await else {
            continue;
        };
        if count == concurrency || !running || live.is_empty() {
            concurrency = count;
            continue;
        }
        concurrency = count;
        run.workers.ensure(u32::from(count));
        for lane in 0..usize::from(count) {
            if live.insert(lane) {
                let run = run.clone();
                lanes.spawn(async move {
                    run_lane(run, lane).await;
                    lane
                });
            }
        }
    }

    let learned = learn_before_completion(&run).await;
    let finished = run
        .with_store(move |store, id| {
            let job = store.summary(id)?;
            if job.status != JobStatus::Running || job.counts.running > 0 {
                return Ok(None);
            }
            if job.counts.pending > 0 {
                // Resumed while the lanes were stopping.
                return Ok(Some(None));
            }
            store.set_status(id, JobStatus::Completed, None)?;
            let counts = job.counts;
            let message = format!(
                "finished: {} final, {} need review, {} rejected, {} failed, {} skipped because they changed",
                counts.finished + counts.drafted, counts.flagged, counts.rejected, counts.failed, counts.conflict
            );
            let message = match &learned {
                Some(learned) => format!("{message}; learned: {learned}"),
                None => message,
            };
            store.add_event(id, "completed", &message, None)?;
            Ok(Some(Some((job.conversation_id, message))))
        })
        .await;
    run.app
        .state::<DesktopState>()
        .finish_job_runner(&run.job_id);
    notify(&run.app, &run.job_id);
    match finished {
        Ok(Some(Some((conversation_id, message)))) if run.project_open() => {
            wake_angelica(
                &run.app,
                &conversation_id,
                job_update(&run.job_id, &message),
            )
            .await;
        }
        Ok(Some(None)) if run.project_open() => spawn_runner(&run.app, &run.job_id),
        _ => {}
    }
}

/// The automatic message that tells Angelica about a job.
fn job_update(job_id: &str, update: &str) -> String {
    format!(
        "[Aeria] Job {job_id} {update}. Check job_status and job_events, then tell the user what happened and what you suggest."
    )
}

/// Pauses a job with a reason, records it, and wakes Angelica.
async fn pause_with_reason(run: &JobRun, reason: String) {
    let paused_reason = reason.clone();
    let conversation = run
        .with_store(move |store, id| {
            let job = store.summary(id)?;
            if job.status != JobStatus::Running {
                return Ok(None);
            }
            store.set_status(id, JobStatus::Paused, Some(&paused_reason))?;
            store.add_event(id, "paused", &paused_reason, None)?;
            Ok(Some(job.conversation_id))
        })
        .await;
    notify(&run.app, &run.job_id);
    if let Ok(Some(conversation_id)) = conversation
        && run.project_open()
    {
        wake_angelica(
            &run.app,
            &conversation_id,
            job_update(&run.job_id, &format!("paused: {reason}")),
        )
        .await;
    }
}

enum ChunkEnd {
    Done,
    /// A retryable provider failure; the chunk's strings are pending again.
    Retry(String),
    /// The job cannot continue.
    Stop(String),
}

/// Why a job must pause before its next chunk, if it must.
fn pause_reason(job: &JobSummary) -> Option<String> {
    if job.usage.prompt_tokens + job.usage.completion_tokens >= job.spec.token_limit {
        return Some(format!(
            "the token limit of {} was reached",
            job.spec.token_limit
        ));
    }
    let processed = job.counts.processed();
    (processed >= REJECTION_SAMPLE && job.counts.rejected * 10 > processed * 3).then(|| {
        format!(
            "{} of {processed} translations were rejected for breaking the string structure",
            job.counts.rejected
        )
    })
}

/// Runs one lane (zero-based) until the job stops or has no chunk left.
async fn run_lane(run: JobRun, lane: usize) {
    lane_loop(&run, lane).await;
    run.workers.set_phase(lane, WorkerPhase::Stopped);
}

async fn lane_loop(run: &JobRun, lane: usize) {
    let mut failures = 0_u32;
    let mut store_failures = 0_u32;
    loop {
        run.workers.set_phase(lane, WorkerPhase::Idle);
        if !run.project_open() {
            pause_with_reason(run, "the project was closed".to_owned()).await;
            return;
        }
        let claimed = run
            .with_store(move |store, id| {
                let job = store.summary(id)?;
                // A lane above a lowered worker count stops here.
                if job.status != JobStatus::Running || lane >= usize::from(job.spec.concurrency) {
                    return Ok(Ok(None));
                }
                if let Some(reason) = pause_reason(&job) {
                    return Ok(Err(reason));
                }
                Ok(Ok(store.claim_chunk(id)?.map(|units| (units, job.spec))))
            })
            .await;
        let (units, spec) = match claimed {
            Ok(Ok(Some(claim))) => claim,
            Ok(Ok(None)) => return,
            Ok(Err(reason)) => {
                pause_with_reason(run, reason).await;
                return;
            }
            // A busy job store is retried; only a lasting failure stops.
            Err(error) => {
                store_failures += 1;
                if store_failures >= MAX_STORE_FAILURES {
                    pause_with_reason(run, error.message).await;
                    return;
                }
                tokio::time::sleep(STORE_BACKOFF * store_failures).await;
                continue;
            }
        };
        store_failures = 0;
        notify(&run.app, &run.job_id);
        if let Some(first) = units.first() {
            let (chunk, sheet) = (first.chunk, first.location.sheet.clone());
            let rows = units.iter().map(|unit| unit.location.row);
            let rows = (
                rows.clone().min().unwrap_or(first.location.row),
                rows.max().unwrap_or(first.location.row),
            );
            let count = u32::try_from(units.len()).unwrap_or(u32::MAX);
            run.workers.update(lane, |activity, now| {
                activity.start_chunk(chunk, &sheet, rows, count, now);
            });
        }
        match run_chunk(run, &spec, units, lane).await {
            ChunkEnd::Done => {
                failures = 0;
                run.workers.update(lane, |activity, _| {
                    activity.chunks_done += 1;
                    activity.last_error = None;
                });
            }
            ChunkEnd::Retry(message) => {
                failures += 1;
                if failures >= MAX_PROVIDER_FAILURES {
                    pause_with_reason(run, format!("the provider keeps failing: {message}")).await;
                    return;
                }
                let delay = PROVIDER_BACKOFF * failures;
                run.workers.update(lane, |activity, now| {
                    let retry_at = now + u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
                    activity.set_backoff(retry_at, message, now);
                });
                tokio::time::sleep(delay).await;
            }
            ChunkEnd::Stop(reason) => {
                pause_with_reason(run, reason).await;
                return;
            }
        }
        notify(&run.app, &run.job_id);
    }
}

/// The worker's view of the project for one job.
struct DesktopJobHost {
    run: JobRun,
}

impl DesktopJobHost {
    fn reader(&self) -> Result<DesktopReader, ToolError> {
        if !self.run.project_open() {
            return Err(ToolError::new("the job's project is no longer open"));
        }
        Ok(DesktopReader {
            app: self.run.app.clone(),
        })
    }
}

impl JobHost for DesktopJobHost {
    fn context(&self, location: &UnitLocation) -> Result<UnitContext, ToolError> {
        let binding = binding_of(location)?;
        let mut context = self.reader()?.with_session(|session| {
            let source = session.source_macro(&binding).map_err(tool_error)?;
            let row = session_row(session, &location.sheet, location.row, location.subrow)?;
            let cell = row.as_ref().and_then(|row| {
                row.cells
                    .iter()
                    .find(|cell| Some(cell.column) == location.column)
            });
            Ok(UnitContext {
                source,
                context: row
                    .as_ref()
                    .map(|row| row.context.clone())
                    .unwrap_or_default(),
                current_target: cell.and_then(|cell| cell.target.clone()),
                note: cell.and_then(|cell| cell.note.clone()),
                memory: Vec::new(),
            })
        })?;
        // Translation memory is a help; a missing index never stops a job.
        context.memory = DesktopSearch {
            app: self.run.app.clone(),
        }
        .similar_translations(&context.source, Some(location), JOB_MEMORY_MATCHES)
        .unwrap_or_default();
        Ok(context)
    }

    fn write(
        &self,
        location: &UnitLocation,
        target: &str,
        expected: &UnitState,
        review: ReviewLabel,
    ) -> Result<(), WriteFailure> {
        let state = self.run.app.state::<DesktopState>();
        let mut project = state
            .lock_project()
            .map_err(|error| WriteFailure::Failed(error.message))?;
        let session = project
            .as_mut()
            .filter(|session| session.repository_root() == self.run.root)
            .ok_or_else(|| {
                WriteFailure::Failed("the job's project is no longer open".to_owned())
            })?;
        write_assisted(
            &self.run.app,
            session,
            location,
            target,
            expected,
            false,
            Some(review),
        )
        .map_err(|error| match error {
            AssistedWriteError::Conflict { .. } => WriteFailure::Conflict(error.to_string()),
            other => WriteFailure::Failed(other.to_string()),
        })
    }

    fn report(&self, message: &str, location: Option<&UnitLocation>) {
        let _ = self
            .run
            .store
            .add_event(&self.run.job_id, "issue", message, location);
    }
}

/// Worker reads, refused once the job's project is no longer open.
struct JobReader {
    run: JobRun,
}

impl JobReader {
    fn reader(&self) -> Result<DesktopReader, ToolError> {
        DesktopJobHost {
            run: self.run.clone(),
        }
        .reader()
    }
}

impl ProjectReader for JobReader {
    fn facts(&self) -> Result<aeria_ai::tools::ProjectFacts, ToolError> {
        self.reader()?.facts()
    }

    fn sheets(&self) -> Result<Vec<aeria_ai::tools::SheetSummary>, ToolError> {
        self.reader()?.sheets()
    }

    fn rows(
        &self,
        sheet: &str,
        after: Option<(u32, u16)>,
        limit: u32,
    ) -> Result<aeria_ai::tools::RowsPage, ToolError> {
        self.reader()?.rows(sheet, after, limit)
    }

    fn row(
        &self,
        sheet: &str,
        row: u32,
        subrow: u16,
    ) -> Result<Option<aeria_ai::tools::RowSnapshot>, ToolError> {
        self.reader()?.row(sheet, row, subrow)
    }

    fn other_languages(
        &self,
        sheet: &str,
        row: u32,
        subrow: u16,
        column: u32,
    ) -> Result<Vec<(String, Option<String>)>, ToolError> {
        self.reader()?.other_languages(sheet, row, subrow, column)
    }

    fn pending_changes(&self) -> Result<serde_json::Value, ToolError> {
        self.reader()?.pending_changes()
    }

    fn unit_history(&self, unit_id: &str, limit: u32) -> Result<serde_json::Value, ToolError> {
        self.reader()?.unit_history(unit_id, limit)
    }

    fn navigate(&self, _: &UnitLocation) -> Result<(), ToolError> {
        Err(ToolError::new("workers cannot move the editor"))
    }

    fn project_file(
        &self,
        file: aeria_ai::guidance::ProjectFile,
    ) -> Result<Option<String>, ToolError> {
        self.reader()?.project_file(file)
    }

    fn dialogue(
        &self,
        sheet: &str,
    ) -> Result<Option<aeria_ai::dialogue::SheetDialogue>, ToolError> {
        self.reader()?.dialogue(sheet)
    }

    fn speakers(&self, query: &str) -> Result<Vec<(String, usize)>, ToolError> {
        self.reader()?.speakers(query)
    }

    fn speaker_lines(
        &self,
        speaker: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(usize, Vec<UnitLocation>), ToolError> {
        self.reader()?.speaker_lines(speaker, offset, limit)
    }
}

/// Most requests of one chunk sent at once. A unit's parts and critics run
/// in parallel; this keeps one chunk from flooding the provider.
const CHUNK_PARALLEL_REQUESTS: usize = 8;

/// Loads each string's context, the scene, the project knowledge, and the
/// job's current instructions.
fn prepare_chunk(run: &JobRun, units: &[JobUnit]) -> CommandResult<PreparedUnit> {
    let reader = JobReader { run: run.clone() };
    let facts = reader.facts().ok();
    let job = run.store.summary(&run.job_id)?;
    Ok(prepare_unit(
        units,
        &DesktopJobHost { run: run.clone() },
        &reader,
        &Knowledge::load(&run.root),
        facts.as_ref(),
        &job.spec.instructions,
    ))
}

/// Sends the localizer's and researchers' requests for one lane, in
/// parallel, and follows them on the lane's activity.
struct JobCaller {
    run: JobRun,
    lane: usize,
    client: aeria_ai::OpenAiCompatibleClient,
    endpoint: aeria_ai::ProviderEndpoint,
    model: String,
    efforts: Vec<aeria_ai::ReasoningEffort>,
    /// The job's effort, a ceiling for every role.
    ceiling: Option<aeria_ai::ReasoningEffort>,
    /// The job is careful: every role asks for a high effort.
    careful: bool,
    session: String,
    images: Vec<ImageRef>,
    /// The images' data, loaded when the model accepts images.
    payloads: Option<ImagePayloads>,
    /// Tokens used so far, kept when the work is interrupted.
    spent: std::sync::Mutex<aeria_ai::chat::Usage>,
}

impl JobCaller {
    /// A caller for the job's model, with the job's images.
    async fn for_job(
        run: &JobRun,
        spec: &JobSpec,
        lane: usize,
        session: String,
    ) -> CommandResult<Self> {
        let endpoint = resolve_endpoint(&run.app, spec.model.provider_id.clone()).await?;
        let client = run.app.state::<DesktopState>().ai_client()?;
        let setup_run = run.clone();
        let selection = spec.model.clone();
        let (model, images, payloads) = run_blocking(move || {
            let settings = settings_store(&setup_run.app)?.load()?;
            let model = settings
                .selected_model(&selection)
                .map_err(|message| CommandError::new("aiInvalidSettings", message))?
                .clone();
            let job = setup_run.store.summary(&setup_run.job_id)?;
            // Images stay with the job's conversation; a deleted
            // conversation leaves the localizer without them.
            let payloads = model.vision.then(|| {
                project_conversation_store(&setup_run.app, &setup_run.root).map_or_else(
                    |_| ImagePayloads::default(),
                    |store| {
                        let images: Vec<&ImageRef> = job.spec.images.iter().collect();
                        load_image_payloads(&store, &job.conversation_id, &images)
                    },
                )
            });
            Ok((model, job.spec.images, payloads))
        })
        .await?;
        Ok(Self {
            run: run.clone(),
            lane,
            client,
            endpoint,
            model: model.id,
            efforts: model.reasoning_efforts,
            ceiling: spec.model.effort,
            careful: spec.quality == JobQuality::Careful,
            session,
            images,
            payloads,
            spent: std::sync::Mutex::new(aeria_ai::chat::Usage::default()),
        })
    }

    async fn send(
        &self,
        index: usize,
        request: LocalizerRequest,
    ) -> Result<(String, aeria_ai::chat::Usage), ProviderError> {
        // The job's images show where the strings appear; they go to the
        // requests that write.
        let images = if matches!(request.role, Role::Contract | Role::Writer) {
            self.images.clone()
        } else {
            Vec::new()
        };
        let messages = vec![ChatMessage::User {
            content: request.user,
            automatic: false,
            images,
        }];
        let chat = ChatRequest {
            model: &self.model,
            effort: effort_for(request.role, &self.efforts, self.ceiling, self.careful),
            system: &request.system,
            messages: &messages,
            tools: &[],
            turn_start: 0,
            images: self.payloads.as_ref(),
        };
        let session = format!("{}-{}-{index}", self.session, request.role.as_str());
        let run = &self.run;
        let lane = self.lane;
        let mut on_delta = |delta: StreamDelta| {
            let reasoning = matches!(delta, StreamDelta::Reasoning(_));
            run.workers
                .update(lane, |activity, now| activity.stream(reasoning, now));
        };
        // All lanes of the job share one limit of requests in flight.
        let _slot = self.run.gate.enter().await;
        let response = self
            .client
            .stream_chat(&self.endpoint, &session, &chat, &mut on_delta)
            .await?;
        let usage = response.usage.unwrap_or_default();
        if let Ok(mut spent) = self.spent.lock() {
            spent.add(usage);
        }
        self.run.workers.update(self.lane, |activity, now| {
            activity.answered(usage.prompt_tokens + usage.completion_tokens, now);
        });
        Ok((response.content, usage))
    }

    fn spent(&self) -> aeria_ai::chat::Usage {
        self.spent.lock().map(|spent| *spent).unwrap_or_default()
    }
}

impl Caller for JobCaller {
    fn call_all(&self, requests: Vec<LocalizerRequest>) -> Replies<'_> {
        Box::pin(async move {
            let mut replies = Vec::with_capacity(requests.len());
            let mut requests = requests.into_iter().enumerate().peekable();
            while requests.peek().is_some() {
                let batch: Vec<_> = requests.by_ref().take(CHUNK_PARALLEL_REQUESTS).collect();
                let count = u32::try_from(batch.len()).unwrap_or(u32::MAX);
                self.run
                    .workers
                    .update(self.lane, |activity, now| activity.send(count, now));
                let results = futures_join(
                    batch
                        .into_iter()
                        .map(|(index, request)| {
                            Box::pin(self.send(index, request))
                                as Pin<Box<dyn Future<Output = _> + Send + '_>>
                        })
                        .collect(),
                )
                .await;
                for result in results {
                    replies.push(result?);
                }
            }
            Ok(replies)
        })
    }

    fn step(&self, step: Step) {
        self.run.workers.update(self.lane, |activity, now| {
            activity.set_step(step.into(), now);
        });
    }
}

/// Awaits futures together and returns their outputs in order.
async fn futures_join<T>(futures: Vec<Pin<Box<dyn Future<Output = T> + Send + '_>>>) -> Vec<T> {
    let mut futures: Vec<Option<Pin<Box<dyn Future<Output = T> + Send + '_>>>> =
        futures.into_iter().map(Some).collect();
    let mut outputs: Vec<Option<T>> = futures.iter().map(|_| None).collect();
    std::future::poll_fn(|context| {
        let mut pending = false;
        for (future, output) in futures.iter_mut().zip(outputs.iter_mut()) {
            if let Some(running) = future {
                match running.as_mut().poll(context) {
                    std::task::Poll::Ready(value) => {
                        *output = Some(value);
                        *future = None;
                    }
                    std::task::Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    outputs.into_iter().flatten().collect()
}

/// Records a chunk's outcomes and usage, retrying a busy job store so the
/// chunk's strings never stay claimed.
async fn record_chunk(
    run: &JobRun,
    outcomes: Vec<(u64, UnitStatus, Option<String>)>,
    usage: aeria_ai::chat::Usage,
) {
    for attempt in 1..=MAX_STORE_FAILURES {
        let outcomes = outcomes.clone();
        let recorded = run
            .with_store(move |store, id| {
                store.finish_units(id, &outcomes)?;
                store.add_usage(id, usage)
            })
            .await;
        if recorded.is_ok() {
            return;
        }
        tokio::time::sleep(STORE_BACKOFF * attempt).await;
    }
}

/// How a chunk the provider interrupted ends, and what its strings become:
/// transient failures and a rejected key requeue them, other failures fail
/// them.
fn interrupted(error: ProviderError) -> (ChunkEnd, (UnitStatus, Option<String>)) {
    match error {
        ProviderError::RateLimited { .. }
        | ProviderError::Unavailable { .. }
        | ProviderError::Timeout
        | ProviderError::Network { .. } => (
            ChunkEnd::Retry(error.to_string()),
            (UnitStatus::Pending, None),
        ),
        ProviderError::Unauthorized { .. } => (
            ChunkEnd::Stop(error.to_string()),
            (UnitStatus::Pending, None),
        ),
        error => (
            ChunkEnd::Done,
            (UnitStatus::Failed, Some(error.to_string())),
        ),
    }
}

/// Writes the localizer's results: a final translation as reviewed, one
/// with an open finding as needing review, and records why a string was
/// not written. Written translations are recorded with their sources, so
/// later jobs know which translations agents wrote.
fn write_outcomes(
    run: &JobRun,
    units: &[JobUnit],
    outcomes: Vec<LineOutcome>,
    sources: &BTreeMap<usize, String>,
) -> Vec<(u64, UnitStatus, Option<String>)> {
    let host = DesktopJobHost { run: run.clone() };
    let mut recorded = Vec::new();
    let mut written = Vec::new();
    for outcome in outcomes {
        let Some(unit) = units.get(outcome.task) else {
            continue;
        };
        let (review, status, message) = match &outcome.finish {
            Finish::Rejected(reason) => {
                recorded.push((unit.seq, UnitStatus::Rejected, Some(reason.clone())));
                continue;
            }
            Finish::Final => (ReviewLabel::Reviewed, UnitStatus::Finished, None),
            Finish::NeedsReview(reason) => (
                ReviewLabel::NeedsReview,
                UnitStatus::Flagged,
                Some(reason.clone()),
            ),
        };
        let Some(target) = outcome.target.as_deref() else {
            recorded.push((
                unit.seq,
                UnitStatus::Rejected,
                Some("no translation was produced".to_owned()),
            ));
            continue;
        };
        recorded.push(
            match host.write(&unit.location, target, &unit.expected, review) {
                Ok(()) => {
                    if let Some(reason) = &message {
                        host.report(&format!("needs review: {reason}"), Some(&unit.location));
                    }
                    written.push((
                        unit.location.clone(),
                        sources.get(&outcome.task).cloned().unwrap_or_default(),
                        target.to_owned(),
                    ));
                    (unit.seq, status, message)
                }
                Err(WriteFailure::Conflict(message)) => {
                    (unit.seq, UnitStatus::Conflict, Some(message))
                }
                Err(WriteFailure::Failed(message)) => (unit.seq, UnitStatus::Failed, Some(message)),
            },
        );
    }
    // The record helps learning; a busy store never fails the chunk.
    let _ = run.store.add_written(&run.job_id, &written);
    recorded
}

/// A script line's address, `sheet:row:subrow:column`, as a location.
fn address_location(address: &str) -> Option<UnitLocation> {
    let mut parts = address.rsplitn(4, ':');
    let (column, subrow, row, sheet) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    Some(UnitLocation {
        sheet: sheet.to_owned(),
        row: row.parse().ok()?,
        subrow: subrow.parse().ok()?,
        column: Some(column.parse().ok()?),
    })
}

/// A script line as learning sees it: its address, its source, and the
/// chunk string it translates.
struct LineRecord {
    address: String,
    source: String,
    task: Option<usize>,
}

/// What a finished unit leaves for learning: its story, for the units of
/// its sheet that follow; the critics' findings, for the mentor; and the
/// findings against the knowledge itself, for Angelica.
fn keep_learning(
    run: &JobRun,
    sheet: &str,
    dialogue: bool,
    lines: &[LineRecord],
    result: &LocalizeResult,
) {
    if dialogue && let Some(story) = contract_story(&result.contract) {
        let knowledge = Knowledge::load(&run.root);
        let text = match knowledge.story(sheet) {
            Some(known) if !known.text.contains(&story) => {
                let joined = format!("{}\n\n{story}", known.text);
                let start = joined
                    .char_indices()
                    .rev()
                    .nth(3_000)
                    .map_or(0, |(index, _)| index);
                joined[start..].to_owned()
            }
            Some(known) => known.text.clone(),
            None => story,
        };
        let _ = aeria_ai::knowledge::set_section(
            &run.root,
            aeria_ai::knowledge::KnowledgeFile::Story,
            aeria_ai::knowledge::Section::new(sheet, &text).with("source", "localizer"),
        );
    }
    let targets: BTreeMap<usize, &str> = result
        .outcomes
        .iter()
        .filter_map(|outcome| Some((outcome.task, outcome.target.as_deref()?)))
        .collect();
    let mut findings = Vec::new();
    for flag in &result.flags {
        let Some(line) = lines.get(flag.line) else {
            continue;
        };
        let location = address_location(&line.address);
        if flag.problem.starts_with("KNOWLEDGE") {
            let _ = run
                .store
                .add_event(&run.job_id, "knowledge", &flag.problem, location.as_ref());
        }
        findings.push(Finding {
            role: flag.role.as_str().to_owned(),
            major: flag.major,
            problem: flag.problem.clone(),
            location,
            source: line.source.clone(),
            target: line
                .task
                .and_then(|task| targets.get(&task))
                .map(|target| (*target).to_owned())
                .unwrap_or_default(),
        });
    }
    let _ = run.store.add_findings(&run.job_id, &findings);
}

/// Decides the unit's terms the knowledge lacks and, when some were
/// written, reads the unit's knowledge again.
async fn study_unit_terms(
    run: &JobRun,
    caller: &JobCaller,
    unit: &mut aeria_ai::localizer::UnitOfWork,
) -> Result<(), ProviderError> {
    caller.step(Step::Terms);
    let host = job_study::DesktopKnowledge {
        app: run.app.clone(),
        root: run.root.clone(),
    };
    let written = study_terms(caller, &host, unit).await?;
    if !written.is_empty() {
        let root = run.root.clone();
        let studied = unit.clone();
        if let Ok(text) =
            run_blocking(move || Ok(unit_knowledge(&Knowledge::load(&root), &studied))).await
        {
            unit.knowledge = text;
        }
    }
    Ok(())
}

/// Localizes one claimed chunk and records the outcomes.
async fn run_chunk(run: &JobRun, spec: &JobSpec, units: Vec<JobUnit>, lane: usize) -> ChunkEnd {
    let chunk = units.first().map_or(0, |unit| unit.chunk);
    let released: Vec<_> = units
        .iter()
        .map(|unit| (unit.seq, UnitStatus::Pending, None))
        .collect();
    let caller = match JobCaller::for_job(run, spec, lane, format!("{}-{chunk}", run.job_id)).await
    {
        Ok(caller) => caller,
        Err(error) => {
            record_chunk(run, released, aeria_ai::chat::Usage::default()).await;
            return ChunkEnd::Stop(error.message);
        }
    };
    let prepare_run = run.clone();
    let prepare_units = units.clone();
    let prepared = match run_blocking(move || prepare_chunk(&prepare_run, &prepare_units)).await {
        Ok(prepared) => prepared,
        Err(error) => {
            record_chunk(run, released, aeria_ai::chat::Usage::default()).await;
            return ChunkEnd::Stop(error.message);
        }
    };
    let outcomes: Vec<(u64, UnitStatus, Option<String>)> = prepared
        .failed
        .iter()
        .filter_map(|(index, reason)| {
            units
                .get(*index)
                .map(|unit| (unit.seq, UnitStatus::Failed, Some(reason.clone())))
        })
        .collect();
    let mut unit = prepared.unit;
    if unit.lines.iter().all(|line| line.task.is_none()) {
        record_chunk(run, outcomes, aeria_ai::chat::Usage::default()).await;
        return ChunkEnd::Done;
    }

    // Terms the knowledge lacks are decided before the contract, so the
    // unit is written with them.
    if let Err(error) = study_unit_terms(run, &caller, &mut unit).await {
        run.workers.set_phase(lane, WorkerPhase::Recording);
        let usage = caller.spent();
        return finish_chunk(
            run,
            &units,
            lane,
            outcomes,
            Err(error),
            usage,
            BTreeMap::new(),
        )
        .await;
    }
    let sheet = unit.sheet.clone();
    let dialogue = unit.domains.iter().any(|domain| {
        matches!(
            domain,
            Domain::Dialogue | Domain::Journal | Domain::Objective | Domain::System
        )
    });
    let records: Vec<LineRecord> = unit
        .lines
        .iter()
        .map(|line| LineRecord {
            address: line.address.clone(),
            source: line.source.clone(),
            task: line.task,
        })
        .collect();
    let sources: BTreeMap<usize, String> = unit
        .lines
        .iter()
        .filter_map(|line| Some((line.task?, line.source.clone())))
        .collect();
    let options = LocalizeOptions {
        careful: caller.careful,
    };
    let result = localize(&caller, unit, options).await;
    run.workers.set_phase(lane, WorkerPhase::Recording);
    if let Ok(result) = &result {
        let learn_run = run.clone();
        let learned = result.clone();
        let _ = run_blocking(move || {
            keep_learning(&learn_run, &sheet, dialogue, &records, &learned);
            Ok(())
        })
        .await;
    }
    finish_chunk(run, &units, lane, outcomes, result, caller.spent(), sources).await
}

/// Writes a localized chunk's results, or returns its strings to the queue
/// when the provider interrupted it, and records the outcomes.
async fn finish_chunk(
    run: &JobRun,
    units: &[JobUnit],
    lane: usize,
    mut outcomes: Vec<(u64, UnitStatus, Option<String>)>,
    result: Result<LocalizeResult, ProviderError>,
    usage: aeria_ai::chat::Usage,
    sources: BTreeMap<usize, String>,
) -> ChunkEnd {
    let end =
        match result {
            Ok(result) => {
                let write_run = run.clone();
                let write_units = units.to_vec();
                let written = run_blocking(move || {
                    Ok(write_outcomes(
                        &write_run,
                        &write_units,
                        result.outcomes,
                        &sources,
                    ))
                })
                .await
                .unwrap_or_default();
                let finished = u32::try_from(written.len()).unwrap_or(u32::MAX);
                run.workers
                    .update(lane, |activity, _| activity.finished_units = finished);
                outcomes.extend(written);
                // A string the localizer did not return fails; none stays
                // claimed.
                let known: BTreeSet<u64> = outcomes.iter().map(|(seq, _, _)| *seq).collect();
                outcomes.extend(units.iter().filter(|unit| !known.contains(&unit.seq)).map(
                    |unit| {
                        (
                            unit.seq,
                            UnitStatus::Failed,
                            Some("the localizer did not return this string".to_owned()),
                        )
                    },
                ));
                ChunkEnd::Done
            }
            // An interrupted chunk writes nothing; its strings return to the
            // queue or fail.
            Err(error) => {
                let (end, (status, message)) = interrupted(error);
                let failed: BTreeSet<u64> = outcomes.iter().map(|(seq, _, _)| *seq).collect();
                outcomes.extend(
                    units
                        .iter()
                        .filter(|unit| !failed.contains(&unit.seq))
                        .map(|unit| (unit.seq, status, message.clone())),
                );
                end
            }
        };
    record_chunk(run, outcomes, usage).await;
    end
}

#[tauri::command(rename_all = "camelCase")]
/// Lists the active project's translation jobs, newest first.
///
/// # Errors
///
/// Returns `noProjectOpen` or a job storage error.
pub async fn angelica_jobs(app: tauri::AppHandle) -> CommandResult<Vec<JobSummary>> {
    run_blocking(move || Ok(job_store(&app)?.list()?)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Lists a job's strings with the given statuses.
///
/// # Errors
///
/// Returns `noProjectOpen` or a job storage error.
pub async fn angelica_job_units(
    app: tauri::AppHandle,
    job_id: String,
    statuses: Vec<UnitStatus>,
) -> CommandResult<Vec<JobUnit>> {
    run_blocking(move || Ok(job_store(&app)?.units(&job_id, &statuses, 0, 500)?)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Lists a job's events.
///
/// # Errors
///
/// Returns `noProjectOpen` or a job storage error.
pub async fn angelica_job_events(
    app: tauri::AppHandle,
    job_id: String,
) -> CommandResult<Vec<JobEvent>> {
    run_blocking(move || Ok(job_store(&app)?.events(&job_id, 0)?)).await
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Shows what each worker of a running job is doing; empty when the job
/// does not run in this process.
pub async fn angelica_job_workers(app: tauri::AppHandle, job_id: String) -> Vec<WorkerActivity> {
    app.state::<DesktopState>()
        .job_workers(&job_id)
        .map(|workers| workers.snapshot())
        .unwrap_or_default()
}

#[tauri::command(rename_all = "camelCase")]
/// Pauses, resumes, or cancels a job.
///
/// # Errors
///
/// Returns `angelicaJobNotFound`, `angelicaJobInvalid`, or a storage error.
pub async fn angelica_job_control(
    app: tauri::AppHandle,
    job_id: String,
    action: JobAction,
) -> CommandResult<JobSummary> {
    run_blocking(move || control_job(&app, &job_id, action)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Requeues a job's rejected, failed, or skipped strings and resumes it.
///
/// # Errors
///
/// Returns `angelicaJobNotFound` or a storage error.
pub async fn angelica_job_retry(
    app: tauri::AppHandle,
    job_id: String,
    statuses: Vec<UnitStatus>,
) -> CommandResult<JobSummary> {
    run_blocking(move || {
        let store = job_store(&app)?;
        if store.requeue(&job_id, &statuses)? > 0 {
            return control_job(&app, &job_id, JobAction::Resume);
        }
        Ok(store.summary(&job_id)?)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Changes how many workers a job runs at once.
///
/// # Errors
///
/// Returns `angelicaJobNotFound`, `angelicaJobInvalid` for a cancelled job,
/// or a storage error.
pub async fn angelica_job_set_concurrency(
    app: tauri::AppHandle,
    job_id: String,
    concurrency: u8,
) -> CommandResult<JobSummary> {
    run_blocking(move || set_job_concurrency(&app, &job_id, concurrency)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Changes a job's token limit and, with `resume`, resumes it when paused.
///
/// # Errors
///
/// Returns `angelicaJobNotFound`, `angelicaJobInvalid` for a cancelled job
/// or a limit not above the tokens used, or a storage error.
pub async fn angelica_job_set_limit(
    app: tauri::AppHandle,
    job_id: String,
    token_limit: u64,
    resume: bool,
) -> CommandResult<JobSummary> {
    run_blocking(move || set_job_limit(&app, &job_id, token_limit, resume)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Removes a job that no longer runs from the list; its drafts stay.
///
/// # Errors
///
/// Returns `angelicaJobNotFound`, `angelicaJobInvalid` for a running job, or
/// a storage error.
pub async fn angelica_job_remove(app: tauri::AppHandle, job_id: String) -> CommandResult<()> {
    run_blocking(move || {
        job_store(&app)?.remove(&job_id)?;
        notify(&app, &job_id);
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {

    use aeria_ai::tools::ReviewLabel;
    use aeria_core::{ReviewState, SourceBinding};

    use super::*;

    fn session() -> (
        (tempfile::TempDir, crate::test_support::TestGame),
        ProjectSession,
    ) {
        let directory = tempfile::tempdir().expect("directory");
        let game = crate::test_support::test_game();
        let session = crate::test_support::test_session(directory.path(), &game);
        ((directory, game), session)
    }

    /// Every sheet's matching strings, as a job over the whole project.
    fn scope_units(session: &ProjectSession, filter: JobFilter) -> Vec<ScopedUnit> {
        let mut units = Vec::new();
        for sheet in crate::dto::ProjectSummaryDto::from_session(session).sheets {
            if sheet.translatable_cell_count == 0 {
                continue;
            }
            let mut after = None;
            while let Some(next) =
                page_scope(session, &sheet.name, after, filter, &mut units).expect("page")
            {
                after = Some(next);
            }
        }
        units
    }

    #[test]
    fn scopes_select_strings_by_review_state_and_never_reviewed_ones() {
        let (_directory, mut session) = session();
        let all = scope_units(&session, JobFilter::Untranslated);
        let unit = all.first().expect("a translatable string");
        let binding = SourceBinding::new(
            unit.location.sheet.clone(),
            unit.location.row,
            unit.location.subrow,
            unit.location.column.expect("column"),
        );
        let source = session.source_macro(&binding).expect("source");
        let id = session.set_target(&binding, &source).expect("target");
        let selected = |session: &ProjectSession, filter| {
            scope_units(session, filter)
                .into_iter()
                .find(|selected| selected.location == unit.location)
        };

        session
            .set_review_state(id, ReviewState::Draft)
            .expect("draft");
        assert_eq!(
            scope_units(&session, JobFilter::Untranslated).len(),
            all.len() - 1
        );
        assert!(selected(&session, JobFilter::NeedsReview).is_none());
        let draft = selected(&session, JobFilter::UntranslatedAndDrafts).expect("draft");
        assert_eq!(draft.expected.target.as_deref(), Some(source.as_str()));
        assert_eq!(draft.expected.review_state, Some(ReviewLabel::Draft));

        session
            .set_review_state(id, ReviewState::NeedsReview)
            .expect("needs review");
        assert!(selected(&session, JobFilter::NeedsReview).is_some());
        assert!(selected(&session, JobFilter::UntranslatedAndDrafts).is_none());

        session
            .set_review_state(id, ReviewState::Reviewed)
            .expect("reviewed");
        for filter in [
            JobFilter::Untranslated,
            JobFilter::NeedsReview,
            JobFilter::UntranslatedAndDrafts,
        ] {
            assert!(selected(&session, filter).is_none(), "{filter:?}");
        }
    }

    #[test]
    fn pauses_follow_the_token_limit_and_the_rejection_share() {
        let mut job = JobSummary {
            id: "j".to_owned(),
            conversation_id: "c".to_owned(),
            status: JobStatus::Running,
            reason: None,
            spec: JobSpec {
                scope: JobScope {
                    sheets: Vec::new(),
                    filter: JobFilter::Untranslated,
                },
                instructions: String::new(),
                model: aeria_ai::ModelSelection {
                    provider_id: "p".to_owned(),
                    model_id: "m".to_owned(),
                    effort: None,
                },
                token_limit: 1_000,
                concurrency: 1,
                images: Vec::new(),
                quality: JobQuality::Fast,
            },
            created_at_unix_ms: 0,
            counts: aeria_ai::jobs::JobCounts::default(),
            active_workers: 0,
            usage: aeria_ai::chat::Usage::default(),
            chunks: 0,
            finished_chunks: 0,
            projected_tokens: None,
        };
        assert_eq!(pause_reason(&job), None);
        job.counts.drafted = 30;
        job.counts.rejected = 9;
        assert_eq!(pause_reason(&job), None, "too few strings to judge");
        job.counts.rejected = 13;
        assert!(pause_reason(&job).expect("rejections").contains("13 of 43"));
        job.counts.rejected = 0;
        job.usage.prompt_tokens = 900;
        job.usage.completion_tokens = 100;
        assert!(pause_reason(&job).expect("tokens").contains("token limit"));
    }
}
