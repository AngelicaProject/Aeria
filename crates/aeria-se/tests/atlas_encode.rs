//! Compares [`aeria_se::codec::encode_checked`] with Harmonia Atlas's
//! `encode` command on recorded results.
//!
//! The input is a file of `input-hex<TAB>result` lines, where `result` is
//! `ok:<hex>` or `err:<code>` as Atlas reported it. Run with:
//!
//! ```text
//! AERIA_ATLAS_ENCODE=path/to/atlas_encode.tsv cargo test -p aeria-se --release --test atlas_encode -- --ignored --nocapture
//! ```

use std::fmt::Write as _;

use aeria_se::codec::{CheckedEncodeError, encode_checked};

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

fn code(error: &CheckedEncodeError) -> &'static str {
    match error {
        CheckedEncodeError::Empty => "empty",
        CheckedEncodeError::Invalid(_) => "invalidMacro",
        CheckedEncodeError::ContainsNul => "containsNul",
        CheckedEncodeError::TooLong { .. } => "tooLong",
        CheckedEncodeError::NotRoundTrip => "notRoundTrip",
    }
}

#[test]
#[ignore = "needs recorded Atlas results in AERIA_ATLAS_ENCODE"]
fn encoding_matches_harmonia_atlas() {
    let path = std::env::var("AERIA_ATLAS_ENCODE").expect("AERIA_ATLAS_ENCODE");
    let recorded = std::fs::read_to_string(path).expect("read results");
    let mut total = 0;
    let mut differences = Vec::new();
    for line in recorded.lines() {
        let (input, expected) = line.split_once('\t').expect("line");
        let text = String::from_utf8(unhex(input)).expect("utf-8 input");
        let actual = match encode_checked(&text) {
            Ok(bytes) => format!("ok:{}", hex(&bytes)),
            Err(error) => format!("err:{}", code(&error)),
        };
        total += 1;
        if actual != expected {
            differences.push(format!(
                "{text:?}\n    atlas {expected}\n    aeria {actual}"
            ));
        }
    }
    println!("{total} cases, {} differ", differences.len());
    for difference in differences.iter().take(20) {
        println!("  {difference}");
    }
    assert!(total > 0);
    assert!(differences.is_empty(), "encoding must match Harmonia Atlas");
}
