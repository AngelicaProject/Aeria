//! Production Workspace Format filesystem persistence.
//!
//! The writer produces Workspace Format v2 only. The reader also accepts
//! Workspace Format v1 so that a source update can migrate it; a v1
//! workspace is never activated for ordinary editing.
//!
//! The DTOs in this module are deliberately private. They describe the file
//! contract without making the domain types in `aeria-core` serialization
//! types.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use aeria_core::{
    DetachReason, ReviewState, Sha256Hash, SourceBinding, SourceFingerprint, SourceLayout,
    SourceStatus, TranslationUnit, TranslationUnitId, WorkspaceMetadata,
};
use aeria_se::{SemanticValidity, parse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::{NamedTempFile, TempDir};
use thiserror::Error;

use super::{Workspace, WorkspaceError};

/// The Workspace Format version written by this implementation.
pub(crate) const FORMAT_VERSION: u8 = 2;
/// The previous Workspace Format version, readable only for migration.
const LEGACY_FORMAT_VERSION: u8 = 1;
const AERIA_DIRECTORY: &str = ".aeria";
const MANIFEST_FILE: &str = "manifest.json";
const UNITS_DIRECTORY: &str = "units";
const BOM: &[u8; 3] = b"\xef\xbb\xbf";

#[cfg(test)]
static FAIL_BEFORE_PUBLICATION: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static READ_SHARD_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static PERSISTENCE_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn fail_next_publication_for_test() {
    FAIL_BEFORE_PUBLICATION.store(true, Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) fn persistence_test_lock() -> std::sync::MutexGuard<'static, ()> {
    PERSISTENCE_TEST_LOCK.lock().expect("persistence test lock")
}

/// Errors raised while opening, validating, or persisting a Workspace Format
/// repository.
#[derive(Debug, Error)]
pub enum WorkspaceStoreError {
    /// The repository root could not be inspected or is not a directory.
    #[error("workspace repository root {path} is unavailable: {source}")]
    RepositoryRoot { path: PathBuf, source: io::Error },

    /// A managed path is not the expected safe filesystem object.
    #[error("unsafe or invalid managed workspace path {path}: {reason}")]
    ManagedPath { path: PathBuf, reason: String },

    /// A required managed path is absent.
    #[error("required workspace path is missing: {path}")]
    MissingPath { path: PathBuf },

    /// An existing project cannot be initialized over.
    #[error("Aeria workspace already exists at {path}")]
    AlreadyInitialized { path: PathBuf },

    /// A JSON or semantic value violates the reader contract.
    #[error("invalid Workspace Format data at {path}{line}: {message}")]
    InvalidData {
        path: PathBuf,
        line: String,
        message: String,
    },

    /// A format version newer than the implementation is not silently opened.
    #[error("unsupported Workspace Format version {version} in {path}")]
    UnsupportedFormatVersion { path: PathBuf, version: u64 },

    /// A readable older format must be migrated by a source update before
    /// it can be edited.
    #[error("Workspace Format version {version} in {path} must be migrated by a source update")]
    MigrationRequired { path: PathBuf, version: u8 },

    /// JSON serialization failed before publication.
    #[error("failed to serialize canonical workspace data for {path}: {source}")]
    Serialization {
        path: PathBuf,
        source: serde_json::Error,
    },

    /// The canonical replacement could not be published.
    #[error("failed to atomically publish canonical workspace file {path}: {source}")]
    AtomicPublication { path: PathBuf, source: io::Error },

    /// The managed workspace changed after this store loaded its session
    /// cache. The caller must reload before retrying the mutation.
    #[error("managed workspace state changed since it was loaded: {path}")]
    ExternalChange { path: PathBuf },

    /// A normal filesystem operation failed with its path and operation.
    #[error("filesystem operation '{operation}' failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },

    /// A loaded unit could not be inserted into the domain workspace.
    #[error("loaded workspace is invalid: {0}")]
    Domain(#[from] super::WorkspaceError),
}

/// Persistence adapter bound to one repository root.
///
/// The interface deliberately keeps the persistence seam small: loading and
/// initialization operate on complete workspaces, while ordinary edits
/// rewrite only the stable ID shard selected by [`TranslationUnitId`].
#[derive(Clone, Debug)]
pub struct WorkspaceStore {
    repository_root: PathBuf,
    session_cache: Arc<Mutex<Option<PersistenceCache>>>,
}

impl PartialEq for WorkspaceStore {
    fn eq(&self, other: &Self) -> bool {
        self.repository_root == other.repository_root
    }
}

impl Eq for WorkspaceStore {}

/// Complete validated persisted state, before it is activated for editing.
///
/// Bound units are unique by binding within one source-layout generation,
/// which admits the intermediate state left by an interrupted source update.
/// Activation additionally requires the current format and unique bound
/// bindings.
#[derive(Clone, Debug)]
pub(crate) struct StoredWorkspace {
    pub(crate) format_version: u8,
    pub(crate) metadata: WorkspaceMetadata,
    pub(crate) units: BTreeMap<TranslationUnitId, TranslationUnit>,
    /// Bound units whose binding another bound unit already claims. Every
    /// ordinary mutation keeps bindings unique, but a Git merge of branches
    /// that bound different units to one occurrence can produce them. They
    /// are resolved by a source update, never by the reader.
    pub(crate) duplicate_bound_bindings: usize,
    layout: ExistingLayout,
}

#[derive(Clone, Debug)]
struct PersistenceCache {
    layout: ExistingLayout,
    metadata: WorkspaceMetadata,
    units: BTreeMap<TranslationUnitId, TranslationUnit>,
    managed_paths: Vec<ManagedPathState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManagedPathState {
    path: PathBuf,
    exists: bool,
    is_dir: bool,
    size: u64,
    modified: Option<SystemTime>,
    content_hash: Option<[u8; 32]>,
}

impl WorkspaceStore {
    /// Binds a persistence adapter to a repository root.
    #[must_use]
    pub fn new(repository_root: impl Into<PathBuf>) -> Self {
        Self {
            repository_root: repository_root.into(),
            session_cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the repository root used by this adapter.
    #[must_use]
    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    /// Loads and validates the complete canonical workspace state for
    /// editing.
    ///
    /// The repository root is only a project container. Only `.aeria/` and
    /// its defined managed entries are inspected.
    ///
    /// # Errors
    ///
    /// Returns an error when the managed namespace is missing, unsafe,
    /// malformed, contains invalid unit data, or uses Workspace Format v1,
    /// which must first be migrated by a source update.
    pub fn load(&self) -> Result<Workspace, WorkspaceStoreError> {
        let stored = self.read_stored()?;
        self.activate(stored)
    }

    /// Reads and validates only the manifest, in either supported format.
    ///
    /// # Errors
    ///
    /// Returns an error when the managed namespace or manifest is missing,
    /// unsafe, malformed, or uses an unsupported format version.
    pub fn read_metadata(&self) -> Result<WorkspaceMetadata, WorkspaceStoreError> {
        let layout = self.inspect_existing_layout()?;
        read_manifest(&layout.manifest_path).map(|(_, metadata)| metadata)
    }

    /// Reads and validates every managed file in either supported format.
    pub(crate) fn read_stored(&self) -> Result<StoredWorkspace, WorkspaceStoreError> {
        let trace = PerfTrace::new();
        let layout = self.inspect_existing_layout()?;
        trace.mark("workspace.layout");
        let (format_version, metadata) = read_manifest(&layout.manifest_path)?;
        let shape = if format_version == FORMAT_VERSION {
            RecordShape::Current
        } else {
            RecordShape::Legacy
        };

        let mut units = BTreeMap::new();
        let mut bound_bindings = BTreeSet::new();
        let mut duplicate_bound_bindings = 0;
        for shard in &layout.shards {
            for unit in read_shard(&shard.path, shard.shard, &shard.name, shape)? {
                let id = unit.id();
                if unit.is_bound() && !bound_bindings.insert(unit.source_binding().clone()) {
                    duplicate_bound_bindings += 1;
                }
                if units.insert(id, unit).is_some() {
                    return Err(invalid(
                        &shard.path,
                        None,
                        format!("duplicate TranslationUnitId {id}"),
                    ));
                }
            }
        }
        trace.mark("workspace.store-read");
        Ok(StoredWorkspace {
            format_version,
            metadata,
            units,
            duplicate_bound_bindings,
            layout,
        })
    }

    /// Activates validated current-format state for editing and primes the
    /// session cache used by ordinary one-unit persistence.
    pub(crate) fn activate(
        &self,
        stored: StoredWorkspace,
    ) -> Result<Workspace, WorkspaceStoreError> {
        if stored.format_version != FORMAT_VERSION {
            return Err(WorkspaceStoreError::MigrationRequired {
                path: stored.layout.manifest_path,
                version: stored.format_version,
            });
        }
        if stored.duplicate_bound_bindings > 0 {
            return Err(invalid(
                &stored.layout.manifest_path,
                None,
                format!(
                    "duplicate current SourceBinding claimed by {} units; a source update must resolve it",
                    stored.duplicate_bound_bindings
                ),
            ));
        }
        let StoredWorkspace {
            metadata,
            units,
            layout,
            ..
        } = stored;
        let workspace = Workspace::from_loaded(metadata.clone(), units.clone())
            .map_err(WorkspaceStoreError::from)?;
        if let Ok(managed_paths) = capture_managed_paths(&layout) {
            self.replace_session_cache(PersistenceCache {
                layout,
                metadata,
                units,
                managed_paths,
            });
        }
        Ok(workspace)
    }

    /// Publishes the result of a source update: every shard in `shards` is
    /// rewritten from `workspace`, then the manifest is written last.
    ///
    /// Each file is replaced atomically. An interruption leaves the previous
    /// manifest content ID in place, so the next open plans the same update
    /// again; the plan is idempotent over already rewritten shards. The
    /// published state is reloaded and must equal `workspace`.
    pub(crate) fn publish_source_update(
        &self,
        workspace: &Workspace,
        shards: &BTreeSet<u8>,
    ) -> Result<Workspace, WorkspaceStoreError> {
        self.invalidate_session_cache();
        let layout = self.inspect_existing_layout()?;
        let aeria_path = self.repository_root.join(AERIA_DIRECTORY);
        let manifest_bytes = canonical_manifest_bytes(workspace, &layout.manifest_path)?;
        let mut staged = Vec::with_capacity(shards.len());
        for shard in shards {
            let path = aeria_path.join(UNITS_DIRECTORY).join(shard_name(*shard));
            let bytes = canonical_shard_bytes(workspace, *shard, &path)?;
            if bytes.is_empty() {
                return Err(invalid(
                    &path,
                    None,
                    "a source update cannot publish an empty unit shard",
                ));
            }
            staged.push((path, bytes));
        }

        if !staged.is_empty() {
            let units_path = aeria_path.join(UNITS_DIRECTORY);
            if layout.units_path.is_none() {
                fs::create_dir(&units_path).map_err(|source| {
                    io_error("create workspace units directory", &units_path, source)
                })?;
            }
            ensure_directory(&units_path)?;
        }
        for (path, bytes) in &staged {
            ensure_optional_regular_file(path)?;
            atomic_publish(&self.repository_root, path, bytes)?;
        }
        atomic_publish(
            &self.repository_root,
            &layout.manifest_path,
            &manifest_bytes,
        )?;

        let reloaded = self.load()?;
        if reloaded != *workspace {
            self.invalidate_session_cache();
            return Err(invalid(
                &layout.manifest_path,
                None,
                "published source update does not reload as the planned workspace",
            ));
        }
        Ok(reloaded)
    }

    /// Initializes a new `.aeria/` directory from an in-memory workspace.
    ///
    /// The complete directory is serialized under the repository root and is
    /// published only after all canonical bytes have been produced. Existing
    /// `.aeria/` state is never replaced.
    ///
    /// # Errors
    ///
    /// Returns an error when metadata is not representable in v1, the
    /// repository is unavailable, or staging/publication fails.
    pub fn initialize(&self, workspace: &Workspace) -> Result<(), WorkspaceStoreError> {
        self.ensure_repository_root()?;
        validate_workspace_metadata(
            workspace.metadata(),
            &self
                .repository_root
                .join(AERIA_DIRECTORY)
                .join(MANIFEST_FILE),
        )?;

        let aeria_path = self.repository_root.join(AERIA_DIRECTORY);
        if path_exists(&aeria_path)? {
            return Err(WorkspaceStoreError::AlreadyInitialized { path: aeria_path });
        }

        let stage = TempDir::new_in(&self.repository_root).map_err(|source| {
            io_error(
                "create initialization staging directory",
                &self.repository_root,
                source,
            )
        })?;
        let manifest_path = stage.path().join(MANIFEST_FILE);
        let manifest_bytes = canonical_manifest_bytes(workspace, &manifest_path)?;
        write_staging_file(&manifest_path, &manifest_bytes)?;

        let shard_numbers: BTreeSet<u8> =
            workspace.units.keys().map(|id| id.as_bytes()[0]).collect();
        if !shard_numbers.is_empty() {
            let units_path = stage.path().join(UNITS_DIRECTORY);
            fs::create_dir(&units_path).map_err(|source| {
                io_error("create initialization units directory", &units_path, source)
            })?;
            for shard in &shard_numbers {
                let shard_path = units_path.join(shard_name(*shard));
                let bytes = canonical_shard_bytes(workspace, *shard, &shard_path)?;
                write_staging_file(&shard_path, &bytes)?;
            }
        }

        let stage_path = stage.keep();
        if let Err(source) = fs::rename(&stage_path, &aeria_path) {
            let _ = fs::remove_dir_all(&stage_path);
            return Err(io_error(
                "publish initialized workspace directory",
                &aeria_path,
                source,
            ));
        }
        let units_path = if shard_numbers.is_empty() {
            None
        } else {
            Some(aeria_path.join(UNITS_DIRECTORY))
        };
        let layout = ExistingLayout {
            manifest_path: aeria_path.join(MANIFEST_FILE),
            units_path,
            shards: shard_numbers
                .iter()
                .map(|shard| ShardFile {
                    shard: *shard,
                    name: shard_name(*shard),
                    path: aeria_path.join(UNITS_DIRECTORY).join(shard_name(*shard)),
                })
                .collect(),
        };
        if let Ok(managed_paths) = capture_managed_paths(&layout) {
            self.replace_session_cache(PersistenceCache {
                layout,
                metadata: workspace.metadata().clone(),
                units: workspace.units.clone().into_iter().collect(),
                managed_paths,
            });
        }
        Ok(())
    }

    /// Replaces exactly the selected unit in its complete canonical shard.
    ///
    /// The selected persisted shard is fully validated on the fallback path;
    /// an open session reuses its validated layout and unit index. All other
    /// units are preserved. The temporary file is created in the repository
    /// root, outside `.aeria/`, and `tempfile` performs the cross-platform
    /// atomic replacement. Atomicity is guaranteed per canonical file; this
    /// method does not claim a multi-shard filesystem transaction.
    ///
    /// # Errors
    ///
    /// Returns an error when the managed namespace is unsafe or its manifest
    /// does not match the workspace, the requested unit is absent, canonical
    /// serialization fails, or atomic publication fails.
    #[allow(clippy::too_many_lines)]
    pub fn persist_unit(
        &self,
        workspace: &Workspace,
        id: TranslationUnitId,
    ) -> Result<(), WorkspaceStoreError> {
        let trace = PerfTrace::new();
        let replacement = workspace
            .unit(id)
            .cloned()
            .ok_or(WorkspaceStoreError::Domain(WorkspaceError::UnitNotFound {
                id,
            }))?;
        let shard = id.as_bytes()[0];
        let target_path = self
            .repository_root
            .join(AERIA_DIRECTORY)
            .join(UNITS_DIRECTORY)
            .join(shard_name(shard));
        let cached = self.take_session_cache();
        let (layout, persisted_metadata, mut all_persisted_units, using_cache) =
            if let Some(cache) = cached {
                if let Err(error) = verify_cached_managed_paths(
                    &cache.managed_paths,
                    &cache.layout.manifest_path,
                    &target_path,
                ) {
                    self.invalidate_session_cache();
                    return Err(error);
                }
                (cache.layout, cache.metadata, cache.units, true)
            } else {
                let layout = self.inspect_existing_layout()?;
                let (format_version, persisted_metadata) = read_manifest(&layout.manifest_path)?;
                if format_version != FORMAT_VERSION {
                    return Err(WorkspaceStoreError::MigrationRequired {
                        path: layout.manifest_path,
                        version: format_version,
                    });
                }
                (layout, persisted_metadata, BTreeMap::new(), false)
            };
        trace.mark(if using_cache {
            "workspace.persist-unit.cache-check"
        } else {
            "workspace.persist-unit.fallback-load"
        });
        require_metadata_match(
            workspace.metadata(),
            &persisted_metadata,
            &layout.manifest_path,
        )?;

        let mut persisted_units = BTreeMap::new();
        if using_cache {
            for (unit_id, unit) in &all_persisted_units {
                if unit_id.as_bytes()[0] == shard {
                    persisted_units.insert(*unit_id, unit.clone());
                }
            }
        }
        if !using_cache
            && let Some(shard_file) = layout.shards.iter().find(|file| file.shard == shard)
        {
            for unit in read_shard(
                &shard_file.path,
                shard,
                &shard_file.name,
                RecordShape::Current,
            )? {
                persisted_units.insert(unit.id(), unit);
            }
        }
        validate_unique_shard_bindings(&persisted_units, &target_path)?;
        if let Some(persisted) = persisted_units.get(&id) {
            require_persisted_identity(persisted, &replacement, &target_path)?;
        } else if using_cache {
            require_new_binding_is_unowned_cached(
                &all_persisted_units,
                &replacement,
                &target_path,
            )?;
        } else {
            require_new_binding_is_unowned(
                &layout,
                shard,
                &persisted_units,
                &replacement,
                &target_path,
            )?;
        }
        persisted_units.insert(id, replacement.clone());
        let bytes = canonical_units_bytes(persisted_units.values(), &target_path)?;
        trace.mark("workspace.persist-unit.serialize");

        let created_units_path = layout.units_path.is_none();
        let units_path = if let Some(path) = &layout.units_path {
            path.clone()
        } else {
            let path = self
                .repository_root
                .join(AERIA_DIRECTORY)
                .join(UNITS_DIRECTORY);
            fs::create_dir(&path)
                .map_err(|source| io_error("create workspace units directory", &path, source))?;
            ensure_directory(&path)?;
            path
        };
        ensure_directory(&self.repository_root.join(AERIA_DIRECTORY))?;
        ensure_directory(&units_path)?;
        let target_path = units_path.join(shard_name(shard));
        let result = ensure_optional_regular_file(&target_path)
            .and_then(|()| atomic_publish(&self.repository_root, &target_path, &bytes));
        if result.is_err() && created_units_path {
            remove_directory_if_empty(&units_path);
        }
        if result.is_err() {
            self.invalidate_session_cache();
            return result;
        }

        let mut published_layout = layout;
        published_layout.units_path = Some(units_path.clone());
        if !published_layout
            .shards
            .iter()
            .any(|file| file.shard == shard)
        {
            published_layout.shards.push(ShardFile {
                shard,
                name: shard_name(shard),
                path: target_path,
            });
            published_layout
                .shards
                .sort_by(|left, right| left.name.cmp(&right.name));
        }
        if using_cache {
            all_persisted_units.insert(id, replacement);
            if let Ok(managed_paths) = capture_managed_paths(&published_layout) {
                self.replace_session_cache(PersistenceCache {
                    layout: published_layout,
                    metadata: persisted_metadata,
                    units: all_persisted_units,
                    managed_paths,
                });
            }
        }
        trace.mark("workspace.persist-unit.publish");
        Ok(())
    }

    fn take_session_cache(&self) -> Option<PersistenceCache> {
        self.session_cache.lock().ok()?.take()
    }

    fn replace_session_cache(&self, cache: PersistenceCache) {
        if let Ok(mut current) = self.session_cache.lock() {
            *current = Some(cache);
        }
    }

    fn invalidate_session_cache(&self) {
        if let Ok(mut current) = self.session_cache.lock() {
            *current = None;
        }
    }

    fn ensure_repository_root(&self) -> Result<(), WorkspaceStoreError> {
        let metadata = fs::metadata(&self.repository_root).map_err(|source| {
            WorkspaceStoreError::RepositoryRoot {
                path: self.repository_root.clone(),
                source,
            }
        })?;
        if !metadata.is_dir() {
            return Err(WorkspaceStoreError::RepositoryRoot {
                path: self.repository_root.clone(),
                source: io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "repository root is not a directory",
                ),
            });
        }
        Ok(())
    }

    fn inspect_existing_layout(&self) -> Result<ExistingLayout, WorkspaceStoreError> {
        self.ensure_repository_root()?;
        let aeria_path = self.repository_root.join(AERIA_DIRECTORY);
        let aeria_metadata =
            symlink_metadata(&aeria_path)?.ok_or_else(|| WorkspaceStoreError::MissingPath {
                path: aeria_path.clone(),
            })?;
        if aeria_metadata.file_type().is_symlink() {
            return Err(managed_path_error(&aeria_path, "symlinks are not allowed"));
        }
        if !aeria_metadata.is_dir() {
            return Err(managed_path_error(&aeria_path, "expected a directory"));
        }

        let manifest_path = aeria_path.join(MANIFEST_FILE);
        ensure_regular_file(&manifest_path)?;

        let units_candidate = aeria_path.join(UNITS_DIRECTORY);
        let units_path = match symlink_metadata(&units_candidate)? {
            Some(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(managed_path_error(
                        &units_candidate,
                        "symlinks are not allowed",
                    ));
                }
                if !metadata.is_dir() {
                    return Err(managed_path_error(&units_candidate, "expected a directory"));
                }
                Some(units_candidate)
            }
            None => None,
        };

        let mut entries = fs::read_dir(&aeria_path).map_err(|source| {
            io_error("enumerate managed workspace directory", &aeria_path, source)
        })?;
        while let Some(entry) = entries.next().transpose().map_err(|source| {
            io_error(
                "read managed workspace directory entry",
                &aeria_path,
                source,
            )
        })? {
            let path = entry.path();
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| invalid(&path, None, "managed entry name is not valid UTF-8"))?;
            if name != MANIFEST_FILE && name != UNITS_DIRECTORY {
                return Err(invalid(&path, None, "unexpected entry inside .aeria/"));
            }
        }

        let mut shards = Vec::new();
        if let Some(units_path) = &units_path {
            let mut entries = fs::read_dir(units_path).map_err(|source| {
                io_error("enumerate workspace unit shards", units_path, source)
            })?;
            while let Some(entry) = entries
                .next()
                .transpose()
                .map_err(|source| io_error("read workspace unit shard entry", units_path, source))?
            {
                let path = entry.path();
                let metadata = symlink_metadata(&path)?
                    .ok_or_else(|| WorkspaceStoreError::MissingPath { path: path.clone() })?;
                if metadata.file_type().is_symlink() {
                    return Err(managed_path_error(&path, "symlinks are not allowed"));
                }
                let name = entry.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| invalid(&path, None, "unit shard name is not valid UTF-8"))?;
                let Some(shard) = parse_shard_name(name) else {
                    return Err(invalid(&path, None, "invalid unit shard filename"));
                };
                if !metadata.is_file() {
                    return Err(invalid(&path, None, "unit shard must be a regular file"));
                }
                shards.push(ShardFile {
                    shard,
                    name: name.to_owned(),
                    path,
                });
            }
        }
        shards.sort_by(|left, right| left.name.cmp(&right.name));

        Ok(ExistingLayout {
            manifest_path,
            units_path,
            shards,
        })
    }
}

fn capture_managed_paths(
    layout: &ExistingLayout,
) -> Result<Vec<ManagedPathState>, WorkspaceStoreError> {
    let aeria_path = layout
        .manifest_path
        .parent()
        .expect("manifest path must have a parent")
        .to_owned();
    let units_path = layout
        .units_path
        .clone()
        .unwrap_or_else(|| aeria_path.join(UNITS_DIRECTORY));
    let mut paths = vec![aeria_path, layout.manifest_path.clone(), units_path];
    paths.extend(layout.shards.iter().map(|shard| shard.path.clone()));
    paths.sort();
    paths.dedup();
    paths
        .iter()
        .map(|path| managed_path_state(path, true))
        .collect()
}

fn managed_path_state(
    path: &Path,
    include_hash: bool,
) -> Result<ManagedPathState, WorkspaceStoreError> {
    let Some(metadata) = symlink_metadata(path)? else {
        return Ok(ManagedPathState {
            path: path.to_owned(),
            exists: false,
            is_dir: false,
            size: 0,
            modified: None,
            content_hash: None,
        });
    };
    if metadata.file_type().is_symlink() {
        return Err(managed_path_error(path, "symlinks are not allowed"));
    }
    let content_hash = if include_hash && metadata.is_file() {
        Some(hash_managed_file(path)?)
    } else {
        None
    };
    Ok(ManagedPathState {
        path: path.to_owned(),
        exists: true,
        is_dir: metadata.is_dir(),
        size: metadata.len(),
        modified: metadata.modified().ok(),
        content_hash,
    })
}

fn hash_managed_file(path: &Path) -> Result<[u8; 32], WorkspaceStoreError> {
    let mut file =
        File::open(path).map_err(|source| io_error("hash managed workspace file", path, source))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| io_error("hash managed workspace file", path, source))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn verify_cached_managed_paths(
    cached_paths: &[ManagedPathState],
    manifest_path: &Path,
    target_path: &Path,
) -> Result<(), WorkspaceStoreError> {
    if cached_paths.is_empty() {
        return Err(WorkspaceStoreError::ExternalChange {
            path: manifest_path.to_owned(),
        });
    }
    for cached in cached_paths {
        let current = managed_path_state(&cached.path, false)?;
        if cached.exists != current.exists
            || cached.is_dir != current.is_dir
            || cached.size != current.size
            || cached.modified != current.modified
        {
            return Err(WorkspaceStoreError::ExternalChange {
                path: cached.path.clone(),
            });
        }
        if cached.exists
            && !cached.is_dir
            && (cached.path == manifest_path || cached.path == target_path)
        {
            let Some(expected_hash) = cached.content_hash else {
                return Err(WorkspaceStoreError::ExternalChange {
                    path: cached.path.clone(),
                });
            };
            if hash_managed_file(&cached.path)? != expected_hash {
                return Err(WorkspaceStoreError::ExternalChange {
                    path: cached.path.clone(),
                });
            }
        }
    }
    Ok(())
}

struct PerfTrace {
    enabled: bool,
    started: Instant,
    last: std::cell::Cell<Instant>,
}

impl PerfTrace {
    fn new() -> Self {
        Self {
            enabled: std::env::var("AERIA_PERF_TRACE").as_deref() == Ok("1"),
            started: Instant::now(),
            last: std::cell::Cell::new(Instant::now()),
        }
    }

    fn mark(&self, phase: &str) {
        if self.enabled {
            let now = Instant::now();
            let duration = now.duration_since(self.last.get()).as_secs_f64() * 1_000.0;
            self.last.set(now);
            eprintln!(
                "[aeria-perf] {phase}: duration_ms={duration:.3} total_ms={:.3}",
                self.started.elapsed().as_secs_f64() * 1_000.0
            );
        }
    }
}

#[derive(Clone, Debug)]
struct ExistingLayout {
    manifest_path: PathBuf,
    units_path: Option<PathBuf>,
    shards: Vec<ShardFile>,
}

#[derive(Clone, Debug)]
struct ShardFile {
    shard: u8,
    name: String,
    path: PathBuf,
}

/// Reads only the manifest version so the matching strict DTO can be used.
#[derive(Deserialize)]
struct ManifestVersionProbe {
    #[serde(rename = "formatVersion")]
    format_version: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDto {
    #[serde(rename = "formatVersion")]
    _format_version: u64,
    #[serde(rename = "sourceLanguage")]
    source_language: String,
    #[serde(rename = "targetLanguage")]
    target_language: String,
    #[serde(rename = "contentId")]
    content_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyManifestDto {
    #[serde(rename = "formatVersion")]
    _format_version: u64,
    #[serde(rename = "sourceLanguage")]
    source_language: String,
    #[serde(rename = "targetLanguage")]
    target_language: String,
    #[serde(rename = "contentId")]
    content_id: String,
    #[serde(rename = "snapshotId")]
    snapshot_id: String,
}

/// Which unit record shape a reader accepts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordShape {
    /// Workspace Format v2 records only.
    Current,
    /// Workspace Format v1 records only.
    Legacy,
    /// Either shape, for Git history and merge inputs that may predate v2.
    Either,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitDto {
    id: String,
    #[serde(rename = "sourceStatus")]
    source_status: String,
    #[serde(rename = "sourceBinding")]
    source_binding: SourceBindingDto,
    #[serde(rename = "sourceFingerprint")]
    source_fingerprint: SourceFingerprintDto,
    #[serde(rename = "sourceLayout")]
    source_layout: Value,
    #[serde(rename = "sourceRowKey")]
    source_row_key: Value,
    #[serde(rename = "targetMacro")]
    target_macro: String,
    #[serde(rename = "reviewState")]
    review_state: String,
    #[serde(rename = "translatorNote")]
    translator_note: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyUnitDto {
    id: String,
    #[serde(rename = "sourceBinding")]
    source_binding: SourceBindingDto,
    #[serde(rename = "sourceFingerprint")]
    source_fingerprint: SourceFingerprintDto,
    #[serde(rename = "targetMacro")]
    target_macro: String,
    #[serde(rename = "reviewState")]
    review_state: String,
    #[serde(rename = "translatorNote")]
    translator_note: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceLayoutDto {
    #[serde(rename = "sheetSchemaHash")]
    sheet_schema_hash: String,
    #[serde(rename = "columnOffset")]
    column_offset: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceBindingDto {
    #[serde(rename = "sheetName")]
    sheet_name: String,
    #[serde(rename = "rowId")]
    row_id: u32,
    #[serde(rename = "subrowId")]
    subrow_id: u16,
    #[serde(rename = "columnIndex")]
    column_index: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_field_names)]
struct SourceFingerprintDto {
    #[serde(rename = "macroTextHash")]
    macro_text_hash: String,
    #[serde(rename = "rawValueHash")]
    raw_value_hash: Value,
    #[serde(rename = "rowTechnicalHash")]
    row_technical_hash: String,
}

#[derive(Serialize)]
struct CanonicalManifestDto<'a> {
    #[serde(rename = "formatVersion")]
    format_version: u8,
    #[serde(rename = "sourceLanguage")]
    source_language: &'a str,
    #[serde(rename = "targetLanguage")]
    target_language: &'a str,
    #[serde(rename = "contentId")]
    content_id: &'a str,
}

#[derive(Serialize)]
struct CanonicalUnitDto {
    id: String,
    #[serde(rename = "sourceStatus")]
    source_status: &'static str,
    #[serde(rename = "sourceBinding")]
    source_binding: CanonicalSourceBindingDto,
    #[serde(rename = "sourceFingerprint")]
    source_fingerprint: CanonicalSourceFingerprintDto,
    #[serde(rename = "sourceLayout")]
    source_layout: Option<CanonicalSourceLayoutDto>,
    #[serde(rename = "sourceRowKey")]
    source_row_key: Option<String>,
    #[serde(rename = "targetMacro")]
    target_macro: String,
    #[serde(rename = "reviewState")]
    review_state: &'static str,
    #[serde(rename = "translatorNote")]
    translator_note: Option<String>,
}

#[derive(Serialize)]
struct CanonicalSourceBindingDto {
    #[serde(rename = "sheetName")]
    sheet_name: String,
    #[serde(rename = "rowId")]
    row_id: u32,
    #[serde(rename = "subrowId")]
    subrow_id: u16,
    #[serde(rename = "columnIndex")]
    column_index: u32,
}

#[derive(Serialize)]
#[allow(clippy::struct_field_names)]
struct CanonicalSourceFingerprintDto {
    #[serde(rename = "macroTextHash")]
    macro_text_hash: String,
    #[serde(rename = "rawValueHash")]
    raw_value_hash: Option<String>,
    #[serde(rename = "rowTechnicalHash")]
    row_technical_hash: String,
}

#[derive(Serialize)]
struct CanonicalSourceLayoutDto {
    #[serde(rename = "sheetSchemaHash")]
    sheet_schema_hash: String,
    #[serde(rename = "columnOffset")]
    column_offset: u32,
}

fn read_manifest(path: &Path) -> Result<(u8, WorkspaceMetadata), WorkspaceStoreError> {
    let bytes = read_file(path, "read manifest")?;
    let text = decode_json_text(&bytes, path, None, true)?;
    let json_error = |source: serde_json::Error| {
        invalid(
            path,
            Some(source.line()),
            format!("manifest JSON is invalid: {source}"),
        )
    };
    let probe: ManifestVersionProbe = serde_json::from_str(text).map_err(json_error)?;
    let (format_version, source_language, target_language, content_id) =
        if probe.format_version == u64::from(FORMAT_VERSION) {
            let manifest: ManifestDto = serde_json::from_str(text).map_err(json_error)?;
            (
                FORMAT_VERSION,
                manifest.source_language,
                manifest.target_language,
                manifest.content_id,
            )
        } else if probe.format_version == u64::from(LEGACY_FORMAT_VERSION) {
            let manifest: LegacyManifestDto = serde_json::from_str(text).map_err(json_error)?;
            validate_hxs_id(&manifest.snapshot_id, "snapshotId", path, None)?;
            (
                LEGACY_FORMAT_VERSION,
                manifest.source_language,
                manifest.target_language,
                manifest.content_id,
            )
        } else {
            return Err(WorkspaceStoreError::UnsupportedFormatVersion {
                path: path.to_owned(),
                version: probe.format_version,
            });
        };
    validate_hxs_id(&content_id, "contentId", path, None)?;
    let metadata = WorkspaceMetadata::new(source_language, target_language, content_id)
        .map_err(|source| invalid(path, None, format!("invalid manifest metadata: {source}")))?;
    Ok((format_version, metadata))
}

fn read_shard(
    path: &Path,
    shard: u8,
    shard_name: &str,
    shape: RecordShape,
) -> Result<Vec<TranslationUnit>, WorkspaceStoreError> {
    #[cfg(test)]
    READ_SHARD_COUNT.fetch_add(1, Ordering::SeqCst);
    let file = File::open(path).map_err(|source| io_error("open unit shard", path, source))?;
    decode_shard(BufReader::new(file), path, shard, shard_name, shape)
}

/// Returns the repository-relative Workspace Format shard path that stores
/// `id` with `/` separators, for example `.aeria/units/7a.jsonl`.
#[must_use]
pub fn unit_shard_path(id: TranslationUnitId) -> String {
    format!(
        "{AERIA_DIRECTORY}/{UNITS_DIRECTORY}/{}",
        shard_name(id.as_bytes()[0])
    )
}

/// Decodes one complete unit shard held in memory, such as a historical
/// revision read from Git.
///
/// `path` names the shard (its file name selects the expected shard) and is
/// used in errors. The full reader contract applies: the shard must be
/// non-empty, strictly ordered, and every record must be valid. Because Git
/// history may predate Workspace Format v2, each record may use either the v2
/// or the v1 record shape; a v1 record decodes as a bound unit without
/// source layout facts.
///
/// # Errors
///
/// Returns [`WorkspaceStoreError::ManagedPath`] when `path` is not a shard
/// file name, or [`WorkspaceStoreError::InvalidData`] for invalid contents.
pub fn decode_unit_shard(
    bytes: &[u8],
    path: &Path,
) -> Result<Vec<TranslationUnit>, WorkspaceStoreError> {
    let (shard, name) = shard_from_path(path)?;
    decode_shard(bytes, path, shard, &name, RecordShape::Either)
}

/// Decodes one unit record line, such as a line taken from a historical Git
/// diff of `path`.
///
/// The record is validated exactly as a shard reader would validate it,
/// including shard placement and intrinsic target validation. As with
/// [`decode_unit_shard`], either the v2 or the v1 record shape is accepted.
///
/// # Errors
///
/// Returns [`WorkspaceStoreError::ManagedPath`] when `path` is not a shard
/// file name, or [`WorkspaceStoreError::InvalidData`] for an invalid record.
pub fn decode_unit_record(line: &str, path: &Path) -> Result<TranslationUnit, WorkspaceStoreError> {
    let (shard, name) = shard_from_path(path)?;
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    parse_unit_record(line, path, None, shard, &name, RecordShape::Either)
}

/// Encodes the canonical Workspace Format v2 bytes of one unit shard, for
/// example the result of a semantic merge. Units are written in ascending ID
/// order. An empty result means the shard must be absent. A unit without
/// source layout facts cannot be encoded.
///
/// # Errors
///
/// Returns [`WorkspaceStoreError::ManagedPath`] when `path` is not a shard
/// file name, [`WorkspaceStoreError::InvalidData`] when a unit belongs to a
/// different shard or appears twice, or a serialization error.
pub fn encode_unit_shard(
    units: &[TranslationUnit],
    path: &Path,
) -> Result<Vec<u8>, WorkspaceStoreError> {
    let (shard, name) = shard_from_path(path)?;
    let mut ordered: Vec<&TranslationUnit> = units.iter().collect();
    ordered.sort_by_key(|unit| unit.id());
    for pair in ordered.windows(2) {
        if pair[0].id() == pair[1].id() {
            return Err(invalid(
                path,
                None,
                format!("duplicate TranslationUnitId {}", pair[0].id()),
            ));
        }
    }
    if let Some(unit) = ordered.iter().find(|unit| unit.id().as_bytes()[0] != shard) {
        return Err(invalid(
            path,
            None,
            format!("TranslationUnitId {} does not belong in {name}", unit.id()),
        ));
    }
    canonical_units_bytes(ordered, path)
}

fn shard_from_path(path: &Path) -> Result<(u8, String), WorkspaceStoreError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| parse_shard_name(name).map(|shard| (shard, name.to_owned())))
        .ok_or_else(|| managed_path_error(path, "expected a [0-9a-f]{2}.jsonl unit shard"))
}

fn decode_shard(
    mut reader: impl BufRead,
    path: &Path,
    shard: u8,
    shard_name: &str,
    shape: RecordShape,
) -> Result<Vec<TranslationUnit>, WorkspaceStoreError> {
    let mut records = Vec::new();
    let mut previous_id = None;
    let mut line_number = 0usize;
    loop {
        let mut line = Vec::new();
        let bytes_read = reader
            .read_until(b'\n', &mut line)
            .map_err(|source| io_error("read unit shard", path, source))?;
        if bytes_read == 0 {
            break;
        }
        line_number += 1;
        let has_lf = line.last() == Some(&b'\n');
        if has_lf {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
        } else if line.last() == Some(&b'\r') {
            return Err(invalid(
                path,
                Some(line_number),
                "bare CR is not a JSONL record terminator",
            ));
        }

        if line.iter().all(u8::is_ascii_whitespace) {
            return Err(invalid(
                path,
                Some(line_number),
                "blank JSONL records are not allowed",
            ));
        }
        let line = decode_json_text(&line, path, Some(line_number), line_number == 1)?;
        let unit = parse_unit_record(line, path, Some(line_number), shard, shard_name, shape)?;
        if let Some(previous_id) = previous_id {
            if unit.id() == previous_id {
                return Err(invalid(
                    path,
                    Some(line_number),
                    format!("duplicate TranslationUnitId {}", unit.id()),
                ));
            }
            if unit.id() < previous_id {
                return Err(invalid(
                    path,
                    Some(line_number),
                    "TranslationUnitId records must be strictly increasing",
                ));
            }
        }
        previous_id = Some(unit.id());
        records.push(unit);
    }
    if records.is_empty() {
        return Err(invalid(path, None, "empty shard files are not allowed"));
    }
    Ok(records)
}

fn parse_unit_record(
    line_text: &str,
    path: &Path,
    line: Option<usize>,
    shard: u8,
    shard_name: &str,
    shape: RecordShape,
) -> Result<TranslationUnit, WorkspaceStoreError> {
    let json_error =
        |source: serde_json::Error| invalid(path, line, format!("unit JSON is invalid: {source}"));
    match shape {
        RecordShape::Current => {
            let dto: UnitDto = serde_json::from_str(line_text).map_err(json_error)?;
            unit_from_dto(dto, path, line, shard, shard_name)
        }
        RecordShape::Legacy => {
            let dto: LegacyUnitDto = serde_json::from_str(line_text).map_err(json_error)?;
            legacy_unit_from_dto(dto, path, line, shard, shard_name)
        }
        RecordShape::Either => match serde_json::from_str::<UnitDto>(line_text) {
            Ok(dto) => unit_from_dto(dto, path, line, shard, shard_name),
            Err(current_error) => match serde_json::from_str::<LegacyUnitDto>(line_text) {
                Ok(dto) => legacy_unit_from_dto(dto, path, line, shard, shard_name),
                Err(_) => Err(json_error(current_error)),
            },
        },
    }
}

fn unit_from_dto(
    dto: UnitDto,
    path: &Path,
    line: Option<usize>,
    shard: u8,
    shard_name: &str,
) -> Result<TranslationUnit, WorkspaceStoreError> {
    let source_status = parse_source_status(&dto.source_status, path, line)?;
    let layout = match dto.source_layout {
        Value::Null if source_status.is_bound() => {
            return Err(invalid(
                path,
                line,
                "sourceLayout must not be null for a bound unit",
            ));
        }
        Value::Null => None,
        value @ Value::Object(_) => {
            let layout: SourceLayoutDto = serde_json::from_value(value).map_err(|source| {
                invalid(path, line, format!("sourceLayout is invalid: {source}"))
            })?;
            let sheet_schema_hash = parse_hash(
                &layout.sheet_schema_hash,
                "sourceLayout.sheetSchemaHash",
                path,
                line,
            )?;
            Some(SourceLayout::new(
                Sha256Hash::from_bytes(sheet_schema_hash),
                layout.column_offset,
            ))
        }
        _ => {
            return Err(invalid(
                path,
                line,
                "sourceLayout must be an object or null",
            ));
        }
    };
    let unit = legacy_unit_from_dto(
        LegacyUnitDto {
            id: dto.id,
            source_binding: dto.source_binding,
            source_fingerprint: dto.source_fingerprint,
            target_macro: dto.target_macro,
            review_state: dto.review_state,
            translator_note: dto.translator_note,
        },
        path,
        line,
        shard,
        shard_name,
    )?;
    let source_row_key = match dto.source_row_key {
        Value::Null => None,
        Value::String(value) => Some(Sha256Hash::from_bytes(parse_hash(
            &value,
            "sourceRowKey",
            path,
            line,
        )?)),
        _ => {
            return Err(invalid(path, line, "sourceRowKey must be a string or null"));
        }
    };
    let unit = unit
        .with_source_status(source_status)
        .with_source_row_key(source_row_key);
    Ok(match layout {
        Some(layout) => unit.with_source_layout(layout),
        None => unit,
    })
}

fn legacy_unit_from_dto(
    dto: LegacyUnitDto,
    path: &Path,
    line: Option<usize>,
    shard: u8,
    shard_name: &str,
) -> Result<TranslationUnit, WorkspaceStoreError> {
    let id = TranslationUnitId::from_str(&dto.id).map_err(|source| {
        invalid(
            path,
            line,
            format!("invalid canonical TranslationUnitId {:?}: {source}", dto.id),
        )
    })?;
    if id.as_bytes()[0] != shard {
        return Err(invalid(
            path,
            line,
            format!("TranslationUnitId is in the wrong shard; expected {shard_name}"),
        ));
    }

    let macro_text_hash = parse_hash(
        &dto.source_fingerprint.macro_text_hash,
        "sourceFingerprint.macroTextHash",
        path,
        line,
    )?;
    let raw_value_hash = match dto.source_fingerprint.raw_value_hash {
        Value::Null => None,
        Value::String(value) => Some(parse_hash(
            &value,
            "sourceFingerprint.rawValueHash",
            path,
            line,
        )?),
        _ => {
            return Err(invalid(
                path,
                line,
                "sourceFingerprint.rawValueHash must be a string or null",
            ));
        }
    };
    let row_technical_hash = parse_hash(
        &dto.source_fingerprint.row_technical_hash,
        "sourceFingerprint.rowTechnicalHash",
        path,
        line,
    )?;

    let review_state = match dto.review_state.as_str() {
        "draft" => ReviewState::Draft,
        "reviewed" => ReviewState::Reviewed,
        "needs-review" => ReviewState::NeedsReview,
        _ => {
            return Err(invalid(
                path,
                line,
                "reviewState must be one of draft, reviewed, needs-review",
            ));
        }
    };

    let validation = parse(&dto.target_macro).semantic_validation();
    if !matches!(
        validation.status(),
        SemanticValidity::ValidAndUnderstood | SemanticValidity::ValidWithOpaque
    ) {
        return Err(invalid(
            path,
            line,
            format!(
                "targetMacro is malformed or unsafe: {:?}",
                validation.diagnostics()
            ),
        ));
    }

    let mut unit = TranslationUnit::new(
        id,
        SourceBinding::new(
            dto.source_binding.sheet_name,
            dto.source_binding.row_id,
            dto.source_binding.subrow_id,
            dto.source_binding.column_index,
        ),
        SourceFingerprint::new(
            Sha256Hash::from_bytes(macro_text_hash),
            raw_value_hash.map(Sha256Hash::from_bytes),
            Sha256Hash::from_bytes(row_technical_hash),
        ),
        dto.target_macro,
    );
    unit.set_review_state(review_state);
    let translator_note = match dto.translator_note {
        Value::Null => None,
        Value::String(value) => Some(value),
        _ => {
            return Err(invalid(
                path,
                line,
                "translatorNote must be a string or null",
            ));
        }
    };
    unit.set_translator_note(translator_note);
    Ok(unit)
}

fn parse_source_status(
    value: &str,
    path: &Path,
    line: Option<usize>,
) -> Result<SourceStatus, WorkspaceStoreError> {
    Ok(match value {
        "bound" => SourceStatus::Bound,
        "sheet-removed" => SourceStatus::Detached(DetachReason::SheetRemoved),
        "sheet-unavailable" => SourceStatus::Detached(DetachReason::SheetUnavailable),
        "row-removed" => SourceStatus::Detached(DetachReason::RowRemoved),
        "cell-removed" => SourceStatus::Detached(DetachReason::CellRemoved),
        "column-unresolved" => SourceStatus::Detached(DetachReason::ColumnUnresolved),
        "not-translatable" => SourceStatus::Detached(DetachReason::NotTranslatable),
        "binding-conflict" => SourceStatus::Detached(DetachReason::BindingConflict),
        _ => {
            return Err(invalid(
                path,
                line,
                "sourceStatus must be bound or a defined detach reason",
            ));
        }
    })
}

const fn source_status_name(status: SourceStatus) -> &'static str {
    match status {
        SourceStatus::Bound => "bound",
        SourceStatus::Detached(DetachReason::SheetRemoved) => "sheet-removed",
        SourceStatus::Detached(DetachReason::SheetUnavailable) => "sheet-unavailable",
        SourceStatus::Detached(DetachReason::RowRemoved) => "row-removed",
        SourceStatus::Detached(DetachReason::CellRemoved) => "cell-removed",
        SourceStatus::Detached(DetachReason::ColumnUnresolved) => "column-unresolved",
        SourceStatus::Detached(DetachReason::NotTranslatable) => "not-translatable",
        SourceStatus::Detached(DetachReason::BindingConflict) => "binding-conflict",
    }
}

fn validate_workspace_metadata(
    metadata: &WorkspaceMetadata,
    path: &Path,
) -> Result<(), WorkspaceStoreError> {
    if metadata.source_language().trim().is_empty() {
        return Err(invalid(
            path,
            None,
            "sourceLanguage must not be empty or whitespace-only",
        ));
    }
    if metadata.target_language().trim().is_empty() {
        return Err(invalid(
            path,
            None,
            "targetLanguage must not be empty or whitespace-only",
        ));
    }
    validate_hxs_id(metadata.source_content_id(), "contentId", path, None)
}

fn require_metadata_match(
    actual: &WorkspaceMetadata,
    persisted: &WorkspaceMetadata,
    path: &Path,
) -> Result<(), WorkspaceStoreError> {
    if actual != persisted {
        return Err(invalid(
            path,
            None,
            "in-memory workspace metadata does not match manifest.json",
        ));
    }
    Ok(())
}

fn validate_unique_shard_bindings(
    units: &BTreeMap<TranslationUnitId, TranslationUnit>,
    path: &Path,
) -> Result<(), WorkspaceStoreError> {
    let mut bindings = BTreeSet::new();
    for unit in units.values().filter(|unit| unit.is_bound()) {
        if !bindings.insert(unit.source_binding().clone()) {
            return Err(invalid(
                path,
                None,
                format!(
                    "duplicate current SourceBinding {:?}",
                    unit.source_binding()
                ),
            ));
        }
    }
    Ok(())
}

fn require_persisted_identity(
    persisted: &TranslationUnit,
    replacement: &TranslationUnit,
    path: &Path,
) -> Result<(), WorkspaceStoreError> {
    if persisted.source_binding() != replacement.source_binding() {
        return Err(invalid(
            path,
            None,
            format!(
                "persist_unit cannot change SourceBinding for existing TranslationUnitId {}",
                replacement.id()
            ),
        ));
    }
    if persisted.source_fingerprint() != replacement.source_fingerprint()
        || persisted.source_layout() != replacement.source_layout()
        || persisted.source_row_key() != replacement.source_row_key()
        || persisted.source_status() != replacement.source_status()
    {
        return Err(invalid(
            path,
            None,
            format!(
                "persist_unit cannot change source facts for existing TranslationUnitId {}",
                replacement.id()
            ),
        ));
    }
    Ok(())
}

fn require_new_binding_is_unowned(
    layout: &ExistingLayout,
    target_shard: u8,
    target_units: &BTreeMap<TranslationUnitId, TranslationUnit>,
    replacement: &TranslationUnit,
    path: &Path,
) -> Result<(), WorkspaceStoreError> {
    if !replacement.is_bound() {
        return Err(invalid(path, None, "a new unit must be bound"));
    }
    let mut bindings = BTreeSet::new();
    for unit in target_units.values().filter(|unit| unit.is_bound()) {
        bindings.insert(unit.source_binding().clone());
    }

    for shard in &layout.shards {
        if shard.shard == target_shard {
            continue;
        }
        for unit in read_shard(&shard.path, shard.shard, &shard.name, RecordShape::Current)? {
            if !unit.is_bound() {
                continue;
            }
            if !bindings.insert(unit.source_binding().clone()) {
                return Err(invalid(
                    &shard.path,
                    None,
                    format!(
                        "duplicate current SourceBinding {:?}",
                        unit.source_binding()
                    ),
                ));
            }
        }
    }

    if !bindings.insert(replacement.source_binding().clone()) {
        return Err(invalid(
            path,
            None,
            format!(
                "SourceBinding {:?} is already owned by a persisted unit",
                replacement.source_binding()
            ),
        ));
    }
    Ok(())
}

fn require_new_binding_is_unowned_cached(
    persisted_units: &BTreeMap<TranslationUnitId, TranslationUnit>,
    replacement: &TranslationUnit,
    path: &Path,
) -> Result<(), WorkspaceStoreError> {
    if !replacement.is_bound() {
        return Err(invalid(path, None, "a new unit must be bound"));
    }
    if persisted_units
        .values()
        .any(|unit| unit.is_bound() && unit.source_binding() == replacement.source_binding())
    {
        return Err(invalid(
            path,
            None,
            format!(
                "SourceBinding {:?} is already owned by a persisted unit",
                replacement.source_binding()
            ),
        ));
    }
    Ok(())
}

fn canonical_manifest_bytes(
    workspace: &Workspace,
    path: &Path,
) -> Result<Vec<u8>, WorkspaceStoreError> {
    validate_workspace_metadata(workspace.metadata(), path)?;
    let dto = CanonicalManifestDto {
        format_version: FORMAT_VERSION,
        source_language: workspace.metadata().source_language(),
        target_language: workspace.metadata().target_language(),
        content_id: workspace.metadata().source_content_id(),
    };
    let mut bytes =
        serde_json::to_vec_pretty(&dto).map_err(|source| WorkspaceStoreError::Serialization {
            path: path.to_owned(),
            source,
        })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonical_shard_bytes(
    workspace: &Workspace,
    shard: u8,
    path: &Path,
) -> Result<Vec<u8>, WorkspaceStoreError> {
    canonical_units_bytes(workspace.units_in_shard(shard), path)
}

fn canonical_units_bytes<'a, I>(units: I, path: &Path) -> Result<Vec<u8>, WorkspaceStoreError>
where
    I: IntoIterator<Item = &'a TranslationUnit>,
{
    let mut bytes = Vec::new();
    for unit in units {
        let dto = canonical_unit_dto(unit, path)?;
        serde_json::to_writer(&mut bytes, &dto).map_err(|source| {
            WorkspaceStoreError::Serialization {
                path: path.to_owned(),
                source,
            }
        })?;
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn canonical_unit_dto(
    unit: &TranslationUnit,
    path: &Path,
) -> Result<CanonicalUnitDto, WorkspaceStoreError> {
    let validation = parse(unit.target_macro()).semantic_validation();
    if !matches!(
        validation.status(),
        SemanticValidity::ValidAndUnderstood | SemanticValidity::ValidWithOpaque
    ) {
        return Err(invalid(
            path,
            None,
            format!(
                "targetMacro is malformed or unsafe: {:?}",
                validation.diagnostics()
            ),
        ));
    }
    if unit.is_bound() && unit.source_layout().is_none() {
        return Err(invalid(
            path,
            None,
            format!(
                "bound TranslationUnitId {} has no source layout and cannot be written",
                unit.id()
            ),
        ));
    }
    Ok(CanonicalUnitDto {
        id: unit.id().to_string(),
        source_status: source_status_name(unit.source_status()),
        source_binding: CanonicalSourceBindingDto {
            sheet_name: unit.source_binding().sheet_name().to_owned(),
            row_id: unit.source_binding().row_id(),
            subrow_id: unit.source_binding().subrow_id(),
            column_index: unit.source_binding().column_index(),
        },
        source_fingerprint: CanonicalSourceFingerprintDto {
            macro_text_hash: unit.source_fingerprint().macro_text_hash().to_hex(),
            raw_value_hash: unit
                .source_fingerprint()
                .raw_value_hash()
                .map(Sha256Hash::to_hex),
            row_technical_hash: unit.source_fingerprint().row_technical_hash().to_hex(),
        },
        source_layout: unit.source_layout().map(|layout| CanonicalSourceLayoutDto {
            sheet_schema_hash: layout.sheet_schema_hash().to_hex(),
            column_offset: layout.column_offset(),
        }),
        source_row_key: unit.source_row_key().map(Sha256Hash::to_hex),
        target_macro: unit.target_macro().to_owned(),
        review_state: review_state_name(unit.review_state()),
        translator_note: unit.translator_note().map(str::to_owned),
    })
}

fn review_state_name(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Draft => "draft",
        ReviewState::Reviewed => "reviewed",
        ReviewState::NeedsReview => "needs-review",
    }
}

fn atomic_publish(
    repository_root: &Path,
    target_path: &Path,
    bytes: &[u8],
) -> Result<(), WorkspaceStoreError> {
    let mut temporary = NamedTempFile::new_in(repository_root).map_err(|source| {
        io_error(
            "create canonical workspace staging file",
            repository_root,
            source,
        )
    })?;
    temporary.write_all(bytes).map_err(|source| {
        io_error(
            "write canonical workspace staging file",
            temporary.path(),
            source,
        )
    })?;
    temporary.flush().map_err(|source| {
        io_error(
            "flush canonical workspace staging file",
            temporary.path(),
            source,
        )
    })?;
    temporary.as_file().sync_all().map_err(|source| {
        io_error(
            "sync canonical workspace staging file",
            temporary.path(),
            source,
        )
    })?;
    #[cfg(test)]
    if FAIL_BEFORE_PUBLICATION.swap(false, Ordering::SeqCst) {
        return Err(WorkspaceStoreError::AtomicPublication {
            path: target_path.to_owned(),
            source: io::Error::other("test failpoint before atomic publication"),
        });
    }
    temporary
        .persist(target_path)
        .map(|_| ())
        .map_err(|source| WorkspaceStoreError::AtomicPublication {
            path: target_path.to_owned(),
            source: source.error,
        })
}

fn write_staging_file(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceStoreError> {
    let mut file = File::create(path)
        .map_err(|source| io_error("create initialization staging file", path, source))?;
    file.write_all(bytes)
        .map_err(|source| io_error("write initialization staging file", path, source))?;
    file.sync_all()
        .map_err(|source| io_error("sync initialization staging file", path, source))?;
    Ok(())
}

fn remove_directory_if_empty(path: &Path) {
    let Ok(mut entries) = fs::read_dir(path) else {
        return;
    };
    let Ok(None) = entries.next().transpose() else {
        return;
    };
    let _ = fs::remove_dir(path);
}

fn read_file(path: &Path, operation: &'static str) -> Result<Vec<u8>, WorkspaceStoreError> {
    let mut file = File::open(path).map_err(|source| io_error(operation, path, source))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|source| io_error(operation, path, source))?;
    Ok(bytes)
}

fn decode_json_text<'a>(
    bytes: &'a [u8],
    path: &Path,
    line: Option<usize>,
    allow_leading_bom: bool,
) -> Result<&'a str, WorkspaceStoreError> {
    let bytes = if allow_leading_bom && bytes.starts_with(BOM) {
        &bytes[BOM.len()..]
    } else {
        bytes
    };
    if bytes.windows(BOM.len()).any(|window| window == BOM) {
        return Err(invalid(
            path,
            line,
            "UTF-8 BOM is only allowed once at the start",
        ));
    }
    std::str::from_utf8(bytes)
        .map_err(|source| invalid(path, line, format!("file is not valid UTF-8: {source}")))
}

fn parse_hash(
    value: &str,
    field: &str,
    path: &Path,
    line: Option<usize>,
) -> Result<[u8; 32], WorkspaceStoreError> {
    if value.len() != 64 {
        return Err(invalid(
            path,
            line,
            format!("{field} must contain exactly 64 lowercase hexadecimal characters"),
        ));
    }
    let mut bytes = [0_u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let offset = index * 2;
        let high = lower_hex_nibble(value.as_bytes()[offset]);
        let low = lower_hex_nibble(value.as_bytes()[offset + 1]);
        let (Some(high), Some(low)) = (high, low) else {
            return Err(invalid(
                path,
                line,
                format!("{field} must contain only lowercase hexadecimal characters"),
            ));
        };
        *byte = (high << 4) | low;
    }
    Ok(bytes)
}

fn validate_hxs_id(
    value: &str,
    field: &str,
    path: &Path,
    line: Option<usize>,
) -> Result<(), WorkspaceStoreError> {
    let bytes = value.as_bytes();
    if bytes.len() != 71
        || &bytes[..bytes.len().min(7)] != b"sha256:"
        || !bytes[7..]
            .iter()
            .all(|byte| lower_hex_nibble(*byte).is_some())
    {
        return Err(invalid(
            path,
            line,
            format!("{field} must be a canonical sha256:<64 lowercase hexadecimal characters> ID"),
        ));
    }
    Ok(())
}

fn lower_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn parse_shard_name(name: &str) -> Option<u8> {
    let bytes = name.as_bytes();
    if bytes.len() != 8 || &bytes[2..] != b".jsonl" {
        return None;
    }
    Some((lower_hex_nibble(bytes[0])? << 4) | lower_hex_nibble(bytes[1])?)
}

fn shard_name(shard: u8) -> String {
    format!("{shard:02x}.jsonl")
}

fn path_exists(path: &Path) -> Result<bool, WorkspaceStoreError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error("inspect workspace path", path, source)),
    }
}

fn symlink_metadata(path: &Path) -> Result<Option<Metadata>, WorkspaceStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error("inspect workspace path", path, source)),
    }
}

fn ensure_regular_file(path: &Path) -> Result<(), WorkspaceStoreError> {
    let metadata = symlink_metadata(path)?.ok_or_else(|| WorkspaceStoreError::MissingPath {
        path: path.to_owned(),
    })?;
    if metadata.file_type().is_symlink() {
        return Err(managed_path_error(path, "symlinks are not allowed"));
    }
    if !metadata.is_file() {
        return Err(managed_path_error(path, "expected a regular file"));
    }
    Ok(())
}

fn ensure_optional_regular_file(path: &Path) -> Result<(), WorkspaceStoreError> {
    if let Some(metadata) = symlink_metadata(path)? {
        if metadata.file_type().is_symlink() {
            return Err(managed_path_error(path, "symlinks are not allowed"));
        }
        if !metadata.is_file() {
            return Err(managed_path_error(path, "expected a regular file"));
        }
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> Result<(), WorkspaceStoreError> {
    let metadata = symlink_metadata(path)?.ok_or_else(|| WorkspaceStoreError::MissingPath {
        path: path.to_owned(),
    })?;
    if metadata.file_type().is_symlink() {
        return Err(managed_path_error(path, "symlinks are not allowed"));
    }
    if !metadata.is_dir() {
        return Err(managed_path_error(path, "expected a directory"));
    }
    Ok(())
}

fn managed_path_error(path: &Path, reason: impl Into<String>) -> WorkspaceStoreError {
    WorkspaceStoreError::ManagedPath {
        path: path.to_owned(),
        reason: reason.into(),
    }
}

fn invalid(path: &Path, line: Option<usize>, message: impl Into<String>) -> WorkspaceStoreError {
    WorkspaceStoreError::InvalidData {
        path: path.to_owned(),
        line: line.map_or_else(String::new, |line| format!(":{line}")),
        message: message.into(),
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> WorkspaceStoreError {
    WorkspaceStoreError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeria_core::ReviewState;
    use std::fs;
    use std::str::FromStr;

    #[test]
    fn failed_publication_leaves_the_previous_shard_and_managed_namespace_unchanged() {
        let _test_lock = PERSISTENCE_TEST_LOCK.lock().expect("test lock");
        let repository = tempfile::tempdir().expect("temporary repository");
        let aeria_path = repository.path().join(AERIA_DIRECTORY);
        let units_path = aeria_path.join(UNITS_DIRECTORY);
        fs::create_dir_all(&units_path).expect("workspace directories");
        fs::write(
            aeria_path.join(MANIFEST_FILE),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/workspace-v2/manifest.json"
            )),
        )
        .expect("manifest");
        fs::write(
            units_path.join("00.jsonl"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/workspace-v2/units/00.jsonl"
            )),
        )
        .expect("shard");

        let store = WorkspaceStore::new(repository.path());
        let mut workspace = store.load().expect("fixture loads");
        let id = TranslationUnitId::from_str(
            "tu1:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("ID");
        workspace
            .update_target(id, "staged but unpublished")
            .expect("target is valid");
        let shard_path = units_path.join("00.jsonl");
        let before = fs::read(&shard_path).expect("previous shard");

        FAIL_BEFORE_PUBLICATION.store(true, Ordering::SeqCst);
        let error = store
            .persist_unit(&workspace, id)
            .expect_err("publication failpoint must fail");
        assert!(matches!(
            error,
            WorkspaceStoreError::AtomicPublication { .. }
        ));

        assert_eq!(fs::read(&shard_path).expect("previous shard"), before);
        let unit_entries: Vec<_> = fs::read_dir(&units_path)
            .expect("units directory")
            .map(|entry| entry.expect("unit entry").file_name())
            .collect();
        assert_eq!(unit_entries, vec![std::ffi::OsString::from("00.jsonl")]);
        let mut managed_entries: Vec<_> = fs::read_dir(&aeria_path)
            .expect("managed directory")
            .map(|entry| entry.expect("managed entry").file_name())
            .collect();
        managed_entries.sort();
        assert_eq!(
            managed_entries,
            vec![
                std::ffi::OsString::from(MANIFEST_FILE),
                std::ffi::OsString::from(UNITS_DIRECTORY),
            ]
        );
    }

    #[test]
    fn ordinary_persistence_uses_the_validated_session_cache() {
        let _test_lock = PERSISTENCE_TEST_LOCK.lock().expect("test lock");
        let repository = tempfile::tempdir().expect("temporary repository");
        let aeria_path = repository.path().join(AERIA_DIRECTORY);
        let units_path = aeria_path.join(UNITS_DIRECTORY);
        fs::create_dir_all(&units_path).expect("workspace directories");
        fs::write(
            aeria_path.join(MANIFEST_FILE),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/workspace-v2/manifest.json"
            )),
        )
        .expect("manifest");
        fs::write(
            units_path.join("00.jsonl"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/workspace-v2/units/00.jsonl"
            )),
        )
        .expect("00 shard");
        fs::write(
            units_path.join("ff.jsonl"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/workspace-v2/units/ff.jsonl"
            )),
        )
        .expect("ff shard");

        let store = WorkspaceStore::new(repository.path());
        let mut workspace = store.load().expect("fixture loads");
        let id = TranslationUnitId::from_str(
            "tu1:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("ID");
        workspace
            .update_target(id, "ordinary update")
            .expect("target is valid");
        READ_SHARD_COUNT.store(0, Ordering::SeqCst);

        store
            .persist_unit(&workspace, id)
            .expect("ordinary update persists");

        assert_eq!(READ_SHARD_COUNT.load(Ordering::SeqCst), 0);

        workspace
            .update_note(id, Some("ordinary note".to_owned()))
            .expect("note is valid");
        READ_SHARD_COUNT.store(0, Ordering::SeqCst);
        store
            .persist_unit(&workspace, id)
            .expect("ordinary note persists");
        assert_eq!(READ_SHARD_COUNT.load(Ordering::SeqCst), 0);

        workspace
            .update_review_state(id, ReviewState::Reviewed)
            .expect("review state is valid");
        READ_SHARD_COUNT.store(0, Ordering::SeqCst);
        store
            .persist_unit(&workspace, id)
            .expect("ordinary review state persists");
        assert_eq!(READ_SHARD_COUNT.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn external_change_in_cached_shard_is_rejected_without_losing_external_unit() {
        let _test_lock = PERSISTENCE_TEST_LOCK.lock().expect("test lock");
        let repository = tempfile::tempdir().expect("temporary repository");
        let aeria_path = repository.path().join(AERIA_DIRECTORY);
        let units_path = aeria_path.join(UNITS_DIRECTORY);
        fs::create_dir_all(&units_path).expect("workspace directories");
        fs::write(
            aeria_path.join(MANIFEST_FILE),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/workspace-v2/manifest.json"
            )),
        )
        .expect("manifest");
        let shard_path = units_path.join("00.jsonl");
        fs::write(
            &shard_path,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/workspace-v2/units/00.jsonl"
            )),
        )
        .expect("00 shard");

        let store = WorkspaceStore::new(repository.path());
        let mut workspace = store.load().expect("fixture loads");
        let original_id = TranslationUnitId::from_str(
            "tu1:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("original ID");
        let external_id = TranslationUnitId::from_str(
            "tu1:0000000000000000000000000000000000000000000000000000000000000002",
        )
        .expect("external ID");

        let mut externally_changed = fs::read(&shard_path).expect("cached shard");
        externally_changed.extend_from_slice(
            br#"{"id":"tu1:0000000000000000000000000000000000000000000000000000000000000002","sourceStatus":"bound","sourceBinding":{"sheetName":"External","rowId":7,"subrowId":0,"columnIndex":1},"sourceFingerprint":{"macroTextHash":"1212121212121212121212121212121212121212121212121212121212121212","rawValueHash":null,"rowTechnicalHash":"3434343434343434343434343434343434343434343434343434343434343434"},"sourceLayout":{"sheetSchemaHash":"5656565656565656565656565656565656565656565656565656565656565656","columnOffset":4},"sourceRowKey":null,"targetMacro":"external","reviewState":"draft","translatorNote":null}"#,
        );
        externally_changed.push(b'\n');
        fs::write(&shard_path, externally_changed).expect("external unit update");

        workspace
            .update_target(original_id, "local update")
            .expect("target is valid");
        let error = store
            .persist_unit(&workspace, original_id)
            .expect_err("stale session cache must not publish");
        assert!(matches!(error, WorkspaceStoreError::ExternalChange { .. }));

        let reloaded = store.load().expect("external shard remains loadable");
        assert_eq!(
            reloaded
                .unit(external_id)
                .expect("external unit")
                .target_macro(),
            "external"
        );
        assert_eq!(
            reloaded
                .unit(original_id)
                .expect("original unit")
                .target_macro(),
            "Quote \" slash \\ line\n tab\t control\u{0}"
        );
    }
}
