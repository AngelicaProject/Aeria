//! Validated Harmonia Source Package and Source Guidance consumption.

#![forbid(unsafe_code)]

mod guidance;
mod hash;
mod model;
mod reader;

pub use guidance::compute_source_evidence_id;
pub use hash::{
    GUIDANCE_HASH_DOMAIN, PACKAGE_HASH_DOMAIN, compute_guidance_bundle_id, compute_package_id,
};
pub use model::{
    GuidanceEvidenceInput, GuidanceIncompatibilityReason, GuidanceIndex, GuidanceOccurrence,
    GuidanceSheet, GuidanceSheetStatus, GuidanceSourceIdentity, HspComponentDescriptor,
    HspManifest, HspSourceIdentity, SourceGuidance, SourcePackage,
};
pub use reader::{open, read_manifest, remove_materialized_source};

/// Maximum decompressed size accepted for the HSP manifest JSON.
pub const MAX_HSP_MANIFEST_BYTES: u64 = 1 << 20;

/// Maximum decompressed size accepted for the HSG JSON component.
pub const MAX_HSP_GUIDANCE_BYTES: u64 = 64 << 20;

use thiserror::Error;

/// Errors returned while opening or validating an HSP v1 package.
#[derive(Debug, Error)]
pub enum HspError {
    #[error("failed to access HSP path {path}: {message}")]
    Io {
        path: std::path::PathBuf,
        message: String,
    },

    #[error("invalid HSP archive: {message}")]
    Archive { message: String },

    #[error("invalid HSP manifest: {message}")]
    Manifest { message: String },

    #[error("invalid HSP component {id:?}: {message}")]
    Component { id: String, message: String },

    #[error("embedded HXS is invalid: {source}")]
    Hxs { source: aeria_hxs::HxsError },

    #[error("embedded HSG is invalid: {message}")]
    Guidance { message: String },

    #[error("HSP source relationships are invalid: {message}")]
    Relationship { message: String },

    #[error("failed to materialize HSP source cache {path}: {message}")]
    Cache {
        path: std::path::PathBuf,
        message: String,
    },
}
