use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use aeria_ai::OpenAiCompatibleClient;
use aeria_ai::chatgpt::AccessToken;
use aeria_atlas::{CancellationHandle, CancellationToken};
use aeria_git::{GitExecutable, UnitAttribution};
use aeria_workspace::ProjectSession;

use crate::error::CommandError;
use crate::job_workers::WorkerBoard;
use crate::search::IndexState;

/// The one authoritative project session owned by the desktop process.
pub struct DesktopState {
    project: Mutex<Option<ProjectSession>>,
    registry: Mutex<()>,
    atlas_job: Mutex<Option<AtlasJob>>,
    next_atlas_job_id: AtomicU64,
    git: OnceLock<GitExecutable>,
    attribution: Mutex<Option<AttributionCache>>,
    ai_client: OnceLock<OpenAiCompatibleClient>,
    web_client: OnceLock<aeria_ai::web::WebClient>,
    angelica_turns: Mutex<Vec<(String, tauri::async_runtime::JoinHandle<()>)>>,
    /// Cached ChatGPT access tokens by provider ID. The async lock also
    /// serializes token refreshes.
    chatgpt_tokens: tauri::async_runtime::Mutex<Vec<(String, AccessToken)>>,
    chatgpt_login: Mutex<Option<(String, tauri::async_runtime::JoinHandle<()>)>>,
    /// Serializes read-modify-write of conversation proposal files.
    proposals: Mutex<()>,
    /// Running translation jobs by job ID, with their workers' activity.
    job_runners: Mutex<Vec<JobRunner>>,
    /// Job stores opened in this process; interrupted jobs are paused once.
    job_stores: Mutex<Vec<PathBuf>>,
    /// Source search indexes by source package ID.
    search_indexes: Mutex<Vec<(String, IndexState)>>,
    /// Running synchronization and export operations, which an application
    /// update must not interrupt.
    activities: Mutex<Vec<(u64, Activity)>>,
    next_activity_id: AtomicU64,
}

/// Work that an application update waits for instead of interrupting.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Activity {
    /// Angelica turns and translation jobs.
    Translation,
    /// Git operations that fetch, push, commit, or change the working tree.
    Sync,
    /// Pack export and publication.
    Export,
    /// Building a source package from the game.
    SourcePackage,
}

/// Marks an activity as running until dropped.
pub(crate) struct ActivityGuard<'a> {
    state: &'a DesktopState,
    id: u64,
}

impl Drop for ActivityGuard<'_> {
    fn drop(&mut self) {
        self.state
            .lock_activities()
            .retain(|(id, _)| *id != self.id);
    }
}

/// Committed unit attribution for one repository commit. It is derived from
/// Git history, so it is disposable and keyed by the exact `HEAD`.
pub(crate) struct AttributionCache {
    pub root: PathBuf,
    pub head: String,
    pub units: Arc<Vec<UnitAttribution>>,
}

struct JobRunner {
    job_id: String,
    handle: tauri::async_runtime::JoinHandle<()>,
    workers: Arc<WorkerBoard>,
}

struct AtlasJob {
    id: String,
    cancellation: CancellationHandle,
    token: CancellationToken,
}

#[derive(Debug)]
pub(crate) struct StartedAtlasJob {
    pub id: String,
}

impl DesktopState {
    /// Creates an application state with no project open.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            project: Mutex::new(None),
            registry: Mutex::new(()),
            atlas_job: Mutex::new(None),
            next_atlas_job_id: AtomicU64::new(1),
            git: OnceLock::new(),
            attribution: Mutex::new(None),
            ai_client: OnceLock::new(),
            web_client: OnceLock::new(),
            angelica_turns: Mutex::new(Vec::new()),
            chatgpt_tokens: tauri::async_runtime::Mutex::const_new(Vec::new()),
            chatgpt_login: Mutex::new(None),
            proposals: Mutex::new(()),
            job_runners: Mutex::new(Vec::new()),
            job_stores: Mutex::new(Vec::new()),
            search_indexes: Mutex::new(Vec::new()),
            activities: Mutex::new(Vec::new()),
            next_activity_id: AtomicU64::new(1),
        }
    }

    /// Records a synchronization or export operation until the guard drops.
    pub(crate) fn begin_activity(&self, activity: Activity) -> ActivityGuard<'_> {
        let id = self.next_activity_id.fetch_add(1, Ordering::Relaxed);
        self.lock_activities().push((id, activity));
        ActivityGuard { state: self, id }
    }

    /// Returns the running activities, each once, in a stable order.
    pub(crate) fn running_activities(&self) -> Vec<Activity> {
        let activities = self.lock_activities();
        self.running_with(&activities)
    }

    /// Runs `operation` only when no activity runs. Synchronization and
    /// export cannot start until it returns.
    ///
    /// # Errors
    ///
    /// Returns `updateBusy`, naming the running activities, without running
    /// `operation`.
    pub(crate) fn while_idle<T>(&self, operation: impl FnOnce() -> T) -> Result<T, CommandError> {
        let activities = self.lock_activities();
        let running = self.running_with(&activities);
        if !running.is_empty() {
            let names: Vec<_> = running
                .iter()
                .map(|activity| format!("{activity:?}"))
                .collect();
            return Err(CommandError::new(
                "updateBusy",
                format!("waiting for running work to finish: {}", names.join(", ")),
            ));
        }
        Ok(operation())
    }

    fn lock_activities(&self) -> MutexGuard<'_, Vec<(u64, Activity)>> {
        self.activities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn running_with(&self, tracked: &[(u64, Activity)]) -> Vec<Activity> {
        let tracks = |wanted: Activity| tracked.iter().any(|(_, activity)| *activity == wanted);
        let translating = self
            .angelica_turns
            .lock()
            .is_ok_and(|turns| !turns.is_empty())
            || self
                .job_runners
                .lock()
                .is_ok_and(|runners| !runners.is_empty());
        let building = self.atlas_job.lock().is_ok_and(|job| job.is_some());
        [
            (Activity::Translation, translating),
            (Activity::Sync, tracks(Activity::Sync)),
            (Activity::Export, tracks(Activity::Export)),
            (Activity::SourcePackage, building),
        ]
        .into_iter()
        .filter_map(|(activity, running)| running.then_some(activity))
        .collect()
    }

    /// Selects the Git executable once, at application setup.
    pub fn set_git(&self, git: GitExecutable) {
        let _ = self.git.set(git);
    }

    /// Returns the selected Git executable, or `git` from `PATH`.
    pub(crate) fn git(&self) -> GitExecutable {
        self.git
            .get()
            .cloned()
            .unwrap_or_else(GitExecutable::system)
    }

    /// Returns the shared AI provider HTTP client, creating it on first use.
    /// The client is cheap to clone and holds no project state.
    pub(crate) fn ai_client(&self) -> Result<OpenAiCompatibleClient, CommandError> {
        if let Some(client) = self.ai_client.get() {
            return Ok(client.clone());
        }
        let client = OpenAiCompatibleClient::new()?;
        Ok(self.ai_client.get_or_init(|| client).clone())
    }

    /// Returns the web page client, creating it on first use.
    pub(crate) fn web_client(&self) -> Result<aeria_ai::web::WebClient, CommandError> {
        if let Some(client) = self.web_client.get() {
            return Ok(client.clone());
        }
        let client = aeria_ai::web::WebClient::new()
            .map_err(|error| CommandError::new("aiClient", error.to_string()))?;
        Ok(self.web_client.get_or_init(|| client).clone())
    }

    pub(crate) const fn chatgpt_tokens(
        &self,
    ) -> &tauri::async_runtime::Mutex<Vec<(String, AccessToken)>> {
        &self.chatgpt_tokens
    }

    /// Registers the one waiting ChatGPT sign-in, replacing an earlier one.
    pub(crate) fn start_chatgpt_login(
        &self,
        login_id: String,
        spawn: impl FnOnce() -> tauri::async_runtime::JoinHandle<()>,
    ) {
        if let Ok(mut login) = self.chatgpt_login.lock() {
            if let Some((_, previous)) = login.take() {
                previous.abort();
            }
            *login = Some((login_id, spawn()));
        }
    }

    pub(crate) fn finish_chatgpt_login(&self, login_id: &str) {
        if let Ok(mut login) = self.chatgpt_login.lock()
            && login.as_ref().is_some_and(|(id, _)| id == login_id)
        {
            *login = None;
        }
    }

    pub(crate) fn cancel_chatgpt_login(&self, login_id: &str) {
        if let Ok(mut login) = self.chatgpt_login.lock()
            && login.as_ref().is_some_and(|(id, _)| id == login_id)
            && let Some((_, handle)) = login.take()
        {
            handle.abort();
        }
    }

    /// Registers a conversation's running turn. The task is spawned while
    /// the registry is locked, so it cannot finish before it is registered.
    /// Returns `false`, without spawning, when the conversation is busy.
    pub(crate) fn start_angelica_turn(
        &self,
        conversation_id: String,
        spawn: impl FnOnce() -> tauri::async_runtime::JoinHandle<()>,
    ) -> bool {
        let Ok(mut turns) = self.angelica_turns.lock() else {
            return false;
        };
        if turns.iter().any(|(id, _)| *id == conversation_id) {
            return false;
        }
        turns.push((conversation_id, spawn()));
        true
    }

    pub(crate) fn angelica_turn_running(&self, conversation_id: &str) -> bool {
        self.angelica_turns
            .lock()
            .is_ok_and(|turns| turns.iter().any(|(id, _)| id == conversation_id))
    }

    /// Forgets a turn that ended on its own.
    pub(crate) fn finish_angelica_turn(&self, conversation_id: &str) {
        if let Ok(mut turns) = self.angelica_turns.lock() {
            turns.retain(|(id, _)| id != conversation_id);
        }
    }

    /// Stops a running turn. Returns whether one was running.
    pub(crate) fn cancel_angelica_turn(&self, conversation_id: &str) -> bool {
        let Ok(mut turns) = self.angelica_turns.lock() else {
            return false;
        };
        let Some(position) = turns.iter().position(|(id, _)| id == conversation_id) else {
            return false;
        };
        let (_, handle) = turns.remove(position);
        handle.abort();
        true
    }

    /// Returns `true` the first time a job store path is used.
    pub(crate) fn first_job_store_use(&self, path: &std::path::Path) -> bool {
        let Ok(mut stores) = self.job_stores.lock() else {
            return false;
        };
        if stores.iter().any(|known| known == path) {
            return false;
        }
        stores.push(path.to_owned());
        true
    }

    /// Registers a job's runner, spawned while the registry is locked.
    /// Returns `false`, without spawning, when the job already runs.
    pub(crate) fn start_job_runner(
        &self,
        job_id: String,
        workers: Arc<WorkerBoard>,
        spawn: impl FnOnce() -> tauri::async_runtime::JoinHandle<()>,
    ) -> bool {
        let Ok(mut runners) = self.job_runners.lock() else {
            return false;
        };
        if runners.iter().any(|runner| runner.job_id == job_id) {
            return false;
        }
        runners.push(JobRunner {
            job_id,
            handle: spawn(),
            workers,
        });
        true
    }

    /// Forgets a runner that ended on its own.
    pub(crate) fn finish_job_runner(&self, job_id: &str) {
        if let Ok(mut runners) = self.job_runners.lock() {
            runners.retain(|runner| runner.job_id != job_id);
        }
    }

    /// Aborts a job's runner, if it runs.
    pub(crate) fn stop_job_runner(&self, job_id: &str) {
        if let Ok(mut runners) = self.job_runners.lock()
            && let Some(position) = runners.iter().position(|runner| runner.job_id == job_id)
        {
            runners.remove(position).handle.abort();
        }
    }

    /// The workers' activity of a running job.
    pub(crate) fn job_workers(&self, job_id: &str) -> Option<Arc<WorkerBoard>> {
        let runners = self.job_runners.lock().ok()?;
        runners
            .iter()
            .find(|runner| runner.job_id == job_id)
            .map(|runner| Arc::clone(&runner.workers))
    }

    pub(crate) fn search_index(&self, package_id: &str) -> Option<IndexState> {
        let indexes = self.search_indexes.lock().ok()?;
        indexes
            .iter()
            .find(|(id, _)| id == package_id)
            .map(|(_, state)| state.clone())
    }

    pub(crate) fn set_search_index(&self, package_id: &str, state: IndexState) {
        if let Ok(mut indexes) = self.search_indexes.lock() {
            indexes.retain(|(id, _)| id != package_id);
            indexes.push((package_id.to_owned(), state));
        }
    }

    pub(crate) fn forget_search_index(&self, package_id: &str) {
        if let Ok(mut indexes) = self.search_indexes.lock() {
            indexes.retain(|(id, _)| id != package_id);
        }
    }

    /// Marks a package's index as building. Returns `false` when it already
    /// has a state, so only one build starts.
    pub(crate) fn claim_search_build(&self, package_id: &str) -> bool {
        let Ok(mut indexes) = self.search_indexes.lock() else {
            return false;
        };
        if indexes.iter().any(|(id, _)| id == package_id) {
            return false;
        }
        indexes.push((package_id.to_owned(), IndexState::Building));
        true
    }

    pub(crate) fn cached_attribution(
        &self,
        root: &std::path::Path,
        head: &str,
    ) -> Option<Arc<Vec<UnitAttribution>>> {
        let cache = self.attribution.lock().ok()?;
        cache
            .as_ref()
            .filter(|cache| cache.root == root && cache.head == head)
            .map(|cache| Arc::clone(&cache.units))
    }

    pub(crate) fn store_attribution(&self, cache: AttributionCache) {
        if let Ok(mut current) = self.attribution.lock() {
            *current = Some(cache);
        }
    }

    pub(crate) fn lock_project(
        &self,
    ) -> Result<MutexGuard<'_, Option<ProjectSession>>, CommandError> {
        self.project
            .lock()
            .map_err(|_| CommandError::internal_state("desktop project state lock is poisoned"))
    }

    pub(crate) fn lock_proposals(&self) -> Result<MutexGuard<'_, ()>, CommandError> {
        self.proposals
            .lock()
            .map_err(|_| CommandError::internal_state("Angelica proposal lock is poisoned"))
    }

    pub(crate) fn lock_registry(&self) -> Result<MutexGuard<'_, ()>, CommandError> {
        self.registry
            .lock()
            .map_err(|_| CommandError::internal_state("desktop project registry lock is poisoned"))
    }

    pub(crate) fn start_atlas_job(&self) -> Result<StartedAtlasJob, CommandError> {
        let mut active = self.atlas_job.lock().map_err(|_| {
            CommandError::internal_state("desktop Atlas job state lock is poisoned")
        })?;
        if active.is_some() {
            return Err(CommandError::new(
                "atlasBusy",
                "another source-package generation job is already active",
            ));
        }
        let sequence = self.next_atlas_job_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("atlas-{sequence:016x}");
        let (token, cancellation) = CancellationToken::new();
        *active = Some(AtlasJob {
            id: id.clone(),
            cancellation,
            token,
        });
        Ok(StartedAtlasJob { id })
    }

    pub(crate) fn cancel_atlas_job(&self, id: &str) -> Result<(), CommandError> {
        let active = self.atlas_job.lock().map_err(|_| {
            CommandError::internal_state("desktop Atlas job state lock is poisoned")
        })?;
        let job = active.as_ref().filter(|job| job.id == id).ok_or_else(|| {
            CommandError::new("atlasCancelled", "the requested Atlas job is not active")
        })?;
        job.cancellation.cancel();
        Ok(())
    }

    pub(crate) fn atlas_job_token(&self, id: &str) -> Result<CancellationToken, CommandError> {
        let active = self.atlas_job.lock().map_err(|_| {
            CommandError::internal_state("desktop Atlas job state lock is poisoned")
        })?;
        active
            .as_ref()
            .filter(|job| job.id == id)
            .map(|job| job.token.clone())
            .ok_or_else(|| {
                CommandError::new("atlasCancelled", "the requested Atlas job is not active")
            })
    }

    pub(crate) fn finish_atlas_job(&self, id: &str) -> Result<(), CommandError> {
        let mut active = self.atlas_job.lock().map_err(|_| {
            CommandError::internal_state("desktop Atlas job state lock is poisoned")
        })?;
        if active.as_ref().is_some_and(|job| job.id == id) {
            *active = None;
        }
        Ok(())
    }

    /// Runs the final Atlas project-publication boundary while holding the
    /// Atlas job slot. Cancellation requests cannot be accepted between the
    /// final cancellation check and the operation's project publication.
    pub(crate) fn with_atlas_publication<T, F>(
        &self,
        id: &str,
        token: &CancellationToken,
        operation: F,
    ) -> Result<T, CommandError>
    where
        F: FnOnce(&Self) -> Result<T, CommandError>,
    {
        let mut active = self.atlas_job.lock().map_err(|_| {
            CommandError::internal_state("desktop Atlas job state lock is poisoned")
        })?;
        if active.as_ref().is_none_or(|job| job.id != id) {
            return Err(CommandError::new(
                "atlasCancelled",
                "the requested Atlas job is not active",
            ));
        }
        if token.is_cancelled() {
            return Err(CommandError::new(
                "atlasCancelled",
                "Atlas project creation was cancelled",
            ));
        }

        let result = operation(self);
        if result.is_ok() {
            *active = None;
        }
        result
    }
}

impl Default for DesktopState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activities_are_reported_while_their_guards_live() {
        let state = DesktopState::new();
        assert!(state.running_activities().is_empty());
        let sync = state.begin_activity(Activity::Sync);
        {
            let _export = state.begin_activity(Activity::Export);
            let _second_sync = state.begin_activity(Activity::Sync);
            assert_eq!(
                state.running_activities(),
                [Activity::Sync, Activity::Export]
            );
        }
        assert_eq!(state.running_activities(), [Activity::Sync]);
        drop(sync);
        assert!(state.running_activities().is_empty());
        let job = state.start_atlas_job().expect("Atlas job");
        assert_eq!(state.running_activities(), [Activity::SourcePackage]);
        state.finish_atlas_job(&job.id).expect("finish Atlas job");
        assert!(state.running_activities().is_empty());
    }

    #[test]
    fn idle_work_is_refused_while_an_activity_runs() {
        let state = DesktopState::new();
        let export = state.begin_activity(Activity::Export);
        let refused = state.while_idle(|| unreachable!("must not run while exporting"));
        assert_eq!(refused.expect_err("busy").code, "updateBusy");
        drop(export);
        assert_eq!(state.while_idle(|| 7).expect("idle"), 7);
    }

    #[test]
    fn desktop_state_allows_one_atlas_job_and_cancels_by_opaque_id() {
        let state = DesktopState::new();
        let first = state.start_atlas_job().expect("first Atlas job");
        let busy = state
            .start_atlas_job()
            .expect_err("second Atlas job is rejected");
        assert_eq!(busy.code, "atlasBusy");

        state
            .cancel_atlas_job(&first.id)
            .expect("active Atlas job cancels");
        assert!(
            state
                .atlas_job_token(&first.id)
                .expect("active Atlas job token")
                .is_cancelled()
        );
        state.finish_atlas_job(&first.id).expect("finish Atlas job");
        state
            .start_atlas_job()
            .expect("a later Atlas job can start");
    }

    #[test]
    fn atlas_job_token_is_only_available_for_the_active_job() {
        let state = DesktopState::new();
        let started = state.start_atlas_job().expect("Atlas job");

        assert!(state.atlas_job_token(&started.id).is_ok());
        assert_eq!(
            state
                .atlas_job_token("atlas-0000000000000002")
                .expect_err("stale job ID"),
            CommandError::new("atlasCancelled", "the requested Atlas job is not active")
        );
    }
}
