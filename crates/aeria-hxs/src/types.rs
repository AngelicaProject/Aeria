use std::fmt;
use std::fmt::Write as _;

/// A verified SHA-256 digest stored in an HXS artifact.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct HxsHash([u8; 32]);

impl HxsHash {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        bytes.try_into().ok().map(Self)
    }

    pub(crate) fn from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the digest bytes in their canonical order.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the lowercase hexadecimal digest without a prefix.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut result = String::with_capacity(64);
        for byte in self.0 {
            write!(&mut result, "{byte:02x}").expect("writing to a String cannot fail");
        }
        result
    }
}

impl fmt::Display for HxsHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

/// HXS-owned sheet variant codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SheetVariant {
    DefaultRows,
    Subrows,
}

impl SheetVariant {
    pub(crate) fn from_code(code: i64) -> Option<Self> {
        match code {
            0 => Some(Self::DefaultRows),
            1 => Some(Self::Subrows),
            _ => None,
        }
    }

    pub(crate) const fn code(self) -> u32 {
        match self {
            Self::DefaultRows => 0,
            Self::Subrows => 1,
        }
    }
}

/// HXS-owned column type codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColumnType {
    String,
    Bool,
    Int8,
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    Float32,
    PackedBool0,
    PackedBool1,
    PackedBool2,
    PackedBool3,
    PackedBool4,
    PackedBool5,
    PackedBool6,
    PackedBool7,
}

impl ColumnType {
    pub(crate) fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            1 => Self::String,
            2 => Self::Bool,
            10 => Self::Int8,
            11 => Self::UInt8,
            12 => Self::Int16,
            13 => Self::UInt16,
            14 => Self::Int32,
            15 => Self::UInt32,
            16 => Self::Int64,
            17 => Self::UInt64,
            20 => Self::Float32,
            30 => Self::PackedBool0,
            31 => Self::PackedBool1,
            32 => Self::PackedBool2,
            33 => Self::PackedBool3,
            34 => Self::PackedBool4,
            35 => Self::PackedBool5,
            36 => Self::PackedBool6,
            37 => Self::PackedBool7,
            _ => return None,
        })
    }

    pub(crate) const fn code(self) -> u32 {
        match self {
            Self::String => 1,
            Self::Bool => 2,
            Self::Int8 => 10,
            Self::UInt8 => 11,
            Self::Int16 => 12,
            Self::UInt16 => 13,
            Self::Int32 => 14,
            Self::UInt32 => 15,
            Self::Int64 => 16,
            Self::UInt64 => 17,
            Self::Float32 => 20,
            Self::PackedBool0 => 30,
            Self::PackedBool1 => 31,
            Self::PackedBool2 => 32,
            Self::PackedBool3 => 33,
            Self::PackedBool4 => 34,
            Self::PackedBool5 => 35,
            Self::PackedBool6 => 36,
            Self::PackedBool7 => 37,
        }
    }

    pub(crate) const fn technical_value_length(self) -> usize {
        match self {
            Self::String => 0,
            Self::Bool
            | Self::Int8
            | Self::UInt8
            | Self::PackedBool0
            | Self::PackedBool1
            | Self::PackedBool2
            | Self::PackedBool3
            | Self::PackedBool4
            | Self::PackedBool5
            | Self::PackedBool6
            | Self::PackedBool7 => 1,
            Self::Int16 | Self::UInt16 => 2,
            Self::Int32 | Self::UInt32 | Self::Float32 => 4,
            Self::Int64 | Self::UInt64 => 8,
        }
    }

    pub(crate) const fn is_string(self) -> bool {
        matches!(self, Self::String)
    }
}

/// Producer provenance recorded by Atlas. These values do not participate in content identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProducerMetadata {
    pub extractor_version: String,
    pub lumina_version: String,
}

/// Aggregate counts recorded in the snapshot metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotCounts {
    pub sheets: u64,
    pub rows: u64,
    pub string_cells: u64,
}

/// Verified HXS snapshot metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotMetadata {
    pub format_version: u32,
    pub game_version: String,
    pub source_language: String,
    pub scope: String,
    pub content_id: String,
    pub snapshot_id: String,
    pub producer: ProducerMetadata,
    pub counts: SnapshotCounts,
}

/// A column in a verified HXS sheet schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColumnMetadata {
    pub index: u32,
    pub offset: u32,
    pub column_type: ColumnType,
}

/// The four canonical hashes stored for a sheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SheetHashes {
    pub schema: HxsHash,
    pub technical: HxsHash,
    pub strings: HxsHash,
    pub content: HxsHash,
}

/// Verified sheet metadata, including its source schema and hashes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SheetMetadata {
    pub name: String,
    pub variant: SheetVariant,
    pub effective_language: String,
    pub columns: Vec<ColumnMetadata>,
    pub row_count: u64,
    pub hashes: SheetHashes,
}

/// The three canonical hashes stored for a row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowHashes {
    pub row: HxsHash,
    pub technical: HxsHash,
    pub strings: HxsHash,
}

/// One source row returned by the reader.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowRecord {
    pub row_id: u32,
    pub subrow_id: u16,
    pub technical_payload: Vec<u8>,
    pub hashes: RowHashes,
}

/// The two canonical hashes stored for a String cell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StringCellHashes {
    pub macro_text: HxsHash,
    pub raw_value: Option<HxsHash>,
}

/// One source String cell, including both source representations when available.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StringCell {
    pub row_id: u32,
    pub subrow_id: u16,
    pub column_index: u32,
    pub macro_text: String,
    pub raw_value: Option<Vec<u8>>,
    pub hashes: StringCellHashes,
}

/// A bounded page of rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowPage {
    pub offset: u64,
    pub rows: Vec<RowRecord>,
    pub next_offset: Option<u64>,
    pub total_rows: u64,
}
