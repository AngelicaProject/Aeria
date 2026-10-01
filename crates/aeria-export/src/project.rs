use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use aeria_po::{Identity, PO_DIR, PoFile, RowName, identity_keys, is_entry, list};
use aeria_se::{SemanticValidity, parse};
use aeria_source::{GameSource, SheetLookup, SheetVariant as SourceVariant, SourceSheet};

use crate::error::ExportError;
use crate::manifest::PackGame;
use crate::writer::{LayoutColumn, PackCell, PackSheet, SheetVariant, source_guard};

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
    pub skipped_untranslated: u64,
    /// Translations marked fuzzy: their source changed since they were
    /// written.
    pub skipped_fuzzy: u64,
}

#[derive(Debug)]
pub struct ProjectExport {
    pub sheets: Vec<PackSheet>,
    pub report: ExportReport,
}

/// The manifest's facts of the game.
#[must_use]
pub fn pack_game(source: &GameSource) -> PackGame {
    PackGame {
        language: source.language().code().to_owned(),
        version: source.version().as_str().to_owned(),
    }
}

/// One translation chosen for the pack.
struct Selected {
    sheet: String,
    row: u32,
    subrow: u16,
    column: u32,
    text: String,
    guard: [u8; 8],
}

/// A sheet of the game with its rows by key, read once.
struct GameSheet {
    sheet: Arc<SourceSheet>,
    keys: HashMap<String, (u32, u16)>,
}

fn game_sheet<'a>(
    sheets: &'a mut BTreeMap<String, GameSheet>,
    source: &GameSource,
    name: &str,
) -> Result<Option<&'a GameSheet>, ExportError> {
    if !sheets.contains_key(name) {
        let SheetLookup::Present(sheet) = source.sheet(name)? else {
            return Ok(None);
        };
        let keys = identity_keys(&sheet)
            .map(|keys| {
                sheet
                    .rows()
                    .iter()
                    .filter_map(|row| {
                        keys.key_of(row.row_id, row.subrow_id)
                            .map(|key| (key.to_owned(), (row.row_id, row.subrow_id)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        sheets.insert(name.to_owned(), GameSheet { sheet, keys });
    }
    Ok(sheets.get(name))
}

/// Finds a translated entry of `po/{path}` in the game and checks it.
fn select(
    path: &str,
    entry: aeria_po::Entry,
    game_sheets: &mut BTreeMap<String, GameSheet>,
    source: &GameSource,
) -> Result<Selected, ExportError> {
    let identity = Identity::parse(&entry.context).map_err(|message| {
        ExportError::Project(format!("po/{path} line {}: {message}", entry.line))
    })?;
    let entry_error = |reason: &str| {
        ExportError::Project(format!(
            "po/{path} line {} ({}): {reason}",
            entry.line, entry.context
        ))
    };
    let validation = parse(&entry.translation).semantic_validation();
    if !matches!(
        validation.status(),
        SemanticValidity::ValidAndUnderstood | SemanticValidity::ValidWithOpaque
    ) {
        return Err(entry_error("the translation is not a valid macro string"));
    }
    let Some(game) = game_sheet(game_sheets, source, &identity.sheet)? else {
        return Err(entry_error("the sheet cannot be read from the game"));
    };
    let (row, subrow) = match &identity.row {
        RowName::Id { row, subrow } => (*row, *subrow),
        RowName::Key(key) => *game
            .keys
            .get(key)
            .ok_or_else(|| entry_error("the game has no row with this key"))?,
    };
    let keys = identity_keys(&game.sheet);
    let cell = game
        .sheet
        .cell(row, subrow, identity.column)
        .filter(|cell| is_entry(cell, keys))
        .ok_or_else(|| entry_error("the string is not in the game"))?;
    if cell.text() != entry.source {
        return Err(entry_error(
            "the msgid is not the game's text; update the project to the game",
        ));
    }
    Ok(Selected {
        sheet: identity.sheet,
        row,
        subrow,
        column: identity.column,
        guard: source_guard(cell.bytes),
        text: entry.translation,
    })
}

/// Collects the translations of the project at `root` for a pack: every
/// translated entry of `po/` that is not fuzzy.
///
/// `source` must be the game the project's files are for. Every entry is
/// found in the game again and its `msgid` compared with the game's text;
/// its source guard is computed from the game's current bytes. Every
/// translation is validated again and encoded; one that fails either step
/// fails the export, because the project must never hold one.
///
/// # Errors
/// Returns [`ExportError`] for an unreadable or broken file, an invalid
/// translation, a failed encoding, an entry that does not describe the game,
/// a game read error, or an encoder failure.
///
/// # Panics
/// Never: a sheet is looked up only after it was inserted.
pub fn collect_project(
    root: &Path,
    source: &GameSource,
    encoder: &mut dyn StringEncoder,
) -> Result<ProjectExport, ExportError> {
    let mut report = ExportReport::default();
    let mut selected: Vec<Selected> = Vec::new();
    let mut game_sheets: BTreeMap<String, GameSheet> = BTreeMap::new();
    let paths = list(root).map_err(|error| ExportError::Project(error.to_string()))?;
    for path in paths {
        let full = root.join(PO_DIR).join(&path);
        let text = std::fs::read_to_string(&full)
            .map_err(|error| ExportError::Project(format!("po/{path}: {error}")))?;
        let (file, problems) = PoFile::parse(&text);
        if let Some(problem) = problems.first() {
            return Err(ExportError::Project(format!(
                "po/{path} line {}: {}",
                problem.line, problem.message
            )));
        }
        for entry in file.entries {
            if entry.translation.is_empty() {
                report.skipped_untranslated += 1;
                continue;
            }
            if entry.fuzzy {
                report.skipped_fuzzy += 1;
                continue;
            }
            selected.push(select(&path, entry, &mut game_sheets, source)?);
        }
    }

    let texts = encode_all(&selected, encoder)?;
    let mut sheets: BTreeMap<String, PackSheet> = BTreeMap::new();
    for (chosen, text) in selected.iter().zip(texts) {
        let sheet = sheets
            .entry(chosen.sheet.clone())
            .or_insert_with(|| pack_sheet(&game_sheets[&chosen.sheet].sheet));
        if !sheet
            .layout
            .iter()
            .any(|column| column.column_index == chosen.column)
        {
            return Err(cell_error(
                chosen,
                "column is not a String column of the game",
            ));
        }
        sheet.cells.push(PackCell {
            row_id: chosen.row,
            subrow_id: chosen.subrow,
            column_index: chosen.column,
            text,
            source_guard: chosen.guard,
        });
        report.exported += 1;
    }

    Ok(ProjectExport {
        sheets: sheets.into_values().collect(),
        report,
    })
}

fn encode_all(
    selected: &[Selected],
    encoder: &mut dyn StringEncoder,
) -> Result<Vec<Vec<u8>>, ExportError> {
    let mut output = Vec::with_capacity(selected.len());
    for batch in selected.chunks(ENCODE_BATCH) {
        let macros: Vec<&str> = batch.iter().map(|chosen| chosen.text.as_str()).collect();
        let results = encoder.encode(&macros).map_err(ExportError::Encoder)?;
        if results.len() != batch.len() {
            return Err(ExportError::Encoder(format!(
                "encoder returned {} results for {} strings",
                results.len(),
                batch.len()
            )));
        }
        for (chosen, result) in batch.iter().zip(results) {
            output.push(
                result
                    .map_err(|reason| cell_error(chosen, &format!("encoding failed: {reason}")))?,
            );
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

fn cell_error(chosen: &Selected, reason: &str) -> ExportError {
    ExportError::Cell {
        sheet: chosen.sheet.clone(),
        row_id: chosen.row,
        subrow_id: chosen.subrow,
        column_index: chosen.column,
        reason: reason.to_owned(),
    }
}
