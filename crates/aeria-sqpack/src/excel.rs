//! Excel sheets: the sheet list (`exd/root.exl`), sheet headers (`.exh`),
//! and row pages (`.exd`).
//!
//! Row data is big-endian. A sheet's fixed row data is `data_offset` bytes;
//! string columns hold an offset past the fixed data to a NUL-terminated
//! `SeString`. A sheet is read in the requested language when its header
//! lists it, otherwise in the language-neutral variant.

use std::sync::Arc;

use thiserror::Error;

use crate::{GameData, SqPackError};

/// A sheet language as the game numbers it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Language {
    /// Language-neutral data.
    None,
    Japanese,
    English,
    German,
    French,
    ChineseSimplified,
    ChineseTraditional,
    Korean,
    TraditionalChinese,
}

impl Language {
    fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::None,
            1 => Self::Japanese,
            2 => Self::English,
            3 => Self::German,
            4 => Self::French,
            5 => Self::ChineseSimplified,
            6 => Self::ChineseTraditional,
            7 => Self::Korean,
            8 => Self::TraditionalChinese,
            _ => return None,
        })
    }

    /// The data file suffix, such as `en`; empty for [`Language::None`].
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Japanese => "ja",
            Self::English => "en",
            Self::German => "de",
            Self::French => "fr",
            Self::ChineseSimplified => "chs",
            Self::ChineseTraditional => "cht",
            Self::Korean => "ko",
            Self::TraditionalChinese => "tc",
        }
    }
}

/// How a sheet stores rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Variant {
    /// One record per row ID.
    Default,
    /// Several records (subrows) per row ID.
    Subrows,
}

/// A column's data type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColumnKind {
    String,
    Bool,
    Int8,
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Float32,
    Int64,
    UInt64,
    /// One bit, 0 to 7, of a byte.
    PackedBool(u8),
}

impl ColumnKind {
    fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            0x0 => Self::String,
            0x1 => Self::Bool,
            0x2 => Self::Int8,
            0x3 => Self::UInt8,
            0x4 => Self::Int16,
            0x5 => Self::UInt16,
            0x6 => Self::Int32,
            0x7 => Self::UInt32,
            0x9 => Self::Float32,
            0xA => Self::Int64,
            0xB => Self::UInt64,
            0x19..=0x20 => Self::PackedBool(u8::try_from(code - 0x19).expect("bit 0 to 7")),
            _ => return None,
        })
    }
}

/// One column: its type and byte offset in the fixed row data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Column {
    pub kind: ColumnKind,
    pub offset: u16,
}

/// Why a sheet cannot be read.
#[derive(Debug, Error)]
pub enum SheetError {
    #[error("the sheet {0:?} has no header")]
    NotFound(String),
    #[error("the sheet {name:?} has the unsupported variant {variant}")]
    UnsupportedVariant { name: String, variant: u8 },
    #[error("the sheet {name:?} has the unsupported column type {code:#x}")]
    UnsupportedColumnType { name: String, code: u16 },
    #[error("the sheet {name:?} cannot be read: {message}")]
    Unreadable { name: String, message: String },
    #[error(transparent)]
    Game(#[from] SqPackError),
}

fn be_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn be_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// Lists the sheets of `exd/root.exl` that have a header, in list order.
///
/// # Errors
///
/// Returns an error when the list is missing or malformed.
pub fn sheet_names(game: &GameData) -> Result<Vec<String>, SqPackError> {
    let list = game
        .file("exd/root.exl")?
        .ok_or_else(|| SqPackError::invalid("exd/root.exl is missing"))?;
    let text = String::from_utf8_lossy(&list);
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default();
    if header.split(',').next() != Some("EXLT") {
        return Err(SqPackError::invalid("exd/root.exl is not an EXLT list"));
    }
    let mut names = Vec::new();
    for line in lines {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split(',');
        let name = fields.next().unwrap_or_default();
        let valid_id = fields
            .next()
            .is_some_and(|id| id.trim().parse::<i32>().is_ok());
        if !valid_id {
            return Err(SqPackError::invalid(format!(
                "exd/root.exl has a malformed line {line:?}"
            )));
        }
        if !names.iter().any(|known: &String| known == name)
            && game.file(&format!("exd/{name}.exh"))?.is_some()
        {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

/// A parsed sheet header.
#[derive(Clone, Debug)]
pub struct SheetHeader {
    /// Size of the fixed row data.
    pub data_offset: u16,
    /// The raw variant byte.
    pub variant: u8,
    /// Raw column type codes and offsets.
    pub columns: Vec<(u16, u16)>,
    /// First row ID and row count of each page.
    pub pages: Vec<(u32, u32)>,
    /// Raw language codes.
    pub languages: Vec<u8>,
    pub row_count: u32,
}

impl SheetHeader {
    fn parse(bytes: &[u8]) -> Option<Self> {
        if !bytes.starts_with(b"EXHF") {
            return None;
        }
        let data_offset = be_u16(bytes, 6)?;
        let column_count = usize::from(be_u16(bytes, 8)?);
        let page_count = usize::from(be_u16(bytes, 10)?);
        let language_count = usize::from(be_u16(bytes, 12)?);
        let variant = *bytes.get(17)?;
        let row_count = be_u32(bytes, 20)?;
        let mut offset = 32;
        let mut columns = Vec::with_capacity(column_count);
        for _ in 0..column_count {
            columns.push((be_u16(bytes, offset)?, be_u16(bytes, offset + 2)?));
            offset += 4;
        }
        let mut pages = Vec::with_capacity(page_count);
        for _ in 0..page_count {
            pages.push((be_u32(bytes, offset)?, be_u32(bytes, offset + 4)?));
            offset += 8;
        }
        let mut languages = Vec::with_capacity(language_count);
        for _ in 0..language_count {
            languages.push(*bytes.get(offset)?);
            // Each language is followed by a NUL-terminated parameter.
            offset += 1;
            let end = bytes.get(offset..)?.iter().position(|byte| *byte == 0)?;
            offset += end + 1;
        }
        Some(Self {
            data_offset,
            variant,
            columns,
            pages,
            languages,
            row_count,
        })
    }

    /// The language a sheet is read in: `requested` when listed, otherwise
    /// the language-neutral data when listed.
    #[must_use]
    pub fn effective_language(&self, requested: Language) -> Option<Language> {
        let listed = |language: Language| {
            self.languages
                .iter()
                .any(|code| Language::from_code(*code) == Some(language))
        };
        if listed(requested) {
            Some(requested)
        } else if listed(Language::None) {
            Some(Language::None)
        } else {
            None
        }
    }
}

/// Reads a sheet's header, or `None` when the sheet has none.
///
/// # Errors
///
/// Returns an error for an unreadable archive or a malformed header.
pub fn sheet_header(game: &GameData, name: &str) -> Result<Option<SheetHeader>, SheetError> {
    let Some(bytes) = game.file(&format!("exd/{name}.exh"))? else {
        return Ok(None);
    };
    SheetHeader::parse(&bytes)
        .map(Some)
        .ok_or_else(|| SheetError::Unreadable {
            name: name.to_owned(),
            message: "the header is malformed".to_owned(),
        })
}

/// A sheet read in one language.
pub struct Sheet {
    pub name: String,
    pub variant: Variant,
    /// The language the data was read in.
    pub language: Language,
    pub columns: Vec<Column>,
    data_offset: u16,
    pages: Vec<Arc<Vec<u8>>>,
    rows: Vec<RowLocation>,
}

#[derive(Clone, Copy)]
struct RowLocation {
    row_id: u32,
    page: usize,
    /// Offset of the row data, after its six-byte header.
    offset: usize,
    subrow_count: u16,
}

/// Reads a sheet in `language`, falling back to the language-neutral data.
///
/// Rows are read in page order and, within a page, in the order of its row
/// index. A page file that the archives do not contain is skipped.
///
/// # Errors
///
/// Returns [`SheetError`] for a missing header, an unsupported variant or
/// column type, a language the sheet does not provide, or unreadable data.
pub fn read_sheet(game: &GameData, name: &str, language: Language) -> Result<Sheet, SheetError> {
    let unreadable = |message: String| SheetError::Unreadable {
        name: name.to_owned(),
        message,
    };
    let header = sheet_header(game, name)?.ok_or_else(|| SheetError::NotFound(name.to_owned()))?;
    let variant = match header.variant {
        1 => Variant::Default,
        2 => Variant::Subrows,
        variant => {
            return Err(SheetError::UnsupportedVariant {
                name: name.to_owned(),
                variant,
            });
        }
    };
    let effective = header.effective_language(language).ok_or_else(|| {
        unreadable(format!(
            "the sheet has no {language:?} or language-neutral data"
        ))
    })?;
    let columns = header
        .columns
        .iter()
        .map(|(code, offset)| {
            ColumnKind::from_code(*code)
                .map(|kind| Column {
                    kind,
                    offset: *offset,
                })
                .ok_or(SheetError::UnsupportedColumnType {
                    name: name.to_owned(),
                    code: *code,
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut pages = Vec::new();
    let mut rows = Vec::new();
    for (start, _) in &header.pages {
        let path = if effective == Language::None {
            format!("exd/{name}_{start}.exd")
        } else {
            format!("exd/{name}_{start}_{}.exd", effective.suffix())
        };
        let Some(page) = game.file(&path)? else {
            continue;
        };
        let located = page_rows(&page, variant, pages.len())
            .map_err(|message| unreadable(format!("{path} {message}")))?;
        rows.extend(located);
        pages.push(Arc::new(page));
    }
    Ok(Sheet {
        name: name.to_owned(),
        variant,
        language: effective,
        columns,
        data_offset: header.data_offset,
        pages,
        rows,
    })
}

/// Locates the rows of one `EXDF` page from its row index.
fn page_rows(
    page: &[u8],
    variant: Variant,
    page_number: usize,
) -> Result<Vec<RowLocation>, String> {
    if !page.starts_with(b"EXDF") {
        return Err("is not an EXDF page".to_owned());
    }
    let index_size = be_u32(page, 8).ok_or("is truncated")? as usize;
    let index = page
        .get(32..32 + index_size)
        .ok_or("has a truncated row index")?;
    let mut rows = Vec::with_capacity(index.len() / 8);
    for entry in index.as_chunks::<8>().0 {
        let (row_id, offset) = entry.split_at(4);
        let row_id = be_u32(row_id, 0).ok_or("has a malformed row index")?;
        let offset = be_u32(offset, 0).ok_or("has a malformed row index")? as usize;
        // Row header: data size, then the subrow count.
        let subrow_count = be_u16(page, offset + 4).ok_or("has a truncated row")?;
        rows.push(RowLocation {
            row_id,
            page: page_number,
            offset: offset + 6,
            subrow_count: if variant == Variant::Subrows {
                subrow_count
            } else {
                1
            },
        });
    }
    Ok(rows)
}

impl Sheet {
    /// Every row, and for a subrow sheet every subrow, in read order.
    ///
    /// A subrow's ID is its position in the row. Its data is preceded by a
    /// stored ID, available as [`Row::stored_subrow_id`].
    #[must_use]
    pub fn rows(&self) -> Vec<Row<'_>> {
        let mut rows = Vec::with_capacity(self.rows.len());
        for location in &self.rows {
            let page = &self.pages[location.page];
            match self.variant {
                Variant::Default => rows.push(Row {
                    sheet: self,
                    page,
                    row_id: location.row_id,
                    subrow_id: 0,
                    offset: location.offset,
                    stored_subrow_id: None,
                }),
                Variant::Subrows => {
                    let stride = usize::from(self.data_offset) + 2;
                    for subrow in 0..location.subrow_count {
                        let start = location.offset + usize::from(subrow) * stride;
                        rows.push(Row {
                            sheet: self,
                            page,
                            row_id: location.row_id,
                            subrow_id: subrow,
                            offset: start + 2,
                            stored_subrow_id: be_u16(page, start),
                        });
                    }
                }
            }
        }
        rows
    }
}

/// One row or subrow of a sheet.
pub struct Row<'a> {
    sheet: &'a Sheet,
    page: &'a [u8],
    pub row_id: u32,
    pub subrow_id: u16,
    offset: usize,
    /// The subrow ID stored in the data, for subrow sheets.
    pub stored_subrow_id: Option<u16>,
}

/// A cell value that cannot be read.
#[derive(Clone, Debug, Eq, PartialEq, Error)]
#[error("{0}")]
pub struct CellError(pub String);

impl Row<'_> {
    fn column(&self, column: usize) -> Result<Column, CellError> {
        self.sheet
            .columns
            .get(column)
            .copied()
            .ok_or_else(|| CellError(format!("the sheet has no column {column}")))
    }

    fn bytes(&self, column: usize, length: usize) -> Result<&[u8], CellError> {
        let offset = self.offset + usize::from(self.column(column)?.offset);
        self.page
            .get(offset..offset + length)
            .ok_or_else(|| CellError(format!("column {column} is outside the page")))
    }

    /// The `SeString` bytes of a string column, without the terminator.
    ///
    /// # Errors
    ///
    /// Returns an error when the string lies outside its page or is not
    /// terminated.
    pub fn string(&self, column: usize) -> Result<&[u8], CellError> {
        if self.column(column)?.kind != ColumnKind::String {
            return Err(CellError(format!("column {column} is not a string")));
        }
        let relative = be_u32(self.bytes(column, 4)?, 0)
            .ok_or_else(|| CellError(format!("column {column} is outside the page")))?;
        let start = self.offset + usize::from(self.sheet.data_offset) + relative as usize;
        let rest = self.page.get(start..).ok_or_else(|| {
            CellError(format!("the string of column {column} is outside the page"))
        })?;
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| CellError(format!("the string of column {column} is not terminated")))?;
        Ok(&rest[..end])
    }

    /// A non-string value in canonical form: little-endian integers and
    /// float bits, and booleans as `0` or `1`.
    ///
    /// # Errors
    ///
    /// Returns an error when the value lies outside its page.
    pub fn value(&self, column: usize) -> Result<Vec<u8>, CellError> {
        let reversed = |length| -> Result<Vec<u8>, CellError> {
            let mut bytes = self.bytes(column, length)?.to_vec();
            bytes.reverse();
            Ok(bytes)
        };
        Ok(match self.column(column)?.kind {
            ColumnKind::String => {
                return Err(CellError(format!("column {column} is a string")));
            }
            ColumnKind::Bool => vec![u8::from(self.bytes(column, 1)?[0] != 0)],
            ColumnKind::Int8 | ColumnKind::UInt8 => self.bytes(column, 1)?.to_vec(),
            ColumnKind::Int16 | ColumnKind::UInt16 => reversed(2)?,
            ColumnKind::Int32 | ColumnKind::UInt32 | ColumnKind::Float32 => reversed(4)?,
            ColumnKind::Int64 | ColumnKind::UInt64 => reversed(8)?,
            ColumnKind::PackedBool(bit) => vec![(self.bytes(column, 1)?[0] >> bit) & 1],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(languages: &[u8]) -> SheetHeader {
        SheetHeader {
            data_offset: 8,
            variant: 1,
            columns: Vec::new(),
            pages: Vec::new(),
            languages: languages.to_vec(),
            row_count: 0,
        }
    }

    #[test]
    fn a_sheet_falls_back_to_language_neutral_data() {
        assert_eq!(
            header(&[1, 2, 3]).effective_language(Language::English),
            Some(Language::English)
        );
        assert_eq!(
            header(&[0]).effective_language(Language::English),
            Some(Language::None)
        );
        assert_eq!(header(&[1]).effective_language(Language::English), None);
    }

    /// A page with two rows of a sheet with one string column at offset 0
    /// and a four-byte fixed row; the second row has two subrows.
    fn subrow_sheet() -> Sheet {
        let mut page = b"EXDF".to_vec();
        page.extend_from_slice(&[0; 4]);
        page.extend_from_slice(&16_u32.to_be_bytes()); // index size
        page.resize(32, 0);
        let first = 48_u32;
        let second = first + 6 + 2 + 4 + 2;
        for (row_id, offset) in [(7_u32, first), (9, second)] {
            page.extend_from_slice(&row_id.to_be_bytes());
            page.extend_from_slice(&offset.to_be_bytes());
        }
        // Row 7: one subrow (stored ID 0), string "A" right after the fixed data.
        page.extend_from_slice(&0_u32.to_be_bytes());
        page.extend_from_slice(&1_u16.to_be_bytes());
        page.extend_from_slice(&0_u16.to_be_bytes());
        page.extend_from_slice(&0_u32.to_be_bytes());
        page.extend_from_slice(b"A ");
        // Row 9: subrows with stored IDs 0 and 1, strings "B" and "C" after
        // both fixed records. A string offset counts from the end of its
        // own subrow's fixed data.
        page.extend_from_slice(&0_u32.to_be_bytes());
        page.extend_from_slice(&2_u16.to_be_bytes());
        page.extend_from_slice(&0_u16.to_be_bytes());
        page.extend_from_slice(&6_u32.to_be_bytes());
        page.extend_from_slice(&1_u16.to_be_bytes());
        page.extend_from_slice(&2_u32.to_be_bytes());
        page.extend_from_slice(b"B C ");
        let rows = page_rows(&page, Variant::Subrows, 0).expect("rows");
        Sheet {
            name: "Test".to_owned(),
            variant: Variant::Subrows,
            language: Language::None,
            columns: vec![Column {
                kind: ColumnKind::String,
                offset: 0,
            }],
            data_offset: 4,
            pages: vec![Arc::new(page)],
            rows,
        }
    }

    #[test]
    fn subrows_are_numbered_by_position_and_read_their_strings() {
        let sheet = subrow_sheet();
        let rows = sheet.rows();
        let read: Vec<(u32, u16, &[u8])> = rows
            .iter()
            .map(|row| (row.row_id, row.subrow_id, row.string(0).expect("string")))
            .collect();
        assert_eq!(
            read,
            [
                (7, 0, b"A".as_slice()),
                (9, 0, b"B".as_slice()),
                (9, 1, b"C".as_slice())
            ]
        );
        assert_eq!(rows[2].stored_subrow_id, Some(1));
        assert!(rows[0].string(1).is_err());
        assert!(rows[0].value(0).is_err());
    }

    #[test]
    fn pages_without_the_magic_or_with_a_short_index_are_rejected() {
        assert!(page_rows(b"EXDX", Variant::Default, 0).is_err());
        let mut page = b"EXDF".to_vec();
        page.extend_from_slice(&[0; 4]);
        page.extend_from_slice(&64_u32.to_be_bytes());
        page.resize(40, 0);
        assert!(page_rows(&page, Variant::Default, 0).is_err());
    }

    #[test]
    fn headers_parse_columns_pages_and_languages() {
        let mut bytes = b"EXHF".to_vec();
        bytes.extend_from_slice(&3_u16.to_be_bytes()); // version
        bytes.extend_from_slice(&8_u16.to_be_bytes()); // data offset
        bytes.extend_from_slice(&2_u16.to_be_bytes()); // columns
        bytes.extend_from_slice(&1_u16.to_be_bytes()); // pages
        bytes.extend_from_slice(&2_u16.to_be_bytes()); // languages
        bytes.extend_from_slice(&[0, 0, 0, 2, 0, 0]); // unknown, variant, unknown
        bytes.extend_from_slice(&5_u32.to_be_bytes()); // rows
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0x19, 0, 4]);
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 5]);
        bytes.extend_from_slice(&[1, 0, 2, 0]);
        let parsed = SheetHeader::parse(&bytes).expect("header");
        assert_eq!(parsed.data_offset, 8);
        assert_eq!(parsed.variant, 2);
        assert_eq!(parsed.columns, [(0, 0), (0x19, 4)]);
        assert_eq!(parsed.pages, [(0, 5)]);
        assert_eq!(parsed.languages, [1, 2]);
        assert_eq!(ColumnKind::from_code(0x19), Some(ColumnKind::PackedBool(0)));
        assert_eq!(ColumnKind::from_code(0x8), None);
    }
}
