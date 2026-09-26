//! Checks translation permission against the guidance (HSG) that Harmonia
//! Atlas produced for the same game version. Run with the installed game and
//! the `guidance/source-guidance.json` member of an HSP archive:
//!
//! ```text
//! AERIA_GAME_PATH="C:/Program Files (x86)/.../FINAL FANTASY XIV Online" \
//! AERIA_HSG=path/to/source-guidance.json \
//! cargo test -p aeria-source --release --test hsg_conformance -- --ignored --nocapture
//! ```

use std::collections::{BTreeMap, BTreeSet};

use aeria_source::{GameSource, SheetLookup};

type Cells = BTreeSet<(u32, u16, u32)>;

fn guidance() -> (String, String, BTreeMap<String, Cells>) {
    let path = std::env::var("AERIA_HSG").expect("AERIA_HSG");
    let text = std::fs::read_to_string(path).expect("guidance");
    let json: serde_json::Value = serde_json::from_str(&text).expect("guidance JSON");
    let version = json["gameVersion"]
        .as_str()
        .expect("game version")
        .to_owned();
    let language = json["source"]["language"]
        .as_str()
        .expect("language")
        .to_owned();
    let mut sheets = BTreeMap::new();
    for sheet in json["sheets"].as_array().expect("sheets") {
        let cells = sheet["translatable"]
            .as_array()
            .expect("translatable")
            .iter()
            .map(|cell| {
                let number = |field: &str| cell[field].as_u64().expect("number");
                (
                    u32::try_from(number("rowId")).expect("row"),
                    u16::try_from(number("subrowId")).expect("subrow"),
                    u32::try_from(number("columnIndex")).expect("column"),
                )
            })
            .collect();
        sheets.insert(sheet["name"].as_str().expect("name").to_owned(), cells);
    }
    (version, language, sheets)
}

#[test]
#[ignore = "needs the installed game in AERIA_GAME_PATH and its HSG in AERIA_HSG"]
fn translatable_cells_match_the_atlas_guidance() {
    let (version, language, expected) = guidance();
    let source = GameSource::open(
        std::env::var("AERIA_GAME_PATH").expect("AERIA_GAME_PATH"),
        language.parse().expect("source language"),
    )
    .expect("game");
    assert_eq!(
        source.version().as_str(),
        version,
        "the game version differs"
    );

    let (mut granted, mut keyed) = (0_usize, 0_usize);
    let mut differences = Vec::new();
    for name in source.sheet_names() {
        let SheetLookup::Present(sheet) = source.read_sheet(name).expect("sheet") else {
            differences.push(format!("{name}: unavailable"));
            continue;
        };
        if sheet.row_keys().is_some() {
            keyed += 1;
        }
        let actual: Cells = sheet
            .rows()
            .iter()
            .flat_map(|row| {
                sheet
                    .cells(row)
                    .filter(|cell| cell.translatable)
                    .map(|cell| (row.row_id, row.subrow_id, cell.column))
            })
            .collect();
        granted += actual.len();
        let expected = expected.get(name).cloned().unwrap_or_default();
        if actual != expected {
            let extra = actual.difference(&expected).count();
            let missing = expected.difference(&actual).count();
            differences.push(format!("{name}: {extra} extra, {missing} missing"));
        }
    }
    println!(
        "{} sheets, {granted} translatable cells, {keyed} keyed sheets",
        source.sheet_names().len()
    );
    for difference in differences.iter().take(20) {
        println!("{difference}");
    }
    assert!(
        differences.is_empty(),
        "{} sheets differ",
        differences.len()
    );
}
