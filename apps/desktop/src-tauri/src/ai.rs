//! Desktop adapter for AI provider settings, API keys, and connection checks.
//!
//! Provider settings are machine-local application data; API keys live only
//! in the OS secret store and never cross the IPC boundary. Settings and
//! secret-store access run in blocking workers; provider requests are async
//! and hold no desktop lock.

use aeria_ai::settings::SETTINGS_FILE_NAME;
use aeria_ai::{
    AiSettings, AiSettingsStore, ApiKey, BaseUrl, HeaderConfig, KeyringSecretStore, ModelConfig,
    ModelSelection, ProviderConfig, ProviderEndpoint, ProviderKind, ReasoningEffort, SecretStore,
    presets,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;

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
    if settings.provider(provider_id).is_none() {
        return Err(provider_not_found(provider_id));
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

pub(crate) fn provider_endpoint(
    store: &AiSettingsStore,
    secrets: &dyn SecretStore,
    provider_id: &str,
) -> CommandResult<ProviderEndpoint> {
    let settings = store.load()?;
    let provider = settings
        .provider(provider_id)
        .ok_or_else(|| provider_not_found(provider_id))?;
    let base_url = BaseUrl::parse(&provider.base_url)
        .map_err(|message| CommandError::new("aiInvalidSettings", message))?;
    let api_key = secrets.get(provider_id)?.ok_or_else(|| {
        CommandError::new(
            "aiApiKeyMissing",
            format!("no API key is stored for {:?}", provider.name),
        )
    })?;
    Ok(ProviderEndpoint {
        base_url,
        api_key,
        session_header: provider.session_header.clone(),
        headers: provider
            .headers
            .iter()
            .map(|header| (header.name.clone(), header.value.clone()))
            .collect(),
    })
}

async fn endpoint_for(
    app: &tauri::AppHandle,
    provider_id: String,
) -> CommandResult<ProviderEndpoint> {
    let store = settings_store(app)?;
    run_blocking(move || provider_endpoint(&store, &KeyringSecretStore, &provider_id)).await
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
    with_settings(&app, move |store, secrets| {
        remove_provider(store, secrets, &provider_id)
    })
    .await
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
    with_settings(&app, move |store, secrets| {
        clear_api_key(store, secrets, &provider_id)
    })
    .await
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
/// Lists the model IDs a provider reports.
///
/// # Errors
///
/// Returns a settings, key, or classified provider error.
pub async fn ai_list_remote_models(
    app: tauri::AppHandle,
    provider_id: String,
) -> CommandResult<Vec<String>> {
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
        let endpoint = provider_endpoint(&store, &secrets, &id).expect("endpoint");
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
    fn endpoint_requires_a_stored_key() {
        let (_directory, store) = store();
        let secrets = MemorySecretStore::default();
        let id = save_provider(&store, &secrets, input(None))
            .expect("save")
            .providers[0]
            .id
            .clone();
        assert_eq!(
            provider_endpoint(&store, &secrets, &id)
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
    }
}
