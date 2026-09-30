//! ChatGPT subscription sign-in for the Codex backend.
//!
//! OpenAI offers no official way for third-party applications to use a
//! ChatGPT plan. This follows the device-code sign-in of the public Codex
//! client, as other open-source agents do, and identifies requests as Aeria.
//! It can stop working whenever OpenAI changes the flow.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ModelError;

const ISSUER: &str = "https://auth.openai.com";
/// The public OAuth client of the Codex CLI, which OpenAI's sign-in accepts.
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// The page where the user enters the device code.
pub const VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
/// How requests identify their harness to the Codex backend.
pub const ORIGINATOR: &str = "aeria";
/// Longest time a device code is polled.
pub const LOGIN_TIMEOUT: Duration = Duration::from_mins(15);
/// An access token is refreshed this long before it expires.
const REFRESH_MARGIN: Duration = Duration::from_secs(120);
const AUTH_TIMEOUT: Duration = Duration::from_secs(20);

/// A token. Its `Debug` output is redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a token after trimming surrounding whitespace.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty token or one with whitespace or control
    /// characters, which cannot be sent as a bearer token.
    pub fn new(value: &str) -> Result<Self, ModelError> {
        let trimmed = value.trim();
        if trimmed.is_empty()
            || trimmed
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(ModelError::Invalid("the token is not usable".to_owned()));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The token text, for sending it to OpenAI.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(<redacted>)")
    }
}

/// A started device sign-in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceLogin {
    pub user_code: String,
    pub device_auth_id: String,
    pub interval: Duration,
}

/// The result of polling a device sign-in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DevicePoll {
    Pending,
    Authorized {
        authorization_code: String,
        code_verifier: String,
    },
}

/// Tokens returned by a sign-in or refresh.
#[derive(Clone, Debug)]
pub struct TokenSet {
    pub access: AccessToken,
    /// A rotated refresh token, when the server issued one.
    pub refresh_token: Option<Secret>,
}

/// A usable access token and the account facts carried in its claims.
#[derive(Clone, Debug)]
pub struct AccessToken {
    pub token: Secret,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub residency: Option<String>,
    /// Unix seconds; `None` when the token carries no expiry.
    pub expires_at: Option<u64>,
}

impl AccessToken {
    /// Reads the claims of a JWT access token.
    ///
    /// # Errors
    ///
    /// Returns an error for a token that is not a readable JWT.
    pub fn parse(token: &str) -> Result<Self, ModelError> {
        let invalid =
            |message: &str| ModelError::Invalid(format!("ChatGPT access token: {message}"));
        let payload = token
            .split('.')
            .nth(1)
            .ok_or_else(|| invalid("not a JWT"))?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .map_err(|_| invalid("claims are not base64url"))?;
        let claims: Value =
            serde_json::from_slice(&bytes).map_err(|_| invalid("claims are not JSON"))?;
        let auth = claims.get("https://api.openai.com/auth");
        let text = |key: &str| {
            auth.and_then(|auth| auth.get(key))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let email = claims
            .pointer("/https:~1~1api.openai.com~1profile/email")
            .or_else(|| claims.get("email"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(Self {
            token: Secret::new(token).map_err(|_| invalid("contains whitespace"))?,
            account_id: text("chatgpt_account_id"),
            email,
            residency: text("chatgpt_data_residency").or_else(|| text("chatgpt_compute_residency")),
            expires_at: claims.get("exp").and_then(Value::as_u64),
        })
    }

    /// Returns whether the token is still usable for a request starting now.
    #[must_use]
    pub fn is_fresh(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        self.expires_at
            .is_none_or(|expires| now + REFRESH_MARGIN.as_secs() < expires)
    }

    /// Headers the Codex backend needs besides the bearer token.
    #[must_use]
    pub fn headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![("originator".to_owned(), ORIGINATOR.to_owned())];
        if let Some(account) = &self.account_id {
            headers.push(("chatgpt-account-id".to_owned(), account.clone()));
        }
        if let Some(residency) = &self.residency {
            headers.push((
                "x-openai-internal-codex-residency".to_owned(),
                residency.clone(),
            ));
        }
        headers
    }
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    user_code: String,
    device_auth_id: String,
    #[serde(default)]
    interval: Option<Value>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

fn transport_error(error: &reqwest::Error) -> ModelError {
    if error.is_timeout() {
        ModelError::Timeout
    } else {
        ModelError::Network(error.to_string())
    }
}

/// Starts a device sign-in and returns the code the user enters at
/// [`VERIFICATION_URL`].
///
/// # Errors
///
/// Returns a classified error when OpenAI refuses or is unreachable.
pub async fn start_login(http: &reqwest::Client) -> Result<DeviceLogin, ModelError> {
    let value = post_json(
        http,
        &format!("{ISSUER}/api/accounts/deviceauth/usercode"),
        &json!({ "client_id": CLIENT_ID }),
    )
    .await?
    .ok_or_else(|| ModelError::Invalid("OpenAI did not issue a device code".to_owned()))?;
    let response: DeviceCodeResponse = serde_json::from_value(value)
        .map_err(|error| ModelError::Invalid(format!("device code: {error}")))?;
    let interval = response
        .interval
        .as_ref()
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(5)
        .max(3);
    Ok(DeviceLogin {
        user_code: response.user_code,
        device_auth_id: response.device_auth_id,
        interval: Duration::from_secs(interval),
    })
}

/// Polls a device sign-in once.
///
/// # Errors
///
/// Returns a classified error for anything but a pending or finished
/// sign-in.
pub async fn poll_login(
    http: &reqwest::Client,
    login: &DeviceLogin,
) -> Result<DevicePoll, ModelError> {
    let response = post_json(
        http,
        &format!("{ISSUER}/api/accounts/deviceauth/token"),
        &json!({ "device_auth_id": login.device_auth_id, "user_code": login.user_code }),
    )
    .await?;
    let Some(value) = response else {
        return Ok(DevicePoll::Pending);
    };
    let field = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    match (field("authorization_code"), field("code_verifier")) {
        (Some(authorization_code), Some(code_verifier)) => Ok(DevicePoll::Authorized {
            authorization_code,
            code_verifier,
        }),
        _ => Err(ModelError::Invalid(
            "the device sign-in finished without an authorization code".to_owned(),
        )),
    }
}

/// Exchanges a finished device sign-in for tokens.
///
/// # Errors
///
/// Returns a classified error when the exchange fails.
pub async fn exchange(
    http: &reqwest::Client,
    authorization_code: &str,
    code_verifier: &str,
) -> Result<TokenSet, ModelError> {
    let redirect = format!("{ISSUER}/deviceauth/callback");
    token(
        http,
        &[
            ("grant_type", "authorization_code"),
            ("code", authorization_code),
            ("redirect_uri", &redirect),
            ("client_id", CLIENT_ID),
            ("code_verifier", code_verifier),
        ],
    )
    .await
}

/// Obtains a new access token. OpenAI may rotate the refresh token; a
/// returned one replaces the stored one.
///
/// # Errors
///
/// Returns [`ModelError::SignInRequired`] when the sign-in is no longer
/// valid and the user must sign in again.
pub async fn refresh(
    http: &reqwest::Client,
    refresh_token: &Secret,
) -> Result<TokenSet, ModelError> {
    token(
        http,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose()),
            ("client_id", CLIENT_ID),
        ],
    )
    .await
}

async fn token(http: &reqwest::Client, form: &[(&str, &str)]) -> Result<TokenSet, ModelError> {
    let response = http
        .post(format!("{ISSUER}/oauth/token"))
        .timeout(AUTH_TIMEOUT)
        .form(form)
        .send()
        .await
        .map_err(|error| transport_error(&error))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| transport_error(&error))?;
    if !status.is_success() {
        if status.is_client_error() && status.as_u16() != 429 {
            return Err(ModelError::SignInRequired);
        }
        return Err(ModelError::Refused {
            status: status.as_u16(),
            message: "the ChatGPT sign-in service refused the request".to_owned(),
        });
    }
    let tokens: TokenResponse = serde_json::from_str(&text)
        .map_err(|error| ModelError::Invalid(format!("token response: {error}")))?;
    Ok(TokenSet {
        access: AccessToken::parse(&tokens.access_token)?,
        refresh_token: tokens
            .refresh_token
            .as_deref()
            .and_then(|token| Secret::new(token).ok()),
    })
}

/// POSTs JSON to the sign-in service. `Ok(None)` means the sign-in is still
/// pending (HTTP 403 or 404).
async fn post_json(
    http: &reqwest::Client,
    url: &str,
    body: &Value,
) -> Result<Option<Value>, ModelError> {
    let response = http
        .post(url)
        .timeout(AUTH_TIMEOUT)
        .json(body)
        .send()
        .await
        .map_err(|error| transport_error(&error))?;
    let status = response.status();
    if matches!(status.as_u16(), 403 | 404) {
        return Ok(None);
    }
    let text = response
        .text()
        .await
        .map_err(|error| transport_error(&error))?;
    if !status.is_success() {
        return Err(ModelError::Refused {
            status: status.as_u16(),
            message: format!("the ChatGPT sign-in service answered {status}"),
        });
    }
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| ModelError::Invalid(format!("sign-in response: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: &Value) -> String {
        format!(
            "eyJhbGciOiJub25lIn0.{}.signature",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    }

    #[test]
    fn access_token_claims_give_account_residency_and_expiry() {
        let token = jwt(&json!({
            "exp": 4_000_000_000_u64,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct-1",
                "chatgpt_compute_residency": "eu",
            },
            "https://api.openai.com/profile": { "email": "ada@example.com" },
        }));
        let access = AccessToken::parse(&token).expect("token");
        assert_eq!(access.account_id.as_deref(), Some("acct-1"));
        assert_eq!(access.email.as_deref(), Some("ada@example.com"));
        assert!(access.is_fresh());
        assert_eq!(
            access.headers(),
            vec![
                ("originator".to_owned(), "aeria".to_owned()),
                ("chatgpt-account-id".to_owned(), "acct-1".to_owned()),
                (
                    "x-openai-internal-codex-residency".to_owned(),
                    "eu".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn expired_and_malformed_tokens_are_detected() {
        let expired = AccessToken::parse(&jwt(&json!({ "exp": 10 }))).expect("token");
        assert!(!expired.is_fresh());
        assert!(AccessToken::parse("not-a-jwt").is_err());
        assert!(AccessToken::parse("a.!!!.c").is_err());
        assert_eq!(
            format!("{:?}", Secret::new("x").expect("secret")),
            "Secret(<redacted>)"
        );
    }
}
