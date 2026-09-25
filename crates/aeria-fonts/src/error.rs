use thiserror::Error;

/// Reasons font glyphs cannot be generated. None is recovered by guessing.
#[derive(Debug, Error)]
pub enum FontError {
    #[error("invalid font settings: {0}")]
    Settings(String),
    #[error("font source {source_id}: {reason}")]
    Source { source_id: String, reason: String },
    #[error("{font}_{size}: {reason}")]
    Glyph {
        font: String,
        size: String,
        reason: String,
    },
    #[error("invalid FONTS section: {0}")]
    Section(String),
    #[error("could not access {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
