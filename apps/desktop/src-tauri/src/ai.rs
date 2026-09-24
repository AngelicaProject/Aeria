//! Desktop adapter for AI provider settings, API keys, and connection checks.
//!
//! Provider settings are machine-local application data; API keys live only
//! in the OS secret store and never cross the IPC boundary. Settings and
//! secret-store access run in blocking workers; provider requests are async
//! and hold no desktop lock.

use aeria_ai::chatgpt::{AccessToken, DevicePoll, LOGIN_TIMEOUT, VERIFICATION_URL};
use aeria_ai::settings::SETTINGS_FILE_NAME;
use aeria_ai::{
    AiSettings, AiSettingsStore, ApiKey, BaseUrl, HeaderConfig, KeyringSecretStore, ModelConfig,
    ModelSelection, Protocol, ProviderConfig, ProviderEndpoint, ProviderError, ProviderKind,
    ReasoningEffort, SecretStore, presets,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// Local AI settings with presets, for the settings UI.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsDto {
    pub providers: Vec<AiProviderDto>,
    pub agent_model: Option<ModelSelection>,
    /// The model for translation-job workers; Angelica's model when unset.
    pub worker_model: Option<ModelSelection>,
    /// Domains whose pages Angelica reads without asking.
    pub web_domains: Vec<String>,
    pub presets: Vec<AiProviderPresetDto>,
}

/// One configured provider. The key itself is never included.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderDto {
    pub id: String,
    pub kind: ProviderKind,
    pub name: String,
    pub base_url: String,
    pub models: Vec<ModelConfig>,
    pub session_header: Option<String>,
    pub headers: Vec<HeaderConfig>,
    pub api_key: ApiKeyStateDto,
}

/// Whether a provider has a stored key.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApiKeyStateDto {
    Stored,
    Missing,
    /// The OS secret store could not be read.
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderPresetDto {
    pub kind: ProviderKind,
    pub name: String,
    pub base_url: Option<String>,
    pub session_header: Option<String>,
}

/// A provider to create (no `id`) or replace.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiProviderInputDto {
    pub id: Option<String>,
    pub kind: ProviderKind,
    pub name: String,
    pub base_url: String,
    pub models: Vec<ModelConfig>,
    pub session_header: Option<String>,
    pub headers: Vec<HeaderConfig>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConnectionCheckDto {
    pub latency_ms: u64,
    /// The model name the provider reported.
    pub model: Option<String>,
}

pub(crate) fn settings_store(app: &tauri::AppHandle) -> CommandResult<AiSettingsStore> {
    app.path()
        .app_data_dir()
        .map(|path| AiSettingsStore::new(path.join(SETTINGS_FILE_NAME)))
        .map_err(|error| {
            CommandError::new(
                "aiSettingsStorage",
                format!("could not resolve the Aeria app-data directory: {error}"),
            )
        })
}

fn provider_not_found(provider_id: &str) -> CommandError {
    CommandError::new(
        "aiProviderNotFound",
        format!("AI provider {provider_id:?} is not configured"),
    )
}

fn settings_dto(settings: &AiSettings, secrets: &dyn SecretStore) -> AiSettingsDto {
    AiSettingsDto {
        providers: settings
            .providers
            .iter()
            .map(|provider| AiProviderDto {
                id: provider.id.clone(),
                kind: provider.kind,
                name: provider.name.clone(),
                base_url: provider.base_url.clone(),
                models: provider.models.clone(),
                session_header: provider.session_header.clone(),
                headers: provider.headers.clone(),
                api_key: match secrets.get(&provider.id) {
                    Ok(Some(_)) => ApiKeyStateDto::Stored,
                    Ok(None) => ApiKeyStateDto::Missing,
                    Err(_) => ApiKeyStateDto::Unavailable,
                },
            })
            .collect(),
        agent_model: settings.agent_model.clone(),
        worker_model: settings.worker_model.clone(),
        web_domains: settings.web_domains.clone(),
        presets: presets()
            .into_iter()
            .map(|preset| AiProviderPresetDto {
                kind: preset.kind,
                name: preset.name.to_owned(),
                base_url: preset.base_url.map(str::to_owned),
                session_header: preset.session_header.map(str::to_owned),
            })
            .collect(),
    }
}

fn save_provider(
    store: &AiSettingsStore,
    secrets: &dyn SecretStore,
    input: AiProviderInputDto,
) -> CommandResult<AiSettingsDto> {
    let base_url = BaseUrl::parse(&input.base_url)
        .map_err(|message| CommandError::new("aiInvalidSettings", message))?;
    let ((), settings) = store.update(|settings| {
        let id = match input.id {
            Some(id) if settings.provider(&id).is_none() => {
                return Err(aeria_ai::AiSettingsError::Rejected {
                    message: format!("AI provider {id:?} is not configured"),
                });
            }
            Some(id) => id,
            None => ProviderConfig::new_id(),
        };
        let provider = ProviderConfig {
            id,
            kind: input.kind,
            name: input.name.trim().to_owned(),
            base_url: base_url.as_str().to_owned(),
            models: input.models,
            session_header: input
                .session_header
                .map(|name| name.trim().to_ascii_lowercase())
                .filter(|name| !name.is_empty()),
            headers: input
                .headers
                .into_iter()
                .map(|header| HeaderConfig {
                    name: header.name.trim().to_ascii_lowercase(),
                    value: header.value.trim().to_owned(),
                })
                .collect(),
        };
        aeria_ai::settings::validate_provider(&provider)
            .map_err(|message| aeria_ai::AiSettingsError::Rejected { message })?;
        settings.upsert_provider(provider);
        Ok(())
    })?;
    Ok(settings_dto(&settings, secrets))
}

fn remove_provider(
    store: &AiSettingsStore,
    secrets: &dyn SecretStore,
    provider_id: &str,
) -> CommandResult<AiSettingsDto> {
    if store.load()?.provider(provider_id).is_none() {
        return Err(provider_not_found(provider_id));
    }
    // The key goes first: a provider must never be forgotten while its key
    // stays behind in the OS store.
    secrets.delete(provider_id)?;
    let ((), settings) = store.update(|settings| {
        settings.remove_provider(provider_id);
        Ok(())
    })?;
    Ok(settings_dto(&settings, secrets))
}

fn set_api_key(
    store: &AiSettingsStore,
    secrets: &dyn SecretStore,
    provider_id: &str,
    api_key: &str,
) -> CommandResult<AiSettingsDto> {
    let settings = store.load()?;
    match settings.provider(provider_id) {
        None => return Err(provider_not_found(provider_id)),
        Some(provider) if provider.kind == ProviderKind::ChatGpt => {
            return Err(CommandError::new(
                "aiInvalidSettings",
                "a ChatGPT provider signs in with a ChatGPT account instead of an API key",
            ));
        }
        Some(_) => {}
    }
    secrets.set(provider_id, &ApiKey::new(api_key)?)?;
    Ok(settings_dto(&settings, secrets))
}

fn clear_api_key(
    store: &AiSettingsStore,
    secrets: &dyn SecretStore,
    provider_id: &str,
) -> CommandResult<AiSettingsDto> {
    let settings = store.load()?;
    if settings.provider(provider_id).is_none() {
        return Err(provider_not_found(provider_id));
    }
    secrets.delete(provider_id)?;
    Ok(settings_dto(&settings, secrets))
}

fn set_agent_model(
    store: &AiSettingsStore,
    secrets: &dyn SecretStore,
    selection: Option<ModelSelection>,
) -> CommandResult<AiSettingsDto> {
    let ((), settings) = store.update(|settings| {
        settings.agent_model = selection;
        Ok(())
    })?;
    Ok(settings_dto(&settings, secrets))
}

fn set_worker_model(
    store: &AiSettingsStore,
    secrets: &dyn SecretStore,
    selection: Option<ModelSelection>,
) -> CommandResult<AiSettingsDto> {
    let ((), settings) = store.update(|settings| {
        settings.worker_model = selection;
        Ok(())
    })?;
    Ok(settings_dto(&settings, secrets))
}

fn set_web_domains(
    store: &AiSettingsStore,
    secrets: &dyn SecretStore,
    domains: &[String],
) -> CommandResult<AiSettingsDto> {
    let mut normalized = Vec::with_capacity(domains.len());
    for domain in domains.iter().filter(|domain| !domain.trim().is_empty()) {
        let domain = aeria_ai::web::normalize_domain(domain)
            .map_err(|message| CommandError::new("aiInvalidSettings", message))?;
        if !normalized.contains(&domain) {
            normalized.push(domain);
        }
    }
    normalized.sort();
    let ((), settings) = store.update(|settings| {
        settings.web_domains = normalized;
        Ok(())
    })?;
    Ok(settings_dto(&settings, secrets))
}

/// Reads a provider and its stored secret: an API key, or the ChatGPT
/// refresh token for a ChatGPT provider.
pub(crate) fn provider_credentials(
    store: &AiSettingsStore,
    secrets: &dyn SecretStore,
    provider_id: &str,
) -> CommandResult<(ProviderConfig, ApiKey)> {
    let settings = store.load()?;
    let provider = settings
        .provider(provider_id)
        .ok_or_else(|| provider_not_found(provider_id))?
        .clone();
    let secret = secrets
        .get(provider_id)?
        .ok_or_else(|| match provider.kind {
            ProviderKind::ChatGpt => sign_in_required(&provider.name),
            _ => CommandError::new(
                "aiApiKeyMissing",
                format!("no API key is stored for {:?}", provider.name),
            ),
        })?;
    Ok((provider, secret))
}

fn sign_in_required(name: &str) -> CommandError {
    CommandError::new(
        "aiChatGptSignInRequired",
        format!("sign in to ChatGPT again to use {name:?}"),
    )
}

fn endpoint_for_provider(
    provider: &ProviderConfig,
    credential: ApiKey,
    identity: Vec<(String, String)>,
) -> CommandResult<ProviderEndpoint> {
    let base_url = BaseUrl::parse(&provider.base_url)
        .map_err(|message| CommandError::new("aiInvalidSettings", message))?;
    let mut headers: Vec<(String, String)> = provider
        .headers
        .iter()
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect();
    // The ChatGPT identity headers always win over configured ones.
    headers.retain(|(name, _)| !identity.iter().any(|(required, _)| required == name));
    headers.extend(identity);
    Ok(ProviderEndpoint {
        base_url,
        api_key: credential,
        session_header: provider.session_header.clone(),
        headers,
        protocol: provider.kind.protocol(),
    })
}

/// Resolves how to reach a provider now. For ChatGPT this refreshes the
/// short-lived access token when needed.
pub(crate) async fn resolve_endpoint(
    app: &tauri::AppHandle,
    provider_id: String,
) -> CommandResult<ProviderEndpoint> {
    let store = settings_store(app)?;
    let (provider, secret) =
        run_blocking(move || provider_credentials(&store, &KeyringSecretStore, &provider_id))
            .await?;
    match provider.kind.protocol() {
        Protocol::ChatCompletions => endpoint_for_provider(&provider, secret, Vec::new()),
        Protocol::CodexResponses => {
            let access = chatgpt_access(app, &provider, secret).await?;
            let identity = access.headers();
            endpoint_for_provider(&provider, access.token, identity)
        }
    }
}

/// Returns a fresh ChatGPT access token, refreshing it under the token lock
/// so concurrent requests never replay a rotated refresh token.
async fn chatgpt_access(
    app: &tauri::AppHandle,
    provider: &ProviderConfig,
    refresh_token: ApiKey,
) -> CommandResult<AccessToken> {
    let state = app.state::<DesktopState>();
    let mut tokens = state.chatgpt_tokens().lock().await;
    if let Some((_, access)) = tokens.iter().find(|(id, _)| *id == provider.id)
        && access.is_fresh()
    {
        return Ok(access.clone());
    }
    let refreshed = state
        .ai_client()?
        .chatgpt_refresh(&refresh_token)
        .await
        .map_err(|error| match error {
            ProviderError::Unauthorized { .. } => sign_in_required(&provider.name),
            other => other.into(),
        })?;
    if let Some(rotated) = refreshed.refresh_token {
        let id = provider.id.clone();
        run_blocking(move || Ok(KeyringSecretStore.set(&id, &rotated)?)).await?;
    }
    tokens.retain(|(id, _)| *id != provider.id);
    tokens.push((provider.id.clone(), refreshed.access.clone()));
    Ok(refreshed.access)
}

async fn endpoint_for(
    app: &tauri::AppHandle,
    provider_id: String,
) -> CommandResult<ProviderEndpoint> {
    resolve_endpoint(app, provider_id).await
}

async fn with_settings<F>(app: &tauri::AppHandle, operation: F) -> CommandResult<AiSettingsDto>
where
    F: FnOnce(&AiSettingsStore, &dyn SecretStore) -> CommandResult<AiSettingsDto> + Send + 'static,
{
    let store = settings_store(app)?;
    run_blocking(move || operation(&store, &KeyringSecretStore)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns local AI provider settings, key presence, and presets.
///
/// # Errors
///
/// Returns a typed error when the settings cannot be read.
pub async fn ai_settings(app: tauri::AppHandle) -> CommandResult<AiSettingsDto> {
    with_settings(&app, |store, secrets| {
        Ok(settings_dto(&store.load()?, secrets))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Creates or replaces a provider configuration.
///
/// # Errors
///
/// Returns `aiInvalidSettings` for an invalid provider, or a storage error.
pub async fn ai_save_provider(
    app: tauri::AppHandle,
    provider: AiProviderInputDto,
) -> CommandResult<AiSettingsDto> {
    with_settings(&app, move |store, secrets| {
        save_provider(store, secrets, provider)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Removes a provider and its stored API key.
///
/// # Errors
///
/// Returns `aiProviderNotFound`, a secret-store error, or a storage error.
pub async fn ai_remove_provider(
    app: tauri::AppHandle,
    provider_id: String,
) -> CommandResult<AiSettingsDto> {
    forget_chatgpt_access(&app, &provider_id).await;
    let id = provider_id.clone();
    with_settings(&app, move |store, secrets| {
        remove_provider(store, secrets, &id)
    })
    .await
}

async fn forget_chatgpt_access(app: &tauri::AppHandle, provider_id: &str) {
    app.state::<DesktopState>()
        .chatgpt_tokens()
        .lock()
        .await
        .retain(|(id, _)| id != provider_id);
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Stores or replaces a provider's API key in the OS secret store.
///
/// # Errors
///
/// Returns `aiProviderNotFound`, `aiInvalidApiKey`, or a secret-store error.
pub async fn ai_set_api_key(
    app: tauri::AppHandle,
    provider_id: String,
    api_key: String,
) -> CommandResult<AiSettingsDto> {
    with_settings(&app, move |store, secrets| {
        set_api_key(store, secrets, &provider_id, &api_key)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Removes a provider's API key from the OS secret store.
///
/// # Errors
///
/// Returns `aiProviderNotFound` or a secret-store error.
pub async fn ai_clear_api_key(
    app: tauri::AppHandle,
    provider_id: String,
) -> CommandResult<AiSettingsDto> {
    forget_chatgpt_access(&app, &provider_id).await;
    let id = provider_id.clone();
    with_settings(&app, move |store, secrets| {
        clear_api_key(store, secrets, &id)
    })
    .await
}

/// A started ChatGPT sign-in.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptLoginDto {
    pub login_id: String,
    pub user_code: String,
    pub verification_url: String,
    /// Whether the sign-in page was opened in the default browser.
    pub browser_opened: bool,
}

/// The end of a ChatGPT sign-in, sent as `ai://chatgpt-login`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptLoginEventDto {
    pub login_id: String,
    pub provider_id: String,
    pub succeeded: bool,
    pub code: Option<String>,
    pub message: Option<String>,
}

/// Renderer event for the end of a ChatGPT sign-in.
pub const CHATGPT_LOGIN_EVENT: &str = "ai://chatgpt-login";

async fn finish_chatgpt_login(
    app: &tauri::AppHandle,
    provider: &ProviderConfig,
    login: &aeria_ai::chatgpt::DeviceLogin,
) -> CommandResult<()> {
    let client = app.state::<DesktopState>().ai_client()?;
    let deadline = std::time::Instant::now() + LOGIN_TIMEOUT;
    let (authorization_code, code_verifier) = loop {
        if std::time::Instant::now() >= deadline {
            return Err(CommandError::new(
                "aiChatGptLoginExpired",
                "the sign-in code expired; start again",
            ));
        }
        tokio::time::sleep(login.interval).await;
        match client.chatgpt_poll_login(login).await? {
            DevicePoll::Pending => {}
            DevicePoll::Authorized {
                authorization_code,
                code_verifier,
            } => break (authorization_code, code_verifier),
        }
    };
    let tokens = client
        .chatgpt_exchange(&authorization_code, &code_verifier)
        .await?;
    let refresh_token = tokens.refresh_token.ok_or_else(|| {
        CommandError::new(
            "aiInvalidResponse",
            "ChatGPT did not return a refresh token",
        )
    })?;
    let id = provider.id.clone();
    run_blocking(move || Ok(KeyringSecretStore.set(&id, &refresh_token)?)).await?;
    let state = app.state::<DesktopState>();
    let mut cached = state.chatgpt_tokens().lock().await;
    cached.retain(|(id, _)| *id != provider.id);
    cached.push((provider.id.clone(), tokens.access));
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
/// Starts a ChatGPT device sign-in for a ChatGPT provider and opens the
/// sign-in page. The result arrives as an `ai://chatgpt-login` event; a new
/// sign-in replaces one still waiting.
///
/// # Errors
///
/// Returns `aiProviderNotFound`, `aiInvalidSettings` for another provider
/// kind, or a classified provider error.
pub async fn ai_chatgpt_login_start(
    app: tauri::AppHandle,
    provider_id: String,
) -> CommandResult<ChatGptLoginDto> {
    use tauri_plugin_opener::OpenerExt;

    let store = settings_store(&app)?;
    let id = provider_id.clone();
    let provider = run_blocking(move || {
        store
            .load()?
            .provider(&id)
            .cloned()
            .ok_or_else(|| provider_not_found(&id))
    })
    .await?;
    if provider.kind != ProviderKind::ChatGpt {
        return Err(CommandError::new(
            "aiInvalidSettings",
            "only a ChatGPT provider signs in with a ChatGPT account",
        ));
    }
    let login = app
        .state::<DesktopState>()
        .ai_client()?
        .chatgpt_start_login()
        .await?;
    let login_id = ProviderConfig::new_id();
    let browser_opened = app
        .opener()
        .open_url(VERIFICATION_URL, None::<&str>)
        .is_ok();
    let response = ChatGptLoginDto {
        login_id: login_id.clone(),
        user_code: login.user_code.clone(),
        verification_url: VERIFICATION_URL.to_owned(),
        browser_opened,
    };
    let task_app = app.clone();
    let task_login_id = login_id.clone();
    let task = async move {
        let result = finish_chatgpt_login(&task_app, &provider, &login).await;
        let (code, message) = match result {
            Ok(()) => (None, None),
            Err(error) => (Some(error.code), Some(error.message)),
        };
        task_app
            .state::<DesktopState>()
            .finish_chatgpt_login(&task_login_id);
        let _ = task_app.emit(
            CHATGPT_LOGIN_EVENT,
            ChatGptLoginEventDto {
                login_id: task_login_id,
                provider_id: provider.id,
                succeeded: code.is_none(),
                code,
                message,
            },
        );
    };
    app.state::<DesktopState>()
        .start_chatgpt_login(login_id, || tauri::async_runtime::spawn(task));
    Ok(response)
}

#[tauri::command(rename_all = "camelCase")]
/// Stops waiting for a ChatGPT sign-in.
///
/// # Errors
///
/// Never fails; cancelling a finished sign-in does nothing.
pub async fn ai_chatgpt_login_cancel(app: tauri::AppHandle, login_id: String) -> CommandResult<()> {
    app.state::<DesktopState>().cancel_chatgpt_login(&login_id);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
/// Sets or clears the default model and effort for Angelica.
///
/// # Errors
///
/// Returns `aiInvalidSettings` when the selection does not match a
/// configured model and effort.
pub async fn ai_set_agent_model(
    app: tauri::AppHandle,
    selection: Option<ModelSelection>,
) -> CommandResult<AiSettingsDto> {
    with_settings(&app, move |store, secrets| {
        set_agent_model(store, secrets, selection)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Sets or clears the model and effort for translation-job workers.
///
/// # Errors
///
/// Returns `aiInvalidSettings` when the selection does not match a
/// configured model and effort.
pub async fn ai_set_worker_model(
    app: tauri::AppHandle,
    selection: Option<ModelSelection>,
) -> CommandResult<AiSettingsDto> {
    with_settings(&app, move |store, secrets| {
        set_worker_model(store, secrets, selection)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Replaces the domains whose pages Angelica reads without asking. Entries
/// may be domains or links; blank entries and repeats are dropped.
///
/// # Errors
///
/// Returns `aiInvalidSettings` for an invalid domain or too many domains.
pub async fn ai_set_web_domains(
    app: tauri::AppHandle,
    domains: Vec<String>,
) -> CommandResult<AiSettingsDto> {
    with_settings(&app, move |store, secrets| {
        set_web_domains(store, secrets, &domains)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Lists the model IDs a provider reports.
///
/// # Errors
///
/// Returns a settings, key, or classified provider error.
pub async fn ai_list_remote_models(
    app: tauri::AppHandle,
    provider_id: String,
) -> CommandResult<Vec<ModelConfig>> {
    let endpoint = endpoint_for(&app, provider_id).await?;
    let client = app.state::<DesktopState>().ai_client()?;
    Ok(client.list_models(&endpoint).await?)
}

#[tauri::command(rename_all = "camelCase")]
/// Sends one minimal request to check a provider, model, and effort.
///
/// The model and effort need not be saved yet, so a model can be probed
/// before it is added or an effort enabled.
///
/// # Errors
///
/// Returns a settings, key, or classified provider error.
pub async fn ai_test_connection(
    app: tauri::AppHandle,
    provider_id: String,
    model_id: String,
    effort: Option<ReasoningEffort>,
) -> CommandResult<AiConnectionCheckDto> {
    let model_id = model_id.trim().to_owned();
    if model_id.is_empty() {
        return Err(CommandError::new(
            "aiInvalidSettings",
            "model id must not be empty",
        ));
    }
    let endpoint = endpoint_for(&app, provider_id).await?;
    let client = app.state::<DesktopState>().ai_client()?;
    let check = client.check_model(&endpoint, &model_id, effort).await?;
    Ok(AiConnectionCheckDto {
        latency_ms: u64::try_from(check.latency.as_millis()).unwrap_or(u64::MAX),
        model: check.model,
    })
}

#[cfg(test)]
mod tests {
    use aeria_ai::MemorySecretStore;

    use super::*;

    fn store() -> (tempfile::TempDir, AiSettingsStore) {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = AiSettingsStore::new(directory.path().join(SETTINGS_FILE_NAME));
        (directory, store)
    }

    fn input(id: Option<String>) -> AiProviderInputDto {
        AiProviderInputDto {
            id,
            kind: ProviderKind::OpenCodeGo,
            name: " OpenCode Go ".to_owned(),
            base_url: "https://opencode.ai/zen/go/v1/".to_owned(),
            models: vec![ModelConfig::new("glm-5.3")],
            session_header: Some(" X-OpenCode-Session ".to_owned()),
            headers: vec![HeaderConfig {
                name: "X-Client".to_owned(),
                value: " aeria ".to_owned(),
            }],
        }
    }

    #[test]
    fn new_provider_gets_an_id_and_normalized_fields() {
        let (_directory, store) = store();
        let secrets = MemorySecretStore::default();
        let settings = save_provider(&store, &secrets, input(None)).expect("save");
        let provider = &settings.providers[0];
        assert!(!provider.id.is_empty());
        assert_eq!(provider.name, "OpenCode Go");
        assert_eq!(provider.base_url, "https://opencode.ai/zen/go/v1");
        assert_eq!(provider.api_key, ApiKeyStateDto::Missing);
        assert_eq!(
            provider.session_header.as_deref(),
            Some("x-opencode-session")
        );
        assert_eq!(provider.headers[0].name, "x-client");
        assert_eq!(provider.headers[0].value, "aeria");
        assert_eq!(settings.presets[0].kind, ProviderKind::OpenCodeGo);
    }

    #[test]
    fn saving_an_unknown_provider_id_is_rejected() {
        let (_directory, store) = store();
        let secrets = MemorySecretStore::default();
        let error = save_provider(&store, &secrets, input(Some("missing".to_owned())))
            .expect_err("unknown id");
        assert_eq!(error.code, "aiInvalidSettings");
        assert!(store.load().expect("load").providers.is_empty());
    }

    #[test]
    fn keys_are_reported_only_as_state_and_removed_with_the_provider() {
        let (_directory, store) = store();
        let secrets = MemorySecretStore::default();
        let id = save_provider(&store, &secrets, input(None))
            .expect("save")
            .providers[0]
            .id
            .clone();

        let settings = set_api_key(&store, &secrets, &id, " sk-secret ").expect("set key");
        assert_eq!(settings.providers[0].api_key, ApiKeyStateDto::Stored);
        let json = serde_json::to_string(&settings).expect("dto json");
        assert!(!json.contains("sk-secret"));
        let (provider, secret) = provider_credentials(&store, &secrets, &id).expect("credentials");
        let endpoint = endpoint_for_provider(&provider, secret, Vec::new()).expect("endpoint");
        assert_eq!(endpoint.api_key.expose(), "sk-secret");
        assert_eq!(
            endpoint.session_header.as_deref(),
            Some("x-opencode-session")
        );
        assert_eq!(
            endpoint.headers,
            vec![("x-client".to_owned(), "aeria".to_owned())]
        );

        remove_provider(&store, &secrets, &id).expect("remove");
        assert_eq!(secrets.get(&id).expect("get"), None);
        assert_eq!(
            remove_provider(&store, &secrets, &id)
                .expect_err("already removed")
                .code,
            "aiProviderNotFound"
        );
    }

    #[test]
    fn chatgpt_identity_headers_replace_configured_ones_and_keys_are_refused() {
        let (_directory, store) = store();
        let secrets = MemorySecretStore::default();
        let settings = save_provider(
            &store,
            &secrets,
            AiProviderInputDto {
                id: None,
                kind: ProviderKind::ChatGpt,
                name: "ChatGPT".to_owned(),
                base_url: aeria_ai::CHATGPT_CODEX_BASE_URL.to_owned(),
                models: Vec::new(),
                session_header: None,
                headers: vec![HeaderConfig {
                    name: "originator".to_owned(),
                    value: "spoofed".to_owned(),
                }],
            },
        )
        .expect("save");
        let provider = store.load().expect("load").providers.remove(0);
        assert_eq!(
            set_api_key(&store, &secrets, &settings.providers[0].id, "sk")
                .expect_err("keys are refused")
                .code,
            "aiInvalidSettings"
        );
        assert_eq!(
            provider_credentials(&store, &secrets, &provider.id)
                .expect_err("not signed in")
                .code,
            "aiChatGptSignInRequired"
        );
        let endpoint = endpoint_for_provider(
            &provider,
            ApiKey::new("access").expect("token"),
            vec![("originator".to_owned(), "aeria".to_owned())],
        )
        .expect("endpoint");
        assert_eq!(endpoint.protocol, Protocol::CodexResponses);
        assert_eq!(
            endpoint.headers,
            vec![("originator".to_owned(), "aeria".to_owned())]
        );
    }

    #[test]
    fn endpoint_requires_a_stored_key() {
        let (_directory, store) = store();
        let secrets = MemorySecretStore::default();
        let id = save_provider(&store, &secrets, input(None))
            .expect("save")
            .providers[0]
            .id
            .clone();
        assert_eq!(
            provider_credentials(&store, &secrets, &id)
                .expect_err("no key")
                .code,
            "aiApiKeyMissing"
        );
        assert_eq!(
            set_api_key(&store, &secrets, &id, "   ")
                .expect_err("empty key")
                .code,
            "aiInvalidApiKey"
        );
        set_api_key(&store, &secrets, &id, "sk").expect("set");
        clear_api_key(&store, &secrets, &id).expect("clear");
        assert_eq!(secrets.get(&id).expect("get"), None);
    }

    #[test]
    fn agent_model_must_match_a_configured_model_and_effort() {
        let (_directory, store) = store();
        let secrets = MemorySecretStore::default();
        let id = save_provider(&store, &secrets, input(None))
            .expect("save")
            .providers[0]
            .id
            .clone();
        let error = set_agent_model(
            &store,
            &secrets,
            Some(ModelSelection {
                provider_id: id.clone(),
                model_id: "glm-5.3".to_owned(),
                effort: Some(ReasoningEffort::High),
            }),
        )
        .expect_err("effort is not enabled for the model");
        assert_eq!(error.code, "aiInvalidSettings");

        let settings = set_agent_model(
            &store,
            &secrets,
            Some(ModelSelection {
                provider_id: id,
                model_id: "glm-5.3".to_owned(),
                effort: None,
            }),
        )
        .expect("valid selection");
        assert!(settings.agent_model.is_some());
        assert!(settings.worker_model.is_none());
    }

    #[test]
    fn web_domains_are_normalized_sorted_and_checked() {
        let (_directory, store) = store();
        let secrets = MemorySecretStore::default();
        let settings = set_web_domains(
            &store,
            &secrets,
            &[
                "https://FFXIV.gamerescape.com/wiki".to_owned(),
                "  ".to_owned(),
                "consolegameswiki.com".to_owned(),
                "ffxiv.gamerescape.com".to_owned(),
            ],
        )
        .expect("valid");
        assert_eq!(
            settings.web_domains,
            ["consolegameswiki.com", "ffxiv.gamerescape.com"]
        );
        assert_eq!(
            set_web_domains(&store, &secrets, &["bad domain".to_owned()])
                .expect_err("invalid")
                .code,
            "aiInvalidSettings"
        );
    }

    #[test]
    fn worker_model_is_validated_like_the_agent_model() {
        let (_directory, store) = store();
        let secrets = MemorySecretStore::default();
        let id = save_provider(&store, &secrets, input(None))
            .expect("save")
            .providers[0]
            .id
            .clone();
        let selection = |model_id: &str| ModelSelection {
            provider_id: id.clone(),
            model_id: model_id.to_owned(),
            effort: None,
        };
        assert_eq!(
            set_worker_model(&store, &secrets, Some(selection("missing")))
                .expect_err("unknown model")
                .code,
            "aiInvalidSettings"
        );
        let settings =
            set_worker_model(&store, &secrets, Some(selection("glm-5.3"))).expect("valid");
        assert_eq!(settings.worker_model, Some(selection("glm-5.3")));
        let settings = set_worker_model(&store, &secrets, None).expect("clear");
        assert!(settings.worker_model.is_none());
    }
}
