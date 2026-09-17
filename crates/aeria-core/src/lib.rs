//! Core domain types and application contracts. Must remain independent of Tauri, persistence implementations, and UI.

#![forbid(unsafe_code)]

use std::fmt;
use std::fmt::Write as _;
use std::str::FromStr;

use sha2::{Digest, Sha256};
use thiserror::Error;

/// The versioned domain separator for newly derived translation-unit IDs.
pub const TRANSLATION_UNIT_ID_DOMAIN: &str = "aeria.translation-unit.v1";

/// The canonical textual prefix for a v1 translation-unit ID.
pub const TRANSLATION_UNIT_ID_PREFIX: &str = "tu1:";

const SHA256_HEX_LENGTH: usize = 64;

/// A SHA-256 digest owned by an Aeria domain value.
///
/// This type deliberately has no dependency on the HXS crate. Workspace
/// adapters copy verified HXS digest bytes into it at the source seam.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Sha256Hash([u8; 32]);

impl Sha256Hash {
    /// Creates a digest from its canonical 32 bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the digest bytes in canonical order.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns lowercase hexadecimal without a prefix.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut result = String::with_capacity(SHA256_HEX_LENGTH);
        for byte in self.0 {
            write!(&mut result, "{byte:02x}").expect("writing to a String cannot fail");
        }
        result
    }
}

impl From<[u8; 32]> for Sha256Hash {
    fn from(bytes: [u8; 32]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl fmt::Display for Sha256Hash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

/// Errors raised when a domain string cannot be constructed safely.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DomainValueError {
    /// A required textual value was empty or whitespace-only.
    #[error("{field} must not be empty or whitespace-only")]
    EmptyValue { field: &'static str },
}

/// A source occurrence coordinate within an HXS snapshot.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceBinding {
    sheet_name: String,
    row_id: u32,
    subrow_id: u16,
    column_index: u32,
}

impl SourceBinding {
    /// Creates a source coordinate.
    ///
    /// Row, subrow, and column zero are valid HXS coordinates. Sheet-name
    /// validity and cell existence are established by the verified HXS reader;
    /// blank and whitespace-only HXS sheet names remain representable here.
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

    /// Returns the HXS sheet name.
    #[must_use]
    pub fn sheet_name(&self) -> &str {
        &self.sheet_name
    }

    /// Returns the HXS row ID.
    #[must_use]
    pub const fn row_id(&self) -> u32 {
        self.row_id
    }

    /// Returns the HXS subrow ID.
    #[must_use]
    pub const fn subrow_id(&self) -> u16 {
        self.subrow_id
    }

    /// Returns the HXS column index.
    #[must_use]
    pub const fn column_index(&self) -> u32 {
        self.column_index
    }
}

/// Verified source hashes required to detect a stale translation binding.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[allow(clippy::struct_field_names)]
pub struct SourceFingerprint {
    macro_text_hash: Sha256Hash,
    raw_value_hash: Option<Sha256Hash>,
    row_technical_hash: Sha256Hash,
}

impl SourceFingerprint {
    /// Creates a source fingerprint from verified HXS digest bytes.
    #[must_use]
    pub const fn new(
        macro_text_hash: Sha256Hash,
        raw_value_hash: Option<Sha256Hash>,
        row_technical_hash: Sha256Hash,
    ) -> Self {
        Self {
            macro_text_hash,
            raw_value_hash,
            row_technical_hash,
        }
    }

    /// Returns the verified macro-text digest.
    #[must_use]
    pub const fn macro_text_hash(&self) -> Sha256Hash {
        self.macro_text_hash
    }

    /// Returns the optional verified raw-value digest.
    #[must_use]
    pub const fn raw_value_hash(&self) -> Option<Sha256Hash> {
        self.raw_value_hash
    }

    /// Returns the verified technical row digest.
    #[must_use]
    pub const fn row_technical_hash(&self) -> Sha256Hash {
        self.row_technical_hash
    }
}

/// Errors raised while deriving a translation-unit identity.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TranslationUnitIdError {
    /// A framed string cannot be represented by the v1 fixed-width length.
    #[error("{field} is too long for translation-unit identity framing")]
    FramingLengthExceeded { field: &'static str },
}

/// The durable identity of one translation unit.
///
/// The ID is derived only when a unit is first created. Rebase/source-update
/// operations retain this value even when the current binding, source text, or
/// source fingerprint changes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TranslationUnitId([u8; 32]);

impl TranslationUnitId {
    /// Derives a v1 ID from source language, source coordinate, and macro hash.
    ///
    /// Raw-value and row-technical hashes are intentionally not part of the
    /// identity. Neither are target-language, target-text, review, snapshot,
    /// content, or game-version values.
    ///
    /// # Panics
    ///
    /// Panics only when an input string exceeds the v1 framing limit. Use
    /// [`Self::try_derive`] for untrusted input.
    #[must_use]
    pub fn derive(
        source_language: &str,
        source_binding: &SourceBinding,
        source_fingerprint: &SourceFingerprint,
    ) -> Self {
        Self::try_derive(source_language, source_binding, source_fingerprint)
            .expect("translation-unit identity inputs fit canonical framing")
    }

    /// Fallible form of [`Self::derive`] for adapters that handle oversized
    /// untrusted metadata without panicking.
    ///
    /// # Errors
    ///
    /// Returns an error when a framed input string exceeds the v1 fixed-width
    /// length limit.
    pub fn try_derive(
        source_language: &str,
        source_binding: &SourceBinding,
        source_fingerprint: &SourceFingerprint,
    ) -> Result<Self, TranslationUnitIdError> {
        let mut hasher = Sha256::new();
        hasher.update(TRANSLATION_UNIT_ID_DOMAIN.as_bytes());
        write_framed_text(&mut hasher, source_language, "source language")?;
        write_framed_text(&mut hasher, source_binding.sheet_name(), "sheet name")?;
        hasher.update(source_binding.row_id().to_le_bytes());
        hasher.update(source_binding.subrow_id().to_le_bytes());
        hasher.update(source_binding.column_index().to_le_bytes());
        hasher.update(source_fingerprint.macro_text_hash().as_bytes());
        Ok(Self(hasher.finalize().into()))
    }

    /// Returns the identity bytes in canonical order.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for TranslationUnitId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(TRANSLATION_UNIT_ID_PREFIX)?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Errors raised while parsing a canonical translation-unit ID.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TranslationUnitIdParseError {
    /// The value does not begin with the supported textual prefix.
    #[error("translation-unit ID must begin with '{TRANSLATION_UNIT_ID_PREFIX}'")]
    InvalidPrefix,

    /// The textual version is not supported.
    #[error("translation-unit ID version is unsupported")]
    UnsupportedVersion,

    /// The hexadecimal digest is not exactly 64 characters.
    #[error("translation-unit ID digest must contain exactly 64 hexadecimal characters")]
    InvalidLength,

    /// The digest contains a non-lowercase hexadecimal character.
    #[error("translation-unit ID digest must use lowercase hexadecimal characters")]
    InvalidHex,
}

impl FromStr for TranslationUnitId {
    type Err = TranslationUnitIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if !value.starts_with("tu") {
            return Err(TranslationUnitIdParseError::InvalidPrefix);
        }
        if !value.starts_with(TRANSLATION_UNIT_ID_PREFIX) {
            return Err(TranslationUnitIdParseError::UnsupportedVersion);
        }
        let hex = &value[TRANSLATION_UNIT_ID_PREFIX.len()..];
        if hex.len() != SHA256_HEX_LENGTH {
            return Err(TranslationUnitIdParseError::InvalidLength);
        }

        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let offset = index * 2;
            *byte = (hex_nibble(hex.as_bytes()[offset])? << 4)
                | hex_nibble(hex.as_bytes()[offset + 1])?;
        }
        Ok(Self(bytes))
    }
}

/// Project/source metadata held once by a workspace.
///
/// A workspace has exactly one target language because this value contains a
/// single canonical target-language field and has no per-unit target-language
/// alternative.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceMetadata {
    source_language: String,
    target_language: String,
    source_content_id: String,
    source_snapshot_id: String,
}

impl WorkspaceMetadata {
    /// Creates validated project/source binding metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when any language or HXS identifier is empty or
    /// whitespace-only.
    pub fn new(
        source_language: impl Into<String>,
        target_language: impl Into<String>,
        source_content_id: impl Into<String>,
        source_snapshot_id: impl Into<String>,
    ) -> Result<Self, DomainValueError> {
        let source_language = source_language.into();
        let target_language = target_language.into();
        let source_content_id = source_content_id.into();
        let source_snapshot_id = source_snapshot_id.into();
        for (value, field) in [
            (&source_language, "source language"),
            (&target_language, "target language"),
            (&source_content_id, "source content ID"),
            (&source_snapshot_id, "source snapshot ID"),
        ] {
            if value.trim().is_empty() {
                return Err(DomainValueError::EmptyValue { field });
            }
        }
        Ok(Self {
            source_language,
            target_language,
            source_content_id,
            source_snapshot_id,
        })
    }

    /// Returns the source language.
    #[must_use]
    pub fn source_language(&self) -> &str {
        &self.source_language
    }

    /// Returns the one canonical target language.
    #[must_use]
    pub fn target_language(&self) -> &str {
        &self.target_language
    }

    /// Returns the current verified HXS content ID.
    #[must_use]
    pub fn source_content_id(&self) -> &str {
        &self.source_content_id
    }

    /// Returns the current verified HXS snapshot ID.
    #[must_use]
    pub fn source_snapshot_id(&self) -> &str {
        &self.source_snapshot_id
    }
}

/// Human review state for a translation unit.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReviewState {
    /// Translation has not received explicit human review.
    Draft,
    /// Translation was explicitly reviewed by a human.
    Reviewed,
    /// A known meaningful source change invalidated prior review.
    NeedsReview,
}

/// One sparse, source-bound translation entry.
///
/// The unit intentionally stores hashes and the current coordinate, not full
/// source macro text. A missing unit in the workspace is distinct from a unit
/// whose target macro string is explicitly empty.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationUnit {
    id: TranslationUnitId,
    source_binding: SourceBinding,
    source_fingerprint: SourceFingerprint,
    target_macro: String,
    review_state: ReviewState,
    translator_note: Option<String>,
}

impl TranslationUnit {
    /// Creates a new draft unit. Target syntax validation belongs to the
    /// `aeria-workspace` seam because it is owned by `aeria-se`.
    #[must_use]
    pub fn new(
        id: TranslationUnitId,
        source_binding: SourceBinding,
        source_fingerprint: SourceFingerprint,
        target_macro: impl Into<String>,
    ) -> Self {
        Self {
            id,
            source_binding,
            source_fingerprint,
            target_macro: target_macro.into(),
            review_state: ReviewState::Draft,
            translator_note: None,
        }
    }

    /// Returns the stable unit ID.
    #[must_use]
    pub const fn id(&self) -> TranslationUnitId {
        self.id
    }

    /// Returns the current source coordinate.
    #[must_use]
    pub const fn source_binding(&self) -> &SourceBinding {
        &self.source_binding
    }

    /// Returns the current source fingerprint.
    #[must_use]
    pub const fn source_fingerprint(&self) -> &SourceFingerprint {
        &self.source_fingerprint
    }

    /// Returns the target macro string, including an explicitly empty string.
    #[must_use]
    pub fn target_macro(&self) -> &str {
        &self.target_macro
    }

    /// Alias for callers using the shorter target terminology.
    #[must_use]
    pub fn target(&self) -> &str {
        self.target_macro()
    }

    /// Returns the current review state.
    #[must_use]
    pub const fn review_state(&self) -> ReviewState {
        self.review_state
    }

    /// Returns the optional translator note.
    #[must_use]
    pub fn translator_note(&self) -> Option<&str> {
        self.translator_note.as_deref()
    }

    /// Replaces the target and resets review state to draft.
    pub fn set_target_macro(&mut self, target_macro: impl Into<String>) {
        let target_macro = target_macro.into();
        if self.target_macro != target_macro {
            self.target_macro = target_macro;
            self.review_state = ReviewState::Draft;
        }
    }

    /// Replaces the optional translator note.
    pub fn set_translator_note(&mut self, translator_note: Option<String>) {
        self.translator_note = translator_note;
    }

    /// Applies an explicit review-state operation.
    pub fn set_review_state(&mut self, review_state: ReviewState) {
        self.review_state = review_state;
    }

    /// Updates source facts after an external process has established a
    /// meaningful source change. The durable ID is intentionally retained.
    pub fn update_source_after_known_change(
        &mut self,
        source_binding: SourceBinding,
        source_fingerprint: SourceFingerprint,
    ) {
        self.source_binding = source_binding;
        self.source_fingerprint = source_fingerprint;
        self.review_state = ReviewState::NeedsReview;
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

fn hex_nibble(value: u8) -> Result<u8, TranslationUnitIdParseError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(TranslationUnitIdParseError::InvalidHex),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ReviewState, Sha256Hash, SourceBinding, SourceFingerprint, TRANSLATION_UNIT_ID_PREFIX,
        TranslationUnit, TranslationUnitId, TranslationUnitIdParseError, WorkspaceMetadata,
    };
    use std::str::FromStr;

    const MACRO_HASH: Sha256Hash = Sha256Hash::from_bytes([
        0x91, 0x75, 0x2f, 0x8c, 0x0c, 0x2c, 0x64, 0xcc, 0x3d, 0x9d, 0x11, 0x53, 0x0f, 0x55, 0x89,
        0x13, 0xe0, 0x3c, 0xcc, 0xf7, 0x99, 0x70, 0xd8, 0x60, 0xd8, 0xf3, 0xb0, 0xc9, 0xe6, 0x60,
        0x48, 0x95,
    ]);
    const RAW_HASH: Sha256Hash = Sha256Hash::from_bytes([0x11; 32]);
    const ROW_HASH: Sha256Hash = Sha256Hash::from_bytes([0x22; 32]);

    fn binding(sheet_name: &str, row_id: u32, subrow_id: u16, column_index: u32) -> SourceBinding {
        SourceBinding::new(sheet_name, row_id, subrow_id, column_index)
    }

    fn fingerprint(macro_text_hash: Sha256Hash) -> SourceFingerprint {
        SourceFingerprint::new(macro_text_hash, Some(RAW_HASH), ROW_HASH)
    }

    #[test]
    fn translation_unit_id_v1_golden_vector_is_stable() {
        let id = TranslationUnitId::derive(
            "en",
            &binding("Synthetic", 42, 0, 0),
            &fingerprint(MACRO_HASH),
        );
        assert_eq!(
            id.to_string(),
            "tu1:77fe2733179bff52bd9c80ccb1008aaa7561b5b490a3e2c9cd134c69ba80cea9"
        );
    }

    #[test]
    fn translation_unit_id_changes_for_identity_inputs_only() {
        let base_binding = binding("Synthetic", 42, 0, 0);
        let base_fingerprint = fingerprint(MACRO_HASH);
        let base = TranslationUnitId::derive("en", &base_binding, &base_fingerprint);

        assert_ne!(
            base,
            TranslationUnitId::derive("ja", &base_binding, &base_fingerprint)
        );
        assert_ne!(
            base,
            TranslationUnitId::derive("en", &binding("Other", 42, 0, 0), &base_fingerprint)
        );
        assert_ne!(
            base,
            TranslationUnitId::derive("en", &binding("Synthetic", 43, 0, 0), &base_fingerprint)
        );
        assert_ne!(
            base,
            TranslationUnitId::derive("en", &binding("Synthetic", 42, 1, 0), &base_fingerprint)
        );
        assert_ne!(
            base,
            TranslationUnitId::derive("en", &binding("Synthetic", 42, 0, 1), &base_fingerprint)
        );
        assert_ne!(
            base,
            TranslationUnitId::derive(
                "en",
                &base_binding,
                &fingerprint(Sha256Hash::from_bytes([0x33; 32]))
            )
        );
        assert_eq!(
            base,
            TranslationUnitId::derive(
                "en",
                &base_binding,
                &SourceFingerprint::new(MACRO_HASH, None, Sha256Hash::from_bytes([0x99; 32]))
            )
        );
    }

    #[test]
    fn translation_unit_id_excludes_project_and_snapshot_metadata() {
        let source_binding = binding("Synthetic", 42, 0, 0);
        let source_fingerprint = fingerprint(MACRO_HASH);
        let first_metadata =
            WorkspaceMetadata::new("en", "fr", "content-a", "snapshot-a").expect("metadata");
        let second_metadata =
            WorkspaceMetadata::new("en", "de", "content-b", "snapshot-b").expect("metadata");

        assert_eq!(
            TranslationUnitId::derive(
                first_metadata.source_language(),
                &source_binding,
                &source_fingerprint,
            ),
            TranslationUnitId::derive(
                second_metadata.source_language(),
                &source_binding,
                &source_fingerprint,
            )
        );
    }

    #[test]
    fn translation_unit_id_display_and_parse_round_trip() {
        let id = TranslationUnitId::derive(
            "en",
            &binding("Synthetic", 42, 0, 0),
            &fingerprint(MACRO_HASH),
        );
        assert_eq!(
            id,
            TranslationUnitId::from_str(&id.to_string()).expect("valid ID")
        );
    }

    #[test]
    fn translation_unit_id_parser_rejects_malformed_values() {
        let valid = TranslationUnitId::derive(
            "en",
            &binding("Synthetic", 42, 0, 0),
            &fingerprint(MACRO_HASH),
        )
        .to_string();
        let cases = [
            ("not-tu", TranslationUnitIdParseError::InvalidPrefix),
            (
                &valid.replacen(TRANSLATION_UNIT_ID_PREFIX, "tu2:", 1),
                TranslationUnitIdParseError::UnsupportedVersion,
            ),
            ("tu1:abcd", TranslationUnitIdParseError::InvalidLength),
            (
                &format!("tu1:{}G", &valid[4..67]),
                TranslationUnitIdParseError::InvalidHex,
            ),
            (
                &format!("tu1:{}A", &valid[4..67]),
                TranslationUnitIdParseError::InvalidHex,
            ),
        ];
        for (value, error) in cases {
            assert_eq!(TranslationUnitId::from_str(value), Err(error));
        }
    }

    #[test]
    fn unit_mutations_follow_review_rules() {
        let binding = binding("Synthetic", 42, 0, 0);
        let fingerprint = fingerprint(MACRO_HASH);
        let id = TranslationUnitId::derive("en", &binding, &fingerprint);
        let mut unit = TranslationUnit::new(id, binding.clone(), fingerprint, "draft");
        unit.set_review_state(ReviewState::Reviewed);
        unit.set_target_macro("");
        assert_eq!(unit.target_macro(), "");
        assert_eq!(unit.review_state(), ReviewState::Draft);
        unit.update_source_after_known_change(binding, fingerprint);
        assert_eq!(unit.review_state(), ReviewState::NeedsReview);
        assert_eq!(unit.id(), id);
    }

    #[test]
    fn workspace_metadata_has_one_target_language() {
        let metadata =
            WorkspaceMetadata::new("en", "fr", "content", "snapshot").expect("valid metadata");
        assert_eq!(metadata.source_language(), "en");
        assert_eq!(metadata.target_language(), "fr");
        assert!(WorkspaceMetadata::new("en", "", "content", "snapshot").is_err());
    }
}
