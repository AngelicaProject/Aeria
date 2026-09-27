//! Checks the Excel reader against a real HXS snapshot of the same game
//! version: sheet variants, columns, row coordinates, and the bytes of every
//! string. Run with the installed game and a snapshot:
//!
//! ```text
//! AERIA_GAME_PATH="C:/Program Files (x86)/.../FINAL FANTASY XIV Online" \
//! AERIA_HXS_CORPUS=path/to/source.hxs \
//! cargo test -p aeria-sqpack --release --test hxs_conformance -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;

use aeria_sqpack::GameData;
use aeria_sqpack::excel::{self, ColumnKind, Language, Variant};

const EXAMPLES: usize = 5;

#[derive(Default)]
struct Mismatches {
    counts: BTreeMap<&'static str, usize>,
    examples: Vec<String>,
}

impl Mismatches {
    fn record(&mut self, kind: &'static str, example: impl FnOnce() -> String) {
        let count = self.counts.entry(kind).or_default();
        *count += 1;
        if *count <= EXAMPLES {
            self.examples.push(format!("{kind}: {}", example()));
        }
    }
}

/// The HXS code of a column type.
fn column_code(kind: ColumnKind) -> i64 {
    match kind {
        ColumnKind::String => 1,
        ColumnKind::Bool => 2,
        ColumnKind::Int8 => 10,
        ColumnKind::UInt8 => 11,
        ColumnKind::Int16 => 12,
        ColumnKind::UInt16 => 13,
        ColumnKind::Int32 => 14,
        ColumnKind::UInt32 => 15,
        ColumnKind::Int64 => 16,
        ColumnKind::UInt64 => 17,
        ColumnKind::Float32 => 20,
        ColumnKind::PackedBool(bit) => 30 + i64::from(bit),
    }
}

#[test]
#[ignore = "needs the installed game in AERIA_GAME_PATH and its HXS snapshot in AERIA_HXS_CORPUS"]
fn every_snapshot_sheet_reads_the_same_from_the_game() {
    let game =
        GameData::open(std::env::var("AERIA_GAME_PATH").expect("AERIA_GAME_PATH")).expect("game");
    let path = std::env::var("AERIA_HXS_CORPUS").expect("AERIA_HXS_CORPUS");
    let db =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open snapshot");
    let language: String = db
        .query_row("SELECT language FROM hxs_meta", [], |row| row.get(0))
        .expect("language");
    let language = match language.as_str() {
        "ja" => Language::Japanese,
        "en" => Language::English,
        "de" => Language::German,
        "fr" => Language::French,
        other => panic!("unsupported snapshot language {other}"),
    };

    let mut names = excel::sheet_names(&game).expect("sheet list");
    names.sort();
    let mut snapshot_names: Vec<String> = db
        .prepare("SELECT name FROM sheets UNION SELECT name FROM excluded_sheets ORDER BY name")
        .expect("sheets")
        .query_map([], |row| row.get(0))
        .expect("sheets")
        .collect::<Result<_, _>>()
        .expect("sheets");
    snapshot_names.sort();
    assert_eq!(names, snapshot_names, "the sheet lists differ");

    let mut sheets = db
        .prepare("SELECT id, name, variant FROM sheets ORDER BY name")
        .expect("sheets");
    let sheets: Vec<(i64, String, i64)> = sheets
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("sheets")
        .collect::<Result<_, _>>()
        .expect("sheets");
    let mut mismatches = Mismatches::default();
    let mut strings = 0_usize;
    for (id, name, variant) in &sheets {
        strings += compare_sheet(&game, &db, language, (*id, name, *variant), &mut mismatches);
    }
    println!("{} sheets, {strings} strings", sheets.len());
    for example in &mismatches.examples {
        println!("{example}");
    }
    assert!(
        mismatches.counts.is_empty(),
        "mismatches: {:?}",
        mismatches.counts
    );
}

/// Compares one stored sheet and returns the number of strings read.
fn compare_sheet(
    game: &GameData,
    db: &rusqlite::Connection,
    language: Language,
    (id, name, variant): (i64, &str, i64),
    mismatches: &mut Mismatches,
) -> usize {
    let mut strings = 0;
    let sheet = excel::read_sheet(game, name, language).expect("readable sheet");
    let expected_variant = if variant == 0 {
        Variant::Default
    } else {
        Variant::Subrows
    };
    if sheet.variant != expected_variant {
        mismatches.record("variant", || name.to_owned());
    }
    let columns: Vec<(i64, i64)> = db
        .prepare("SELECT offset, type FROM columns WHERE sheet_id = ?1 ORDER BY column_index")
        .expect("columns")
        .query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("columns")
        .collect::<Result<_, _>>()
        .expect("columns");
    let read: Vec<(i64, i64)> = sheet
        .columns
        .iter()
        .map(|column| (i64::from(column.offset), column_code(column.kind)))
        .collect();
    if columns != read {
        mismatches.record("columns", || name.to_owned());
        return 0;
    }

    let mut expected: BTreeMap<(i64, i64, i64), Option<Vec<u8>>> = BTreeMap::new();
    let mut statement = db
        .prepare("SELECT row_id, subrow_id, column_index, raw_value FROM string_cells WHERE sheet_id = ?1")
        .expect("cells");
    let mut rows = statement.query([id]).expect("cells");
    while let Some(row) = rows.next().expect("cell") {
        expected.insert(
            (
                row.get(0).expect("row"),
                row.get(1).expect("subrow"),
                row.get(2).expect("column"),
            ),
            row.get(3).expect("raw"),
        );
    }
    let row_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM rows WHERE sheet_id = ?1",
            [id],
            |row| row.get(0),
        )
        .expect("row count");
    let rows = sheet.rows();
    if i64::try_from(rows.len()).expect("count") != row_count {
        mismatches.record("row count", || {
            format!("{name}: {} read, {row_count} in snapshot", rows.len())
        });
    }
    for row in &rows {
        for (index, column) in sheet.columns.iter().enumerate() {
            if column.kind != ColumnKind::String {
                continue;
            }
            strings += 1;
            let key = (
                i64::from(row.row_id),
                i64::from(row.subrow_id),
                i64::try_from(index).expect("index"),
            );
            let bytes = row.string(index).expect("string");
            match expected.remove(&key) {
                None => mismatches.record("missing in snapshot", || format!("{name} {key:?}")),
                Some(None) => {
                    mismatches.record("no raw value in snapshot", || format!("{name} {key:?}"));
                }
                Some(Some(raw)) if raw != bytes => {
                    mismatches.record("bytes", || format!("{name} {key:?}"));
                }
                Some(Some(_)) => {}
            }
        }
    }
    for key in expected.keys() {
        mismatches.record("missing in game", || format!("{name} {key:?}"));
    }
    strings
}
