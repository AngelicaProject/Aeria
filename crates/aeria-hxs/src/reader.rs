use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::MAX_ROW_PAGE_SIZE;
use crate::error::HxsError;
use crate::types::{RowPage, RowRecord, SheetMetadata, SnapshotMetadata, StringCell};
use crate::validation::{
    APPLICATION_ID, FORMAT_VERSION, VerifiedSnapshot, read_row_record, read_string_cell,
    validate_and_read,
};

/// A verified, read-only handle to one immutable HXS source artifact.
pub struct HxsSnapshot {
    connection: Connection,
    metadata: SnapshotMetadata,
    sheets: Vec<SheetMetadata>,
    sheet_ids: HashMap<String, i64>,
}

impl HxsSnapshot {
    /// Opens and fully verifies an HXS v1 file without taking ownership of or modifying it.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be opened read-only, does not have the HXS v1
    /// identity, has an invalid schema, or fails any logical verification check.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, HxsError> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(HxsError::storage)?;
        connection
            .execute_batch("PRAGMA query_only = ON; PRAGMA foreign_keys = ON;")
            .map_err(HxsError::storage)?;
        validate_identity(&connection)?;
        let VerifiedSnapshot { metadata, sheets } = validate_and_read(&connection)?;
        let sheet_ids = sheets
            .iter()
            .map(|(id, sheet)| (sheet.name.clone(), *id))
            .collect();
        Ok(Self {
            connection,
            metadata,
            sheets: sheets.into_iter().map(|(_, sheet)| sheet).collect(),
            sheet_ids,
        })
    }

    /// Returns verified source metadata as an owned DTO.
    #[must_use]
    pub fn metadata(&self) -> SnapshotMetadata {
        self.metadata.clone()
    }

    /// Enumerates verified sheets in their physical HXS catalog order.
    #[must_use]
    pub fn sheets(&self) -> Vec<SheetMetadata> {
        self.sheets.clone()
    }

    /// Returns verified metadata for one sheet, if it exists.
    #[must_use]
    pub fn sheet(&self, name: &str) -> Option<SheetMetadata> {
        self.sheets.iter().find(|sheet| sheet.name == name).cloned()
    }

    /// Reads one bounded page of rows in row/subrow order.
    ///
    /// `limit` must be between one and [`MAX_ROW_PAGE_SIZE`] inclusive.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown sheet, an invalid page size, an offset that SQLite cannot
    /// represent, or a storage/read failure.
    pub fn page_rows(
        &self,
        sheet_name: &str,
        offset: u64,
        limit: u32,
    ) -> Result<RowPage, HxsError> {
        if !(1..=MAX_ROW_PAGE_SIZE).contains(&limit) {
            return Err(HxsError::request(
                "row page limit must be between 1 and MAX_ROW_PAGE_SIZE",
            ));
        }
        let sheet = self
            .sheet(sheet_name)
            .ok_or_else(|| HxsError::SheetNotFound {
                name: sheet_name.to_owned(),
            })?;
        let sheet_id = self.sheet_id(sheet_name)?;
        let offset_sql = i64::try_from(offset)
            .map_err(|_| HxsError::request("row page offset is too large for SQLite"))?;
        let limit_sql = i64::from(limit);
        let mut statement = self
            .connection
            .prepare(
                "SELECT row_id, subrow_id, technical_payload, row_hash, technical_hash, string_hash \
                 FROM rows WHERE sheet_id = ?1 \
                 ORDER BY row_id, subrow_id LIMIT ?2 OFFSET ?3",
            )
            .map_err(HxsError::storage)?;
        let mut rows = statement
            .query((sheet_id, limit_sql, offset_sql))
            .map_err(HxsError::storage)?;
        let mut page = Vec::new();
        while let Some(row) = rows.next().map_err(HxsError::storage)? {
            page.push(read_row_record(row)?);
        }
        let page_length = u64::try_from(page.len())
            .map_err(|_| HxsError::data("row page length is out of range"))?;
        let next_offset = offset
            .checked_add(page_length)
            .filter(|next| *next < sheet.row_count);
        Ok(RowPage {
            offset,
            rows: page,
            next_offset,
            total_rows: sheet.row_count,
        })
    }

    /// Reads one row by its source coordinate.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown sheet or a storage/read failure.
    pub fn row(
        &self,
        sheet_name: &str,
        row_id: u32,
        subrow_id: u16,
    ) -> Result<Option<RowRecord>, HxsError> {
        let sheet_id = self.sheet_id(sheet_name)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT row_id, subrow_id, technical_payload, row_hash, technical_hash, string_hash \
                 FROM rows WHERE sheet_id = ?1 AND row_id = ?2 AND subrow_id = ?3",
            )
            .map_err(HxsError::storage)?;
        let mut rows = statement
            .query((sheet_id, i64::from(row_id), i64::from(subrow_id)))
            .map_err(HxsError::storage)?;
        rows.next()
            .map_err(HxsError::storage)?
            .map(read_row_record)
            .transpose()
    }

    /// Reads one String cell, including macro text and optional raw source bytes.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown sheet or a storage/read failure.
    pub fn string_cell(
        &self,
        sheet_name: &str,
        row_id: u32,
        subrow_id: u16,
        column_index: u32,
    ) -> Result<Option<StringCell>, HxsError> {
        let sheet_id = self.sheet_id(sheet_name)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT row_id, subrow_id, column_index, macro_text, raw_value, macro_hash, raw_hash \
                 FROM string_cells WHERE sheet_id = ?1 AND row_id = ?2 \
                   AND subrow_id = ?3 AND column_index = ?4",
            )
            .map_err(HxsError::storage)?;
        let mut rows = statement
            .query((
                sheet_id,
                i64::from(row_id),
                i64::from(subrow_id),
                i64::from(column_index),
            ))
            .map_err(HxsError::storage)?;
        rows.next()
            .map_err(HxsError::storage)?
            .map(read_string_cell)
            .transpose()
    }

    fn sheet_id(&self, sheet_name: &str) -> Result<i64, HxsError> {
        self.sheet_ids
            .get(sheet_name)
            .copied()
            .ok_or_else(|| HxsError::SheetNotFound {
                name: sheet_name.to_owned(),
            })
    }
}

fn validate_identity(connection: &Connection) -> Result<(), HxsError> {
    let application_id = read_pragma_i64(connection, "application_id")?;
    if application_id != i64::from(APPLICATION_ID) {
        return Err(HxsError::UnsupportedApplicationId {
            expected: APPLICATION_ID,
            found: application_id,
        });
    }
    let format_version = read_pragma_i64(connection, "user_version")?;
    if format_version != FORMAT_VERSION {
        return Err(HxsError::UnsupportedFormatVersion {
            expected: FORMAT_VERSION,
            found: format_version,
        });
    }
    Ok(())
}

fn read_pragma_i64(connection: &Connection, pragma: &str) -> Result<i64, HxsError> {
    connection
        .query_row(&format!("PRAGMA {pragma}"), [], |row| row.get::<_, i64>(0))
        .map_err(HxsError::storage)
}
