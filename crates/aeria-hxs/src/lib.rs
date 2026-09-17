//! Read-only HXS access and verification.

#![forbid(unsafe_code)]

mod error;
mod hashing;
mod reader;
mod types;
mod validation;

pub use error::HxsError;
pub use reader::HxsSnapshot;
pub use types::{
    ColumnMetadata, ColumnType, HxsHash, ProducerMetadata, RowHashes, RowPage, RowRecord,
    SheetHashes, SheetMetadata, SheetVariant, SnapshotCounts, SnapshotMetadata, StringCell,
    StringCellHashes,
};
