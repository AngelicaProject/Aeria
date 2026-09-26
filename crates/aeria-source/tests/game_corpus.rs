//! Checks the string codec against every string of an installed game.
//!
//! Every string must print as macro text that parses without diagnostics
//! and encodes back to the same bytes, unless it holds bytes that are not a
//! valid game string, which print as `<raw …>`. Run with a game folder:
//!
//! ```text
//! AERIA_GAME_PATH="C:/Program Files (x86)/.../FINAL FANTASY XIV Online" cargo test -p aeria-source --release --test game_corpus -- --ignored --nocapture
//! ```

use std::collections::BTreeSet;

use aeria_se::{bytes, codec, parse};
use aeria_source::{GameSource, SheetLookup, SourceLanguage};

#[test]
#[ignore = "needs a game installation in AERIA_GAME_PATH"]
fn every_game_string_round_trips_through_macro_text() {
    let game = std::env::var("AERIA_GAME_PATH").expect("AERIA_GAME_PATH");
    let mut failures = Vec::new();
    let mut raw = 0_usize;
    let mut checked = 0_usize;
    for language in [
        SourceLanguage::English,
        SourceLanguage::Japanese,
        SourceLanguage::German,
        SourceLanguage::French,
    ] {
        let source = GameSource::open(&game, language).expect("open the game");
        let mut seen = BTreeSet::new();
        for name in source.sheet_names().to_vec() {
            let SheetLookup::Present(sheet) = source.read_sheet(&name).expect("read a sheet")
            else {
                continue;
            };
            for row in sheet.rows() {
                for cell in sheet.cells(row) {
                    if cell.bytes.is_empty() || !seen.insert(cell.bytes.to_vec()) {
                        continue;
                    }
                    checked += 1;
                    let text = codec::decode(cell.bytes);
                    let document = parse(&text);
                    let has_raw = document.diagnostics().iter().all(codec::is_raw_diagnostic)
                        && !document.diagnostics().is_empty();
                    if has_raw {
                        raw += 1;
                        assert_eq!(bytes::encode(&bytes::decode(cell.bytes)), cell.bytes);
                        continue;
                    }
                    let round_trip = codec::encode(&text);
                    if round_trip.as_deref() != Ok(cell.bytes) && failures.len() < 20 {
                        failures.push(format!("{name}#{}: {text}\n    {round_trip:?}", row.row_id));
                    }
                }
            }
        }
    }
    println!("checked {checked} distinct strings, {raw} with raw bytes");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
