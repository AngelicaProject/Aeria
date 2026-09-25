//! Project-shared pack settings (Pack Settings v1, `aeria-pack.json`).
//!
//! The file is committed with the project so every maintainer exports the
//! same pack identity and the repository's feed workflow can build the feed
//! without Aeria. See `docs/formats/pack-settings-v1.md`.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::ExportError;
use crate::manifest::{Publisher, is_pack_id, is_version};
use crate::transport::write_file_atomically;

/// The project-root file name of Pack Settings v1.
pub const PACK_SETTINGS_FILE: &str = "aeria-pack.json";
const FORMAT_VERSION: u64 = 1;

/// Release-independent pack metadata shared by all maintainers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackSettings {
    pub pack_id: String,
    pub title: String,
    pub publisher: Publisher,
    pub license: Option<String>,
    pub min_harmonia: String,
    /// Fingerprint of the key that signs published packs; `None` until a
    /// key is chosen. Published packs must be signed by exactly this key.
    pub signing_key_fingerprint: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsJson {
    format_version: u64,
    pack_id: String,
    title: String,
    publisher: PublisherJson,
    #[serde(default)]
    license: Option<String>,
    min_harmonia: String,
    #[serde(default)]
    signing_key_fingerprint: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PublisherJson {
    name: String,
    #[serde(default)]
    url: Option<String>,
}

impl PackSettings {
    /// Reads the settings from `project_root`; `None` when the file does not
    /// exist.
    ///
    /// # Errors
    /// Returns [`ExportError::Settings`] for invalid or newer settings and
    /// [`ExportError::Io`] when the file cannot be read.
    pub fn load(project_root: &Path) -> Result<Option<Self>, ExportError> {
        match fs::read_to_string(project_root.join(PACK_SETTINGS_FILE)) {
            Ok(text) => Self::parse(&text).map(Some),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Parses Pack Settings v1 JSON. A leading BOM is ignored.
    ///
    /// # Errors
    /// Returns [`ExportError::Settings`] for invalid or newer settings.
    pub fn parse(text: &str) -> Result<Self, ExportError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| ExportError::Settings(error.to_string()))?;
        match value
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64)
        {
            Some(FORMAT_VERSION) => {}
            Some(version) => {
                return Err(ExportError::Settings(format!(
                    "unsupported formatVersion {version}; update Aeria"
                )));
            }
            None => {
                return Err(ExportError::Settings(
                    "formatVersion must be the integer 1".to_owned(),
                ));
            }
        }
        let json: SettingsJson = serde_json::from_value(value)
            .map_err(|error| ExportError::Settings(error.to_string()))?;
        let settings = Self {
            pack_id: json.pack_id,
            title: json.title,
            publisher: Publisher {
                name: json.publisher.name,
                url: json.publisher.url,
            },
            license: json.license,
            min_harmonia: json.min_harmonia,
            signing_key_fingerprint: json.signing_key_fingerprint,
        };
        settings.validate()?;
        Ok(settings)
    }

    /// Checks every field against the format rules.
    ///
    /// # Errors
    /// Returns [`ExportError::Settings`] naming the first invalid field.
    pub fn validate(&self) -> Result<(), ExportError> {
        let fail = |reason: &str| Err(ExportError::Settings(reason.to_owned()));
        if !is_pack_id(&self.pack_id) {
            return fail("packId must match [a-z0-9][a-z0-9-]{0,63}");
        }
        for (field, value) in [
            ("title", Some(&self.title)),
            ("publisher.name", Some(&self.publisher.name)),
            ("publisher.url", self.publisher.url.as_ref()),
            ("license", self.license.as_ref()),
        ] {
            if value.is_some_and(|v| v.trim().is_empty() || v.trim() != v) {
                return Err(ExportError::Settings(format!(
                    "{field} must be non-empty without surrounding whitespace"
                )));
            }
        }
        if !is_version(&self.min_harmonia) {
            return fail("minHarmonia must be a dotted numeric version");
        }
        if self.signing_key_fingerprint.as_ref().is_some_and(|value| {
            value.len() != 64
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) {
            return fail("signingKeyFingerprint must be 64 lowercase hex digits or null");
        }
        Ok(())
    }

    /// The canonical file contents: two-space indented JSON with a final LF.
    ///
    /// # Panics
    /// Never: serializing these plain structs cannot fail.
    #[must_use]
    pub fn to_canonical_json(&self) -> String {
        let json = SettingsJson {
            format_version: FORMAT_VERSION,
            pack_id: self.pack_id.clone(),
            title: self.title.clone(),
            publisher: PublisherJson {
                name: self.publisher.name.clone(),
                url: self.publisher.url.clone(),
            },
            license: self.license.clone(),
            min_harmonia: self.min_harmonia.clone(),
            signing_key_fingerprint: self.signing_key_fingerprint.clone(),
        };
        let mut text =
            serde_json::to_string_pretty(&json).expect("settings serialization cannot fail");
        text.push('\n');
        text
    }

    /// Validates and writes the canonical file into `project_root`.
    ///
    /// # Errors
    /// Returns [`ExportError::Settings`] for invalid settings or the I/O error.
    pub fn save(&self, project_root: &Path) -> Result<(), ExportError> {
        self.validate()?;
        write_file_atomically(
            &project_root.join(PACK_SETTINGS_FILE),
            self.to_canonical_json().as_bytes(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> PackSettings {
        PackSettings {
            pack_id: "ru-main".to_owned(),
            title: "Русский перевод".to_owned(),
            publisher: Publisher {
                name: "Example team".to_owned(),
                url: None,
            },
            license: Some("CC-BY-NC-SA-4.0".to_owned()),
            min_harmonia: "1.0.0".to_owned(),
            signing_key_fingerprint: Some("ab".repeat(32)),
        }
    }

    #[test]
    fn settings_round_trip_canonically() {
        let text = settings().to_canonical_json();
        assert_eq!(
            text,
            format!(
                "{{\n  \"formatVersion\": 1,\n  \"packId\": \"ru-main\",\n  \"title\": \"Русский перевод\",\n  \"publisher\": {{\n    \"name\": \"Example team\",\n    \"url\": null\n  }},\n  \"license\": \"CC-BY-NC-SA-4.0\",\n  \"minHarmonia\": \"1.0.0\",\n  \"signingKeyFingerprint\": \"{}\"\n}}\n",
                "ab".repeat(32)
            )
        );
        assert_eq!(PackSettings::parse(&text).expect("parse"), settings());
        assert_eq!(
            PackSettings::parse(&format!("\u{feff}{text}")).expect("parse with BOM"),
            settings()
        );
    }

    #[test]
    fn invalid_or_newer_settings_are_rejected() {
        let valid = settings().to_canonical_json();
        for text in [
            valid.replace("\"formatVersion\": 1", "\"formatVersion\": 2"),
            valid.replace("ru-main", "RU"),
            valid.replace("1.0.0", "1"),
            valid.replace("Example team", " "),
            valid.replace(&"ab".repeat(32), "AB"),
            valid.replace("\"license\"", "\"extra\": 1,\n  \"license\""),
            valid.replace("  \"minHarmonia\": \"1.0.0\",\n", ""),
            "[]".to_owned(),
        ] {
            assert!(PackSettings::parse(&text).is_err(), "{text}");
        }
    }

    #[test]
    fn missing_file_means_no_settings() {
        let temp = tempfile::tempdir().expect("temp");
        assert_eq!(PackSettings::load(temp.path()).expect("load"), None);
        settings().save(temp.path()).expect("save");
        assert_eq!(
            PackSettings::load(temp.path()).expect("load"),
            Some(settings())
        );
    }
}
