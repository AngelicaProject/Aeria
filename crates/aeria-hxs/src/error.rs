use thiserror::Error;

/// Errors returned while opening, verifying, or reading an HXS snapshot.
#[derive(Debug, Error)]
pub enum HxsError {
    #[error("failed to access HXS storage: {message}")]
    Storage { message: String },

    #[error("unsupported HXS SQLite application id: expected 0x{expected:08x}, found {found}")]
    UnsupportedApplicationId { expected: u32, found: i64 },

    #[error("unsupported HXS format version: expected {expected}, found {found}")]
    UnsupportedFormatVersion { expected: i64, found: i64 },

    #[error("invalid HXS schema: {message}")]
    InvalidSchema { message: String },

    #[error("invalid HXS data: {message}")]
    InvalidData { message: String },

    #[error("invalid HXS request: {message}")]
    InvalidRequest { message: String },

    #[error("HXS sheet was not found: {name}")]
    SheetNotFound { name: String },
}

impl HxsError {
    pub(crate) fn storage(error: impl std::fmt::Display) -> Self {
        Self::Storage {
            message: error.to_string(),
        }
    }

    pub(crate) fn schema(message: impl Into<String>) -> Self {
        Self::InvalidSchema {
            message: message.into(),
        }
    }

    pub(crate) fn data(message: impl Into<String>) -> Self {
        Self::InvalidData {
            message: message.into(),
        }
    }

    pub(crate) fn request(message: impl Into<String>) -> Self {
        Self::InvalidRequest {
            message: message.into(),
        }
    }
}
