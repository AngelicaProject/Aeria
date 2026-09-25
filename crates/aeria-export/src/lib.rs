//! Harmonia Pack Format v1 writer.
//!
//! The contract is `docs/formats/pack-v1.md`; the update feed entry is
//! `docs/formats/feed-v1.md`. The writer validates its whole input before it
//! produces a byte, and identical input always produces identical bytes.

mod error;
mod feed;
mod manifest;
mod project;
mod settings;
mod signing;
mod transport;
mod writer;

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

pub use error::ExportError;
pub use feed::{FeedDownload, feed_entry};
pub use manifest::{Channel, ContentPolicy, PackManifest, PackSource, Publisher};
pub use project::{ExportReport, ProjectExport, StringEncoder, collect_project, pack_source};
pub use settings::{PACK_SETTINGS_FILE, PackSettings};
pub use signing::{KeyEndorsement, PackSigner, fingerprint};
pub use transport::{compress_for_transport, write_file_atomically};
pub use writer::{
    BuiltPack, CellState, LayoutColumn, PackCell, PackCounts, PackSheet, SheetVariant,
    source_guard, write_pack, write_pack_with_fonts,
};
