//! Writes small synthetic game installations for tests.
//!
//! [`FakeGame`] produces a folder with `game/ffxivgame.ver` and a
//! `game/sqpack/ffxiv` Excel archive that [`crate::GameData`] reads like the
//! real game: the sheet list, one header per sheet, and one page per sheet
//! and language. Data blocks are stored without compression.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use crate::excel::{ColumnKind, Language, Variant};
use crate::path_hash;

/// One cell value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FakeValue {
    /// `SeString` bytes without the terminator.
    String(Vec<u8>),
    UInt32(u32),
}

impl From<&str> for FakeValue {
    fn from(text: &str) -> Self {
        Self::String(text.as_bytes().to_vec())
    }
}

impl From<u32> for FakeValue {
    fn from(value: u32) -> Self {
        Self::UInt32(value)
    }
}

/// One row: its ID and one value list per subrow. A default sheet has
/// exactly one subrow per row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeRow {
    pub row_id: u32,
    pub subrows: Vec<Vec<FakeValue>>,
}

impl FakeRow {
    /// A row of a default sheet.
    #[must_use]
    pub fn new(row_id: u32, values: Vec<FakeValue>) -> Self {
        Self {
            row_id,
            subrows: vec![values],
        }
    }
}

/// One sheet: its columns and its rows per language. [`Language::None`]
/// stands for language-neutral data.
#[derive(Clone, Debug)]
pub struct FakeSheet {
    pub variant: Variant,
    /// Column types; only [`ColumnKind::String`] and
    /// [`ColumnKind::UInt32`] are supported. Offsets are assigned in order.
    pub columns: Vec<ColumnKind>,
    /// Explicit column offsets, overriding the assigned ones.
    pub offsets: Option<Vec<u16>>,
    pub rows: BTreeMap<Language, Vec<FakeRow>>,
}

impl FakeSheet {
    /// A default sheet with the given column types and no data.
    #[must_use]
    pub fn new(columns: Vec<ColumnKind>) -> Self {
        Self {
            variant: Variant::Default,
            columns,
            offsets: None,
            rows: BTreeMap::new(),
        }
    }

    /// Sets the rows of one language.
    #[must_use]
    pub fn with_rows(mut self, language: Language, rows: Vec<FakeRow>) -> Self {
        self.rows.insert(language, rows);
        self
    }

    fn offsets(&self) -> Vec<u16> {
        self.offsets.clone().unwrap_or_else(|| {
            (0..self.columns.len())
                .map(|index| u16::try_from(index * 4).expect("few columns"))
                .collect()
        })
    }

    fn data_offset(&self) -> u16 {
        self.offsets()
            .iter()
            .map(|offset| offset + 4)
            .max()
            .unwrap_or_default()
    }
}

/// A synthetic installation.
#[derive(Clone, Debug)]
pub struct FakeGame {
    pub version: String,
    pub sheets: BTreeMap<String, FakeSheet>,
    /// Sheets listed in `root.exl` without a header.
    pub headerless: Vec<String>,
    /// Other files by game path, such as `game_script/…/x.luab`.
    pub files: BTreeMap<String, Vec<u8>>,
}

impl FakeGame {
    /// An installation of `version` without sheets.
    #[must_use]
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            sheets: BTreeMap::new(),
            headerless: Vec::new(),
            files: BTreeMap::new(),
        }
    }

    /// Adds or replaces a file outside `exd/`.
    #[must_use]
    pub fn with_file(mut self, path: impl Into<String>, bytes: Vec<u8>) -> Self {
        self.files.insert(path.into(), bytes);
        self
    }

    /// Adds or replaces a sheet.
    #[must_use]
    pub fn with_sheet(mut self, name: impl Into<String>, sheet: FakeSheet) -> Self {
        self.sheets.insert(name.into(), sheet);
        self
    }

    /// Writes the installation into `folder`, replacing an earlier one.
    ///
    /// # Errors
    ///
    /// Returns an error when a file cannot be written.
    ///
    /// # Panics
    ///
    /// Panics when a sheet's values do not match its columns.
    pub fn write(&self, folder: &Path) -> io::Result<()> {
        let game = folder.join("game");
        let archive = game.join("sqpack").join("ffxiv");
        std::fs::create_dir_all(&archive)?;
        std::fs::write(game.join("ffxivgame.ver"), &self.version)?;

        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        let mut list = String::from("EXLT,2\n");
        for name in self.sheets.keys().chain(&self.headerless) {
            list.push_str(name);
            list.push_str(",-1\n");
        }
        files.push(("exd/root.exl".to_owned(), list.into_bytes()));
        for (name, sheet) in &self.sheets {
            files.push((format!("exd/{}.exh", name.to_lowercase()), header(sheet)));
            for (language, rows) in &sheet.rows {
                let path = if *language == Language::None {
                    format!("exd/{}_0.exd", name.to_lowercase())
                } else {
                    format!("exd/{}_0_{}.exd", name.to_lowercase(), language.suffix())
                };
                files.push((path, page(sheet, rows)));
            }
        }

        files.extend(
            self.files
                .iter()
                .map(|(path, bytes)| (path.to_lowercase(), bytes.clone())),
        );

        let mut archives: BTreeMap<u8, Vec<(String, Vec<u8>)>> = BTreeMap::new();
        for (path, bytes) in files {
            let category = path
                .split('/')
                .next()
                .and_then(crate::category_id)
                .expect("files belong to a known category");
            archives.entry(category).or_default().push((path, bytes));
        }
        for (category, files) in archives {
            let mut data = vec![0_u8; 0x800];
            let mut entries: Vec<(u64, u32)> = Vec::new();
            for (path, bytes) in &files {
                let offset = data.len();
                data.extend_from_slice(&standard_file(bytes));
                data.resize(data.len().next_multiple_of(128), 0);
                let hash = path_hash(path).expect("paths have a folder");
                let index_data = u32::try_from(offset / 8).expect("small archive");
                entries.push((hash, index_data));
            }
            let prefix = format!("{category:02x}0000.win32");
            std::fs::write(archive.join(format!("{prefix}.dat0")), &data)?;
            std::fs::write(archive.join(format!("{prefix}.index")), index(&entries))?;
        }
        Ok(())
    }
}

fn be16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn be32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn column_code(kind: ColumnKind) -> u16 {
    match kind {
        ColumnKind::String => 0x0,
        ColumnKind::UInt32 => 0x7,
        other => panic!("fake sheets do not support {other:?} columns"),
    }
}

fn header(sheet: &FakeSheet) -> Vec<u8> {
    let row_count = sheet.rows.values().map(Vec::len).max().unwrap_or_default();
    let mut bytes = b"EXHF".to_vec();
    be16(&mut bytes, 3);
    be16(&mut bytes, sheet.data_offset());
    be16(
        &mut bytes,
        u16::try_from(sheet.columns.len()).expect("few columns"),
    );
    be16(&mut bytes, 1);
    be16(
        &mut bytes,
        u16::try_from(sheet.rows.len()).expect("few languages"),
    );
    bytes.extend_from_slice(&[0, 0, 0]);
    bytes.push(match sheet.variant {
        Variant::Default => 1,
        Variant::Subrows => 2,
    });
    bytes.extend_from_slice(&[0, 0]);
    be32(&mut bytes, u32::try_from(row_count).expect("few rows"));
    bytes.resize(32, 0);
    for (kind, offset) in sheet.columns.iter().zip(sheet.offsets()) {
        be16(&mut bytes, column_code(*kind));
        be16(&mut bytes, offset);
    }
    be32(&mut bytes, 0);
    be32(&mut bytes, u32::try_from(row_count).expect("few rows"));
    for language in sheet.rows.keys() {
        bytes.push(language_code(*language));
        bytes.push(0);
    }
    bytes
}

fn language_code(language: Language) -> u8 {
    match language {
        Language::None => 0,
        Language::Japanese => 1,
        Language::English => 2,
        Language::German => 3,
        Language::French => 4,
        Language::ChineseSimplified => 5,
        Language::ChineseTraditional => 6,
        Language::Korean => 7,
        Language::TraditionalChinese => 8,
    }
}

/// One subrow's fixed data and the strings that follow it.
fn record(sheet: &FakeSheet, values: &[FakeValue], strings: &mut Vec<u8>) -> Vec<u8> {
    assert_eq!(values.len(), sheet.columns.len(), "one value per column");
    let mut fixed = vec![0_u8; usize::from(sheet.data_offset())];
    for ((kind, offset), value) in sheet.columns.iter().zip(sheet.offsets()).zip(values) {
        let offset = usize::from(offset);
        let word = match (kind, value) {
            (ColumnKind::String, FakeValue::String(text)) => {
                let relative = u32::try_from(strings.len()).expect("small row");
                strings.extend_from_slice(text);
                strings.push(0);
                relative
            }
            (ColumnKind::UInt32, FakeValue::UInt32(number)) => *number,
            _ => panic!("value {value:?} does not match column {kind:?}"),
        };
        fixed[offset..offset + 4].copy_from_slice(&word.to_be_bytes());
    }
    fixed
}

fn page(sheet: &FakeSheet, rows: &[FakeRow]) -> Vec<u8> {
    let index_size = rows.len() * 8;
    let mut body = Vec::new();
    let mut index = Vec::new();
    for row in rows {
        let offset = 32 + index_size + body.len();
        be32(&mut index, row.row_id);
        be32(&mut index, u32::try_from(offset).expect("small page"));
        let mut data = Vec::new();
        match sheet.variant {
            Variant::Default => {
                assert_eq!(row.subrows.len(), 1, "a default row has one record");
                let mut strings = Vec::new();
                data.extend_from_slice(&record(sheet, &row.subrows[0], &mut strings));
                data.extend_from_slice(&strings);
            }
            Variant::Subrows => {
                // Each subrow's string offsets count from the end of its own
                // fixed data, and every string follows the last subrow.
                let stride = usize::from(sheet.data_offset()) + 2;
                let mut strings = Vec::new();
                for (position, values) in row.subrows.iter().enumerate() {
                    let behind = (row.subrows.len() - position - 1) * stride;
                    let mut own = Vec::new();
                    let start = strings.len();
                    let fixed = record(sheet, values, &mut own);
                    let shifted = shift_strings(sheet, &fixed, behind + start);
                    be16(&mut data, u16::try_from(position).expect("few subrows"));
                    data.extend_from_slice(&shifted);
                    strings.extend_from_slice(&own);
                }
                data.extend_from_slice(&strings);
            }
        }
        be32(&mut body, u32::try_from(data.len()).expect("small row"));
        be16(
            &mut body,
            u16::try_from(row.subrows.len()).expect("few subrows"),
        );
        body.extend_from_slice(&data);
    }
    let mut bytes = b"EXDF".to_vec();
    be16(&mut bytes, 2);
    be16(&mut bytes, 0);
    be32(&mut bytes, u32::try_from(index_size).expect("small index"));
    be32(&mut bytes, u32::try_from(body.len()).expect("small page"));
    bytes.resize(32, 0);
    bytes.extend_from_slice(&index);
    bytes.extend_from_slice(&body);
    bytes
}

/// Adds `shift` to every string offset of one subrow's fixed data.
fn shift_strings(sheet: &FakeSheet, fixed: &[u8], shift: usize) -> Vec<u8> {
    let mut fixed = fixed.to_vec();
    for (kind, offset) in sheet.columns.iter().zip(sheet.offsets()) {
        if *kind == ColumnKind::String {
            let offset = usize::from(offset);
            let mut word = [0; 4];
            word.copy_from_slice(&fixed[offset..offset + 4]);
            let value = u32::from_be_bytes(word) + u32::try_from(shift).expect("small row");
            fixed[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }
    }
    fixed
}

/// A standard file with uncompressed blocks of at most 16,000 bytes.
fn standard_file(bytes: &[u8]) -> Vec<u8> {
    let blocks: Vec<&[u8]> = if bytes.is_empty() {
        Vec::new()
    } else {
        bytes.chunks(16_000).collect()
    };
    let header_size = (24 + blocks.len() * 8).next_multiple_of(128);
    let mut file = Vec::new();
    for value in [
        u32::try_from(header_size).expect("small header"),
        2,
        u32::try_from(bytes.len()).expect("small file"),
        0,
        0,
        u32::try_from(blocks.len()).expect("few blocks"),
    ] {
        file.extend_from_slice(&value.to_le_bytes());
    }
    let mut block_offset = 0_usize;
    for block in &blocks {
        file.extend_from_slice(
            &u32::try_from(block_offset)
                .expect("small file")
                .to_le_bytes(),
        );
        file.extend_from_slice(&[0; 4]);
        block_offset += 16 + block.len();
    }
    file.resize(header_size, 0);
    for block in blocks {
        for value in [
            16,
            0,
            32_000,
            u32::try_from(block.len()).expect("small block"),
        ] {
            file.extend_from_slice(&u32::to_le_bytes(value));
        }
        file.extend_from_slice(block);
    }
    file
}

fn index(entries: &[(u64, u32)]) -> Vec<u8> {
    let mut bytes = vec![0_u8; 0x800];
    bytes[..8].copy_from_slice(b"SqPack\0\0");
    bytes[12..16].copy_from_slice(&0x400_u32.to_le_bytes());
    bytes[0x408..0x40C].copy_from_slice(&0x800_u32.to_le_bytes());
    let table_size = u32::try_from(entries.len() * 16).expect("few files");
    bytes[0x40C..0x410].copy_from_slice(&table_size.to_le_bytes());
    for (hash, data) in entries {
        bytes.extend_from_slice(&hash.to_le_bytes());
        bytes.extend_from_slice(&data.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
    }
    bytes
}

/// A sheet of translatable text for tests: English text as given, and in
/// Japanese, German, and French the same text followed by ` [ja]`, ` [de]`,
/// or ` [fr]`, so it differs by language. Key columns hold the same text in
/// every language. Cells a row does not set are empty strings, and
/// non-String columns hold `0`.
#[derive(Clone, Debug)]
pub struct TextSheet {
    pub kinds: Vec<ColumnKind>,
    pub key_columns: Vec<usize>,
    pub rows: Vec<(u32, BTreeMap<usize, Vec<u8>>)>,
}

impl TextSheet {
    /// A sheet of `width` columns whose String columns are `string_columns`;
    /// the others are [`ColumnKind::UInt32`].
    #[must_use]
    pub fn new(width: usize, string_columns: &[usize]) -> Self {
        Self {
            kinds: (0..width)
                .map(|index| {
                    if string_columns.contains(&index) {
                        ColumnKind::String
                    } else {
                        ColumnKind::UInt32
                    }
                })
                .collect(),
            key_columns: Vec::new(),
            rows: Vec::new(),
        }
    }

    /// Makes `column` the same in every language.
    #[must_use]
    pub fn keyed(mut self, column: usize) -> Self {
        self.key_columns.push(column);
        self
    }

    /// Adds a row with texts per column.
    #[must_use]
    pub fn row(mut self, row_id: u32, cells: &[(usize, &str)]) -> Self {
        self.rows.push((
            row_id,
            cells
                .iter()
                .map(|(column, text)| (*column, text.as_bytes().to_vec()))
                .collect(),
        ));
        self
    }

    /// Adds a row with raw string bytes in one column.
    #[must_use]
    pub fn row_bytes(mut self, row_id: u32, column: usize, bytes: &[u8]) -> Self {
        self.rows
            .push((row_id, BTreeMap::from([(column, bytes.to_vec())])));
        self
    }

    /// The sheet in the four global languages.
    #[must_use]
    pub fn fake(&self) -> FakeSheet {
        let mut sheet = FakeSheet::new(self.kinds.clone());
        for language in [
            Language::Japanese,
            Language::English,
            Language::German,
            Language::French,
        ] {
            let rows = self
                .rows
                .iter()
                .map(|(row_id, cells)| {
                    let values = self
                        .kinds
                        .iter()
                        .enumerate()
                        .map(|(column, kind)| {
                            if *kind != ColumnKind::String {
                                return FakeValue::UInt32(0);
                            }
                            let mut bytes = cells.get(&column).cloned().unwrap_or_default();
                            if language != Language::English
                                && !bytes.is_empty()
                                && !self.key_columns.contains(&column)
                            {
                                bytes.extend_from_slice(
                                    format!(" [{}]", language.suffix()).as_bytes(),
                                );
                            }
                            FakeValue::String(bytes)
                        })
                        .collect();
                    FakeRow::new(*row_id, values)
                })
                .collect();
            sheet.rows.insert(language, rows);
        }
        sheet
    }
}

impl FakeGame {
    /// Adds or replaces a text sheet.
    #[must_use]
    pub fn with_text(self, name: impl Into<String>, sheet: &TextSheet) -> Self {
        self.with_sheet(name, sheet.fake())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameData;
    use crate::excel::{self, ColumnKind};

    type ReadRow = (u32, Vec<u8>, Vec<u8>, Vec<u8>);

    #[test]
    fn fake_installations_read_back_through_the_archive() {
        let folder = std::env::temp_dir().join(format!("aeria-fake-game-{}", std::process::id()));
        let long = "x".repeat(40_000);
        let default = FakeSheet::new(vec![
            ColumnKind::String,
            ColumnKind::UInt32,
            ColumnKind::String,
        ])
        .with_rows(
            Language::English,
            vec![
                FakeRow::new(3, vec!["Three".into(), 7.into(), "".into()]),
                FakeRow::new(1, vec!["One".into(), 9.into(), long.as_str().into()]),
            ],
        );
        let mut subrows = FakeSheet::new(vec![ColumnKind::String]);
        subrows.variant = Variant::Subrows;
        subrows.rows.insert(
            Language::None,
            vec![FakeRow {
                row_id: 5,
                subrows: vec![vec!["A".into()], vec!["B".into()]],
            }],
        );
        FakeGame::new("2026.01.02.0000.0000")
            .with_sheet("Addon", default)
            .with_sheet("quest/000/Line", subrows)
            .write(&folder)
            .expect("write");

        let game = GameData::open(&folder).expect("open");
        assert_eq!(
            excel::sheet_names(&game).expect("names"),
            ["Addon", "quest/000/Line"]
        );
        let sheet = excel::read_sheet(&game, "Addon", Language::English).expect("sheet");
        let rows = sheet.rows();
        let read: Vec<ReadRow> = rows
            .iter()
            .map(|row| {
                (
                    row.row_id,
                    row.string(0).expect("string").to_vec(),
                    row.value(1).expect("number"),
                    row.string(2).expect("string").to_vec(),
                )
            })
            .collect();
        assert_eq!(
            read[0],
            (
                3,
                b"Three".to_vec(),
                7_u32.to_le_bytes().to_vec(),
                Vec::new()
            )
        );
        assert_eq!(read[1].3.len(), 40_000);
        let lines = excel::read_sheet(&game, "quest/000/Line", Language::French).expect("neutral");
        let line_rows = lines.rows();
        let texts: Vec<(u32, u16, &[u8])> = line_rows
            .iter()
            .map(|row| (row.row_id, row.subrow_id, row.string(0).expect("string")))
            .collect();
        assert_eq!(texts, [(5, 0, b"A".as_slice()), (5, 1, b"B".as_slice())]);
        drop(game);
        std::fs::remove_dir_all(folder).expect("clean up");
    }
}
