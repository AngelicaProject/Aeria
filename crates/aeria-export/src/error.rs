use thiserror::Error;

/// Reasons a pack cannot be produced. Every variant is an input or
/// environment problem the caller can report; none is recovered by guessing.
#[derive(Debug, Error)]
pub enum ExportError {
    #[error("invalid pack manifest: {0}")]
    Manifest(String),
    #[error("invalid pack settings: {0}")]
    Settings(String),
    #[error("invalid sheet {sheet}: {reason}")]
    Sheet { sheet: String, reason: String },
    #[error("invalid cell {sheet}/{row_id}/{subrow_id}/{column_index}: {reason}")]
    Cell {
        sheet: String,
        row_id: u32,
        subrow_id: u16,
        column_index: u32,
        reason: String,
    },
    #[error("pack exceeds format limits: {0}")]
    Limit(String),
    #[error("string encoder failed: {0}")]
    Encoder(String),
    #[error(transparent)]
    Fonts(#[from] aeria_fonts::FontError),
    #[error("invalid signing key")]
    SigningKey,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
