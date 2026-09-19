//! Bounded, read-only application composition for source occurrences.

use aeria_core::{ReviewState, SourceBinding, SourceFingerprint, TranslationUnitId};
use aeria_hxs::{HxsError, StringOccurrenceCoordinate};
use thiserror::Error;

use crate::{ProjectSession, source_fingerprint_from_hashes};

/// Conservative application-level maximum for one translation read page.
pub const MAX_TRANSLATION_PAGE_SIZE: u32 = 256;

/// One browsable verified source String occurrence and its optional sparse
/// Workspace overlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationEntryView {
    pub source_binding: SourceBinding,
    pub source_macro: String,
    pub translation: Option<TranslationOverlayView>,
}

/// The Workspace state overlaid on one verified source occurrence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationOverlayView {
    pub translation_unit_id: TranslationUnitId,
    pub target_macro: String,
    pub review_state: ReviewState,
    pub translator_note: Option<String>,
}

/// One bounded page of translation entries from one HXS sheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationEntryPage {
    pub entries: Vec<TranslationEntryView>,
    pub next_after: Option<SourceBinding>,
}

/// Errors raised while composing a bounded translation read page.
#[derive(Debug, Error)]
pub enum TranslationReadError {
    /// The application page size is outside its bounded contract.
    #[error("translation page limit {limit} must be between 1 and {max}")]
    InvalidPageLimit { limit: u32, max: u32 },

    /// A cursor from a different sheet cannot be interpreted in this page.
    #[error(
        "translation cursor belongs to sheet {cursor_sheet:?}, not requested sheet {requested_sheet:?}"
    )]
    CursorSheetMismatch {
        requested_sheet: String,
        cursor_sheet: String,
    },

    /// The verified source page could not be read.
    #[error("verified HXS source error: {0}")]
    Source(#[from] HxsError),

    /// A sparse unit does not describe the exact verified source occurrence.
    #[error(
        "translation unit {translation_unit_id} at {source_binding:?} has a stale source fingerprint: persisted {expected:?}, verified {verified:?}"
    )]
    WorkspaceSourceMismatch {
        translation_unit_id: TranslationUnitId,
        source_binding: SourceBinding,
        expected: Box<SourceFingerprint>,
        verified: Box<SourceFingerprint>,
    },
}

impl ProjectSession {
    /// Reads one bounded page of verified source String occurrences from one
    /// sheet and composes the sparse Workspace overlay.
    ///
    /// The cursor is exclusive and ordered by row ID, subrow ID, then column
    /// index. A missing Workspace unit remains untranslated and has no
    /// `TranslationUnitId`; a present unit is returned even when its target
    /// macro is explicitly empty.
    ///
    /// # Errors
    ///
    /// Returns a typed request error for invalid limits or a cross-sheet
    /// cursor, a source error for HXS failures, or an integrity error when an
    /// existing Workspace unit has stale source facts.
    pub fn page_translation_entries(
        &self,
        sheet_name: &str,
        after: Option<&SourceBinding>,
        limit: u32,
    ) -> Result<TranslationEntryPage, TranslationReadError> {
        if !(1..=MAX_TRANSLATION_PAGE_SIZE).contains(&limit) {
            return Err(TranslationReadError::InvalidPageLimit {
                limit,
                max: MAX_TRANSLATION_PAGE_SIZE,
            });
        }
        if let Some(after) = after
            && after.sheet_name() != sheet_name
        {
            return Err(TranslationReadError::CursorSheetMismatch {
                requested_sheet: sheet_name.to_owned(),
                cursor_sheet: after.sheet_name().to_owned(),
            });
        }

        let after_coordinate = after.map(|binding| {
            StringOccurrenceCoordinate::new(
                binding.sheet_name(),
                binding.row_id(),
                binding.subrow_id(),
                binding.column_index(),
            )
        });
        let source_page = self.source().page_string_occurrence_records(
            sheet_name,
            after_coordinate.as_ref(),
            limit,
        )?;

        let mut entries = Vec::with_capacity(source_page.occurrences.len());
        for occurrence in source_page.occurrences {
            let coordinate = &occurrence.fingerprint.coordinate;
            let source_binding = SourceBinding::new(
                coordinate.sheet_name.clone(),
                coordinate.row_id,
                coordinate.subrow_id,
                coordinate.column_index,
            );
            let verified_fingerprint = source_fingerprint_from_hashes(
                &occurrence.fingerprint.macro_text_hash,
                occurrence.fingerprint.raw_value_hash.as_ref(),
                &occurrence.fingerprint.row_technical_hash,
            );
            let translation = self
                .workspace()
                .unit_by_source_binding(&source_binding)
                .map(|unit| {
                    if unit.source_fingerprint() != &verified_fingerprint {
                        return Err(TranslationReadError::WorkspaceSourceMismatch {
                            translation_unit_id: unit.id(),
                            source_binding: source_binding.clone(),
                            expected: Box::new(*unit.source_fingerprint()),
                            verified: Box::new(verified_fingerprint),
                        });
                    }
                    Ok(TranslationOverlayView {
                        translation_unit_id: unit.id(),
                        target_macro: unit.target_macro().to_owned(),
                        review_state: unit.review_state(),
                        translator_note: unit.translator_note().map(str::to_owned),
                    })
                })
                .transpose()?;

            entries.push(TranslationEntryView {
                source_binding,
                source_macro: occurrence.macro_text,
                translation,
            });
        }

        Ok(TranslationEntryPage {
            entries,
            next_after: source_page.next_after.map(|coordinate| {
                SourceBinding::new(
                    coordinate.sheet_name,
                    coordinate.row_id,
                    coordinate.subrow_id,
                    coordinate.column_index,
                )
            }),
        })
    }
}
