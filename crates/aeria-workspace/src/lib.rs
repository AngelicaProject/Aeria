//! Sparse translation workspace operations over the installed game.
//! This crate owns the source adapter, in-memory workspace operations,
//! Workspace Format persistence, and the atomic application of source
//! updates planned by `aeria-rebase`.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use aeria_core::{
    DomainValueError, ReviewState, SourceBinding, SourceFacts, TranslationUnit, TranslationUnitId,
    TranslationUnitIdError, WorkspaceMetadata,
};
use aeria_se::{Diagnostic, SemanticValidity, parse};
use aeria_source::{GameSource, SourceError};
use thiserror::Error;

mod mutation;
mod persistence;
mod read;
mod session;
mod update;

pub use mutation::{
    AssistedExpectation, AssistedWrite, AssistedWriteError, TranslationMutationError,
};
pub use persistence::{
    FORMAT_VERSION as WORKSPACE_FORMAT_VERSION, WorkspaceStore, WorkspaceStoreError,
    decode_unit_record, decode_unit_shard, encode_unit_shard, unit_shard_path,
};
pub use read::{
    MAX_TRANSLATION_PAGE_SIZE, SheetTranslationProgress, TranslationCellView,
    TranslationContextCellView, TranslationOverlayView, TranslationReadError, TranslationRowCursor,
    TranslationRowPage, TranslationRowView,
};
pub use session::{ProjectSession, ProjectSessionError, SourceUpdateRequirement};
pub use update::SourceUpdateReport;

/// Errors raised by the in-memory translation workspace.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// Workspace metadata failed core validation.
    #[error("invalid workspace metadata: {0}")]
    InvalidMetadata(#[from] DomainValueError),

    /// The game could not be read.
    #[error("game source error: {0}")]
    Source(#[from] SourceError),

    /// The requested sheet/row/subrow/column is not a String cell of the game.
    #[error("String cell was not found in the game: {binding:?}")]
    SourceCellNotFound { binding: SourceBinding },

    /// The workspace and the game use different source languages.
    #[error("source language mismatch: workspace has {expected:?}, the game source has {found:?}")]
    SourceLanguageMismatch { expected: String, found: String },

    /// Identity inputs exceeded the v1 canonical framing limit.
    #[error("could not derive translation-unit ID: {0}")]
    Identity(#[from] TranslationUnitIdError),

    /// A target macro string failed intrinsic `SeString` validation.
    #[error("target macro string is malformed or unsafe")]
    InvalidTarget { diagnostics: Vec<Diagnostic> },

    /// No unit has the requested durable identity.
    #[error("translation unit was not found: {id}")]
    UnitNotFound { id: TranslationUnitId },

    /// A durable identity is already present in the sparse workspace.
    #[error("translation unit ID already exists: {id}")]
    DuplicateUnitId { id: TranslationUnitId },

    /// A current source coordinate is already owned by another bound unit.
    #[error("source binding already belongs to another translation unit: {binding:?}")]
    DuplicateSourceBinding { binding: SourceBinding },

    /// The operation requires a bound unit, but the unit is detached.
    #[error("translation unit {id} is detached from the current source")]
    DetachedUnit { id: TranslationUnitId },
}

/// A deterministic, sparse in-memory translation workspace.
///
/// Only units explicitly created by the caller are held. Source facts are
/// read on demand from the game; the workspace does not enumerate or cache
/// the source corpus. The binding index contains
/// bound units only; detached units keep their last binding but own no
/// current source occurrence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workspace {
    metadata: WorkspaceMetadata,
    units: BTreeMap<TranslationUnitId, TranslationUnit>,
    source_bindings: BTreeMap<SourceBinding, TranslationUnitId>,
}

impl Workspace {
    /// Creates an empty workspace with one validated source/target binding.
    #[must_use]
    pub fn new(metadata: WorkspaceMetadata) -> Self {
        Self {
            metadata,
            units: BTreeMap::new(),
            source_bindings: BTreeMap::new(),
        }
    }

    /// Creates an empty workspace for the game's source language and version.
    ///
    /// # Errors
    ///
    /// Returns an error when the target language is empty or whitespace-only.
    pub fn for_source(
        source: &GameSource,
        target_language: impl Into<String>,
    ) -> Result<Self, WorkspaceError> {
        let metadata = WorkspaceMetadata::new(
            source.language().code(),
            target_language,
            source.version().clone(),
        )?;
        Ok(Self::new(metadata))
    }

    /// Returns the workspace with other metadata and the same units.
    pub(crate) fn with_metadata(&self, metadata: WorkspaceMetadata) -> Self {
        Self {
            metadata,
            ..self.clone()
        }
    }

    /// Returns project/source metadata without exposing persistence concerns.
    #[must_use]
    pub const fn metadata(&self) -> &WorkspaceMetadata {
        &self.metadata
    }

    /// Returns sparse units in deterministic translation-unit ID order.
    pub fn units(&self) -> impl Iterator<Item = &TranslationUnit> {
        self.units.values()
    }

    /// Finds a unit by durable identity.
    #[must_use]
    pub fn unit(&self, id: TranslationUnitId) -> Option<&TranslationUnit> {
        self.units.get(&id)
    }

    /// Returns detached units in deterministic translation-unit ID order.
    pub fn detached_units(&self) -> impl Iterator<Item = &TranslationUnit> {
        self.units.values().filter(|unit| !unit.is_bound())
    }

    /// Finds the bound unit at a current source coordinate.
    #[must_use]
    pub fn unit_by_source_binding(&self, binding: &SourceBinding) -> Option<&TranslationUnit> {
        self.source_bindings
            .get(binding)
            .and_then(|id| self.units.get(id))
    }

    /// Creates a draft unit from the source facts of one String cell.
    ///
    /// `target_macro` may be empty, which represents an explicit empty target
    /// on a unit that is present in this sparse workspace.
    ///
    /// # Errors
    ///
    /// Returns an error when the target is unsafe or the identity or binding
    /// is already present.
    pub fn create_unit(
        &mut self,
        source: SourceFacts,
        target_macro: &str,
    ) -> Result<TranslationUnitId, WorkspaceError> {
        validate_target(target_macro)?;
        let id = TranslationUnitId::derive(source.binding(), source.text())?;
        self.insert_unit(TranslationUnit::new(id, source, target_macro))?;
        Ok(id)
    }

    /// Updates a target after validating its intrinsic `SeString` syntax.
    ///
    /// Protected source/target structures are deliberately not compared here:
    /// that conservative check belongs to future assisted/AI operations.
    ///
    /// # Errors
    ///
    /// Returns an error when the target is unsafe or the unit does not exist.
    pub fn update_target(
        &mut self,
        id: TranslationUnitId,
        target_macro: &str,
    ) -> Result<(), WorkspaceError> {
        validate_target(target_macro)?;
        self.unit_mut(id)?.set_target_macro(target_macro);
        Ok(())
    }

    /// Replaces a translator note without changing review state.
    ///
    /// # Errors
    ///
    /// Returns an error when the unit does not exist.
    pub fn update_note(
        &mut self,
        id: TranslationUnitId,
        note: Option<String>,
    ) -> Result<(), WorkspaceError> {
        self.unit_mut(id)?.set_translator_note(note);
        Ok(())
    }

    /// Applies an explicit review-state operation.
    ///
    /// # Errors
    ///
    /// Returns an error when the unit does not exist.
    pub fn update_review_state(
        &mut self,
        id: TranslationUnitId,
        review_state: ReviewState,
    ) -> Result<(), WorkspaceError> {
        self.unit_mut(id)?.set_review_state(review_state);
        Ok(())
    }

    fn insert_unit(&mut self, unit: TranslationUnit) -> Result<(), WorkspaceError> {
        let id = unit.id();
        let binding = unit.source_binding().clone();
        if self.units.contains_key(&id) {
            return Err(WorkspaceError::DuplicateUnitId { id });
        }
        if unit.is_bound() {
            if self.source_bindings.contains_key(&binding) {
                return Err(WorkspaceError::DuplicateSourceBinding { binding });
            }
            self.source_bindings.insert(binding, id);
        }
        self.units.insert(id, unit);
        Ok(())
    }

    pub(crate) fn remove_unit(&mut self, id: TranslationUnitId) -> Option<TranslationUnit> {
        let unit = self.units.remove(&id)?;
        if unit.is_bound() {
            self.source_bindings.remove(unit.source_binding());
        }
        Some(unit)
    }

    pub(crate) fn restore_unit(&mut self, unit: TranslationUnit) {
        let id = unit.id();
        let binding = unit.source_binding().clone();
        let existing = self
            .units
            .get_mut(&id)
            .expect("transaction rollback target must still exist");
        debug_assert_eq!(existing.source_binding(), &binding);
        debug_assert_eq!(existing.source_status(), unit.source_status());
        *existing = unit;
    }

    fn from_loaded(
        metadata: WorkspaceMetadata,
        units: BTreeMap<TranslationUnitId, TranslationUnit>,
    ) -> Result<Self, WorkspaceError> {
        let mut source_bindings = BTreeMap::new();
        for (id, unit) in units.iter().filter(|(_, unit)| unit.is_bound()) {
            let binding = unit.source_binding().clone();
            if source_bindings.insert(binding.clone(), *id).is_some() {
                return Err(WorkspaceError::DuplicateSourceBinding { binding });
            }
        }
        Ok(Self {
            metadata,
            units,
            source_bindings,
        })
    }

    fn units_in_shard(&self, shard: u8) -> impl Iterator<Item = &TranslationUnit> {
        use std::ops::Bound::{Excluded, Included, Unbounded};

        let mut lower_bytes = [0_u8; 16];
        lower_bytes[0] = shard;
        let lower = TranslationUnitId::from_bytes(lower_bytes);
        let upper = if shard == u8::MAX {
            Unbounded
        } else {
            let mut upper_bytes = [0_u8; 16];
            upper_bytes[0] = shard + 1;
            Excluded(TranslationUnitId::from_bytes(upper_bytes))
        };
        self.units
            .range((Included(lower), upper))
            .map(|(_, unit)| unit)
    }

    fn unit_mut(&mut self, id: TranslationUnitId) -> Result<&mut TranslationUnit, WorkspaceError> {
        let unit = self
            .units
            .get_mut(&id)
            .ok_or(WorkspaceError::UnitNotFound { id })?;
        if !unit.is_bound() {
            return Err(WorkspaceError::DetachedUnit { id });
        }
        Ok(unit)
    }

    /// Requires that `source` is in the workspace's source language.
    pub(crate) fn require_source_language(
        &self,
        source: &GameSource,
    ) -> Result<(), WorkspaceError> {
        if source.language().code() == self.metadata.source_language() {
            return Ok(());
        }
        Err(WorkspaceError::SourceLanguageMismatch {
            expected: self.metadata.source_language().to_owned(),
            found: source.language().code().to_owned(),
        })
    }
}

fn validate_target(target_macro: &str) -> Result<(), WorkspaceError> {
    let validation = parse(target_macro).semantic_validation();
    if matches!(
        validation.status(),
        SemanticValidity::ValidAndUnderstood | SemanticValidity::ValidWithOpaque
    ) {
        Ok(())
    } else {
        Err(WorkspaceError::InvalidTarget {
            diagnostics: validation.diagnostics().to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_unit(sheet_name: &str, row_id: u32, text: &str) -> TranslationUnit {
        let binding = SourceBinding::new(sheet_name, row_id, 0, 0);
        let id = TranslationUnitId::derive(&binding, text).expect("short identity inputs");
        let source = SourceFacts::new(
            binding,
            aeria_core::LayoutHash::from_bytes([0x22; 8]),
            text,
            None,
        );
        TranslationUnit::new(id, source, "target")
    }

    fn test_metadata() -> WorkspaceMetadata {
        WorkspaceMetadata::new("en", "fr", "2026.09.15.0000.0000".parse().expect("version"))
            .expect("test metadata is valid")
    }

    fn assert_index_consistent(workspace: &Workspace) {
        assert_eq!(workspace.units.len(), workspace.source_bindings.len());
        for (binding, id) in &workspace.source_bindings {
            assert_eq!(
                workspace.units.get(id).map(TranslationUnit::source_binding),
                Some(binding)
            );
        }
        for (id, unit) in &workspace.units {
            assert_eq!(
                workspace.source_bindings.get(unit.source_binding()),
                Some(id)
            );
        }
    }

    #[test]
    fn source_binding_index_rejects_duplicates_and_stays_consistent() {
        let mut workspace = Workspace::new(test_metadata());
        let first = test_unit("Synthetic", 42, "first");
        let first_id = first.id();
        let binding = first.source_binding().clone();
        workspace.insert_unit(first).expect("first unit inserts");

        assert_eq!(
            workspace
                .unit_by_source_binding(&binding)
                .map(TranslationUnit::id),
            Some(first_id)
        );
        assert_index_consistent(&workspace);

        let duplicate_binding = test_unit("Synthetic", 42, "other");
        assert_ne!(duplicate_binding.id(), first_id);
        assert!(matches!(
            workspace.insert_unit(duplicate_binding),
            Err(WorkspaceError::DuplicateSourceBinding { binding: found }) if found == binding
        ));
        assert_index_consistent(&workspace);

        let second = test_unit("Synthetic", 7, "second");
        let second_id = second.id();
        let second_binding = second.source_binding().clone();
        workspace.insert_unit(second).expect("second unit inserts");
        workspace
            .update_target(second_id, "updated target")
            .expect("target update");
        workspace
            .update_note(second_id, Some("note".to_owned()))
            .expect("note update");
        workspace
            .update_review_state(second_id, ReviewState::Reviewed)
            .expect("review update");
        assert_eq!(
            workspace
                .unit_by_source_binding(&second_binding)
                .map(TranslationUnit::id),
            Some(second_id)
        );
        assert_index_consistent(&workspace);
    }
}
