//! Bounded, read-only application composition for source rows.

use aeria_core::{ReviewState, SourceBinding, SourceFacts, TranslationUnitId};
use aeria_se::parse;
use aeria_source::{SheetLookup, SourceError};
use std::collections::BTreeMap;
use thiserror::Error;

use crate::ProjectSession;

/// Conservative application-level maximum for one translation read page.
///
/// The limit counts physical row/subrow groups scanned. A page can contain
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
    /// The source text has no letters outside protected structure, such as
    /// punctuation, digits, or number formatting. Such a cell is still
    /// translatable; it is marked so the editor can show it as formatting.
    pub formatting_only: bool,
    pub translation: Option<TranslationOverlayView>,
}

/// One logical editor row composed from one physical row/subrow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationRowView {
    pub sheet_name: String,
    pub row_id: u32,
    pub subrow_id: u16,
    pub context: Vec<TranslationContextCellView>,
    pub cells: Vec<TranslationCellView>,
}

/// The Workspace state overlaid on one source String cell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationOverlayView {
    pub translation_unit_id: TranslationUnitId,
    pub target_macro: String,
    pub review_state: ReviewState,
    pub translator_note: Option<String>,
}

/// One bounded page of logical translation rows from one sheet.
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

    /// The game could not be read.
    #[error("game source error: {0}")]
    Source(#[from] SourceError),

    /// A sparse unit does not describe the game's cell.
    #[error(
        "translation unit {translation_unit_id} at {source_binding:?} has stale source facts: persisted {expected:?}, game {found:?}"
    )]
    WorkspaceSourceMismatch {
        translation_unit_id: TranslationUnitId,
        source_binding: SourceBinding,
        expected: Box<SourceFacts>,
        found: Box<Option<SourceFacts>>,
    },
}

impl ProjectSession {
    /// Reads one bounded page of logical source rows and composes the sparse
    /// Workspace overlay for each translatable String cell.
    ///
    /// The cursor is exclusive and ordered by row ID then subrow ID. The page
    /// bound counts source rows, so rows without translatable cells may make
    /// the visible result shorter without preventing the cursor from
    /// advancing. A sheet that is missing or cannot be read has no rows.
    ///
    /// A translatable cell is editable. A non-empty cell that is not
    /// translatable is returned as read-only context; empty ones are omitted.
    ///
    /// # Errors
    ///
    /// Returns a typed request error for invalid limits or a cross-sheet
    /// cursor, a source error when the game cannot be read, or an integrity
    /// error when an existing unit has stale source facts.
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
        let SheetLookup::Present(sheet) = self.source.sheet(sheet_name)? else {
            return Ok(TranslationRowPage {
                rows: Vec::new(),
                next_after: None,
            });
        };

        let source_rows =
            sheet.rows_after(after.map(|cursor| (cursor.row_id(), cursor.subrow_id())));
        let page_length = source_rows.len().min(limit as usize);
        let mut rows = Vec::with_capacity(page_length);
        for source_row in &source_rows[..page_length] {
            let mut context = Vec::new();
            let mut cells = Vec::new();
            for cell in sheet.cells(source_row) {
                let source_macro = cell.text();
                if !cell.translatable {
                    if !source_macro.is_empty() {
                        context.push(TranslationContextCellView {
                            column_index: cell.column,
                            source_macro,
                        });
                    }
                    continue;
                }
                let source_binding = SourceBinding::new(
                    sheet_name,
                    source_row.row_id,
                    source_row.subrow_id,
                    cell.column,
                );
                let translation = self.overlay_for_binding(&source_binding, || {
                    sheet.facts(source_row.row_id, source_row.subrow_id, cell.column)
                })?;
                let formatting_only = parse(&source_macro).is_formatting_only();
                cells.push(TranslationCellView {
                    source_binding,
                    source_macro,
                    formatting_only,
                    translation,
                });
            }
            if cells.is_empty() {
                continue;
            }
            rows.push(TranslationRowView {
                sheet_name: sheet_name.to_owned(),
                row_id: source_row.row_id,
                subrow_id: source_row.subrow_id,
                context,
                cells,
            });
        }

        let next_after = (page_length < source_rows.len()).then(|| {
            let last = &source_rows[page_length - 1];
            TranslationRowCursor::new(sheet_name, last.row_id, last.subrow_id)
        });
        Ok(TranslationRowPage { rows, next_after })
    }

    fn overlay_for_binding(
        &self,
        source_binding: &SourceBinding,
        facts: impl FnOnce() -> Option<SourceFacts>,
    ) -> Result<Option<TranslationOverlayView>, TranslationReadError> {
        let Some(unit) = self.workspace().unit_by_source_binding(source_binding) else {
            return Ok(None);
        };
        let found = facts();
        if found.as_ref() != Some(unit.source()) {
            return Err(TranslationReadError::WorkspaceSourceMismatch {
                translation_unit_id: unit.id(),
                source_binding: source_binding.clone(),
                expected: Box::new(unit.source().clone()),
                found: Box::new(found),
            });
        }
        Ok(Some(TranslationOverlayView {
            translation_unit_id: unit.id(),
            target_macro: unit.target_macro().to_owned(),
            review_state: unit.review_state(),
            translator_note: unit.translator_note().map(str::to_owned),
        }))
    }
}

/// Workspace coverage for one sheet.
///
/// `translated` counts Workspace units, including explicitly empty targets.
/// `reviewed` and `needs_review` are subsets of `translated`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SheetTranslationProgress {
    pub sheet_name: String,
    pub translated: usize,
    pub reviewed: usize,
    pub needs_review: usize,
}

impl ProjectSession {
    /// Summarizes bound Workspace units per sheet, ordered by sheet name.
    ///
    /// Detached units are not counted, and sheets without units are omitted.
    /// This reads in-memory Workspace state only; bound units were verified
    /// against the game when the project was opened.
    #[must_use]
    pub fn translation_progress(&self) -> Vec<SheetTranslationProgress> {
        let mut by_sheet: BTreeMap<&str, SheetTranslationProgress> = BTreeMap::new();
        for unit in self.workspace().units().filter(|unit| unit.is_bound()) {
            let binding = unit.source_binding();
            let progress =
                by_sheet
                    .entry(binding.sheet_name())
                    .or_insert_with(|| SheetTranslationProgress {
                        sheet_name: binding.sheet_name().to_owned(),
                        ..SheetTranslationProgress::default()
                    });
            progress.translated += 1;
            match unit.review_state() {
                ReviewState::Reviewed => progress.reviewed += 1,
                ReviewState::NeedsReview => progress.needs_review += 1,
                ReviewState::Draft => {}
            }
        }
        by_sheet.into_values().collect()
    }
}
