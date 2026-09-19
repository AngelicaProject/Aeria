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
    StringCellHashes, StringOccurrenceCoordinate, StringOccurrenceFingerprint,
    StringOccurrencePage, StringOccurrenceRecord, StringOccurrenceRecordPage, StringRowCoordinate,
    StringRowRecord, StringRowRecordPage,
};

/// Maximum number of rows returned by one [`HxsSnapshot::page_rows`] call.
pub const MAX_ROW_PAGE_SIZE: u32 = 4096;

/// Maximum number of occurrence fingerprints returned by one page.
pub const MAX_STRING_OCCURRENCE_PAGE_SIZE: u32 = 4096;

/// Maximum number of physical row/subrow groups returned by one String-row page.
pub const MAX_STRING_ROW_PAGE_SIZE: u32 = 4096;
