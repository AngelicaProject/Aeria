//! Machine translation in the desktop: signing in with a ChatGPT
//! subscription and one run at a time over files of the open project (see
//! `docs/architecture/translate.md`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use aeria_model::auth::{DeviceLogin, VERIFICATION_URL};
use aeria_model::{Codex, KeyringStore, ModelError, ModelInfo, Options, Run, Status};
use serde::Serialize;
use tauri::Manager;

use crate::error::CommandError;
use crate::paths::AeriaPaths;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// The client, the sign-in in progress, and the run of the desktop.
#[derive(Default)]
pub struct Translation {
    codex: OnceLock<Arc<Codex>>,
    login: Mutex<Option<DeviceLogin>>,
    run: Mutex<Option<Arc<Run>>>,
}

impl Translation {
    fn codex(&self) -> CommandResult<Arc<Codex>> {
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
/// The sheets whose strings are the game's names, in the order a run
/// translates them before any other sheet.
#[must_use]
pub fn translation_name_sheets() -> Vec<&'static str> {
    aeria_model::names::NAME_SHEETS.to_vec()
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
        },
    )
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
        let run = Arc::new(match journal(app) {
            Some(file) => Run::with_log(file),
            None => Run::new(),
        });
        *current = Some(Arc::clone(&run));
        run
    };
    tauri::async_runtime::spawn(aeria_model::run::run(session, codex, options, run));
    Ok(())
}

/// Journals of runs kept in the data folder; older ones are deleted.
const JOURNALS: usize = 20;

/// A new journal for a run in `logs/` of the data folder, named by the time
/// it starts, after deleting the oldest beyond [`JOURNALS`]. A run goes on
/// without one when the folder cannot be written.
fn journal(app: &tauri::AppHandle) -> Option<std::fs::File> {
    let folder = app.aeria_data_dir().ok()?.join("logs");
    std::fs::create_dir_all(&folder).ok()?;
    prune_journals(&folder, JOURNALS - 1);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    std::fs::File::create(folder.join(format!("translation-{millis}.log"))).ok()
}

/// Deletes the oldest run journals of `folder` so that at most `keep` stay.
/// Their names hold the start time, so name order is age order.
fn prune_journals(folder: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut journals: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "log")
                && path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.starts_with("translation-"))
        })
        .collect();
    journals.sort();
    let excess = journals.len().saturating_sub(keep);
    for old in &journals[..excess] {
        let _ = std::fs::remove_file(old);
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
    pub binding: Option<crate::dto::SourceBindingDto>,
    /// The model's last translation, which was not written.
    pub translation: String,
    pub problems: Vec<crate::dto::IssueDto>,
}

/// The progress of a run, with its rejected strings.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    #[serde(flatten)]
    pub status: Status,
    pub rejections: Vec<RejectionDto>,
}

fn rejection_dto(
    session: Option<&aeria_po::Session>,
    rejected: &aeria_model::Rejected,
) -> RejectionDto {
    let mut problems: Vec<crate::dto::IssueDto> = rejected
        .problems
        .iter()
        .filter(|message| {
            !rejected
                .issues
                .iter()
                .any(|issue| issue.to_string() == **message)
        })
        .map(|message| crate::dto::IssueDto {
            kind: "other".to_owned(),
            group: "other".to_owned(),
            message: message.clone(),
            ..crate::dto::IssueDto::default()
        })
        .collect();
    problems.extend(rejected.issues.iter().map(crate::dto::IssueDto::from));
    RejectionDto {
        path: rejected.path.clone(),
        context: rejected.context.clone(),
        binding: session.and_then(|session| crate::search::binding(session, &rejected.context)),
        translation: rejected.translation.clone(),
        problems,
    }
}

#[tauri::command(rename_all = "camelCase")]
/// The progress of the last run; `None` before one started.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn translation_status(app: tauri::AppHandle) -> Option<StatusDto> {
    let status = app.state::<Translation>().run()?.status();
    let session = app.state::<DesktopState>().session().ok();
    let rejections = status
        .rejections
        .iter()
        .map(|rejected| rejection_dto(session.as_deref(), rejected))
        .collect();
    Some(StatusDto { status, rejections })
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
        },
    )
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
        for millis in [1_000, 3_000, 2_000] {
            std::fs::write(folder.path().join(format!("translation-{millis}.log")), "")
                .expect("journal");
        }
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
        assert_eq!(
            left,
            ["other.log", "translation-2000.log", "translation-3000.log"]
        );
    }
}
