//! Owned application-layer state for one opened Aeria project.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aeria_core::{GameVersion, TranslationUnit};
use aeria_rebase::{SourceUpdateError, SourceUpdatePlan, plan_source_update};
use aeria_source::GameSource;
use thiserror::Error;

use crate::persistence::StoredWorkspace;
use crate::update::{SourceUpdateReport, apply_plan};
use crate::{Workspace, WorkspaceError, WorkspaceStore, WorkspaceStoreError};

/// The owned runtime representation of one opened Aeria project.
///
/// A session keeps the opened game source alongside the loaded sparse
/// workspace, its persistence adapter, and the repository root. The game path
/// is runtime configuration and is not part of Workspace Format state.
pub struct ProjectSession {
    repository_root: PathBuf,
    pub(crate) store: WorkspaceStore,
    pub(crate) workspace: Workspace,
    pub(crate) source: Arc<GameSource>,
}

/// Errors raised while opening or initializing a project session.
#[derive(Debug, Error)]
pub enum ProjectSessionError {
    /// The workspace could not be loaded, initialized, or published.
    #[error("failed to load or initialize workspace at {repository_root}: {source}")]
    Store {
        repository_root: PathBuf,
        #[source]
        source: WorkspaceStoreError,
    },

    /// The workspace and the game use different source languages. A source
    /// update cannot change the source language.
    #[error("workspace at {repository_root} is incompatible with the game: {source}")]
    Compatibility {
        repository_root: PathBuf,
        #[source]
        source: WorkspaceError,
    },

    /// Workspace metadata could not be constructed.
    #[error("could not create workspace metadata for {repository_root}: {source}")]
    Workspace {
        repository_root: PathBuf,
        #[source]
        source: WorkspaceError,
    },

    /// The game is older than the version the project describes. The
    /// project cannot be edited until the game is updated.
    #[error(
        "the game version {game} is older than the version {project} of the project at {repository_root}; update the game"
    )]
    GameOutdated {
        repository_root: PathBuf,
        project: GameVersion,
        game: GameVersion,
    },

    /// The workspace must be moved onto the game by a source update before
    /// it can be edited.
    #[error("workspace at {repository_root} requires a source update: {requirement}")]
    SourceUpdateRequired {
        repository_root: PathBuf,
        requirement: SourceUpdateRequirement,
    },

    /// The language is not a BCP 47 language tag a project can translate
    /// into.
    #[error("{tag:?} is not a target language; use a BCP 47 tag such as ru or pt-BR")]
    InvalidTargetLanguage { tag: String },

    /// A source update could not be planned.
    #[error("could not plan the source update for workspace at {repository_root}: {source}")]
    SourceUpdate {
        repository_root: PathBuf,
        #[source]
        source: SourceUpdateError,
    },
}

/// Why a workspace cannot be edited against the game as stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceUpdateRequirement {
    /// The game is newer than the version the project describes.
    GameUpdated {
        project: GameVersion,
        game: GameVersion,
    },
    /// The project describes the game's version, but this many units do not
    /// describe the game: their cell or text differs, another bound unit
    /// claims the same cell, the cell is no longer translatable, or a
    /// detached unit can be attached again. Ordinary editing never produces
    /// this state; a Git merge of work done on different game versions does.
    SourceFactsMismatch { units: usize },
}

impl std::fmt::Display for SourceUpdateRequirement {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GameUpdated { project, game } => write!(
                formatter,
                "the project describes game version {project}, the game is {game}"
            ),
            Self::SourceFactsMismatch { units } => {
                write!(formatter, "{units} translation units do not match the game")
            }
        }
    }
}

impl ProjectSession {
    /// Opens an existing project against the game.
    ///
    /// Opening loads the complete sparse workspace and checks that it
    /// describes the game. It never writes; a workspace that needs a source
    /// update is reported as [`ProjectSessionError::SourceUpdateRequired`].
    ///
    /// # Errors
    ///
    /// Returns a typed error when the workspace cannot be loaded, uses
    /// another source language or a newer game version, or needs a source
    /// update.
    pub fn open(
        repository_root: impl Into<PathBuf>,
        source: Arc<GameSource>,
    ) -> Result<Self, ProjectSessionError> {
        let repository_root = repository_root.into();
        let store = WorkspaceStore::new(repository_root.clone());
        let stored = read_stored(&repository_root, &store)?;
        let plan = plan_for(&repository_root, &stored, &source)?;
        if let Some(requirement) = requirement(&stored, &plan) {
            return Err(ProjectSessionError::SourceUpdateRequired {
                repository_root,
                requirement,
            });
        }
        let workspace = store
            .activate(stored)
            .map_err(|source| store_error(&repository_root, source))?;
        Ok(Self {
            repository_root,
            store,
            workspace,
            source,
        })
    }

    /// Plans the source update that would move the workspace onto the game,
    /// without writing anything.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the workspace cannot be read, uses another
    /// source language or a newer game version, or the plan cannot be built.
    pub fn preview_source_update(
        repository_root: impl Into<PathBuf>,
        source: &GameSource,
    ) -> Result<SourceUpdateReport, ProjectSessionError> {
        let repository_root = repository_root.into();
        let store = WorkspaceStore::new(repository_root.clone());
        let stored = read_stored(&repository_root, &store)?;
        let plan = plan_for(&repository_root, &stored, source)?;
        Ok(SourceUpdateReport { plan })
    }

    /// Opens a project and, when required, first applies the deterministic
    /// source update that moves it onto the game.
    ///
    /// The update keeps every unit, target, note, and ID. It rebinds units
    /// whose cell is established deterministically, marks changed source
    /// text for review, and detaches units without a current cell. Changed
    /// shards are published first and the manifest last, so an interrupted
    /// update is planned again on the next open. The report is `None` when
    /// no update was required.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the workspace cannot be read, uses another
    /// source language or a newer game version, or the update cannot be
    /// planned or published.
    pub fn open_with_source_update(
        repository_root: impl Into<PathBuf>,
        source: Arc<GameSource>,
    ) -> Result<(Self, Option<SourceUpdateReport>), ProjectSessionError> {
        let repository_root = repository_root.into();
        let store = WorkspaceStore::new(repository_root.clone());
        let stored = read_stored(&repository_root, &store)?;
        let plan = plan_for(&repository_root, &stored, &source)?;
        let (workspace, report) = if plan.changes_workspace() {
            let workspace = apply_update(&repository_root, &store, &stored, &plan)?;
            (workspace, Some(SourceUpdateReport { plan }))
        } else {
            let workspace = store
                .activate(stored)
                .map_err(|source| store_error(&repository_root, source))?;
            (workspace, None)
        };
        Ok((
            Self {
                repository_root,
                store,
                workspace,
                source,
            },
            report,
        ))
    }

    /// Initializes a new project for the game and a target language.
    ///
    /// Existing `.aeria/` state is never replaced.
    ///
    /// # Errors
    ///
    /// Returns a typed error when workspace construction or atomic workspace
    /// initialization fails.
    pub fn initialize(
        repository_root: impl Into<PathBuf>,
        source: Arc<GameSource>,
        target_language: impl Into<String>,
    ) -> Result<Self, ProjectSessionError> {
        let repository_root = repository_root.into();
        let workspace = Workspace::for_source(&source, target_language).map_err(|source| {
            ProjectSessionError::Workspace {
                repository_root: repository_root.clone(),
                source,
            }
        })?;
        let store = WorkspaceStore::new(repository_root.clone());
        store
            .initialize(&workspace)
            .map_err(|source| store_error(&repository_root, source))?;
        Ok(Self {
            repository_root,
            store,
            workspace,
            source,
        })
    }

    /// Sets the project's target language, the language it translates into.
    ///
    /// Only the manifest changes; units, targets, and IDs stay as they are.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectSessionError::InvalidTargetLanguage`] for a value
    /// that is not a language tag or is `und`, and a store error when the
    /// manifest cannot be published.
    pub fn set_target_language(&mut self, tag: &str) -> Result<(), ProjectSessionError> {
        if !aeria_core::is_target_language(tag) {
            return Err(ProjectSessionError::InvalidTargetLanguage {
                tag: tag.to_owned(),
            });
        }
        if self.workspace.metadata().target_language() == tag {
            return Ok(());
        }
        let metadata = self
            .workspace
            .metadata()
            .with_target_language(tag)
            .map_err(|source| ProjectSessionError::Workspace {
                repository_root: self.repository_root.clone(),
                source: source.into(),
            })?;
        let next = self.workspace.with_metadata(metadata);
        self.workspace = self
            .store
            .publish_metadata(&next)
            .map_err(|source| store_error(&self.repository_root, source))?;
        Ok(())
    }

    /// Reloads the workspace from disk after the repository changed outside
    /// the ordinary mutation path, for example after a Git merge.
    ///
    /// The reloaded state passes the same checks as [`ProjectSession::open`].
    /// On failure the session keeps its previous in-memory state, whose store
    /// cache fails closed on the next mutation.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the on-disk workspace is invalid, does not
    /// describe the game, or requires a source update.
    pub fn reload_workspace(&mut self) -> Result<(), ProjectSessionError> {
        let store = WorkspaceStore::new(self.repository_root.clone());
        let stored = read_stored(&self.repository_root, &store)?;
        let plan = plan_for(&self.repository_root, &stored, &self.source)?;
        if let Some(requirement) = requirement(&stored, &plan) {
            return Err(ProjectSessionError::SourceUpdateRequired {
                repository_root: self.repository_root.clone(),
                requirement,
            });
        }
        self.workspace = store
            .activate(stored)
            .map_err(|source| store_error(&self.repository_root, source))?;
        self.store = store;
        Ok(())
    }

    /// Reloads the workspace after a Git operation and reconciles it with the
    /// game when the merged state requires it.
    ///
    /// A merge can combine units bound on different game versions, for
    /// example translations made on a branch that had not applied the latest
    /// source update. Such state is moved onto the game by the same
    /// deterministic update used for game patches: a changed cell is marked
    /// for review, a cell claimed twice keeps one deterministic owner, and no
    /// unit, target, note, or ID is removed. The report is `None` when the
    /// reloaded state was already current.
    ///
    /// Only state that records the session's game version is reconciled. A
    /// workspace that records an older version still fails with
    /// [`ProjectSessionError::SourceUpdateRequired`]. On failure the session
    /// keeps its previous in-memory state.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the on-disk workspace is invalid, uses
    /// another source language or game version, or the update cannot be
    /// planned or published.
    pub fn reload_and_reconcile_workspace(
        &mut self,
    ) -> Result<Option<SourceUpdateReport>, ProjectSessionError> {
        let store = WorkspaceStore::new(self.repository_root.clone());
        let stored = read_stored(&self.repository_root, &store)?;
        let plan = plan_for(&self.repository_root, &stored, &self.source)?;
        let (workspace, report) = match requirement(&stored, &plan) {
            None => (
                store
                    .activate(stored)
                    .map_err(|source| store_error(&self.repository_root, source))?,
                None,
            ),
            Some(SourceUpdateRequirement::SourceFactsMismatch { .. }) => {
                let workspace = apply_update(&self.repository_root, &store, &stored, &plan)?;
                (workspace, Some(SourceUpdateReport { plan }))
            }
            Some(requirement) => {
                return Err(ProjectSessionError::SourceUpdateRequired {
                    repository_root: self.repository_root.clone(),
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

    /// Returns the loaded sparse workspace.
    #[must_use]
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Returns the game source.
    #[must_use]
    pub fn source(&self) -> &GameSource {
        &self.source
    }

    /// Returns a shared handle to the game source.
    #[must_use]
    pub fn source_handle(&self) -> Arc<GameSource> {
        Arc::clone(&self.source)
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

/// Plans the update of `stored` onto the game, mapping the planner's
/// language and version errors to session errors.
fn plan_for(
    repository_root: &Path,
    stored: &StoredWorkspace,
    source: &GameSource,
) -> Result<SourceUpdatePlan, ProjectSessionError> {
    plan_source_update(&stored.metadata, stored.units.values(), source).map_err(|error| match error
    {
        SourceUpdateError::SourceLanguageMismatch { expected, found } => {
            ProjectSessionError::Compatibility {
                repository_root: repository_root.to_owned(),
                source: WorkspaceError::SourceLanguageMismatch { expected, found },
            }
        }
        SourceUpdateError::GameOutdated { project, game } => ProjectSessionError::GameOutdated {
            repository_root: repository_root.to_owned(),
            project,
            game,
        },
        source => ProjectSessionError::SourceUpdate {
            repository_root: repository_root.to_owned(),
            source,
        },
    })
}

/// Returns why stored state cannot be edited against the game as is.
fn requirement(
    stored: &StoredWorkspace,
    plan: &SourceUpdatePlan,
) -> Option<SourceUpdateRequirement> {
    if plan.changes_game_version() {
        return Some(SourceUpdateRequirement::GameUpdated {
            project: stored.metadata.game_version().clone(),
            game: plan.game_version.clone(),
        });
    }
    (plan.summary.changed_units > 0).then_some(SourceUpdateRequirement::SourceFactsMismatch {
        units: plan.summary.changed_units,
    })
}

/// Applies and publishes a plan.
fn apply_update(
    repository_root: &Path,
    store: &WorkspaceStore,
    stored: &StoredWorkspace,
    plan: &SourceUpdatePlan,
) -> Result<Workspace, ProjectSessionError> {
    let (planned, shards) =
        apply_plan(&stored.metadata, &stored.units, plan).map_err(|source| {
            ProjectSessionError::Workspace {
                repository_root: repository_root.to_owned(),
                source,
            }
        })?;
    store
        .publish_source_update(&planned, &shards)
        .map_err(|source| store_error(repository_root, source))
}
