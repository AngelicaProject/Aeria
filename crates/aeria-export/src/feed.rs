use serde::Serialize;

use crate::manifest::{Channel, ContentPolicy, PackManifest};
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
    sequence: u64,
    version: &'a str,
    channel: Channel,
    pack_hash: String,
    source: SourceJson<'a>,
    target: TargetJson<'a>,
    content_policy: ContentPolicy,
    min_harmonia: &'a str,
    download: DownloadJson<'a>,
    changelog: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceJson<'a> {
    language: &'a str,
    game_version: &'a str,
    content_id: &'a str,
}

#[derive(Serialize)]
struct TargetJson<'a> {
    language: &'a str,
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
        sequence: manifest.sequence,
        version: &manifest.version,
        channel: manifest.channel,
        pack_hash: pack.pack_hash_text(),
        source: SourceJson {
            language: &manifest.source.language,
            game_version: &manifest.source.game_version,
            content_id: &manifest.source.content_id,
        },
        target: TargetJson {
            language: &manifest.target_language,
        },
        content_policy: manifest.content_policy,
        min_harmonia: &manifest.min_harmonia,
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
