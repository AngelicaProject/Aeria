use serde::Serialize;

use crate::error::ExportError;
use crate::version::PackVersion;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Testing,
}

/// The team that publishes the pack: the name players see.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Team {
    pub name: String,
    pub url: Option<String>,
}

/// The game the pack is built from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackGame {
    /// The client language the project translates from.
    pub language: String,
    /// The text of `game/ffxivgame.ver`.
    pub version: String,
}

/// Release metadata of one pack. `version` is an explicit export input so
/// identical projects exported for the same release produce identical bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackManifest {
    pub pack_id: String,
    pub title: String,
    pub team: Team,
    /// The people credited in the pack, in the order the project lists them.
    pub authors: Vec<String>,
    pub license: Option<String>,
    pub version: PackVersion,
    pub channel: Channel,
    /// The language of the translations.
    pub language: String,
    pub game: PackGame,
    /// The producing Aeria version.
    pub aeria: String,
    /// The Git commit of the exported project.
    pub commit: String,
    pub min_harmonia: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestJson<'a> {
    pack_id: &'a str,
    title: &'a str,
    team: TeamJson<'a>,
    authors: &'a [String],
    license: Option<&'a str>,
    version: String,
    channel: Channel,
    language: &'a str,
    game: GameJson<'a>,
    built: BuiltJson<'a>,
    min_harmonia: &'a str,
}

#[derive(Serialize)]
struct TeamJson<'a> {
    name: &'a str,
    url: Option<&'a str>,
}

#[derive(Serialize)]
struct GameJson<'a> {
    language: &'a str,
    version: &'a str,
}

#[derive(Serialize)]
struct BuiltJson<'a> {
    aeria: &'a str,
    commit: &'a str,
}

impl PackManifest {
    pub(crate) fn validate(&self) -> Result<(), ExportError> {
        let fail = |reason: &str| Err(ExportError::Manifest(reason.to_owned()));
        if !is_pack_id(&self.pack_id) {
            return fail("packId must match [a-z0-9][a-z0-9-]{0,63}");
        }
        for (field, value) in [
            ("title", &self.title),
            ("team.name", &self.team.name),
            ("language", &self.language),
            ("game.language", &self.game.language),
            ("game.version", &self.game.version),
            ("built.aeria", &self.aeria),
        ] {
            if !is_display_text(value) {
                return Err(ExportError::Manifest(format!(
                    "{field} must be non-empty without surrounding whitespace"
                )));
            }
        }
        for (field, value) in [("team.url", &self.team.url), ("license", &self.license)] {
            if value.as_ref().is_some_and(|v| !is_display_text(v)) {
                return Err(ExportError::Manifest(format!(
                    "{field} must be absent or non-empty"
                )));
            }
        }
        if let Err(reason) = check_authors(&self.authors) {
            return fail(&reason);
        }
        if !aeria_core::is_target_language(&self.language) {
            return fail(
                "language must be a BCP 47 language tag other than und; choose the project's target language",
            );
        }
        if self.commit.len() != 40 || !is_lower_hex(&self.commit) {
            return fail("built.commit must be 40 lowercase hex digits");
        }
        if !is_version(&self.min_harmonia) {
            return fail("minHarmonia must be a dotted numeric version");
        }
        Ok(())
    }

    pub(crate) fn to_json(&self) -> Vec<u8> {
        let json = ManifestJson {
            pack_id: &self.pack_id,
            title: &self.title,
            team: TeamJson {
                name: &self.team.name,
                url: self.team.url.as_deref(),
            },
            authors: &self.authors,
            license: self.license.as_deref(),
            version: self.version.to_string(),
            channel: self.channel,
            language: &self.language,
            game: GameJson {
                language: &self.game.language,
                version: &self.game.version,
            },
            built: BuiltJson {
                aeria: &self.aeria,
                commit: &self.commit,
            },
            min_harmonia: &self.min_harmonia,
        };
        let mut bytes =
            serde_json::to_vec_pretty(&json).expect("manifest serialization cannot fail");
        bytes.push(b'\n');
        bytes
    }
}

/// Non-empty, without surrounding whitespace.
pub(crate) fn is_display_text(value: &str) -> bool {
    !value.trim().is_empty() && value.trim() == value
}

/// Each author is display text, and none appears twice.
pub(crate) fn check_authors(authors: &[String]) -> Result<(), String> {
    for (index, author) in authors.iter().enumerate() {
        if !is_display_text(author) {
            return Err("authors must be non-empty without surrounding whitespace".to_owned());
        }
        if authors[..index].contains(author) {
            return Err(format!("authors lists {author:?} twice"));
        }
    }
    Ok(())
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

// Two to four dot-separated numbers, the form .NET `Version.Parse` accepts.
pub(crate) fn is_version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    (2..=4).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
}
