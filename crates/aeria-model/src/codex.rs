//! The Codex backend: the signed-in account, its models, and one response
//! to one request.

use std::sync::{Arc, Once};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::ModelError;
use crate::auth::{self, AccessToken, DeviceLogin, DevicePoll, Secret, TokenSet};

/// The Codex backend of a ChatGPT subscription.
pub const BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// A response that sends nothing for this long, its headers included, is
/// given up.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(180);
const MODELS_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How often an open connection is checked, so a connection that died
/// without closing fails its requests instead of holding them.
const KEEP_ALIVE: Duration = Duration::from_secs(20);

/// Where the refresh token of the sign-in is kept.
pub trait TokenStore: Send + Sync {
    /// The stored refresh token, if any.
    ///
    /// # Errors
    ///
    /// Returns a description when the store cannot be read.
    fn get(&self) -> Result<Option<Secret>, String>;

    /// Stores or replaces the refresh token.
    ///
    /// # Errors
    ///
    /// Returns a description when the store cannot be written.
    fn set(&self, token: &Secret) -> Result<(), String>;

    /// Removes the refresh token; removing a missing one succeeds.
    ///
    /// # Errors
    ///
    /// Returns a description when the store cannot be written.
    fn delete(&self) -> Result<(), String>;
}

/// The OS credential store: Windows Credential Manager, the Secret Service
/// on Linux, or the macOS Keychain.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyringStore;

impl KeyringStore {
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new("Aeria", "chatgpt/refresh-token").map_err(|error| error.to_string())
    }
}

impl TokenStore for KeyringStore {
    fn get(&self) -> Result<Option<Secret>, String> {
        match Self::entry()?.get_password() {
            Ok(value) => Ok(Secret::new(&value).ok()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn set(&self, token: &Secret) -> Result<(), String> {
        Self::entry()?
            .set_password(token.expose())
            .map_err(|error| error.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// A model the account can use, with the reasoning efforts it accepts.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub efforts: Vec<String>,
}

/// One request: instructions that stay the same for a run (so the backend
/// serves them from its cache), the task, and the model.
#[derive(Clone, Debug)]
pub struct Request {
    pub model: String,
    pub effort: Option<String>,
    pub instructions: String,
    pub input: String,
    /// Requests with the same key and the same beginning share a cache.
    pub cache_key: String,
}

/// Tokens one response used.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub input: u64,
    /// Input tokens served from the backend's prompt cache.
    pub cached: u64,
    pub output: u64,
}

/// The text of one response and what it used.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Reply {
    pub text: String,
    pub usage: Usage,
    /// The response stopped before it was finished (a length limit).
    pub incomplete: bool,
}

fn install_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // Another component may already have installed a provider; either is
        // acceptable for verifying provider certificates.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// The signed-in ChatGPT account and requests to the Codex backend.
pub struct Codex {
    http: reqwest::Client,
    store: Arc<dyn TokenStore>,
    access: tokio::sync::Mutex<Option<AccessToken>>,
    base: String,
    silence: Duration,
}

impl Codex {
    /// A client over `store`, which holds the sign-in between runs of Aeria.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be built.
    pub fn new(store: Arc<dyn TokenStore>) -> Result<Self, ModelError> {
        Self::with_base(store, BASE_URL)
    }

    /// A client of another backend with the same protocol, for tests.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be built.
    pub fn with_base(store: Arc<dyn TokenStore>, base: &str) -> Result<Self, ModelError> {
        install_crypto_provider();
        let http = reqwest::Client::builder()
            .user_agent(concat!("Aeria/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .tcp_keepalive(KEEP_ALIVE)
            .http2_keep_alive_interval(KEEP_ALIVE)
            .http2_keep_alive_timeout(KEEP_ALIVE)
            .http2_keep_alive_while_idle(true)
            .build()
            .map_err(|error| ModelError::Network(error.to_string()))?;
        Ok(Self {
            http,
            store,
            access: tokio::sync::Mutex::new(None),
            base: base.trim_end_matches('/').to_owned(),
            silence: SILENCE_TIMEOUT,
        })
    }

    /// The same client giving up a response after `silence` without a byte,
    /// for tests.
    #[must_use]
    pub const fn with_silence(mut self, silence: Duration) -> Self {
        self.silence = silence;
        self
    }

    /// A client of another backend that already holds an access token, for
    /// tests: it never signs in or refreshes.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be built.
    pub fn with_access(
        store: Arc<dyn TokenStore>,
        base: &str,
        access: AccessToken,
    ) -> Result<Self, ModelError> {
        let codex = Self::with_base(store, base)?;
        *codex
            .access
            .try_lock()
            .map_err(|error| ModelError::Invalid(error.to_string()))? = Some(access);
        Ok(codex)
    }

    /// Whether a sign-in is stored.
    ///
    /// # Errors
    ///
    /// Returns an error when the credential store cannot be read.
    pub fn signed_in(&self) -> Result<bool, ModelError> {
        Ok(self.store.get().map_err(ModelError::Store)?.is_some())
    }

    /// The e-mail address of the signed-in account, when a token was read.
    pub async fn account(&self) -> Option<String> {
        self.access
            .lock()
            .await
            .as_ref()
            .and_then(|access| access.email.clone())
    }

    /// Starts a device sign-in.
    ///
    /// # Errors
    ///
    /// Returns a classified error when OpenAI refuses or is unreachable.
    pub async fn sign_in_start(&self) -> Result<DeviceLogin, ModelError> {
        auth::start_login(&self.http).await
    }

    /// Polls a device sign-in once; `true` once it finished and the
    /// sign-in is stored.
    ///
    /// # Errors
    ///
    /// Returns a classified error when the sign-in failed.
    pub async fn sign_in_poll(&self, login: &DeviceLogin) -> Result<bool, ModelError> {
        match auth::poll_login(&self.http, login).await? {
            DevicePoll::Pending => Ok(false),
            DevicePoll::Authorized {
                authorization_code,
                code_verifier,
            } => {
                let tokens =
                    auth::exchange(&self.http, &authorization_code, &code_verifier).await?;
                self.keep(tokens).await?;
                Ok(true)
            }
        }
    }

    /// Forgets the sign-in.
    ///
    /// # Errors
    ///
    /// Returns an error when the credential store cannot be written.
    pub async fn sign_out(&self) -> Result<(), ModelError> {
        *self.access.lock().await = None;
        self.store.delete().map_err(ModelError::Store)
    }

    async fn keep(&self, tokens: TokenSet) -> Result<(), ModelError> {
        if let Some(refresh) = &tokens.refresh_token {
            self.store.set(refresh).map_err(ModelError::Store)?;
        }
        *self.access.lock().await = Some(tokens.access);
        Ok(())
    }

    /// A fresh access token, refreshed one at a time so a rotated refresh
    /// token is never used twice.
    async fn token(&self) -> Result<AccessToken, ModelError> {
        let mut access = self.access.lock().await;
        if let Some(token) = access.as_ref()
            && token.is_fresh()
        {
            return Ok(token.clone());
        }
        let refresh = self
            .store
            .get()
            .map_err(ModelError::Store)?
            .ok_or(ModelError::SignInRequired)?;
        let tokens = auth::refresh(&self.http, &refresh).await?;
        if let Some(rotated) = &tokens.refresh_token {
            self.store.set(rotated).map_err(ModelError::Store)?;
        }
        *access = Some(tokens.access.clone());
        Ok(tokens.access)
    }

    fn authorize(builder: reqwest::RequestBuilder, token: &AccessToken) -> reqwest::RequestBuilder {
        let mut builder = builder.bearer_auth(token.token.expose());
        for (name, value) in token.headers() {
            builder = builder.header(name, value);
        }
        builder
    }

    /// The models the account can use.
    ///
    /// # Errors
    ///
    /// Returns a classified error when the backend cannot be reached or
    /// refuses.
    pub async fn models(&self) -> Result<Vec<ModelInfo>, ModelError> {
        let token = self.token().await?;
        // The catalog hides models newer than the stated client version; an
        // empty answer is retried without the gate.
        for version in ["99.0.0", "0.0.0"] {
            let response = Self::authorize(self.http.get(format!("{}/models", self.base)), &token)
                .query(&[("client_version", version)])
                .timeout(MODELS_TIMEOUT)
                .send()
                .await
                .map_err(|error| transport(&error))?;
            let status = response.status();
            let text = response.text().await.map_err(|error| transport(&error))?;
            if !status.is_success() {
                return Err(status_error(status.as_u16(), &text, None));
            }
            let value: Value = serde_json::from_str(&text)
                .map_err(|error| ModelError::Invalid(format!("model list: {error}")))?;
            let models = parse_models(&value);
            if !models.is_empty() {
                return Ok(models);
            }
        }
        Ok(Vec::new())
    }

    /// Sends one request and returns the response's text.
    ///
    /// # Errors
    ///
    /// Returns a classified error: [`ModelError::UsageLimit`] when the
    /// plan's limit is reached, [`ModelError::RateLimited`] when requests
    /// come too fast, [`ModelError::SignInRequired`], a network error, or
    /// [`ModelError::Timeout`] when the service sends nothing, not even the
    /// response's headers, for three minutes.
    pub async fn respond(&self, request: &Request) -> Result<Reply, ModelError> {
        let token = self.token().await?;
        let mut reasoning = json!({ "summary": "auto" });
        if let Some(effort) = &request.effort {
            reasoning["effort"] = Value::from(effort.as_str());
        }
        let body = json!({
            "model": request.model,
            "instructions": request.instructions,
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": request.input }],
            }],
            "store": false,
            "stream": true,
            "reasoning": reasoning,
            "include": [],
            "prompt_cache_key": request.cache_key,
        });
        let sent = Self::authorize(self.http.post(format!("{}/responses", self.base)), &token)
            .header("session_id", &request.cache_key)
            .header("accept", "text/event-stream")
            .json(&body)
            .send();
        let response = tokio::time::timeout(self.silence, sent)
            .await
            .map_err(|_| ModelError::Timeout)?
            .map_err(|error| transport(&error))?;
        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let text = tokio::time::timeout(self.silence, response.text())
                .await
                .map_err(|_| ModelError::Timeout)?
                .map_err(|error| transport(&error))?;
            return Err(status_error(status.as_u16(), &text, retry_after));
        }
        let mut response = response;
        let mut events = Events::default();
        loop {
            let chunk = tokio::time::timeout(self.silence, response.chunk())
                .await
                .map_err(|_| ModelError::Timeout)?
                .map_err(|error| transport(&error))?;
            let Some(chunk) = chunk else { break };
            events.push(&chunk)?;
            if events.done {
                break;
            }
        }
        events.finish()
    }
}

fn transport(error: &reqwest::Error) -> ModelError {
    if error.is_timeout() {
        ModelError::Timeout
    } else {
        ModelError::Network(error.to_string())
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// Classifies a failed answer. The plan's usage limit is reported with the
/// time it resets when the backend says; other refusals keep their status.
fn status_error(status: u16, body: &str, retry_after: Option<u64>) -> ModelError {
    let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let error = value.get("error").unwrap_or(&value);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map_or_else(|| body.chars().take(300).collect(), str::to_owned);
    let kind = error
        .get("type")
        .or_else(|| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind.contains("usage_limit") || message.to_lowercase().contains("usage limit") {
        let resets_at = error.get("resets_at").and_then(Value::as_u64).or_else(|| {
            error
                .get("resets_in_seconds")
                .and_then(Value::as_u64)
                .map(|seconds| now_secs() + seconds)
        });
        return ModelError::UsageLimit { message, resets_at };
    }
    match status {
        401 | 403 => ModelError::SignInRequired,
        429 => ModelError::RateLimited {
            message,
            retry_after: retry_after.map(Duration::from_secs),
        },
        _ => ModelError::Refused { status, message },
    }
}

/// Reads the Codex model catalog (`GET /models?client_version=…`): visible
/// models with the reasoning efforts they accept.
#[must_use]
pub fn parse_models(value: &Value) -> Vec<ModelInfo> {
    let Some(models) = value.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut result: Vec<ModelInfo> = models
        .iter()
        .filter(|model| model.get("visibility").and_then(Value::as_str) != Some("hide"))
        .filter_map(|model| {
            let id = model.get("slug").and_then(Value::as_str)?.trim();
            if id.is_empty() {
                return None;
            }
            let efforts = model
                .get("supported_reasoning_levels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|level| {
                    level
                        .get("effort")
                        .and_then(Value::as_str)
                        .or_else(|| level.as_str())
                        .map(str::to_owned)
                })
                .collect();
            Some(ModelInfo {
                id: id.to_owned(),
                efforts,
            })
        })
        .collect();
    result.dedup_by(|left, right| left.id == right.id);
    result
}

/// The server-sent events of one response.
#[derive(Default)]
struct Events {
    buffer: Vec<u8>,
    reply: Reply,
    done: bool,
}

impl Events {
    fn push(&mut self, bytes: &[u8]) -> Result<(), ModelError> {
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            self.line(line.trim_end_matches(['\n', '\r']))?;
        }
        Ok(())
    }

    fn line(&mut self, line: &str) -> Result<(), ModelError> {
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(());
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return Ok(());
        }
        let event: Value = serde_json::from_str(data)
            .map_err(|error| ModelError::Invalid(format!("invalid stream event: {error}")))?;
        match event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "response.output_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    self.reply.text.push_str(delta);
                }
            }
            "response.completed" | "response.incomplete" => {
                let response = event.get("response").unwrap_or(&Value::Null);
                self.reply.incomplete =
                    response.get("status").and_then(Value::as_str) == Some("incomplete");
                if let Some(usage) = response.get("usage") {
                    let number =
                        |pointer: &str| usage.pointer(pointer).and_then(Value::as_u64).unwrap_or(0);
                    self.reply.usage = Usage {
                        input: number("/input_tokens"),
                        cached: number("/input_tokens_details/cached_tokens"),
                        output: number("/output_tokens"),
                    };
                }
                self.done = true;
            }
            "response.failed" | "error" => {
                let error = event
                    .pointer("/response/error")
                    .or_else(|| event.get("error"))
                    .unwrap_or(&event);
                return Err(status_error(
                    500,
                    &json!({ "error": error }).to_string(),
                    None,
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Reply, ModelError> {
        let rest = String::from_utf8_lossy(&std::mem::take(&mut self.buffer)).into_owned();
        self.line(rest.trim_end_matches('\r'))?;
        if !self.done {
            return Err(ModelError::Network(
                "the response ended before it completed".to_owned(),
            ));
        }
        Ok(self.reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_become_text_and_usage() {
        let mut events = Events::default();
        events
            .push(b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"{\\\"1\\\":\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"\\\"OK\\\"}\"}\n")
            .expect("deltas");
        events
            .push(b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":100,\"input_tokens_details\":{\"cached_tokens\":80},\"output_tokens\":5}}}\n")
            .expect("completed");
        let reply = events.finish().expect("reply");
        assert_eq!(reply.text, "{\"1\":\"OK\"}");
        assert_eq!(
            reply.usage,
            Usage {
                input: 100,
                cached: 80,
                output: 5
            }
        );
        assert!(Events::default().finish().is_err());
    }

    #[test]
    fn limits_and_refusals_are_classified() {
        let usage = status_error(
            429,
            r#"{"error":{"type":"usage_limit_reached","message":"You've hit your usage limit","resets_at":1900000000}}"#,
            None,
        );
        assert!(matches!(
            usage,
            ModelError::UsageLimit {
                resets_at: Some(1_900_000_000),
                ..
            }
        ));
        let rate = status_error(429, r#"{"error":{"message":"slow down"}}"#, Some(7));
        assert!(
            matches!(rate, ModelError::RateLimited { retry_after: Some(duration), .. } if duration == Duration::from_secs(7))
        );
        assert!(matches!(
            status_error(401, "", None),
            ModelError::SignInRequired
        ));
        let mut failed = Events::default();
        assert!(matches!(
            failed.push(b"data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"You've hit your usage limit\"}}}\n"),
            Err(ModelError::UsageLimit { .. })
        ));
    }

    #[test]
    fn the_catalog_lists_visible_models_with_their_efforts() {
        let models = parse_models(&json!({
            "models": [
                { "slug": "gpt-5.5", "supported_reasoning_levels": [{ "effort": "low" }, { "effort": "high" }] },
                { "slug": "internal", "visibility": "hide" },
                { "slug": "gpt-5.5-mini", "supported_reasoning_levels": ["medium"] },
                { "name": "no slug" },
            ]
        }));
        assert_eq!(
            models,
            vec![
                ModelInfo {
                    id: "gpt-5.5".to_owned(),
                    efforts: vec!["low".to_owned(), "high".to_owned()]
                },
                ModelInfo {
                    id: "gpt-5.5-mini".to_owned(),
                    efforts: vec!["medium".to_owned()]
                },
            ]
        );
    }
}
