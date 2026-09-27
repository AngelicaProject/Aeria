use std::collections::BTreeMap;
use std::sync::Arc;

use aeria_core::{ReviewState, SourceBinding, TranslationUnit};
use aeria_se::{SemanticValidity, parse};
use aeria_source::{GameSource, SheetLookup, SheetVariant as SourceVariant, SourceSheet};
use aeria_workspace::Workspace;

use crate::error::ExportError;
use crate::manifest::{ContentPolicy, PackSource};
use crate::writer::{CellState, LayoutColumn, PackCell, PackSheet, SheetVariant, source_guard};

const ENCODE_BATCH: usize = 4096;

/// Turns validated target macro text into the exact `SeString` bytes the game
/// reads. Production uses [`SeStringEncoder`].
pub trait StringEncoder {
    /// Encodes `macros` in order. Per-string failures are returned in place;
    /// `Err` means the encoder itself failed.
    ///
    /// # Errors
    /// Returns a description of an encoder failure (process, protocol, I/O).
    fn encode(&mut self, macros: &[&str]) -> Result<Vec<Result<Vec<u8>, String>>, String>;
}

/// Encodes with `aeria_se::codec::encode_checked`, the inverse of the
/// decoder that prints source text.
#[derive(Clone, Copy, Debug, Default)]
pub struct SeStringEncoder;

impl StringEncoder for SeStringEncoder {
    fn encode(&mut self, macros: &[&str]) -> Result<Vec<Result<Vec<u8>, String>>, String> {
        Ok(macros
            .iter()
            .map(|text| aeria_se::codec::encode_checked(text).map_err(|error| error.to_string()))
            .collect())
    }
}

/// What the export left out and why. Nothing here is an error; the report is
/// shown to the person exporting.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExportReport {
    pub exported: u64,
    pub skipped_detached: u64,
    pub skipped_untranslated: u64,
    pub skipped_unreviewed: u64,
}

#[derive(Debug)]
pub struct ProjectExport {
    pub sheets: Vec<PackSheet>,
    pub report: ExportReport,
}

/// Manifest source facts of the game.
#[must_use]
pub fn pack_source(source: &GameSource) -> PackSource {
    PackSource {
        language: source.language().code().to_owned(),
        game_version: source.version().as_str().to_owned(),
    }
}

/// Collects the translations of a project for a pack.
///
/// `source` must be the game the workspace describes. Every exported unit is
/// checked against the game again, and its source guard is computed from
/// the game's current bytes. Every selected target is validated again and
/// encoded; a target that fails either step fails the export, because the
/// workspace must never hold one.
///
/// # Errors
/// Returns [`ExportError`] for an invalid target, a failed encoding, a bound
/// unit that does not describe the game, a game read error, or an encoder
/// failure.
///
/// # Panics
/// Never: a sheet is looked up only after it was inserted.
pub fn collect_project(
    workspace: &Workspace,
    source: &GameSource,
    policy: ContentPolicy,
    encoder: &mut dyn StringEncoder,
) -> Result<ProjectExport, ExportError> {
    let mut report = ExportReport::default();
    let mut selected: Vec<(&TranslationUnit, CellState, [u8; 8])> = Vec::new();
    let mut game_sheets: BTreeMap<&str, Arc<SourceSheet>> = BTreeMap::new();

    for unit in workspace.units() {
        if !unit.is_bound() {
            report.skipped_detached += 1;
            continue;
        }
        if unit.target_macro().is_empty() {
            report.skipped_untranslated += 1;
            continue;
        }
        let state = match (unit.review_state(), policy) {
            (ReviewState::Reviewed, _) => CellState::Reviewed,
            (_, ContentPolicy::All) => CellState::Unreviewed,
            (_, ContentPolicy::Reviewed) => {
                report.skipped_unreviewed += 1;
                continue;
            }
        };
        let validation = parse(unit.target_macro()).semantic_validation();
        if !matches!(
            validation.status(),
            SemanticValidity::ValidAndUnderstood | SemanticValidity::ValidWithOpaque
        ) {
            return Err(cell_error(
                unit.source_binding(),
                "target is not a valid macro string",
            ));
        }
        let binding = unit.source_binding();
        if !game_sheets.contains_key(binding.sheet_name()) {
            let SheetLookup::Present(sheet) = source.sheet(binding.sheet_name())? else {
                return Err(cell_error(
                    binding,
                    "the sheet cannot be read from the game",
                ));
            };
            game_sheets.insert(binding.sheet_name(), sheet);
        }
        let sheet = &game_sheets[binding.sheet_name()];
        let facts = sheet.facts(
            binding.row_id(),
            binding.subrow_id(),
            binding.column_index(),
        );
        let cell = sheet.cell(
            binding.row_id(),
            binding.subrow_id(),
            binding.column_index(),
        );
        let (Some(facts), Some(cell)) = (facts, cell) else {
            return Err(cell_error(binding, "the cell is not in the game"));
        };
        if &facts != unit.source() {
            return Err(cell_error(
                binding,
                "the translation does not describe the game; open the project again",
            ));
        }
        selected.push((unit, state, source_guard(cell.bytes)));
    }

    let texts = encode_all(&selected, encoder)?;
    let mut sheets: BTreeMap<&str, PackSheet> = BTreeMap::new();
    for ((unit, state, guard), text) in selected.iter().zip(texts) {
        let binding = unit.source_binding();
        if !sheets.contains_key(binding.sheet_name()) {
            sheets.insert(
                binding.sheet_name(),
                pack_sheet(&game_sheets[binding.sheet_name()]),
            );
        }
        let sheet = sheets
            .get_mut(binding.sheet_name())
            .expect("inserted above");
        if !sheet
            .layout
            .iter()
            .any(|column| column.column_index == binding.column_index())
        {
            return Err(cell_error(
                binding,
                "column is not a String column of the game",
            ));
        }
        sheet.cells.push(PackCell {
            row_id: binding.row_id(),
            subrow_id: binding.subrow_id(),
            column_index: binding.column_index(),
            state: *state,
            text,
            source_guard: *guard,
        });
        report.exported += 1;
    }

    Ok(ProjectExport {
        sheets: sheets.into_values().collect(),
        report,
    })
}

fn encode_all(
    selected: &[(&TranslationUnit, CellState, [u8; 8])],
    encoder: &mut dyn StringEncoder,
) -> Result<Vec<Vec<u8>>, ExportError> {
    let mut output = Vec::with_capacity(selected.len());
    for batch in selected.chunks(ENCODE_BATCH) {
        let macros: Vec<&str> = batch
            .iter()
            .map(|(unit, _, _)| unit.target_macro())
            .collect();
        let results = encoder.encode(&macros).map_err(ExportError::Encoder)?;
        if results.len() != batch.len() {
            return Err(ExportError::Encoder(format!(
                "encoder returned {} results for {} strings",
                results.len(),
                batch.len()
            )));
        }
        for ((unit, _, _), result) in batch.iter().zip(results) {
            output.push(result.map_err(|reason| {
                cell_error(unit.source_binding(), &format!("encoding failed: {reason}"))
            })?);
        }
    }
    Ok(output)
}

fn pack_sheet(sheet: &SourceSheet) -> PackSheet {
    PackSheet {
        name: sheet.name().to_owned(),
        variant: match sheet.variant() {
            SourceVariant::Default => SheetVariant::DefaultRows,
            SourceVariant::Subrows => SheetVariant::Subrows,
        },
        layout: sheet
            .columns()
            .iter()
            .map(|column| LayoutColumn {
                column_index: column.index,
                offset: u32::from(column.offset),
            })
            .collect(),
        cells: Vec::new(),
    }
}

fn cell_error(binding: &SourceBinding, reason: &str) -> ExportError {
    ExportError::Cell {
        sheet: binding.sheet_name().to_owned(),
        row_id: binding.row_id(),
        subrow_id: binding.subrow_id(),
        column_index: binding.column_index(),
        reason: reason.to_owned(),
    }
}
