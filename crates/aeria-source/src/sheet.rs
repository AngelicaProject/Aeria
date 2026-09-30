//! One sheet read in the source language, with its layout, translation
//! permission, and row keys.

use std::collections::{BTreeMap, HashMap};

use aeria_core::LayoutHash;
use aeria_sqpack::GameData;
use aeria_sqpack::excel::{self, Column, ColumnKind, SheetError, Variant};
use sha2::{Digest, Sha256};

use crate::{SourceError, SourceLanguage};

/// The smallest number of rows for which a sheet can be keyed.
pub const MIN_KEYED_ROWS: usize = 2;

const LAYOUT_DOMAIN: &str = "aeria.sheet-layout.v1";

/// One String column: its index among all columns and its byte offset in the
/// row.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StringColumn {
    pub index: u32,
    pub offset: u16,
}

/// Hashes a layout: the String columns in column order.
#[must_use]
pub fn layout_hash(columns: &[StringColumn]) -> LayoutHash {
    let mut hasher = Sha256::new();
    hasher.update(LAYOUT_DOMAIN.as_bytes());
    for column in columns {
        hasher.update(column.index.to_le_bytes());
        hasher.update(column.offset.to_le_bytes());
    }
    let digest = hasher.finalize();
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&digest[..8]);
    LayoutHash::from_bytes(bytes)
}

/// One row or subrow and its String cells.
#[derive(Debug)]
pub struct SourceRow {
    pub row_id: u32,
    pub subrow_id: u16,
    /// Cell bytes in the sheet's String column order.
    strings: Box<[Box<[u8]>]>,
    translatable: Box<[bool]>,
}

/// One String cell of a row.
#[derive(Clone, Copy, Debug)]
pub struct SourceCell<'a> {
    /// The column index.
    pub column: u32,
    /// The string bytes, without the terminator.
    pub bytes: &'a [u8],
    /// Whether the cell may be translated.
    pub translatable: bool,
}

impl SourceCell<'_> {
    /// The cell's text as macro text.
    #[must_use]
    pub fn text(&self) -> String {
        aeria_se::codec::decode(self.bytes)
    }
}

/// One sheet read in the source language.
#[derive(Debug)]
pub struct SourceSheet {
    name: String,
    variant: Variant,
    columns: Vec<StringColumn>,
    layout: LayoutHash,
    rows: Vec<SourceRow>,
    row_keys: Option<RowKeys>,
}

/// Why a listed sheet cannot be read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Unavailable {
    pub message: String,
}

impl SourceSheet {
    /// The sheet name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the sheet stores one record or several subrows per row.
    #[must_use]
    pub const fn variant(&self) -> Variant {
        self.variant
    }

    /// The String columns in column order.
    #[must_use]
    pub fn columns(&self) -> &[StringColumn] {
        &self.columns
    }

    /// The layout hash.
    #[must_use]
    pub const fn layout(&self) -> LayoutHash {
        self.layout
    }

    /// Every row in `(row, subrow)` order.
    #[must_use]
    pub fn rows(&self) -> &[SourceRow] {
        &self.rows
    }

    /// The rows after `after`, exclusive, in `(row, subrow)` order.
    #[must_use]
    pub fn rows_after(&self, after: Option<(u32, u16)>) -> &[SourceRow] {
        let Some(after) = after else {
            return &self.rows;
        };
        let start = self
            .rows
            .partition_point(|row| (row.row_id, row.subrow_id) <= after);
        &self.rows[start..]
    }

    /// Finds one row or subrow.
    #[must_use]
    pub fn row(&self, row_id: u32, subrow_id: u16) -> Option<&SourceRow> {
        self.rows
            .binary_search_by_key(&(row_id, subrow_id), |row| (row.row_id, row.subrow_id))
            .ok()
            .map(|index| &self.rows[index])
    }

    /// Finds one String cell.
    #[must_use]
    pub fn cell(&self, row_id: u32, subrow_id: u16, column: u32) -> Option<SourceCell<'_>> {
        self.row(row_id, subrow_id)
            .and_then(|row| self.cell_of(row, column))
    }

    /// Finds the String cell of `row` in `column`.
    #[must_use]
    pub fn cell_of<'a>(&'a self, row: &'a SourceRow, column: u32) -> Option<SourceCell<'a>> {
        let position = self
            .columns
            .binary_search_by_key(&column, |string_column| string_column.index)
            .ok()?;
        Some(SourceCell {
            column,
            bytes: &row.strings[position],
            translatable: row.translatable[position],
        })
    }

    /// The String cells of `row` in column order.
    pub fn cells<'a>(&'a self, row: &'a SourceRow) -> impl Iterator<Item = SourceCell<'a>> {
        self.columns
            .iter()
            .zip(row.strings.iter().zip(row.translatable.iter()))
            .map(|(column, (bytes, translatable))| SourceCell {
                column: column.index,
                bytes,
                translatable: *translatable,
            })
    }

    /// The sheet's row keys, when it has a row key column.
    #[must_use]
    pub const fn row_keys(&self) -> Option<&RowKeys> {
        self.row_keys.as_ref()
    }

    /// Reads the String cells of this sheet in another client language, in
    /// this sheet's column order. `None` when that language cannot be read
    /// or has another variant or other String columns, so its cells cannot
    /// be matched to this sheet's.
    pub(crate) fn strings_in(
        &self,
        game: &GameData,
        language: SourceLanguage,
    ) -> Result<Option<StringRows>, SourceError> {
        let Ok(sheet) = read_excel(game, &self.name, language)? else {
            return Ok(None);
        };
        let same_columns = sheet
            .columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.kind == ColumnKind::String)
            .map(|(index, column)| (index, column.offset))
            .eq(self
                .columns
                .iter()
                .map(|column| (column.index as usize, column.offset)));
        if sheet.variant != self.variant || !same_columns {
            return Ok(None);
        }
        Ok(string_rows(&sheet, &self.columns).ok())
    }

    /// The macro text of one cell of `strings`, rows this sheet read in
    /// another language with [`Self::strings_in`].
    pub(crate) fn text_in(
        &self,
        strings: &StringRows,
        row_id: u32,
        subrow_id: u16,
        column: u32,
    ) -> Option<String> {
        let position = self
            .columns
            .binary_search_by_key(&column, |string_column| string_column.index)
            .ok()?;
        let row = strings
            .binary_search_by_key(&(row_id, subrow_id), |(coordinate, _)| *coordinate)
            .ok()?;
        Some(aeria_se::codec::decode(&strings[row].1[position]))
    }

    /// Reads a listed sheet, its translation permission, and its row keys.
    pub(crate) fn read(
        game: &GameData,
        name: &str,
        language: SourceLanguage,
    ) -> Result<Result<Self, Unavailable>, SourceError> {
        let sheet = match read_excel(game, name, language)? {
            Ok(sheet) => sheet,
            Err(unavailable) => return Ok(Err(unavailable)),
        };
        let columns: Vec<StringColumn> = sheet
            .columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.kind == ColumnKind::String)
            .map(|(index, column)| StringColumn {
                index: u32::try_from(index).expect("a header has at most u16::MAX columns"),
                offset: column.offset,
            })
            .collect();
        let strings = match string_rows(&sheet, &columns) {
            Ok(rows) => rows,
            Err(unavailable) => return Ok(Err(unavailable)),
        };
        let differs = differing_cells(game, name, language, &sheet, &columns, &strings)?;
        let rows: Vec<SourceRow> = strings
            .into_iter()
            .zip(differs)
            .map(|(((row_id, subrow_id), strings), differs)| {
                let translatable = strings
                    .iter()
                    .zip(differs.iter())
                    .map(|(bytes, differs)| !bytes.is_empty() && *differs)
                    .collect();
                SourceRow {
                    row_id,
                    subrow_id,
                    strings,
                    translatable,
                }
            })
            .collect();
        let row_keys = RowKeys::detect(&columns, &rows);
        Ok(Ok(Self {
            name: name.to_owned(),
            variant: sheet.variant,
            layout: layout_hash(&columns),
            columns,
            rows,
            row_keys,
        }))
    }
}

fn read_excel(
    game: &GameData,
    name: &str,
    language: SourceLanguage,
) -> Result<Result<excel::Sheet, Unavailable>, SourceError> {
    match excel::read_sheet(game, name, language.excel()) {
        Ok(sheet) => Ok(Ok(sheet)),
        Err(SheetError::Game(error)) => Err(SourceError::Game(error)),
        Err(error) => Ok(Err(Unavailable {
            message: error.to_string(),
        })),
    }
}

pub(crate) type StringRows = Vec<((u32, u16), Box<[Box<[u8]>]>)>;

/// Reads the String cells of every row, sorted by `(row, subrow)`.
fn string_rows(sheet: &excel::Sheet, columns: &[StringColumn]) -> Result<StringRows, Unavailable> {
    let mut rows = Vec::new();
    for row in sheet.rows() {
        let strings = columns
            .iter()
            .map(|column| {
                row.string(column.index as usize)
                    .map(Box::<[u8]>::from)
                    .map_err(|error| Unavailable {
                        message: format!("row {} subrow {}: {error}", row.row_id, row.subrow_id),
                    })
            })
            .collect::<Result<Box<[_]>, _>>()?;
        rows.push(((row.row_id, row.subrow_id), strings));
    }
    rows.sort_by_key(|(coordinate, _)| *coordinate);
    if let Some(pair) = rows.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        return Err(Unavailable {
            message: format!("row {} subrow {} appears twice", pair[0].0.0, pair[0].0.1),
        });
    }
    Ok(rows)
}

/// For every cell, whether any other evidence language has different bytes.
/// All `false` when the sheet is not comparable across the languages.
fn differing_cells(
    game: &GameData,
    name: &str,
    language: SourceLanguage,
    sheet: &excel::Sheet,
    columns: &[StringColumn],
    rows: &StringRows,
) -> Result<Vec<Vec<bool>>, SourceError> {
    let mut differs: Vec<Vec<bool>> = rows
        .iter()
        .map(|(_, strings)| vec![false; strings.len()])
        .collect();
    let mut comparisons = Vec::new();
    for other in SourceLanguage::ALL {
        if other == language {
            continue;
        }
        let Ok(other_sheet) = read_excel(game, name, other)? else {
            return Ok(all_false(rows));
        };
        if other_sheet.variant != sheet.variant
            || !same_columns(&other_sheet.columns, &sheet.columns)
        {
            return Ok(all_false(rows));
        }
        let Ok(other_rows) = string_rows(&other_sheet, columns) else {
            return Ok(all_false(rows));
        };
        if other_rows.len() != rows.len()
            || other_rows
                .iter()
                .zip(rows)
                .any(|(other, own)| other.0 != own.0)
        {
            return Ok(all_false(rows));
        }
        comparisons.push(other_rows);
    }
    for other_rows in comparisons {
        for ((row_differs, (_, own)), (_, other)) in differs.iter_mut().zip(rows).zip(other_rows) {
            for ((cell_differs, own), other) in
                row_differs.iter_mut().zip(own.iter()).zip(other.iter())
            {
                *cell_differs |= own != other;
            }
        }
    }
    Ok(differs)
}

fn all_false(rows: &StringRows) -> Vec<Vec<bool>> {
    rows.iter()
        .map(|(_, strings)| vec![false; strings.len()])
        .collect()
}

fn same_columns(left: &[Column], right: &[Column]) -> bool {
    left == right
}

/// The row key column of one sheet, and its keys.
///
/// A String column is the row key column when the sheet has at least
/// [`MIN_KEYED_ROWS`] rows, every row has a non-empty text in the column, the
/// texts are unique, and none of the column's cells is translatable. When
/// several columns qualify, the lowest column index is used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowKeys {
    column: u32,
    by_row: BTreeMap<(u32, u16), String>,
    by_key: HashMap<String, (u32, u16)>,
}

impl RowKeys {
    fn detect(columns: &[StringColumn], rows: &[SourceRow]) -> Option<Self> {
        if rows.len() < MIN_KEYED_ROWS {
            return None;
        }
        'columns: for (position, column) in columns.iter().enumerate() {
            if rows
                .iter()
                .any(|row| row.strings[position].is_empty() || row.translatable[position])
            {
                continue;
            }
            let mut by_row = BTreeMap::new();
            let mut by_key = HashMap::with_capacity(rows.len());
            for row in rows {
                let key = aeria_se::codec::decode(&row.strings[position]);
                if by_key
                    .insert(key.clone(), (row.row_id, row.subrow_id))
                    .is_some()
                {
                    continue 'columns;
                }
                by_row.insert((row.row_id, row.subrow_id), key);
            }
            return Some(Self {
                column: column.index,
                by_row,
                by_key,
            });
        }
        None
    }

    /// The row key column index.
    #[must_use]
    pub const fn column(&self) -> u32 {
        self.column
    }

    /// The key of one row, when the row exists.
    #[must_use]
    pub fn key_of(&self, row_id: u32, subrow_id: u16) -> Option<&str> {
        self.by_row.get(&(row_id, subrow_id)).map(String::as_str)
    }

    /// The row that holds `key`.
    #[must_use]
    pub fn row_of(&self, key: &str) -> Option<(u32, u16)> {
        self.by_key.get(key).copied()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    type TestRow<'a> = ((u32, u16), &'a [(&'a str, bool)]);

    /// Builds rows for tests: every cell is `(text, translatable)`.
    pub(crate) fn rows(cells: &[TestRow<'_>]) -> Vec<SourceRow> {
        cells
            .iter()
            .map(|((row_id, subrow_id), cells)| SourceRow {
                row_id: *row_id,
                subrow_id: *subrow_id,
                strings: cells
                    .iter()
                    .map(|(text, _)| Box::<[u8]>::from(text.as_bytes()))
                    .collect(),
                translatable: cells
                    .iter()
                    .map(|(_, translatable)| *translatable)
                    .collect(),
            })
            .collect()
    }

    fn columns(count: u32) -> Vec<StringColumn> {
        (0..count)
            .map(|index| StringColumn {
                index: index * 2,
                offset: u16::try_from(index * 4).expect("small"),
            })
            .collect()
    }

    #[test]
    fn layout_hashes_follow_the_documented_encoding() {
        let columns = [
            StringColumn {
                index: 0,
                offset: 0,
            },
            StringColumn {
                index: 3,
                offset: 8,
            },
        ];
        assert_eq!(layout_hash(&columns).to_string(), "898602844f6ecf4f");
        assert_ne!(layout_hash(&columns), layout_hash(&columns[..1]));
    }

    #[test]
    fn the_first_unique_language_invariant_column_is_the_row_key() {
        let rows = rows(&[
            ((1, 0), &[("same", false), ("K1", false), ("Alpha", true)]),
            ((2, 0), &[("same", false), ("K2", false), ("Beta", true)]),
        ]);
        let keys = RowKeys::detect(&columns(3), &rows).expect("keyed");
        assert_eq!(keys.column(), 2);
        assert_eq!(keys.key_of(2, 0), Some("K2"));
        assert_eq!(keys.row_of("K1"), Some((1, 0)));
        assert_eq!(keys.row_of("K3"), None);
    }

    #[test]
    fn empty_duplicate_translatable_or_single_rows_are_not_keys() {
        let empty = rows(&[((1, 0), &[("K1", false)]), ((2, 0), &[("", false)])]);
        assert!(RowKeys::detect(&columns(1), &empty).is_none());
        let duplicate = rows(&[((1, 0), &[("K", false)]), ((2, 0), &[("K", false)])]);
        assert!(RowKeys::detect(&columns(1), &duplicate).is_none());
        let translatable = rows(&[((1, 0), &[("A", false)]), ((2, 0), &[("B", true)])]);
        assert!(RowKeys::detect(&columns(1), &translatable).is_none());
        let single = rows(&[((1, 0), &[("K1", false)])]);
        assert!(RowKeys::detect(&columns(1), &single).is_none());
    }
}
