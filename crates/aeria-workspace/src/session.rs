//! Owned application-layer state for one opened Aeria project.

use std::path::{Path, PathBuf};
use std::time::Instant;

use aeria_core::{SourceBinding, TranslationUnitId};
use aeria_hsp::{HspError, SourcePackage};
use aeria_hxs::HxsSnapshot;
use thiserror::Error;

use crate::{Workspace, WorkspaceError, WorkspaceStore, WorkspaceStoreError};

/// The owned runtime representation of one opened Aeria project.
///
/// A session keeps the local source-package path alongside the validated
/// package, verified immutable HXS handle, loaded sparse workspace,
/// persistence adapter, and repository root. Package and cache paths are
/// runtime configuration and are not part of Workspace Format v1 state.
pub struct ProjectSession {
    repository_root: PathBuf,
    source_package_path: PathBuf,
    pub(crate) store: WorkspaceStore,
    pub(crate) workspace: Workspace,
    pub(crate) source_package: SourcePackage,
}

/// Errors raised while opening or initializing a project session.
#[derive(Debug, Error)]
pub enum ProjectSessionError {
    /// The local HSP file could not be opened or fully verified.
    #[error("failed to open and verify HSP source package {path}: {source}")]
    Source { path: PathBuf, source: HspError },

    /// Workspace Format v1 could not be loaded or initialized.
    #[error("failed to load or initialize workspace at {repository_root}: {source}")]
    Store {
        repository_root: PathBuf,
        #[source]
        source: WorkspaceStoreError,
    },

    /// The loaded workspace is bound to a different source snapshot.
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

    /// The existing sparse workspace contains a binding blocked by HSG.
    #[error(
        "workspace at {repository_root} contains translation unit {translation_unit_id} at {source_binding:?}, which is not permitted by source guidance"
    )]
    BlockedWorkspaceUnit {
        repository_root: PathBuf,
        translation_unit_id: TranslationUnitId,
        source_binding: SourceBinding,
    },
}

impl ProjectSession {
    /// Opens an existing project against its currently bound, verified HXS source.
    ///
    /// Opening verifies the source, loads the complete sparse Workspace Format
    /// v1 state, checks the existing workspace/source binding contract, and
    /// only then constructs the session. It never updates either input.
    ///
    /// # Errors
    ///
    /// Returns a typed error when source verification, workspace loading, or
    /// source compatibility fails.
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
    /// Returns a typed error when workspace loading, source compatibility, or
    /// source-guidance validation fails.
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
        })
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
    /// or atomic Workspace Format v1 initialization fails.
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
    /// Returns an error when workspace construction or atomic Workspace
    /// Format v1 initialization fails.
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
    /// incompatible with the session source, or contains blocked units.
    pub fn reload_workspace(&mut self) -> Result<(), ProjectSessionError> {
        let store = WorkspaceStore::new(self.repository_root.clone());
        let workspace =
            load_compatible_workspace(&self.repository_root, &store, &self.source_package)?;
        self.store = store;
        self.workspace = workspace;
        Ok(())
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
}

fn load_compatible_workspace(
    repository_root: &Path,
    store: &WorkspaceStore,
    source_package: &SourcePackage,
) -> Result<Workspace, ProjectSessionError> {
    let workspace = store.load().map_err(|source| ProjectSessionError::Store {
        repository_root: repository_root.to_owned(),
        source,
    })?;
    workspace
        .require_compatible_snapshot(source_package.source())
        .map_err(|source| ProjectSessionError::Compatibility {
            repository_root: repository_root.to_owned(),
            source_package_path: source_package.package_path().to_owned(),
            source,
        })?;

    if let Some(unit) = workspace.units().find(|unit| {
        let binding = unit.source_binding();
        !source_package.guidance_index().is_translatable(
            binding.sheet_name(),
            binding.row_id(),
            binding.subrow_id(),
            binding.column_index(),
        )
    }) {
        return Err(ProjectSessionError::BlockedWorkspaceUnit {
            repository_root: repository_root.to_owned(),
            translation_unit_id: unit.id(),
            source_binding: unit.source_binding().clone(),
        });
    }
    Ok(workspace)
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
