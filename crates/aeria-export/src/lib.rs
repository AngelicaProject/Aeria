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
mod version;
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
pub use manifest::{Channel, PackGame, PackManifest, Team};
pub use project::{
    ExportReport, ProjectExport, SeStringEncoder, StringEncoder, collect_project, pack_game,
};
pub use settings::{PACK_SETTINGS_FILE, PackSettings};
pub use signing::{KeyEndorsement, PackSigner, fingerprint};
pub use transport::{compress_for_transport, write_file_atomically};
pub use version::{PackVersion, ReleaseDate};
pub use writer::{
    BuiltPack, LayoutColumn, PackCell, PackCounts, PackSheet, SheetVariant, source_guard,
    write_pack, write_pack_with_fonts,
};
