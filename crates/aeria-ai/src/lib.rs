//! Provider-neutral AI translation orchestration and response validation.
//!
//! This crate does not depend on Tauri. It currently owns provider
//! configuration, local provider settings, API-key storage, and the
//! OpenAI-compatible transport; see `docs/architecture/ai.md`.

#![forbid(unsafe_code)]

pub mod client;
pub mod provider;
pub mod secrets;
pub mod settings;

pub use client::{ModelCheck, OpenAiCompatibleClient, ProviderEndpoint, ProviderError};
pub use provider::{
    BaseUrl, HeaderConfig, ModelConfig, ProviderConfig, ProviderKind, ProviderPreset,
    ReasoningEffort, presets,
};
pub use secrets::{ApiKey, KeyringSecretStore, MemorySecretStore, SecretStore, SecretStoreError};
pub use settings::{AiSettings, AiSettingsError, AiSettingsStore, ModelSelection};
