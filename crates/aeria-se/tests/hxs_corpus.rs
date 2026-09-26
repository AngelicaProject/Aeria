//! Checks the codec against every string of a real HXS snapshot.
//!
//! HXS stores each string's original bytes (`raw_value`) beside the macro
//! text Lumina printed for them (`macro_text`). Run with a snapshot, for
//! example the `source/source.hxs` member of an HSP archive:
//!
//! ```text
//! AERIA_HXS_CORPUS=path/to/source.hxs cargo test -p aeria-se --release --test hxs_corpus -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::fmt::Write as _;

use aeria_se::codec::{decode, encode};

const EXAMPLES_PER_KIND: usize = 5;

#[derive(Default)]
struct Mismatches {
    counts: BTreeMap<&'static str, usize>,
    examples: BTreeMap<&'static str, Vec<String>>,
}

impl Mismatches {
    fn record(&mut self, kind: &'static str, example: impl FnOnce() -> String) {
        let count = self.counts.entry(kind).or_default();
        *count += 1;
        let examples = self.examples.entry(kind).or_default();
        if examples.len() < EXAMPLES_PER_KIND {
            examples.push(example());
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[test]
#[ignore = "needs a real HXS snapshot in AERIA_HXS_CORPUS"]
fn every_snapshot_string_decodes_to_its_macro_text_and_encodes_to_its_bytes() {
    let path = std::env::var("AERIA_HXS_CORPUS").expect("AERIA_HXS_CORPUS");
    let connection =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open snapshot");
    let mut statement = connection
        .prepare(
            "SELECT s.name, c.row_id, c.subrow_id, c.column_index, c.macro_text, c.raw_value \
             FROM string_cells c JOIN sheets s ON s.id = c.sheet_id \
             WHERE c.raw_value IS NOT NULL",
        )
        .expect("query");
    let mut rows = statement.query([]).expect("rows");
    let mut total = 0_usize;
    let mut mismatches = Mismatches::default();
    while let Some(row) = rows.next().expect("row") {
        let sheet: String = row.get(0).expect("sheet");
        let address = || {
            format!(
                "{sheet}:{}:{}:{}",
                row.get::<_, i64>(1).unwrap_or_default(),
                row.get::<_, i64>(2).unwrap_or_default(),
                row.get::<_, i64>(3).unwrap_or_default()
            )
        };
        let text: String = row.get(4).expect("macro_text");
        let raw: Vec<u8> = row.get(5).expect("raw_value");
        total += 1;
        let decoded = decode(&raw);
        if decoded != text {
            mismatches.record("decode differs from macro_text", || {
                format!(
                    "{}\n    raw     {}\n    stored  {text:?}\n    decoded {decoded:?}",
                    address(),
                    hex(&raw)
                )
            });
        }
        match encode(&text) {
            Ok(bytes) if bytes == raw => {}
            Ok(bytes) => mismatches.record("encode differs from raw_value", || {
                format!(
                    "{}\n    text    {text:?}\n    raw     {}\n    encoded {}",
                    address(),
                    hex(&raw),
                    hex(&bytes)
                )
            }),
            Err(error) => mismatches.record("encode rejects macro_text", || {
                format!("{}\n    text    {text:?}\n    error   {error}", address())
            }),
        }
    }
    println!("{total} strings checked");
    for (kind, count) in &mismatches.counts {
        println!("\n{kind}: {count}");
        for example in &mismatches.examples[kind] {
            println!("  {example}");
        }
    }
    assert!(total > 0, "the snapshot has no strings with raw values");
    assert!(
        !mismatches
            .counts
            .contains_key("decode differs from macro_text"),
        "decoding must reproduce Lumina's macro text exactly"
    );
}
