//! Production Workspace Format v1 filesystem persistence.
//!
//! The DTOs in this module are deliberately private. They describe the
//! frozen file contract without making the domain types in `aeria-core`
//! serialization types.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
#[cfg(test)]
use std::sync::Mutex;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use aeria_core::{
    ReviewState, Sha256Hash, SourceBinding, SourceFingerprint, TranslationUnit, TranslationUnitId,
    WorkspaceMetadata,
};
use aeria_se::{SemanticValidity, parse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::{NamedTempFile, TempDir};
use thiserror::Error;

use super::{Workspace, WorkspaceError};

const FORMAT_VERSION: u8 = 1;
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

/// Errors raised while opening, validating, or persisting a Workspace Format
/// v1 repository.
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
    #[error("Workspace Format v1 project already exists at {path}")]
    AlreadyInitialized { path: PathBuf },

    /// A JSON or semantic value violates the frozen reader contract.
    #[error("invalid Workspace Format v1 data at {path}{line}: {message}")]
    InvalidData {
        path: PathBuf,
        line: String,
        message: String,
    },

    /// A format version newer than the implementation is not silently opened.
    #[error("unsupported Workspace Format version {version} in {path}")]
    UnsupportedFormatVersion { path: PathBuf, version: u64 },

    /// JSON serialization failed before publication.
    #[error("failed to serialize canonical workspace data for {path}: {source}")]
    Serialization {
        path: PathBuf,
        source: serde_json::Error,
    },

    /// The canonical replacement could not be published.
    #[error("failed to atomically publish canonical workspace file {path}: {source}")]
    AtomicPublication { path: PathBuf, source: io::Error },

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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceStore {
    repository_root: PathBuf,
}

impl WorkspaceStore {
    /// Binds a persistence adapter to a repository root.
    #[must_use]
    pub fn new(repository_root: impl Into<PathBuf>) -> Self {
        Self {
            repository_root: repository_root.into(),
        }
    }

    /// Returns the repository root used by this adapter.
    #[must_use]
    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    /// Loads and validates the complete canonical workspace state.
    ///
    /// The repository root is only a project container. Only `.aeria/` and
    /// its defined managed entries are inspected.
    ///
    /// # Errors
    ///
    /// Returns an error when the managed namespace is missing, unsafe,
    /// malformed, or contains invalid unit data.
    pub fn load(&self) -> Result<Workspace, WorkspaceStoreError> {
        let layout = self.inspect_existing_layout()?;
        let metadata = read_manifest(&layout.manifest_path)?;

        let mut units = BTreeMap::new();
        let mut bindings = BTreeSet::new();
        for shard in &layout.shards {
            for unit in read_shard(&shard.path, shard.shard, &shard.name)? {
                let id = unit.id();
                if units.insert(id, unit.clone()).is_some() {
                    return Err(invalid(
                        &shard.path,
                        None,
                        format!("duplicate TranslationUnitId {id}"),
                    ));
                }
                let binding = unit.source_binding().clone();
                if !bindings.insert(binding.clone()) {
                    return Err(WorkspaceStoreError::InvalidData {
                        path: shard.path.clone(),
                        line: String::new(),
                        message: format!("duplicate current SourceBinding {binding:?}"),
                    });
                }
            }
        }

        Workspace::from_loaded(metadata, units).map_err(WorkspaceStoreError::from)
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
            for shard in shard_numbers {
                let shard_path = units_path.join(shard_name(shard));
                let bytes = canonical_shard_bytes(workspace, shard, &shard_path)?;
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
        Ok(())
    }

    /// Replaces exactly the selected unit in its complete canonical shard.
    ///
    /// The selected persisted shard is fully validated and all of its other
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
    pub fn persist_unit(
        &self,
        workspace: &Workspace,
        id: TranslationUnitId,
    ) -> Result<(), WorkspaceStoreError> {
        let replacement = workspace
            .unit(id)
            .cloned()
            .ok_or(WorkspaceStoreError::Domain(WorkspaceError::UnitNotFound {
                id,
            }))?;
        let layout = self.inspect_existing_layout()?;
        let persisted_metadata = read_manifest(&layout.manifest_path)?;
        require_metadata_match(
            workspace.metadata(),
            &persisted_metadata,
            &layout.manifest_path,
        )?;

        let shard = id.as_bytes()[0];
        let target_path = self
            .repository_root
            .join(AERIA_DIRECTORY)
            .join(UNITS_DIRECTORY)
            .join(shard_name(shard));
        let mut persisted_units = BTreeMap::new();
        if let Some(shard_file) = layout.shards.iter().find(|file| file.shard == shard) {
            for unit in read_shard(&shard_file.path, shard, &shard_file.name)? {
                persisted_units.insert(unit.id(), unit);
            }
        }
        validate_unique_shard_bindings(&persisted_units, &target_path)?;
        if let Some(persisted) = persisted_units.get(&id) {
            require_persisted_identity(persisted, &replacement, &target_path)?;
        } else {
            require_new_binding_is_unowned(
                &layout,
                shard,
                &persisted_units,
                &replacement,
                &target_path,
            )?;
        }
        persisted_units.insert(id, replacement);
        let bytes = canonical_units_bytes(persisted_units.values(), &target_path)?;

        let created_units_path = layout.units_path.is_none();
        let units_path = if let Some(path) = layout.units_path {
            path
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
        let target_path = units_path.join(shard_name(shard));
        let result = ensure_optional_regular_file(&target_path)
            .and_then(|()| atomic_publish(&self.repository_root, &target_path, &bytes));
        if result.is_err() && created_units_path {
            remove_directory_if_empty(&units_path);
        }
        result
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

struct ExistingLayout {
    manifest_path: PathBuf,
    units_path: Option<PathBuf>,
    shards: Vec<ShardFile>,
}

struct ShardFile {
    shard: u8,
    name: String,
    path: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDto {
    #[serde(rename = "formatVersion")]
    format_version: u64,
    #[serde(rename = "sourceLanguage")]
    source_language: String,
    #[serde(rename = "targetLanguage")]
    target_language: String,
    #[serde(rename = "contentId")]
    content_id: String,
    #[serde(rename = "snapshotId")]
    snapshot_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitDto {
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
    #[serde(rename = "snapshotId")]
    snapshot_id: &'a str,
}

#[derive(Serialize)]
struct CanonicalUnitDto {
    id: String,
    #[serde(rename = "sourceBinding")]
    source_binding: CanonicalSourceBindingDto,
    #[serde(rename = "sourceFingerprint")]
    source_fingerprint: CanonicalSourceFingerprintDto,
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

fn read_manifest(path: &Path) -> Result<WorkspaceMetadata, WorkspaceStoreError> {
    let bytes = read_file(path, "read manifest")?;
    let text = decode_json_text(&bytes, path, None, true)?;
    let manifest: ManifestDto = serde_json::from_str(text).map_err(|source| {
        invalid(
            path,
            Some(source.line()),
            format!("manifest JSON is invalid: {source}"),
        )
    })?;
    if manifest.format_version != u64::from(FORMAT_VERSION) {
        return Err(WorkspaceStoreError::UnsupportedFormatVersion {
            path: path.to_owned(),
            version: manifest.format_version,
        });
    }
    validate_hxs_id(&manifest.content_id, "contentId", path, None)?;
    validate_hxs_id(&manifest.snapshot_id, "snapshotId", path, None)?;
    WorkspaceMetadata::new(
        manifest.source_language,
        manifest.target_language,
        manifest.content_id,
        manifest.snapshot_id,
    )
    .map_err(|source| invalid(path, None, format!("invalid manifest metadata: {source}")))
}

fn read_shard(
    path: &Path,
    shard: u8,
    shard_name: &str,
) -> Result<Vec<TranslationUnit>, WorkspaceStoreError> {
    #[cfg(test)]
    READ_SHARD_COUNT.fetch_add(1, Ordering::SeqCst);
    let file = File::open(path).map_err(|source| io_error("open unit shard", path, source))?;
    let mut reader = BufReader::new(file);
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
        let dto: UnitDto = serde_json::from_str(line).map_err(|source| {
            invalid(
                path,
                Some(line_number),
                format!("unit JSON is invalid: {source}"),
            )
        })?;
        let unit = unit_from_dto(dto, path, line_number, shard, shard_name)?;
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

fn unit_from_dto(
    dto: UnitDto,
    path: &Path,
    line: usize,
    shard: u8,
    shard_name: &str,
) -> Result<TranslationUnit, WorkspaceStoreError> {
    let id = TranslationUnitId::from_str(&dto.id).map_err(|source| {
        invalid(
            path,
            Some(line),
            format!("invalid canonical TranslationUnitId {:?}: {source}", dto.id),
        )
    })?;
    if id.as_bytes()[0] != shard {
        return Err(invalid(
            path,
            Some(line),
            format!("TranslationUnitId is in the wrong shard; expected {shard_name}"),
        ));
    }

    let macro_text_hash = parse_hash(
        &dto.source_fingerprint.macro_text_hash,
        "sourceFingerprint.macroTextHash",
        path,
        Some(line),
    )?;
    let raw_value_hash = match dto.source_fingerprint.raw_value_hash {
        Value::Null => None,
        Value::String(value) => Some(parse_hash(
            &value,
            "sourceFingerprint.rawValueHash",
            path,
            Some(line),
        )?),
        _ => {
            return Err(invalid(
                path,
                Some(line),
                "sourceFingerprint.rawValueHash must be a string or null",
            ));
        }
    };
    let row_technical_hash = parse_hash(
        &dto.source_fingerprint.row_technical_hash,
        "sourceFingerprint.rowTechnicalHash",
        path,
        Some(line),
    )?;

    let review_state = match dto.review_state.as_str() {
        "draft" => ReviewState::Draft,
        "reviewed" => ReviewState::Reviewed,
        "needs-review" => ReviewState::NeedsReview,
        _ => {
            return Err(invalid(
                path,
                Some(line),
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
            Some(line),
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
                Some(line),
                "translatorNote must be a string or null",
            ));
        }
    };
    unit.set_translator_note(translator_note);
    Ok(unit)
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
    validate_hxs_id(metadata.source_content_id(), "contentId", path, None)?;
    validate_hxs_id(metadata.source_snapshot_id(), "snapshotId", path, None)
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
    for unit in units.values() {
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
    if persisted.source_fingerprint() != replacement.source_fingerprint() {
        return Err(invalid(
            path,
            None,
            format!(
                "persist_unit cannot change SourceFingerprint for existing TranslationUnitId {}",
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
    let mut bindings = BTreeSet::new();
    for unit in target_units.values() {
        bindings.insert(unit.source_binding().clone());
    }

    for shard in &layout.shards {
        if shard.shard == target_shard {
            continue;
        }
        for unit in read_shard(&shard.path, shard.shard, &shard.name)? {
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
        snapshot_id: workspace.metadata().source_snapshot_id(),
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
    Ok(CanonicalUnitDto {
        id: unit.id().to_string(),
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
                "/tests/fixtures/workspace-v1/manifest.json"
            )),
        )
        .expect("manifest");
        fs::write(
            units_path.join("00.jsonl"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/workspace-v1/units/00.jsonl"
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
    fn ordinary_persistence_reads_only_the_selected_shard() {
        let _test_lock = PERSISTENCE_TEST_LOCK.lock().expect("test lock");
        let repository = tempfile::tempdir().expect("temporary repository");
        let aeria_path = repository.path().join(AERIA_DIRECTORY);
        let units_path = aeria_path.join(UNITS_DIRECTORY);
        fs::create_dir_all(&units_path).expect("workspace directories");
        fs::write(
            aeria_path.join(MANIFEST_FILE),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/workspace-v1/manifest.json"
            )),
        )
        .expect("manifest");
        fs::write(
            units_path.join("00.jsonl"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/workspace-v1/units/00.jsonl"
            )),
        )
        .expect("00 shard");
        fs::write(
            units_path.join("ff.jsonl"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/workspace-v1/units/ff.jsonl"
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

        assert_eq!(READ_SHARD_COUNT.load(Ordering::SeqCst), 1);

        workspace
            .update_note(id, Some("ordinary note".to_owned()))
            .expect("note is valid");
        READ_SHARD_COUNT.store(0, Ordering::SeqCst);
        store
            .persist_unit(&workspace, id)
            .expect("ordinary note persists");
        assert_eq!(READ_SHARD_COUNT.load(Ordering::SeqCst), 1);

        workspace
            .update_review_state(id, ReviewState::Reviewed)
            .expect("review state is valid");
        READ_SHARD_COUNT.store(0, Ordering::SeqCst);
        store
            .persist_unit(&workspace, id)
            .expect("ordinary review state persists");
        assert_eq!(READ_SHARD_COUNT.load(Ordering::SeqCst), 1);
    }
}
