//! Owned application-layer state for one opened Aeria project.

use std::path::{Path, PathBuf};

use aeria_hxs::{HxsError, HxsSnapshot};
use thiserror::Error;

use crate::{Workspace, WorkspaceError, WorkspaceStore, WorkspaceStoreError};

/// The owned runtime representation of one opened Aeria project.
///
/// A session keeps the local source path alongside the verified immutable HXS
/// handle, loaded sparse workspace, persistence adapter, and repository root.
/// The source path is runtime configuration and is not part of Workspace
/// Format v1 state.
pub struct ProjectSession {
    repository_root: PathBuf,
    source_path: PathBuf,
    pub(crate) store: WorkspaceStore,
    pub(crate) workspace: Workspace,
    pub(crate) source: HxsSnapshot,
}

/// Errors raised while opening or initializing a project session.
#[derive(Debug, Error)]
pub enum ProjectSessionError {
    /// The local HXS file could not be opened or fully verified.
    #[error("failed to open and verify HXS source {path}: {source}")]
    Source {
        path: PathBuf,
        #[source]
        source: HxsError,
    },

    /// Workspace Format v1 could not be loaded or initialized.
    #[error("failed to load or initialize workspace at {repository_root}: {source}")]
    Store {
        repository_root: PathBuf,
        #[source]
        source: WorkspaceStoreError,
    },

    /// The loaded workspace is bound to a different source snapshot.
    #[error(
        "workspace at {repository_root} is incompatible with HXS source {source_path}: {source}"
    )]
    Compatibility {
        repository_root: PathBuf,
        source_path: PathBuf,
        #[source]
        source: WorkspaceError,
    },

    /// The requested target language could not be used to construct workspace metadata.
    #[error(
        "could not create workspace metadata for {repository_root} from HXS source {source_path}: {source}"
    )]
    Workspace {
        repository_root: PathBuf,
        source_path: PathBuf,
        #[source]
        source: WorkspaceError,
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
        source_path: impl Into<PathBuf>,
    ) -> Result<Self, ProjectSessionError> {
        let repository_root = repository_root.into();
        let source_path = source_path.into();
        let source =
            HxsSnapshot::open(&source_path).map_err(|source| ProjectSessionError::Source {
                path: source_path.clone(),
                source,
            })?;
        let store = WorkspaceStore::new(repository_root.clone());
        let workspace = store.load().map_err(|source| ProjectSessionError::Store {
            repository_root: repository_root.clone(),
            source,
        })?;
        workspace
            .require_compatible_snapshot(&source)
            .map_err(|source| ProjectSessionError::Compatibility {
                repository_root: repository_root.clone(),
                source_path: source_path.clone(),
                source,
            })?;

        Ok(Self {
            repository_root,
            source_path,
            store,
            workspace,
            source,
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
        source_path: impl Into<PathBuf>,
        target_language: impl Into<String>,
    ) -> Result<Self, ProjectSessionError> {
        let repository_root = repository_root.into();
        let source_path = source_path.into();
        let source =
            HxsSnapshot::open(&source_path).map_err(|source| ProjectSessionError::Source {
                path: source_path.clone(),
                source,
            })?;
        let workspace = Workspace::from_verified_snapshot(&source, target_language.into())
            .map_err(|source| ProjectSessionError::Workspace {
                repository_root: repository_root.clone(),
                source_path: source_path.clone(),
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
            source_path,
            store,
            workspace,
            source,
        })
    }

    /// Returns the local repository root for this project.
    #[must_use]
    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    /// Returns the local HXS path used by this session.
    #[must_use]
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    /// Returns the loaded sparse workspace.
    #[must_use]
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Returns the verified immutable HXS source handle.
    #[must_use]
    pub fn source(&self) -> &HxsSnapshot {
        &self.source
    }
}
