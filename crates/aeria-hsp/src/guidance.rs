use std::cmp::Ordering;

use aeria_hxs::{
    HxsError, HxsSnapshot, MAX_EVIDENCE_STRING_ROW_PAGE_SIZE, SheetVariant, StringRowCoordinate,
};
use sha2::{Digest, Sha256};

use crate::hash::{compute_guidance_bundle_id, is_sha256, to_hash_string};
use crate::model::{GuidanceSheetStatus, SourceGuidance};

pub(crate) fn parse_and_validate(bytes: &[u8]) -> Result<SourceGuidance, String> {
    let guidance: SourceGuidance = serde_json::from_slice(bytes)
        .map_err(|error| format!("source guidance JSON is invalid: {error}"))?;
    validate(&guidance)?;
    Ok(guidance)
}

pub(crate) fn validate(guidance: &SourceGuidance) -> Result<(), String> {
    if guidance.format_version != 1
        || guidance.game_version.trim().is_empty()
        || guidance.scope.trim().is_empty()
        || !is_sha256(&guidance.bundle_id)
    {
        return Err("source guidance metadata is invalid".to_owned());
    }
    if !is_canonical_language(&guidance.source.language)
        || !is_sha256(&guidance.source.content_id)
        || !is_sha256(&guidance.source.snapshot_id)
    {
        return Err("source guidance source identity is invalid".to_owned());
    }

    validate_evidence_inputs(guidance)?;
    let mut previous_sheet = None;
    for sheet in &guidance.sheets {
        if sheet.name.trim().is_empty() || !is_sha256(&sheet.schema_hash) {
            return Err(format!("source guidance sheet {:?} is invalid", sheet.name));
        }
        if previous_sheet.is_some_and(|previous| previous >= sheet.name.as_str()) {
            return Err("source guidance sheets are not in ordinal order".to_owned());
        }
        previous_sheet = Some(sheet.name.as_str());
        validate_sheet(sheet)?;
    }

    let expected = compute_guidance_bundle_id(guidance)
        .map_err(|error| format!("source guidance cannot be canonically hashed: {error}"))?;
    if guidance.bundle_id != expected {
        return Err("source guidance bundleId does not match its canonical content".to_owned());
    }
    Ok(())
}

pub(crate) fn validate_relationships(
    guidance: &SourceGuidance,
    source: &HxsSnapshot,
) -> Result<(), HxsError> {
    let metadata = source.metadata();
    if guidance.game_version != metadata.game_version
        || guidance.scope != metadata.scope
        || guidance.source.language != metadata.source_language
        || guidance.source.content_id != metadata.content_id
        || guidance.source.snapshot_id != metadata.snapshot_id
    {
        return Err(HxsError::InvalidData {
            message: "embedded source guidance does not apply to the embedded HXS".to_owned(),
        });
    }

    for sheet in guidance
        .sheets
        .iter()
        .filter(|sheet| sheet.status == GuidanceSheetStatus::Compatible)
    {
        let Some(source_sheet) = source.sheet(&sheet.name) else {
            return Err(HxsError::InvalidData {
                message: format!("compatible HSG sheet {:?} is absent from HXS", sheet.name),
            });
        };
        let expected = format!("sha256:{}", source_sheet.hashes.schema.to_hex());
        if sheet.schema_hash != expected {
            return Err(HxsError::InvalidData {
                message: format!(
                    "compatible HSG sheet {:?} has a schema mismatch",
                    sheet.name
                ),
            });
        }
    }

    let source_evidence = guidance
        .evidence_inputs
        .iter()
        .find(|input| input.language == metadata.source_language)
        .ok_or_else(|| HxsError::InvalidData {
            message: "source guidance has no source-language evidence input".to_owned(),
        })?;
    let expected_evidence = compute_source_evidence_id(source)?;
    if source_evidence.evidence_id != expected_evidence {
        return Err(HxsError::InvalidData {
            message: "source guidance evidence does not match the embedded HXS".to_owned(),
        });
    }
    Ok(())
}

fn validate_evidence_inputs(guidance: &SourceGuidance) -> Result<(), String> {
    if guidance.evidence_inputs.len() < 2 {
        return Err("source guidance requires at least two evidence inputs".to_owned());
    }
    let mut languages = std::collections::HashSet::new();
    let mut previous = None;
    let mut source_count = 0;
    for input in &guidance.evidence_inputs {
        if !is_canonical_language(&input.language)
            || !is_sha256(&input.evidence_id)
            || !languages.insert(input.language.as_str())
            || previous.is_some_and(|previous| previous >= input.language.as_str())
        {
            return Err(
                "source guidance evidence inputs are invalid or not in ordinal order".to_owned(),
            );
        }
        if input.language == guidance.source.language {
            source_count += 1;
        }
        previous = Some(input.language.as_str());
    }
    if source_count != 1 {
        return Err(
            "source guidance evidence inputs must contain the source language exactly once"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_sheet(sheet: &crate::model::GuidanceSheet) -> Result<(), String> {
    if sheet.status == GuidanceSheetStatus::Compatible && !sheet.incompatibility_reasons.is_empty()
    {
        return Err(format!(
            "compatible sheet {:?} has incompatibility reasons",
            sheet.name
        ));
    }
    if sheet.status == GuidanceSheetStatus::Incompatible
        && (sheet.incompatibility_reasons.is_empty() || !sheet.translatable.is_empty())
    {
        return Err(format!(
            "incompatible sheet {:?} has invalid allowlist state",
            sheet.name
        ));
    }
    let mut previous_reason = None;
    for reason in &sheet.incompatibility_reasons {
        if previous_reason.is_some_and(|previous| previous >= *reason) {
            return Err(format!("sheet {:?} has unordered reasons", sheet.name));
        }
        previous_reason = Some(*reason);
    }
    for pair in sheet.translatable.windows(2) {
        if pair[0].cmp(&pair[1]) != Ordering::Less {
            return Err(format!("sheet {:?} has unordered occurrences", sheet.name));
        }
    }
    Ok(())
}

fn is_canonical_language(language: &str) -> bool {
    matches!(
        language,
        "en" | "ja" | "de" | "fr" | "zh-cn" | "zh-tw" | "ko"
    )
}

struct EvidenceHasher {
    hash: Sha256,
    current_sheet: bool,
    current_row: bool,
    previous_sheet: Option<String>,
    previous_row: Option<(u32, u16)>,
    previous_column: Option<u32>,
}

impl EvidenceHasher {
    fn new(game_version: &str, scope: &str, language: &str) -> Result<Self, String> {
        let mut result = Self {
            hash: Sha256::new(),
            current_sheet: false,
            current_row: false,
            previous_sheet: None,
            previous_row: None,
            previous_column: None,
        };
        result
            .hash
            .update(crate::hash::EVIDENCE_HASH_DOMAIN.as_bytes());
        result.write_utf8(game_version)?;
        result.write_utf8(scope)?;
        result.write_utf8(language)?;
        Ok(result)
    }

    fn add_sheet(
        &mut self,
        name: &str,
        variant: SheetVariant,
        schema: &[u8; 32],
    ) -> Result<(), String> {
        if self
            .previous_sheet
            .as_ref()
            .is_some_and(|previous| previous.as_str() >= name)
        {
            return Err("HXS sheets are not in ordinal evidence order".to_owned());
        }
        self.finish_sheet();
        self.write_byte(0x01);
        self.write_utf8(name)?;
        self.write_u32(match variant {
            SheetVariant::DefaultRows => 0,
            SheetVariant::Subrows => 1,
        });
        self.write_u32(32);
        self.hash.update(schema);
        self.previous_sheet = Some(name.to_owned());
        self.current_sheet = true;
        self.previous_row = None;
        Ok(())
    }

    fn add_row(&mut self, row_id: u32, subrow_id: u16) -> Result<(), String> {
        if !self.current_sheet {
            return Err("evidence row has no sheet".to_owned());
        }
        if self
            .previous_row
            .is_some_and(|previous| (row_id, subrow_id) <= previous)
        {
            return Err("HXS rows are not in evidence order".to_owned());
        }
        self.finish_row();
        self.write_byte(0x02);
        self.write_u32(row_id);
        self.write_u32(u32::from(subrow_id));
        self.previous_row = Some((row_id, subrow_id));
        self.previous_column = None;
        self.current_row = true;
        Ok(())
    }

    fn add_occurrence(&mut self, column_index: u32, macro_text: &str) -> Result<(), String> {
        if !self.current_row
            || self
                .previous_column
                .is_some_and(|previous| column_index <= previous)
        {
            return Err("HXS String occurrences are not in evidence order".to_owned());
        }
        self.write_byte(0x03);
        self.write_u32(column_index);
        self.write_utf8(macro_text)?;
        self.previous_column = Some(column_index);
        Ok(())
    }

    fn finish(mut self) -> String {
        self.finish_sheet();
        to_hash_string(self.hash.finalize().into())
    }

    fn finish_sheet(&mut self) {
        self.finish_row();
        if self.current_sheet {
            self.write_byte(0x05);
            self.current_sheet = false;
        }
    }

    fn finish_row(&mut self) {
        if self.current_row {
            self.write_byte(0x04);
            self.current_row = false;
        }
    }

    fn write_byte(&mut self, value: u8) {
        self.hash.update([value]);
    }

    fn write_u32(&mut self, value: u32) {
        self.hash.update(value.to_le_bytes());
    }

    fn write_utf8(&mut self, value: &str) -> Result<(), String> {
        self.write_u32(u32::try_from(value.len()).map_err(|_| "evidence text is too long")?);
        self.hash.update(value.as_bytes());
        Ok(())
    }
}

/// Computes the Atlas source evidence identity from a verified HXS stream.
///
/// # Errors
///
/// Returns the underlying HXS read/validation error when the source cannot be
/// scanned consistently.
pub fn compute_source_evidence_id(snapshot: &HxsSnapshot) -> Result<String, HxsError> {
    let metadata = snapshot.metadata();
    let mut hasher = EvidenceHasher::new(
        &metadata.game_version,
        &metadata.scope,
        &metadata.source_language,
    )
    .map_err(|message| HxsError::InvalidData { message })?;

    // Atlas uses StringComparer.Ordinal for these producer-owned names. Rust's
    // lexical ordering is equivalent for the current canonical HSP contract.
    let mut sheets = snapshot.sheets();
    sheets.sort_by(|left, right| left.name.cmp(&right.name));
    for sheet in sheets {
        hasher
            .add_sheet(&sheet.name, sheet.variant, sheet.hashes.schema.as_bytes())
            .map_err(|message| HxsError::InvalidData { message })?;
        let mut after: Option<StringRowCoordinate> = None;
        loop {
            let page = snapshot.page_evidence_string_rows(
                &sheet.name,
                after.as_ref(),
                MAX_EVIDENCE_STRING_ROW_PAGE_SIZE,
            )?;
            for row in &page.rows {
                hasher
                    .add_row(row.row_id, row.subrow_id)
                    .map_err(|message| HxsError::InvalidData { message })?;
                for occurrence in &row.occurrences {
                    hasher
                        .add_occurrence(occurrence.column_index, &occurrence.macro_text)
                        .map_err(|message| HxsError::InvalidData { message })?;
                }
            }
            let Some(next) = page.next_after else {
                break;
            };
            after = Some(next);
        }
    }
    Ok(hasher.finish())
}
