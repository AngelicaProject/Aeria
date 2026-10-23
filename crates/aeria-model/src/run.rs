//! A run: the untranslated strings of chosen files of `po/`, translated in
//! batches in parallel. Its only state is the files: what is left is the
//! strings still untranslated, so a run that stopped continues when it is
//! started again.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aeria_knowledge::GlossaryEntry;
use aeria_knowledge::Knowledge;
use aeria_po::length::length_budget;
use aeria_po::{
    EditKind, Entry, EntryEdit, Issue, PO_DIR, PoFile, Session, check_translation, is_scene,
};
use serde::Serialize;
use tokio::task::JoinSet;

use crate::ModelError;
use crate::codex::{Codex, Reply, Request};
use crate::fit;
use crate::prompt::{self, Answer, FileTask, Item, Term};
use crate::report::{Record, Report, RunKind, StringAttempt, StringOutcome, StringRecord};

/// Strings of a scene file that are one request; a longer scene is split.
const SCENE_BATCH: usize = 150;
/// Most files of one request: small files are packed together up to
/// [`SCENE_BATCH`] strings.
const PACK_FILES: usize = 12;
/// Strings per request otherwise.
const BATCH: usize = 100;
/// Most requests in flight.
const CEILING: usize = 16;
/// Translated strings of the same file sent as examples.
const EXAMPLES: usize = 40;
/// Most terms sent with one request: every term a pack of files of up to
/// [`SCENE_BATCH`] strings usually has, the names the project agreed on
/// among them.
const TERMS: usize = 200;
/// Rejected strings a status lists; the count is kept for all.
const LISTED_REJECTIONS: usize = 200;
/// Corrections a status lists; the count of all is `written`.
const LISTED_CORRECTIONS: usize = 1000;
/// Times the translations that fail the checks go back to the model, with
/// what is wrong, before they are rejected.
const RETRIES: usize = 3;
/// The problem of a string the answer did not translate readably.
const MISSING: &str = "the answer has no readable translation of this string: answer it as \
     [\"first words\", \"translation\"], with every quote inside a string escaped as \\\"";
/// The problem of a string the answer of a correction did not answer
/// readably.
const MISSING_CORRECTION: &str = "the answer has no readable entry for this string: answer it \
     as [\"first words\", \"translation\", \"reason\"], the translation unchanged and the reason \
     empty when nothing needs to change, with every quote inside a string escaped as \\\"";
/// Failures of the service in a row that stop a run.
const FAILURES: u32 = 3;
/// Wait after a failure, times the failures in a row.
const BACKOFF: Duration = Duration::from_secs(20);

/// What a run translates, with which model. A report keeps it, so a run
/// that stopped can be continued.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    /// Files and folders relative to `po/`; empty for all of `po/`.
    pub paths: Vec<String>,
    /// Translate fuzzy strings too, with their old translation.
    pub fuzzy: bool,
    /// Only these strings, by `msgctxt`; empty for every string of `paths`.
    pub contexts: Vec<String>,
    pub model: String,
    pub effort: Option<String>,
    /// Correct translations instead of translating untranslated strings.
    pub fix: Option<Fix>,
}

/// What a run that corrects translations asks: it takes the translated
/// strings that are not reviewed, and sends each translation with what is
/// wrong with it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Fix {
    /// Send the issues the checks find in a translation.
    pub issues: bool,
    /// Only the issues of these groups ([`Issue::group`]); empty for every
    /// issue. A problem always goes: a correction must pass the checks.
    pub groups: Vec<String>,
    /// Adapt the translation of a fuzzy string to its changed source; what
    /// passes is written with the fuzzy mark cleared.
    pub adapt: bool,
    /// What a translator asks of every translation of the run.
    pub request: Option<String>,
    /// The name of the project's skill `request` is the text of (see
    /// `aeria_knowledge::skills`), to name the run by.
    pub skill: Option<String>,
    /// Sheets and folders left out of the run's paths.
    pub exclude: Vec<String>,
    /// Regular expressions the strings the run takes match.
    pub filter: Filter,
    /// Bring the translations written with an older version of these
    /// terms, by headword, or of any term when empty, to the terms as they
    /// are (see [`Entry::outdated_terms`]); `None` for none.
    pub terms: Option<Vec<String>>,
}

impl Fix {
    /// Whether the answer of a request may leave out the strings it does not
    /// change. An adaptation answered unchanged clears the fuzzy mark, and a
    /// correction to terms answered unchanged records that the translation
    /// follows them, so those runs answer every string.
    #[must_use]
    pub fn omits_unchanged(&self) -> bool {
        !self.adapt && self.terms.is_none()
    }
}

/// Regular expressions, each ignoring case, that the strings a correction
/// takes match, all those given: of its source and translation as macro
/// text, and of one of its lines in the other client languages.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filter {
    pub source: Option<String>,
    pub translation: Option<String>,
    pub other: Option<String>,
}

/// A [`Filter`] with its expressions compiled.
#[derive(Clone, Debug, Default)]
pub struct Matcher {
    source: Option<regex::Regex>,
    translation: Option<regex::Regex>,
    other: Option<regex::Regex>,
}

impl Matcher {
    /// Compiles `filter`.
    ///
    /// # Errors
    ///
    /// Returns the expression that does not compile, with why.
    pub fn new(filter: &Filter) -> Result<Self, String> {
        let compile = |field: &str, pattern: &Option<String>| {
            pattern
                .as_deref()
                .map(|pattern| {
                    regex::RegexBuilder::new(pattern)
                        .case_insensitive(true)
                        .build()
                        .map_err(|error| format!("{field}: {error}"))
                })
                .transpose()
        };
        Ok(Self {
            source: compile("source", &filter.source)?,
            translation: compile("translation", &filter.translation)?,
            other: compile("other", &filter.other)?,
        })
    }

    /// Whether `entry` matches every expression given.
    #[must_use]
    pub fn matches(&self, entry: &Entry) -> bool {
        self.source
            .as_ref()
            .is_none_or(|regex| regex.is_match(&entry.source))
            && self
                .translation
                .as_ref()
                .is_none_or(|regex| regex.is_match(&entry.translation))
            && self.other.as_ref().is_none_or(|regex| {
                entry.extracted.iter().any(|line| {
                    ["ja: ", "de: ", "fr: "]
                        .iter()
                        .find_map(|prefix| line.strip_prefix(prefix))
                        .is_some_and(|text| regex.is_match(text))
                })
            })
    }
}

/// Whether `path`, relative to `po/`, is one of `exclude`; none when it is
/// empty.
fn excluded(path: &str, exclude: &[String]) -> bool {
    !exclude.is_empty() && selected(path, exclude)
}

/// The issues of a translation a correction is asked to fix: what the
/// checks find, of the chosen groups, and every problem. A term exception
/// that names no term of the source is a person's to remove, not a
/// translation's to change.
#[must_use]
pub fn fix_issues(fix: &Fix, knowledge: &Knowledge, target: &str, entry: &Entry) -> Vec<Issue> {
    if entry.translation.is_empty() {
        return Vec::new();
    }
    check_translation(knowledge, target, entry, &entry.translation)
        .issues
        .into_iter()
        .filter(|issue| {
            issue.is_problem()
                || (fix.issues
                    && !matches!(issue, Issue::StaleTermException(_))
                    && (fix.groups.is_empty() || fix.groups.iter().any(|group| issue.is_of(group))))
        })
        .collect()
}

/// The terms of `fix` a translation was written with an older version of,
/// or without (see [`Entry::outdated_terms`]).
#[must_use]
pub fn fix_terms<'a>(fix: &Fix, knowledge: &'a Knowledge, entry: &Entry) -> Vec<&'a GlossaryEntry> {
    let Some(wanted) = &fix.terms else {
        return Vec::new();
    };
    entry
        .outdated_terms(&knowledge.terms)
        .into_iter()
        .filter(|term| {
            wanted.is_empty()
                || wanted
                    .iter()
                    .any(|name| name.to_lowercase() == term.term.to_lowercase())
        })
        .collect()
}

/// Whether a correction takes a string: a translation that is not
/// reviewed, with an issue to fix or a term to follow, or every one with a
/// translator's request.
fn needs_fix(fix: &Fix, knowledge: &Knowledge, target: &str, entry: &Entry) -> bool {
    !entry.is_reviewed()
        && !entry.translation.is_empty()
        && (fix.request.is_some()
            || (fix.adapt && entry.fuzzy)
            || !fix_issues(fix, knowledge, target, entry).is_empty()
            || !fix_terms(fix, knowledge, entry).is_empty())
}

/// What a correction of `entry` is asked to fix, as the model is told: the
/// issues of [`fix_issues`], and the terms of [`fix_terms`] to follow.
fn fix_items(fix: &Fix, knowledge: &Knowledge, target: &str, entry: &Entry) -> Vec<String> {
    let mut items: Vec<String> = fix_issues(fix, knowledge, target, entry)
        .iter()
        .map(ToString::to_string)
        .collect();
    let terms: Vec<String> = fix_terms(fix, knowledge, entry)
        .iter()
        .map(|term| format!("{:?}", term.term))
        .collect();
    if !terms.is_empty() {
        items.push(format!(
            "the terms {} may have changed since this translation was written: make it \
             follow them as `terms` give them now, and change nothing else",
            terms.join(", ")
        ));
    }
    items
}

/// A translation a correction changed, with why, as the model says.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Corrected {
    /// The file, relative to `po/`.
    pub path: String,
    pub context: String,
    pub before: String,
    pub after: String,
    pub reason: Option<String>,
}

/// A string whose translation still failed the checks after the retries; it
/// stays as it was.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rejected {
    /// The file, relative to `po/`.
    pub path: String,
    pub context: String,
    /// The model's last translation, which was not written.
    pub translation: String,
    /// Every problem in English, as the model was told.
    pub problems: Vec<String>,
    /// The issues the translation checks found among `problems`, as data;
    /// the rest (another string's answer, a label too long) has none.
    #[serde(skip)]
    pub issues: Vec<aeria_po::Issue>,
}

/// Why a run stopped.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "reason"
)]
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

/// A request the service is answering now.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Active {
    #[serde(skip)]
    id: u64,
    /// The request's first file, relative to `po/`.
    pub path: String,
    pub files: usize,
    pub strings: usize,
    /// 0 for the translation; 1 and up for the retries of the translations
    /// that failed the checks.
    pub retry: usize,
    /// Unix milliseconds.
    pub started_at: u64,
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
    /// Translations a correction answered unchanged.
    pub unchanged: usize,
    pub rejected: usize,
    /// The run corrects translations rather than translating.
    pub fixing: bool,
    /// The first rejected strings; an interface sends them with what it
    /// knows of them.
    #[serde(skip_serializing)]
    pub rejections: Vec<Rejected>,
    /// The first translations a correction wrote, with why each changed.
    #[serde(skip_serializing)]
    pub corrections: Vec<Corrected>,
    pub input_tokens: u64,
    pub cached_tokens: u64,
    pub output_tokens: u64,
    /// Requests in flight allowed now.
    pub pace: usize,
    /// The requests the service is answering now, oldest first. Tokens
    /// count as each answers, so a long request shows here until then.
    pub active: Vec<Active>,
    pub stop: Option<Stop>,
    /// The last failure of the service, while the run waits and retries.
    pub message: Option<String>,
    /// While the run waits for the plan's usage limit to reset: when it
    /// asks again, in Unix seconds.
    pub waiting_until: Option<u64>,
    /// Unix milliseconds.
    pub started_at: u64,
}

/// Where a run writes its report, when it has one.
#[derive(Default)]
struct Sink(Option<Box<dyn Report>>);

impl std::fmt::Debug for Sink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0.is_some() {
            "Sink(..)"
        } else {
            "Sink(None)"
        })
    }
}

/// A run in progress: its status and a way to stop it.
#[derive(Debug, Default)]
pub struct Run {
    status: Mutex<Status>,
    cancel: AtomicBool,
    cancelled: tokio::sync::Notify,
    /// The journal; dropped when it cannot be written.
    log: Mutex<Option<std::fs::File>>,
    /// The report; dropped when it cannot be written.
    report: Mutex<Sink>,
    /// What this run continues: the report of the run it continues, and
    /// the strings, by `msgctxt`, that run and the ones it continued settled.
    continues: Option<(String, HashSet<String>)>,
    requests: AtomicU64,
}

impl Run {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A run that writes its journal to `file`: what it set out to
    /// translate, each request with its file, time, retries, and tokens,
    /// each failure, and why it stopped. The texts sent and received are not
    /// written.
    #[must_use]
    pub fn with_log(file: std::fs::File) -> Self {
        Self {
            log: Mutex::new(Some(file)),
            ..Self::default()
        }
    }

    /// The run that also writes its report to `report`: what became of
    /// every string it asked (see [`crate::report`]).
    #[must_use]
    pub fn with_report(self, report: Box<dyn Report>) -> Self {
        Self {
            report: Mutex::new(Sink(Some(report))),
            ..self
        }
    }

    /// The run that continues the run of the report `report`: the same
    /// options, but for the strings of `settled`, which the runs before
    /// settled.
    #[must_use]
    pub fn continuing(self, report: String, settled: HashSet<String>) -> Self {
        Self {
            continues: Some((report, settled)),
            ..self
        }
    }

    /// Whether the strings planned leave out `context`: one a run this
    /// continues settled.
    fn settled(&self, context: &str) -> bool {
        self.continues
            .as_ref()
            .is_some_and(|(_, settled)| settled.contains(context))
    }

    /// Writes a record of the report.
    fn report(&self, record: &Record) {
        let mut sink = self.report.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(report) = sink.0.as_mut()
            && let Err(error) = report.write(record)
        {
            // A report that cannot be written does not stop the run.
            sink.0 = None;
            drop(sink);
            self.log(format_args!("report not written: {error}"));
        }
    }

    /// Writes a line of the journal, with the seconds since the run started.
    fn log(&self, line: std::fmt::Arguments<'_>) {
        let started = self.status().started_at;
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(file) = log.as_mut() {
            let elapsed = now_millis().saturating_sub(started);
            let written = writeln!(
                file,
                "{:>6}.{}s {line}",
                elapsed / 1000,
                elapsed % 1000 / 100
            );
            // A journal that cannot be written does not stop the run.
            if written.is_err() {
                *log = None;
            }
        }
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
    if entry.is_reviewed() {
        // A person reviewed it: only a person changes it.
        false
    } else if entry.fuzzy {
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
/// translation, in file order; with `contexts`, only those strings.
///
/// # Errors
///
/// Returns a description when `po/` cannot be listed or a file cannot be
/// read.
pub fn plan(
    root: &std::path::Path,
    paths: &[String],
    fuzzy: bool,
    contexts: &[String],
) -> Result<Vec<Batch>, String> {
    plan_of(root, paths, &[], contexts, &|entry| {
        needs_work(entry, fuzzy)
    })
}

/// The batches of the strings of the chosen files that `wanted` takes.
fn plan_of(
    root: &std::path::Path,
    paths: &[String],
    exclude: &[String],
    contexts: &[String],
    wanted: &dyn Fn(&Entry) -> bool,
) -> Result<Vec<Batch>, String> {
    let only: std::collections::HashSet<&str> = contexts.iter().map(String::as_str).collect();
    let files = aeria_po::list(root).map_err(|error| error.to_string())?;
    let mut batches = Vec::new();
    for path in files
        .into_iter()
        .filter(|path| selected(path, paths) && !excluded(path, exclude))
    {
        let full = root.join(PO_DIR).join(&path);
        let text = std::fs::read_to_string(&full).map_err(|error| format!("po/{path}: {error}"))?;
        let file = PoFile::parse(&text).0;
        let open: Vec<usize> = file
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                wanted(entry) && (only.is_empty() || only.contains(entry.context.as_str()))
            })
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
    knowledge: Arc<Knowledge>,
    target: String,
    instructions: String,
    cache_key: String,
    options: Options,
    run: Arc<Run>,
    /// The filter of a correction, compiled.
    matcher: Matcher,
}

impl Shared {
    /// Whether the run takes a string, as its file is now.
    fn wants(&self, entry: &Entry) -> bool {
        match &self.options.fix {
            Some(fix) => {
                needs_fix(fix, &self.knowledge, &self.target, entry) && self.matcher.matches(entry)
            }
            None => needs_work(entry, self.options.fuzzy),
        }
    }
}

/// What one batch did.
struct Done {
    written: usize,
    unchanged: usize,
    rejected: Vec<Rejected>,
    corrected: Vec<Corrected>,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The strings of a request by the ids it gives them, with their file.
type Strings = Vec<(String, String, Entry)>;

/// The batches one request translates: one batch of a file split into
/// several, or whole small files packed together.
type Pack = Vec<Batch>;

/// The request of a batch and its strings.
struct Built {
    input: String,
    strings: Strings,
}

/// A string's context for the model: the comments of its entry, with what
/// its macros do explained again from the catalog. The `macro:` comments say
/// what the catalog knew when the file was made, so a macro the catalog has
/// learned since reaches the model at once, before the next game update
/// rewrites the comments.
fn context_of(entry: &Entry) -> Vec<String> {
    let Ok(constructs) = aeria_se::constructs(&entry.source) else {
        return entry.extracted.clone();
    };
    entry
        .extracted
        .iter()
        .filter(|line| !line.starts_with("macro: "))
        .cloned()
        .chain(
            constructs
                .iter()
                .map(|construct| format!("macro: {}", construct.legend())),
        )
        .collect()
}

/// One file of a request, from the file as it is now: the strings of
/// `batch` that still need a translation, numbered after those already in
/// `strings`, with examples and what the file is. `None` when nothing is
/// left to translate.
fn file_task(
    shared: &Shared,
    batch: &Batch,
    examples_wanted: usize,
    strings: &mut Strings,
) -> Result<Option<FileTask>, String> {
    let full = shared.session.root().join(PO_DIR).join(&batch.path);
    let text =
        std::fs::read_to_string(&full).map_err(|error| format!("po/{}: {error}", batch.path))?;
    let file = PoFile::parse(&text).0;
    let wanted: std::collections::HashSet<&str> =
        batch.contexts.iter().map(String::as_str).collect();
    let start = strings.len();
    for entry in &file.entries {
        if wanted.contains(entry.context.as_str()) && shared.wants(entry) {
            let id = (strings.len() + 1).to_string();
            strings.push((id, batch.path.clone(), entry.clone()));
        }
    }
    let mine = &strings[start..];
    if mine.is_empty() {
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
    // Translations a person reviewed come first: the model continues their
    // wording rather than its own.
    examples.sort_by_key(|(distance, entry)| (!entry.is_reviewed(), *distance));
    let examples: Vec<(String, String)> = examples
        .into_iter()
        .take(examples_wanted)
        .map(|(_, entry)| (entry.source.clone(), entry.translation.clone()))
        .collect();
    // What the file is, from its header: `quest/000/X — «Title» · in play order`.
    let about = file.header.comments.first().cloned().unwrap_or_default();
    let items: Vec<Item> = mine
        .iter()
        .map(|(id, _, entry)| Item {
            id: id.clone(),
            source: entry.source.clone(),
            context: context_of(entry),
            previous: entry
                .fuzzy
                .then(|| {
                    entry
                        .previous
                        .clone()
                        .map(|source| (source, entry.translation.clone()))
                })
                .flatten(),
            max_length: length_budget(entry),
            term_exceptions: entry.term_exceptions.clone(),
            translation: shared
                .options
                .fix
                .is_some()
                .then(|| entry.translation.clone()),
            fix: shared.options.fix.as_ref().map_or_else(Vec::new, |fix| {
                fix_items(fix, &shared.knowledge, &shared.target, entry)
            }),
        })
        .collect();
    let task = FileTask {
        file: format!("{PO_DIR}/{}", batch.path),
        about,
        examples,
        items,
    };
    Ok(Some(task))
}

/// The request of a pack: each file's part, and the terms that occur in
/// any of them.
fn build(shared: &Shared, pack: &[Batch]) -> Result<Option<Built>, String> {
    let examples_wanted = (EXAMPLES / pack.len().max(1)).max(5);
    let mut strings: Strings = Vec::new();
    let mut files = Vec::new();
    for batch in pack {
        if let Some(task) = file_task(shared, batch, examples_wanted, &mut strings)? {
            files.push(task);
        }
    }
    if strings.is_empty() {
        return Ok(None);
    }
    // A term goes with the batch when it applies to one of its strings: a
    // string's term exceptions keep its term out.
    let mut terms: Vec<Term> = Vec::new();
    for (_, _, string) in &strings {
        for entry in shared
            .knowledge
            .terms
            .matches_except(&string.source, &string.term_exceptions)
        {
            // Each meaning of a name goes once.
            let known = terms.iter().any(|term| {
                term.term == entry.term
                    && term.translation == entry.translation
                    && term.category == entry.category
            });
            if terms.len() < TERMS && !known {
                terms.push(Term {
                    term: entry.term.clone(),
                    translation: entry.translation.clone(),
                    forms: entry.forms.clone(),
                    note: entry.full_note(),
                    category: entry.category.clone(),
                });
            }
        }
    }
    let input = prompt::input(&files, &terms);
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

/// The problems of an answer for the string `id` of a batch: that it
/// belongs to another string, what the translation checks find, and a name
/// longer than the game shows.
fn problems(shared: &Shared, strings: &Strings, id: &str, answer: &Answer) -> Vec<String> {
    let Some((_, _, entry)) = strings.iter().find(|(candidate, _, _)| candidate == id) else {
        return vec!["not a string of the batch".to_owned()];
    };
    match &answer.start {
        None => {
            return vec![
                "the answer does not begin with the first words of this string's source".to_owned(),
            ];
        }
        Some(start) if !fit::matches_start(start, &entry.source) => {
            return vec![format!(
                "the answer begins with \"{start}\", which is not the start of this string's \
                 source: its translation belongs to another string; translate this string"
            )];
        }
        Some(_) => {}
    }
    if let Some(debris) = answer_debris(&entry.source, &answer.text) {
        return vec![debris];
    }
    machine_problems(shared, entry, &answer.text)
}

/// What refuses a machine translation or correction of `entry`: the
/// problems of the checks, and a name longer than the game shows, which is
/// only advice for a person.
fn machine_problems(shared: &Shared, entry: &Entry, text: &str) -> Vec<String> {
    check_translation(&shared.knowledge, &shared.target, entry, text)
        .issues
        .into_iter()
        .filter_map(|issue| match issue {
            Issue::NameTooLong { .. } => Some(format!(
                "{issue}: shorten it, with the usual abbreviations when needed"
            )),
            _ if issue.is_problem() => Some(issue.to_string()),
            _ => None,
        })
        .collect()
}

/// The separators of the answer's JSON (`", "`, `","`, `", [`) inside a
/// translation whose source has none: what a reading of a broken answer
/// left of the elements after the translation.
fn answer_debris(source: &str, text: &str) -> Option<String> {
    let compact = |text: &str| -> String { text.chars().filter(|c| !c.is_whitespace()).collect() };
    let (source, text) = (compact(source), compact(text));
    ["\",\"", "\",["]
        .iter()
        .any(|separator| text.contains(separator) && !source.contains(separator))
        .then(|| {
            "the translation holds separators of the answer's JSON: answer the translation \
             alone as its own string, and the reason and terms as their own elements"
                .to_owned()
        })
}

/// A request listed as in flight until it answers, fails, or is dropped
/// with its task.
struct Listed<'a> {
    run: &'a Run,
    id: u64,
}

impl Drop for Listed<'_> {
    fn drop(&mut self) {
        self.run
            .update(|status| status.active.retain(|active| active.id != self.id));
    }
}

/// Sends one request of a batch: the run lists it while the service
/// answers, counts its tokens as soon as it has, and journals it.
async fn ask(
    shared: &Shared,
    built: &Built,
    retry: usize,
    strings: usize,
    input: String,
) -> Result<Reply, ModelError> {
    let mut paths: Vec<&str> = built
        .strings
        .iter()
        .map(|(_, path, _)| path.as_str())
        .collect();
    paths.dedup();
    let run = &*shared.run;
    let id = run.requests.fetch_add(1, Ordering::Relaxed) + 1;
    let active = Active {
        id,
        path: paths.first().copied().unwrap_or_default().to_owned(),
        files: paths.len(),
        strings,
        retry,
        started_at: now_millis(),
    };
    run.log(format_args!(
        "request {id}: {} ({} files, {strings} strings){}",
        active.path,
        active.files,
        if retry > 0 {
            format!(", retry {retry} of {RETRIES}")
        } else {
            String::new()
        }
    ));
    run.update(|status| status.active.push(active));
    let listed = Listed { run, id };
    let started = std::time::Instant::now();
    let reply = shared.codex.respond(&request(shared, input)).await;
    let seconds = started.elapsed().as_secs();
    match &reply {
        Ok(reply) => {
            run.update(|status| {
                status.input_tokens += reply.usage.input;
                status.cached_tokens += reply.usage.cached;
                status.output_tokens += reply.usage.output;
            });
            run.log(format_args!(
                "request {id}: answered in {seconds} s, {} tokens in ({} cached), {} out{}",
                reply.usage.input,
                reply.usage.cached,
                reply.usage.output,
                if reply.incomplete { ", incomplete" } else { "" }
            ));
        }
        Err(error) => run.log(format_args!(
            "request {id}: failed after {seconds} s: {error}"
        )),
    }
    drop(listed);
    reply
}

/// Kinds of problems a journal line names; the rest are counted.
const JOURNALED_PROBLEMS: usize = 4;

/// Journals how many of `asked` strings fail the checks and their most
/// frequent problems. A problem is named up to its first quote or colon, so
/// the journal holds no translation's text.
fn journal_failing(shared: &Shared, asked: usize, failing: &[(String, Answer, Vec<String>)]) {
    if failing.is_empty() {
        shared
            .run
            .log(format_args!("checks: all {asked} strings pass"));
        return;
    }
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for (_, _, problems) in failing {
        for problem in problems {
            let kind = problem
                .split(['"', ':', '«'])
                .next()
                .unwrap_or_default()
                .trim();
            let kind: String = kind.chars().take(100).collect();
            *kinds.entry(kind).or_default() += 1;
        }
    }
    let mut kinds: Vec<(String, usize)> = kinds.into_iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let named: Vec<String> = kinds
        .iter()
        .take(JOURNALED_PROBLEMS)
        .map(|(kind, count)| format!("{count}× {kind}"))
        .collect();
    let others = kinds.len().saturating_sub(JOURNALED_PROBLEMS);
    shared.run.log(format_args!(
        "checks: {} of {asked} strings fail: {}{}",
        failing.len(),
        named.join("; "),
        if others > 0 {
            format!("; and {others} other kinds")
        } else {
            String::new()
        }
    ));
}

/// Whether a problem told to the model is that its answer had no readable
/// entry for the string.
#[must_use]
pub fn is_missing(problem: &str) -> bool {
    problem == MISSING || problem == MISSING_CORRECTION
}

/// Whether a problem told to the model is `issue`: its message, or its
/// message with what to do about it after a colon.
#[must_use]
pub fn names(problem: &str, issue: &Issue) -> bool {
    let message = issue.to_string();
    problem
        .strip_prefix(message.as_str())
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(": "))
}

/// Each answer for each string of a request, by the string's id.
type Attempts = HashMap<String, Vec<StringAttempt>>;

/// Keeps an answer for the string `id` among its attempts, with the issues
/// of the checks among its problems as data.
fn note(
    attempts: &mut Attempts,
    shared: &Shared,
    strings: &Strings,
    id: &str,
    answer: &Answer,
    problems: &[String],
) {
    let issues = strings
        .iter()
        .find(|(candidate, _, _)| candidate == id)
        .filter(|_| !problems.is_empty() && !answer.text.is_empty())
        .map(|(_, _, entry)| {
            check_translation(&shared.knowledge, &shared.target, entry, &answer.text)
                .issues
                .into_iter()
                .filter(|issue| problems.iter().any(|problem| names(problem, issue)))
                .collect()
        })
        .unwrap_or_default();
    attempts
        .entry(id.to_owned())
        .or_default()
        .push(StringAttempt {
            translation: answer.text.clone(),
            problems: problems.to_vec(),
            issues,
        });
}

/// What became of a string, as its report record says.
struct Fate {
    outcome: StringOutcome,
    after: Option<String>,
    reason: Option<String>,
}

impl Fate {
    fn bare(outcome: StringOutcome) -> Self {
        Self {
            outcome,
            after: None,
            reason: None,
        }
    }
}

/// Reports what became of each string of a request, by `msgctxt`; the
/// refused ones are those of `rejected`.
fn report_strings(
    shared: &Shared,
    built: &Built,
    mut attempts: Attempts,
    rejected: &[Rejected],
    mut fates: HashMap<String, Fate>,
) {
    for found in rejected {
        fates.insert(found.context.clone(), Fate::bare(StringOutcome::Refused));
    }
    for (id, path, entry) in &built.strings {
        let Some(fate) = fates.remove(&entry.context) else {
            continue;
        };
        shared.run.report(&Record::String(StringRecord {
            path: path.clone(),
            context: entry.context.clone(),
            source: entry.source.clone(),
            before: entry.translation.clone(),
            attempts: attempts.remove(id).unwrap_or_default(),
            outcome: fate.outcome,
            after: fate.after,
            reason: fate.reason,
        }));
    }
}

/// Whether the strings an answer left out are unchanged: a correction that
/// may leave them out, in an answer read whole and not cut short.
fn leaves_out_unchanged(shared: &Shared, whole: bool, reply: &crate::codex::Reply) -> bool {
    whole
        && !reply.incomplete
        && shared
            .options
            .fix
            .as_ref()
            .is_some_and(Fix::omits_unchanged)
}

/// Sends the translations that fail the checks back with their problems,
/// up to [`RETRIES`] times; what still fails is rejected and stays as it was.
/// Every answer is kept in `attempts`.
async fn settle(
    shared: &Shared,
    built: &Built,
    answers: &mut HashMap<String, Answer>,
    attempts: &mut Attempts,
    omitted: bool,
    untouched: &mut HashSet<String>,
) -> Result<Vec<Rejected>, ModelError> {
    let mut failing: Vec<(String, Answer, Vec<String>)> = Vec::new();
    for (id, answer) in answers.iter() {
        let found = problems(shared, &built.strings, id, answer);
        note(attempts, shared, &built.strings, id, answer, &found);
        if !found.is_empty() {
            failing.push((id.clone(), answer.clone(), found));
        }
    }
    // A string the answer has no translation of, or whose entry could not be
    // read, is asked for again like a failing one, and its report says so;
    // with `omitted`, a correction read whole left it out as unchanged.
    let missing = if shared.options.fix.is_some() {
        MISSING_CORRECTION
    } else {
        MISSING
    };
    for (id, _, _) in &built.strings {
        if !answers.contains_key(id) {
            if omitted {
                untouched.insert(id.clone());
                continue;
            }
            let answer = Answer::bare(None, String::new());
            let found = vec![missing.to_owned()];
            note(attempts, shared, &built.strings, id, &answer, &found);
            failing.push((id.clone(), answer, found));
        }
    }
    for (id, _, _) in &failing {
        answers.remove(id);
    }
    journal_failing(shared, built.strings.len(), &failing);
    for retry in 1..=RETRIES {
        if failing.is_empty() {
            break;
        }
        let sent: Vec<(String, String, Vec<String>)> = failing
            .iter()
            .map(|(id, answer, found)| (id.clone(), answer.text.clone(), found.clone()))
            .collect();
        let reply = ask(
            shared,
            built,
            retry,
            sent.len(),
            prompt::retry_input(&built.input, &sent),
        )
        .await?;
        let (fixed, whole) = prompt::read_answer(&reply.text, shared.options.fix.is_some());
        // As for the first answer: a retry read whole that leaves out a
        // string asked only for being left out leaves it unchanged.
        let omits = leaves_out_unchanged(shared, whole, &reply);
        let mut still = Vec::new();
        for (id, last, asked) in failing {
            if omits && !fixed.contains_key(&id) && asked.iter().all(|problem| is_missing(problem))
            {
                untouched.insert(id);
                continue;
            }
            // A string this answer left out keeps its last translation and
            // is noted as missing from it.
            let (answer, found) = if let Some(answer) = fixed.get(&id) {
                let found = problems(shared, &built.strings, &id, answer);
                note(attempts, shared, &built.strings, &id, answer, &found);
                (answer.clone(), found)
            } else {
                let found = vec![missing.to_owned()];
                let empty = Answer::bare(None, String::new());
                note(attempts, shared, &built.strings, &id, &empty, &found);
                (last, found)
            };
            if found.is_empty() {
                answers.insert(id, answer);
            } else {
                still.push((id, answer, found));
            }
        }
        failing = still;
        journal_failing(shared, sent.len(), &failing);
    }
    Ok(failing
        .into_iter()
        .filter_map(|(id, answer, found)| {
            let (_, path, entry) = built
                .strings
                .iter()
                .find(|(candidate, _, _)| *candidate == id)?;
            let issues = check_translation(&shared.knowledge, &shared.target, entry, &answer.text)
                .issues
                .into_iter()
                .filter(|issue| found.iter().any(|problem| names(problem, issue)))
                .collect();
            Some(Rejected {
                path: path.clone(),
                context: entry.context.clone(),
                translation: answer.text,
                problems: found,
                issues,
            })
        })
        .collect())
}

/// Translates one pack: one request, up to [`RETRIES`] more for the translations that
/// fail the checks, and one write per file of what passes.
async fn translate(shared: Arc<Shared>, pack: Pack) -> Result<Done, ModelError> {
    let built = {
        let shared = Arc::clone(&shared);
        tokio::task::spawn_blocking(move || build(&shared, &pack))
            .await
            .map_err(|error| ModelError::Invalid(error.to_string()))?
            .map_err(ModelError::Invalid)?
    };
    let Some(built) = built else {
        return Ok(Done {
            written: 0,
            unchanged: 0,
            rejected: Vec::new(),
            corrected: Vec::new(),
        });
    };
    let reply = ask(&shared, &built, 0, built.strings.len(), built.input.clone()).await?;
    let (mut answers, whole) = prompt::read_answer(&reply.text, shared.options.fix.is_some());
    // A correction that may leave out what it does not change leaves it out
    // only in an answer read whole and not cut short.
    let omitted = leaves_out_unchanged(&shared, whole, &reply);
    let mut attempts = Attempts::new();
    let mut untouched = HashSet::new();
    let rejected = settle(
        &shared,
        &built,
        &mut answers,
        &mut attempts,
        omitted,
        &mut untouched,
    )
    .await?;
    if shared.options.fix.is_some() {
        return correct(&shared, &built, &answers, &untouched, rejected, attempts).await;
    }
    let mut by_file: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for (id, path, entry) in &built.strings {
        if let Some(answer) = answers.get(id).filter(|answer| !answer.text.is_empty()) {
            by_file
                .entry(path.clone())
                .or_default()
                .push((entry.context.clone(), answer.text.clone()));
        }
    }
    let written = {
        let session = Arc::clone(&shared.session);
        let fuzzy = shared.options.fuzzy;
        let knowledge = Arc::clone(&shared.knowledge);
        tokio::task::spawn_blocking(move || {
            by_file
                .iter()
                .try_fold(HashSet::new(), |mut written, (path, translations)| {
                    written.extend(session.fill(path, translations, fuzzy, &knowledge.terms)?);
                    Ok::<_, aeria_po::EditError>(written)
                })
        })
        .await
        .map_err(|error| ModelError::Invalid(error.to_string()))?
        .map_err(|error| ModelError::Invalid(error.to_string()))?
    };
    let fates = built
        .strings
        .iter()
        .filter_map(|(id, _, entry)| {
            let answer = answers.get(id).filter(|answer| !answer.text.is_empty())?;
            let fate = if written.contains(&entry.context) {
                Fate {
                    after: Some(answer.text.clone()),
                    ..Fate::bare(StringOutcome::Written)
                }
            } else {
                Fate::bare(StringOutcome::Stale)
            };
            Some((entry.context.clone(), fate))
        })
        .collect();
    report_strings(&shared, &built, attempts, &rejected, fates);
    Ok(Done {
        written: written.len(),
        unchanged: 0,
        rejected,
        corrected: Vec::new(),
    })
}

/// What a correction that passed the checks writes to `entry`: its
/// adaptation, or its correction. A correction answered unchanged is
/// written too: it records that the translation follows the terms as they
/// are (see [`Entry::record_terms`]).
fn correction_edit(fix: &Fix, entry: &Entry, answer: &Answer) -> EditKind {
    // An adapted translation answered unchanged still fits the new source:
    // it is written to clear the fuzzy mark.
    if fix.adapt && entry.fuzzy {
        EditKind::Adapt(answer.text.clone())
    } else {
        EditKind::Correct(answer.text.clone())
    }
}

/// Writes the corrections of a pack that passed the checks, each only while
/// its translation is still the one the request was built from and not
/// reviewed, so work saved meanwhile is kept. An answer that left the
/// translation as it was, or left out a string of `untouched`, is counted
/// unchanged.
async fn correct(
    shared: &Shared,
    built: &Built,
    answers: &HashMap<String, Answer>,
    untouched: &HashSet<String>,
    rejected: Vec<Rejected>,
    attempts: Attempts,
) -> Result<Done, ModelError> {
    let mut edits = Vec::new();
    for (id, path, entry) in &built.strings {
        let Some(answer) = answers.get(id).filter(|answer| !answer.text.is_empty()) else {
            continue;
        };
        let Some(fix) = &shared.options.fix else {
            continue;
        };
        edits.push(EntryEdit {
            path: path.clone(),
            context: entry.context.clone(),
            expected_text: entry.translation.clone(),
            expected_fuzzy: entry.fuzzy,
            kind: correction_edit(fix, entry, answer),
        });
    }
    let asked = edits.len();
    let applied = {
        let session = Arc::clone(&shared.session);
        let knowledge = Arc::clone(&shared.knowledge);
        tokio::task::spawn_blocking(move || session.apply_edits_with(&edits, &knowledge))
            .await
            .map_err(|error| ModelError::Invalid(error.to_string()))?
            .map_err(|error| ModelError::Invalid(error.to_string()))?
    };
    if !applied.skipped.is_empty() {
        shared.run.log(format_args!(
            "{} corrections not written: their strings changed or were reviewed meanwhile",
            applied.skipped.len()
        ));
    }
    let reasons: HashMap<&str, Option<String>> = built
        .strings
        .iter()
        .filter_map(|(id, _, entry)| {
            let answer = answers.get(id)?;
            Some((entry.context.as_str(), answer.reason.clone()))
        })
        .collect();
    // A changed text, or an adaptation that clears the fuzzy mark, is a
    // correction; a record of the terms alone leaves the translation as it
    // was.
    let corrected: Vec<Corrected> = applied
        .done
        .iter()
        .filter(|done| done.before.text != done.after.text || done.before.fuzzy != done.after.fuzzy)
        .map(|done| Corrected {
            path: done.path.clone(),
            context: done.context.clone(),
            before: done.before.text.clone(),
            after: done.after.text.clone(),
            reason: reasons.get(done.context.as_str()).cloned().flatten(),
        })
        .collect();
    let mut fates: HashMap<String, Fate> = HashMap::new();
    for (id, _, entry) in &built.strings {
        if untouched.contains(id)
            || answers
                .get(id)
                .is_some_and(|answer| !answer.text.is_empty())
        {
            fates.insert(entry.context.clone(), Fate::bare(StringOutcome::Unchanged));
        }
    }
    for skipped in &applied.skipped {
        fates.insert(skipped.context.clone(), Fate::bare(StringOutcome::Stale));
    }
    for done in &corrected {
        fates.insert(
            done.context.clone(),
            Fate {
                outcome: StringOutcome::Written,
                after: Some(done.after.clone()),
                reason: done.reason.clone(),
            },
        );
    }
    report_strings(shared, built, attempts, &rejected, fates);
    // Every string of the run is counted once: an answer that changed
    // nothing of its text, or was not written, left it as it was.
    Ok(Done {
        written: corrected.len(),
        unchanged: asked.saturating_sub(corrected.len()) + untouched.len(),
        rejected,
        corrected,
    })
}

/// Runs the translation of `options.paths` until every batch was sent, the
/// run is cancelled, or the service stops it. Progress and the reason it
/// stopped are in `run`.
pub async fn run(session: Arc<Session>, codex: Arc<Codex>, options: Options, handle: Arc<Run>) {
    handle.update(|status| {
        *status = Status {
            running: true,
            fixing: options.fix.is_some(),
            pace: 1,
            started_at: now_millis(),
            ..Status::default()
        };
    });
    handle.report(&Record::Run {
        version: crate::report::VERSION,
        kind: if options.fix.is_some() {
            RunKind::Fixing
        } else {
            RunKind::Translating
        },
        model: options.model.clone(),
        effort: options.effort.clone(),
        terms: options
            .fix
            .as_ref()
            .and_then(|fix| fix.terms.clone())
            .unwrap_or_default(),
        skill: options.fix.as_ref().and_then(|fix| fix.skill.clone()),
        started_at: handle.status().started_at,
    });
    handle.report(&Record::Options {
        options: options.clone(),
        continues: handle.continues.as_ref().map(|(report, _)| report.clone()),
    });
    handle.log(format_args!(
        "run: model {}, reasoning {}, {} paths{}{}",
        options.model,
        options.effort.as_deref().unwrap_or("default"),
        if options.paths.is_empty() {
            "all".to_owned()
        } else {
            options.paths.len().to_string()
        },
        if options.contexts.is_empty() {
            String::new()
        } else {
            format!(", {} chosen strings", options.contexts.len())
        },
        if options.fuzzy { ", fuzzy too" } else { "" }
    ));
    if let Some(fix) = &options.fix {
        handle.log(format_args!(
            "correcting: {}{}{}{}",
            match (fix.issues, fix.groups.len()) {
                (false, _) => "problems only".to_owned(),
                (true, 0) => "every issue".to_owned(),
                (true, groups) => format!("{groups} kinds of issues"),
            },
            if fix.adapt { ", adapting fuzzy" } else { "" },
            match (&fix.skill, &fix.request) {
                (Some(name), _) => format!(", with the skill {name:?}"),
                (None, Some(_)) => ", with a request".to_owned(),
                (None, None) => String::new(),
            },
            match &fix.terms {
                None => String::new(),
                Some(terms) if terms.is_empty() => ", following every term".to_owned(),
                Some(terms) => format!(", following {} terms", terms.len()),
            }
        ));
    }
    let stop = drive(session, codex, options, &handle).await;
    handle.log(format_args!("stopped: {stop:?}"));
    handle.report(&Record::Stop { stop: stop.clone() });
    handle.update(|status| {
        status.running = false;
        status.pace = 0;
        status.active.clear();
        status.stop = Some(stop);
    });
}

/// The batches of a run and what its requests share.
async fn prepare(
    session: Arc<Session>,
    codex: Arc<Codex>,
    options: Options,
    run: Arc<Run>,
) -> Result<(Arc<Shared>, Vec<Batch>), Stop> {
    let prepared = {
        let session = Arc::clone(&session);
        let options = options.clone();
        let run = Arc::clone(&run);
        tokio::task::spawn_blocking(move || {
            let batches = match &options.fix {
                Some(fix) => {
                    let knowledge = session.knowledge();
                    let target = session.settings().target_language;
                    let matcher = Matcher::new(&fix.filter)?;
                    plan_of(
                        session.root(),
                        &options.paths,
                        &fix.exclude,
                        &options.contexts,
                        &|entry| {
                            !run.settled(&entry.context)
                                && needs_fix(fix, &knowledge, &target, entry)
                                && matcher.matches(entry)
                        },
                    )?
                }
                None => plan_of(
                    session.root(),
                    &options.paths,
                    &[],
                    &options.contexts,
                    &|entry| !run.settled(&entry.context) && needs_work(entry, options.fuzzy),
                )?,
            };
            Ok::<_, String>(batches)
        })
        .await
    };
    let batches = match prepared {
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
    let instructions = match &options.fix {
        Some(fix) => prompt::fix_instructions(
            &settings.source_language,
            &settings.target_language,
            knowledge.style.as_deref(),
            fix.adapt,
            fix.omits_unchanged(),
            fix.request.as_deref(),
        ),
        None => prompt::instructions(
            &settings.source_language,
            &settings.target_language,
            knowledge.style.as_deref(),
        ),
    };
    let matcher = options
        .fix
        .as_ref()
        .map(|fix| Matcher::new(&fix.filter))
        .transpose()
        .map_err(|message| Stop::Failed { message })?
        .unwrap_or_default();
    let shared = Arc::new(Shared {
        matcher,
        instructions,
        target: settings.target_language,
        knowledge,
        cache_key: format!("aeria-{}", now_millis()),
        session,
        codex,
        options,
        run,
    });
    Ok((shared, batches))
}

/// How the loop goes on after a failed batch.
enum Next {
    Continue,
    Wait(Duration),
    /// The plan's usage limit: the batch goes back, and the run waits until
    /// the limit resets, at these Unix seconds when the service said.
    Limit(Option<u64>),
    Stop(Stop),
}

/// Handles a failed batch: the service's limit and a lost sign-in stop the
/// run; a rate limit, network error, or timeout returns the batch to the
/// queue after a wait (a rate limit also halves the pace), and the third in
/// a row stops the run; anything else leaves the batch's strings for the
/// next run.
fn failed(error: ModelError, failures: &mut u32, pace: &mut usize, handle: &Run) -> Next {
    match error {
        ModelError::UsageLimit { resets_at, .. } => Next::Limit(resets_at),
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
            handle.log(format_args!(
                "waiting {} s after failure {} in a row: {error}",
                wait.as_secs(),
                *failures
            ));
            Next::Wait(wait)
        }
        error => {
            handle.log(format_args!("batch left for the next run: {error}"));
            handle.update(|status| {
                status.batches_done += 1;
                status.message = Some(error.to_string());
            });
            Next::Continue
        }
    }
}

/// The requests of a queue of batches: a file whose open strings are one
/// batch is packed with the files after it, up to [`SCENE_BATCH`] strings and
/// [`PACK_FILES`] files; a batch of a file split into several goes alone, so
/// a long scene still goes part by part.
fn pack(batches: VecDeque<Batch>) -> VecDeque<Pack> {
    let mut per_file: HashMap<String, usize> = HashMap::new();
    for batch in &batches {
        *per_file.entry(batch.path.clone()).or_default() += 1;
    }
    let mut packs = VecDeque::new();
    let mut open: Pack = Vec::new();
    let mut size = 0;
    for batch in batches {
        if per_file[&batch.path] > 1 {
            if !open.is_empty() {
                packs.push_back(std::mem::take(&mut open));
                size = 0;
            }
            packs.push_back(vec![batch]);
            continue;
        }
        if !open.is_empty()
            && (size + batch.contexts.len() > SCENE_BATCH || open.len() == PACK_FILES)
        {
            packs.push_back(std::mem::take(&mut open));
            size = 0;
        }
        size += batch.contexts.len();
        open.push(batch);
    }
    if !open.is_empty() {
        packs.push_back(open);
    }
    packs
}

/// The request pace and the failures in a row, kept across phases.
struct Pace {
    now: usize,
    failures: u32,
    /// When the last wait for the usage limit ended.
    limit_waited: Option<std::time::Instant>,
}

/// The first pack of `queue` none of whose files has a batch in flight.
/// Batches of a scene go one after another, so each continues the dialogue
/// the one before translated; other files go side by side.
fn take_next(queue: &mut VecDeque<Pack>, busy: &HashSet<String>) -> Option<Pack> {
    let index = queue
        .iter()
        .position(|pack| pack.iter().all(|batch| !busy.contains(&batch.path)))?;
    queue.remove(index)
}

/// Sends the batches of `queue` until all have answered; a stop ends it
/// early.
async fn drain(
    shared: &Arc<Shared>,
    mut queue: VecDeque<Pack>,
    pace: &mut Pace,
    handle: &Run,
) -> Option<Stop> {
    let mut running: JoinSet<(Pack, Result<Done, ModelError>)> = JoinSet::new();
    // Scene files with a batch in flight, and the files of each task.
    let mut busy: HashSet<String> = HashSet::new();
    let mut files: HashMap<tokio::task::Id, Vec<String>> = HashMap::new();
    loop {
        if handle.cancelled() {
            running.abort_all();
            return Some(Stop::Cancelled);
        }
        while running.len() < pace.now
            && let Some(next) = take_next(&mut queue, &busy)
        {
            let paths: Vec<String> = next.iter().map(|batch| batch.path.clone()).collect();
            busy.extend(paths.iter().filter(|path| is_scene(path)).cloned());
            let shared = Arc::clone(shared);
            let task = running.spawn(async move {
                let result = translate(shared, next.clone()).await;
                (next, result)
            });
            files.insert(task.id(), paths);
        }
        let joined = tokio::select! {
            // Nothing left running means every batch has answered.
            joined = running.join_next_with_id() => joined?,
            () = handle.until_cancelled() => {
                running.abort_all();
                return Some(Stop::Cancelled);
            }
        };
        let id = match &joined {
            Ok((id, _)) => *id,
            Err(error) => error.id(),
        };
        for path in files.remove(&id).unwrap_or_default() {
            busy.remove(&path);
        }
        let Ok((_, (sent, result))) = joined else {
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
                count_done(handle, done, pace.now);
            }
            Err(error) => {
                let retry = matches!(
                    error,
                    ModelError::RateLimited { .. } | ModelError::Network(_) | ModelError::Timeout
                );
                let message = error.to_string();
                match failed(error, &mut pace.failures, &mut pace.now, handle) {
                    Next::Continue => handle.report(&Record::Left {
                        paths: sent.iter().map(|batch| batch.path.clone()).collect(),
                        strings: sent.iter().map(|batch| batch.contexts.len()).sum(),
                        error: message,
                    }),
                    Next::Wait(wait) => {
                        if retry {
                            queue.push_front(sent);
                        }
                        tokio::select! {
                            () = tokio::time::sleep(wait) => {}
                            () = handle.until_cancelled() => {
                                running.abort_all();
                                return Some(Stop::Cancelled);
                            }
                        }
                    }
                    Next::Limit(resets_at) => {
                        queue.push_front(sent);
                        if !wait_for_limit(pace, handle, resets_at).await {
                            running.abort_all();
                            return Some(Stop::Cancelled);
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

/// Counts what a batch did in the run's status, at the pace `now`.
fn count_done(handle: &Run, done: Done, now: usize) {
    handle.update(|status| {
        status.batches_done += 1;
        status.written += done.written;
        status.unchanged += done.unchanged;
        status.rejected += done.rejected.len();
        let room = LISTED_REJECTIONS.saturating_sub(status.rejections.len());
        status
            .rejections
            .extend(done.rejected.into_iter().take(room));
        let room = LISTED_CORRECTIONS.saturating_sub(status.corrections.len());
        status
            .corrections
            .extend(done.corrected.into_iter().take(room));
        status.pace = now;
        status.message = None;
    });
}

/// Waits for the plan's usage limit to reset, at `resets_at` when the
/// service said, and goes on at the pace of one request; `false` when the
/// run is cancelled meanwhile. The other requests in flight meet the same
/// limit: one that fails just after a wait goes back without waiting again.
async fn wait_for_limit(pace: &mut Pace, handle: &Run, resets_at: Option<u64>) -> bool {
    if pace
        .limit_waited
        .is_some_and(|at| at.elapsed() < LIMIT_SETTLE)
    {
        return true;
    }
    pace.now = 1;
    let wait = limit_wait(resets_at);
    handle.log(format_args!(
        "usage limit: waiting {} s for it to reset",
        wait.as_secs()
    ));
    let until = now_millis() / 1000 + wait.as_secs();
    handle.update(|status| {
        status.pace = 1;
        status.waiting_until = Some(until);
    });
    let waited = tokio::select! {
        () = tokio::time::sleep(wait) => true,
        () = handle.until_cancelled() => false,
    };
    handle.update(|status| status.waiting_until = None);
    pace.limit_waited = Some(std::time::Instant::now());
    waited
}

/// How long a usage-limit failure that follows a wait for the limit counts
/// as one met before the wait ended.
const LIMIT_SETTLE: Duration = Duration::from_secs(10);
/// How long a run waits for a usage limit whose reset the service did not
/// say, before it asks again.
const LIMIT_RETRY: Duration = Duration::from_mins(15);
/// What a run waits past the reset the service said.
const LIMIT_MARGIN: Duration = Duration::from_secs(60);

/// How long to wait for a usage limit that resets at `resets_at`, Unix
/// seconds, when the service said.
fn limit_wait(resets_at: Option<u64>) -> Duration {
    match resets_at {
        Some(at) => Duration::from_secs(at.saturating_sub(now_millis() / 1000)) + LIMIT_MARGIN,
        None => LIMIT_RETRY,
    }
}

async fn drive(
    session: Arc<Session>,
    codex: Arc<Codex>,
    options: Options,
    handle: &Arc<Run>,
) -> Stop {
    let (shared, batches) = match prepare(session, codex, options, Arc::clone(handle)).await {
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
    let planned = handle.status();
    handle.log(format_args!(
        "planned: {} strings of {} files in {} batches",
        planned.strings, planned.files, planned.batches
    ));
    // The first request goes alone and stores the instructions in the
    // service's cache; the others start when it has answered.
    let mut pace = Pace {
        now: 1,
        failures: 0,
        limit_waited: None,
    };
    if let Some(stop) = drain(&shared, pack(batches.into()), &mut pace, handle).await {
        return stop;
    }
    Stop::Finished
}

/// Counts the strings a correction `fix` of `paths` takes, by the first of
/// `paths` that chooses each one's file, as the run plans them: one count
/// when `paths` is empty, for every string of the project.
///
/// # Errors
///
/// Returns a description when the filter does not compile or `po/` cannot
/// be read.
pub fn count_fix(session: &Session, fix: &Fix, paths: &[String]) -> Result<Vec<usize>, String> {
    let knowledge = session.knowledge();
    let target = session.settings().target_language;
    let matcher = Matcher::new(&fix.filter)?;
    let root = session.root();
    let mut counts = vec![0; paths.len().max(1)];
    for path in aeria_po::list(root).map_err(|error| error.to_string())? {
        if excluded(&path, &fix.exclude) {
            continue;
        }
        let Some(at) = (if paths.is_empty() {
            Some(0)
        } else {
            paths
                .iter()
                .position(|chosen| selected(&path, std::slice::from_ref(chosen)))
        }) else {
            continue;
        };
        let full = root.join(PO_DIR).join(&path);
        let text = std::fs::read_to_string(&full).map_err(|error| format!("po/{path}: {error}"))?;
        counts[at] += PoFile::parse(&text)
            .0
            .entries
            .iter()
            .filter(|entry| needs_fix(fix, &knowledge, &target, entry) && matcher.matches(entry))
            .count();
    }
    Ok(counts)
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
    for batch in plan(root, paths, fuzzy, &[])? {
        *counts.entry(batch.path).or_insert(0) += batch.contexts.len();
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macros_are_explained_from_the_catalog_of_now() {
        let entry = Entry {
            extracted: vec![
                "ja: <if $gn7>…<else>{$gs2}</if>".to_owned(),
                "macro: <if $gn7>a<else>{$gs2}</if> — condition = global number 7".to_owned(),
            ],
            source: "<if $gn7>a<else>{$gs2}</if>".to_owned(),
            ..Entry::default()
        };
        let context = context_of(&entry);
        assert_eq!(context[0], entry.extracted[0], "other comments stay");
        assert_eq!(context.len(), 2);
        assert!(context[1].starts_with("macro: <if $gn7>"), "{}", context[1]);
        assert!(context[1].contains("ObjStr row"), "{}", context[1]);
    }

    #[test]
    fn a_correction_of_terms_takes_the_strings_written_with_an_older_version() {
        let term = |term: &str, translation: &str| GlossaryEntry {
            term: term.to_owned(),
            translation: translation.to_owned(),
            ..GlossaryEntry::default()
        };
        let knowledge = |moghome: &str| Knowledge {
            terms: aeria_knowledge::Glossary::new(vec![
                term("Moghome", moghome),
                term("Kupo", "Купо"),
            ]),
            ..Knowledge::default()
        };
        let before = knowledge("Моглин");
        let mut entry = Entry {
            source: "Kupo! Welcome to the Moghome.".to_owned(),
            translation: "Купо! Добро пожаловать в Моглин.".to_owned(),
            ..Entry::default()
        };
        entry.record_terms(&before.terms);
        let fix = Fix {
            terms: Some(Vec::new()),
            ..Fix::default()
        };
        assert!(!needs_fix(&fix, &before, "ru", &entry));
        let after = knowledge("Моглодом");
        assert!(needs_fix(&fix, &after, "ru", &entry));
        let items = fix_items(&fix, &after, "ru", &entry);
        assert_eq!(items.len(), 1);
        assert!(items[0].contains("\"Moghome\""), "{}", items[0]);
        assert!(!items[0].contains("Kupo"), "{}", items[0]);
        // Only the terms asked for; a reviewed translation is a person's.
        let other = Fix {
            terms: Some(vec!["kupo".to_owned()]),
            ..Fix::default()
        };
        assert!(!needs_fix(&other, &after, "ru", &entry));
        entry.set_reviewed(true);
        assert!(!needs_fix(&fix, &after, "ru", &entry));
        // A run that is not about terms does not take it.
        entry.set_reviewed(false);
        assert!(!needs_fix(&Fix::default(), &after, "ru", &entry));
    }

    #[test]
    fn a_usage_limit_is_waited_for_until_it_resets() {
        let now = now_millis() / 1000;
        let wait = limit_wait(Some(now + 600));
        assert!(
            wait >= Duration::from_secs(600) && wait <= Duration::from_secs(661),
            "{wait:?}"
        );
        // A reset already past is asked again after the margin.
        assert_eq!(limit_wait(Some(now.saturating_sub(10))), LIMIT_MARGIN);
        assert_eq!(limit_wait(None), LIMIT_RETRY);
    }

    #[test]
    fn what_a_broken_answer_left_of_its_json_is_refused() {
        assert!(answer_debris("Mount Yorn", r#"Гора Йорн","",["mount"#).is_some());
        assert!(answer_debris("Hi", r#"Привет", "калька"#).is_some());
        assert!(answer_debris("Hi", "Привет, «друг»").is_none());
        assert!(answer_debris(r#"Say "yes", "no""#, r#"Скажи «да», "нет""#).is_none());
    }

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
    fn a_stop_reads_as_the_renderer_expects() {
        let stop = serde_json::to_value(Stop::UsageLimit {
            resets_at: Some(1_900_000_000),
        })
        .expect("json");
        assert_eq!(
            stop,
            serde_json::json!({ "reason": "usageLimit", "resetsAt": 1_900_000_000 })
        );
    }

    #[test]
    fn a_scene_waits_for_its_batch_in_flight() {
        let batch = |path: &str, first: usize| Batch {
            path: path.to_owned(),
            contexts: vec![format!("{path}:{first}")],
            first,
            last: first,
        };
        let mut queue: VecDeque<Pack> = vec![
            vec![batch("cut_scene/024/A.po", 100)],
            vec![batch("cut_scene/024/A.po", 200)],
            vec![batch("quest/001/B.po", 0), batch("Addon/0.po", 0)],
        ]
        .into();
        let busy: HashSet<String> = HashSet::from(["cut_scene/024/A.po".to_owned()]);
        let next = take_next(&mut queue, &busy).expect("other files");
        assert_eq!(next[1].path, "Addon/0.po");
        assert!(take_next(&mut queue, &busy).is_none());
        let next = take_next(&mut queue, &HashSet::new()).expect("the scene goes on");
        assert_eq!(next[0].first, 100);
    }

    #[test]
    fn small_files_go_together_and_a_split_file_alone() {
        let batch = |path: &str, strings: usize| Batch {
            path: path.to_owned(),
            contexts: (0..strings)
                .map(|index| format!("{path}:{index}"))
                .collect(),
            first: 0,
            last: strings.saturating_sub(1),
        };
        let queue: VecDeque<Batch> = vec![
            batch("quest/000/A.po", 30),
            batch("quest/000/B.po", 60),
            batch("quest/000/C.po", 70),
            batch("cut_scene/000/D.po", 100),
            batch("cut_scene/000/D.po", 100),
            batch("quest/000/E.po", 5),
        ]
        .into();
        let packs: Vec<Vec<String>> = pack(queue)
            .into_iter()
            .map(|pack| {
                pack.into_iter()
                    .map(|batch| batch.path[batch.path.len() - 4..].to_owned())
                    .collect()
            })
            .collect();
        assert_eq!(
            packs,
            vec![
                vec!["A.po", "B.po"],
                vec!["C.po"],
                vec!["D.po"],
                vec!["D.po"],
                vec!["E.po"],
            ]
        );
    }
}
