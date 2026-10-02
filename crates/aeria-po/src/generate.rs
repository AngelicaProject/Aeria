//! The files of a sheet, made from the installed game with empty
//! translations. [`crate::merge`] then carries the translations over.

use std::fmt::Write as _;

use aeria_source::{
    DialogueKind, GameSource, LineRole, RowKeys, SheetInLanguage, SheetLookup, SourceCell,
    SourceLanguage, SourceRow, SourceSheet, line_role,
};

use crate::identity::{Identity, RowName, SheetPaths, is_scene, splits};
use crate::po::{Entry, Header, PoFile};

/// Errors while reading the game.
#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    #[error("the game cannot be read: {0}")]
    Source(#[from] aeria_source::SourceError),
}

/// What a project's files say about themselves.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Languages {
    /// The target language tag, such as `ru`.
    pub target: String,
}

/// The header fields of a file made for the installed game.
fn header(source: &GameSource, languages: &Languages, comments: Vec<String>) -> Header {
    Header {
        comments,
        fields: vec![
            ("Language".to_owned(), languages.target.clone()),
            (
                "Content-Type".to_owned(),
                "text/plain; charset=UTF-8".to_owned(),
            ),
            ("X-Game-Version".to_owned(), source.version().to_string()),
            (
                "X-Source-Language".to_owned(),
                source.language().code().to_owned(),
            ),
        ],
    }
}

/// The title of a quest sheet's quest.
fn quest_title(source: &GameSource, sheet: &str) -> Option<String> {
    let (row, subrow) = source.quest_row(sheet).ok().flatten()?;
    let SheetLookup::Present(quest) = source.sheet("Quest").ok()? else {
        return None;
    };
    let row = quest.row(row, subrow)?;
    quest
        .cells(row)
        .find(|cell| cell.translatable && !cell.bytes.is_empty())
        .map(|cell| cell.text())
}

/// What a dialogue line's key says about it.
fn role_comment(sheet: &str, key: &str) -> String {
    match line_role(sheet, key) {
        LineRole::Journal => "kind: journal".to_owned(),
        LineRole::Objective => "kind: objective".to_owned(),
        LineRole::Speech { speaker } => format!("speaker: {speaker}"),
        LineRole::Other => "kind: other".to_owned(),
    }
}

/// The row keys that name a sheet's rows in its identities: those of a keyed
/// sheet, when no key is empty or could be mistaken for a separator.
#[must_use]
pub fn identity_keys(sheet: &SourceSheet) -> Option<&RowKeys> {
    sheet.row_keys().filter(|keys| {
        sheet.rows().iter().all(|row| {
            keys.key_of(row.row_id, row.subrow_id)
                .is_some_and(|key| !key.contains(':') && !key.is_empty())
        })
    })
}

/// The identity of a string: by the row's key when the sheet's rows are
/// named by keys (see [`identity_keys`]), otherwise by row ID and subrow.
#[must_use]
pub fn identity_of(sheet: &str, key: Option<&str>, row: u32, subrow: u16, column: u32) -> Identity {
    Identity {
        sheet: sheet.to_owned(),
        row: match key {
            Some(key) => RowName::Key(key.to_owned()),
            None => RowName::Id { row, subrow },
        },
        column,
    }
}

/// Whether a cell of a sheet is an entry of the project: a translatable,
/// non-empty string that is not the row key.
#[must_use]
pub fn is_entry(cell: &SourceCell<'_>, keys: Option<&RowKeys>) -> bool {
    cell.translatable
        && !cell.bytes.is_empty()
        && keys.is_none_or(|keys| keys.column() != cell.column)
}

/// The files of one sheet, each with its path relative to `po/`, in order.
/// A sheet the game does not have or cannot read has none.
///
/// # Errors
///
/// Returns an error when a game file cannot be read.
pub fn sheet_files(
    source: &GameSource,
    paths: &SheetPaths,
    languages: &Languages,
    name: &str,
) -> Result<Vec<(String, PoFile)>, GenerateError> {
    let SheetLookup::Present(sheet) = source.sheet(name)? else {
        return Ok(Vec::new());
    };
    let keys = identity_keys(&sheet);
    let dialogue = DialogueKind::of(name).is_some() && keys.is_some();
    let highest = sheet.rows().last().map_or(0, |row| row.row_id);
    let split = splits(name, highest);
    let title = if name.starts_with("quest/") {
        quest_title(source, name)
    } else {
        None
    };
    let mut comment = name.to_owned();
    if let Some(title) = &title {
        let _ = write!(comment, " — «{title}»");
    }
    if is_scene(name) {
        comment.push_str(" · in play order");
    }
    // The other client languages, each read once for the whole sheet.
    let others: Vec<(SourceLanguage, SheetInLanguage)> = SourceLanguage::ALL
        .into_iter()
        .filter(|language| *language != source.language())
        .map(|language| Ok((language, source.sheet_in_language(&sheet, language)?)))
        .collect::<Result<_, GenerateError>>()?;
    let mut files: Vec<(String, PoFile)> = Vec::new();
    for row in sheet.rows() {
        let key = keys.and_then(|keys| keys.key_of(row.row_id, row.subrow_id));
        let cells: Vec<_> = sheet.cells(row).collect();
        for cell in &cells {
            if !is_entry(cell, keys) {
                continue;
            }
            let text = cell.text();
            let extracted = extracted(&others, name, row, cell, &cells, keys, dialogue);
            let identity = identity_of(name, key, row.row_id, row.subrow_id, cell.column);
            let path = paths.file(name, split, row.row_id);
            let entry = Entry {
                extracted,
                context: identity.to_string(),
                source: text,
                ..Entry::default()
            };
            match files.last_mut() {
                Some((last, file)) if *last == path => file.entries.push(entry),
                _ => files.push((
                    path,
                    PoFile {
                        header: header(source, languages, vec![comment.clone()]),
                        entries: vec![entry],
                        obsolete: Vec::new(),
                    },
                )),
            }
        }
    }
    Ok(files)
}

/// The `#.` lines of a cell: the other client languages, the speaker or
/// kind of a dialogue line or the row's other cells, and what its macros do.
fn extracted(
    others: &[(SourceLanguage, SheetInLanguage)],
    name: &str,
    row: &SourceRow,
    cell: &SourceCell<'_>,
    cells: &[SourceCell<'_>],
    keys: Option<&RowKeys>,
    dialogue: bool,
) -> Vec<String> {
    let text = cell.text();
    let mut lines: Vec<String> = others
        .iter()
        .filter_map(|(language, sheet)| {
            sheet
                .text(row.row_id, row.subrow_id, cell.column)
                .filter(|other| !other.trim().is_empty())
                .map(|other| format!("{}: {other}", code(*language)))
        })
        .collect();
    let key = keys.and_then(|keys| keys.key_of(row.row_id, row.subrow_id));
    if dialogue {
        if let Some(key) = key {
            lines.push(role_comment(name, key));
        }
    } else {
        lines.extend(
            cells
                .iter()
                .filter(|other| other.column != cell.column && !other.bytes.is_empty())
                .filter(|other| keys.is_none_or(|keys| keys.column() != other.column))
                .map(|other| format!("column {}: {}", other.column, other.text())),
        );
    }
    if let Ok(constructs) = aeria_se::constructs(&text) {
        lines.extend(
            constructs
                .iter()
                .map(|construct| format!("macro: {}", construct.legend())),
        );
    }
    lines
}

fn code(language: SourceLanguage) -> &'static str {
    language.code()
}
