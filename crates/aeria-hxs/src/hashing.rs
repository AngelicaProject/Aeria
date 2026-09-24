use std::cmp::Ordering;

use sha2::{Digest, Sha256};

use crate::error::HxsError;
use crate::types::{ColumnType, ExcludedSheet, HxsHash, SheetVariant, StringCell};

pub(crate) struct TechnicalCell {
    pub column_index: u32,
    pub column_type: ColumnType,
    pub value: Vec<u8>,
}

pub(crate) fn ordinal_cmp(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

pub(crate) struct CanonicalHasher(Sha256);

impl CanonicalHasher {
    fn new() -> Self {
        Self(Sha256::new())
    }

    fn byte(&mut self, value: u8) {
        self.0.update([value]);
    }

    fn u32(&mut self, value: u32) {
        self.0.update(value.to_le_bytes());
    }

    fn utf8(&mut self, value: &str) -> Result<(), HxsError> {
        let length = u32::try_from(value.len())
            .map_err(|_| HxsError::data("UTF-8 value exceeds HXS framing limits"))?;
        self.u32(length);
        self.0.update(value.as_bytes());
        Ok(())
    }

    fn bytes(&mut self, value: &[u8]) -> Result<(), HxsError> {
        let length = u32::try_from(value.len())
            .map_err(|_| HxsError::data("byte value exceeds HXS framing limits"))?;
        self.u32(length);
        self.0.update(value);
        Ok(())
    }

    fn hash(&mut self, value: &HxsHash) {
        self.0.update(value.as_bytes());
    }

    fn finish(self) -> HxsHash {
        HxsHash::from_array(self.0.finalize().into())
    }
}

fn domain(hasher: &mut CanonicalHasher, value: &str) {
    hasher.0.update(value.as_bytes());
}

fn row_identity(
    hasher: &mut CanonicalHasher,
    sheet_name: &str,
    row_id: u32,
    subrow_id: u16,
) -> Result<(), HxsError> {
    hasher.utf8(sheet_name)?;
    hasher.u32(row_id);
    hasher.u32(u32::from(subrow_id));
    Ok(())
}

fn write_technical_cell(
    hasher: &mut CanonicalHasher,
    cell: &TechnicalCell,
) -> Result<(), HxsError> {
    hasher.u32(cell.column_index);
    hasher.u32(cell.column_type.code());
    hasher.bytes(&cell.value)
}

pub(crate) fn encode_technical_payload(cells: &[TechnicalCell]) -> Result<Vec<u8>, HxsError> {
    let mut bytes = Vec::new();
    for cell in cells {
        bytes.extend(cell.column_index.to_le_bytes());
        bytes.extend(cell.column_type.code().to_le_bytes());
        let length = u32::try_from(cell.value.len())
            .map_err(|_| HxsError::data("technical value exceeds HXS framing limits"))?;
        bytes.extend(length.to_le_bytes());
        bytes.extend(&cell.value);
    }
    Ok(bytes)
}

pub(crate) fn decode_technical_payload(
    payload: &[u8],
    columns: &[(u32, ColumnType)],
) -> Result<Vec<TechnicalCell>, HxsError> {
    let mut cells = Vec::new();
    let mut offset = 0usize;
    let mut previous_column = None;

    while offset < payload.len() {
        let column_index = read_u32(payload, &mut offset, "technical column index")?;
        let type_code = read_u32(payload, &mut offset, "technical type code")?;
        let value_length = read_u32(payload, &mut offset, "technical value length")?;
        let column_type = ColumnType::from_code(i64::from(type_code)).ok_or_else(|| {
            HxsError::data(format!("unsupported technical type code {type_code}"))
        })?;
        if previous_column.is_some_and(|previous| column_index <= previous) {
            return Err(HxsError::data(
                "technical payload columns are not strictly ordered",
            ));
        }
        let Some((_, expected_type)) = columns.iter().find(|(index, _)| *index == column_index)
        else {
            return Err(HxsError::data(format!(
                "technical payload references unknown column {column_index}"
            )));
        };
        if *expected_type != column_type || column_type.is_string() {
            return Err(HxsError::data(format!(
                "technical payload type does not match column {column_index}"
            )));
        }
        let expected_length = column_type.technical_value_length();
        let value_length = usize::try_from(value_length)
            .map_err(|_| HxsError::data("technical value length is out of range"))?;
        if value_length != expected_length || value_length > payload.len() - offset {
            return Err(HxsError::data(format!(
                "technical payload length is invalid for column {column_index}"
            )));
        }
        let value = payload[offset..offset + value_length].to_vec();
        offset += value_length;
        if is_boolean(column_type) && value[0] > 1 {
            return Err(HxsError::data(
                "boolean technical payload values must be 0x00 or 0x01",
            ));
        }
        cells.push(TechnicalCell {
            column_index,
            column_type,
            value,
        });
        previous_column = Some(column_index);
    }

    let expected_count = columns
        .iter()
        .filter(|(_, column_type)| !column_type.is_string())
        .count();
    if cells.len() != expected_count {
        return Err(HxsError::data(
            "technical payload does not contain every non-String column exactly once",
        ));
    }
    Ok(cells)
}

pub(crate) fn hash_macro(macro_text: &str) -> HxsHash {
    let mut hasher = CanonicalHasher::new();
    domain(&mut hasher, "HARMONIA-HXS-V1-MACRO");
    hasher
        .utf8(macro_text)
        .expect("a Rust string cannot exceed the HXS framing limit");
    hasher.finish()
}

pub(crate) fn hash_raw(raw_value: &[u8]) -> Result<HxsHash, HxsError> {
    let mut hasher = CanonicalHasher::new();
    domain(&mut hasher, "HARMONIA-HXS-V1-RAW-STRING");
    hasher.bytes(raw_value)?;
    Ok(hasher.finish())
}

pub(crate) fn hash_row_technical(
    sheet_name: &str,
    row_id: u32,
    subrow_id: u16,
    cells: &[TechnicalCell],
) -> Result<HxsHash, HxsError> {
    let mut hasher = CanonicalHasher::new();
    domain(&mut hasher, "HARMONIA-HXS-V1-ROW-TECHNICAL");
    row_identity(&mut hasher, sheet_name, row_id, subrow_id)?;
    for cell in cells {
        write_technical_cell(&mut hasher, cell)?;
    }
    Ok(hasher.finish())
}

pub(crate) fn hash_row_strings(
    sheet_name: &str,
    row_id: u32,
    subrow_id: u16,
    cells: &[StringCell],
) -> Result<HxsHash, HxsError> {
    let mut hasher = CanonicalHasher::new();
    domain(&mut hasher, "HARMONIA-HXS-V1-ROW-STRINGS");
    row_identity(&mut hasher, sheet_name, row_id, subrow_id)?;
    for cell in cells {
        hasher.u32(cell.column_index);
        hasher.hash(&cell.hashes.macro_text);
        hasher.byte(u8::from(cell.hashes.raw_value.is_some()));
        if let Some(raw_hash) = &cell.hashes.raw_value {
            hasher.hash(raw_hash);
        }
    }
    Ok(hasher.finish())
}

pub(crate) fn hash_row(
    sheet_name: &str,
    row_id: u32,
    subrow_id: u16,
    technical_hash: &HxsHash,
    string_hash: &HxsHash,
) -> Result<HxsHash, HxsError> {
    let mut hasher = CanonicalHasher::new();
    domain(&mut hasher, "HARMONIA-HXS-V1-ROW");
    row_identity(&mut hasher, sheet_name, row_id, subrow_id)?;
    hasher.hash(technical_hash);
    hasher.hash(string_hash);
    Ok(hasher.finish())
}

pub(crate) fn hash_schema(
    sheet_name: &str,
    variant: SheetVariant,
    columns: &[(u32, u32, ColumnType)],
) -> Result<HxsHash, HxsError> {
    let mut hasher = CanonicalHasher::new();
    domain(&mut hasher, "HARMONIA-HXS-V1-SCHEMA");
    hasher.utf8(sheet_name)?;
    hasher.u32(variant.code());
    for (index, offset, column_type) in columns {
        hasher.u32(*index);
        hasher.u32(*offset);
        hasher.u32(column_type.code());
    }
    Ok(hasher.finish())
}

pub(crate) fn begin_sheet_hash(
    domain_name: &str,
    sheet_name: &str,
) -> Result<CanonicalHasher, HxsError> {
    let mut hasher = CanonicalHasher::new();
    domain(&mut hasher, domain_name);
    hasher.utf8(sheet_name)?;
    Ok(hasher)
}

pub(crate) fn add_sheet_row(
    hasher: &mut CanonicalHasher,
    row_id: u32,
    subrow_id: u16,
    row_hash: &HxsHash,
) {
    hasher.u32(row_id);
    hasher.u32(u32::from(subrow_id));
    hasher.hash(row_hash);
}

pub(crate) fn finish_sheet_hash(hasher: CanonicalHasher) -> HxsHash {
    hasher.finish()
}

pub(crate) fn hash_sheet_content(
    sheet_name: &str,
    variant: SheetVariant,
    schema_hash: &HxsHash,
    technical_hash: &HxsHash,
    string_hash: &HxsHash,
) -> Result<HxsHash, HxsError> {
    let mut hasher = CanonicalHasher::new();
    domain(&mut hasher, "HARMONIA-HXS-V1-SHEET");
    hasher.utf8(sheet_name)?;
    hasher.u32(variant.code());
    hasher.hash(schema_hash);
    hasher.hash(technical_hash);
    hasher.hash(string_hash);
    Ok(hasher.finish())
}

/// Computes `contentId`. `sheets` and `excluded` must be ordered by ordinal
/// name. HXS v1 has no exclusions and `excluded` must be `None` for it.
pub(crate) fn compute_content_id(
    language: &str,
    sheets: &[(&str, &str, &HxsHash, &HxsHash)],
    excluded: Option<&[ExcludedSheet]>,
) -> Result<String, HxsError> {
    let mut hasher = CanonicalHasher::new();
    match excluded {
        None => domain(&mut hasher, "HARMONIA-HXS-CONTENT-v1"),
        Some(_) => domain(&mut hasher, "HARMONIA-HXS-CONTENT-v2"),
    }
    hasher.utf8(language)?;
    if excluded.is_some() {
        hasher.u32(count_u32(sheets.len())?);
    }
    for (name, effective_language, schema_hash, content_hash) in sheets {
        hasher.utf8(name)?;
        hasher.utf8(effective_language)?;
        hasher.hash(schema_hash);
        hasher.hash(content_hash);
    }
    if let Some(excluded) = excluded {
        hasher.u32(count_u32(excluded.len())?);
        for sheet in excluded {
            hasher.utf8(&sheet.name)?;
            hasher.u32(sheet.reason.code());
        }
    }
    Ok(format!("sha256:{}", hasher.finish()))
}

fn count_u32(count: usize) -> Result<u32, HxsError> {
    u32::try_from(count).map_err(|_| HxsError::data("sheet count exceeds HXS framing limits"))
}

pub(crate) fn compute_snapshot_id(
    game_version: &str,
    language: &str,
    content_id: &str,
) -> Result<String, HxsError> {
    let mut hasher = CanonicalHasher::new();
    domain(&mut hasher, "HARMONIA-HXS-SNAPSHOT-v1");
    hasher.utf8(game_version)?;
    hasher.utf8(language)?;
    hasher.utf8(content_id)?;
    Ok(format!("sha256:{}", hasher.finish()))
}

fn read_u32(payload: &[u8], offset: &mut usize, field: &str) -> Result<u32, HxsError> {
    if payload.len() - *offset < 4 {
        return Err(HxsError::data(format!(
            "technical payload is truncated at {field}"
        )));
    }
    let value = u32::from_le_bytes(
        payload[*offset..*offset + 4]
            .try_into()
            .expect("slice length is checked"),
    );
    *offset += 4;
    Ok(value)
}

fn is_boolean(column_type: ColumnType) -> bool {
    matches!(
        column_type,
        ColumnType::Bool
            | ColumnType::PackedBool0
            | ColumnType::PackedBool1
            | ColumnType::PackedBool2
            | ColumnType::PackedBool3
            | ColumnType::PackedBool4
            | ColumnType::PackedBool5
            | ColumnType::PackedBool6
            | ColumnType::PackedBool7
    )
}
