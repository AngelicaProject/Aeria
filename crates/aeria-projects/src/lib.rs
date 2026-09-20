//! Local, bounded recent-project registry persistence.
//!
//! This crate deliberately knows nothing about Tauri, Workspace Format v1, or
//! source-package validation. The desktop layer supplies metadata from an
//! already successful project operation.

#![forbid(unsafe_code)]

use std::collections::HashSet;
#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

pub const FORMAT_VERSION: u32 = 1;
pub const RETENTION_LIMIT: usize = 50;

/// The filename used by the desktop application under its app-data directory.
pub const REGISTRY_FILE_NAME: &str = "projects-v1.json";

const PARTIAL_FILE_NAME: &str = "projects-v1.json.partial";
const PREVIOUS_FILE_NAME: &str = "projects-v1.json.previous";

/// Metadata obtained from an already opened or initialized project.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectMetadata {
    pub repository_root: PathBuf,
    pub source_package_path: PathBuf,
    pub source_package_id: String,
    pub source_language: String,
    pub target_language: String,
    pub game_version: String,
}

/// One local recent-project record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    pub id: String,
    #[serde(rename = "repositoryRoot")]
    pub repository_root: String,
    #[serde(rename = "sourcePackagePath")]
    pub source_package_path: String,
    #[serde(rename = "sourcePackageId")]
    pub source_package_id: String,
    #[serde(rename = "sourceLanguage")]
    pub source_language: String,
    #[serde(rename = "targetLanguage")]
    pub target_language: String,
    #[serde(rename = "gameVersion")]
    pub game_version: String,
    #[serde(rename = "lastOpenedAtUnixMs")]
    pub last_opened_at_unix_ms: u64,
}

/// The complete versioned registry document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryDocument {
    #[serde(rename = "formatVersion")]
    pub format_version: u32,
    pub projects: Vec<RegistryEntry>,
}

/// Errors raised while loading, validating, or publishing the local registry.
#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("registry filesystem operation '{operation}' failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },

    #[error("invalid recent-project registry JSON at {path}: {message}")]
    InvalidJson { path: PathBuf, message: String },

    #[error("unsupported recent-project registry format version {version} in {path}")]
    UnsupportedVersion { path: PathBuf, version: u64 },

    #[error("invalid recent-project registry data at {path}: {message}")]
    InvalidData { path: PathBuf, message: String },

    #[error("failed to serialize recent-project registry for {path}: {source}")]
    Serialization {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("failed to publish recent-project registry {path}: {source}")]
    AtomicPublication { path: PathBuf, source: io::Error },

    #[error("recent project {id:?} was not found")]
    EntryNotFound { id: String },
}

/// A filesystem-backed local registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectRegistry {
    path: PathBuf,
}

impl ProjectRegistry {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the registry without canonicalizing any stored paths.
    ///
    /// Missing state is the normal empty first-run state. Owned partial state
    /// is never read as a registry; it is cleaned when possible. A valid
    /// previous file is recovered only when the final file is absent.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the registry is malformed, unsupported, or
    /// cannot be read or recovered.
    pub fn load(&self) -> Result<Vec<RegistryEntry>, RegistryError> {
        self.cleanup_partial()?;

        if self.path.is_file() {
            let mut projects = Self::read_document(&self.path)?.projects;
            sort_projects(&mut projects);
            return Ok(projects);
        }

        if self.path.exists() {
            return Err(io_error(
                "inspect registry file",
                &self.path,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "registry path is not a regular file",
                ),
            ));
        }

        let previous = self.previous_path();
        if previous.is_file() {
            let document = Self::read_document(&previous)?;
            fs::rename(&previous, &self.path)
                .map_err(|source| io_error("recover previous registry file", &self.path, source))?;
            #[cfg(unix)]
            sync_parent(&self.path).map_err(|source| {
                io_error("sync recovered registry directory", &self.path, source)
            })?;
            let mut projects = document.projects;
            sort_projects(&mut projects);
            return Ok(projects);
        }

        if previous.exists() {
            return Err(io_error(
                "inspect registry recovery file",
                &previous,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "registry recovery path is not a regular file",
                ),
            ));
        }

        Ok(Vec::new())
    }

    /// Upserts a successful project and publishes the bounded registry.
    ///
    /// Both paths are canonicalized here, and only here, so merely loading
    /// stale entries never makes them disappear or changes their display data.
    ///
    /// # Errors
    ///
    /// Returns a typed error when either project path cannot be canonicalized,
    /// the existing registry is invalid, or the new document cannot be
    /// published.
    pub fn upsert(
        &self,
        metadata: &ProjectMetadata,
        last_opened_at_unix_ms: u64,
    ) -> Result<RegistryEntry, RegistryError> {
        let repository_root = canonicalize(&metadata.repository_root)?;
        let source_package_path = canonicalize(&metadata.source_package_path)?;
        let mut projects = self.load()?;

        let existing = projects
            .iter()
            .position(|project| Path::new(&project.repository_root) == repository_root);
        let id = existing
            .and_then(|index| projects.get(index).map(|project| project.id.clone()))
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let entry = RegistryEntry {
            id,
            repository_root: repository_root.to_string_lossy().into_owned(),
            source_package_path: source_package_path.to_string_lossy().into_owned(),
            source_package_id: metadata.source_package_id.clone(),
            source_language: metadata.source_language.clone(),
            target_language: metadata.target_language.clone(),
            game_version: metadata.game_version.clone(),
            last_opened_at_unix_ms,
        };

        if let Some(index) = existing {
            projects[index] = entry.clone();
        } else {
            projects.push(entry.clone());
        }
        sort_and_prune(&mut projects);
        self.write_document(&RegistryDocument {
            format_version: FORMAT_VERSION,
            projects,
        })?;
        Ok(entry)
    }

    /// Removes exactly one local entry without touching any project files.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the registry is invalid, the ID is absent,
    /// or the updated document cannot be published.
    pub fn remove(&self, id: &str) -> Result<(), RegistryError> {
        let mut projects = self.load()?;
        let original_len = projects.len();
        projects.retain(|project| project.id != id);
        if projects.len() == original_len {
            return Err(RegistryError::EntryNotFound { id: id.to_owned() });
        }
        self.write_document(&RegistryDocument {
            format_version: FORMAT_VERSION,
            projects,
        })
    }

    fn read_document(path: &Path) -> Result<RegistryDocument, RegistryError> {
        let contents = fs::read_to_string(path)
            .map_err(|source| io_error("read registry file", path, source))?;
        let value: Value =
            serde_json::from_str(&contents).map_err(|source| RegistryError::InvalidJson {
                path: path.to_owned(),
                message: source.to_string(),
            })?;
        let version = value
            .get("formatVersion")
            .and_then(Value::as_u64)
            .ok_or_else(|| RegistryError::InvalidData {
                path: path.to_owned(),
                message: "formatVersion must be an unsigned integer".to_owned(),
            })?;
        if version != u64::from(FORMAT_VERSION) {
            return Err(RegistryError::UnsupportedVersion {
                path: path.to_owned(),
                version,
            });
        }
        let document = serde_json::from_value::<RegistryDocument>(value).map_err(|source| {
            RegistryError::InvalidData {
                path: path.to_owned(),
                message: source.to_string(),
            }
        })?;
        validate_document(&document, path)?;
        Ok(document)
    }

    fn write_document(&self, document: &RegistryDocument) -> Result<(), RegistryError> {
        validate_document(document, &self.path)?;
        let bytes = canonical_bytes(document, &self.path)?;
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|source| io_error("create registry directory", parent, source))?;

        let partial = self.partial_path();
        remove_owned_file(&partial)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|source| io_error("create registry temporary file", &partial, source))?;
        file.write_all(&bytes)
            .map_err(|source| io_error("write registry temporary file", &partial, source))?;
        file.flush()
            .map_err(|source| io_error("flush registry temporary file", &partial, source))?;
        file.sync_all()
            .map_err(|source| io_error("sync registry temporary file", &partial, source))?;
        drop(file);

        publish(&partial, &self.path, &self.previous_path())?;
        #[cfg(unix)]
        sync_parent(&self.path)
            .map_err(|source| io_error("sync published registry directory", parent, source))?;
        Ok(())
    }

    fn cleanup_partial(&self) -> Result<(), RegistryError> {
        remove_owned_file(&self.partial_path())
    }

    fn partial_path(&self) -> PathBuf {
        sibling(&self.path, PARTIAL_FILE_NAME)
    }

    fn previous_path(&self) -> PathBuf {
        sibling(&self.path, PREVIOUS_FILE_NAME)
    }
}

fn canonicalize(path: &Path) -> Result<PathBuf, RegistryError> {
    fs::canonicalize(path).map_err(|source| io_error("canonicalize project path", path, source))
}

fn validate_document(document: &RegistryDocument, path: &Path) -> Result<(), RegistryError> {
    if document.format_version != FORMAT_VERSION {
        return Err(RegistryError::UnsupportedVersion {
            path: path.to_owned(),
            version: u64::from(document.format_version),
        });
    }

    let mut ids = HashSet::with_capacity(document.projects.len());
    for project in &document.projects {
        if project.id.trim().is_empty() {
            return invalid_data(path, "project id must not be empty");
        }
        if !ids.insert(&project.id) {
            return invalid_data(path, format!("duplicate project id {:?}", project.id));
        }
        for (name, value) in [
            ("repositoryRoot", &project.repository_root),
            ("sourcePackagePath", &project.source_package_path),
            ("sourceLanguage", &project.source_language),
            ("targetLanguage", &project.target_language),
        ] {
            if value.trim().is_empty() {
                return invalid_data(path, format!("{name} must not be empty"));
            }
        }
        if !is_canonical_package_id(&project.source_package_id) {
            return invalid_data(
                path,
                format!(
                    "sourcePackageId must match sha256:<64 lowercase hex>, got {:?}",
                    project.source_package_id
                ),
            );
        }
    }
    Ok(())
}

fn sort_and_prune(projects: &mut Vec<RegistryEntry>) {
    sort_projects(projects);
    projects.truncate(RETENTION_LIMIT);
}

fn sort_projects(projects: &mut [RegistryEntry]) {
    projects.sort_by(|left, right| {
        right
            .last_opened_at_unix_ms
            .cmp(&left.last_opened_at_unix_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn canonical_bytes(document: &RegistryDocument, path: &Path) -> Result<Vec<u8>, RegistryError> {
    let mut bytes =
        serde_json::to_vec_pretty(document).map_err(|source| RegistryError::Serialization {
            path: path.to_owned(),
            source,
        })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn invalid_data(path: &Path, message: impl Into<String>) -> Result<(), RegistryError> {
    Err(RegistryError::InvalidData {
        path: path.to_owned(),
        message: message.into(),
    })
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> RegistryError {
    RegistryError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}

fn remove_owned_file(path: &Path) -> Result<(), RegistryError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(
            "remove owned registry temporary file",
            path,
            source,
        )),
    }
}

fn sibling(path: &Path, name: &str) -> PathBuf {
    path.parent().unwrap_or_else(|| Path::new(".")).join(name)
}

#[cfg(not(windows))]
fn publish(partial: &Path, final_path: &Path, previous: &Path) -> Result<(), RegistryError> {
    fs::rename(partial, final_path).map_err(|source| RegistryError::AtomicPublication {
        path: final_path.to_owned(),
        source,
    })?;
    remove_owned_file(previous)
}

#[cfg(windows)]
fn publish(partial: &Path, final_path: &Path, previous: &Path) -> Result<(), RegistryError> {
    if final_path.exists() {
        remove_owned_file(previous)?;
        fs::rename(final_path, previous).map_err(|source| RegistryError::AtomicPublication {
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
                format!("{source}; restoring previous registry also failed: {restore_error}")
            }
        };
        return Err(RegistryError::AtomicPublication {
            path: final_path.to_owned(),
            source: io::Error::other(message),
        });
    }
    remove_owned_file(previous)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

/// Returns whether a package identity has the canonical HSP SHA-256 spelling.
#[must_use]
pub fn is_canonical_package_id(value: &str) -> bool {
    value.len() == "sha256:".len() + 64
        && value.starts_with("sha256:")
        && value["sha256:".len()..]
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{TempDir, tempdir};

    const PACKAGE_ID: &str =
        "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn registry(temp: &TempDir) -> ProjectRegistry {
        ProjectRegistry::new(temp.path().join(REGISTRY_FILE_NAME))
    }

    fn metadata(temp: &TempDir, name: &str) -> ProjectMetadata {
        let repository_root = temp.path().join(name);
        fs::create_dir_all(&repository_root).expect("repository");
        let source_package_path = temp.path().join(format!("{name}.hsp"));
        fs::write(&source_package_path, b"hsp").expect("source package");
        ProjectMetadata {
            repository_root,
            source_package_path,
            source_package_id: PACKAGE_ID.to_owned(),
            source_language: "en".to_owned(),
            target_language: "fr".to_owned(),
            game_version: "test-game".to_owned(),
        }
    }

    fn document(projects: Vec<RegistryEntry>) -> RegistryDocument {
        RegistryDocument {
            format_version: FORMAT_VERSION,
            projects,
        }
    }

    fn entry(id: &str, timestamp: u64) -> RegistryEntry {
        RegistryEntry {
            id: id.to_owned(),
            repository_root: format!("C:/projects/{id}"),
            source_package_path: format!("C:/sources/{id}.hsp"),
            source_package_id: PACKAGE_ID.to_owned(),
            source_language: "en".to_owned(),
            target_language: "fr".to_owned(),
            game_version: "test".to_owned(),
            last_opened_at_unix_ms: timestamp,
        }
    }

    #[test]
    fn missing_file_is_empty() {
        let temp = tempdir().expect("tempdir");
        assert!(registry(&temp).load().expect("missing registry").is_empty());
    }

    #[test]
    fn valid_v1_round_trip_is_pretty_and_canonical() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        let first = store.upsert(&metadata(&temp, "one"), 10).expect("upsert");
        let loaded = store.load().expect("load");
        assert_eq!(loaded, vec![first]);
        let contents = fs::read_to_string(store.path()).expect("contents");
        assert!(contents.contains("\n  \"formatVersion\": 1,"));
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn unsupported_version_is_explicit() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        fs::write(store.path(), r#"{"formatVersion":2,"projects":[]}"#).expect("write");
        assert!(matches!(
            store.load(),
            Err(RegistryError::UnsupportedVersion { version: 2, .. })
        ));
    }

    #[test]
    fn malformed_json_is_explicit_and_preserves_original() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        let original = b"{not-json";
        fs::write(store.path(), original).expect("write");
        assert!(matches!(
            store.load(),
            Err(RegistryError::InvalidJson { .. })
        ));
        assert_eq!(fs::read(store.path()).expect("contents"), original);
    }

    #[test]
    fn duplicate_ids_and_invalid_package_ids_are_rejected() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        let duplicate = document(vec![entry("same", 2), entry("same", 1)]);
        fs::write(store.path(), serde_json::to_vec(&duplicate).expect("json")).expect("write");
        assert!(matches!(
            store.load(),
            Err(RegistryError::InvalidData { .. })
        ));

        let mut invalid = entry("different", 1);
        invalid.source_package_id = "sha256:ABC".to_owned();
        fs::write(
            store.path(),
            serde_json::to_vec(&document(vec![invalid])).expect("json"),
        )
        .expect("write");
        assert!(matches!(
            store.load(),
            Err(RegistryError::InvalidData { .. })
        ));
    }

    #[test]
    fn persisted_order_is_newest_first_with_id_tie_breaker() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        for (name, timestamp) in [("old", 1), ("new", 3), ("middle", 2)] {
            store
                .upsert(&metadata(&temp, name), timestamp)
                .expect("upsert");
        }
        let loaded = store.load().expect("load");
        assert_eq!(
            loaded
                .iter()
                .map(|project| Path::new(&project.repository_root)
                    .file_name()
                    .expect("name")
                    .to_string_lossy()
                    .into_owned())
                .collect::<Vec<_>>(),
            vec!["new", "middle", "old"]
        );
    }

    #[test]
    fn equal_timestamps_use_local_id_as_a_deterministic_tie_breaker() {
        let mut projects = vec![entry("b", 1), entry("a", 1)];
        sort_and_prune(&mut projects);
        assert_eq!(
            projects
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn same_canonical_repository_preserves_id_and_refreshes_association() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        let first = store.upsert(&metadata(&temp, "one"), 1).expect("first");
        let mut refreshed = metadata(&temp, "one");
        refreshed.source_package_path = temp.path().join("replacement.hsp");
        fs::write(&refreshed.source_package_path, b"hsp").expect("replacement");
        refreshed.source_package_id = PACKAGE_ID.replace('0', "f");
        refreshed.target_language = "de".to_owned();
        let second = store.upsert(&refreshed, 2).expect("refresh");
        assert_eq!(second.id, first.id);
        assert_eq!(second.target_language, "de");
        assert_eq!(second.last_opened_at_unix_ms, 2);
        assert_eq!(store.load().expect("load").len(), 1);
    }

    #[test]
    fn retention_prunes_oldest_entries() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        for index in 0..=RETENTION_LIMIT {
            store
                .upsert(&metadata(&temp, &format!("project-{index}")), index as u64)
                .expect("upsert");
        }
        let loaded = store.load().expect("load");
        assert_eq!(loaded.len(), RETENTION_LIMIT);
        assert!(
            loaded
                .iter()
                .all(|project| !project.repository_root.ends_with("project-0"))
        );
    }

    #[test]
    fn remove_only_removes_registry_entry() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        let project = store.upsert(&metadata(&temp, "one"), 1).expect("upsert");
        store.remove(&project.id).expect("remove");
        assert!(store.load().expect("load").is_empty());
        assert!(temp.path().join("one").is_dir());
        assert!(temp.path().join("one.hsp").is_file());
    }

    #[test]
    fn partial_is_never_loaded_and_valid_final_wins_over_previous() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        store.upsert(&metadata(&temp, "final"), 2).expect("final");
        fs::write(store.partial_path(), b"{not-json").expect("partial");
        fs::write(
            store.previous_path(),
            serde_json::to_vec(&document(vec![entry("previous", 1)])).expect("json"),
        )
        .expect("previous");
        let loaded = store.load().expect("final wins");
        assert!(loaded[0].repository_root.ends_with("final"));
        assert!(!store.partial_path().exists());
    }

    #[test]
    fn valid_previous_is_recovered_when_final_is_missing() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        fs::write(
            store.previous_path(),
            serde_json::to_vec(&document(vec![entry("previous", 1)])).expect("json"),
        )
        .expect("previous");
        let loaded = store.load().expect("recover");
        assert_eq!(loaded[0].id, "previous");
        assert!(store.path().is_file());
        assert!(!store.previous_path().exists());
    }

    #[test]
    fn malformed_final_is_not_replaced_by_recovery_state() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        fs::write(store.path(), b"{not-json").expect("final");
        fs::write(
            store.previous_path(),
            serde_json::to_vec(&document(vec![entry("previous", 1)])).expect("json"),
        )
        .expect("previous");
        assert!(matches!(
            store.load(),
            Err(RegistryError::InvalidJson { .. })
        ));
        assert_eq!(fs::read(store.path()).expect("final bytes"), b"{not-json");
    }
}
