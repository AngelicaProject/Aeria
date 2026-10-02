//! Core domain types shared by the game source, the project, and export:
//! game versions, sheet layout hashes, and language tags. Must remain
//! independent of Tauri, persistence implementations, and UI.

#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// Lowercase hexadecimal of `bytes`.
fn hex(bytes: &[u8], formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    for byte in bytes {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

/// Parses exactly `N` bytes of lowercase hexadecimal.
fn parse_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let digits = text.as_bytes();
    if digits.len() != N * 2 {
        return None;
    }
    let nibble = |digit: u8| match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    };
    let mut bytes = [0; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = (nibble(digits[index * 2])? << 4) | nibble(digits[index * 2 + 1])?;
    }
    Some(bytes)
}

/// A game version such as `2026.09.15.0000.0000`: dot-separated decimal
/// numbers, compared component by component.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GameVersion {
    text: String,
}

/// A game version text that does not have the expected form.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{text:?} is not a game version of dot-separated numbers")]
pub struct GameVersionError {
    pub text: String,
}

impl GameVersion {
    /// Returns the version as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The numeric components; the text was validated on construction.
    fn components(&self) -> impl Iterator<Item = u32> + '_ {
        self.text
            .split('.')
            .map(|part| part.parse().unwrap_or(u32::MAX))
    }
}

impl FromStr for GameVersion {
    type Err = GameVersionError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || GameVersionError {
            text: text.to_owned(),
        };
        for part in text.split('.') {
            if part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || part.parse::<u32>().is_err()
            {
                return Err(invalid());
            }
        }
        Ok(Self {
            text: text.to_owned(),
        })
    }
}

impl Ord for GameVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.components()
            .cmp(other.components())
            .then_with(|| self.text.cmp(&other.text))
    }
}

impl PartialOrd for GameVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for GameVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

/// The hash of a sheet's String column layout: which column index sits at
/// which row offset. See `docs/architecture/source.md`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LayoutHash([u8; 8]);

/// A layout hash that is not 16 lowercase hexadecimal characters.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("a layout hash is 16 lowercase hexadecimal characters")]
pub struct LayoutHashParseError;

impl LayoutHash {
    /// Creates a hash from its bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }

    /// Returns the hash bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }
}

impl fmt::Display for LayoutHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex(&self.0, formatter)
    }
}

impl FromStr for LayoutHash {
    type Err = LayoutHashParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_hex(text).map(Self).ok_or(LayoutHashParseError)
    }
}

/// The target language of a project created before its language was chosen:
/// BCP 47 "undetermined". Such a project has no translation language yet.
pub const UNDETERMINED_LANGUAGE: &str = "und";

/// Whether `tag` is a BCP 47 language tag in the form Aeria accepts: a
/// language of 2–3 letters, then optional subtags of 1–8 letters or digits,
/// such as `ru`, `pt-BR`, or `zh-Hant`.
#[must_use]
pub fn is_language_tag(tag: &str) -> bool {
    let mut subtags = tag.split('-');
    let language = subtags.next().unwrap_or_default();
    (2..=3).contains(&language.len())
        && language.bytes().all(|byte| byte.is_ascii_alphabetic())
        && subtags.all(|subtag| {
            (1..=8).contains(&subtag.len())
                && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

/// Whether `tag` names a language a project can translate into: a language
/// tag other than [`UNDETERMINED_LANGUAGE`].
#[must_use]
pub fn is_target_language(tag: &str) -> bool {
    is_language_tag(tag) && !tag.eq_ignore_ascii_case(UNDETERMINED_LANGUAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> GameVersion {
        text.parse().expect("version")
    }

    #[test]
    fn layout_hashes_round_trip_and_reject_other_spellings() {
        let layout = LayoutHash::from_bytes([0xab, 1, 2, 3, 4, 5, 6, 0xff]);
        assert_eq!(layout.to_string(), "ab010203040506ff");
        assert_eq!(layout.to_string().parse(), Ok(layout));
        assert!("AB010203040506FF".parse::<LayoutHash>().is_err());
        assert!("ab01".parse::<LayoutHash>().is_err());
    }

    #[test]
    fn game_versions_compare_by_number() {
        assert!(version("2026.09.15.0000.0000") < version("2026.10.01.0000.0000"));
        assert!(version("2026.9.15.0000.0000") < version("2026.10.01.0000.0000"));
        assert!(version("2026.09.15.0000.0000") < version("2026.09.15.0000.0001"));
        assert_eq!(
            version("2026.09.15.0000.0000").cmp(&version("2026.09.15.0000.0000")),
            Ordering::Equal
        );
        for text in [
            "",
            "2026..15",
            "2026.09.x",
            " 2026.09.15",
            "2026.09.15.",
            "+1.2",
        ] {
            assert!(text.parse::<GameVersion>().is_err(), "{text:?}");
        }
    }

    #[test]
    fn language_tags_follow_bcp_47_and_und_is_no_target() {
        for tag in ["ru", "uk", "pt-BR", "zh-Hant", "es-419", "haw"] {
            assert!(is_target_language(tag), "{tag}");
        }
        for tag in [
            "",
            "r",
            "russian",
            "ru_RU",
            "ru-",
            "-ru",
            "ru-toolongsubtag",
            "und",
            "UND",
        ] {
            assert!(!is_target_language(tag), "{tag}");
        }
        assert!(is_language_tag("und"));
    }
}
