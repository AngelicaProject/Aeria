//! OpenAI-compatible Chat Completions transport.

use std::sync::Once;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::chat::{
    AssistantResponse, ChatMessage, ChatRequest, StreamAccumulator, StreamDelta, StreamError,
};
use crate::chatgpt::parse_model_catalog;
use crate::provider::{BaseUrl, ModelConfig, Protocol, ReasoningEffort};
use crate::responses::{self, ResponsesAccumulator};
use crate::secrets::ApiKey;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CHECK_TIMEOUT: Duration = Duration::from_secs(90);
const MODELS_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest silence accepted between two reads of a response.
const READ_TIMEOUT: Duration = Duration::from_secs(180);
/// Longest provider error text kept in an error message.
const MAX_PROVIDER_MESSAGE_CHARS: usize = 400;
/// Largest model list accepted from a provider.
pub const MAX_REMOTE_MODELS: usize = 5000;

/// Where and how to reach one provider.
#[derive(Clone, Debug)]
pub struct ProviderEndpoint {
    pub base_url: BaseUrl,
    pub api_key: ApiKey,
    /// Header that receives the conversation's session ID, when the
    /// provider asks for one.
    pub session_header: Option<String>,
    /// Validated extra static headers.
    pub headers: Vec<(String, String)>,
    pub protocol: Protocol,
}

impl ProviderEndpoint {
    /// Adds authentication, extra headers, and the session header.
    fn authorize(
        &self,
        request: reqwest::RequestBuilder,
        session: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let mut request = request.bearer_auth(self.api_key.expose());
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        let session_header = match self.protocol {
            Protocol::CodexResponses => Some("session_id"),
            Protocol::ChatCompletions => self.session_header.as_deref(),
        };
        if let (Some(name), Some(session)) = (session_header, session) {
            request = request.header(name, session);
        }
        request
    }
}

/// Transport failures, classified for the user. Messages never contain the
/// API key.
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("could not reach the provider: {message}")]
    Network { message: String },

    #[error("the provider did not respond in time")]
    Timeout,

    #[error("the provider rejected the API key ({status}): {message}")]
    Unauthorized { status: u16, message: String },

    #[error("the provider does not offer this endpoint or model ({status}): {message}")]
    NotFound { status: u16, message: String },

    #[error("the provider is rate limiting requests: {message}")]
    RateLimited { message: String },

    #[error("the provider rejected the request ({status}): {message}")]
    Rejected { status: u16, message: String },

    #[error("the provider failed ({status}): {message}")]
    Unavailable { status: u16, message: String },

    #[error("the provider returned an unexpected response: {message}")]
    InvalidResponse { message: String },

    #[error("could not start the HTTP client: {message}")]
    Client { message: String },
}

/// The outcome of a successful one-request model check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelCheck {
    /// The model name the provider reported, which may differ from the ID
    /// requested when the provider routes aliases.
    pub model: Option<String>,
    pub latency: Duration,
}

/// A reusable client for OpenAI-compatible providers.
#[derive(Clone, Debug)]
pub struct OpenAiCompatibleClient {
    http: reqwest::Client,
}

impl OpenAiCompatibleClient {
    /// Builds a client that verifies TLS with the platform's trust store.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Client`] when the HTTP stack cannot start.
    pub fn new() -> Result<Self, ProviderError> {
        install_crypto_provider();
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .user_agent(concat!("Aeria/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| ProviderError::Client {
                message: error.to_string(),
            })?;
        Ok(Self { http })
    }

    /// The shared HTTP client.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Lists the models the provider reports, sorted by ID and without
    /// duplicates.
    ///
    /// Chat Completions providers list IDs at `GET /models`; some list models
    /// they serve only through other APIs, so a listed model still has to
    /// pass [`Self::check_model`]. The Codex backend's catalog also states
    /// context windows and reasoning levels.
    ///
    /// # Errors
    ///
    /// Returns a classified [`ProviderError`].
    pub async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Vec<ModelConfig>, ProviderError> {
        #[derive(Deserialize)]
        struct ModelList {
            data: Vec<ModelEntry>,
        }
        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
        }

        let mut models = match endpoint.protocol {
            Protocol::ChatCompletions => {
                let request = endpoint
                    .authorize(self.http.get(endpoint.base_url.endpoint("models")), None)
                    .timeout(MODELS_TIMEOUT);
                let body = send(request, &endpoint.api_key).await?;
                let list: ModelList = serde_json::from_value(body).map_err(|error| {
                    ProviderError::InvalidResponse {
                        message: format!("model list: {error}"),
                    }
                })?;
                list.data
                    .into_iter()
                    .map(|entry| entry.id.trim().to_owned())
                    .filter(|id| !id.is_empty())
                    .map(ModelConfig::new)
                    .collect::<Vec<_>>()
            }
            Protocol::CodexResponses => {
                // The catalog hides models newer than the stated client
                // version; an empty answer is retried without the gate.
                let mut models = Vec::new();
                for version in ["99.0.0", "0.0.0"] {
                    let request = endpoint
                        .authorize(
                            self.http
                                .get(endpoint.base_url.endpoint("models"))
                                .query(&[("client_version", version)]),
                            None,
                        )
                        .timeout(MODELS_TIMEOUT);
                    models = parse_model_catalog(&send(request, &endpoint.api_key).await?);
                    if !models.is_empty() {
                        break;
                    }
                }
                models
            }
        };
        if models.len() > MAX_REMOTE_MODELS {
            return Err(ProviderError::InvalidResponse {
                message: format!("model list has more than {MAX_REMOTE_MODELS} entries"),
            });
        }
        models.sort_by(|left, right| left.id.cmp(&right.id));
        models.dedup_by(|left, right| left.id == right.id);
        Ok(models)
    }

    /// Sends one minimal request to confirm that the credentials, endpoint,
    /// model, effort, and headers are accepted. The check uses its own
    /// one-off session ID.
    ///
    /// # Errors
    ///
    /// Returns a classified [`ProviderError`].
    pub async fn check_model(
        &self,
        endpoint: &ProviderEndpoint,
        model: &str,
        effort: Option<ReasoningEffort>,
    ) -> Result<ModelCheck, ProviderError> {
        let session = uuid::Uuid::new_v4().to_string();
        let started = Instant::now();
        if endpoint.protocol == Protocol::CodexResponses {
            let messages = [ChatMessage::User {
                content: "Reply with the single word OK.".to_owned(),
            }];
            let request = ChatRequest {
                model,
                effort,
                system: "Answer briefly.",
                messages: &messages,
                tools: &[],
                turn_start: 0,
            };
            let (_, answered_by) = self
                .stream_codex(endpoint, &session, &request, &mut |_| {})
                .await?;
            return Ok(ModelCheck {
                model: answered_by,
                latency: started.elapsed(),
            });
        }
        let mut body = json!({
            "model": model,
            "messages": [{ "role": "user", "content": "Reply with the single word OK." }],
            "max_tokens": 32,
            "stream": false,
        });
        if let Some(effort) = effort {
            body["reasoning_effort"] = Value::from(effort.as_str());
        }
        let request = endpoint
            .authorize(
                self.http
                    .post(endpoint.base_url.endpoint("chat/completions")),
                Some(&session),
            )
            .timeout(CHECK_TIMEOUT)
            .json(&body);
        let response = send(request, &endpoint.api_key).await?;
        let latency = started.elapsed();
        if response
            .get("choices")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
        {
            return Err(ProviderError::InvalidResponse {
                message: "the completion has no choices".to_owned(),
            });
        }
        Ok(ModelCheck {
            model: response
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned),
            latency,
        })
    }

    /// Streams one response, passing text and reasoning deltas to
    /// `on_delta` as they arrive, and returns the assembled response.
    ///
    /// Dropping the returned future cancels the request.
    ///
    /// # Errors
    ///
    /// Returns a classified [`ProviderError`]; a malformed stream is
    /// [`ProviderError::InvalidResponse`].
    pub async fn stream_chat(
        &self,
        endpoint: &ProviderEndpoint,
        session: &str,
        request: &ChatRequest<'_>,
        on_delta: &mut (dyn FnMut(StreamDelta) + Send),
    ) -> Result<AssistantResponse, ProviderError> {
        if endpoint.protocol == Protocol::CodexResponses {
            return self
                .stream_codex(endpoint, session, request, on_delta)
                .await
                .map(|(response, _)| response);
        }
        let mut response = self
            .open_stream(endpoint, "chat/completions", session, &request.body())
            .await?;
        let invalid = |error: StreamError| ProviderError::InvalidResponse {
            message: redact(&error.0, &endpoint.api_key),
        };
        let mut accumulator = StreamAccumulator::default();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| transport_error(&error, &endpoint.api_key))?
        {
            accumulator.push(&chunk, on_delta).map_err(invalid)?;
            if accumulator.is_done() {
                break;
            }
        }
        accumulator.finish(on_delta).map_err(invalid)
    }

    async fn stream_codex(
        &self,
        endpoint: &ProviderEndpoint,
        session: &str,
        request: &ChatRequest<'_>,
        on_delta: &mut (dyn FnMut(StreamDelta) + Send),
    ) -> Result<(AssistantResponse, Option<String>), ProviderError> {
        let body = responses::request_body(request, session);
        let mut response = self
            .open_stream(endpoint, "responses", session, &body)
            .await?;
        let invalid =
            |error: StreamError| classify_stream_failure(&redact(&error.0, &endpoint.api_key));
        let mut accumulator = ResponsesAccumulator::default();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| transport_error(&error, &endpoint.api_key))?
        {
            accumulator.push(&chunk, on_delta).map_err(invalid)?;
            if accumulator.is_done() {
                break;
            }
        }
        accumulator.finish(on_delta).map_err(invalid)
    }

    async fn open_stream(
        &self,
        endpoint: &ProviderEndpoint,
        path: &str,
        session: &str,
        body: &Value,
    ) -> Result<reqwest::Response, ProviderError> {
        let response = endpoint
            .authorize(
                self.http.post(endpoint.base_url.endpoint(path)),
                Some(session),
            )
            .header("accept", "text/event-stream")
            .json(body)
            .send()
            .await
            .map_err(|error| transport_error(&error, &endpoint.api_key))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let text = response
            .text()
            .await
            .map_err(|error| transport_error(&error, &endpoint.api_key))?;
        Err(status_error(
            status,
            &provider_message(&text, &endpoint.api_key),
        ))
    }
}

/// A failed Codex response reports plan limits in its message; they are
/// shown as rate limiting so the user knows to wait rather than reconfigure.
fn classify_stream_failure(message: &str) -> ProviderError {
    let lower = message.to_lowercase();
    if lower.contains("usage limit") || lower.contains("rate limit") {
        ProviderError::RateLimited {
            message: message.to_owned(),
        }
    } else {
        ProviderError::InvalidResponse {
            message: message.to_owned(),
        }
    }
}

fn install_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // Another component may already have installed a provider; either is
        // acceptable for verifying provider certificates.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

async fn send(request: reqwest::RequestBuilder, key: &ApiKey) -> Result<Value, ProviderError> {
    let response = request
        .send()
        .await
        .map_err(|error| transport_error(&error, key))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| transport_error(&error, key))?;
    if !status.is_success() {
        return Err(status_error(status, &provider_message(&text, key)));
    }
    serde_json::from_str(&text).map_err(|error| ProviderError::InvalidResponse {
        message: format!("response is not JSON: {error}"),
    })
}

fn transport_error(error: &reqwest::Error, key: &ApiKey) -> ProviderError {
    if error.is_timeout() {
        return ProviderError::Timeout;
    }
    ProviderError::Network {
        message: redact(&error.to_string(), key),
    }
}

pub(crate) fn status_error_for(status: StatusCode, message: &str) -> ProviderError {
    status_error(status, message)
}

fn status_error(status: StatusCode, message: &str) -> ProviderError {
    let code = status.as_u16();
    let message = message.to_owned();
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderError::Unauthorized {
            status: code,
            message,
        },
        StatusCode::NOT_FOUND => ProviderError::NotFound {
            status: code,
            message,
        },
        StatusCode::TOO_MANY_REQUESTS => ProviderError::RateLimited { message },
        status if status.is_server_error() => ProviderError::Unavailable {
            status: code,
            message,
        },
        _ => ProviderError::Rejected {
            status: code,
            message,
        },
    }
}

/// Extracts the provider's error text from `{"error":{"message":…}}` or the
/// raw body, bounded and with the key removed.
fn provider_message(body: &str, key: &ApiKey) -> String {
    let parsed = serde_json::from_str::<Value>(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("error"))
                .or_else(|| value.get("message"))
        })
        .and_then(Value::as_str)
        .unwrap_or(body)
        .trim();
    let message = redact(message, key);
    if message.is_empty() {
        return "no details".to_owned();
    }
    let mut bounded: String = message.chars().take(MAX_PROVIDER_MESSAGE_CHARS).collect();
    if bounded.len() < message.len() {
        bounded.push('…');
    }
    bounded
}

fn redact(text: &str, key: &ApiKey) -> String {
    text.replace(key.expose(), "<redacted>")
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};

    use super::*;

    const KEY: &str = "sk-test-secret";

    /// Serves one canned HTTP response and returns the raw request.
    fn serve_once(status: &str, body: &str) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream);
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("header line");
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().expect("content length");
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).expect("body");
            reader
                .get_mut()
                .write_all(response.as_bytes())
                .expect("response");
            head + &String::from_utf8(body).expect("utf-8 body")
        });
        (format!("http://127.0.0.1:{}/v1", address.port()), handle)
    }

    fn endpoint(base_url: &str) -> ProviderEndpoint {
        ProviderEndpoint {
            base_url: BaseUrl::parse(base_url).expect("base URL"),
            api_key: ApiKey::new(KEY).expect("key"),
            session_header: None,
            headers: Vec::new(),
            protocol: Protocol::ChatCompletions,
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    #[test]
    fn check_model_sends_bearer_key_model_and_effort() {
        let (url, server) = serve_once(
            "200 OK",
            r#"{"model":"glm-5.3-2026","choices":[{"message":{"content":"OK"}}]}"#,
        );
        let client = OpenAiCompatibleClient::new().expect("client");
        let check = runtime()
            .block_on(client.check_model(&endpoint(&url), "glm-5.3", Some(ReasoningEffort::Low)))
            .expect("check succeeds");
        assert_eq!(check.model.as_deref(), Some("glm-5.3-2026"));

        let request = server.join().expect("server");
        assert!(
            request.starts_with("POST /v1/chat/completions "),
            "{request}"
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains(&format!("authorization: bearer {KEY}"))
        );
        let body: Value =
            serde_json::from_str(request.split("\r\n\r\n").nth(1).expect("body")).expect("json");
        assert_eq!(body["model"], "glm-5.3");
        assert_eq!(body["reasoning_effort"], "low");
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn requests_carry_extra_headers_and_a_session_id_only_for_completions() {
        let configured = |url: &str| {
            let mut endpoint = endpoint(url);
            endpoint.session_header = Some("x-opencode-session".to_owned());
            endpoint.headers = vec![("x-client".to_owned(), "aeria".to_owned())];
            endpoint
        };
        let client = OpenAiCompatibleClient::new().expect("client");

        let (url, server) = serve_once("200 OK", r#"{"choices":[{}]}"#);
        runtime()
            .block_on(client.check_model(&configured(&url), "glm-5.3", None))
            .expect("check succeeds");
        let request = server.join().expect("server").to_ascii_lowercase();
        assert!(request.contains("x-client: aeria"), "{request}");
        let session = request
            .lines()
            .find_map(|line| line.strip_prefix("x-opencode-session: "))
            .expect("session header");
        assert_eq!(session.trim().len(), 36, "{session}");

        let (url, server) = serve_once("200 OK", r#"{"data":[]}"#);
        runtime()
            .block_on(client.list_models(&configured(&url)))
            .expect("models");
        let request = server.join().expect("server").to_ascii_lowercase();
        assert!(request.contains("x-client: aeria"));
        assert!(!request.contains("x-opencode-session"));
    }

    #[test]
    fn codex_requests_use_responses_with_session_and_identity_headers() {
        let events = [
            r#"{"type":"response.output_text.delta","delta":"OK"}"#,
            r#"{"type":"response.completed","response":{"status":"completed","model":"gpt-5.5","usage":{"input_tokens":3,"output_tokens":1}}}"#,
        ];
        let body = format!(
            "data: {}

data: {}

",
            events[0], events[1]
        );
        let (url, server) = serve_once("200 OK", &body);
        let mut endpoint = endpoint(&url);
        endpoint.protocol = Protocol::CodexResponses;
        endpoint.headers = vec![
            ("originator".to_owned(), "aeria".to_owned()),
            ("chatgpt-account-id".to_owned(), "acct-1".to_owned()),
        ];
        let client = OpenAiCompatibleClient::new().expect("client");
        let check = runtime()
            .block_on(client.check_model(&endpoint, "gpt-5.5", Some(ReasoningEffort::High)))
            .expect("check succeeds");
        assert_eq!(check.model.as_deref(), Some("gpt-5.5"));

        let request = server.join().expect("server");
        assert!(request.starts_with("POST /v1/responses "), "{request}");
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("originator: aeria"));
        assert!(lower.contains("chatgpt-account-id: acct-1"));
        assert!(lower.contains("session_id: "));
        let sent: Value =
            serde_json::from_str(request.split("\r\n\r\n").nth(1).expect("body")).expect("json");
        assert_eq!(sent["store"], false);
        assert_eq!(sent["reasoning"]["effort"], "high");
        assert_eq!(sent["input"][0]["content"][0]["type"], "input_text");
    }

    #[test]
    fn codex_limit_failures_are_reported_as_rate_limits() {
        let body = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"You've hit your usage limit\"}}}\n\n";
        let (url, server) = serve_once("200 OK", body);
        let mut endpoint = endpoint(&url);
        endpoint.protocol = Protocol::CodexResponses;
        let client = OpenAiCompatibleClient::new().expect("client");
        let error = runtime()
            .block_on(client.check_model(&endpoint, "gpt-5.5", None))
            .expect_err("limit reached");
        server.join().expect("server");
        assert!(
            matches!(error, ProviderError::RateLimited { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn check_model_omits_effort_when_none_is_selected() {
        let (url, server) = serve_once("200 OK", r#"{"choices":[{}]}"#);
        let client = OpenAiCompatibleClient::new().expect("client");
        runtime()
            .block_on(client.check_model(&endpoint(&url), "kimi-k3", None))
            .expect("check succeeds");
        let request = server.join().expect("server");
        assert!(!request.contains("reasoning_effort"));
    }

    #[test]
    fn provider_errors_are_classified_and_never_echo_the_key() {
        let cases = [
            ("401 Unauthorized", "unauthorized"),
            ("404 Not Found", "not found"),
            ("429 Too Many Requests", "rate limited"),
            ("400 Bad Request", "rejected"),
            ("503 Service Unavailable", "unavailable"),
        ];
        for (status, expected) in cases {
            let body = format!(r#"{{"error":{{"message":"bad key {KEY}"}}}}"#);
            let (url, server) = serve_once(status, &body);
            let client = OpenAiCompatibleClient::new().expect("client");
            let error = runtime()
                .block_on(client.check_model(&endpoint(&url), "glm-5.3", None))
                .expect_err("request fails");
            server.join().expect("server");
            let matched = match expected {
                "unauthorized" => matches!(error, ProviderError::Unauthorized { status: 401, .. }),
                "not found" => matches!(error, ProviderError::NotFound { .. }),
                "rate limited" => matches!(error, ProviderError::RateLimited { .. }),
                "rejected" => matches!(error, ProviderError::Rejected { status: 400, .. }),
                _ => matches!(error, ProviderError::Unavailable { status: 503, .. }),
            };
            assert!(matched, "{status}: {error:?}");
            let text = error.to_string();
            assert!(!text.contains(KEY), "{text}");
            assert!(text.contains("bad key <redacted>"), "{text}");
        }
    }

    #[test]
    fn completion_without_choices_is_an_invalid_response() {
        let (url, server) = serve_once("200 OK", r#"{"choices":[]}"#);
        let client = OpenAiCompatibleClient::new().expect("client");
        let error = runtime()
            .block_on(client.check_model(&endpoint(&url), "glm-5.3", None))
            .expect_err("empty choices");
        server.join().expect("server");
        assert!(matches!(error, ProviderError::InvalidResponse { .. }));
    }

    #[test]
    fn list_models_returns_sorted_unique_ids() {
        let (url, server) = serve_once(
            "200 OK",
            r#"{"object":"list","data":[{"id":"kimi-k3"},{"id":"glm-5.3"},{"id":"kimi-k3"},{"id":" "}]}"#,
        );
        let client = OpenAiCompatibleClient::new().expect("client");
        let models = runtime()
            .block_on(client.list_models(&endpoint(&url)))
            .expect("models");
        let request = server.join().expect("server");
        assert!(request.starts_with("GET /v1/models "), "{request}");
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["glm-5.3", "kimi-k3"]
        );
    }

    #[test]
    fn long_provider_messages_are_bounded() {
        let key = ApiKey::new(KEY).expect("key");
        let message = provider_message(&"x".repeat(2000), &key);
        assert_eq!(message.chars().count(), MAX_PROVIDER_MESSAGE_CHARS + 1);
        assert_eq!(provider_message("", &key), "no details");
    }
}
