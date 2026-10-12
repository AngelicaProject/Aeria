//! The sheets of the game a project declares part of its glossary:
//! `aeria-knowledge/term-sheets.csv` (`docs/formats/knowledge-v1.md`).

use serde::Serialize;

const COLUMNS: [&str; 4] = ["sheet", "column", "category", "note"];

/// A sheet of the game whose strings of one column are names: each name is
/// a term, with the translation its strings have most often.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermSheet {
    /// The sheet's name as the game has it.
    pub sheet: String,
    /// The column of the strings that are names.
    pub column: u32,
    /// What the sheet's names are (`задание`, `место`): the category of its
    /// terms.
    pub category: String,
    /// What the sheet's names are and where they apply: the note of every
    /// term of the sheet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The declared sheets of a file, and its rows that cannot be used, each
/// with its line and why.
pub type TermSheets = (Vec<TermSheet>, Vec<(u64, String)>);

/// Reads the declared sheets. Rows that cannot be used are left out and
/// returned with their line and why.
///
/// # Errors
///
/// Returns a description for a file whose header is invalid or that is not
/// CSV.
pub fn parse_term_sheets(text: &str) -> Result<TermSheets, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|error| format!("invalid CSV: {error}"))?
        .clone();
    let mut index: [Option<usize>; 4] = [None; 4];
    for (position, header) in headers.iter().enumerate() {
        let name = header.trim();
        let Some(column) = COLUMNS.iter().position(|known| *known == name) else {
            return Err(format!("unknown column {name:?}"));
        };
        if index[column].replace(position).is_some() {
            return Err(format!("the column {name:?} is repeated"));
        }
    }
    if index[0].is_none() || index[2].is_none() {
        return Err("the header must name the columns sheet and category".to_owned());
    }
    let mut sheets: Vec<TermSheet> = Vec::new();
    let mut rejected = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|error| format!("invalid CSV: {error}"))?;
        let line = record.position().map_or(0, csv::Position::line);
        if record.iter().all(|value| value.trim().is_empty()) {
            continue;
        }
        let field = |column: usize| {
            index[column]
                .and_then(|position| record.get(position))
                .map(str::trim)
                .unwrap_or_default()
        };
        let sheet = field(0);
        let category = field(2);
        if sheet.is_empty() || category.is_empty() {
            rejected.push((
                line,
                "the sheet and its category must not be empty".to_owned(),
            ));
            continue;
        }
        let column = match field(1) {
            "" => 0,
            column => {
                let Ok(column) = column.parse() else {
                    rejected.push((line, format!("the column {column:?} is not a number")));
                    continue;
                };
                column
            }
        };
        let text = |column: usize| Some(field(column).to_owned()).filter(|text| !text.is_empty());
        let declared = TermSheet {
            sheet: sheet.to_owned(),
            column,
            category: category.to_owned(),
            note: text(3),
        };
        if sheets
            .iter()
            .any(|known| known.sheet == declared.sheet && known.column == declared.column)
        {
            rejected.push((
                line,
                format!("the sheet {sheet:?} is already declared above"),
            ));
            continue;
        }
        sheets.push(declared);
    }
    Ok((sheets, rejected))
}

/// The file of `sheets` in its canonical form: the header, one record per
/// sheet, LF line endings.
#[must_use]
pub fn write_term_sheets(sheets: &[TermSheet]) -> String {
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(Vec::new());
    let _ = writer.write_record(COLUMNS);
    for sheet in sheets {
        let _ = writer.write_record([
            sheet.sheet.as_str(),
            &sheet.column.to_string(),
            sheet.category.as_str(),
            sheet.note.as_deref().unwrap_or_default(),
        ]);
    }
    String::from_utf8(writer.into_inner().unwrap_or_default()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_sheets_read_back_and_bad_rows_are_reported() {
        let sheets = vec![
            TermSheet {
                sheet: "Quest".to_owned(),
                column: 0,
                category: "задание".to_owned(),
                note: Some("Названия заданий, не достижений.".to_owned()),
            },
            TermSheet {
                sheet: "custom/000/Names".to_owned(),
                column: 2,
                category: "персонаж".to_owned(),
                note: None,
            },
        ];
        let text = write_term_sheets(&sheets);
        assert_eq!(
            text,
            "sheet,column,category,note\nQuest,0,задание,\"Названия заданий, не достижений.\"\ncustom/000/Names,2,персонаж,\n"
        );
        assert_eq!(parse_term_sheets(&text), Ok((sheets, Vec::new())));
        assert_eq!(write_term_sheets(&[]), "sheet,column,category,note\n");
        // A sheet declared with another note is the same sheet.
        let (read, rejected) =
            parse_term_sheets("sheet,category,note\nQuest,задание,one\nQuest,задание,two\n")
                .expect("rows");
        assert_eq!((read.len(), rejected.len()), (1, 1));

        let (read, rejected) =
            parse_term_sheets("\u{feff}sheet,category\nQuest,задание\nQuest,задание\n")
                .expect("a column may be absent");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].column, 0);
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].0, 3);
        let (read, rejected) = parse_term_sheets(
            "column,sheet,category\nx,Quest,задание\n,,задание\n1,,задание\n2,Quest,\n",
        )
        .expect("rows");
        assert!(read.is_empty());
        assert_eq!(rejected.len(), 4, "{rejected:?}");
        assert!(parse_term_sheets("sheet,category,folder\n").is_err());
        assert!(parse_term_sheets("sheet,category,forms\n").is_err());
        assert!(parse_term_sheets("column,category\n").is_err());
        assert!(parse_term_sheets("sheet,column\n").is_err());
    }
}
