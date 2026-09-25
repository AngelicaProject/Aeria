//! GitHub releases of the translation repository.
//!
//! A pack is published as release `harmonia/<sequence>` with the `.hpk.br`
//! and `feed-entry.json` assets (`docs/formats/feed-v1.md`). The release is
//! created as a draft, the assets are uploaded, and only then is it
//! published, so the feed workflow never sees a release without its assets.

use std::fmt::Write as _;
use std::sync::Once;
use std::time::Duration;

use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

const API_BASE: &str = "https://api.github.com";
const UPLOADS_BASE: &str = "https://uploads.github.com/";
const API_VERSION: &str = "2022-11-28";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_mins(15);
const MAX_RELEASE_PAGES: u32 = 50;
/// The host whose Git credential authorizes API calls.
pub const GITHUB_HOST: &str = "github.com";
/// Release tags of packs are `harmonia/<sequence>`.
pub const RELEASE_TAG_PREFIX: &str = "harmonia/";

/// A repository on github.com.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHubRepository {
    pub owner: String,
    pub name: String,
}

impl GitHubRepository {
    /// Recognizes HTTPS and SSH remote URLs of github.com repositories.
    #[must_use]
    pub fn from_remote_url(url: &str) -> Option<Self> {
        let url = url.trim();
        let path = if let Some(rest) = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("ssh://"))
        {
            let (authority, path) = rest.split_once('/')?;
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            let host = host.split_once(':').map_or(host, |(host, _)| host);
            if !host.eq_ignore_ascii_case(GITHUB_HOST) {
                return None;
            }
            path
        } else {
            let (user_host, path) = url.split_once(':')?;
            let host = user_host
                .rsplit_once('@')
                .map_or(user_host, |(_, host)| host);
            if !host.eq_ignore_ascii_case(GITHUB_HOST) || path.starts_with('/') {
                return None;
            }
            path
        };
        let path = path.trim_end_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        let (owner, name) = path.split_once('/')?;
        let valid = |part: &str| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        (valid(owner) && valid(name)).then(|| Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    #[must_use]
    pub fn homepage(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }

    /// Where the feed workflow publishes the feed on GitHub Pages.
    #[must_use]
    pub fn feed_url(&self) -> String {
        let host = format!("{}.github.io", self.owner.to_ascii_lowercase());
        if self.name.eq_ignore_ascii_case(&host) {
            format!("https://{host}/harmonia/feed-v1.json")
        } else {
            format!("https://{host}/{}/harmonia/feed-v1.json", self.name)
        }
    }

    /// The download URL of a published release asset.
    #[must_use]
    pub fn asset_download_url(&self, sequence: u64, asset_name: &str) -> String {
        format!(
            "{}/releases/download/{}/{asset_name}",
            self.homepage(),
            release_tag(sequence)
        )
    }
}

#[must_use]
pub fn release_tag(sequence: u64) -> String {
    format!("{RELEASE_TAG_PREFIX}{sequence}")
}

/// The `.hpk.br` asset name of a release.
#[must_use]
pub fn pack_asset_name(pack_id: &str, sequence: u64) -> String {
    format!("{pack_id}-{sequence}.hpk.br")
}

/// A pack release that exists on GitHub.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExistingRelease {
    pub sequence: u64,
    pub draft: bool,
}

/// One file attached to a release.
#[derive(Clone, Debug)]
pub struct ReleaseAsset {
    pub name: String,
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

/// A release to create.
#[derive(Clone, Debug)]
pub struct ReleaseRequest {
    pub sequence: u64,
    /// The commit the tag is created on; it must already be on GitHub.
    pub commit: String,
    pub name: String,
    pub body: String,
    pub prerelease: bool,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedRelease {
    pub html_url: String,
}

/// GitHub API failures. Messages come from GitHub and never contain the token.
#[derive(Debug, Error)]
pub enum PublishError {
    #[error("could not reach GitHub: {message}")]
    Network { message: String },
    #[error("GitHub refused the sign-in: {message}")]
    Unauthorized { message: String },
    #[error("GitHub denied access: {message}")]
    Forbidden { message: String },
    #[error("the GitHub repository was not found or is not accessible: {message}")]
    NotFound { message: String },
    #[error("GitHub rejected the request: {message}")]
    Rejected { message: String },
    #[error("GitHub returned an unexpected response: {message}")]
    InvalidResponse { message: String },
}

/// A GitHub REST API client for release publishing.
#[derive(Clone, Debug)]
pub struct GitHubClient {
    http: reqwest::Client,
    api_base: String,
    /// Upload URLs returned by the API must start with this, so the token is
    /// only ever sent to GitHub.
    uploads_base: String,
}

#[derive(Deserialize)]
struct ReleaseJson {
    id: u64,
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    upload_url: String,
    #[serde(default)]
    html_url: String,
}

impl GitHubClient {
    /// # Errors
    /// Returns [`PublishError::Network`] when the HTTP client cannot be built.
    pub fn new() -> Result<Self, PublishError> {
        Self::with_base_urls(API_BASE, UPLOADS_BASE)
    }

    /// A client for other API and upload roots, used by tests.
    ///
    /// # Errors
    /// Returns [`PublishError::Network`] when the HTTP client cannot be built.
    pub fn with_base_urls(api_base: &str, uploads_base: &str) -> Result<Self, PublishError> {
        install_crypto_provider();
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("Aeria/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| PublishError::Network {
                message: error.to_string(),
            })?;
        Ok(Self {
            http,
            api_base: api_base.trim_end_matches('/').to_owned(),
            uploads_base: uploads_base.to_owned(),
        })
    }

    fn request(&self, method: Method, url: &str, token: &str) -> RequestBuilder {
        self.http
            .request(method, url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
    }

    fn releases_url(&self, repository: &GitHubRepository) -> String {
        format!(
            "{}/repos/{}/{}/releases",
            self.api_base, repository.owner, repository.name
        )
    }

    /// Lists the pack releases (`harmonia/<sequence>` tags), drafts included.
    ///
    /// # Errors
    /// Returns a [`PublishError`] for failed or unexpected responses.
    pub async fn pack_releases(
        &self,
        repository: &GitHubRepository,
        token: &str,
    ) -> Result<Vec<ExistingRelease>, PublishError> {
        let url = self.releases_url(repository);
        let mut releases = Vec::new();
        for page in 1..=MAX_RELEASE_PAGES {
            let response = send(
                self.request(Method::GET, &url, token)
                    .query(&[("per_page", "100"), ("page", &page.to_string())]),
            )
            .await?;
            let batch: Vec<ReleaseJson> = parse(response).await?;
            let done = batch.len() < 100;
            releases.extend(batch.into_iter().filter_map(|release| {
                let sequence = release
                    .tag_name
                    .strip_prefix(RELEASE_TAG_PREFIX)?
                    .parse::<u64>()
                    .ok()?;
                Some(ExistingRelease {
                    sequence,
                    draft: release.draft,
                })
            }));
            if done {
                break;
            }
        }
        releases.sort_by_key(|release| std::cmp::Reverse(release.sequence));
        Ok(releases)
    }

    /// Creates the release as a draft, uploads its assets, and publishes it.
    /// A draft left by a failed attempt is deleted.
    ///
    /// # Errors
    /// Returns a [`PublishError`]; nothing stays published on failure.
    pub async fn publish_release(
        &self,
        repository: &GitHubRepository,
        token: &str,
        release: &ReleaseRequest,
    ) -> Result<PublishedRelease, PublishError> {
        let url = self.releases_url(repository);
        let response = send(self.request(Method::POST, &url, token).json(&json!({
            "tag_name": release_tag(release.sequence),
            "target_commitish": release.commit,
            "name": release.name,
            "body": release.body,
            "draft": true,
            "prerelease": release.prerelease,
        })))
        .await?;
        let draft: ReleaseJson = parse(response).await?;
        let release_url = format!("{url}/{}", draft.id);

        let outcome = async {
            let upload_url = draft
                .upload_url
                .split_once('{')
                .map_or(draft.upload_url.as_str(), |(base, _)| base);
            if !upload_url.starts_with(&self.uploads_base) {
                return Err(PublishError::InvalidResponse {
                    message: "the release upload URL is not on GitHub".to_owned(),
                });
            }
            for asset in &release.assets {
                send(
                    self.request(Method::POST, upload_url, token)
                        .query(&[("name", asset.name.as_str())])
                        .header("Content-Type", asset.content_type)
                        .body(asset.bytes.clone()),
                )
                .await?;
            }
            let response = send(
                self.request(Method::PATCH, &release_url, token)
                    .json(&json!({ "draft": false })),
            )
            .await?;
            let published: ReleaseJson = parse(response).await?;
            Ok(PublishedRelease {
                html_url: published.html_url,
            })
        }
        .await;
        if outcome.is_err() {
            let _ = send(self.request(Method::DELETE, &release_url, token)).await;
        }
        outcome
    }
}

async fn send(request: RequestBuilder) -> Result<Response, PublishError> {
    let response = request
        .send()
        .await
        .map_err(|error| PublishError::Network {
            message: error.without_url().to_string(),
        })?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let message = error_message(status, &response.text().await.unwrap_or_default());
    Err(match status {
        StatusCode::UNAUTHORIZED => PublishError::Unauthorized { message },
        StatusCode::FORBIDDEN => PublishError::Forbidden { message },
        StatusCode::NOT_FOUND => PublishError::NotFound { message },
        _ if status.is_client_error() => PublishError::Rejected { message },
        _ => PublishError::Network { message },
    })
}

async fn parse<T: serde::de::DeserializeOwned>(response: Response) -> Result<T, PublishError> {
    response
        .json()
        .await
        .map_err(|error| PublishError::InvalidResponse {
            message: error.without_url().to_string(),
        })
}

/// GitHub's `message` plus any validation error codes, e.g.
/// `Validation Failed (already_exists)`.
fn error_message(status: StatusCode, body: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let mut message = value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map_or_else(|| status.to_string(), str::to_owned);
    let codes: Vec<&str> = value
        .get("errors")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|error| error.get("code").and_then(serde_json::Value::as_str))
        .collect();
    if !codes.is_empty() {
        let _ = write!(message, " ({})", codes.join(", "));
    }
    message
}

fn install_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // Another component may already have installed a provider; either is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_remotes_are_recognized() {
        let expected = Some(GitHubRepository {
            owner: "Angelica-Project".to_owned(),
            name: "ru.translation".to_owned(),
        });
        for url in [
            "https://github.com/Angelica-Project/ru.translation.git",
            "https://github.com/Angelica-Project/ru.translation",
            "https://someone@GitHub.com/Angelica-Project/ru.translation/",
            "git@github.com:Angelica-Project/ru.translation.git",
            "ssh://git@github.com/Angelica-Project/ru.translation.git",
            "ssh://git@github.com:22/Angelica-Project/ru.translation",
        ] {
            assert_eq!(GitHubRepository::from_remote_url(url), expected, "{url}");
        }
        for url in [
            "https://gitlab.com/owner/repo.git",
            "https://github.com.evil.example/owner/repo",
            "https://github.com/owner",
            "https://github.com/owner/repo/extra",
            "git@github.com:/owner/repo",
            "D:/projects/repo",
            "https://github.com/../repo",
        ] {
            assert_eq!(GitHubRepository::from_remote_url(url), None, "{url}");
        }
    }

    #[test]
    fn release_urls_follow_the_feed_layout() {
        let repository = GitHubRepository {
            owner: "Owner".to_owned(),
            name: "ru".to_owned(),
        };
        assert_eq!(
            repository.asset_download_url(42, &pack_asset_name("ru-main", 42)),
            "https://github.com/Owner/ru/releases/download/harmonia/42/ru-main-42.hpk.br"
        );
        assert_eq!(
            repository.feed_url(),
            "https://owner.github.io/ru/harmonia/feed-v1.json"
        );
        let site = GitHubRepository {
            owner: "Owner".to_owned(),
            name: "owner.github.io".to_owned(),
        };
        assert_eq!(
            site.feed_url(),
            "https://owner.github.io/harmonia/feed-v1.json"
        );
    }

    #[test]
    fn validation_errors_name_their_codes() {
        assert_eq!(
            error_message(
                StatusCode::UNPROCESSABLE_ENTITY,
                r#"{"message":"Validation Failed","errors":[{"code":"already_exists"}]}"#
            ),
            "Validation Failed (already_exists)"
        );
        assert_eq!(
            error_message(StatusCode::BAD_GATEWAY, "<html>"),
            "502 Bad Gateway"
        );
    }
}
