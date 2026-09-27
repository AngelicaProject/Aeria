//! Transactional application-level translation mutations.

use aeria_core::{ReviewState, SourceBinding, SourceFacts, TranslationUnit, TranslationUnitId};
use aeria_source::{SheetLookup, SourceCell};
use thiserror::Error;

use crate::{ProjectSession, WorkspaceError, WorkspaceStoreError};

/// Errors raised while applying one transactional translation mutation.
#[derive(Debug, Error)]
pub enum TranslationMutationError {
    /// The requested target contains no non-whitespace content.
    #[error("translation target must not be empty or whitespace-only")]
    EmptyTarget,

    /// The cell does not exist or is not translatable.
    #[error("source cell is not translatable: {source_binding:?}")]
    SourceNotTranslatable { source_binding: SourceBinding },

    /// The requested domain mutation was invalid.
    #[error("translation workspace mutation failed: {0}")]
    Workspace(#[from] WorkspaceError),

    /// The canonical shard could not be persisted.
    #[error("translation workspace persistence failed: {0}")]
    Persistence(#[from] WorkspaceStoreError),

    /// An existing unit no longer describes the game's cell.
    #[error(
        "translation unit {translation_unit_id} at {source_binding:?} has stale source facts: persisted {persisted:?}, game {found:?}"
    )]
    SourceIntegrity {
        translation_unit_id: TranslationUnitId,
        source_binding: Box<SourceBinding>,
        persisted: Box<SourceFacts>,
        found: Box<Option<SourceFacts>>,
    },
}

/// The state an assisted write expects its unit to be in, captured when the
/// translation was produced. `None` fields describe an untranslated string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistedExpectation {
    pub target: Option<String>,
    pub review_state: Option<ReviewState>,
}

/// Errors from an assisted write. Nothing is written when one is returned.
#[derive(Debug, Error)]
pub enum AssistedWriteError {
    /// The ordinary target checks or persistence failed.
    #[error(transparent)]
    Mutation(#[from] TranslationMutationError),

    /// The translation breaks the assisted structure policy.
    #[error("the translation breaks the source structure: {}", .messages.join("; "))]
    Structure { messages: Vec<String> },

    /// The unit changed after the translation was produced.
    #[error("the string changed after the translation was produced")]
    Conflict { current: AssistedExpectation },

    /// The unit is reviewed and the write was not approved to replace it.
    #[error("the string is reviewed; replacing it needs explicit approval")]
    Reviewed,
}

impl ProjectSession {
    /// Returns the source facts of a translatable cell.
    fn translatable_facts(
        &self,
        source_binding: &SourceBinding,
    ) -> Result<SourceFacts, TranslationMutationError> {
        let not_translatable = || TranslationMutationError::SourceNotTranslatable {
            source_binding: source_binding.clone(),
        };
        let SheetLookup::Present(sheet) = self
            .source
            .sheet(source_binding.sheet_name())
            .map_err(|error| TranslationMutationError::Workspace(WorkspaceError::Source(error)))?
        else {
            return Err(not_translatable());
        };
        let (row, subrow, column) = (
            source_binding.row_id(),
            source_binding.subrow_id(),
            source_binding.column_index(),
        );
        if !sheet
            .cell(row, subrow, column)
            .is_some_and(|cell: SourceCell<'_>| cell.translatable)
        {
            return Err(not_translatable());
        }
        sheet
            .facts(row, subrow, column)
            .ok_or_else(not_translatable)
    }

    /// Returns the source macro text of one translatable cell.
    ///
    /// # Errors
    ///
    /// Returns `SourceNotTranslatable` for a cell that does not exist or is
    /// not translatable.
    pub fn source_macro(
        &self,
        source_binding: &SourceBinding,
    ) -> Result<String, TranslationMutationError> {
        self.translatable_facts(source_binding)
            .map(|facts| facts.text().to_owned())
    }

    /// Returns the current target and review state of a bound unit, or the
    /// untranslated state.
    #[must_use]
    pub fn assisted_state(&self, source_binding: &SourceBinding) -> AssistedExpectation {
        self.workspace
            .unit_by_source_binding(source_binding)
            .map_or(
                AssistedExpectation {
                    target: None,
                    review_state: None,
                },
                |unit| AssistedExpectation {
                    target: Some(unit.target_macro().to_owned()),
                    review_state: Some(unit.review_state()),
                },
            )
    }

    /// Writes a target produced by assisted translation.
    ///
    /// Besides the ordinary [`Self::set_target`] checks, the target must
    /// satisfy the assisted structure policy against the verified source, and
    /// the unit must still be in the `expected` state (compare-and-set), so a
    /// translation never replaces work saved after it was produced. A
    /// reviewed unit is replaced only when `replace_reviewed` records the
    /// user's explicit approval. The written target is a draft.
    ///
    /// # Errors
    ///
    /// Returns a structure, conflict, reviewed, or ordinary mutation error;
    /// nothing is written in that case.
    pub fn set_assisted_target(
        &mut self,
        source_binding: &SourceBinding,
        target_macro: &str,
        expected: &AssistedExpectation,
        replace_reviewed: bool,
    ) -> Result<TranslationUnitId, AssistedWriteError> {
        if target_macro.trim().is_empty() {
            return Err(TranslationMutationError::EmptyTarget.into());
        }
        let source = self.source_macro(source_binding)?;
        aeria_se::check_assisted_structure(&source, target_macro).map_err(|errors| {
            AssistedWriteError::Structure {
                messages: errors.into_iter().map(|error| error.message).collect(),
            }
        })?;
        let current = self.assisted_state(source_binding);
        if &current != expected {
            return Err(AssistedWriteError::Conflict { current });
        }
        if current.review_state == Some(ReviewState::Reviewed) && !replace_reviewed {
            return Err(AssistedWriteError::Reviewed);
        }
        Ok(self.set_target(source_binding, target_macro)?)
    }
}

impl ProjectSession {
    /// Creates or updates the translation unit at one verified source binding.
    ///
    /// A missing unit is created with a stable ID derived from the game's
    /// cell. An existing unit keeps its durable ID and uses the
    /// normal workspace target-edit semantics. Empty and whitespace-only
    /// targets are rejected before any workspace or persistence mutation.
    ///
    /// # Errors
    ///
    /// Returns a typed empty-target, workspace, persistence, or
    /// source-integrity error.
    pub fn set_target(
        &mut self,
        source_binding: &SourceBinding,
        target_macro: &str,
    ) -> Result<TranslationUnitId, TranslationMutationError> {
        if target_macro.trim().is_empty() {
            return Err(TranslationMutationError::EmptyTarget);
        }
        let facts = self.translatable_facts(source_binding)?;
        let existing_id = self
            .workspace
            .unit_by_source_binding(source_binding)
            .map(TranslationUnit::id);

        let Some(id) = existing_id else {
            self.workspace.require_source_language(&self.source)?;
            let id = self.workspace.create_unit(facts, target_macro)?;
            if let Err(error) = self.store.persist_unit(&self.workspace, id) {
                debug_assert!(self.workspace.remove_unit(id).is_some());
                return Err(error.into());
            }
            return Ok(id);
        };

        self.verify_current_source(id)?;
        let Some(current) = self.workspace.unit(id) else {
            return Err(WorkspaceError::UnitNotFound { id }.into());
        };
        if current.target_macro() == target_macro {
            return Ok(id);
        }

        let previous = current.clone();
        self.workspace.update_target(id, target_macro)?;
        if let Err(error) = self.store.persist_unit(&self.workspace, id) {
            self.workspace.restore_unit(previous);
            return Err(error.into());
        }
        Ok(id)
    }

    /// Replaces the note on an existing translation unit.
    ///
    /// Clearing a note uses `None`. A note-only mutation does not change the
    /// review state, and an identical note does not rewrite the canonical
    /// shard.
    ///
    /// # Errors
    ///
    /// Returns a typed workspace, persistence, or source-integrity error.
    pub fn set_note(
        &mut self,
        translation_unit_id: TranslationUnitId,
        note: Option<String>,
    ) -> Result<(), TranslationMutationError> {
        self.verify_current_source(translation_unit_id)?;
        let Some(current) = self.workspace.unit(translation_unit_id) else {
            return Err(WorkspaceError::UnitNotFound {
                id: translation_unit_id,
            }
            .into());
        };
        if current.translator_note() == note.as_deref() {
            return Ok(());
        }

        let previous = current.clone();
        self.workspace.update_note(translation_unit_id, note)?;
        if let Err(error) = self
            .store
            .persist_unit(&self.workspace, translation_unit_id)
        {
            self.workspace.restore_unit(previous);
            return Err(error.into());
        }
        Ok(())
    }

    /// Applies an explicit review-state operation to an existing unit.
    ///
    /// An identical state does not rewrite the canonical shard.
    ///
    /// # Errors
    ///
    /// Returns a typed workspace, persistence, or source-integrity error.
    pub fn set_review_state(
        &mut self,
        translation_unit_id: TranslationUnitId,
        review_state: ReviewState,
    ) -> Result<(), TranslationMutationError> {
        self.verify_current_source(translation_unit_id)?;
        let Some(current) = self.workspace.unit(translation_unit_id) else {
            return Err(WorkspaceError::UnitNotFound {
                id: translation_unit_id,
            }
            .into());
        };
        if current.review_state() == review_state {
            return Ok(());
        }

        let previous = current.clone();
        self.workspace
            .update_review_state(translation_unit_id, review_state)?;
        if let Err(error) = self
            .store
            .persist_unit(&self.workspace, translation_unit_id)
        {
            self.workspace.restore_unit(previous);
            return Err(error.into());
        }
        Ok(())
    }

    fn verify_current_source(
        &self,
        translation_unit_id: TranslationUnitId,
    ) -> Result<(), TranslationMutationError> {
        let unit =
            self.workspace
                .unit(translation_unit_id)
                .ok_or(WorkspaceError::UnitNotFound {
                    id: translation_unit_id,
                })?;
        if !unit.is_bound() {
            return Err(WorkspaceError::DetachedUnit {
                id: translation_unit_id,
            }
            .into());
        }
        let source_binding = unit.source_binding().clone();
        let found = match self
            .source
            .sheet(source_binding.sheet_name())
            .map_err(WorkspaceError::Source)?
        {
            SheetLookup::Present(sheet) => sheet.facts(
                source_binding.row_id(),
                source_binding.subrow_id(),
                source_binding.column_index(),
            ),
            SheetLookup::Missing | SheetLookup::Unavailable(_) => None,
        };
        if found.as_ref() != Some(unit.source()) {
            return Err(TranslationMutationError::SourceIntegrity {
                translation_unit_id,
                source_binding: Box::new(source_binding),
                persisted: Box::new(unit.source().clone()),
                found: Box::new(found),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use aeria_source::{GameSource, SourceLanguage};
    use aeria_sqpack::testing::{FakeGame, TextSheet};

    use super::*;
    use crate::persistence::{fail_next_publication_for_test, persistence_test_lock};

    fn session() -> (tempfile::TempDir, tempfile::TempDir, ProjectSession) {
        let game = tempfile::tempdir().expect("game");
        FakeGame::new("2026.09.15.0000.0000")
            .with_text("Addon", &TextSheet::new(1, &[0]).row(1, &[(0, "Hello")]))
            .write(game.path())
            .expect("write game");
        let source = GameSource::open(game.path(), SourceLanguage::English).expect("source");
        let repository = tempfile::tempdir().expect("repository");
        let session = ProjectSession::initialize(repository.path(), Arc::new(source), "fr")
            .expect("initialize");
        (game, repository, session)
    }

    #[test]
    fn a_failed_publication_rolls_back_a_new_unit() {
        let _lock = persistence_test_lock();
        let (_game, _repository, mut session) = session();
        let binding = SourceBinding::new("Addon", 1, 0, 0);
        fail_next_publication_for_test();
        assert!(matches!(
            session.set_target(&binding, "Bonjour"),
            Err(TranslationMutationError::Persistence(_))
        ));
        assert!(
            session
                .workspace()
                .unit_by_source_binding(&binding)
                .is_none()
        );
        session
            .set_target(&binding, "Bonjour")
            .expect("a retry succeeds");
    }

    #[test]
    fn a_failed_publication_restores_an_existing_unit() {
        let _lock = persistence_test_lock();
        let (_game, _repository, mut session) = session();
        let binding = SourceBinding::new("Addon", 1, 0, 0);
        let id = session.set_target(&binding, "Bonjour").expect("target");
        let before = session.workspace().unit(id).cloned();
        fail_next_publication_for_test();
        assert!(session.set_target(&binding, "Salut").is_err());
        assert_eq!(session.workspace().unit(id).cloned(), before);
        fail_next_publication_for_test();
        assert!(session.set_note(id, Some("note".to_owned())).is_err());
        assert_eq!(session.workspace().unit(id).cloned(), before);
    }
}
