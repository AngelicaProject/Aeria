//! Row keys: a language-invariant String column that identifies rows.
//!
//! Some sheets, such as quest and cutscene dialogue, number their rows as a
//! sequence and carry a stable text key (for example `TEXT_..._000_000`) in a
//! String column. Inserting a line renumbers every later row, so the row ID
//! is not a durable coordinate there, but the key is. A row key lets a source
//! update follow a row whose ID changed and recognize a row whose line was
//! removed even when its ID is reused.

use std::collections::{BTreeMap, BTreeSet};

use aeria_core::{Sha256Hash, SourceBinding, SourceFingerprint};
use aeria_hxs::HxsSnapshot;
use sha2::{Digest, Sha256};

use crate::{CurrentSheet, SourceUpdateError};

/// The smallest number of rows for which a sheet can be keyed.
pub const MIN_KEYED_ROWS: usize = 2;

/// The row key column of one sheet in one snapshot, and its keys.
///
/// A String column is the row key column when, in every row of the sheet, it
/// holds a non-empty macro text, the values are unique across rows, and
/// source guidance permits none of its occurrences. Guidance permits only
/// text that differs between official languages, so a row key column is
/// identical in every language. When several columns qualify, the lowest
/// column index is chosen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowKeys {
    column: u32,
    by_row: BTreeMap<(u32, u16), Sha256Hash>,
    by_key: BTreeMap<Sha256Hash, (u32, u16)>,
}

impl RowKeys {
    /// Reads one verified sheet and detects its row key column.
    ///
    /// `is_translatable` must answer from the guidance that belongs to
    /// `snapshot`. Returns `None` for an unknown or unkeyed sheet.
    ///
    /// # Errors
    ///
    /// Returns an error when the sheet's String occurrences cannot be read.
    pub fn read<F>(
        snapshot: &HxsSnapshot,
        sheet_name: &str,
        is_translatable: F,
    ) -> Result<Option<Self>, SourceUpdateError>
    where
        F: Fn(&SourceBinding) -> bool,
    {
        let Some(sheet) = snapshot.sheet(sheet_name) else {
            return Ok(None);
        };
        let current = CurrentSheet::read(snapshot, &sheet)?;
        Ok(Self::detect(
            &current.name,
            current.string_columns.keys().copied(),
            &current.occurrences,
            is_translatable,
        ))
    }

    pub(crate) fn detect<F>(
        sheet_name: &str,
        string_columns: impl IntoIterator<Item = u32>,
        occurrences: &BTreeMap<(u32, u16, u32), SourceFingerprint>,
        is_translatable: F,
    ) -> Option<Self>
    where
        F: Fn(&SourceBinding) -> bool,
    {
        let rows: BTreeSet<(u32, u16)> = occurrences
            .keys()
            .map(|&(row_id, subrow_id, _)| (row_id, subrow_id))
            .collect();
        if rows.len() < MIN_KEYED_ROWS {
            return None;
        }
        let empty = empty_macro_hash();
        'columns: for column in string_columns {
            let mut by_row = BTreeMap::new();
            let mut by_key = BTreeMap::new();
            for &(row_id, subrow_id) in &rows {
                let Some(fingerprint) = occurrences.get(&(row_id, subrow_id, column)) else {
                    continue 'columns;
                };
                let key = fingerprint.macro_text_hash();
                if key == empty || by_key.insert(key, (row_id, subrow_id)).is_some() {
                    continue 'columns;
                }
                by_row.insert((row_id, subrow_id), key);
            }
            let permitted = rows.iter().any(|&(row_id, subrow_id)| {
                is_translatable(&SourceBinding::new(sheet_name, row_id, subrow_id, column))
            });
            if !permitted {
                return Some(Self {
                    column,
                    by_row,
                    by_key,
                });
            }
        }
        None
    }

    /// Returns the row key column index.
    #[must_use]
    pub const fn column(&self) -> u32 {
        self.column
    }

    /// Returns the key of one row, if the row exists.
    #[must_use]
    pub fn key_of(&self, row_id: u32, subrow_id: u16) -> Option<Sha256Hash> {
        self.by_row.get(&(row_id, subrow_id)).copied()
    }

    /// Returns the row that holds `key`, if any.
    #[must_use]
    pub fn row_of(&self, key: Sha256Hash) -> Option<(u32, u16)> {
        self.by_key.get(&key).copied()
    }
}

/// The HXS v1 macro hash of the empty string.
fn empty_macro_hash() -> Sha256Hash {
    let mut hasher = Sha256::new();
    hasher.update(b"HARMONIA-HXS-V1-MACRO");
    hasher.update(0_u32.to_le_bytes());
    Sha256Hash::from_bytes(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(macro_byte: u8) -> SourceFingerprint {
        SourceFingerprint::new(
            Sha256Hash::from_bytes([macro_byte; 32]),
            None,
            Sha256Hash::from_bytes([0; 32]),
        )
    }

    fn occurrences(
        cells: &[(u32, u32, SourceFingerprint)],
    ) -> BTreeMap<(u32, u16, u32), SourceFingerprint> {
        cells
            .iter()
            .map(|&(row_id, column, fingerprint)| ((row_id, 0, column), fingerprint))
            .collect()
    }

    #[test]
    fn a_unique_non_empty_untranslatable_column_is_the_row_key() {
        let cells = occurrences(&[
            (1, 0, fingerprint(10)),
            (1, 1, fingerprint(20)),
            (2, 0, fingerprint(11)),
            (2, 1, fingerprint(21)),
        ]);
        let keys = RowKeys::detect("quest/000/Test", [0, 1], &cells, |binding| {
            binding.column_index() == 1
        })
        .expect("column 0 is a key");
        assert_eq!(keys.column(), 0);
        assert_eq!(keys.key_of(2, 0), Some(Sha256Hash::from_bytes([11; 32])));
        assert_eq!(keys.row_of(Sha256Hash::from_bytes([10; 32])), Some((1, 0)));
        assert_eq!(keys.row_of(Sha256Hash::from_bytes([21; 32])), None);
    }

    #[test]
    fn translatable_duplicate_empty_and_tiny_columns_are_not_keys() {
        let unique = occurrences(&[(1, 0, fingerprint(10)), (2, 0, fingerprint(11))]);
        assert!(RowKeys::detect("Sheet", [0], &unique, |_| true).is_none());

        let duplicate = occurrences(&[(1, 0, fingerprint(10)), (2, 0, fingerprint(10))]);
        assert!(RowKeys::detect("Sheet", [0], &duplicate, |_| false).is_none());

        let empty =
            SourceFingerprint::new(empty_macro_hash(), None, Sha256Hash::from_bytes([0; 32]));
        let with_empty = occurrences(&[(1, 0, fingerprint(10)), (2, 0, empty)]);
        assert!(RowKeys::detect("Sheet", [0], &with_empty, |_| false).is_none());

        let single = occurrences(&[(1, 0, fingerprint(10))]);
        assert!(RowKeys::detect("Sheet", [0], &single, |_| false).is_none());
    }
}
