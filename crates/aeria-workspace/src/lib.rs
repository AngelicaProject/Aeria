//! Sparse translation workspace operations over verified HXS source.
//! This crate owns the source adapter, in-memory workspace operations, and
//! Workspace Format v1 persistence.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use aeria_core::{
    DomainValueError, ReviewState, Sha256Hash, SourceBinding, SourceFingerprint, TranslationUnit,
    TranslationUnitId, TranslationUnitIdError, WorkspaceMetadata,
};
use aeria_hxs::{HxsError, HxsSnapshot};
use aeria_se::{Diagnostic, SemanticValidity, parse};
use thiserror::Error;

mod persistence;

pub use persistence::{WorkspaceStore, WorkspaceStoreError};

/// Errors raised by the in-memory translation workspace.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// Workspace metadata failed core validation.
    #[error("invalid workspace metadata: {0}")]
    InvalidMetadata(#[from] DomainValueError),

    /// A verified HXS reader operation failed.
    #[error("verified HXS source error: {0}")]
    Hxs(#[from] HxsError),

    /// The requested sheet/row/subrow/column is not a String cell in HXS.
    #[error("HXS String cell was not found: {binding:?}")]
    SourceCellNotFound { binding: SourceBinding },

    /// The workspace and verified source use different source languages.
    #[error("source language mismatch: workspace has {expected:?}, snapshot has {found:?}")]
    SourceLanguageMismatch { expected: String, found: String },

    /// The workspace and verified source are bound to different content.
    #[error("source content ID mismatch: workspace has {expected:?}, snapshot has {found:?}")]
    SourceContentMismatch { expected: String, found: String },

    /// The workspace and verified source are bound to different snapshots.
    #[error("source snapshot ID mismatch: workspace has {expected:?}, snapshot has {found:?}")]
    SourceSnapshotMismatch { expected: String, found: String },

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

    /// A current source coordinate is already owned by another unit.
    #[error("source binding already belongs to another translation unit: {binding:?}")]
    DuplicateSourceBinding { binding: SourceBinding },
}

/// A deterministic, sparse in-memory translation workspace.
///
/// Only units explicitly created by the caller are held. Source cells are
/// fetched on demand from an already verified [`HxsSnapshot`]; the workspace
/// does not enumerate or cache the source corpus.
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

    /// Creates an empty workspace bound to an already verified HXS snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the target language is empty or whitespace-only.
    pub fn from_verified_snapshot(
        snapshot: &HxsSnapshot,
        target_language: impl Into<String>,
    ) -> Result<Self, WorkspaceError> {
        let source = snapshot.metadata();
        let metadata = WorkspaceMetadata::new(
            source.source_language,
            target_language,
            source.content_id,
            source.snapshot_id,
        )?;
        Ok(Self::new(metadata))
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

    /// Finds a unit by its current source coordinate.
    #[must_use]
    pub fn unit_by_source_binding(&self, binding: &SourceBinding) -> Option<&TranslationUnit> {
        self.source_bindings
            .get(binding)
            .and_then(|id| self.units.get(id))
    }

    /// Creates a draft unit from one String cell in an already verified HXS
    /// snapshot.
    ///
    /// The source macro text is used only by the HXS reader to establish that
    /// the requested cell is present; only verified hashes enter the unit.
    /// `target_macro` may be empty, which represents an explicit empty target
    /// on a unit that is present in this sparse workspace.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshot is incompatible with this workspace,
    /// the requested cell is absent, the target is unsafe, or the identity or
    /// current binding is already present.
    pub fn create_unit_from_hxs(
        &mut self,
        snapshot: &HxsSnapshot,
        sheet_name: &str,
        row_id: u32,
        subrow_id: u16,
        column_index: u32,
        target_macro: &str,
    ) -> Result<TranslationUnitId, WorkspaceError> {
        let binding = SourceBinding::new(sheet_name, row_id, subrow_id, column_index);
        self.create_unit_from_verified_source(snapshot, binding, target_macro)
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

    fn create_unit_from_verified_source(
        &mut self,
        snapshot: &HxsSnapshot,
        binding: SourceBinding,
        target_macro: &str,
    ) -> Result<TranslationUnitId, WorkspaceError> {
        self.require_compatible_snapshot(snapshot)?;
        let fingerprint = verified_fingerprint(snapshot, &binding)?;
        validate_target(target_macro)?;
        let id =
            TranslationUnitId::derive(self.metadata.source_language(), &binding, &fingerprint)?;

        let unit = TranslationUnit::new(id, binding, fingerprint, target_macro);
        self.insert_unit(unit)?;
        Ok(id)
    }

    fn insert_unit(&mut self, unit: TranslationUnit) -> Result<(), WorkspaceError> {
        let id = unit.id();
        let binding = unit.source_binding().clone();
        if self.units.contains_key(&id) {
            return Err(WorkspaceError::DuplicateUnitId { id });
        }
        if self.source_bindings.contains_key(&binding) {
            return Err(WorkspaceError::DuplicateSourceBinding { binding });
        }

        self.units.insert(id, unit);
        self.source_bindings.insert(binding, id);
        Ok(())
    }

    fn from_loaded(
        metadata: WorkspaceMetadata,
        units: BTreeMap<TranslationUnitId, TranslationUnit>,
    ) -> Result<Self, WorkspaceError> {
        let mut source_bindings = BTreeMap::new();
        for (id, unit) in &units {
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

        let mut lower_bytes = [0_u8; 32];
        lower_bytes[0] = shard;
        let lower = TranslationUnitId::from_bytes(lower_bytes);
        let upper = if shard == u8::MAX {
            Unbounded
        } else {
            let mut upper_bytes = [0_u8; 32];
            upper_bytes[0] = shard + 1;
            Excluded(TranslationUnitId::from_bytes(upper_bytes))
        };
        self.units
            .range((Included(lower), upper))
            .map(|(_, unit)| unit)
    }

    fn unit_mut(&mut self, id: TranslationUnitId) -> Result<&mut TranslationUnit, WorkspaceError> {
        self.units
            .get_mut(&id)
            .ok_or(WorkspaceError::UnitNotFound { id })
    }

    fn require_compatible_snapshot(&self, snapshot: &HxsSnapshot) -> Result<(), WorkspaceError> {
        let source = snapshot.metadata();
        if source.source_language != self.metadata.source_language() {
            return Err(WorkspaceError::SourceLanguageMismatch {
                expected: self.metadata.source_language().to_owned(),
                found: source.source_language,
            });
        }
        if source.content_id != self.metadata.source_content_id() {
            return Err(WorkspaceError::SourceContentMismatch {
                expected: self.metadata.source_content_id().to_owned(),
                found: source.content_id,
            });
        }
        if source.snapshot_id != self.metadata.source_snapshot_id() {
            return Err(WorkspaceError::SourceSnapshotMismatch {
                expected: self.metadata.source_snapshot_id().to_owned(),
                found: source.snapshot_id,
            });
        }
        Ok(())
    }
}

fn verified_fingerprint(
    snapshot: &HxsSnapshot,
    binding: &SourceBinding,
) -> Result<SourceFingerprint, WorkspaceError> {
    let cell = snapshot
        .string_cell(
            binding.sheet_name(),
            binding.row_id(),
            binding.subrow_id(),
            binding.column_index(),
        )?
        .ok_or_else(|| WorkspaceError::SourceCellNotFound {
            binding: binding.clone(),
        })?;
    let row = snapshot
        .row(binding.sheet_name(), binding.row_id(), binding.subrow_id())?
        .ok_or_else(|| WorkspaceError::SourceCellNotFound {
            binding: binding.clone(),
        })?;
    Ok(SourceFingerprint::new(
        Sha256Hash::from_bytes(*cell.hashes.macro_text.as_bytes()),
        cell.hashes
            .raw_value
            .map(|hash| Sha256Hash::from_bytes(*hash.as_bytes())),
        Sha256Hash::from_bytes(*row.hashes.technical.as_bytes()),
    ))
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

    fn test_unit(sheet_name: &str, row_id: u32, macro_byte: u8) -> TranslationUnit {
        let binding = SourceBinding::new(sheet_name, row_id, 0, 0);
        let fingerprint = SourceFingerprint::new(
            Sha256Hash::from_bytes([macro_byte; 32]),
            None,
            Sha256Hash::from_bytes([0x22; 32]),
        );
        let id = TranslationUnitId::derive("en", &binding, &fingerprint)
            .expect("test identity inputs fit canonical framing");
        TranslationUnit::new(id, binding, fingerprint, "target")
    }

    fn test_metadata() -> WorkspaceMetadata {
        WorkspaceMetadata::new("en", "fr", "content", "snapshot").expect("test metadata is valid")
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
        let first = test_unit("Synthetic", 42, 0x11);
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

        let duplicate_binding = test_unit("Synthetic", 42, 0x33);
        assert_ne!(duplicate_binding.id(), first_id);
        assert!(matches!(
            workspace.insert_unit(duplicate_binding),
            Err(WorkspaceError::DuplicateSourceBinding { binding: found }) if found == binding
        ));
        assert_index_consistent(&workspace);

        let second = test_unit("Synthetic", 7, 0x44);
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
