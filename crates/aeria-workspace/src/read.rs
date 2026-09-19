//! Bounded, read-only application composition for source rows.

use aeria_core::{ReviewState, SourceBinding, SourceFingerprint, TranslationUnitId};
use aeria_hxs::{HxsError, StringRowCoordinate};
use thiserror::Error;

use crate::{ProjectSession, source_fingerprint_from_hashes};

/// Conservative application-level maximum for one translation read page.
///
/// The limit counts physical HXS row/subrow groups scanned. A page can contain
/// fewer visible rows because context-only and empty-only source rows are
/// deliberately filtered from the editor projection.
pub const MAX_TRANSLATION_PAGE_SIZE: u32 = 256;

/// A narrow application cursor for one physical source row.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TranslationRowCursor {
    sheet_name: String,
    row_id: u32,
    subrow_id: u16,
}

impl TranslationRowCursor {
    /// Creates a row cursor.
    #[must_use]
    pub fn new(sheet_name: impl Into<String>, row_id: u32, subrow_id: u16) -> Self {
        Self {
            sheet_name: sheet_name.into(),
            row_id,
            subrow_id,
        }
    }

    /// Returns the sheet name owned by this cursor.
    #[must_use]
    pub fn sheet_name(&self) -> &str {
        &self.sheet_name
    }

    /// Returns the physical row ID.
    #[must_use]
    pub const fn row_id(&self) -> u32 {
        self.row_id
    }

    /// Returns the physical subrow ID.
    #[must_use]
    pub const fn subrow_id(&self) -> u16 {
        self.subrow_id
    }
}

/// One read-only context String cell in a logical translation row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationContextCellView {
    pub column_index: u32,
    pub source_macro: String,
}

/// One translatable String cell and its optional sparse Workspace overlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationCellView {
    pub source_binding: SourceBinding,
    pub source_macro: String,
    pub translation: Option<TranslationOverlayView>,
}

/// One logical editor row composed from one HXS physical row/subrow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationRowView {
    pub sheet_name: String,
    pub row_id: u32,
    pub subrow_id: u16,
    pub context: Vec<TranslationContextCellView>,
    pub cells: Vec<TranslationCellView>,
}

/// The Workspace state overlaid on one verified source String cell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationOverlayView {
    pub translation_unit_id: TranslationUnitId,
    pub target_macro: String,
    pub review_state: ReviewState,
    pub translator_note: Option<String>,
}

/// One bounded page of logical translation rows from one HXS sheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationRowPage {
    pub rows: Vec<TranslationRowView>,
    pub next_after: Option<TranslationRowCursor>,
}

/// Errors raised while composing a bounded translation row page.
#[derive(Debug, Error)]
pub enum TranslationReadError {
    /// The application page size is outside its bounded contract.
    #[error("translation page limit {limit} must be between 1 and {max}")]
    InvalidPageLimit { limit: u32, max: u32 },

    /// A cursor from a different sheet cannot be interpreted in this page.
    #[error(
        "translation row cursor belongs to sheet {cursor_sheet:?}, not requested sheet {requested_sheet:?}"
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
    /// Reads one bounded page of logical source rows and composes the sparse
    /// Workspace overlay for each translatable String cell.
    ///
    /// The cursor is exclusive and ordered by row ID then subrow ID. The HXS
    /// page bound counts source row groups, so context-only and empty-only
    /// groups may make the visible result shorter without preventing the
    /// source cursor from advancing.
    ///
    /// Empty macro text is ignored. A non-empty macro beginning with the exact
    /// uppercase prefix `TEXT_` is read-only context. Every other non-empty
    /// String cell is translatable. This deliberately narrow classification is
    /// not semantic schema metadata.
    ///
    /// # Errors
    ///
    /// Returns a typed request error for invalid limits or a cross-sheet
    /// cursor, a source error for HXS failures, or an integrity error when an
    /// existing Workspace unit has stale source facts.
    pub fn page_translation_rows(
        &self,
        sheet_name: &str,
        after: Option<&TranslationRowCursor>,
        limit: u32,
    ) -> Result<TranslationRowPage, TranslationReadError> {
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

        let after_coordinate = after.map(|cursor| {
            StringRowCoordinate::new(cursor.sheet_name(), cursor.row_id(), cursor.subrow_id())
        });
        let source_page =
            self.source()
                .page_string_rows(sheet_name, after_coordinate.as_ref(), limit)?;

        let mut rows = Vec::with_capacity(source_page.rows.len());
        for source_row in source_page.rows {
            let mut context = Vec::new();
            let mut cells = Vec::new();
            for occurrence in source_row.occurrences {
                let coordinate = &occurrence.fingerprint.coordinate;
                let column_index = coordinate.column_index;
                let source_macro = occurrence.macro_text;
                if source_macro.is_empty() {
                    continue;
                }
                if source_macro.starts_with("TEXT_") {
                    context.push(TranslationContextCellView {
                        column_index,
                        source_macro,
                    });
                    continue;
                }

                let source_binding = SourceBinding::new(
                    coordinate.sheet_name.clone(),
                    coordinate.row_id,
                    coordinate.subrow_id,
                    column_index,
                );
                let verified_fingerprint = source_fingerprint_from_hashes(
                    &occurrence.fingerprint.macro_text_hash,
                    occurrence.fingerprint.raw_value_hash.as_ref(),
                    &occurrence.fingerprint.row_technical_hash,
                );
                let translation =
                    self.overlay_for_binding(&source_binding, &verified_fingerprint)?;
                cells.push(TranslationCellView {
                    source_binding,
                    source_macro,
                    translation,
                });
            }

            if cells.is_empty() {
                continue;
            }
            rows.push(TranslationRowView {
                sheet_name: source_row.coordinate.sheet_name,
                row_id: source_row.coordinate.row_id,
                subrow_id: source_row.coordinate.subrow_id,
                context,
                cells,
            });
        }

        Ok(TranslationRowPage {
            rows,
            next_after: source_page.next_after.map(|coordinate| {
                TranslationRowCursor::new(
                    coordinate.sheet_name,
                    coordinate.row_id,
                    coordinate.subrow_id,
                )
            }),
        })
    }

    fn overlay_for_binding(
        &self,
        source_binding: &SourceBinding,
        verified_fingerprint: &SourceFingerprint,
    ) -> Result<Option<TranslationOverlayView>, TranslationReadError> {
        self.workspace()
            .unit_by_source_binding(source_binding)
            .map(|unit| {
                if unit.source_fingerprint() != verified_fingerprint {
                    return Err(TranslationReadError::WorkspaceSourceMismatch {
                        translation_unit_id: unit.id(),
                        source_binding: source_binding.clone(),
                        expected: Box::new(*unit.source_fingerprint()),
                        verified: Box::new(*verified_fingerprint),
                    });
                }
                Ok(TranslationOverlayView {
                    translation_unit_id: unit.id(),
                    target_macro: unit.target_macro().to_owned(),
                    review_state: unit.review_state(),
                    translator_note: unit.translator_note().map(str::to_owned),
                })
            })
            .transpose()
    }
}
