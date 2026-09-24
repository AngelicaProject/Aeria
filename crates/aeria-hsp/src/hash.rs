use sha2::{Digest, Sha256};
use std::fmt::Write as _;

use crate::model::{HspManifest, SourceGuidance};

pub const PACKAGE_HASH_DOMAIN: &str = "HARMONIA-SOURCE-PACKAGE-v1";
pub const GUIDANCE_HASH_DOMAIN: &str = "HARMONIA-SOURCE-GUIDANCE-v1";
pub const EVIDENCE_HASH_DOMAIN: &str = "HARMONIA-SOURCE-GUIDANCE-EVIDENCE-v1";

/// Computes the Atlas logical HSP package identity.
///
/// # Errors
///
/// Returns an error when a canonical framed value exceeds the v1 length limit.
pub fn compute_package_id(manifest: &HspManifest) -> Result<String, String> {
    let mut hasher = CanonicalHasher::new(PACKAGE_HASH_DOMAIN);
    hasher.write_u32(manifest.format_version);
    hasher.write_utf8(&manifest.game_version)?;
    hasher.write_utf8(&manifest.scope)?;
    hasher.write_utf8(&manifest.source.language)?;
    hasher.write_utf8(&manifest.source.content_id)?;
    hasher.write_utf8(&manifest.source.snapshot_id)?;

    // Atlas uses StringComparer.Ordinal for producer-owned identifiers. Rust's
    // lexical ordering is equivalent for the current canonical HSP contract.
    let mut components = manifest.components.iter().collect::<Vec<_>>();
    components.sort_by(|left, right| left.id.cmp(&right.id));
    hasher.write_u32(u32::try_from(components.len()).map_err(|_| "too many components")?);
    for component in components {
        hasher.write_utf8(&component.id)?;
        hasher.write_utf8(&component.kind)?;
        hasher.write_u32(component.format_version);
        hasher.write_byte(u8::from(component.required));
        hasher.write_utf8(&component.path)?;
        hasher.write_i64(component.size);
        hasher.write_utf8(&component.sha256)?;
    }

    Ok(to_hash_string(hasher.finish()))
}

/// Computes the Atlas logical HSG bundle identity.
///
/// # Errors
///
/// Returns an error when a canonical framed value exceeds the v1 length limit.
pub fn compute_guidance_bundle_id(guidance: &SourceGuidance) -> Result<String, String> {
    let mut hasher = CanonicalHasher::new(GUIDANCE_HASH_DOMAIN);
    hasher.write_utf8(&guidance.game_version)?;
    hasher.write_utf8(&guidance.scope)?;
    hasher.write_utf8(&guidance.source.language)?;
    hasher.write_utf8(&guidance.source.content_id)?;
    hasher.write_utf8(&guidance.source.snapshot_id)?;

    let mut evidence_inputs = guidance.evidence_inputs.iter().collect::<Vec<_>>();
    evidence_inputs.sort_by(|left, right| left.language.cmp(&right.language));
    hasher.write_u32(u32::try_from(evidence_inputs.len()).map_err(|_| "too many evidence inputs")?);
    for input in evidence_inputs {
        hasher.write_utf8(&input.language)?;
        hasher.write_utf8(&input.evidence_id)?;
    }

    let mut sheets = guidance.sheets.iter().collect::<Vec<_>>();
    sheets.sort_by(|left, right| left.name.cmp(&right.name));
    hasher.write_u32(u32::try_from(sheets.len()).map_err(|_| "too many guidance sheets")?);
    for sheet in sheets {
        hasher.write_utf8(&sheet.name)?;
        hasher.write_utf8(sheet.status.as_str())?;
        hasher.write_utf8(&sheet.schema_hash)?;

        let mut reasons = sheet.incompatibility_reasons.clone();
        reasons.sort();
        hasher.write_u32(u32::try_from(reasons.len()).map_err(|_| "too many reasons")?);
        for reason in reasons {
            hasher.write_utf8(reason.as_str())?;
        }

        let mut occurrences = sheet.translatable.clone();
        occurrences.sort();
        hasher.write_u32(u32::try_from(occurrences.len()).map_err(|_| "too many occurrences")?);
        for occurrence in occurrences {
            hasher.write_u32(occurrence.row_id);
            hasher.write_u32(u32::from(occurrence.subrow_id));
            hasher.write_u32(occurrence.column_index);
        }
    }

    Ok(to_hash_string(hasher.finish()))
}

pub fn to_hash_string(hash: [u8; 32]) -> String {
    let mut result = String::with_capacity(71);
    result.push_str("sha256:");
    for byte in hash {
        write!(&mut result, "{byte:02x}").expect("writing to a String cannot fail");
    }
    result
}

pub fn is_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn sha256_reader(reader: &mut impl std::io::Read) -> std::io::Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    let mut size = 0_u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(u64::try_from(read).expect("buffer length fits u64"))
            .ok_or_else(|| std::io::Error::other("component size overflow"))?;
        hasher.update(&buffer[..read]);
    }
    let hash: [u8; 32] = hasher.finalize().into();
    Ok((size, to_hash_string(hash)))
}

struct CanonicalHasher {
    bytes: Vec<u8>,
}

impl CanonicalHasher {
    fn new(domain: &str) -> Self {
        Self {
            bytes: domain.as_bytes().to_vec(),
        }
    }

    fn write_byte(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn write_u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_utf8(&mut self, value: &str) -> Result<(), String> {
        let length = u32::try_from(value.len()).map_err(|_| "UTF-8 value is too long")?;
        self.write_u32(length);
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    fn finish(self) -> [u8; 32] {
        Sha256::digest(self.bytes).into()
    }
}

impl crate::model::GuidanceSheetStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Compatible => "compatible",
            Self::Incompatible => "incompatible",
        }
    }
}

impl crate::model::GuidanceIncompatibilityReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::MissingInInput => "missingInInput",
            Self::SheetVariantMismatch => "sheetVariantMismatch",
            Self::ColumnDefinitionMismatch => "columnDefinitionMismatch",
            Self::SchemaHashMismatch => "schemaHashMismatch",
            Self::RowTopologyMismatch => "rowTopologyMismatch",
            Self::UnreadableInInput => "unreadableInInput",
        }
    }
}
