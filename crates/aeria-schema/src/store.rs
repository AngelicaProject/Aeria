//! EXDSchema from GitHub, kept in a local cache.
//!
//! EXDSchema has a branch `ver/<game version>` for every game version that
//! changed sheet headers. A game version uses the newest branch not newer
//! than itself, resolved to its commit once per application run; the
//! resolution is kept on disk for runs without network. Schema files are
//! read from that commit and cached under it, so a cached file never
//! changes; a sheet the commit has no file for is cached as missing.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use aeria_core::GameVersion;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::fields::{FieldsError, field_names};

const API_BASE: &str = "https://api.github.com";
const RAW_BASE: &str = "https://raw.githubusercontent.com";
const REPOSITORY: &str = "xivdev/EXDSchema";
const BRANCH_PREFIX: &str = "ver/";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// A larger schema file is not read.
const MAX_SCHEMA_BYTES: usize = 4 * 1024 * 1024;

/// Why column names could not be read.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("could not reach EXDSchema: {0}")]
    Network(String),
    #[error("EXDSchema answered {status} for {url}")]
    Status { status: u16, url: String },
    #[error("the EXDSchema cache could not be used: {0}")]
    Cache(#[from] std::io::Error),
    #[error("the schema of {sheet} cannot be read: {source}")]
    Schema { sheet: String, source: FieldsError },
}

/// The EXDSchema branch and commit a game version uses.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resolved {
    /// The branch's game version, such as `2026.09.01.0000.0000`.
    pub version: String,
    pub commit: String,
}

/// The fields of a sheet's schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SheetFields {
    /// The game version of the schema's branch.
    pub version: String,
    /// The field names in the schema's order; see
    /// [`crate::column_names`].
    pub fields: Vec<String>,
}

/// The newest of `versions` (branch game versions) not newer than `game`.
#[must_use]
pub fn pick_version<'a>(
    versions: impl IntoIterator<Item = &'a str>,
    game: &GameVersion,
) -> Option<&'a str> {
    versions
        .into_iter()
        .filter_map(|text| Some((text.parse::<GameVersion>().ok()?, text)))
        .filter(|(version, _)| version <= game)
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, text)| text)
}

/// Whether the game names a sheet in a way EXDSchema has a file for: root
/// sheets such as `PlaceName`; quest and cutscene sheets such as
/// `quest/000/ClsArc001_00001` have none.
fn is_schema_name(sheet: &str) -> bool {
    !sheet.is_empty()
        && sheet
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_commit(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Reads EXDSchema files from GitHub through a cache directory.
pub struct SchemaStore {
    http: reqwest::Client,
    api_base: String,
    raw_base: String,
    cache: PathBuf,
    /// Resolutions of this run by game version; `None` when no branch
    /// applies or GitHub could not be reached and nothing was kept.
    resolved: Mutex<HashMap<String, Option<Resolved>>>,
}

#[derive(Deserialize)]
struct MatchingRef {
    #[serde(rename = "ref")]
    name: String,
    object: RefObject,
}

#[derive(Deserialize)]
struct RefObject {
    sha: String,
}

impl SchemaStore {
    /// A store over GitHub with its cache in `cache`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Network`] when the HTTP client cannot be built.
    pub fn new(cache: PathBuf) -> Result<Self, StoreError> {
        Self::with_base_urls(cache, API_BASE, RAW_BASE)
    }

    /// A store over other API and file roots, used by tests.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Network`] when the HTTP client cannot be built.
    pub fn with_base_urls(
        cache: PathBuf,
        api_base: &str,
        raw_base: &str,
    ) -> Result<Self, StoreError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("Aeria/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| StoreError::Network(error.to_string()))?;
        Ok(Self {
            http,
            api_base: api_base.trim_end_matches('/').to_owned(),
            raw_base: raw_base.trim_end_matches('/').to_owned(),
            cache,
            resolved: Mutex::new(HashMap::new()),
        })
    }

    /// The fields of `sheet`'s schema for `game`, or `None` when EXDSchema
    /// has no branch for the game version or no file for the sheet.
    ///
    /// # Errors
    ///
    /// Returns an error when GitHub cannot be reached for something not in
    /// the cache, the cache cannot be written, or the file is not a schema.
    pub async fn sheet_fields(
        &self,
        game: &GameVersion,
        sheet: &str,
    ) -> Result<Option<SheetFields>, StoreError> {
        if !is_schema_name(sheet) {
            return Ok(None);
        }
        let Some(resolved) = self.resolve(game).await? else {
            return Ok(None);
        };
        let Some(text) = self.schema_file(&resolved.commit, sheet).await? else {
            return Ok(None);
        };
        let fields = field_names(&text).map_err(|source| StoreError::Schema {
            sheet: sheet.to_owned(),
            source,
        })?;
        Ok(Some(SheetFields {
            version: resolved.version,
            fields,
        }))
    }

    fn resolution_path(&self, game: &GameVersion) -> PathBuf {
        self.cache
            .join("resolved")
            .join(format!("{}.json", game.as_str()))
    }

    async fn resolve(&self, game: &GameVersion) -> Result<Option<Resolved>, StoreError> {
        if let Some(known) = self.lock_resolved().get(game.as_str()) {
            return Ok(known.clone());
        }
        let resolved = match self.resolve_online(game).await {
            Ok(resolved) => {
                match &resolved {
                    Some(resolved) => write_atomic(
                        &self.resolution_path(game),
                        &serde_json::to_vec(resolved).map_err(std::io::Error::other)?,
                    )?,
                    None => remove_if_present(&self.resolution_path(game))?,
                }
                resolved
            }
            // Without network, the resolution of an earlier run.
            Err(error) => match read_resolution(&self.resolution_path(game)) {
                Some(resolved) => Some(resolved),
                None => return Err(error),
            },
        };
        self.lock_resolved()
            .insert(game.as_str().to_owned(), resolved.clone());
        Ok(resolved)
    }

    async fn resolve_online(&self, game: &GameVersion) -> Result<Option<Resolved>, StoreError> {
        let url = format!(
            "{}/repos/{REPOSITORY}/git/matching-refs/heads/{BRANCH_PREFIX}",
            self.api_base
        );
        let response = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|error| StoreError::Network(error.to_string()))?;
        if !response.status().is_success() {
            return Err(StoreError::Status {
                status: response.status().as_u16(),
                url,
            });
        }
        let refs: Vec<MatchingRef> = response
            .json()
            .await
            .map_err(|error| StoreError::Network(error.to_string()))?;
        let prefix = format!("refs/heads/{BRANCH_PREFIX}");
        let branches: Vec<(&str, &str)> = refs
            .iter()
            .filter(|found| is_commit(&found.object.sha))
            .filter_map(|found| {
                Some((found.name.strip_prefix(&prefix)?, found.object.sha.as_str()))
            })
            .collect();
        Ok(
            pick_version(branches.iter().map(|(version, _)| *version), game).map(|version| {
                let commit = branches
                    .iter()
                    .find(|(candidate, _)| *candidate == version)
                    .map(|(_, commit)| (*commit).to_owned())
                    .unwrap_or_default();
                Resolved {
                    version: version.to_owned(),
                    commit,
                }
            }),
        )
    }

    async fn schema_file(&self, commit: &str, sheet: &str) -> Result<Option<String>, StoreError> {
        let directory = self.cache.join(commit);
        let file = directory.join(format!("{sheet}.yml"));
        let missing = directory.join(format!("{sheet}.missing"));
        if let Ok(text) = fs::read_to_string(&file) {
            return Ok(Some(text));
        }
        if missing.is_file() {
            return Ok(None);
        }
        let url = format!("{}/{REPOSITORY}/{commit}/{sheet}.yml", self.raw_base);
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|error| StoreError::Network(error.to_string()))?;
        if response.status() == StatusCode::NOT_FOUND {
            write_atomic(&missing, b"")?;
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(StoreError::Status {
                status: response.status().as_u16(),
                url,
            });
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| StoreError::Network(error.to_string()))?;
        if bytes.len() > MAX_SCHEMA_BYTES {
            return Err(StoreError::Network(format!("{url} is too large")));
        }
        let text = String::from_utf8(bytes.to_vec())
            .map_err(|_| StoreError::Network(format!("{url} is not UTF-8")))?;
        write_atomic(&file, text.as_bytes())?;
        Ok(Some(text))
    }

    fn lock_resolved(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<Resolved>>> {
        self.resolved.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn read_resolution(path: &Path) -> Option<Resolved> {
    let resolved: Resolved = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    is_commit(&resolved.commit).then_some(resolved)
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> GameVersion {
        text.parse().expect("version")
    }

    #[test]
    fn a_game_uses_the_newest_branch_not_newer_than_itself() {
        let branches = [
            "2026.04.21.0000.0000",
            "2026.09.01.0000.0000",
            "2026.07.16.0001.0000",
            "not-a-version",
        ];
        assert_eq!(
            pick_version(branches, &version("2026.10.06.0000.0000")),
            Some("2026.09.01.0000.0000")
        );
        assert_eq!(
            pick_version(branches, &version("2026.09.01.0000.0000")),
            Some("2026.09.01.0000.0000")
        );
        assert_eq!(
            pick_version(branches, &version("2026.07.16.0000.0000")),
            Some("2026.04.21.0000.0000")
        );
        assert_eq!(
            pick_version(branches, &version("2020.01.01.0000.0000")),
            None
        );
    }

    #[test]
    fn only_root_sheets_have_schema_files() {
        assert!(is_schema_name("PlaceName"));
        assert!(is_schema_name("AOZScore"));
        assert!(!is_schema_name("quest/000/ClsArc001_00001"));
        assert!(!is_schema_name("../PlaceName"));
        assert!(!is_schema_name(""));
    }
}
