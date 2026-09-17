//! Sparse translation workspace operations over verified HXS source.
//!
//! This crate owns the source adapter and in-memory workspace operations. It
//! deliberately does not choose or implement the eventual on-disk workspace
//! serialization format.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use aeria_core::{
    DomainValueError, ReviewState, Sha256Hash, SourceBinding, SourceFingerprint, TranslationUnit,
    TranslationUnitId, TranslationUnitIdError, WorkspaceMetadata,
};
use aeria_hxs::{HxsError, HxsSnapshot};
use aeria_se::{Diagnostic, SemanticValidity, parse};
use thiserror::Error;

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
}

impl Workspace {
    /// Creates an empty workspace with one validated source/target binding.
    #[must_use]
    pub fn new(metadata: WorkspaceMetadata) -> Self {
        Self {
            metadata,
            units: BTreeMap::new(),
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
        self.units
            .values()
            .find(|unit| unit.source_binding() == binding)
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

    /// Marks a known meaningful source change and retains the unit's durable
    /// identity while replacing its current binding and fingerprint.
    ///
    /// # Errors
    ///
    /// Returns an error when the unit does not exist or the new binding is
    /// already owned by another unit.
    pub fn mark_source_changed(
        &mut self,
        id: TranslationUnitId,
        source_binding: SourceBinding,
        source_fingerprint: SourceFingerprint,
    ) -> Result<(), WorkspaceError> {
        if !self.units.contains_key(&id) {
            return Err(WorkspaceError::UnitNotFound { id });
        }
        if self
            .unit_by_source_binding(&source_binding)
            .is_some_and(|unit| unit.id() != id)
        {
            return Err(WorkspaceError::DuplicateSourceBinding {
                binding: source_binding,
            });
        }
        self.unit_mut(id)?
            .update_source_after_known_change(source_binding, source_fingerprint);
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
            TranslationUnitId::try_derive(self.metadata.source_language(), &binding, &fingerprint)?;
        if self.units.contains_key(&id) {
            return Err(WorkspaceError::DuplicateUnitId { id });
        }
        if self.unit_by_source_binding(&binding).is_some() {
            return Err(WorkspaceError::DuplicateSourceBinding { binding });
        }

        let unit = TranslationUnit::new(id, binding, fingerprint, target_macro);
        self.units.insert(id, unit);
        Ok(id)
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
