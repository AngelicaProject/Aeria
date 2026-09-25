//! Local, bounded recent-project registry persistence.
//!
//! This crate deliberately knows nothing about Tauri, the workspace format, or
//! source-package validation. The desktop layer supplies metadata from an
//! already successful project operation.

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

pub const FORMAT_VERSION: u32 = 1;
pub const RETENTION_LIMIT: usize = 50;
pub const MAX_REGISTRY_FILE_BYTES: u64 = 256 * 1024;

/// The filename used by the desktop application under its app-data directory.
pub const REGISTRY_FILE_NAME: &str = "projects-v1.json";

const PARTIAL_FILE_NAME: &str = "projects-v1.json.partial";
const PREVIOUS_FILE_NAME: &str = "projects-v1.json.previous";
const LOCK_FILE_NAME: &str = "projects-v1.json.lock";

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

struct RegistryLock {
    file: File,
}

impl Drop for RegistryLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
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
        let _lock = self.acquire_lock()?;
        self.load_locked()
    }

    fn load_locked(&self) -> Result<Vec<RegistryEntry>, RegistryError> {
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
        let _lock = self.acquire_lock()?;
        let mut projects = self.load_locked()?;

        #[cfg(test)]
        pause_test_writer_after_load();

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

    /// Points entries whose source package lies under `from` at the same
    /// relative path under `to`, after the package folder was moved there.
    /// Paths are compared without the Windows verbatim prefix; rewritten
    /// paths use the canonical form of `to`. Returns the number of entries
    /// changed; nothing is written when none match.
    ///
    /// # Errors
    ///
    /// Returns a typed error when `to` cannot be canonicalized, the registry
    /// is invalid, or the updated document cannot be published.
    pub fn relocate_source_packages(&self, from: &Path, to: &Path) -> Result<usize, RegistryError> {
        let to = canonicalize(to)?;
        let from = without_verbatim_prefix(from);
        let _lock = self.acquire_lock()?;
        let mut projects = self.load_locked()?;
        let mut changed = 0;
        for project in &mut projects {
            let path = without_verbatim_prefix(Path::new(&project.source_package_path));
            if let Ok(relative) = path.strip_prefix(&from) {
                project.source_package_path = to.join(relative).to_string_lossy().into_owned();
                changed += 1;
            }
        }
        if changed > 0 {
            self.write_document(&RegistryDocument {
                format_version: FORMAT_VERSION,
                projects,
            })?;
        }
        Ok(changed)
    }

    /// Removes exactly one local entry without touching any project files.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the registry is invalid, the ID is absent,
    /// or the updated document cannot be published.
    pub fn remove(&self, id: &str) -> Result<(), RegistryError> {
        let _lock = self.acquire_lock()?;
        let mut projects = self.load_locked()?;
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
        let file =
            File::open(path).map_err(|source| io_error("read registry file", path, source))?;
        let length = file
            .metadata()
            .map_err(|source| io_error("inspect registry file", path, source))?
            .len();
        if length > MAX_REGISTRY_FILE_BYTES {
            return invalid_data(
                path,
                format!("registry file exceeds the {MAX_REGISTRY_FILE_BYTES}-byte limit"),
            );
        }
        let mut contents = Vec::new();
        file.take(MAX_REGISTRY_FILE_BYTES + 1)
            .read_to_end(&mut contents)
            .map_err(|source| io_error("read registry file", path, source))?;
        if contents.len() as u64 > MAX_REGISTRY_FILE_BYTES {
            return invalid_data(
                path,
                format!("registry file exceeds the {MAX_REGISTRY_FILE_BYTES}-byte limit"),
            );
        }
        let value: Value =
            serde_json::from_slice(&contents).map_err(|source| RegistryError::InvalidJson {
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
        if bytes.len() as u64 > MAX_REGISTRY_FILE_BYTES {
            return invalid_data(
                &self.path,
                format!("serialized registry exceeds the {MAX_REGISTRY_FILE_BYTES}-byte limit"),
            );
        }
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

    fn lock_path(&self) -> PathBuf {
        sibling(&self.path, LOCK_FILE_NAME)
    }

    fn acquire_lock(&self) -> Result<RegistryLock, RegistryError> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|source| io_error("create registry directory", parent, source))?;
        let path = self.lock_path();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| io_error("open registry lock file", &path, source))?;
        file.lock_exclusive()
            .map_err(|source| io_error("lock registry file", &path, source))?;
        Ok(RegistryLock { file })
    }
}

/// Drops the Windows verbatim prefix (`\\?\`) of a drive path.
fn without_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_owned(),
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

    if document.projects.len() > RETENTION_LIMIT {
        return invalid_data(
            path,
            format!("projects must contain at most {RETENTION_LIMIT} entries"),
        );
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

fn invalid_data<T>(path: &Path, message: impl Into<String>) -> Result<T, RegistryError> {
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
fn pause_test_writer_after_load() {
    let Ok(index) = std::env::var("AERIA_PROJECTS_TEST_WRITER_INDEX") else {
        return;
    };
    if index != "0" {
        return;
    }
    let Ok(ready_path) = std::env::var("AERIA_PROJECTS_TEST_WRITER_READY") else {
        return;
    };
    let Ok(release_path) = std::env::var("AERIA_PROJECTS_TEST_WRITER_RELEASE") else {
        return;
    };
    fs::write(ready_path, b"ready").expect("signal test writer lock acquisition");
    while !Path::new(&release_path).exists() {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};
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
    fn moved_source_packages_are_relocated() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        let old_root = temp.path().join("старые данные");
        let new_root = temp.path().join("Новые данные Aeria");
        fs::create_dir_all(&new_root).expect("new root");
        let moved = |name: &str| format!("{}", old_root.join(format!("{name}.hsp")).display());
        let mut inside = entry("inside", 20);
        inside.source_package_path = format!(r"\\?\{}", moved("inside"));
        let mut plain = entry("plain", 10);
        plain.source_package_path = moved("plain");
        let outside = entry("outside", 5);
        fs::write(
            store.path(),
            serde_json::to_vec(&document(vec![inside, plain, outside.clone()])).expect("json"),
        )
        .expect("write");

        // Only Windows drive paths carry a verbatim prefix.
        let root_prefixed = cfg!(windows);
        let changed = store
            .relocate_source_packages(&old_root, &new_root)
            .expect("relocate");
        let canonical = fs::canonicalize(&new_root).expect("canonical");
        let loaded = store.load().expect("load");
        let path_of = |id: &str| {
            loaded
                .iter()
                .find(|project| project.id == id)
                .map(|project| project.source_package_path.clone())
                .expect(id)
        };
        assert_eq!(changed, if root_prefixed { 2 } else { 1 });
        assert_eq!(
            path_of("plain"),
            canonical.join("plain.hsp").to_string_lossy()
        );
        if root_prefixed {
            assert_eq!(
                path_of("inside"),
                canonical.join("inside.hsp").to_string_lossy()
            );
        }
        assert_eq!(path_of("outside"), outside.source_package_path);
        assert_eq!(
            store
                .relocate_source_packages(&old_root, &new_root)
                .expect("again"),
            0
        );
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
    fn oversized_registry_is_rejected_before_deserialization() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        let mut oversized = br#"{"formatVersion":1,"projects":[]}"#.to_vec();
        oversized.resize(
            usize::try_from(MAX_REGISTRY_FILE_BYTES + 1).expect("test size fits usize"),
            b' ',
        );
        fs::write(store.path(), oversized).expect("write");
        assert!(matches!(
            store.load(),
            Err(RegistryError::InvalidData { .. })
        ));
    }

    #[test]
    fn documents_with_more_than_the_retention_limit_are_rejected() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        let projects = (0..=RETENTION_LIMIT)
            .map(|index| entry(&format!("project-{index}"), index as u64))
            .collect();
        fs::write(
            store.path(),
            serde_json::to_vec(&document(projects)).expect("json"),
        )
        .expect("write");
        assert!(matches!(
            store.load(),
            Err(RegistryError::InvalidData { .. })
        ));
    }

    #[test]
    fn oversized_upsert_preserves_existing_registry_and_is_loadable() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        store
            .upsert(&metadata(&temp, "existing"), 1)
            .expect("initial upsert");
        let original = fs::read(store.path()).expect("original registry");

        let mut oversized = metadata(&temp, "oversized");
        oversized.game_version =
            "g".repeat(usize::try_from(MAX_REGISTRY_FILE_BYTES).expect("test size fits usize"));
        let error = store
            .upsert(&oversized, 2)
            .expect_err("oversized registry write");
        assert!(matches!(error, RegistryError::InvalidData { .. }));
        assert_eq!(fs::read(store.path()).expect("final registry"), original);
        assert!(!store.partial_path().exists());
        assert_eq!(store.load().expect("load preserved registry").len(), 1);
    }

    #[test]
    fn oversized_first_write_leaves_no_registry_artifacts() {
        let temp = tempdir().expect("tempdir");
        let store = registry(&temp);
        let mut oversized = metadata(&temp, "oversized");
        oversized.game_version =
            "g".repeat(usize::try_from(MAX_REGISTRY_FILE_BYTES).expect("test size fits usize"));

        let error = store
            .upsert(&oversized, 1)
            .expect_err("oversized registry write");
        assert!(matches!(error, RegistryError::InvalidData { .. }));
        assert!(!store.path().exists());
        assert!(!store.partial_path().exists());
        assert!(store.load().expect("load missing registry").is_empty());
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

    #[test]
    fn registry_writer_helper_process() {
        let Some(root) = std::env::var_os("AERIA_PROJECTS_TEST_WRITER_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let index = std::env::var("AERIA_PROJECTS_TEST_WRITER_INDEX")
            .expect("writer index")
            .parse::<usize>()
            .expect("numeric writer index");
        let repository_root = root.join(format!("project-{index}"));
        fs::create_dir_all(&repository_root).expect("repository");
        let source_package_path = root.join(format!("project-{index}.hsp"));
        fs::write(&source_package_path, b"hsp").expect("source package");
        let metadata = ProjectMetadata {
            repository_root,
            source_package_path,
            source_package_id: PACKAGE_ID.to_owned(),
            source_language: "en".to_owned(),
            target_language: "fr".to_owned(),
            game_version: "test-game".to_owned(),
        };
        ProjectRegistry::new(root.join(REGISTRY_FILE_NAME))
            .upsert(&metadata, index as u64)
            .expect("concurrent upsert");
    }

    #[test]
    fn concurrent_process_upserts_preserve_all_entries() {
        let temp = tempdir().expect("tempdir");
        let ready_path = temp.path().join("writer-ready");
        let release_path = temp.path().join("writer-release");
        let executable = std::env::current_exe().expect("test executable");
        let helper = "tests::registry_writer_helper_process";
        let mut children = Vec::new();
        children.push(
            Command::new(&executable)
                .args(["--exact", helper])
                .env("AERIA_PROJECTS_TEST_WRITER_ROOT", temp.path())
                .env("AERIA_PROJECTS_TEST_WRITER_INDEX", "0")
                .env("AERIA_PROJECTS_TEST_WRITER_READY", &ready_path)
                .env("AERIA_PROJECTS_TEST_WRITER_RELEASE", &release_path)
                .spawn()
                .expect("first writer"),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready_path.exists() {
            assert!(
                Instant::now() < deadline,
                "first writer did not acquire lock"
            );
            thread::sleep(Duration::from_millis(10));
        }

        for index in 1..=8 {
            children.push(
                Command::new(&executable)
                    .args(["--exact", helper])
                    .env("AERIA_PROJECTS_TEST_WRITER_ROOT", temp.path())
                    .env("AERIA_PROJECTS_TEST_WRITER_INDEX", index.to_string())
                    .spawn()
                    .expect("concurrent writer"),
            );
        }
        fs::write(&release_path, b"release").expect("release writers");

        for mut child in children {
            assert!(child.wait().expect("writer status").success());
        }

        let projects = registry(&temp).load().expect("load concurrent registry");
        assert_eq!(projects.len(), 9);
        for index in 0..=8 {
            assert!(projects.iter().any(|project| {
                project
                    .repository_root
                    .ends_with(format!("project-{index}").as_str())
            }));
        }
    }
}
