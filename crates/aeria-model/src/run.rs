//! A run: the untranslated strings of chosen files of `po/`, translated in
//! batches in parallel. Its only state is the files: what is left is the
//! strings still untranslated, so a run that stopped continues when it is
//! started again.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aeria_knowledge::Knowledge;
use aeria_po::{Entry, PO_DIR, PoFile, Session, check_translation, is_scene};
use serde::Serialize;
use tokio::task::JoinSet;

use crate::ModelError;
use crate::codex::{Codex, Request, Usage};
use crate::names::{Names, name_sheet_of};
use crate::prompt::{self, Item, Term};

/// Strings of a scene file that are one request; a longer scene is split.
const SCENE_BATCH: usize = 150;
/// Strings per request otherwise.
const BATCH: usize = 100;
/// Most requests in flight.
const CEILING: usize = 16;
/// Translated strings of the same file sent as examples.
const EXAMPLES: usize = 40;
/// Most names and terms sent with one batch.
const NAMES: usize = 80;
const TERMS: usize = 60;
/// Rejected strings a status lists; the count is kept for all.
const LISTED_REJECTIONS: usize = 200;
/// Failures of the service in a row that stop a run.
const FAILURES: u32 = 3;
/// Wait after a failure, times the failures in a row.
const BACKOFF: Duration = Duration::from_secs(20);

/// What a run translates, with which model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Options {
    /// Files and folders relative to `po/`; empty for all of `po/`.
    pub paths: Vec<String>,
    /// Translate fuzzy strings too, with their old translation.
    pub fuzzy: bool,
    pub model: String,
    pub effort: Option<String>,
}

/// A string whose translation failed the checks twice; it stays untranslated.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rejected {
    /// The file, relative to `po/`.
    pub path: String,
    pub context: String,
    pub translation: String,
    pub problems: Vec<String>,
}

/// Why a run stopped.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "reason")]
pub enum Stop {
    /// Every batch was sent.
    Finished,
    Cancelled,
    /// The plan's usage limit; starting again after it resets continues.
    UsageLimit {
        /// Unix seconds, when the service said.
        resets_at: Option<u64>,
    },
    SignInRequired,
    Failed {
        message: String,
    },
}

/// Where a run is.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub running: bool,
    pub files: usize,
    pub batches: usize,
    pub batches_done: usize,
    /// Strings the run set out to translate.
    pub strings: usize,
    pub written: usize,
    pub rejected: usize,
    pub rejections: Vec<Rejected>,
    pub input_tokens: u64,
    pub cached_tokens: u64,
    pub output_tokens: u64,
    /// Requests in flight allowed now.
    pub pace: usize,
    pub stop: Option<Stop>,
    /// The last failure of the service, while the run waits and retries.
    pub message: Option<String>,
    /// Unix milliseconds.
    pub started_at: u64,
}

/// A run in progress: its status and a way to stop it.
#[derive(Debug, Default)]
pub struct Run {
    status: Mutex<Status>,
    cancel: AtomicBool,
    cancelled: tokio::sync::Notify,
}

impl Run {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn update(&self, change: impl FnOnce(&mut Status)) {
        change(&mut self.status.lock().unwrap_or_else(PoisonError::into_inner));
    }

    /// Asks the run to stop; batches in flight are dropped and translated
    /// by the next run.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancelled.notify_waiters();
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Returns once the run is asked to stop, even while every request in
    /// flight is still waiting for the service.
    async fn until_cancelled(&self) {
        loop {
            let notified = self.cancelled.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.cancelled() {
                return;
            }
            notified.await;
        }
    }
}

/// Strings of one file sent in one request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Batch {
    /// The file, relative to `po/`.
    pub path: String,
    /// The `msgctxt` of each string, in file order.
    pub contexts: Vec<String>,
    /// The index of the batch's first and last entry in the file.
    pub first: usize,
    pub last: usize,
}

fn needs_work(entry: &Entry, fuzzy: bool) -> bool {
    if entry.fuzzy {
        fuzzy
    } else {
        entry.translation.is_empty()
    }
}

/// Whether a file is chosen: `paths` name files (with or without `.po`)
/// and folders, relative to `po/`.
fn selected(path: &str, paths: &[String]) -> bool {
    paths.is_empty()
        || paths.iter().any(|chosen| {
            let chosen = chosen.trim_matches('/');
            chosen.is_empty()
                || path == chosen
                || path.strip_suffix(".po") == Some(chosen)
                || path
                    .strip_prefix(chosen)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

/// The batches of a run: the strings of the chosen files that need a
/// translation, in file order.
///
/// # Errors
///
/// Returns a description when `po/` cannot be listed or a file cannot be
/// read.
pub fn plan(root: &std::path::Path, paths: &[String], fuzzy: bool) -> Result<Vec<Batch>, String> {
    let files = aeria_po::list(root).map_err(|error| error.to_string())?;
    let mut batches = Vec::new();
    for path in files.into_iter().filter(|path| selected(path, paths)) {
        let full = root.join(PO_DIR).join(&path);
        let text = std::fs::read_to_string(&full).map_err(|error| format!("po/{path}: {error}"))?;
        let file = PoFile::parse(&text).0;
        let open: Vec<usize> = file
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| needs_work(entry, fuzzy))
            .map(|(index, _)| index)
            .collect();
        let scene = file
            .entries
            .first()
            .and_then(|entry| aeria_po::Identity::parse(&entry.context).ok())
            .is_some_and(|identity| is_scene(&identity.sheet));
        let size = if scene && open.len() <= SCENE_BATCH {
            SCENE_BATCH
        } else {
            BATCH
        };
        for chunk in open.chunks(size) {
            batches.push(Batch {
                path: path.clone(),
                contexts: chunk
                    .iter()
                    .map(|index| file.entries[*index].context.clone())
                    .collect(),
                first: chunk[0],
                last: chunk[chunk.len() - 1],
            });
        }
    }
    Ok(batches)
}

/// What a run shares among its requests.
#[derive(Clone)]
struct Shared {
    session: Arc<Session>,
    codex: Arc<Codex>,
    names: Arc<Names>,
    knowledge: Arc<Knowledge>,
    target: String,
    instructions: String,
    cache_key: String,
    options: Options,
}

/// What one batch did.
struct Done {
    written: usize,
    rejected: Vec<Rejected>,
    usage: Usage,
}

fn add(total: &mut Usage, usage: Usage) {
    total.input += usage.input;
    total.cached += usage.cached;
    total.output += usage.output;
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The strings of a request by the ids it gives them.
type Strings = Vec<(String, Entry)>;

/// The request of a batch and its strings.
struct Built {
    input: String,
    strings: Strings,
}

/// The request of a batch, from the file as it is now: the strings that
/// still need a translation, examples, names, and terms.
fn build(shared: &Shared, batch: &Batch) -> Result<Option<Built>, String> {
    let full = shared.session.root().join(PO_DIR).join(&batch.path);
    let text =
        std::fs::read_to_string(&full).map_err(|error| format!("po/{}: {error}", batch.path))?;
    let file = PoFile::parse(&text).0;
    let wanted: std::collections::HashSet<&str> =
        batch.contexts.iter().map(String::as_str).collect();
    let mut strings: Strings = Vec::new();
    for entry in &file.entries {
        if wanted.contains(entry.context.as_str()) && needs_work(entry, shared.options.fuzzy) {
            strings.push(((strings.len() + 1).to_string(), entry.clone()));
        }
    }
    if strings.is_empty() {
        return Ok(None);
    }
    let mut examples: Vec<(usize, &Entry)> = file
        .entries
        .iter()
        .enumerate()
        .filter(|(index, entry)| {
            !entry.translation.is_empty()
                && !entry.fuzzy
                && !(batch.first..=batch.last).contains(index)
        })
        .map(|(index, entry)| {
            let distance = if index < batch.first {
                batch.first - index
            } else {
                index - batch.last
            };
            (distance, entry)
        })
        .collect();
    examples.sort_by_key(|(distance, _)| *distance);
    let examples: Vec<(String, String)> = examples
        .into_iter()
        .take(EXAMPLES)
        .map(|(_, entry)| (entry.source.clone(), entry.translation.clone()))
        .collect();
    let sources: Vec<&str> = strings
        .iter()
        .map(|(_, entry)| entry.source.as_str())
        .collect();
    let names = shared.names.in_texts(sources.iter().copied(), NAMES);
    let mut terms: Vec<Term> = Vec::new();
    for source in &sources {
        for entry in shared.knowledge.terms_in(source) {
            if terms.len() < TERMS && !terms.iter().any(|term| term.term == entry.term) {
                terms.push(Term {
                    term: entry.term.clone(),
                    translation: entry.translation.clone(),
                    note: entry.note.clone(),
                    never: entry.forbidden.clone(),
                });
            }
        }
    }
    let items: Vec<Item> = strings
        .iter()
        .map(|(id, entry)| Item {
            id: id.clone(),
            source: entry.source.clone(),
            context: entry.extracted.clone(),
            previous: entry
                .fuzzy
                .then(|| {
                    entry
                        .previous
                        .clone()
                        .map(|source| (source, entry.translation.clone()))
                })
                .flatten(),
        })
        .collect();
    let input = prompt::input(
        &format!("{PO_DIR}/{}", batch.path),
        &names,
        &terms,
        &examples,
        &items,
    );
    Ok(Some(Built { input, strings }))
}

fn request(shared: &Shared, input: String) -> Request {
    Request {
        model: shared.options.model.clone(),
        effort: shared.options.effort.clone(),
        instructions: shared.instructions.clone(),
        input,
        cache_key: shared.cache_key.clone(),
    }
}

/// The problems of a translation of the string `id` of a batch.
fn problems(shared: &Shared, strings: &Strings, id: &str, text: &str) -> Vec<String> {
    let Some((_, entry)) = strings.iter().find(|(candidate, _)| candidate == id) else {
        return vec!["not a string of the batch".to_owned()];
    };
    check_translation(
        &shared.knowledge,
        &shared.target,
        &entry.source,
        text,
        &entry.extracted,
    )
    .problems
}

/// Sends the translations that failed the checks back once with their
/// problems; what fails again is rejected and stays untranslated.
async fn settle(
    shared: &Shared,
    batch: &Batch,
    built: &Built,
    answers: &mut HashMap<String, String>,
    usage: &mut Usage,
) -> Result<Vec<Rejected>, ModelError> {
    let failing: Vec<(String, String, Vec<String>)> = answers
        .iter()
        .filter_map(|(id, text)| {
            let found = problems(shared, &built.strings, id, text);
            (!found.is_empty()).then(|| (id.clone(), text.clone(), found))
        })
        .collect();
    if failing.is_empty() {
        return Ok(Vec::new());
    }
    for (id, _, _) in &failing {
        answers.remove(id);
    }
    let retry = shared
        .codex
        .respond(&request(
            shared,
            prompt::retry_input(&built.input, &failing),
        ))
        .await?;
    add(usage, retry.usage);
    let fixed = prompt::parse(&retry.text).unwrap_or_default();
    let mut rejected = Vec::new();
    for (id, first, first_problems) in failing {
        let (translation, found) = match fixed.get(&id) {
            Some(text) => (text.clone(), problems(shared, &built.strings, &id, text)),
            None => (first, first_problems),
        };
        if found.is_empty() {
            answers.insert(id, translation);
        } else if let Some((_, entry)) =
            built.strings.iter().find(|(candidate, _)| *candidate == id)
        {
            rejected.push(Rejected {
                path: batch.path.clone(),
                context: entry.context.clone(),
                translation,
                problems: found,
            });
        }
    }
    Ok(rejected)
}

/// Translates one batch: one request, one more for the translations that
/// fail the checks, and one write of what passes.
async fn translate(shared: Arc<Shared>, batch: Batch) -> Result<Done, ModelError> {
    let built = {
        let shared = Arc::clone(&shared);
        let batch = batch.clone();
        tokio::task::spawn_blocking(move || build(&shared, &batch))
            .await
            .map_err(|error| ModelError::Invalid(error.to_string()))?
            .map_err(ModelError::Invalid)?
    };
    let Some(built) = built else {
        return Ok(Done {
            written: 0,
            rejected: Vec::new(),
            usage: Usage::default(),
        });
    };
    let mut usage = Usage::default();
    let reply = shared
        .codex
        .respond(&request(&shared, built.input.clone()))
        .await?;
    add(&mut usage, reply.usage);
    let mut answers = prompt::parse(&reply.text).map_err(ModelError::Invalid)?;
    let rejected = settle(&shared, &batch, &built, &mut answers, &mut usage).await?;
    let translations: Vec<(String, String)> = built
        .strings
        .iter()
        .filter_map(|(id, entry)| {
            answers
                .get(id)
                .filter(|text| !text.is_empty())
                .map(|text| (entry.context.clone(), text.clone()))
        })
        .collect();
    let written = {
        let session = Arc::clone(&shared.session);
        let path = batch.path.clone();
        let fuzzy = shared.options.fuzzy;
        tokio::task::spawn_blocking(move || session.fill(&path, &translations, fuzzy))
            .await
            .map_err(|error| ModelError::Invalid(error.to_string()))?
            .map_err(|error| ModelError::Invalid(error.to_string()))?
    };
    Ok(Done {
        written: written.len(),
        rejected,
        usage,
    })
}

/// Runs the translation of `options.paths` until every batch was sent, the
/// run is cancelled, or the service stops it. Progress and the reason it
/// stopped are in `run`.
pub async fn run(session: Arc<Session>, codex: Arc<Codex>, options: Options, handle: Arc<Run>) {
    handle.update(|status| {
        *status = Status {
            running: true,
            pace: 1,
            started_at: now_millis(),
            ..Status::default()
        };
    });
    let stop = drive(session, codex, options, &handle).await;
    handle.update(|status| {
        status.running = false;
        status.pace = 0;
        status.stop = Some(stop);
    });
}

/// The batches of a run and what its requests share.
async fn prepare(
    session: Arc<Session>,
    codex: Arc<Codex>,
    options: Options,
) -> Result<(Arc<Shared>, Vec<Batch>), Stop> {
    let prepared = {
        let session = Arc::clone(&session);
        let options = options.clone();
        tokio::task::spawn_blocking(move || {
            let batches = plan(session.root(), &options.paths, options.fuzzy)?;
            Ok::<_, String>((batches, Arc::new(Names::load(session.root()))))
        })
        .await
    };
    let (batches, names) = match prepared {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(message)) => return Err(Stop::Failed { message }),
        Err(error) => {
            return Err(Stop::Failed {
                message: error.to_string(),
            });
        }
    };
    let settings = session.settings();
    let knowledge = session.knowledge();
    let shared = Arc::new(Shared {
        instructions: prompt::instructions(
            &settings.source_language,
            &settings.target_language,
            knowledge.style.as_deref(),
        ),
        target: settings.target_language,
        knowledge,
        names,
        cache_key: format!("aeria-{}", now_millis()),
        session,
        codex,
        options,
    });
    Ok((shared, batches))
}

/// How the loop goes on after a failed batch.
enum Next {
    Continue,
    Wait(Duration),
    Stop(Stop),
}

/// Handles a failed batch: the service's limit and a lost sign-in stop the
/// run; a rate limit, network error, or timeout returns the batch to the
/// queue after a wait (a rate limit also halves the pace), and the third in
/// a row stops the run; anything else leaves the batch's strings for the
/// next run.
fn failed(error: ModelError, failures: &mut u32, pace: &mut usize, handle: &Run) -> Next {
    match error {
        ModelError::UsageLimit { resets_at, .. } => Next::Stop(Stop::UsageLimit { resets_at }),
        ModelError::SignInRequired => Next::Stop(Stop::SignInRequired),
        error @ (ModelError::RateLimited { .. } | ModelError::Network(_) | ModelError::Timeout) => {
            *failures += 1;
            if *failures >= FAILURES {
                return Next::Stop(Stop::Failed {
                    message: error.to_string(),
                });
            }
            let wait = match &error {
                ModelError::RateLimited { retry_after, .. } => {
                    *pace = (*pace / 2).max(2);
                    retry_after.unwrap_or(BACKOFF * *failures)
                }
                _ => BACKOFF * *failures,
            };
            handle.update(|status| {
                status.pace = *pace;
                status.message = Some(error.to_string());
            });
            Next::Wait(wait)
        }
        error => {
            handle.update(|status| {
                status.batches_done += 1;
                status.message = Some(error.to_string());
            });
            Next::Continue
        }
    }
}

/// The batches of a run in the order it sends them: each name sheet's on
/// its own, in the order of [`crate::names::NAME_SHEETS`], and then all
/// others. The names are read again after each name sheet, so later
/// batches use the translations it wrote.
fn phases(batches: Vec<Batch>) -> Vec<(bool, VecDeque<Batch>)> {
    let mut names: BTreeMap<usize, VecDeque<Batch>> = BTreeMap::new();
    let mut rest = VecDeque::new();
    for batch in batches {
        match name_sheet_of(&batch.path) {
            Some(order) => names.entry(order).or_default().push_back(batch),
            None => rest.push_back(batch),
        }
    }
    let mut phases: Vec<(bool, VecDeque<Batch>)> =
        names.into_values().map(|queue| (true, queue)).collect();
    if !rest.is_empty() {
        phases.push((false, rest));
    }
    phases
}

/// The request pace and the failures in a row, kept across phases.
struct Pace {
    now: usize,
    failures: u32,
}

/// Sends the batches of `queue` until all have answered; a stop ends it
/// early.
async fn drain(
    shared: &Arc<Shared>,
    mut queue: VecDeque<Batch>,
    pace: &mut Pace,
    handle: &Run,
) -> Option<Stop> {
    let mut running: JoinSet<(Batch, Result<Done, ModelError>)> = JoinSet::new();
    loop {
        if handle.cancelled() {
            running.abort_all();
            return Some(Stop::Cancelled);
        }
        while running.len() < pace.now
            && let Some(batch) = queue.pop_front()
        {
            let shared = Arc::clone(shared);
            running.spawn(async move {
                let result = translate(shared, batch.clone()).await;
                (batch, result)
            });
        }
        let joined = tokio::select! {
            // Nothing left running means every batch has answered.
            joined = running.join_next() => joined?,
            () = handle.until_cancelled() => {
                running.abort_all();
                return Some(Stop::Cancelled);
            }
        };
        let Ok((batch, result)) = joined else {
            continue;
        };
        match result {
            Ok(done) => {
                pace.failures = 0;
                pace.now = if pace.now == 1 {
                    CEILING
                } else {
                    (pace.now + 1).min(CEILING)
                };
                let now = pace.now;
                handle.update(|status| {
                    status.batches_done += 1;
                    status.written += done.written;
                    status.rejected += done.rejected.len();
                    let room = LISTED_REJECTIONS.saturating_sub(status.rejections.len());
                    status
                        .rejections
                        .extend(done.rejected.into_iter().take(room));
                    status.input_tokens += done.usage.input;
                    status.cached_tokens += done.usage.cached;
                    status.output_tokens += done.usage.output;
                    status.pace = now;
                    status.message = None;
                });
            }
            Err(error) => {
                let retry = matches!(
                    error,
                    ModelError::RateLimited { .. } | ModelError::Network(_) | ModelError::Timeout
                );
                match failed(error, &mut pace.failures, &mut pace.now, handle) {
                    Next::Continue => {}
                    Next::Wait(wait) => {
                        if retry {
                            queue.push_front(batch);
                        }
                        tokio::select! {
                            () = tokio::time::sleep(wait) => {}
                            () = handle.until_cancelled() => {
                                running.abort_all();
                                return Some(Stop::Cancelled);
                            }
                        }
                    }
                    Next::Stop(stop) => {
                        running.abort_all();
                        return Some(stop);
                    }
                }
            }
        }
    }
}

async fn drive(session: Arc<Session>, codex: Arc<Codex>, options: Options, handle: &Run) -> Stop {
    let (mut shared, batches) = match prepare(session, codex, options).await {
        Ok(prepared) => prepared,
        Err(stop) => return stop,
    };
    handle.update(|status| {
        status.batches = batches.len();
        status.strings = batches.iter().map(|batch| batch.contexts.len()).sum();
        let mut files: Vec<&str> = batches.iter().map(|batch| batch.path.as_str()).collect();
        files.dedup();
        status.files = files.len();
    });
    // The first request goes alone and stores the instructions in the
    // service's cache; the others start when it has answered.
    let mut pace = Pace {
        now: 1,
        failures: 0,
    };
    let mut names_changed = false;
    for (names, queue) in phases(batches) {
        if names_changed {
            let root = shared.session.root().to_owned();
            let Ok(names) = tokio::task::spawn_blocking(move || Names::load(&root)).await else {
                return Stop::Failed {
                    message: "the game's names could not be read again".to_owned(),
                };
            };
            shared = Arc::new(Shared {
                names: Arc::new(names),
                ..(*shared).clone()
            });
        }
        if let Some(stop) = drain(&shared, queue, &mut pace, handle).await {
            return stop;
        }
        names_changed = names;
    }
    Stop::Finished
}

/// Counts what a run of `paths` would translate: strings by file, relative
/// to `po/`.
///
/// # Errors
///
/// Returns a description when `po/` cannot be read.
pub fn count(
    root: &std::path::Path,
    paths: &[String],
    fuzzy: bool,
) -> Result<HashMap<String, usize>, String> {
    let mut counts = HashMap::new();
    for batch in plan(root, paths, fuzzy)? {
        *counts.entry(batch.path).or_insert(0) += batch.contexts.len();
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_select_files_and_folders() {
        let paths = vec![
            "Addon".to_owned(),
            "quest/000/A.po".to_owned(),
            "Quest~".to_owned(),
        ];
        assert!(selected("Addon/0.po", &paths));
        assert!(selected("Quest~.po", &paths));
        assert!(selected("quest/000/A.po", &paths));
        assert!(!selected("AddonTransient.po", &paths));
        assert!(!selected("quest/000/B.po", &paths));
        assert!(selected("anything.po", &[]));
    }

    #[test]
    fn name_sheets_go_first_one_at_a_time() {
        let batch = |path: &str| Batch {
            path: path.to_owned(),
            contexts: vec![format!("{path}:1")],
            first: 0,
            last: 0,
        };
        let batches = vec![
            batch("Addon/0.po"),
            batch("Item/0.po"),
            batch("quest/001/X.po"),
            batch("PlaceName.po"),
            batch("Item/1000.po"),
        ];
        let order: Vec<(bool, Vec<String>)> = phases(batches)
            .into_iter()
            .map(|(names, queue)| (names, queue.into_iter().map(|batch| batch.path).collect()))
            .collect();
        assert_eq!(
            order,
            vec![
                (true, vec!["PlaceName.po".to_owned()]),
                (
                    true,
                    vec!["Item/0.po".to_owned(), "Item/1000.po".to_owned()]
                ),
                (
                    false,
                    vec!["Addon/0.po".to_owned(), "quest/001/X.po".to_owned()]
                ),
            ]
        );
    }
}
