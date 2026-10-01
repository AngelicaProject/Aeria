use serde::Serialize;

use crate::manifest::{Channel, PackManifest};
use crate::writer::BuiltPack;

/// Where and how a release's pack is downloaded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedDownload {
    pub url: String,
    pub brotli: bool,
    pub size: u64,
    pub sha256: [u8; 32],
    pub unpacked_size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryJson<'a> {
    version: String,
    channel: Channel,
    language: &'a str,
    game: GameJson<'a>,
    min_harmonia: &'a str,
    pack_hash: String,
    download: DownloadJson<'a>,
    changelog: Option<&'a str>,
}

#[derive(Serialize)]
struct GameJson<'a> {
    language: &'a str,
    version: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadJson<'a> {
    url: &'a str,
    encoding: &'static str,
    size: u64,
    sha256: String,
    unpacked_size: u64,
}

/// The `releases[]` object of feed v1 for one built pack, published as the
/// release's `feed-entry.json` asset.
///
/// # Panics
/// Never in practice: serializing these plain structs cannot fail.
#[must_use]
pub fn feed_entry(
    manifest: &PackManifest,
    pack: &BuiltPack,
    download: &FeedDownload,
    changelog: Option<&str>,
) -> Vec<u8> {
    let entry = EntryJson {
        version: manifest.version.to_string(),
        channel: manifest.channel,
        language: &manifest.language,
        game: GameJson {
            language: &manifest.game.language,
            version: &manifest.game.version,
        },
        min_harmonia: &manifest.min_harmonia,
        pack_hash: pack.pack_hash_text(),
        download: DownloadJson {
            url: &download.url,
            encoding: if download.brotli { "br" } else { "identity" },
            size: download.size,
            sha256: crate::hex(&download.sha256),
            unpacked_size: download.unpacked_size,
        },
        changelog,
    };
    let mut bytes =
        serde_json::to_vec_pretty(&entry).expect("feed entry serialization cannot fail");
    bytes.push(b'\n');
    bytes
}
