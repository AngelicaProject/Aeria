//! Local, machine-only AI provider settings.
//!
//! Settings live in application data and are never written to a project
//! repository. API keys are not part of this document; see
//! [`crate::secrets`].

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::provider::{
    BaseUrl, CHATGPT_CODEX_BASE_URL, MAX_EXTRA_HEADERS, MAX_NAME_CHARS, ModelConfig,
    ProviderConfig, ProviderKind, ReasoningEffort, validate_header_name, validate_header_value,
};

pub const FORMAT_VERSION: u32 = 1;
pub const MAX_SETTINGS_FILE_BYTES: u64 = 256 * 1024;
pub const MAX_PROVIDERS: usize = 32;
pub const MAX_MODELS_PER_PROVIDER: usize = 256;

/// The filename used by the desktop application under its app-data directory.
pub const SETTINGS_FILE_NAME: &str = "ai-settings-v1.json";

const PARTIAL_FILE_NAME: &str = "ai-settings-v1.json.partial";
const PREVIOUS_FILE_NAME: &str = "ai-settings-v1.json.previous";
const LOCK_FILE_NAME: &str = "ai-settings-v1.json.lock";

/// A provider model and optional effort chosen as a default.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSelection {
    pub provider_id: String,
    pub model_id: String,
    pub effort: Option<ReasoningEffort>,
}

/// The complete versioned settings document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiSettings {
    pub format_version: u32,
    pub providers: Vec<ProviderConfig>,
    /// The default model for Angelica's conversations.
    pub agent_model: Option<ModelSelection>,
    /// The model for translation-job workers; Angelica's model when unset.
    #[serde(default)]
    pub worker_model: Option<ModelSelection>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            providers: Vec::new(),
            agent_model: None,
            worker_model: None,
        }
    }
}

impl AiSettings {
    /// Returns the provider with the given ID.
    #[must_use]
    pub fn provider(&self, provider_id: &str) -> Option<&ProviderConfig> {
        self.providers
            .iter()
            .find(|provider| provider.id == provider_id)
    }

    /// Adds a provider or replaces the provider with the same ID.
    ///
    /// A default model selection that the replacement no longer supports is
    /// cleared rather than left dangling.
    pub fn upsert_provider(&mut self, provider: ProviderConfig) {
        match self
            .providers
            .iter_mut()
            .find(|existing| existing.id == provider.id)
        {
            Some(existing) => *existing = provider,
            None => self.providers.push(provider),
        }
        self.drop_unsupported_selection();
    }

    /// Removes a provider and any default selection that uses it.
    ///
    /// Returns whether the provider existed.
    pub fn remove_provider(&mut self, provider_id: &str) -> bool {
        let before = self.providers.len();
        self.providers.retain(|provider| provider.id != provider_id);
        self.drop_unsupported_selection();
        self.providers.len() != before
    }

    fn drop_unsupported_selection(&mut self) {
        if self
            .agent_model
            .as_ref()
            .is_some_and(|selection| self.selection_error(selection).is_some())
        {
            self.agent_model = None;
        }
        if self
            .worker_model
            .as_ref()
            .is_some_and(|selection| self.selection_error(selection).is_some())
        {
            self.worker_model = None;
        }
    }

    fn selection_error(&self, selection: &ModelSelection) -> Option<String> {
        let Some(provider) = self.provider(&selection.provider_id) else {
            return Some(format!(
                "provider {:?} does not exist",
                selection.provider_id
            ));
        };
        let Some(model) = provider.model(&selection.model_id) else {
            return Some(format!(
                "model {:?} is not configured for provider {:?}",
                selection.model_id, provider.name
            ));
        };
        match selection.effort {
            Some(effort) if !model.reasoning_efforts.contains(&effort) => Some(format!(
                "model {:?} does not accept effort {:?}",
                model.id,
                effort.as_str()
            )),
            _ => None,
        }
    }

    /// Returns the configured model a selection names, when the selection
    /// matches a configured provider, model, and accepted effort.
    ///
    /// # Errors
    ///
    /// Returns a description of the mismatch.
    pub fn selected_model(&self, selection: &ModelSelection) -> Result<&ModelConfig, String> {
        if let Some(message) = self.selection_error(selection) {
            return Err(message);
        }
        self.provider(&selection.provider_id)
            .and_then(|provider| provider.model(&selection.model_id))
            .ok_or_else(|| "the selected model is not configured".to_owned())
    }

    /// Checks every settings invariant.
    ///
    /// # Errors
    ///
    /// Returns a description of the first violated rule.
    pub fn validate(&self) -> Result<(), String> {
        if self.format_version != FORMAT_VERSION {
            return Err(format!(
                "formatVersion must be {FORMAT_VERSION}, got {}",
                self.format_version
            ));
        }
        if self.providers.len() > MAX_PROVIDERS {
            return Err(format!("at most {MAX_PROVIDERS} providers are allowed"));
        }
        let mut provider_ids = HashSet::with_capacity(self.providers.len());
        for provider in &self.providers {
            validate_provider(provider)?;
            if !provider_ids.insert(provider.id.as_str()) {
                return Err(format!("duplicate provider id {:?}", provider.id));
            }
        }
        if let Some(selection) = &self.agent_model
            && let Some(message) = self.selection_error(selection)
        {
            return Err(format!("agentModel is invalid: {message}"));
        }
        if let Some(selection) = &self.worker_model
            && let Some(message) = self.selection_error(selection)
        {
            return Err(format!("workerModel is invalid: {message}"));
        }
        Ok(())
    }
}

/// Checks the invariants of one provider configuration.
///
/// # Errors
///
/// Returns a description of the first violated rule.
pub fn validate_provider(provider: &ProviderConfig) -> Result<(), String> {
    validate_name("provider id", &provider.id)?;
    validate_name("provider name", &provider.name)?;
    BaseUrl::parse(&provider.base_url)?;
    if provider.kind == ProviderKind::ChatGpt
        && (provider.base_url != CHATGPT_CODEX_BASE_URL || provider.session_header.is_some())
    {
        return Err(format!(
            "a ChatGPT provider uses {CHATGPT_CODEX_BASE_URL} and its own session header"
        ));
    }
    if provider.base_url != provider.base_url.trim() || provider.base_url.ends_with('/') {
        return Err("base URL must be stored in normalized form".to_owned());
    }
    if provider.models.len() > MAX_MODELS_PER_PROVIDER {
        return Err(format!(
            "at most {MAX_MODELS_PER_PROVIDER} models are allowed per provider"
        ));
    }
    if let Some(header) = &provider.session_header {
        validate_header_name(header)?;
    }
    if provider.headers.len() > MAX_EXTRA_HEADERS {
        return Err(format!(
            "at most {MAX_EXTRA_HEADERS} extra headers are allowed"
        ));
    }
    let mut header_names = HashSet::with_capacity(provider.headers.len());
    for header in &provider.headers {
        validate_header_name(&header.name)?;
        validate_header_value(&header.value)?;
        if !header_names.insert(header.name.as_str())
            || provider.session_header.as_deref() == Some(header.name.as_str())
        {
            return Err(format!("header {:?} is configured twice", header.name));
        }
    }
    let mut model_ids = HashSet::with_capacity(provider.models.len());
    for model in &provider.models {
        validate_name("model id", &model.id)?;
        if !model_ids.insert(model.id.as_str()) {
            return Err(format!("duplicate model id {:?}", model.id));
        }
        if model.context_window == Some(0) {
            return Err(format!(
                "model {:?} context window must be positive",
                model.id
            ));
        }
        let mut efforts = HashSet::with_capacity(model.reasoning_efforts.len());
        if !model
            .reasoning_efforts
            .iter()
            .all(|effort| efforts.insert(*effort))
        {
            return Err(format!("model {:?} lists an effort twice", model.id));
        }
    }
    Ok(())
}

fn validate_name(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value != value.trim() {
        return Err(format!("{label} must not start or end with whitespace"));
    }
    if value.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "{label} must be at most {MAX_NAME_CHARS} characters"
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{label} must not contain control characters"));
    }
    Ok(())
}

/// Errors raised while loading, validating, or publishing settings.
#[derive(Debug, Error)]
pub enum AiSettingsError {
    #[error("AI settings filesystem operation '{operation}' failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },

    #[error("invalid AI settings JSON at {path}: {message}")]
    InvalidJson { path: PathBuf, message: String },

    #[error("unsupported AI settings format version {version} in {path}")]
    UnsupportedVersion { path: PathBuf, version: u64 },

    #[error("invalid AI settings at {path}: {message}")]
    InvalidData { path: PathBuf, message: String },

    #[error("failed to serialize AI settings for {path}: {source}")]
    Serialization {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("failed to publish AI settings {path}: {source}")]
    AtomicPublication { path: PathBuf, source: io::Error },

    #[error("{message}")]
    Rejected { message: String },
}

/// A filesystem-backed settings document shared by all Aeria processes.
///
/// Every operation holds an exclusive OS file lock, and writes publish a
/// synced temporary file atomically, as the recent-project registry does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiSettingsStore {
    path: PathBuf,
}

struct SettingsLock {
    file: File,
}

impl Drop for SettingsLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl AiSettingsStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the settings. Missing settings are the empty first-run state.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the document is unreadable, malformed,
    /// oversized, of an unsupported version, or violates an invariant. Such a
    /// document is never replaced with defaults.
    pub fn load(&self) -> Result<AiSettings, AiSettingsError> {
        let _lock = self.acquire_lock()?;
        self.load_locked()
    }

    /// Applies a change and publishes the result.
    ///
    /// The change runs on the current document under the lock. The document
    /// is validated before publication; nothing is written when the change or
    /// validation fails.
    ///
    /// # Errors
    ///
    /// Returns the change's error, a validation error as
    /// [`AiSettingsError::Rejected`], or a load or publication error.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut AiSettings) -> Result<T, AiSettingsError>,
    ) -> Result<(T, AiSettings), AiSettingsError> {
        let _lock = self.acquire_lock()?;
        let mut settings = self.load_locked()?;
        let value = change(&mut settings)?;
        settings
            .validate()
            .map_err(|message| AiSettingsError::Rejected { message })?;
        self.write_document(&settings)?;
        Ok((value, settings))
    }

    fn load_locked(&self) -> Result<AiSettings, AiSettingsError> {
        remove_owned_file(&self.partial_path())?;
        if self.path.exists() {
            return read_document(&self.path);
        }
        let previous = self.previous_path();
        if previous.exists() {
            let settings = read_document(&previous)?;
            fs::rename(&previous, &self.path).map_err(|source| {
                AiSettingsError::AtomicPublication {
                    path: self.path.clone(),
                    source,
                }
            })?;
            return Ok(settings);
        }
        Ok(AiSettings::default())
    }

    fn write_document(&self, settings: &AiSettings) -> Result<(), AiSettingsError> {
        let mut bytes = serde_json::to_vec_pretty(settings).map_err(|source| {
            AiSettingsError::Serialization {
                path: self.path.clone(),
                source,
            }
        })?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_SETTINGS_FILE_BYTES {
            return Err(AiSettingsError::Rejected {
                message: format!(
                    "AI settings would exceed the {MAX_SETTINGS_FILE_BYTES}-byte limit"
                ),
            });
        }
        let partial = self.partial_path();
        remove_owned_file(&partial)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|source| io_error("create settings temporary file", &partial, source))?;
        file.write_all(&bytes)
            .map_err(|source| io_error("write settings temporary file", &partial, source))?;
        file.sync_all()
            .map_err(|source| io_error("sync settings temporary file", &partial, source))?;
        drop(file);
        publish(&partial, &self.path, &self.previous_path())
    }

    fn partial_path(&self) -> PathBuf {
        sibling(&self.path, PARTIAL_FILE_NAME)
    }

    fn previous_path(&self) -> PathBuf {
        sibling(&self.path, PREVIOUS_FILE_NAME)
    }

    fn acquire_lock(&self) -> Result<SettingsLock, AiSettingsError> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|source| io_error("create settings directory", parent, source))?;
        let path = sibling(&self.path, LOCK_FILE_NAME);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| io_error("open settings lock file", &path, source))?;
        file.lock_exclusive()
            .map_err(|source| io_error("lock settings file", &path, source))?;
        Ok(SettingsLock { file })
    }
}

fn read_document(path: &Path) -> Result<AiSettings, AiSettingsError> {
    let file = File::open(path).map_err(|source| io_error("read settings file", path, source))?;
    let mut contents = Vec::new();
    file.take(MAX_SETTINGS_FILE_BYTES + 1)
        .read_to_end(&mut contents)
        .map_err(|source| io_error("read settings file", path, source))?;
    if contents.len() as u64 > MAX_SETTINGS_FILE_BYTES {
        return Err(AiSettingsError::InvalidData {
            path: path.to_owned(),
            message: format!("settings file exceeds the {MAX_SETTINGS_FILE_BYTES}-byte limit"),
        });
    }
    let value: Value =
        serde_json::from_slice(&contents).map_err(|source| AiSettingsError::InvalidJson {
            path: path.to_owned(),
            message: source.to_string(),
        })?;
    let version = value
        .get("formatVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| AiSettingsError::InvalidData {
            path: path.to_owned(),
            message: "formatVersion must be an unsigned integer".to_owned(),
        })?;
    if version != u64::from(FORMAT_VERSION) {
        return Err(AiSettingsError::UnsupportedVersion {
            path: path.to_owned(),
            version,
        });
    }
    let settings = serde_json::from_value::<AiSettings>(value).map_err(|source| {
        AiSettingsError::InvalidData {
            path: path.to_owned(),
            message: source.to_string(),
        }
    })?;
    settings
        .validate()
        .map_err(|message| AiSettingsError::InvalidData {
            path: path.to_owned(),
            message,
        })?;
    Ok(settings)
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> AiSettingsError {
    AiSettingsError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}

fn remove_owned_file(path: &Path) -> Result<(), AiSettingsError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(
            "remove owned settings temporary file",
            path,
            source,
        )),
    }
}

fn sibling(path: &Path, name: &str) -> PathBuf {
    path.parent().unwrap_or_else(|| Path::new(".")).join(name)
}

#[cfg(not(windows))]
fn publish(partial: &Path, final_path: &Path, previous: &Path) -> Result<(), AiSettingsError> {
    fs::rename(partial, final_path).map_err(|source| AiSettingsError::AtomicPublication {
        path: final_path.to_owned(),
        source,
    })?;
    remove_owned_file(previous)?;
    File::open(final_path.parent().unwrap_or_else(|| Path::new(".")))
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("sync settings directory", final_path, source))
}

#[cfg(windows)]
fn publish(partial: &Path, final_path: &Path, previous: &Path) -> Result<(), AiSettingsError> {
    if final_path.exists() {
        remove_owned_file(previous)?;
        fs::rename(final_path, previous).map_err(|source| AiSettingsError::AtomicPublication {
            path: final_path.to_owned(),
            source,
        })?;
    }
    if let Err(source) = fs::rename(partial, final_path) {
        let restore = if previous.exists() {
            fs::rename(previous, final_path)
        } else {
            Ok(())
        };
        let message = match restore {
            Ok(()) => source.to_string(),
            Err(restore_error) => {
                format!("{source}; restoring previous settings also failed: {restore_error}")
            }
        };
        return Err(AiSettingsError::AtomicPublication {
            path: final_path.to_owned(),
            source: io::Error::other(message),
        });
    }
    remove_owned_file(previous)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str) -> ProviderConfig {
        ProviderConfig {
            id: id.to_owned(),
            kind: ProviderKind::OpenCodeGo,
            name: "OpenCode Go".to_owned(),
            base_url: "https://opencode.ai/zen/go/v1".to_owned(),
            models: vec![ModelConfig {
                id: "glm-5.3".to_owned(),
                context_window: Some(200_000),
                reasoning_efforts: vec![ReasoningEffort::Low, ReasoningEffort::High],
            }],
            session_header: Some("x-opencode-session".to_owned()),
            headers: Vec::new(),
        }
    }

    fn selection(provider_id: &str, effort: Option<ReasoningEffort>) -> ModelSelection {
        ModelSelection {
            provider_id: provider_id.to_owned(),
            model_id: "glm-5.3".to_owned(),
            effort,
        }
    }

    fn store() -> (tempfile::TempDir, AiSettingsStore) {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = AiSettingsStore::new(directory.path().join("app").join(SETTINGS_FILE_NAME));
        (directory, store)
    }

    #[test]
    fn missing_settings_load_as_empty_defaults() {
        let (_directory, store) = store();
        assert_eq!(store.load().expect("load"), AiSettings::default());
    }

    #[test]
    fn updates_round_trip_through_the_published_file() {
        let (_directory, store) = store();
        store
            .update(|settings| {
                settings.upsert_provider(provider("p1"));
                settings.agent_model = Some(selection("p1", Some(ReasoningEffort::High)));
                Ok(())
            })
            .expect("update");

        let loaded = store.load().expect("load");
        assert_eq!(loaded.providers, vec![provider("p1")]);
        assert_eq!(
            loaded.agent_model,
            Some(selection("p1", Some(ReasoningEffort::High)))
        );
        let text = fs::read_to_string(store.path()).expect("settings text");
        assert!(text.contains("\"formatVersion\": 1"));
        assert!(text.contains("\"reasoningEfforts\""));
        assert!(!store.path().with_extension("json.partial").exists());
    }

    #[test]
    fn invalid_update_writes_nothing() {
        let (_directory, store) = store();
        let error = store
            .update(|settings| {
                settings.upsert_provider(provider("p1"));
                settings.agent_model = Some(selection("p1", Some(ReasoningEffort::Medium)));
                Ok(())
            })
            .expect_err("unsupported effort is rejected");
        assert!(matches!(error, AiSettingsError::Rejected { .. }));
        assert!(!store.path().exists());
    }

    #[test]
    fn removing_a_provider_clears_its_default_selection() {
        let mut settings = AiSettings::default();
        settings.upsert_provider(provider("p1"));
        settings.agent_model = Some(selection("p1", None));
        assert!(settings.remove_provider("p1"));
        assert_eq!(settings.agent_model, None);
        assert!(!settings.remove_provider("p1"));
    }

    #[test]
    fn replacing_a_provider_clears_a_selection_it_no_longer_supports() {
        let mut settings = AiSettings::default();
        settings.upsert_provider(provider("p1"));
        settings.agent_model = Some(selection("p1", Some(ReasoningEffort::Low)));

        let mut replacement = provider("p1");
        replacement.models[0].reasoning_efforts = vec![ReasoningEffort::High];
        settings.upsert_provider(replacement);
        assert_eq!(settings.agent_model, None);
    }

    #[test]
    fn provider_validation_rejects_duplicates_and_unnormalized_urls() {
        let mut duplicate_model = provider("p1");
        duplicate_model
            .models
            .push(duplicate_model.models[0].clone());
        assert!(validate_provider(&duplicate_model).is_err());

        let mut trailing_slash = provider("p1");
        trailing_slash.base_url.push('/');
        assert!(validate_provider(&trailing_slash).is_err());

        let mut duplicate_header = provider("p1");
        duplicate_header
            .headers
            .push(crate::provider::HeaderConfig {
                name: "x-opencode-session".to_owned(),
                value: "fixed".to_owned(),
            });
        assert!(validate_provider(&duplicate_header).is_err());

        let mut moved_chatgpt = provider("p1");
        moved_chatgpt.kind = ProviderKind::ChatGpt;
        moved_chatgpt.session_header = None;
        assert!(validate_provider(&moved_chatgpt).is_err());
        moved_chatgpt.base_url = CHATGPT_CODEX_BASE_URL.to_owned();
        assert!(validate_provider(&moved_chatgpt).is_ok());

        let mut empty_name = provider("p1");
        empty_name.name = "  ".to_owned();
        assert!(validate_provider(&empty_name).is_err());

        let settings = AiSettings {
            providers: vec![provider("p1"), provider("p1")],
            ..AiSettings::default()
        };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn malformed_newer_and_unknown_documents_are_typed_errors_and_kept() {
        let cases = [
            ("{", "json"),
            (
                "{\"formatVersion\":2,\"providers\":[],\"agentModel\":null}",
                "version",
            ),
            (
                "{\"formatVersion\":1,\"providers\":[],\"agentModel\":null,\"apiKey\":\"x\"}",
                "data",
            ),
        ];
        for (contents, expected) in cases {
            let (_directory, store) = store();
            fs::create_dir_all(store.path().parent().expect("parent")).expect("directory");
            fs::write(store.path(), contents).expect("write");
            let error = store.load().expect_err("invalid document");
            let matched = match expected {
                "json" => matches!(error, AiSettingsError::InvalidJson { .. }),
                "version" => matches!(
                    error,
                    AiSettingsError::UnsupportedVersion { version: 2, .. }
                ),
                _ => matches!(error, AiSettingsError::InvalidData { .. }),
            };
            assert!(matched, "{contents}: {error}");
            assert_eq!(fs::read_to_string(store.path()).expect("kept"), contents);
        }
    }

    #[test]
    fn documents_without_header_fields_still_load() {
        let (_directory, store) = store();
        fs::create_dir_all(store.path().parent().expect("parent")).expect("directory");
        fs::write(
            store.path(),
            r#"{"formatVersion":1,"providers":[{"id":"p1","kind":"openCodeGo","name":"Go","baseUrl":"https://opencode.ai/zen/go/v1","models":[]}],"agentModel":null}"#,
        )
        .expect("write");
        let loaded = store.load().expect("load");
        assert_eq!(loaded.providers[0].session_header, None);
        assert!(loaded.providers[0].headers.is_empty());
    }

    #[test]
    fn previous_file_is_recovered_only_when_the_final_file_is_missing() {
        let (_directory, store) = store();
        store
            .update(|settings| {
                settings.upsert_provider(provider("p1"));
                Ok(())
            })
            .expect("update");
        let previous = store.path().with_file_name(PREVIOUS_FILE_NAME);
        fs::rename(store.path(), &previous).expect("simulate interrupted publication");

        let loaded = store.load().expect("recovered");
        assert_eq!(loaded.providers, vec![provider("p1")]);
        assert!(store.path().exists());
        assert!(!previous.exists());
    }
}
