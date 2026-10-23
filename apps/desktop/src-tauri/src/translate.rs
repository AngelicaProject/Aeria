//! Machine translation in the desktop: signing in with a ChatGPT
//! subscription and one run at a time over files of the open project (see
//! `docs/architecture/translate.md`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use aeria_model::auth::{DeviceLogin, VERIFICATION_URL};
use aeria_model::{Codex, Fix, KeyringStore, ModelError, ModelInfo, Options, Run, Status};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::commands::run_blocking;
use crate::dto::SourceBindingDto;
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// The client, the sign-in in progress, and the run of the desktop.
#[derive(Default)]
pub struct Translation {
    codex: OnceLock<Arc<Codex>>,
    login: Mutex<Option<DeviceLogin>>,
    run: Mutex<Option<Arc<Run>>>,
    /// Where in the game the strings the run lists are, found once per run:
    /// the status is read every second and finding a string can read its
    /// sheet from the game.
    bindings: Mutex<HashMap<String, Option<SourceBindingDto>>>,
    /// The report of the run, by its id.
    report: Mutex<Option<String>>,
}

impl Translation {
    pub(crate) fn codex(&self) -> CommandResult<Arc<Codex>> {
        if let Some(codex) = self.codex.get() {
            return Ok(Arc::clone(codex));
        }
        let codex = Arc::new(Codex::new(Arc::new(KeyringStore))?);
        Ok(Arc::clone(self.codex.get_or_init(|| codex)))
    }

    fn run(&self) -> Option<Arc<Run>> {
        self.run
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Where each of `contexts` is in the game, found only for those not
    /// seen before in this run.
    fn bindings<'a>(
        &self,
        session: Option<&aeria_po::Session>,
        contexts: impl Iterator<Item = &'a str>,
    ) -> HashMap<String, Option<SourceBindingDto>> {
        let mut known = self.bindings.lock().unwrap_or_else(PoisonError::into_inner);
        // Without a project nothing is found, and nothing is kept as not found.
        let Some(session) = session else {
            return known.clone();
        };
        let mut missing: Vec<&str> = contexts
            .filter(|context| !known.contains_key(*context))
            .collect();
        // A context begins with its sheet, so sorted ones read each sheet once.
        missing.sort_unstable();
        missing.dedup();
        for context in missing {
            known.insert(context.to_owned(), crate::search::binding(session, context));
        }
        known.clone()
    }
}

impl From<ModelError> for CommandError {
    fn from(error: ModelError) -> Self {
        let code = match &error {
            ModelError::SignInRequired => "modelSignInRequired",
            ModelError::UsageLimit { .. } => "modelUsageLimit",
            ModelError::RateLimited { .. } => "modelRateLimited",
            ModelError::Network(_) | ModelError::Timeout => "modelUnreachable",
            ModelError::Refused { .. } | ModelError::Invalid(_) => "modelRefused",
            ModelError::Store(_) => "modelCredentialStore",
        };
        Self::new(code, error.to_string())
    }
}

/// Whether a ChatGPT subscription is signed in.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelAccountDto {
    pub signed_in: bool,
    /// The account's e-mail address, once a request read it.
    pub account: Option<String>,
}

#[tauri::command(rename_all = "camelCase")]
/// Whether a ChatGPT subscription is signed in.
///
/// # Errors
///
/// Returns `modelCredentialStore` when the OS credential store cannot be
/// read.
pub async fn model_account(app: tauri::AppHandle) -> CommandResult<ModelAccountDto> {
    let codex = app.state::<Translation>().codex()?;
    Ok(ModelAccountDto {
        signed_in: codex.signed_in()?,
        account: codex.account().await,
    })
}

/// A device sign-in to finish in the browser.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSignInDto {
    pub user_code: String,
    pub verification_url: String,
    /// Seconds between polls.
    pub interval: u64,
}

#[tauri::command(rename_all = "camelCase")]
/// Starts signing in with a ChatGPT subscription: the code to enter on
/// OpenAI's page.
///
/// # Errors
///
/// Returns a model error when OpenAI refuses or is unreachable.
pub async fn model_sign_in_start(app: tauri::AppHandle) -> CommandResult<ModelSignInDto> {
    let translation = app.state::<Translation>();
    let login = translation.codex()?.sign_in_start().await?;
    let dto = ModelSignInDto {
        user_code: login.user_code.clone(),
        verification_url: VERIFICATION_URL.to_owned(),
        interval: login.interval.as_secs(),
    };
    *translation
        .login
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(login);
    Ok(dto)
}

#[tauri::command(rename_all = "camelCase")]
/// Opens OpenAI's page where the sign-in code is entered.
///
/// # Errors
///
/// Returns `openFailed` when no browser can be started.
#[allow(clippy::needless_pass_by_value)]
pub fn model_open_sign_in_page(app: tauri::AppHandle) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;

    app.opener()
        .open_url(VERIFICATION_URL, None::<&str>)
        .map_err(|error| CommandError::new("openFailed", error.to_string()))
}

#[tauri::command(rename_all = "camelCase")]
/// Polls the sign-in once; `true` once it finished.
///
/// # Errors
///
/// Returns `modelSignInRequired` without a sign-in in progress, or a model
/// error when the sign-in failed.
pub async fn model_sign_in_poll(app: tauri::AppHandle) -> CommandResult<bool> {
    let translation = app.state::<Translation>();
    let login = translation
        .login
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or(ModelError::SignInRequired)?;
    let done = translation.codex()?.sign_in_poll(&login).await?;
    if done {
        *translation
            .login
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }
    Ok(done)
}

#[tauri::command(rename_all = "camelCase")]
/// Forgets the ChatGPT sign-in.
///
/// # Errors
///
/// Returns `modelCredentialStore` when the OS credential store cannot be
/// written.
pub async fn model_sign_out(app: tauri::AppHandle) -> CommandResult<()> {
    Ok(app.state::<Translation>().codex()?.sign_out().await?)
}

#[tauri::command(rename_all = "camelCase")]
/// The models the signed-in account can use.
///
/// # Errors
///
/// Returns a model error when the account is not signed in or the service
/// cannot be reached.
pub async fn model_list(app: tauri::AppHandle) -> CommandResult<Vec<ModelInfo>> {
    Ok(app.state::<Translation>().codex()?.models().await?)
}

/// The files and folders of `po/` of a scope: sheet names, and folders of
/// sheet names ending with `/`, such as `quest/001/`. An empty scope is the
/// whole project.
fn paths_of(session: &aeria_po::Session, scope: &[String]) -> Vec<String> {
    scope
        .iter()
        .map(|entry| {
            entry
                .strip_suffix('/')
                .map_or_else(|| session.sheet_base(entry), str::to_owned)
        })
        .collect()
}

#[tauri::command(rename_all = "camelCase")]
/// Starts translating `scope` (see [`paths_of`]) of the open project with
/// `model`. One run at a time; its progress is [`translation_status`].
///
/// # Errors
///
/// Returns `translationRunning` while a run goes, or `noProjectOpen`.
pub async fn translation_start(
    app: tauri::AppHandle,
    scope: Vec<String>,
    fuzzy: bool,
    model: String,
    effort: Option<String>,
) -> CommandResult<()> {
    let session = app.state::<DesktopState>().session()?;
    start_run(
        &app,
        Options {
            paths: paths_of(&session, &scope),
            fuzzy,
            contexts: Vec::new(),
            model,
            effort: effort.filter(|effort| !effort.is_empty()),
            fix: None,
        },
    )
}

/// The id of the report of the run going, if one goes.
pub(crate) fn running_report(app: &tauri::AppHandle) -> Option<String> {
    let translation = app.state::<Translation>();
    if !translation.run().is_some_and(|run| run.status().running) {
        return None;
    }
    translation
        .report
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Whether a run is going.
pub(crate) fn is_running(app: &tauri::AppHandle) -> bool {
    app.state::<Translation>()
        .run()
        .is_some_and(|run| run.status().running)
}

/// Starts a run of the open project; one run at a time.
///
/// # Errors
///
/// Returns `translationRunning` while a run goes, `noProjectOpen`, or an
/// error of the credential store.
pub(crate) fn start_run(app: &tauri::AppHandle, options: Options) -> CommandResult<()> {
    start_run_with(app, options, None)
}

/// Starts a run as [`start_run`] does; with `continues`, the run continues
/// the run of that report, leaving out the strings settled before.
pub(crate) fn start_run_with(
    app: &tauri::AppHandle,
    options: Options,
    continues: Option<(String, std::collections::HashSet<String>)>,
) -> CommandResult<()> {
    let translation = app.state::<Translation>();
    let codex = translation.codex()?;
    let session = app.state::<DesktopState>().session()?;
    let run = {
        let mut current = translation
            .run
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if current.as_ref().is_some_and(|run| run.status().running) {
            return Err(running());
        }
        let (id, log, report) = journal(app);
        *translation
            .report
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = report.is_some().then_some(id);
        let run = match log {
            Some(file) => Run::with_log(file),
            None => Run::new(),
        };
        let run = match continues {
            Some((report, settled)) => run.continuing(report, settled),
            None => run,
        };
        let run = Arc::new(match report {
            Some(file) => run.with_report(Box::new(crate::report::ReportFile::new(file))),
            None => run,
        });
        *current = Some(Arc::clone(&run));
        translation
            .bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        run
    };
    tauri::async_runtime::spawn(aeria_model::run::run(session, codex, options, run));
    Ok(())
}

/// Journals of runs kept in the data folder; older ones are deleted.
const JOURNALS: usize = 20;

/// A new journal and report for a run in `logs/` of the data folder, named
/// by the time it starts, after deleting the oldest beyond [`JOURNALS`]. A
/// run goes on without them when the folder cannot be written.
fn journal(app: &tauri::AppHandle) -> (String, Option<std::fs::File>, Option<std::fs::File>) {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis())
        .to_string();
    let Some(folder) = crate::report::folder(app) else {
        return (millis, None, None);
    };
    if std::fs::create_dir_all(&folder).is_err() {
        return (millis, None, None);
    }
    prune_journals(&folder, JOURNALS - 1);
    let create = |extension: &str| {
        std::fs::File::create(folder.join(format!("translation-{millis}.{extension}"))).ok()
    };
    let (log, report) = (create("log"), create("jsonl"));
    (millis, log, report)
}

/// Deletes the oldest run journals and reports of `folder` so that at most
/// `keep` of each stay. Their names hold the start time, so age order is the
/// order of the times.
fn prune_journals(folder: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    for extension in ["log", "jsonl"] {
        let mut runs: Vec<(u128, &PathBuf)> = paths
            .iter()
            .filter(|path| path.extension().is_some_and(|found| found == extension))
            .filter_map(|path| {
                let stem = path.file_stem()?.to_str()?;
                Some((stem.strip_prefix("translation-")?.parse().ok()?, path))
            })
            .collect();
        runs.sort();
        let excess = runs.len().saturating_sub(keep);
        for (_, old) in &runs[..excess] {
            let _ = std::fs::remove_file(old);
        }
    }
}

/// The error of a second run.
pub(crate) fn running() -> CommandError {
    CommandError::new(
        "translationRunning",
        "a machine translation is running; stop it or wait for it to finish",
    )
}

/// A string whose translation the checks rejected, with where it is in the
/// game and its problems as data.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectionDto {
    pub path: String,
    pub context: String,
    pub binding: Option<SourceBindingDto>,
    /// The model's last translation, which was not written.
    pub translation: String,
    pub problems: Vec<crate::dto::IssueDto>,
}

/// A translation a correction changed, with where it is in the game.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionDto {
    #[serde(flatten)]
    pub corrected: aeria_model::Corrected,
    pub binding: Option<SourceBindingDto>,
}

/// The progress of a run, with its rejected strings and the corrections it
/// wrote.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    #[serde(flatten)]
    pub status: Status,
    pub rejections: Vec<RejectionDto>,
    pub corrections: Vec<CorrectionDto>,
    /// The id of the run's report, when it has one.
    pub report: Option<String>,
}

fn rejection_dto(
    rejected: &aeria_model::Rejected,
    binding: Option<SourceBindingDto>,
) -> RejectionDto {
    let problems = crate::report::problem_dtos(&rejected.problems, &rejected.issues);
    RejectionDto {
        path: rejected.path.clone(),
        context: rejected.context.clone(),
        binding,
        translation: rejected.translation.clone(),
        problems,
    }
}

#[tauri::command(rename_all = "camelCase")]
/// The progress of the last run; `None` before one started.
///
/// # Errors
///
/// Returns an internal error when the desktop worker fails.
pub async fn translation_status(app: tauri::AppHandle) -> CommandResult<Option<StatusDto>> {
    run_blocking(move || Ok(status_with_state(&app))).await
}

fn status_with_state(app: &tauri::AppHandle) -> Option<StatusDto> {
    let translation = app.state::<Translation>();
    let status = translation.run()?.status();
    let session = app.state::<DesktopState>().session().ok();
    let bindings = translation.bindings(
        session.as_deref(),
        status
            .rejections
            .iter()
            .map(|rejected| rejected.context.as_str())
            .chain(
                status
                    .corrections
                    .iter()
                    .map(|corrected| corrected.context.as_str()),
            ),
    );
    let binding = |context: &str| bindings.get(context).cloned().flatten();
    let rejections = status
        .rejections
        .iter()
        .map(|rejected| rejection_dto(rejected, binding(&rejected.context)))
        .collect();
    let corrections = status
        .corrections
        .iter()
        .map(|corrected| CorrectionDto {
            binding: binding(&corrected.context),
            corrected: corrected.clone(),
        })
        .collect();
    let report = translation
        .report
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Some(StatusDto {
        status,
        rejections,
        corrections,
        report,
    })
}

#[tauri::command(rename_all = "camelCase")]
/// Translates the given strings again, by `msgctxt`: those still
/// untranslated, or fuzzy, as rejected strings stay.
///
/// # Errors
///
/// Returns `translationRunning` while a run goes, or `noProjectOpen`.
pub async fn translation_retry(
    app: tauri::AppHandle,
    contexts: Vec<String>,
    model: String,
    effort: Option<String>,
) -> CommandResult<()> {
    app.state::<DesktopState>().session()?;
    start_run(
        &app,
        Options {
            paths: Vec::new(),
            fuzzy: true,
            contexts,
            model,
            effort: effort.filter(|effort| !effort.is_empty()),
            fix: None,
        },
    )
}

/// What a correction asks of each translation.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixDto {
    /// Fix what the checks find, only of `groups` when it is not empty.
    pub issues: bool,
    #[serde(default)]
    pub groups: Vec<String>,
    /// Adapt a fuzzy translation to its changed source and clear the mark.
    #[serde(default)]
    pub adapt: bool,
    /// What the translator asks of each translation.
    #[serde(default)]
    pub request: Option<String>,
    /// The name of the project's skill to follow; its saved text, not its
    /// scope, is asked of each translation, after `request`.
    #[serde(default)]
    pub skill: Option<String>,
    /// Bring the translations written with an older version of these terms,
    /// by headword, or of any term when empty, to the terms as they are.
    #[serde(default)]
    pub terms: Option<Vec<String>>,
}

/// Starts a correction run of the strings `contexts` of the files `paths`.
fn start_fix(
    app: &tauri::AppHandle,
    paths: Vec<String>,
    contexts: Vec<String>,
    fix: FixDto,
    model: String,
    effort: Option<String>,
) -> CommandResult<()> {
    let skill_text = match fix.skill.as_deref() {
        Some(name) => {
            let root = app.state::<DesktopState>().session()?.root().to_owned();
            let text = aeria_knowledge::read_skill(&root, name)
                .map_err(|message| CommandError::new("projectKnowledgeRead", message))?
                .ok_or_else(|| {
                    CommandError::new(
                        "skillNotFound",
                        format!("the project has no skill {name:?}"),
                    )
                })?;
            let (_, body) = aeria_knowledge::parse_skill(&text)
                .map_err(|message| CommandError::new("projectKnowledgeInvalid", message))?;
            Some(body)
        }
        None => None,
    };
    // The translator's words, then the skill's.
    let request = [fix.request.as_deref(), skill_text.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let request = (!request.is_empty()).then_some(request);
    if !fix.issues && !fix.adapt && request.is_none() && fix.terms.is_none() {
        return Err(CommandError::new(
            "fixEmpty",
            "choose a skill, the issues to fix, adapting, or terms",
        ));
    }
    start_run(
        app,
        Options {
            paths,
            fuzzy: false,
            contexts,
            model,
            effort: effort.filter(|effort| !effort.is_empty()),
            fix: Some(Fix {
                issues: fix.issues,
                groups: fix.groups,
                adapt: fix.adapt,
                request,
                skill: fix.skill,
                terms: fix.terms,
                ..Fix::default()
            }),
        },
    )
}

#[tauri::command(rename_all = "camelCase")]
/// Corrects the translations of the given strings, by `msgctxt` in the
/// files `paths` (relative to `po/`), as `fix` asks. A reviewed translation
/// is left as it is.
///
/// # Errors
///
/// Returns `translationRunning` while a run goes, `fixEmpty` when there is
/// nothing to ask, or `noProjectOpen`.
pub async fn translation_fix(
    app: tauri::AppHandle,
    paths: Vec<String>,
    contexts: Vec<String>,
    fix: FixDto,
    model: String,
    effort: Option<String>,
) -> CommandResult<()> {
    app.state::<DesktopState>().session()?;
    start_fix(&app, paths, contexts, fix, model, effort)
}

#[tauri::command(rename_all = "camelCase")]
/// Corrects the translation of one string, by where it is in the game, as
/// `fix` asks.
///
/// # Errors
///
/// As [`translation_fix`], and `translationPersistence` when the string is
/// not an entry of the project.
pub async fn translation_fix_string(
    app: tauri::AppHandle,
    source_binding: SourceBindingDto,
    fix: FixDto,
    model: String,
    effort: Option<String>,
) -> CommandResult<()> {
    let session = app.state::<DesktopState>().session()?;
    let (path, context, _) = session.locate(
        &source_binding.sheet_name,
        source_binding.row_id,
        source_binding.subrow_id,
        source_binding.column_index,
    )?;
    start_fix(&app, vec![path], vec![context], fix, model, effort)
}

#[tauri::command(rename_all = "camelCase")]
/// Stops the run; batches in flight are dropped and translated by the next
/// run.
#[allow(clippy::needless_pass_by_value)]
pub fn translation_stop(app: tauri::AppHandle) {
    if let Some(run) = app.state::<Translation>().run() {
        run.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_journals_are_deleted() {
        let folder = tempfile::tempdir().expect("folder");
        for millis in [1_000, 3_000, 2_000, 500] {
            std::fs::write(folder.path().join(format!("translation-{millis}.log")), "")
                .expect("journal");
        }
        std::fs::write(folder.path().join("translation-500.jsonl"), "").expect("report");
        std::fs::write(folder.path().join("other.log"), "").expect("other");
        prune_journals(folder.path(), 2);
        let mut left: Vec<String> = std::fs::read_dir(folder.path())
            .expect("list")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .into_string()
                    .expect("name")
            })
            .collect();
        left.sort();
        // 500 is older than 1000, though its name sorts after it.
        assert_eq!(
            left,
            [
                "other.log",
                "translation-2000.log",
                "translation-3000.log",
                "translation-500.jsonl"
            ]
        );
    }
}
