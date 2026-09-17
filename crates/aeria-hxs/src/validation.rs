use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, Row, types::ValueRef};

use crate::error::HxsError;
use crate::hashing::{
    add_sheet_row, begin_sheet_hash, compute_content_id, compute_snapshot_id,
    decode_technical_payload, encode_technical_payload, finish_sheet_hash, hash_macro, hash_raw,
    hash_row, hash_row_strings, hash_row_technical, hash_schema, hash_sheet_content, ordinal_cmp,
};
use crate::types::{
    ColumnMetadata, ColumnType, HxsHash, ProducerMetadata, RowHashes, SheetHashes, SheetMetadata,
    SheetVariant, SnapshotCounts, SnapshotMetadata, StringCell, StringCellHashes,
};

pub(crate) const APPLICATION_ID: u32 = 0x4841_544c;
pub(crate) const FORMAT_VERSION: i64 = 1;

const REQUIRED_TABLES: [&str; 5] = ["hxs_meta", "sheets", "columns", "rows", "string_cells"];

pub(crate) struct VerifiedSnapshot {
    pub metadata: SnapshotMetadata,
    pub sheets: Vec<(i64, SheetMetadata)>,
}

#[allow(clippy::too_many_lines)]
pub(crate) fn validate_and_read(connection: &Connection) -> Result<VerifiedSnapshot, HxsError> {
    validate_schema(connection)?;
    validate_integrity(connection)?;
    let metadata = read_metadata(connection)?;

    let mut sheets = Vec::new();
    let mut names = HashSet::new();
    let mut actual_row_count = 0u64;
    let mut actual_string_count = 0u64;

    let mut statement = connection
        .prepare(
            "SELECT id, name, variant, effective_language, column_count, row_count, \
             schema_hash, technical_hash, string_hash, content_hash \
             FROM sheets ORDER BY id",
        )
        .map_err(HxsError::storage)?;
    let mut rows = statement.query([]).map_err(HxsError::storage)?;
    while let Some(row) = rows.next().map_err(HxsError::storage)? {
        let sheet_id = read_i64(row, 0, "sheets.id")?;
        if sheet_id <= 0 {
            return Err(HxsError::data("sheets.id must be positive"));
        }
        let name = read_text(row, 1, "sheets.name")?;
        if name.is_empty() {
            return Err(HxsError::data("sheet names must not be empty"));
        }
        if !names.insert(name.clone()) {
            return Err(HxsError::data(format!("duplicate sheet name '{name}'")));
        }
        let variant_code = read_i64(row, 2, "sheets.variant")?;
        let variant = SheetVariant::from_code(variant_code).ok_or_else(|| {
            HxsError::data(format!("unsupported HXS sheet variant {variant_code}"))
        })?;
        let effective_language = read_text(row, 3, "sheets.effective_language")?;
        if effective_language.is_empty() {
            return Err(HxsError::data(format!(
                "effective language is empty for sheet '{name}'"
            )));
        }
        let column_count = read_non_negative_u64(row, 4, "sheets.column_count")?;
        let stored_row_count = read_non_negative_u64(row, 5, "sheets.row_count")?;
        let schema_hash = read_hash(row, 6, "sheets.schema_hash")?;
        let stored_technical_hash = read_hash(row, 7, "sheets.technical_hash")?;
        let stored_string_hash = read_hash(row, 8, "sheets.string_hash")?;
        let stored_content_hash = read_hash(row, 9, "sheets.content_hash")?;

        let columns = read_columns(connection, sheet_id, &name)?;
        if u64::try_from(columns.len()).expect("a Vec length fits in u64") != column_count {
            return Err(HxsError::data(format!(
                "sheet '{name}' column count does not match its columns"
            )));
        }
        let schema_columns: Vec<_> = columns
            .iter()
            .map(|column| (column.index, column.offset, column.column_type))
            .collect();
        let expected_schema_hash = hash_schema(&name, variant, &schema_columns)?;
        require_hash(
            &schema_hash,
            &expected_schema_hash,
            &format!("schema hash for sheet '{name}'"),
        )?;

        let (row_count, string_count, technical_hash, string_hash) =
            verify_rows(connection, sheet_id, &name, &columns, stored_row_count)?;
        require_hash(
            &stored_technical_hash,
            &technical_hash,
            &format!("technical hash for sheet '{name}'"),
        )?;
        require_hash(
            &stored_string_hash,
            &string_hash,
            &format!("string hash for sheet '{name}'"),
        )?;
        let content_hash =
            hash_sheet_content(&name, variant, &schema_hash, &technical_hash, &string_hash)?;
        require_hash(
            &stored_content_hash,
            &content_hash,
            &format!("content hash for sheet '{name}'"),
        )?;

        actual_row_count = actual_row_count
            .checked_add(row_count)
            .ok_or_else(|| HxsError::data("snapshot row count overflow"))?;
        actual_string_count = actual_string_count
            .checked_add(string_count)
            .ok_or_else(|| HxsError::data("snapshot String-cell count overflow"))?;
        sheets.push((
            sheet_id,
            SheetMetadata {
                name,
                variant,
                effective_language,
                columns,
                row_count,
                hashes: SheetHashes {
                    schema: schema_hash,
                    technical: stored_technical_hash,
                    strings: stored_string_hash,
                    content: stored_content_hash,
                },
            },
        ));
    }

    let sheet_count = u64::try_from(sheets.len()).expect("a Vec length fits in u64");
    if metadata.counts
        != (SnapshotCounts {
            sheets: sheet_count,
            rows: actual_row_count,
            string_cells: actual_string_count,
        })
    {
        return Err(HxsError::data(
            "HXS metadata counts do not match the stored artifact",
        ));
    }

    let mut content_sheets: Vec<_> = sheets
        .iter()
        .map(|(_, sheet)| {
            (
                sheet.name.as_str(),
                sheet.effective_language.as_str(),
                &sheet.hashes.schema,
                &sheet.hashes.content,
            )
        })
        .collect();
    content_sheets.sort_unstable_by(|left, right| ordinal_cmp(left.0, right.0));
    let content_id = compute_content_id(&metadata.source_language, &content_sheets)?;
    if metadata.content_id != content_id {
        return Err(HxsError::data(
            "HXS content_id does not match the stored source content",
        ));
    }
    let snapshot_id = compute_snapshot_id(
        &metadata.game_version,
        &metadata.source_language,
        &content_id,
    )?;
    if metadata.snapshot_id != snapshot_id {
        return Err(HxsError::data(
            "HXS snapshot_id does not match the stored source snapshot",
        ));
    }

    Ok(VerifiedSnapshot { metadata, sheets })
}

fn validate_integrity(connection: &Connection) -> Result<(), HxsError> {
    let mut integrity = connection
        .prepare("PRAGMA integrity_check")
        .map_err(HxsError::storage)?;
    let mut integrity_rows = integrity.query([]).map_err(HxsError::storage)?;
    let Some(row) = integrity_rows.next().map_err(HxsError::storage)? else {
        return Err(HxsError::data("SQLite integrity_check returned no result"));
    };
    if !read_text(row, 0, "integrity_check")?.eq_ignore_ascii_case("ok") {
        return Err(HxsError::data("SQLite integrity_check failed"));
    }

    let mut foreign_keys = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(HxsError::storage)?;
    let mut foreign_key_rows = foreign_keys.query([]).map_err(HxsError::storage)?;
    if foreign_key_rows
        .next()
        .map_err(HxsError::storage)?
        .is_some()
    {
        return Err(HxsError::data("SQLite foreign_key_check failed"));
    }
    Ok(())
}

fn validate_schema(connection: &Connection) -> Result<(), HxsError> {
    let expected: HashSet<String> = REQUIRED_TABLES.into_iter().map(str::to_owned).collect();
    let mut actual = HashSet::new();
    let mut statement = connection
        .prepare(
            "SELECT type, name FROM sqlite_master \
             WHERE type IN ('table', 'view', 'trigger', 'index') \
               AND name NOT GLOB 'sqlite_*' ORDER BY type, name",
        )
        .map_err(HxsError::storage)?;
    let mut rows = statement.query([]).map_err(HxsError::storage)?;
    while let Some(row) = rows.next().map_err(HxsError::storage)? {
        let object_type = read_text(row, 0, "sqlite_master.type")?;
        let object_name = read_text(row, 1, "sqlite_master.name")?;
        if object_type != "table" || !expected.contains(object_name.as_str()) {
            return Err(HxsError::schema(format!(
                "unexpected HXS schema object: {object_type} {object_name}"
            )));
        }
        actual.insert(object_name);
    }
    if actual != expected {
        let missing: Vec<_> = expected.difference(&actual).cloned().collect();
        return Err(HxsError::schema(format!(
            "required HXS tables are missing: {}",
            missing.join(", ")
        )));
    }

    for table in REQUIRED_TABLES {
        validate_table_columns(connection, table)?;
        validate_primary_key(connection, table)?;
    }
    validate_foreign_keys(connection)?;
    Ok(())
}

fn validate_table_columns(connection: &Connection, table: &str) -> Result<(), HxsError> {
    let expected: HashSet<String> = expected_columns(table)
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut actual = HashSet::new();
    let mut statement = connection
        .prepare(&format!("PRAGMA table_xinfo(\"{table}\")"))
        .map_err(HxsError::storage)?;
    let mut rows = statement.query([]).map_err(HxsError::storage)?;
    while let Some(row) = rows.next().map_err(HxsError::storage)? {
        let name = read_text(row, 1, "table_xinfo.name")?;
        let declared_type = read_text(row, 2, "table_xinfo.type")?;
        if expected_column_type(&name) != Some(declared_type.as_str()) {
            return Err(HxsError::schema(format!(
                "HXS table '{table}' column '{name}' has the wrong declared type"
            )));
        }
        if read_i64(row, 6, "table_xinfo.hidden")? != 0 {
            return Err(HxsError::schema(format!(
                "HXS table '{table}' contains a hidden or generated column"
            )));
        }
        actual.insert(name);
    }
    if actual != expected {
        return Err(HxsError::schema(format!(
            "HXS table '{table}' columns do not match the required schema"
        )));
    }
    Ok(())
}

fn expected_column_type(column: &str) -> Option<&'static str> {
    let integer = [
        "id",
        "format_version",
        "variant",
        "column_count",
        "row_count",
        "sheet_count",
        "string_cell_count",
        "sheet_id",
        "column_index",
        "offset",
        "type",
        "row_id",
        "subrow_id",
    ];
    let text = [
        "game_version",
        "language",
        "scope",
        "content_id",
        "snapshot_id",
        "extractor_version",
        "lumina_version",
        "name",
        "effective_language",
        "macro_text",
    ];
    let blob = [
        "schema_hash",
        "technical_hash",
        "string_hash",
        "content_hash",
        "technical_payload",
        "row_hash",
        "raw_value",
        "macro_hash",
        "raw_hash",
    ];
    if integer.contains(&column) {
        Some("INTEGER")
    } else if text.contains(&column) {
        Some("TEXT")
    } else if blob.contains(&column) {
        Some("BLOB")
    } else {
        None
    }
}

fn validate_primary_key(connection: &Connection, table: &str) -> Result<(), HxsError> {
    let expected = expected_primary_key(table);
    let mut actual = HashMap::new();
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .map_err(HxsError::storage)?;
    let mut rows = statement.query([]).map_err(HxsError::storage)?;
    while let Some(row) = rows.next().map_err(HxsError::storage)? {
        actual.insert(
            read_text(row, 1, "table_info.name")?,
            read_i64(row, 5, "table_info.pk")?,
        );
    }
    if actual != expected {
        return Err(HxsError::schema(format!(
            "HXS table '{table}' primary key does not match the required schema"
        )));
    }
    Ok(())
}

fn validate_foreign_keys(connection: &Connection) -> Result<(), HxsError> {
    for table in ["columns", "rows", "string_cells"] {
        let mut actual = Vec::new();
        let mut statement = connection
            .prepare(&format!("PRAGMA foreign_key_list(\"{table}\")"))
            .map_err(HxsError::storage)?;
        let mut rows = statement.query([]).map_err(HxsError::storage)?;
        while let Some(row) = rows.next().map_err(HxsError::storage)? {
            actual.push((
                read_text(row, 2, "foreign_key_list.table")?,
                read_text(row, 3, "foreign_key_list.from")?,
                read_text(row, 4, "foreign_key_list.to")?,
            ));
        }
        actual.sort_unstable();
        let mut expected = expected_foreign_keys(table);
        expected.sort_unstable();
        if actual != expected {
            return Err(HxsError::schema(format!(
                "HXS table '{table}' foreign keys do not match the required schema"
            )));
        }
    }
    Ok(())
}

fn expected_columns(table: &str) -> Vec<&'static str> {
    match table {
        "hxs_meta" => vec![
            "id",
            "format_version",
            "game_version",
            "language",
            "scope",
            "content_id",
            "snapshot_id",
            "extractor_version",
            "lumina_version",
            "sheet_count",
            "row_count",
            "string_cell_count",
        ],
        "sheets" => vec![
            "id",
            "name",
            "variant",
            "effective_language",
            "column_count",
            "row_count",
            "schema_hash",
            "technical_hash",
            "string_hash",
            "content_hash",
        ],
        "columns" => vec!["sheet_id", "column_index", "offset", "type"],
        "rows" => vec![
            "sheet_id",
            "row_id",
            "subrow_id",
            "technical_payload",
            "row_hash",
            "technical_hash",
            "string_hash",
        ],
        "string_cells" => vec![
            "sheet_id",
            "row_id",
            "subrow_id",
            "column_index",
            "macro_text",
            "raw_value",
            "macro_hash",
            "raw_hash",
        ],
        _ => unreachable!("the caller only asks for required tables"),
    }
}

fn expected_primary_key(table: &str) -> HashMap<String, i64> {
    let columns = expected_columns(table);
    let key_columns: &[&str] = match table {
        "hxs_meta" | "sheets" => &["id"],
        "columns" => &["sheet_id", "column_index"],
        "rows" => &["sheet_id", "row_id", "subrow_id"],
        "string_cells" => &["sheet_id", "row_id", "subrow_id", "column_index"],
        _ => unreachable!("the caller only asks for required tables"),
    };
    columns
        .into_iter()
        .map(|column| {
            let position = key_columns
                .iter()
                .position(|key| *key == column)
                .map_or(0, |position| {
                    i64::try_from(position + 1).expect("small key")
                });
            (column.to_owned(), position)
        })
        .collect()
}

fn expected_foreign_keys(table: &str) -> Vec<(String, String, String)> {
    match table {
        "columns" | "rows" => vec![("sheets".into(), "sheet_id".into(), "id".into())],
        "string_cells" => vec![
            ("rows".into(), "sheet_id".into(), "sheet_id".into()),
            ("rows".into(), "row_id".into(), "row_id".into()),
            ("rows".into(), "subrow_id".into(), "subrow_id".into()),
        ],
        _ => Vec::new(),
    }
}

fn read_metadata(connection: &Connection) -> Result<SnapshotMetadata, HxsError> {
    let mut statement = connection
        .prepare(
            "SELECT id, format_version, game_version, language, scope, content_id, snapshot_id, \
             extractor_version, lumina_version, sheet_count, row_count, string_cell_count \
             FROM hxs_meta",
        )
        .map_err(HxsError::storage)?;
    let mut rows = statement.query([]).map_err(HxsError::storage)?;
    let mut result = None;
    while let Some(row) = rows.next().map_err(HxsError::storage)? {
        if result.is_some() {
            return Err(HxsError::data("hxs_meta must contain exactly one row"));
        }
        if read_i64(row, 0, "hxs_meta.id")? != 1 {
            return Err(HxsError::data("hxs_meta must contain row id 1"));
        }
        let format_version = read_i64(row, 1, "hxs_meta.format_version")?;
        if format_version != FORMAT_VERSION {
            return Err(HxsError::UnsupportedFormatVersion {
                expected: FORMAT_VERSION,
                found: format_version,
            });
        }
        let game_version = read_text(row, 2, "hxs_meta.game_version")?;
        let source_language = read_text(row, 3, "hxs_meta.language")?;
        let scope = read_text(row, 4, "hxs_meta.scope")?;
        let content_id = read_text(row, 5, "hxs_meta.content_id")?;
        let snapshot_id = read_text(row, 6, "hxs_meta.snapshot_id")?;
        let extractor_version = read_text(row, 7, "hxs_meta.extractor_version")?;
        let lumina_version = read_text(row, 8, "hxs_meta.lumina_version")?;
        if game_version.is_empty()
            || source_language.is_empty()
            || extractor_version.is_empty()
            || lumina_version.is_empty()
            || scope != "full"
        {
            return Err(HxsError::data(
                "HXS metadata contains invalid required text",
            ));
        }
        validate_id("content_id", &content_id)?;
        validate_id("snapshot_id", &snapshot_id)?;
        result = Some(SnapshotMetadata {
            format_version: u32::try_from(format_version).expect("format version is validated"),
            game_version,
            source_language,
            scope,
            content_id,
            snapshot_id,
            producer: ProducerMetadata {
                extractor_version,
                lumina_version,
            },
            counts: SnapshotCounts {
                sheets: read_non_negative_u64(row, 9, "hxs_meta.sheet_count")?,
                rows: read_non_negative_u64(row, 10, "hxs_meta.row_count")?,
                string_cells: read_non_negative_u64(row, 11, "hxs_meta.string_cell_count")?,
            },
        });
    }
    result.ok_or_else(|| HxsError::data("hxs_meta must contain exactly one row"))
}

fn read_columns(
    connection: &Connection,
    sheet_id: i64,
    sheet_name: &str,
) -> Result<Vec<ColumnMetadata>, HxsError> {
    let mut statement = connection
        .prepare(
            "SELECT column_index, offset, type FROM columns \
             WHERE sheet_id = ?1 ORDER BY column_index",
        )
        .map_err(HxsError::storage)?;
    let mut rows = statement.query([sheet_id]).map_err(HxsError::storage)?;
    let mut columns = Vec::new();
    let mut previous_index = None;
    while let Some(row) = rows.next().map_err(HxsError::storage)? {
        let index = read_non_negative_u32(row, 0, "columns.column_index")?;
        if previous_index.is_some_and(|previous| index <= previous) {
            return Err(HxsError::data(format!(
                "columns are not strictly ordered in sheet '{sheet_name}'"
            )));
        }
        let offset = read_non_negative_u32(row, 1, "columns.offset")?;
        let type_code = read_i64(row, 2, "columns.type")?;
        let column_type = ColumnType::from_code(type_code)
            .ok_or_else(|| HxsError::data(format!("unsupported HXS column type {type_code}")))?;
        columns.push(ColumnMetadata {
            index,
            offset,
            column_type,
        });
        previous_index = Some(index);
    }
    Ok(columns)
}

#[allow(clippy::too_many_lines)]
fn verify_rows(
    connection: &Connection,
    sheet_id: i64,
    sheet_name: &str,
    columns: &[ColumnMetadata],
    expected_row_count: u64,
) -> Result<(u64, u64, HxsHash, HxsHash), HxsError> {
    let technical_columns: Vec<_> = columns
        .iter()
        .map(|column| (column.index, column.column_type))
        .collect();
    let mut technical_sheet_hash = begin_sheet_hash("HARMONIA-HXS-V1-SHEET-TECHNICAL", sheet_name)?;
    let mut string_sheet_hash = begin_sheet_hash("HARMONIA-HXS-V1-SHEET-STRINGS", sheet_name)?;
    let mut statement = connection
        .prepare(
            "SELECT r.row_id, r.subrow_id, r.technical_payload, r.row_hash, \
                    r.technical_hash, r.string_hash, sc.column_index, sc.macro_text, \
                    sc.raw_value, sc.macro_hash, sc.raw_hash \
             FROM rows AS r \
             LEFT JOIN string_cells AS sc \
               ON sc.sheet_id = r.sheet_id AND sc.row_id = r.row_id \
              AND sc.subrow_id = r.subrow_id \
             WHERE r.sheet_id = ?1 \
             ORDER BY r.row_id, r.subrow_id, sc.column_index",
        )
        .map_err(HxsError::storage)?;
    let mut rows = statement.query([sheet_id]).map_err(HxsError::storage)?;
    let mut current = None;
    let mut previous_key = None;
    let mut row_count = 0u64;
    let mut string_count = 0u64;

    while let Some(row) = rows.next().map_err(HxsError::storage)? {
        let row_id = read_non_negative_u32(row, 0, "rows.row_id")?;
        let subrow_id = read_non_negative_u16(row, 1, "rows.subrow_id")?;
        let key = (row_id, subrow_id);
        if current.as_ref().is_none_or(|pending: &PendingRow| {
            pending.row_id != row_id || pending.subrow_id != subrow_id
        }) {
            if let Some(pending) = current.take() {
                let (verified, cells_count) =
                    finalize_row(pending, sheet_name, &technical_columns, columns)?;
                add_sheet_row(
                    &mut technical_sheet_hash,
                    verified.row_id,
                    verified.subrow_id,
                    &verified.hashes.technical,
                );
                add_sheet_row(
                    &mut string_sheet_hash,
                    verified.row_id,
                    verified.subrow_id,
                    &verified.hashes.strings,
                );
                row_count = row_count
                    .checked_add(1)
                    .ok_or_else(|| HxsError::data("sheet row count overflow"))?;
                string_count = string_count
                    .checked_add(cells_count)
                    .ok_or_else(|| HxsError::data("sheet String-cell count overflow"))?;
            }
            if previous_key.is_some_and(|previous| key <= previous) {
                return Err(HxsError::data(format!(
                    "rows are not strictly ordered in sheet '{sheet_name}'"
                )));
            }
            previous_key = Some(key);
            current = Some(PendingRow {
                row_id,
                subrow_id,
                technical_payload: read_blob(row, 2, "rows.technical_payload")?,
                hashes: RowHashes {
                    row: read_hash(row, 3, "rows.row_hash")?,
                    technical: read_hash(row, 4, "rows.technical_hash")?,
                    strings: read_hash(row, 5, "rows.string_hash")?,
                },
                string_cells: Vec::new(),
            });
        }

        if !matches!(row.get_ref(6).map_err(HxsError::storage)?, ValueRef::Null) {
            let pending = current
                .as_mut()
                .expect("a row exists before reading its String cells");
            let column_index = read_non_negative_u32(row, 6, "string_cells.column_index")?;
            if pending
                .string_cells
                .last()
                .is_some_and(|cell: &StringCell| column_index <= cell.column_index)
            {
                return Err(HxsError::data(format!(
                    "String cells are not strictly ordered in sheet '{sheet_name}'"
                )));
            }
            let macro_text = read_text(row, 7, "string_cells.macro_text")?;
            let raw_value = read_optional_blob(row, 8, "string_cells.raw_value")?;
            let macro_hash = read_hash(row, 9, "string_cells.macro_hash")?;
            let raw_hash = read_optional_hash(row, 10, "string_cells.raw_hash")?;
            require_hash(
                &macro_hash,
                &hash_macro(&macro_text),
                &format!("macro hash for {sheet_name}/{row_id}/{subrow_id}/{column_index}"),
            )?;
            match (&raw_value, &raw_hash) {
                (None, Some(_)) => {
                    return Err(HxsError::data(format!(
                        "raw hash is present without raw bytes for {sheet_name}/{row_id}/{subrow_id}/{column_index}"
                    )));
                }
                (Some(_), None) => {
                    return Err(HxsError::data(format!(
                        "raw bytes are present without a raw hash for {sheet_name}/{row_id}/{subrow_id}/{column_index}"
                    )));
                }
                (Some(raw), Some(raw_hash)) => {
                    require_hash(
                        raw_hash,
                        &hash_raw(raw)?,
                        &format!("raw hash for {sheet_name}/{row_id}/{subrow_id}/{column_index}"),
                    )?;
                }
                (None, None) => {}
            }
            pending.string_cells.push(StringCell {
                row_id,
                subrow_id,
                column_index,
                macro_text,
                raw_value,
                hashes: StringCellHashes {
                    macro_text: macro_hash,
                    raw_value: raw_hash,
                },
            });
        }
    }

    if let Some(pending) = current {
        let (verified, cells_count) =
            finalize_row(pending, sheet_name, &technical_columns, columns)?;
        add_sheet_row(
            &mut technical_sheet_hash,
            verified.row_id,
            verified.subrow_id,
            &verified.hashes.technical,
        );
        add_sheet_row(
            &mut string_sheet_hash,
            verified.row_id,
            verified.subrow_id,
            &verified.hashes.strings,
        );
        row_count = row_count
            .checked_add(1)
            .ok_or_else(|| HxsError::data("sheet row count overflow"))?;
        string_count = string_count
            .checked_add(cells_count)
            .ok_or_else(|| HxsError::data("sheet String-cell count overflow"))?;
    }
    if row_count != expected_row_count {
        return Err(HxsError::data(format!(
            "sheet '{sheet_name}' row count does not match its rows"
        )));
    }
    Ok((
        row_count,
        string_count,
        finish_sheet_hash(technical_sheet_hash),
        finish_sheet_hash(string_sheet_hash),
    ))
}

struct PendingRow {
    row_id: u32,
    subrow_id: u16,
    technical_payload: Vec<u8>,
    hashes: RowHashes,
    string_cells: Vec<StringCell>,
}

fn finalize_row(
    pending: PendingRow,
    sheet_name: &str,
    technical_columns: &[(u32, ColumnType)],
    columns: &[ColumnMetadata],
) -> Result<(crate::types::RowRecord, u64), HxsError> {
    let technical_cells = decode_technical_payload(&pending.technical_payload, technical_columns)?;
    let canonical_payload = encode_technical_payload(&technical_cells)?;
    if canonical_payload != pending.technical_payload {
        return Err(HxsError::data(format!(
            "technical payload is not canonical for {sheet_name}/{}/{}",
            pending.row_id, pending.subrow_id
        )));
    }

    let expected_string_columns: HashSet<u32> = columns
        .iter()
        .filter(|column| column.column_type.is_string())
        .map(|column| column.index)
        .collect();
    if pending.string_cells.len() != expected_string_columns.len()
        || pending
            .string_cells
            .iter()
            .any(|cell| !expected_string_columns.contains(&cell.column_index))
    {
        return Err(HxsError::data(format!(
            "String-cell coverage is invalid for {sheet_name}/{}/{}",
            pending.row_id, pending.subrow_id
        )));
    }
    let technical_hash = hash_row_technical(
        sheet_name,
        pending.row_id,
        pending.subrow_id,
        &technical_cells,
    )?;
    let string_hash = hash_row_strings(
        sheet_name,
        pending.row_id,
        pending.subrow_id,
        &pending.string_cells,
    )?;
    let row_hash = hash_row(
        sheet_name,
        pending.row_id,
        pending.subrow_id,
        &technical_hash,
        &string_hash,
    )?;
    require_hash(
        &pending.hashes.technical,
        &technical_hash,
        &format!(
            "technical row hash for {sheet_name}/{}/{}",
            pending.row_id, pending.subrow_id
        ),
    )?;
    require_hash(
        &pending.hashes.strings,
        &string_hash,
        &format!(
            "String row hash for {sheet_name}/{}/{}",
            pending.row_id, pending.subrow_id
        ),
    )?;
    require_hash(
        &pending.hashes.row,
        &row_hash,
        &format!(
            "row hash for {sheet_name}/{}/{}",
            pending.row_id, pending.subrow_id
        ),
    )?;
    Ok((
        crate::types::RowRecord {
            row_id: pending.row_id,
            subrow_id: pending.subrow_id,
            technical_payload: pending.technical_payload,
            hashes: pending.hashes,
        },
        u64::try_from(expected_string_columns.len()).expect("a set length fits in u64"),
    ))
}

fn validate_id(field: &str, value: &str) -> Result<(), HxsError> {
    let bytes = value.as_bytes();
    if bytes.len() != 71
        || &bytes[..7] != b"sha256:"
        || !bytes[7..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(HxsError::data(format!(
            "HXS {field} is not a canonical sha256 identifier"
        )));
    }
    Ok(())
}

fn require_hash(actual: &HxsHash, expected: &HxsHash, description: &str) -> Result<(), HxsError> {
    if actual != expected {
        return Err(HxsError::data(format!(
            "HXS {description} does not match its canonical value"
        )));
    }
    Ok(())
}

fn read_hash(row: &Row<'_>, index: usize, field: &str) -> Result<HxsHash, HxsError> {
    let bytes = read_blob(row, index, field)?;
    HxsHash::from_bytes(&bytes)
        .ok_or_else(|| HxsError::data(format!("HXS {field} must contain a 32-byte SHA-256 hash")))
}

fn read_optional_hash(
    row: &Row<'_>,
    index: usize,
    field: &str,
) -> Result<Option<HxsHash>, HxsError> {
    read_optional_blob(row, index, field)?
        .map(|bytes| {
            HxsHash::from_bytes(&bytes).ok_or_else(|| {
                HxsError::data(format!("HXS {field} must contain a 32-byte SHA-256 hash"))
            })
        })
        .transpose()
}

pub(crate) fn read_row_record(row: &Row<'_>) -> Result<crate::types::RowRecord, HxsError> {
    Ok(crate::types::RowRecord {
        row_id: read_non_negative_u32(row, 0, "rows.row_id")?,
        subrow_id: read_non_negative_u16(row, 1, "rows.subrow_id")?,
        technical_payload: read_blob(row, 2, "rows.technical_payload")?,
        hashes: RowHashes {
            row: read_hash(row, 3, "rows.row_hash")?,
            technical: read_hash(row, 4, "rows.technical_hash")?,
            strings: read_hash(row, 5, "rows.string_hash")?,
        },
    })
}

pub(crate) fn read_string_cell(row: &Row<'_>) -> Result<StringCell, HxsError> {
    let row_id = read_non_negative_u32(row, 0, "string_cells.row_id")?;
    let subrow_id = read_non_negative_u16(row, 1, "string_cells.subrow_id")?;
    let column_index = read_non_negative_u32(row, 2, "string_cells.column_index")?;
    let macro_text = read_text(row, 3, "string_cells.macro_text")?;
    let raw_value = read_optional_blob(row, 4, "string_cells.raw_value")?;
    let macro_hash = read_hash(row, 5, "string_cells.macro_hash")?;
    let raw_hash = read_optional_hash(row, 6, "string_cells.raw_hash")?;
    Ok(StringCell {
        row_id,
        subrow_id,
        column_index,
        macro_text,
        raw_value,
        hashes: StringCellHashes {
            macro_text: macro_hash,
            raw_value: raw_hash,
        },
    })
}

fn read_text(row: &Row<'_>, index: usize, field: &str) -> Result<String, HxsError> {
    match row.get_ref(index).map_err(HxsError::storage)? {
        ValueRef::Text(value) => String::from_utf8(value.to_vec())
            .map_err(|_| HxsError::data(format!("{field} is not valid UTF-8"))),
        _ => Err(HxsError::data(format!("{field} must be SQLite TEXT"))),
    }
}

fn read_blob(row: &Row<'_>, index: usize, field: &str) -> Result<Vec<u8>, HxsError> {
    match row.get_ref(index).map_err(HxsError::storage)? {
        ValueRef::Blob(value) => Ok(value.to_vec()),
        _ => Err(HxsError::data(format!("{field} must be a SQLite BLOB"))),
    }
}

fn read_optional_blob(
    row: &Row<'_>,
    index: usize,
    field: &str,
) -> Result<Option<Vec<u8>>, HxsError> {
    match row.get_ref(index).map_err(HxsError::storage)? {
        ValueRef::Null => Ok(None),
        ValueRef::Blob(value) => Ok(Some(value.to_vec())),
        _ => Err(HxsError::data(format!(
            "{field} must be a SQLite BLOB or NULL"
        ))),
    }
}

fn read_i64(row: &Row<'_>, index: usize, field: &str) -> Result<i64, HxsError> {
    match row.get_ref(index).map_err(HxsError::storage)? {
        ValueRef::Integer(value) => Ok(value),
        _ => Err(HxsError::data(format!("{field} must be a SQLite INTEGER"))),
    }
}

fn read_non_negative_u64(row: &Row<'_>, index: usize, field: &str) -> Result<u64, HxsError> {
    let value = read_i64(row, index, field)?;
    u64::try_from(value).map_err(|_| HxsError::data(format!("{field} is out of range")))
}

fn read_non_negative_u32(row: &Row<'_>, index: usize, field: &str) -> Result<u32, HxsError> {
    let value = read_i64(row, index, field)?;
    u32::try_from(value).map_err(|_| HxsError::data(format!("{field} is out of range")))
}

fn read_non_negative_u16(row: &Row<'_>, index: usize, field: &str) -> Result<u16, HxsError> {
    let value = read_i64(row, index, field)?;
    u16::try_from(value).map_err(|_| HxsError::data(format!("{field} is out of range")))
}
