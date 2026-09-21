use std::path::{Path, PathBuf};

use aeria_hxs::HxsSnapshot;
use serde::{Deserialize, Serialize};

/// The HSP v1 manifest source identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HspSourceIdentity {
    pub language: String,
    pub content_id: String,
    pub snapshot_id: String,
}

/// One logical component described by an HSP manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HspComponentDescriptor {
    pub id: String,
    pub kind: String,
    pub format_version: u32,
    pub required: bool,
    pub path: String,
    pub size: i64,
    pub sha256: String,
}

/// The HSP v1 manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HspManifest {
    pub format_version: u32,
    pub package_id: String,
    pub game_version: String,
    pub scope: String,
    pub source: HspSourceIdentity,
    pub components: Vec<HspComponentDescriptor>,
}

/// The source identity embedded in an HSG bundle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceSourceIdentity {
    pub language: String,
    pub content_id: String,
    pub snapshot_id: String,
}

/// One source-language evidence fingerprint used to derive HSG.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceEvidenceInput {
    pub language: String,
    pub evidence_id: String,
}

/// One exact physical String occurrence granted by HSG.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceOccurrence {
    pub row_id: u32,
    pub subrow_id: u16,
    pub column_index: u32,
}

impl<'de> Deserialize<'de> for GuidanceOccurrence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RawOccurrence {
            row_id: u32,
            subrow_id: u16,
            column_index: u32,
        }

        let raw = RawOccurrence::deserialize(deserializer)?;
        Ok(Self {
            row_id: raw.row_id,
            subrow_id: raw.subrow_id,
            column_index: raw.column_index,
        })
    }
}

/// HSG sheet status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GuidanceSheetStatus {
    Compatible,
    Incompatible,
}

/// HSG incompatibility reasons in Atlas contract order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GuidanceIncompatibilityReason {
    MissingInInput,
    SheetVariantMismatch,
    ColumnDefinitionMismatch,
    SchemaHashMismatch,
    RowTopologyMismatch,
}

/// One HSG sheet allowlist.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceSheet {
    pub name: String,
    pub schema_hash: String,
    pub status: GuidanceSheetStatus,
    pub translatable: Vec<GuidanceOccurrence>,
    pub incompatibility_reasons: Vec<GuidanceIncompatibilityReason>,
}

/// Parsed and validated Source Guidance v1.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceGuidance {
    pub format_version: u32,
    pub game_version: String,
    pub scope: String,
    pub bundle_id: String,
    pub source: GuidanceSourceIdentity,
    pub evidence_inputs: Vec<GuidanceEvidenceInput>,
    pub sheets: Vec<GuidanceSheet>,
}

/// A compact positive allowlist used by the editor and mutation layer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GuidanceIndex {
    sheets: std::collections::BTreeMap<String, GuidanceSheetIndex>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GuidanceSheetIndex {
    status: GuidanceSheetStatus,
    schema_hash: String,
    occurrences: Vec<GuidanceOccurrence>,
}

impl GuidanceIndex {
    pub(crate) fn from_guidance(guidance: &SourceGuidance) -> Self {
        let sheets = guidance
            .sheets
            .iter()
            .map(|sheet| {
                (
                    sheet.name.clone(),
                    GuidanceSheetIndex {
                        status: sheet.status,
                        schema_hash: sheet.schema_hash.clone(),
                        occurrences: sheet.translatable.clone(),
                    },
                )
            })
            .collect();
        Self { sheets }
    }

    /// Returns whether an exact HXS String occurrence is granted by HSG.
    #[must_use]
    pub fn is_translatable(
        &self,
        sheet_name: &str,
        row_id: u32,
        subrow_id: u16,
        column_index: u32,
    ) -> bool {
        let Some(sheet) = self.sheets.get(sheet_name) else {
            return false;
        };
        sheet.status == GuidanceSheetStatus::Compatible
            && sheet
                .occurrences
                .binary_search(&GuidanceOccurrence {
                    row_id,
                    subrow_id,
                    column_index,
                })
                .is_ok()
    }

    /// Returns the validated schema hash for a guidance sheet, if present.
    #[must_use]
    pub fn schema_hash(&self, sheet_name: &str) -> Option<&str> {
        self.sheets
            .get(sheet_name)
            .map(|sheet| sheet.schema_hash.as_str())
    }
}

/// A validated HSP and its verified, locally materialized source snapshot.
pub struct SourcePackage {
    package_path: PathBuf,
    package_id: String,
    game_version: String,
    scope: String,
    source: HspSourceIdentity,
    materialized_hxs_path: PathBuf,
    source_snapshot: HxsSnapshot,
    guidance: SourceGuidance,
    guidance_index: GuidanceIndex,
}

impl SourcePackage {
    /// Opens and validates an HSP v1 package using the caller-owned cache root.
    ///
    /// # Errors
    ///
    /// Returns [`crate::HspError`] when the package, embedded HXS/HSG, cache,
    /// or cross-artifact identity checks are invalid.
    pub fn open(
        package_path: impl AsRef<Path>,
        cache_root: impl AsRef<Path>,
    ) -> Result<Self, crate::HspError> {
        crate::reader::open(package_path, cache_root)
    }

    pub(crate) fn new(
        package_path: PathBuf,
        manifest: HspManifest,
        materialized_hxs_path: PathBuf,
        source_snapshot: HxsSnapshot,
        guidance: SourceGuidance,
    ) -> Self {
        let guidance_index = GuidanceIndex::from_guidance(&guidance);
        Self {
            package_path,
            package_id: manifest.package_id,
            game_version: manifest.game_version,
            scope: manifest.scope,
            source: manifest.source,
            materialized_hxs_path,
            source_snapshot,
            guidance,
            guidance_index,
        }
    }

    /// The package path supplied by the caller.
    #[must_use]
    pub fn package_path(&self) -> &Path {
        &self.package_path
    }

    /// Moves the runtime package path after the package has already been
    /// fully validated.
    ///
    /// This only updates the path carried by the validated runtime object. It
    /// does not reopen the archive or repeat any HSP, HXS, or HSG validation.
    #[must_use]
    pub fn relocate_package_path(mut self, package_path: impl Into<PathBuf>) -> Self {
        let package_path = package_path.into();
        crate::reader::relocate_verified_cache(&self.package_path, &package_path);
        self.package_path = package_path;
        self
    }

    /// The logical package identity from the validated manifest.
    #[must_use]
    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    #[must_use]
    pub fn game_version(&self) -> &str {
        &self.game_version
    }

    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    #[must_use]
    pub fn source_language(&self) -> &str {
        &self.source.language
    }

    #[must_use]
    pub fn source_content_id(&self) -> &str {
        &self.source.content_id
    }

    #[must_use]
    pub fn source_snapshot_id(&self) -> &str {
        &self.source.snapshot_id
    }

    /// The disposable cache path of the verified embedded HXS.
    #[must_use]
    pub fn materialized_hxs_path(&self) -> &Path {
        &self.materialized_hxs_path
    }

    /// The verified embedded HXS snapshot.
    #[must_use]
    pub fn source(&self) -> &HxsSnapshot {
        &self.source_snapshot
    }

    /// The validated embedded Source Guidance bundle.
    #[must_use]
    pub fn guidance(&self) -> &SourceGuidance {
        &self.guidance
    }

    /// The compact runtime translation-permission index.
    #[must_use]
    pub fn guidance_index(&self) -> &GuidanceIndex {
        &self.guidance_index
    }
}
