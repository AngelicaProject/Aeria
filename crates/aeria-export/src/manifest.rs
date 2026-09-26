use serde::Serialize;

use crate::error::ExportError;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Testing,
}

/// Which review states a pack contains; chosen per export.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentPolicy {
    /// Only units a person reviewed.
    Reviewed,
    /// Also `draft` and `needs-review` units, marked unreviewed per cell.
    All,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Publisher {
    pub name: String,
    pub url: Option<String>,
}

/// Facts copied from the verified HXS the pack is built from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackSource {
    pub language: String,
    pub game_version: String,
    pub content_id: String,
    pub snapshot_id: String,
}

/// Release metadata of one pack. `sequence` is an explicit export input so
/// identical projects exported for the same release produce identical bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackManifest {
    pub pack_id: String,
    pub title: String,
    pub publisher: Publisher,
    pub license: Option<String>,
    pub sequence: u64,
    pub version: String,
    pub channel: Channel,
    pub target_language: String,
    pub source: PackSource,
    pub content_policy: ContentPolicy,
    pub project_commit: String,
    pub exporter_aeria: String,
    /// `exporter.atlas`: the string dialect the cells were encoded in, such
    /// as `lumina-7.7.0`; packs from Aeria 0.x before the Rust codec hold
    /// the Harmonia Atlas version instead.
    pub exporter_atlas: String,
    pub min_harmonia: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestJson<'a> {
    pack_id: &'a str,
    title: &'a str,
    publisher: PublisherJson<'a>,
    license: Option<&'a str>,
    release: ReleaseJson<'a>,
    target: LanguageJson<'a>,
    source: SourceJson<'a>,
    content_policy: ContentPolicy,
    project: ProjectJson<'a>,
    exporter: ExporterJson<'a>,
    min_harmonia: &'a str,
    counts: CountsJson,
}

#[derive(Serialize)]
struct PublisherJson<'a> {
    name: &'a str,
    url: Option<&'a str>,
}

#[derive(Serialize)]
struct ReleaseJson<'a> {
    sequence: u64,
    version: &'a str,
    channel: Channel,
}

#[derive(Serialize)]
struct LanguageJson<'a> {
    language: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceJson<'a> {
    language: &'a str,
    game_version: &'a str,
    content_id: &'a str,
    snapshot_id: &'a str,
}

#[derive(Serialize)]
struct ProjectJson<'a> {
    commit: &'a str,
}

#[derive(Serialize)]
struct ExporterJson<'a> {
    aeria: &'a str,
    atlas: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CountsJson {
    pub sheets: u64,
    pub rows: u64,
    pub cells: u64,
    pub reviewed_cells: u64,
    pub strings: u64,
}

impl PackManifest {
    pub(crate) fn validate(&self) -> Result<(), ExportError> {
        let fail = |reason: &str| Err(ExportError::Manifest(reason.to_owned()));
        if !is_pack_id(&self.pack_id) {
            return fail("packId must match [a-z0-9][a-z0-9-]{0,63}");
        }
        for (field, value) in [
            ("title", &self.title),
            ("publisher.name", &self.publisher.name),
            ("release.version", &self.version),
            ("target.language", &self.target_language),
            ("source.language", &self.source.language),
            ("source.gameVersion", &self.source.game_version),
            ("exporter.aeria", &self.exporter_aeria),
            ("exporter.atlas", &self.exporter_atlas),
        ] {
            if value.trim().is_empty() || value.trim() != value {
                return Err(ExportError::Manifest(format!(
                    "{field} must be non-empty without surrounding whitespace"
                )));
            }
        }
        for (field, value) in [
            ("publisher.url", &self.publisher.url),
            ("license", &self.license),
        ] {
            if value
                .as_ref()
                .is_some_and(|v| v.trim().is_empty() || v.trim() != v)
            {
                return Err(ExportError::Manifest(format!(
                    "{field} must be absent or non-empty"
                )));
            }
        }
        if self.sequence == 0 || self.sequence > i64::MAX as u64 {
            return fail("release.sequence must be positive");
        }
        if !is_sha256_identity(&self.source.content_id)
            || !is_sha256_identity(&self.source.snapshot_id)
        {
            return fail("source identities must be sha256:<64 lowercase hex>");
        }
        if self.project_commit.len() != 40 || !is_lower_hex(&self.project_commit) {
            return fail("project.commit must be 40 lowercase hex digits");
        }
        if !is_version(&self.min_harmonia) {
            return fail("minHarmonia must be a dotted numeric version");
        }
        Ok(())
    }

    pub(crate) fn to_json(&self, counts: CountsJson) -> Vec<u8> {
        let json = ManifestJson {
            pack_id: &self.pack_id,
            title: &self.title,
            publisher: PublisherJson {
                name: &self.publisher.name,
                url: self.publisher.url.as_deref(),
            },
            license: self.license.as_deref(),
            release: ReleaseJson {
                sequence: self.sequence,
                version: &self.version,
                channel: self.channel,
            },
            target: LanguageJson {
                language: &self.target_language,
            },
            source: SourceJson {
                language: &self.source.language,
                game_version: &self.source.game_version,
                content_id: &self.source.content_id,
                snapshot_id: &self.source.snapshot_id,
            },
            content_policy: self.content_policy,
            project: ProjectJson {
                commit: &self.project_commit,
            },
            exporter: ExporterJson {
                aeria: &self.exporter_aeria,
                atlas: &self.exporter_atlas,
            },
            min_harmonia: &self.min_harmonia,
            counts,
        };
        let mut bytes =
            serde_json::to_vec_pretty(&json).expect("manifest serialization cannot fail");
        bytes.push(b'\n');
        bytes
    }
}

pub(crate) fn is_pack_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0] != b'-'
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_sha256_identity(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && is_lower_hex(hex))
}

// Two to four dot-separated numbers, the form .NET `Version.Parse` accepts.
pub(crate) fn is_version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    (2..=4).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
}
