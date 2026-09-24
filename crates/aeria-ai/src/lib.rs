//! Provider-neutral AI translation orchestration and response validation.
//!
//! This crate does not depend on Tauri. It owns provider configuration,
//! local provider settings, API-key storage, the OpenAI-compatible
//! transport, and Angelica's conversation loop, tools, and conversation
//! storage; see `docs/architecture/ai.md`.

#![forbid(unsafe_code)]

pub mod agent;
pub mod chat;
pub mod chatgpt;
pub mod client;
pub mod conversation;
pub mod draft;
pub mod guidance;
pub mod jobs;
pub mod prompt;
pub mod provider;
pub mod responses;
pub mod search;
pub mod secrets;
pub mod settings;
pub mod tools;
pub mod web;
pub mod worker;

pub use client::{ModelCheck, OpenAiCompatibleClient, ProviderEndpoint, ProviderError};
pub use provider::{
    BaseUrl, CHATGPT_CODEX_BASE_URL, HeaderConfig, ModelConfig, Protocol, ProviderConfig,
    ProviderKind, ProviderPreset, ReasoningEffort, presets,
};
pub use secrets::{ApiKey, KeyringSecretStore, MemorySecretStore, SecretStore, SecretStoreError};
pub use settings::{AiSettings, AiSettingsError, AiSettingsStore, ModelSelection};
