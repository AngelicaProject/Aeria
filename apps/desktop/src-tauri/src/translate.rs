//! Machine translation in the desktop: signing in with a ChatGPT
//! subscription and one run at a time over files of the open project (see
//! `docs/architecture/translate.md`).

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use aeria_model::auth::{DeviceLogin, VERIFICATION_URL};
use aeria_model::{Codex, KeyringStore, ModelError, ModelInfo, Options, Run, Status};
use serde::Serialize;
use tauri::Manager;

use crate::error::CommandError;
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
    let translation = app.state::<Translation>();
    let codex = translation.codex()?;
    let session = app.state::<DesktopState>().session()?;
    let run = {
        let mut current = translation
            .run
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if current.as_ref().is_some_and(|run| run.status().running) {
            return Err(CommandError::new(
                "translationRunning",
                "a machine translation is running; stop it or wait for it to finish",
            ));
        }
        let run = Arc::new(Run::new());
        *current = Some(Arc::clone(&run));
        run
    };
    let options = Options {
        paths: paths_of(&session, &scope),
        fuzzy,
        model,
        effort: effort.filter(|effort| !effort.is_empty()),
    };
    tauri::async_runtime::spawn(aeria_model::run::run(session, codex, options, run));
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
/// The progress of the last run; `None` before one started.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn translation_status(app: tauri::AppHandle) -> Option<Status> {
    app.state::<Translation>().run().map(|run| run.status())
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
