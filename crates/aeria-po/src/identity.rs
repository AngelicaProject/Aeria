//! The identity of an entry (`msgctxt`) and the file that holds it.

use std::collections::BTreeMap;
use std::fmt;

/// How an entry names its row.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RowName {
    /// The row ID and subrow: the ID of a game object.
    Id { row: u32, subrow: u16 },
    /// The row key of a keyed sheet, such as quest dialogue.
    Key(String),
}

/// A string of the game: `sheet:row:subrow:column`, or `sheet:key:column`
/// in a keyed sheet.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Identity {
    pub sheet: String,
    pub row: RowName,
    pub column: u32,
}

impl fmt::Display for Identity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.row {
            RowName::Id { row, subrow } => {
                write!(formatter, "{}:{row}:{subrow}:{}", self.sheet, self.column)
            }
            RowName::Key(key) => write!(formatter, "{}:{key}:{}", self.sheet, self.column),
        }
    }
}

impl Identity {
    /// Reads a `msgctxt`. Sheet names and keys never contain `:`, so four
    /// parts are a row ID and three a key.
    ///
    /// # Errors
    ///
    /// Returns what is wrong when the text is not an identity.
    pub fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.split(':').collect();
        let number = |part: &str| {
            part.parse::<u32>()
                .map_err(|_| format!("{text:?}: {part:?} is not a number"))
        };
        match parts.as_slice() {
            [sheet, row, subrow, column] if !sheet.is_empty() => Ok(Self {
                sheet: (*sheet).to_owned(),
                row: RowName::Id {
                    row: number(row)?,
                    subrow: subrow
                        .parse()
                        .map_err(|_| format!("{text:?}: {subrow:?} is not a subrow"))?,
                },
                column: number(column)?,
            }),
            [sheet, key, column] if !sheet.is_empty() && !key.is_empty() => Ok(Self {
                sheet: (*sheet).to_owned(),
                row: RowName::Key((*key).to_owned()),
                column: number(column)?,
            }),
            _ => Err(format!(
                "{text:?} is not an identity: sheet:row:subrow:column, or sheet:key:column in a keyed sheet"
            )),
        }
    }
}

/// Row IDs per file of a sheet that is split.
pub const ROWS_PER_FILE: u32 = 1000;

/// Whether a sheet is a scene, one file in play order: quest and cutscene
/// dialogue.
#[must_use]
pub fn is_scene(sheet: &str) -> bool {
    sheet.starts_with("quest/") || sheet.starts_with("cut_scene/")
}

/// Names Windows reserves for devices, in any case and with any extension.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn reserved(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or(segment);
    RESERVED.iter().any(|name| name.eq_ignore_ascii_case(stem))
}

/// The paths of sheets in `po/`, safe on systems that ignore case:
///
/// - a segment Windows reserves gets `~`;
/// - a folder takes the casing it first has in the sheet list, so two
///   sheets never ask for one folder in two casings;
/// - a sheet whose path is also a folder of other sheets, in any case, gets
///   `~`, so a sheet split into files by row never shares a folder with
///   other sheets (the data sheet `Quest` next to the dialogue in `quest/`);
/// - a sheet whose path differs from an earlier one's only by case gets `~`
///   and its number among them.
#[derive(Clone, Debug, Default)]
pub struct SheetPaths {
    paths: BTreeMap<String, String>,
}

fn segments(sheet: &str) -> Vec<String> {
    sheet
        .split('/')
        .map(|segment| {
            if reserved(segment) {
                format!("{segment}~")
            } else {
                segment.to_owned()
            }
        })
        .collect()
}

impl SheetPaths {
    /// Paths for the sheets of a game, in the game's sorted sheet list.
    #[must_use]
    pub fn new<'a>(sheets: impl IntoIterator<Item = &'a str>) -> Self {
        let sheets: Vec<&str> = sheets.into_iter().collect();
        // Every folder, by its lowercase path, in the casing it first has.
        let mut folders: BTreeMap<String, String> = BTreeMap::new();
        for sheet in &sheets {
            let parts = segments(sheet);
            let mut folder = String::new();
            for part in &parts[..parts.len().saturating_sub(1)] {
                let next = if folder.is_empty() {
                    part.clone()
                } else {
                    format!("{folder}/{part}")
                };
                folder.clone_from(folders.entry(next.to_lowercase()).or_insert(next));
            }
        }
        let mut paths = BTreeMap::new();
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for sheet in sheets {
            let parts = segments(sheet);
            let (name, parent) = parts.split_last().unwrap_or((&parts[0], &[]));
            let mut path = if parent.is_empty() {
                name.clone()
            } else {
                let parent = parent.join("/");
                let parent = folders
                    .get(&parent.to_lowercase())
                    .cloned()
                    .unwrap_or(parent);
                format!("{parent}/{name}")
            };
            if folders.contains_key(&path.to_lowercase()) {
                path.push('~');
            }
            let count = seen.entry(path.to_lowercase()).or_default();
            *count += 1;
            if *count > 1 {
                path = format!("{path}~{count}");
            }
            paths.insert(sheet.to_owned(), path);
        }
        Self { paths }
    }

    /// The path of a sheet without the extension.
    #[must_use]
    pub fn base<'a>(&'a self, sheet: &'a str) -> &'a str {
        self.paths.get(sheet).map_or(sheet, String::as_str)
    }

    /// The file of a row of a sheet, relative to `po/`. `split` is whether
    /// the sheet is split by row ID range (see [`splits`]).
    #[must_use]
    pub fn file(&self, sheet: &str, split: bool, row: u32) -> String {
        let base = self.base(sheet);
        if split {
            let start = row / ROWS_PER_FILE * ROWS_PER_FILE;
            format!("{base}/{start}.po")
        } else {
            format!("{base}.po")
        }
    }
}

/// Whether a sheet is split into files by row ID range: a sheet other than a
/// scene with a row ID of at least [`ROWS_PER_FILE`].
#[must_use]
pub fn splits(sheet: &str, highest_row: u32) -> bool {
    !is_scene(sheet) && highest_row >= ROWS_PER_FILE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_read_back() {
        for text in [
            "BNpcName:3726:0:2",
            "quest/000/ClsArc000_00021:TEXT_CLSARC000_00021_SEQ_00:1",
        ] {
            assert_eq!(Identity::parse(text).expect("identity").to_string(), text);
        }
        assert!(Identity::parse("BNpcName:x:0:2").is_err());
        assert!(Identity::parse("BNpcName:1").is_err());
        assert!(Identity::parse(":1:0:0").is_err());
    }

    #[test]
    fn files_by_row_range_and_safe_paths() {
        let paths = SheetPaths::new([
            "Aux",
            "BNpcName",
            "Custom/1/A",
            "Item",
            "Quest",
            "custom/2/B",
            "item",
            "quest/000/A",
        ]);
        assert_eq!(paths.file("BNpcName", true, 3726), "BNpcName/3000.po");
        assert_eq!(paths.file("BNpcName", true, 999), "BNpcName/0.po");
        assert_eq!(paths.file("Addon", false, 5), "Addon.po");
        assert_eq!(paths.base("Aux"), "Aux~");
        assert_eq!(paths.base("Item"), "Item");
        assert_eq!(paths.base("item"), "item~2");
        // A sheet named like a folder of other sheets, and folders in one
        // casing.
        assert_eq!(paths.base("Quest"), "Quest~");
        assert_eq!(paths.file("Quest", true, 65_123), "Quest~/65000.po");
        assert_eq!(paths.base("quest/000/A"), "quest/000/A");
        assert_eq!(paths.base("custom/2/B"), "Custom/2/B");
        assert!(splits("BNpcName", 15_000));
        assert!(!splits("quest/000/A", 15_000));
        assert!(!splits("Race", 20));
    }
}
