//! Machine translation of the project: a program translates the
//! untranslated strings of `po/` with a language model, a small batch per
//! request, checks every answer, and writes what passes. See
//! `docs/architecture/translate.md`.
//!
//! The model is reached through a ChatGPT subscription: the sign-in of the
//! public Codex client ([`auth`]) and the Codex backend's Responses API
//! ([`codex`]). The run ([`run`]) builds each request from the project:
//! its style and terms, the translations of the game's names that occur in
//! the batch ([`names`]), and translated strings of the same file as
//! examples ([`prompt`]). The editor shows a person what a request tells
//! the model about one string ([`hints`]).

pub mod agree;
pub mod auth;
pub mod codex;
pub mod fit;
pub mod hints;
pub mod names;
pub mod prompt;
pub mod run;
pub mod sounds;

use std::time::Duration;

pub use codex::{Codex, KeyringStore, ModelInfo, Reply, Request, TokenStore, Usage};
pub use run::{Options, Rejected, Run, Status, Stop, plan};

/// Errors of the sign-in and of requests to the model.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ModelError {
    #[error("sign in to ChatGPT first")]
    SignInRequired,
    #[error("the plan's usage limit is reached: {message}")]
    UsageLimit {
        message: String,
        /// Unix seconds when the limit resets, when the backend says.
        resets_at: Option<u64>,
    },
    #[error("too many requests: {message}")]
    RateLimited {
        message: String,
        retry_after: Option<Duration>,
    },
    #[error("the model service is unreachable: {0}")]
    Network(String),
    #[error("the model service did not answer in time")]
    Timeout,
    #[error("the model service refused the request ({status}): {message}")]
    Refused { status: u16, message: String },
    #[error("unexpected answer: {0}")]
    Invalid(String),
    #[error("the OS credential store failed: {0}")]
    Store(String),
}
