//! Core domain types and application contracts. Must remain independent of Tauri, persistence implementations, and UI.

#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use sha2::{Digest, Sha256};
use thiserror::Error;

/// The versioned domain separator for newly derived translation-unit IDs.
pub const TRANSLATION_UNIT_ID_DOMAIN: &str = "aeria.translation-unit.v2";

/// Errors raised when a domain string cannot be constructed safely.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DomainValueError {
    /// A required textual value was empty or whitespace-only.
    #[error("{field} must not be empty or whitespace-only")]
    EmptyValue { field: &'static str },
}

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

/// A String cell coordinate: sheet, row, subrow, and column index.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceBinding {
    sheet_name: String,
    row_id: u32,
    subrow_id: u16,
    column_index: u32,
}

impl SourceBinding {
    /// Creates a source coordinate. Whether the cell exists is decided by the
    /// game source.
    #[must_use]
    pub fn new(
        sheet_name: impl Into<String>,
        row_id: u32,
        subrow_id: u16,
        column_index: u32,
    ) -> Self {
        Self {
            sheet_name: sheet_name.into(),
            row_id,
            subrow_id,
            column_index,
        }
    }

    /// Returns the sheet name.
    #[must_use]
    pub fn sheet_name(&self) -> &str {
        &self.sheet_name
    }

    /// Returns the row ID.
    #[must_use]
    pub const fn row_id(&self) -> u32 {
        self.row_id
    }

    /// Returns the subrow ID.
    #[must_use]
    pub const fn subrow_id(&self) -> u16 {
        self.subrow_id
    }

    /// Returns the column index.
    #[must_use]
    pub const fn column_index(&self) -> u32 {
        self.column_index
    }
}

/// Where a unit was last bound and what the source said there.
///
/// `text` and `row_key` are macro texts printed from the game's string
/// bytes. `layout` states which String column layout `binding`'s column
/// index refers to.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceFacts {
    binding: SourceBinding,
    layout: LayoutHash,
    text: String,
    row_key: Option<String>,
}

impl SourceFacts {
    /// Creates source facts.
    #[must_use]
    pub fn new(
        binding: SourceBinding,
        layout: LayoutHash,
        text: impl Into<String>,
        row_key: Option<String>,
    ) -> Self {
        Self {
            binding,
            layout,
            text: text.into(),
            row_key,
        }
    }

    /// Returns the cell coordinate.
    #[must_use]
    pub const fn binding(&self) -> &SourceBinding {
        &self.binding
    }

    /// Returns the sheet layout hash at the binding.
    #[must_use]
    pub const fn layout(&self) -> LayoutHash {
        self.layout
    }

    /// Returns the source text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the row key, when the sheet is keyed.
    #[must_use]
    pub fn row_key(&self) -> Option<&str> {
        self.row_key.as_deref()
    }
}

/// Why a translation unit is no longer bound to a current source cell.
///
/// A detached unit keeps its ID, its last source facts, target, note, and
/// review state. It is not shown at a source cell, is not exported, and is
/// evaluated again by every later source update.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DetachReason {
    /// The unit's sheet no longer exists.
    SheetRemoved,
    /// The unit's sheet exists but cannot be read.
    SheetUnavailable,
    /// The unit's row or subrow no longer exists, or its row key was
    /// removed.
    RowRemoved,
    /// The unit's column is no longer a String column of its sheet.
    CellRemoved,
    /// The sheet layout changed and the unit's column could not be mapped.
    ColumnUnresolved,
    /// The resolved cell is not translatable.
    NotTranslatable,
    /// Another unit deterministically owns the resolved cell.
    BindingConflict,
}

/// Whether a translation unit is attached to a current source cell.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SourceStatus {
    /// The unit's source facts describe the game at the project version.
    #[default]
    Bound,
    /// The unit is preserved without a current source cell.
    Detached(DetachReason),
}

impl SourceStatus {
    /// Returns whether the unit is bound to a current source cell.
    #[must_use]
    pub const fn is_bound(self) -> bool {
        matches!(self, Self::Bound)
    }
}

/// Errors raised while deriving a translation-unit identity.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TranslationUnitIdError {
    /// A framed string cannot be represented by the fixed-width length.
    #[error("{field} is too long for translation-unit identity framing")]
    FramingLengthExceeded { field: &'static str },
}

/// The durable identity of one translation unit: 16 bytes, written as 32
/// lowercase hexadecimal characters.
///
/// The ID is derived only when a unit is created. Source updates keep it
/// when the binding or the source text changes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TranslationUnitId([u8; 16]);

impl TranslationUnitId {
    /// Creates an identity from its bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Derives an ID from a source coordinate and its text.
    ///
    /// # Errors
    ///
    /// Returns an error when the sheet name or text is longer than
    /// `u32::MAX` bytes.
    pub fn derive(binding: &SourceBinding, text: &str) -> Result<Self, TranslationUnitIdError> {
        let mut hasher = Sha256::new();
        hasher.update(TRANSLATION_UNIT_ID_DOMAIN.as_bytes());
        write_framed_text(&mut hasher, binding.sheet_name(), "sheet name")?;
        hasher.update(binding.row_id().to_le_bytes());
        hasher.update(binding.subrow_id().to_le_bytes());
        hasher.update(binding.column_index().to_le_bytes());
        write_framed_text(&mut hasher, text, "source text")?;
        let digest = hasher.finalize();
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&digest[..16]);
        Ok(Self(bytes))
    }

    /// Returns the identity bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for TranslationUnitId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex(&self.0, formatter)
    }
}

/// A translation-unit ID that is not 32 lowercase hexadecimal characters.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("a translation-unit ID is 32 lowercase hexadecimal characters")]
pub struct TranslationUnitIdParseError;

impl FromStr for TranslationUnitId {
    type Err = TranslationUnitIdParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_hex(text).map(Self).ok_or(TranslationUnitIdParseError)
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

/// Project metadata held once by a workspace: one source language, one
/// target language, and the game version the bound units describe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceMetadata {
    source_language: String,
    target_language: String,
    game_version: GameVersion,
}

impl WorkspaceMetadata {
    /// Creates validated project metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when a language is empty or whitespace-only.
    pub fn new(
        source_language: impl Into<String>,
        target_language: impl Into<String>,
        game_version: GameVersion,
    ) -> Result<Self, DomainValueError> {
        let source_language = source_language.into();
        let target_language = target_language.into();
        for (value, field) in [
            (&source_language, "source language"),
            (&target_language, "target language"),
        ] {
            if value.trim().is_empty() {
                return Err(DomainValueError::EmptyValue { field });
            }
        }
        Ok(Self {
            source_language,
            target_language,
            game_version,
        })
    }

    /// Returns the metadata with another target language.
    ///
    /// # Errors
    ///
    /// Returns an error when the language is empty or whitespace-only.
    pub fn with_target_language(
        &self,
        target_language: impl Into<String>,
    ) -> Result<Self, DomainValueError> {
        Self::new(
            self.source_language.clone(),
            target_language,
            self.game_version.clone(),
        )
    }

    /// Returns the metadata with another game version.
    #[must_use]
    pub fn with_game_version(&self, game_version: GameVersion) -> Self {
        Self {
            game_version,
            ..self.clone()
        }
    }

    #[must_use]
    pub fn source_language(&self) -> &str {
        &self.source_language
    }

    #[must_use]
    pub fn target_language(&self) -> &str {
        &self.target_language
    }

    #[must_use]
    pub const fn game_version(&self) -> &GameVersion {
        &self.game_version
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReviewState {
    Draft,
    Reviewed,
    NeedsReview,
}

/// One translation of one source cell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationUnit {
    id: TranslationUnitId,
    source_status: SourceStatus,
    source: SourceFacts,
    target_macro: String,
    review_state: ReviewState,
    translator_note: Option<String>,
}

impl TranslationUnit {
    /// Creates a bound draft unit.
    #[must_use]
    pub fn new(
        id: TranslationUnitId,
        source: SourceFacts,
        target_macro: impl Into<String>,
    ) -> Self {
        Self {
            id,
            source_status: SourceStatus::Bound,
            source,
            target_macro: target_macro.into(),
            review_state: ReviewState::Draft,
            translator_note: None,
        }
    }

    #[must_use]
    pub const fn with_source_status(mut self, source_status: SourceStatus) -> Self {
        self.source_status = source_status;
        self
    }

    #[must_use]
    pub const fn id(&self) -> TranslationUnitId {
        self.id
    }

    #[must_use]
    pub const fn source_status(&self) -> SourceStatus {
        self.source_status
    }

    #[must_use]
    pub const fn is_bound(&self) -> bool {
        self.source_status.is_bound()
    }

    /// Returns the unit's source facts.
    #[must_use]
    pub const fn source(&self) -> &SourceFacts {
        &self.source
    }

    /// Returns the unit's current or last binding.
    #[must_use]
    pub const fn source_binding(&self) -> &SourceBinding {
        self.source.binding()
    }

    #[must_use]
    pub fn target_macro(&self) -> &str {
        &self.target_macro
    }

    #[must_use]
    pub fn target(&self) -> &str {
        self.target_macro()
    }

    #[must_use]
    pub const fn review_state(&self) -> ReviewState {
        self.review_state
    }

    #[must_use]
    pub fn translator_note(&self) -> Option<&str> {
        self.translator_note.as_deref()
    }

    /// Replaces the target; a changed target becomes a draft.
    pub fn set_target_macro(&mut self, target_macro: impl Into<String>) {
        let target_macro = target_macro.into();
        if self.target_macro != target_macro {
            self.target_macro = target_macro;
            self.review_state = ReviewState::Draft;
        }
    }

    pub fn set_translator_note(&mut self, translator_note: Option<String>) {
        self.translator_note = translator_note;
    }

    pub const fn set_review_state(&mut self, review_state: ReviewState) {
        self.review_state = review_state;
    }

    /// Binds the unit to new source facts after a source update. A changed
    /// source text needs review.
    pub fn bind_after_source_update(&mut self, source: SourceFacts, source_changed: bool) {
        self.source_status = SourceStatus::Bound;
        self.source = source;
        if source_changed {
            self.review_state = ReviewState::NeedsReview;
        }
    }

    pub const fn detach(&mut self, reason: DetachReason) {
        self.source_status = SourceStatus::Detached(reason);
    }
}

fn write_framed_text(
    hasher: &mut Sha256,
    value: &str,
    field: &'static str,
) -> Result<(), TranslationUnitIdError> {
    let length = u32::try_from(value.len())
        .map_err(|_| TranslationUnitIdError::FramingLengthExceeded { field })?;
    hasher.update(length.to_le_bytes());
    hasher.update(value.as_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(sheet_name: &str, row_id: u32, subrow_id: u16, column_index: u32) -> SourceBinding {
        SourceBinding::new(sheet_name, row_id, subrow_id, column_index)
    }

    fn derived(binding: &SourceBinding, text: &str) -> TranslationUnitId {
        TranslationUnitId::derive(binding, text).expect("short inputs")
    }

    fn version(text: &str) -> GameVersion {
        text.parse().expect("valid version")
    }

    #[test]
    fn translation_unit_id_golden_vectors_are_stable() {
        assert_eq!(
            derived(&binding("Addon", 42, 0, 0), "Retainer").to_string(),
            "4e8c631995bf7f7d8a74042aefb63b4a"
        );
        assert_eq!(
            derived(&binding("台詞", u32::MAX, u16::MAX, 7), "こんにちは").to_string(),
            "521f521288cc9c69a6180506752a454b"
        );
    }

    #[test]
    fn translation_unit_id_depends_on_every_identity_input() {
        let base = derived(&binding("Addon", 1, 0, 0), "A");
        for other in [
            derived(&binding("Addon2", 1, 0, 0), "A"),
            derived(&binding("Addon", 2, 0, 0), "A"),
            derived(&binding("Addon", 1, 1, 0), "A"),
            derived(&binding("Addon", 1, 0, 1), "A"),
            derived(&binding("Addon", 1, 0, 0), "B"),
            // Framing keeps the sheet name and text apart.
            derived(&binding("AddonA", 1, 0, 0), ""),
        ] {
            assert_ne!(base, other);
        }
    }

    #[test]
    fn translation_unit_ids_round_trip_and_reject_other_spellings() {
        let id = derived(&binding("Addon", 1, 0, 0), "A");
        assert_eq!(id.to_string().parse(), Ok(id));
        for text in [
            "",
            "0123",
            "tu1:0123456789abcdef0123456789abcdef",
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef0123456789abcdefff",
        ] {
            assert_eq!(
                text.parse::<TranslationUnitId>(),
                Err(TranslationUnitIdParseError)
            );
        }
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
    fn unit_mutations_follow_review_rules() {
        let binding = binding("Addon", 1, 0, 0);
        let source = SourceFacts::new(binding.clone(), LayoutHash::from_bytes([1; 8]), "A", None);
        let mut unit = TranslationUnit::new(derived(&binding, "A"), source.clone(), "a");
        unit.set_review_state(ReviewState::Reviewed);
        unit.set_target_macro("a");
        assert_eq!(
            unit.review_state(),
            ReviewState::Reviewed,
            "an identical target keeps review"
        );
        unit.set_target_macro("b");
        assert_eq!(unit.review_state(), ReviewState::Draft);

        unit.set_review_state(ReviewState::Reviewed);
        unit.detach(DetachReason::RowRemoved);
        assert!(!unit.is_bound());
        unit.bind_after_source_update(source.clone(), false);
        assert!(unit.is_bound());
        assert_eq!(unit.review_state(), ReviewState::Reviewed);
        let changed = SourceFacts::new(binding, LayoutHash::from_bytes([1; 8]), "B", None);
        unit.bind_after_source_update(changed, true);
        assert_eq!(unit.review_state(), ReviewState::NeedsReview);
        assert_eq!(unit.source().text(), "B");
    }

    #[test]
    fn workspace_metadata_rejects_empty_languages() {
        let version = version("2026.09.15.0000.0000");
        assert!(WorkspaceMetadata::new("en", " ", version.clone()).is_err());
        let metadata = WorkspaceMetadata::new("en", "ru", version).expect("metadata");
        let updated = metadata.with_game_version(self::version("2026.10.01.0000.0000"));
        let retargeted = updated.with_target_language("uk").expect("language");
        assert_eq!(retargeted.target_language(), "uk");
        assert_eq!(retargeted.game_version(), updated.game_version());
        assert_eq!(updated.game_version().as_str(), "2026.10.01.0000.0000");
        assert_eq!(updated.target_language(), "ru");
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
