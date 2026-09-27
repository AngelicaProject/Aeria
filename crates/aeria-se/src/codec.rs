//! `SeString` bytes and macro text.
//!
//! [`decode`] prints bytes as macro text and [`encode`] reads macro text back
//! into bytes. Every input decodes, and `encode(&decode(bytes)) == bytes`
//! for every valid game string. Bytes that are not a valid game string
//! decode to `<raw …>`, which shows them but cannot be encoded. The macro
//! text grammar is in `docs/architecture/strings.md`.

use crate::bytes;
use crate::syntax::{Diagnostic, DiagnosticKind, parse, print};

/// Prints `SeString` bytes as macro text.
#[must_use]
pub fn decode(bytes: &[u8]) -> String {
    print(&bytes::decode(bytes))
}

/// Macro text that cannot be encoded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodeError {
    /// What is wrong.
    pub message: String,
    /// Byte offset into the macro text.
    pub offset: usize,
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for EncodeError {}

impl From<&Diagnostic> for EncodeError {
    fn from(diagnostic: &Diagnostic) -> Self {
        Self {
            message: diagnostic.message.clone(),
            offset: diagnostic.span.start(),
        }
    }
}

/// Encodes macro text as `SeString` bytes.
///
/// # Errors
///
/// Returns the first diagnostic of the text, such as an unknown macro name,
/// a missing closing tag, or `<raw>` bytes.
pub fn encode(text: &str) -> Result<Vec<u8>, EncodeError> {
    let document = parse(text);
    if let Some(diagnostic) = document.diagnostics().first() {
        return Err(EncodeError::from(diagnostic));
    }
    let nodes = document.to_nodes().ok_or_else(|| EncodeError {
        message: "the text could not be parsed".to_owned(),
        offset: 0,
    })?;
    Ok(bytes::encode(&nodes))
}

/// Why macro text cannot be written into the game.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckedEncodeError {
    Empty,
    Invalid(EncodeError),
    ContainsNul,
    TooLong { length: usize },
    NotRoundTrip,
}

impl std::fmt::Display for CheckedEncodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("the macro text encodes to no bytes"),
            Self::Invalid(error) => error.fmt(formatter),
            Self::ContainsNul => formatter.write_str("the encoded string contains a NUL byte"),
            Self::TooLong { length } => write!(
                formatter,
                "the encoded string is {length} bytes, the limit is {MAX_ENCODED_LENGTH}"
            ),
            Self::NotRoundTrip => formatter
                .write_str("the encoded bytes do not survive a decode and encode round trip"),
        }
    }
}

impl std::error::Error for CheckedEncodeError {}

/// Longest encoded string the game accepts.
pub const MAX_ENCODED_LENGTH: usize = 65535;

/// Encodes macro text for a pack string: the bytes are not empty, contain no
/// NUL, fit the length limit, and encode again to the same bytes after
/// decoding.
///
/// # Errors
///
/// Returns the first check that fails.
pub fn encode_checked(text: &str) -> Result<Vec<u8>, CheckedEncodeError> {
    if text.is_empty() {
        return Err(CheckedEncodeError::Empty);
    }
    let bytes = encode(text).map_err(CheckedEncodeError::Invalid)?;
    if bytes.is_empty() {
        return Err(CheckedEncodeError::Empty);
    }
    if bytes.contains(&0) {
        return Err(CheckedEncodeError::ContainsNul);
    }
    if bytes.len() > MAX_ENCODED_LENGTH {
        return Err(CheckedEncodeError::TooLong {
            length: bytes.len(),
        });
    }
    match encode(&decode(&bytes)) {
        Ok(again) if again == bytes => Ok(bytes),
        _ => Err(CheckedEncodeError::NotRoundTrip),
    }
}

/// Whether a diagnostic reports `<raw>` bytes.
#[must_use]
pub fn is_raw_diagnostic(diagnostic: &Diagnostic) -> bool {
    diagnostic.kind == DiagnosticKind::RawBytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
            .collect()
    }

    #[test]
    fn bytes_decode_to_readable_macro_text() {
        assert_eq!(decode(&hex("021305FCFFEEFF03")), "<color #FFEE00FF>");
        assert_eq!(
            decode(&hex("02080BE4E902E903FF0241FF0103")),
            "<if ($gn1 == $gn2)>A</if>"
        );
        assert_eq!(decode(b"a<b\\c{"), "a\\<b\\\\c\\{");
        assert_eq!(decode(&[0x61, 0x00, 0x62]), "a<raw 00>b");
        assert_eq!(decode(&hex("02FA0103")), "<code:FA>");
        assert_eq!(decode(&hex("0220020103")), "<num 0>");
        assert_eq!(decode(&[0x61, 0xFF, 0x62]), "a<raw FF>b");
    }

    #[test]
    fn valid_strings_round_trip_and_raw_bytes_do_not_encode() {
        for text in [
            "<if ($gn1 == $gn2)>A</if>",
            "<i>Omnilex</i>",
            "<color #FF13212F>x</color>",
            "<switch $n1><case>a<case>{2}</switch>",
            "<code:FA 1 x>",
        ] {
            let bytes = encode(text).expect(text);
            assert_eq!(decode(&bytes), text);
        }
        assert!(encode("a<raw 00>b").is_err());
        assert!(encode("<nope>").is_err());
        assert_eq!(encode_checked(""), Err(CheckedEncodeError::Empty));
        assert_eq!(
            encode_checked("<string $s1>x"),
            Ok(encode("<string $s1>x").expect("encode"))
        );
    }
}
