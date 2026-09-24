//! Provider and model configuration shared by settings, transport, and UI.

use std::fmt;

use reqwest::Url;
use serde::{Deserialize, Serialize};

/// Longest accepted provider name, model ID, or model display name.
pub const MAX_NAME_CHARS: usize = 256;
/// Longest accepted base URL.
pub const MAX_BASE_URL_CHARS: usize = 2048;

/// The ready-made configuration a provider was created from.
///
/// Every kind uses the same OpenAI-compatible transport; the kind only
/// records which preset supplied the defaults.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    OpenCodeGo,
    OpenRouter,
    Custom,
}

/// A reasoning-effort value sent as the `reasoning_effort` request field.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    /// Returns the wire spelling of the effort.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// One model offered by a provider, with the capabilities Aeria relies on.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConfig {
    /// The provider's model identifier, sent as the request `model`.
    pub id: String,
    /// Context window in tokens, when known.
    pub context_window: Option<u32>,
    /// Effort values the model accepts. Empty means the field is never sent.
    pub reasoning_efforts: Vec<ReasoningEffort>,
}

impl ModelConfig {
    /// Creates a model entry with no known capabilities.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            context_window: None,
            reasoning_efforts: Vec::new(),
        }
    }
}

/// One configured provider. The API key is not part of the configuration; it
/// lives in the OS secret store under the provider ID.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub name: String,
    pub base_url: String,
    pub models: Vec<ModelConfig>,
}

impl ProviderConfig {
    /// Returns a new opaque, machine-local provider ID.
    #[must_use]
    pub fn new_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Returns the configured model with the given ID.
    #[must_use]
    pub fn model(&self, model_id: &str) -> Option<&ModelConfig> {
        self.models.iter().find(|model| model.id == model_id)
    }
}

/// Defaults for creating a provider of one kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderPreset {
    pub kind: ProviderKind,
    pub name: &'static str,
    /// `None` for presets whose URL the user supplies.
    pub base_url: Option<&'static str>,
    pub models: Vec<ModelConfig>,
}

/// Models that `OpenCode` Go serves through its Chat Completions endpoint.
/// Models it serves only through other endpoints are deliberately absent.
const OPENCODE_GO_MODELS: &[&str] = &[
    "glm-5.3",
    "glm-5.3-flash",
    "kimi-k3",
    "kimi-k2.7-code",
    "deepseek-v4-pro",
    "deepseek-v4-flash",
    "mimo-v2.6-pro",
    "mimo-v2.6-flash",
];

/// Returns the ready-made provider presets, first-choice preset first.
#[must_use]
pub fn presets() -> Vec<ProviderPreset> {
    vec![
        ProviderPreset {
            kind: ProviderKind::OpenCodeGo,
            name: "OpenCode Go",
            base_url: Some("https://opencode.ai/zen/go/v1"),
            models: OPENCODE_GO_MODELS
                .iter()
                .map(|id| ModelConfig::new(*id))
                .collect(),
        },
        ProviderPreset {
            kind: ProviderKind::OpenRouter,
            name: "OpenRouter",
            base_url: Some("https://openrouter.ai/api/v1"),
            models: Vec::new(),
        },
        ProviderPreset {
            kind: ProviderKind::Custom,
            name: "OpenAI-compatible",
            base_url: None,
            models: Vec::new(),
        },
    ]
}

/// A validated provider base URL without a trailing slash.
#[derive(Clone, Eq, PartialEq)]
pub struct BaseUrl(Url);

impl BaseUrl {
    /// Parses and validates a base URL.
    ///
    /// HTTPS is required, except plain HTTP to a loopback host for local
    /// servers. Credentials, queries, and fragments are rejected so that no
    /// secret can be stored in settings through the URL.
    ///
    /// # Errors
    ///
    /// Returns a description of the first violated rule.
    pub fn parse(value: &str) -> Result<Self, String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err("base URL must not be empty".to_owned());
        }
        if trimmed.chars().count() > MAX_BASE_URL_CHARS {
            return Err(format!(
                "base URL must be at most {MAX_BASE_URL_CHARS} characters"
            ));
        }
        let url = Url::parse(trimmed).map_err(|error| format!("invalid base URL: {error}"))?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        match url.scheme() {
            "https" => {}
            "http" if loopback => {}
            "http" => {
                return Err("base URL must use https unless it points to this computer".to_owned());
            }
            scheme => return Err(format!("unsupported base URL scheme {scheme:?}")),
        }
        if url.host_str().is_none_or(str::is_empty) {
            return Err("base URL must have a host".to_owned());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("base URL must not contain credentials".to_owned());
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err("base URL must not contain a query or fragment".to_owned());
        }
        let normalized = url.as_str().trim_end_matches('/').to_owned();
        Url::parse(&normalized)
            .map(Self)
            .map_err(|error| format!("invalid base URL: {error}"))
    }

    /// Returns the normalized URL text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str().trim_end_matches('/')
    }

    /// Returns the URL of an endpoint below the base, such as `models`.
    #[must_use]
    pub fn endpoint(&self, path: &str) -> String {
        format!("{}/{path}", self.as_str())
    }
}

impl fmt::Debug for BaseUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BaseUrl")
            .field(&self.as_str())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_normalized_without_trailing_slash() {
        let url = BaseUrl::parse(" https://opencode.ai/zen/go/v1/ ").expect("valid URL");
        assert_eq!(url.as_str(), "https://opencode.ai/zen/go/v1");
        assert_eq!(
            url.endpoint("chat/completions"),
            "https://opencode.ai/zen/go/v1/chat/completions"
        );
    }

    #[test]
    fn plain_http_is_only_accepted_for_loopback_hosts() {
        assert!(BaseUrl::parse("http://localhost:11434/v1").is_ok());
        assert!(BaseUrl::parse("http://127.0.0.1:8080/v1").is_ok());
        assert!(BaseUrl::parse("http://[::1]:8080/v1").is_ok());
        assert!(BaseUrl::parse("http://example.com/v1").is_err());
    }

    #[test]
    fn base_url_rejects_credentials_queries_and_other_schemes() {
        for value in [
            "",
            "not a url",
            "ftp://example.com/v1",
            "https://user:secret@example.com/v1",
            "https://example.com/v1?key=secret",
            "https://example.com/v1#fragment",
        ] {
            assert!(BaseUrl::parse(value).is_err(), "{value:?} must be rejected");
        }
    }

    #[test]
    fn debug_output_shows_only_the_url() {
        let url = BaseUrl::parse("https://openrouter.ai/api/v1").expect("valid URL");
        assert_eq!(
            format!("{url:?}"),
            "BaseUrl(\"https://openrouter.ai/api/v1\")"
        );
    }

    #[test]
    fn every_preset_with_a_url_is_valid_and_opencode_go_is_first() {
        let presets = presets();
        assert_eq!(presets[0].kind, ProviderKind::OpenCodeGo);
        for preset in &presets {
            if let Some(url) = preset.base_url {
                BaseUrl::parse(url).expect("preset URL is valid");
            }
        }
        assert!(
            presets[0]
                .models
                .iter()
                .all(|model| model.reasoning_efforts.is_empty())
        );
    }
}
