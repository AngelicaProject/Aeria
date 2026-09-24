//! Owned application-layer state for one opened Aeria project.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use aeria_core::TranslationUnit;
use aeria_hsp::{HspError, SourcePackage};
use aeria_hxs::{ColumnType, HxsError, HxsSnapshot, MAX_STRING_OCCURRENCE_PAGE_SIZE};
use aeria_rebase::{RowKeys, SourceUpdateError, plan_source_update};
use thiserror::Error;

use crate::persistence::{FORMAT_VERSION, StoredWorkspace};
use crate::update::{SourceUpdateReport, apply_plan};
use crate::{
    Workspace, WorkspaceError, WorkspaceStore, WorkspaceStoreError, source_fingerprint_from_hashes,
    source_layout_from_hashes,
};

/// The owned runtime representation of one opened Aeria project.
///
/// A session keeps the local source-package path alongside the validated
/// package, verified immutable HXS handle, loaded sparse workspace,
/// persistence adapter, and repository root. Package and cache paths are
/// runtime configuration and are not part of Workspace Format state.
pub struct ProjectSession {
    repository_root: PathBuf,
    source_package_path: PathBuf,
    pub(crate) store: WorkspaceStore,
    pub(crate) workspace: Workspace,
    pub(crate) source_package: SourcePackage,
    /// Row key columns detected per sheet of the immutable source, filled
    /// on first use when a unit is created in that sheet.
    pub(crate) row_keys: BTreeMap<String, Option<RowKeys>>,
}

/// Errors raised while opening or initializing a project session.
#[derive(Debug, Error)]
pub enum ProjectSessionError {
    /// The local HSP file could not be opened or fully verified.
    #[error("failed to open and verify HSP source package {path}: {source}")]
    Source { path: PathBuf, source: HspError },

    /// The workspace could not be loaded, initialized, or published.
    #[error("failed to load or initialize workspace at {repository_root}: {source}")]
    Store {
        repository_root: PathBuf,
        #[source]
        source: WorkspaceStoreError,
    },

    /// The workspace and source package use different source languages. A
    /// source update cannot change the source language.
    #[error(
        "workspace at {repository_root} is incompatible with HSP source package {source_package_path}: {source}"
    )]
    Compatibility {
        repository_root: PathBuf,
        source_package_path: PathBuf,
        #[source]
        source: WorkspaceError,
    },

    /// The requested target language could not be used to construct workspace metadata.
    #[error(
        "could not create workspace metadata for {repository_root} from HSP source package {source_package_path}: {source}"
    )]
    Workspace {
        repository_root: PathBuf,
        source_package_path: PathBuf,
        #[source]
        source: WorkspaceError,
    },

    /// The workspace must be moved onto the package source by a source
    /// update before it can be edited.
    #[error(
        "workspace at {repository_root} requires a source update for HSP source package {source_package_path}: {requirement}"
    )]
    SourceUpdateRequired {
        repository_root: PathBuf,
        source_package_path: PathBuf,
        requirement: SourceUpdateRequirement,
    },

    /// A source update could not be planned.
    #[error("could not plan the source update for workspace at {repository_root}: {source}")]
    SourceUpdate {
        repository_root: PathBuf,
        #[source]
        source: SourceUpdateError,
    },
}

/// Why a workspace cannot be edited against a source package as stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceUpdateRequirement {
    /// The workspace uses an older Workspace Format version.
    FormatMigration { version: u8 },
    /// The workspace is bound to different verified source content.
    ContentChanged {
        workspace_content_id: String,
        source_content_id: String,
    },
    /// The workspace records the package's source content, but this many
    /// bound units do not describe it: their occurrence, fingerprint, or
    /// layout differs, or another bound unit claims the same occurrence.
    /// Ordinary editing never produces this state; a Git merge of work done
    /// against different game versions does.
    SourceFactsMismatch { units: usize },
    /// The source content is unchanged, but the package's guidance no longer
    /// permits this many bound units.
    PermissionChanged { blocked_units: usize },
}

impl std::fmt::Display for SourceUpdateRequirement {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FormatMigration { version } => {
                write!(
                    formatter,
                    "Workspace Format version {version} must be migrated"
                )
            }
            Self::ContentChanged {
                workspace_content_id,
                source_content_id,
            } => write!(
                formatter,
                "workspace content {workspace_content_id} differs from source content {source_content_id}"
            ),
            Self::SourceFactsMismatch { units } => write!(
                formatter,
                "{units} bound translation units do not match the current source"
            ),
            Self::PermissionChanged { blocked_units } => write!(
                formatter,
                "{blocked_units} bound translation units are no longer permitted by source guidance"
            ),
        }
    }
}

impl ProjectSession {
    /// Opens an existing project against its currently bound, verified HXS source.
    ///
    /// Opening verifies the source, loads the complete sparse workspace state,
    /// checks that it is current for the source, and only then constructs the
    /// session. It never updates either input; a workspace that needs a source
    /// update is reported as [`ProjectSessionError::SourceUpdateRequired`].
    ///
    /// # Errors
    ///
    /// Returns a typed error when source verification, workspace loading, or
    /// source compatibility fails, or a source update is required.
    pub fn open(
        repository_root: impl Into<PathBuf>,
        source_package_path: impl Into<PathBuf>,
        cache_root: impl Into<PathBuf>,
    ) -> Result<Self, ProjectSessionError> {
        let repository_root = repository_root.into();
        let source_package_path = source_package_path.into();
        let source_package =
            SourcePackage::open(&source_package_path, cache_root.into()).map_err(|source| {
                ProjectSessionError::Source {
                    path: source_package_path.clone(),
                    source,
                }
            })?;
        Self::open_from_source_package(repository_root, source_package)
    }

    /// Opens an existing project from an HSP that has already been fully
    /// validated by [`SourcePackage::open`]. The package is consumed and
    /// becomes the package owned by the session, so callers can validate an
    /// association before workspace compatibility is checked without
    /// reopening the archive.
    ///
    /// # Errors
    ///
    /// Returns a typed error when workspace loading or source compatibility
    /// fails, or when the workspace requires a source update.
    pub fn open_from_source_package(
        repository_root: impl Into<PathBuf>,
        source_package: SourcePackage,
    ) -> Result<Self, ProjectSessionError> {
        let trace = PerfTrace::new();
        let repository_root = repository_root.into();
        let source_package_path = source_package.package_path().to_owned();
        let store = WorkspaceStore::new(repository_root.clone());
        let workspace = load_compatible_workspace(&repository_root, &store, &source_package)?;
        trace.mark("workspace.guidance");

        Ok(Self {
            repository_root,
            source_package_path,
            store,
            workspace,
            source_package,
            row_keys: BTreeMap::new(),
        })
    }

    /// Plans the source update that would move the workspace onto the
    /// package source, without writing anything.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the workspace cannot be read, uses another
    /// source language, or the plan cannot be built.
    pub fn preview_source_update(
        repository_root: impl Into<PathBuf>,
        source_package: &SourcePackage,
    ) -> Result<SourceUpdateReport, ProjectSessionError> {
        let repository_root = repository_root.into();
        let store = WorkspaceStore::new(repository_root.clone());
        let stored = read_stored(&repository_root, &store)?;
        require_source_language(&repository_root, &stored, source_package)?;
        plan_update(&repository_root, &stored, source_package)
    }

    /// Opens a project and, when required, first applies the deterministic
    /// source update that moves it onto the package source.
    ///
    /// The update keeps every unit, target, note, and translation-unit ID. It
    /// rebinds units whose occurrence is established deterministically,
    /// marks changed source text for review, and detaches units without a
    /// current occurrence. Changed shards are published first and the
    /// manifest last, so an interrupted update is planned again on the next
    /// open. The report is `None` when no update was required.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the workspace cannot be read, uses another
    /// source language, or the update cannot be planned or published.
    pub fn open_with_source_update(
        repository_root: impl Into<PathBuf>,
        source_package: SourcePackage,
    ) -> Result<(Self, Option<SourceUpdateReport>), ProjectSessionError> {
        let repository_root = repository_root.into();
        let source_package_path = source_package.package_path().to_owned();
        let store = WorkspaceStore::new(repository_root.clone());
        let stored = read_stored(&repository_root, &store)?;
        require_source_language(&repository_root, &stored, &source_package)?;

        let (workspace, report) =
            if source_update_requirement(&repository_root, &stored, &source_package)?.is_none() {
                let workspace = store
                    .activate(stored)
                    .map_err(|source| store_error(&repository_root, source))?;
                (workspace, None)
            } else {
                let (workspace, report) =
                    apply_source_update(&repository_root, &store, &stored, &source_package)?;
                (workspace, Some(report))
            };

        Ok((
            Self {
                repository_root,
                source_package_path,
                store,
                workspace,
                source_package,
                row_keys: BTreeMap::new(),
            },
            report,
        ))
    }

    /// Initializes a new project from a verified HXS source and target language.
    ///
    /// Source verification and workspace construction happen before
    /// [`WorkspaceStore::initialize`] publishes the new `.aeria/` directory.
    /// Existing project state is never replaced.
    ///
    /// # Errors
    ///
    /// Returns a typed error when source verification, workspace construction,
    /// or atomic workspace initialization fails.
    pub fn initialize(
        repository_root: impl Into<PathBuf>,
        source_package_path: impl Into<PathBuf>,
        cache_root: impl Into<PathBuf>,
        target_language: impl Into<String>,
    ) -> Result<Self, ProjectSessionError> {
        let repository_root = repository_root.into();
        let source_package_path = source_package_path.into();
        let source_package =
            SourcePackage::open(&source_package_path, cache_root.into()).map_err(|source| {
                ProjectSessionError::Source {
                    path: source_package_path.clone(),
                    source,
                }
            })?;
        Self::initialize_from_source_package(repository_root, source_package, target_language)
    }

    /// Initializes a project from a package that has already been fully
    /// validated by [`SourcePackage::open`]. This constructor is used by the
    /// Atlas creation flow so the package is not reopened and source evidence
    /// is not scanned twice.
    ///
    /// # Errors
    ///
    /// Returns an error when workspace construction or atomic workspace
    /// initialization fails.
    pub fn initialize_from_source_package(
        repository_root: impl Into<PathBuf>,
        source_package: SourcePackage,
        target_language: impl Into<String>,
    ) -> Result<Self, ProjectSessionError> {
        let repository_root = repository_root.into();
        let source_package_path = source_package.package_path().to_owned();
        let workspace =
            Workspace::from_verified_snapshot(source_package.source(), target_language.into())
                .map_err(|source| ProjectSessionError::Workspace {
                    repository_root: repository_root.clone(),
                    source_package_path: source_package_path.clone(),
                    source,
                })?;
        let store = WorkspaceStore::new(repository_root.clone());
        store
            .initialize(&workspace)
            .map_err(|source| ProjectSessionError::Store {
                repository_root: repository_root.clone(),
                source,
            })?;

        Ok(Self {
            repository_root,
            source_package_path,
            store,
            workspace,
            source_package,
            row_keys: BTreeMap::new(),
        })
    }

    /// Reloads the workspace from disk after the repository changed outside
    /// the ordinary mutation path, for example after a Git merge.
    ///
    /// The reloaded state passes the same validation, source compatibility,
    /// and source-guidance checks as [`ProjectSession::open_from_source_package`].
    /// On failure the session keeps its previous in-memory state, whose store
    /// cache fails closed on the next mutation.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the on-disk workspace is invalid,
    /// incompatible with the session source, or requires a source update.
    pub fn reload_workspace(&mut self) -> Result<(), ProjectSessionError> {
        let store = WorkspaceStore::new(self.repository_root.clone());
        let workspace =
            load_compatible_workspace(&self.repository_root, &store, &self.source_package)?;
        self.store = store;
        self.workspace = workspace;
        Ok(())
    }

    /// Reloads the workspace after a Git operation and reconciles it with the
    /// session source when the merged state requires it.
    ///
    /// A merge can combine units bound against different game versions, for
    /// example translations made on a branch that had not applied the latest
    /// source update. Their stored source facts then no longer describe the
    /// session source. Such state is moved onto the source by the same
    /// deterministic update used for game patches: a changed occurrence is
    /// marked for review, a binding claimed twice keeps one deterministic
    /// owner, and no unit, target, note, or ID is removed. The report is
    /// `None` when the reloaded state was already current.
    ///
    /// Only state that records this session's source content is reconciled.
    /// A workspace bound to other source content, or in an older format,
    /// still fails with [`ProjectSessionError::SourceUpdateRequired`]; it
    /// needs that source's package. On failure the session keeps its
    /// previous in-memory state.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the on-disk workspace is invalid, uses
    /// another source language or source content, or the update cannot be
    /// planned or published.
    pub fn reload_and_reconcile_workspace(
        &mut self,
    ) -> Result<Option<SourceUpdateReport>, ProjectSessionError> {
        let store = WorkspaceStore::new(self.repository_root.clone());
        let stored = read_stored(&self.repository_root, &store)?;
        require_source_language(&self.repository_root, &stored, &self.source_package)?;
        let (workspace, report) = match source_update_requirement(
            &self.repository_root,
            &stored,
            &self.source_package,
        )? {
            None => (
                store
                    .activate(stored)
                    .map_err(|source| store_error(&self.repository_root, source))?,
                None,
            ),
            Some(
                SourceUpdateRequirement::SourceFactsMismatch { .. }
                | SourceUpdateRequirement::PermissionChanged { .. },
            ) => {
                let (workspace, report) = apply_source_update(
                    &self.repository_root,
                    &store,
                    &stored,
                    &self.source_package,
                )?;
                (workspace, Some(report))
            }
            Some(requirement) => {
                return Err(ProjectSessionError::SourceUpdateRequired {
                    repository_root: self.repository_root.clone(),
                    source_package_path: self.source_package_path.clone(),
                    requirement,
                });
            }
        };
        self.store = store;
        self.workspace = workspace;
        Ok(report)
    }

    /// Returns the local repository root for this project.
    #[must_use]
    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    /// Returns the local HSP path used by this session.
    #[must_use]
    pub fn source_package_path(&self) -> &Path {
        &self.source_package_path
    }

    /// Returns the loaded sparse workspace.
    #[must_use]
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Returns the verified immutable HXS source handle.
    #[must_use]
    pub fn source(&self) -> &HxsSnapshot {
        self.source_package.source()
    }

    /// Returns the validated source package owned by this session.
    #[must_use]
    pub fn source_package(&self) -> &SourcePackage {
        &self.source_package
    }

    /// Returns detached units in deterministic translation-unit ID order.
    pub fn detached_units(&self) -> impl Iterator<Item = &TranslationUnit> {
        self.workspace.detached_units()
    }
}

fn store_error(repository_root: &Path, source: WorkspaceStoreError) -> ProjectSessionError {
    ProjectSessionError::Store {
        repository_root: repository_root.to_owned(),
        source,
    }
}

fn read_stored(
    repository_root: &Path,
    store: &WorkspaceStore,
) -> Result<StoredWorkspace, ProjectSessionError> {
    store
        .read_stored()
        .map_err(|source| store_error(repository_root, source))
}

fn require_source_language(
    repository_root: &Path,
    stored: &StoredWorkspace,
    source_package: &SourcePackage,
) -> Result<(), ProjectSessionError> {
    let found = source_package.source().metadata().source_language;
    if found == stored.metadata.source_language() {
        return Ok(());
    }
    Err(ProjectSessionError::Compatibility {
        repository_root: repository_root.to_owned(),
        source_package_path: source_package.package_path().to_owned(),
        source: WorkspaceError::SourceLanguageMismatch {
            expected: stored.metadata.source_language().to_owned(),
            found,
        },
    })
}

/// Returns why stored state cannot be edited against the package as is.
fn source_update_requirement(
    repository_root: &Path,
    stored: &StoredWorkspace,
    source_package: &SourcePackage,
) -> Result<Option<SourceUpdateRequirement>, ProjectSessionError> {
    if stored.format_version != FORMAT_VERSION {
        return Ok(Some(SourceUpdateRequirement::FormatMigration {
            version: stored.format_version,
        }));
    }
    let source_content_id = source_package.source().metadata().content_id;
    if source_content_id != stored.metadata.source_content_id() {
        return Ok(Some(SourceUpdateRequirement::ContentChanged {
            workspace_content_id: stored.metadata.source_content_id().to_owned(),
            source_content_id,
        }));
    }
    let mismatched = stored.duplicate_bound_bindings
        + count_mismatched_bound_units(stored, source_package.source()).map_err(|source| {
            ProjectSessionError::SourceUpdate {
                repository_root: repository_root.to_owned(),
                source: SourceUpdateError::SourceRead(source),
            }
        })?;
    if mismatched > 0 {
        return Ok(Some(SourceUpdateRequirement::SourceFactsMismatch {
            units: mismatched,
        }));
    }
    let guidance = source_package.guidance_index();
    let blocked_units = stored
        .units
        .values()
        .filter(|unit| unit.is_bound())
        .filter(|unit| {
            let binding = unit.source_binding();
            !guidance.is_translatable(
                binding.sheet_name(),
                binding.row_id(),
                binding.subrow_id(),
                binding.column_index(),
            )
        })
        .count();
    Ok((blocked_units > 0).then_some(SourceUpdateRequirement::PermissionChanged { blocked_units }))
}

/// Counts bound units whose stored source facts do not describe `source`:
/// the sheet, String column, or row is missing, the layout differs, or the
/// fingerprint differs. Only sheets that hold bound units are read, one
/// bounded hash-only page at a time.
fn count_mismatched_bound_units(
    stored: &StoredWorkspace,
    source: &HxsSnapshot,
) -> Result<usize, HxsError> {
    let mut by_sheet: BTreeMap<&str, HashMap<(u32, u16, u32), &TranslationUnit>> = BTreeMap::new();
    for unit in stored.units.values().filter(|unit| unit.is_bound()) {
        let binding = unit.source_binding();
        by_sheet.entry(binding.sheet_name()).or_default().insert(
            (
                binding.row_id(),
                binding.subrow_id(),
                binding.column_index(),
            ),
            unit,
        );
    }

    let mut mismatched = 0;
    for (sheet_name, mut pending) in by_sheet {
        let Some(sheet) = source.sheet(sheet_name) else {
            mismatched += pending.len();
            continue;
        };
        let string_offsets: HashMap<u32, u32> = sheet
            .columns
            .iter()
            .filter(|column| column.column_type == ColumnType::String)
            .map(|column| (column.index, column.offset))
            .collect();
        pending.retain(|&(_, _, column_index), unit| {
            let current = string_offsets
                .get(&column_index)
                .map(|offset| source_layout_from_hashes(&sheet.hashes.schema, *offset));
            let matches = current.is_some() && unit.source_layout() == current;
            if !matches {
                mismatched += 1;
            }
            matches
        });

        let mut after = None;
        while !pending.is_empty() {
            let page = source.page_string_occurrences(
                sheet_name,
                after.as_ref(),
                MAX_STRING_OCCURRENCE_PAGE_SIZE,
            )?;
            for occurrence in &page.occurrences {
                let coordinate = &occurrence.coordinate;
                let key = (
                    coordinate.row_id,
                    coordinate.subrow_id,
                    coordinate.column_index,
                );
                if let Some(unit) = pending.remove(&key) {
                    let fingerprint = source_fingerprint_from_hashes(
                        &occurrence.macro_text_hash,
                        occurrence.raw_value_hash.as_ref(),
                        &occurrence.row_technical_hash,
                    );
                    if unit.source_fingerprint() != &fingerprint {
                        mismatched += 1;
                    }
                }
            }
            match page.next_after {
                Some(next) => after = Some(next),
                None => break,
            }
        }
        // Units whose row/subrow no longer holds a String cell.
        mismatched += pending.len();
    }
    Ok(mismatched)
}

/// Plans, applies, and publishes the update that moves `stored` onto the
/// package source.
fn apply_source_update(
    repository_root: &Path,
    store: &WorkspaceStore,
    stored: &StoredWorkspace,
    source_package: &SourcePackage,
) -> Result<(Workspace, SourceUpdateReport), ProjectSessionError> {
    let report = plan_update(repository_root, stored, source_package)?;
    let (planned, shards) = apply_plan(
        &stored.metadata,
        &stored.units,
        &report.plan,
        stored.format_version != FORMAT_VERSION,
    )
    .map_err(|source| ProjectSessionError::Workspace {
        repository_root: repository_root.to_owned(),
        source_package_path: source_package.package_path().to_owned(),
        source,
    })?;
    let workspace = store
        .publish_source_update(&planned, &shards)
        .map_err(|source| store_error(repository_root, source))?;
    Ok((workspace, report))
}

fn plan_update(
    repository_root: &Path,
    stored: &StoredWorkspace,
    source_package: &SourcePackage,
) -> Result<SourceUpdateReport, ProjectSessionError> {
    let guidance = source_package.guidance_index();
    let plan = plan_source_update(
        &stored.metadata,
        stored.units.values(),
        source_package.source(),
        |binding| {
            guidance.is_translatable(
                binding.sheet_name(),
                binding.row_id(),
                binding.subrow_id(),
                binding.column_index(),
            )
        },
    )
    .map_err(|source| ProjectSessionError::SourceUpdate {
        repository_root: repository_root.to_owned(),
        source,
    })?;
    Ok(SourceUpdateReport {
        plan,
        previous_format_version: stored.format_version,
    })
}

fn load_compatible_workspace(
    repository_root: &Path,
    store: &WorkspaceStore,
    source_package: &SourcePackage,
) -> Result<Workspace, ProjectSessionError> {
    let stored = read_stored(repository_root, store)?;
    require_source_language(repository_root, &stored, source_package)?;
    if let Some(requirement) = source_update_requirement(repository_root, &stored, source_package)?
    {
        return Err(ProjectSessionError::SourceUpdateRequired {
            repository_root: repository_root.to_owned(),
            source_package_path: source_package.package_path().to_owned(),
            requirement,
        });
    }
    store
        .activate(stored)
        .map_err(|source| store_error(repository_root, source))
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
