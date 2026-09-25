//! Project-shared font settings (Font Settings v1, `aeria-fonts.json`).
//!
//! See `docs/formats/font-settings-v1.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::FontError;
use crate::targets::game_font;

/// The project-root file name of Font Settings v1.
pub const FONT_SETTINGS_FILE: &str = "aeria-fonts.json";
/// The project directory that holds source font and license files.
pub const FONTS_DIR: &str = "fonts";
const FORMAT_VERSION: u64 = 1;
const MIN_SCALE: f64 = 0.5;
const MAX_SCALE: f64 = 2.0;
const MAX_BASELINE_SHIFT: i32 = 16;
const MIN_TRACKING: f64 = -0.5;
const MAX_TRACKING: f64 = 1.0;

/// Which glyph of the source font draws a character.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaseMapping {
    /// The character's own glyph.
    #[default]
    None,
    /// The glyph of the character's uppercase form, for fonts with capitals only.
    Upper,
}

/// A source font file in the project and its authorship.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontSource {
    pub id: String,
    /// Project-relative path under `fonts/`.
    pub file: String,
    pub family: String,
    pub copyright: String,
    /// SPDX license identifier.
    pub license: String,
    /// Project-relative path of the full license text under `fonts/`.
    pub license_file: String,
}

/// Rendering parameters. In a size override every field is optional and
/// falls back to the font entry.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SizeOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axes: Option<BTreeMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_shift: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking: Option<f64>,
}

/// Glyph generation for one game font.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontTarget {
    /// Game font name, e.g. `Jupiter`.
    pub font: String,
    /// Id of the default source.
    pub source: String,
    /// Variation axis values by four-letter tag.
    #[serde(default)]
    pub axes: BTreeMap<String, f64>,
    /// Multiplies the size at which the source capitals match the native
    /// capital height.
    #[serde(default = "one")]
    pub scale: f64,
    /// Horizontal scale of outlines and advances.
    #[serde(default = "one")]
    pub width_scale: f64,
    /// Pixels to move glyphs down (positive) or up.
    #[serde(default)]
    pub baseline_shift: i32,
    /// Space added to every advance, as a fraction of the native capital
    /// height.
    #[serde(default)]
    pub tracking: f64,
    #[serde(default)]
    pub case_mapping: CaseMapping,
    /// Per-size overrides keyed by the `.fdt` size part.
    #[serde(default)]
    pub sizes: BTreeMap<String, SizeOverride>,
}

fn one() -> f64 {
    1.0
}

/// The effective parameters of one game font size.
#[derive(Clone, Debug, PartialEq)]
pub struct SizeParameters<'a> {
    pub source: &'a FontSource,
    pub axes: &'a BTreeMap<String, f64>,
    pub scale: f64,
    pub width_scale: f64,
    pub baseline_shift: i32,
    pub tracking: f64,
    pub case_mapping: CaseMapping,
}

/// The whole `aeria-fonts.json` document without its `formatVersion`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontSettings {
    /// Characters to add to every target, in the order written.
    pub characters: String,
    pub sources: Vec<FontSource>,
    pub fonts: Vec<FontTarget>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsJson {
    format_version: u64,
    characters: String,
    sources: Vec<FontSource>,
    fonts: Vec<FontTarget>,
}

impl FontSettings {
    /// Reads the settings from `project_root`; `None` when the file does not
    /// exist.
    ///
    /// # Errors
    /// Returns [`FontError::Settings`] for invalid or newer settings and
    /// [`FontError::Io`] when the file cannot be read.
    pub fn load(project_root: &Path) -> Result<Option<Self>, FontError> {
        match fs::read_to_string(project_root.join(FONT_SETTINGS_FILE)) {
            Ok(text) => Self::parse(&text).map(Some),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(FontError::Io {
                path: FONT_SETTINGS_FILE.to_owned(),
                source: error,
            }),
        }
    }

    /// Parses Font Settings v1 JSON. A leading BOM is ignored.
    ///
    /// # Errors
    /// Returns [`FontError::Settings`] for invalid or newer settings.
    pub fn parse(text: &str) -> Result<Self, FontError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| FontError::Settings(error.to_string()))?;
        match value
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64)
        {
            Some(FORMAT_VERSION) => {}
            Some(version) => {
                return Err(FontError::Settings(format!(
                    "unsupported formatVersion {version}; update Aeria"
                )));
            }
            None => {
                return Err(FontError::Settings(
                    "formatVersion must be the integer 1".to_owned(),
                ));
            }
        }
        let json: SettingsJson = serde_json::from_value(value)
            .map_err(|error| FontError::Settings(error.to_string()))?;
        let settings = Self {
            characters: json.characters,
            sources: json.sources,
            fonts: json.fonts,
        };
        settings.validate()?;
        Ok(settings)
    }

    /// Checks every field against the format rules.
    ///
    /// # Errors
    /// Returns [`FontError::Settings`] naming the first invalid field.
    pub fn validate(&self) -> Result<(), FontError> {
        let fail = |reason: String| Err(FontError::Settings(reason));
        if self.characters.is_empty() {
            return fail("characters must not be empty".to_owned());
        }
        let mut seen = BTreeSet::new();
        for character in self.characters.chars() {
            if u32::from(character) < 0x80 || character.is_control() {
                return fail(format!(
                    "character U+{:04X} is ASCII or a control character",
                    u32::from(character)
                ));
            }
            if !seen.insert(character) {
                return fail(format!("character {character} is listed twice"));
            }
        }

        let mut ids = BTreeSet::new();
        for source in &self.sources {
            if !is_id(&source.id) {
                return fail(format!(
                    "source id {:?} must match [a-z0-9][a-z0-9-]{{0,63}}",
                    source.id
                ));
            }
            if !ids.insert(source.id.as_str()) {
                return fail(format!("source id {} is used twice", source.id));
            }
            for (field, value) in [
                ("family", &source.family),
                ("copyright", &source.copyright),
                ("license", &source.license),
            ] {
                if value.trim().is_empty() || value.trim() != value {
                    return fail(format!(
                        "source {} {field} must be non-empty without surrounding whitespace",
                        source.id
                    ));
                }
            }
            for path in [&source.file, &source.license_file] {
                if !is_font_path(path) {
                    return fail(format!(
                        "source {} path {path:?} must be a relative path under {FONTS_DIR}/",
                        source.id
                    ));
                }
            }
        }

        let mut fonts = BTreeSet::new();
        for target in &self.fonts {
            let Some(game) = game_font(&target.font) else {
                return fail(format!("{} is not a supported game font", target.font));
            };
            if !fonts.insert(target.font.as_str()) {
                return fail(format!("{} is listed twice", target.font));
            }
            check_parameters(
                &target.font,
                &ids,
                &target.source,
                &target.axes,
                [target.scale, target.width_scale],
                target.baseline_shift,
                target.tracking,
            )?;
            for size in target.sizes.keys() {
                if game.size(size).is_none() {
                    return fail(format!("{} has no size {size}", target.font));
                }
                let parameters = self.parameters(target, size).ok_or_else(|| {
                    FontError::Settings(format!("{} size {size}: unknown source", target.font))
                })?;
                check_parameters(
                    &format!("{} size {size}", target.font),
                    &ids,
                    &parameters.source.id,
                    parameters.axes,
                    [parameters.scale, parameters.width_scale],
                    parameters.baseline_shift,
                    parameters.tracking,
                )?;
            }
        }
        Ok(())
    }

    /// The source with `id`.
    #[must_use]
    pub fn source(&self, id: &str) -> Option<&FontSource> {
        self.sources.iter().find(|source| source.id == id)
    }

    /// The effective parameters of `size` of `target`; `None` when an
    /// override names an unknown source.
    #[must_use]
    pub fn parameters<'a>(
        &'a self,
        target: &'a FontTarget,
        size: &str,
    ) -> Option<SizeParameters<'a>> {
        let size = target.sizes.get(size);
        let pick = |value: Option<f64>, default: f64| value.unwrap_or(default);
        let source_id = size
            .and_then(|size| size.source.as_deref())
            .unwrap_or(&target.source);
        Some(SizeParameters {
            source: self.source(source_id)?,
            axes: size
                .and_then(|size| size.axes.as_ref())
                .unwrap_or(&target.axes),
            scale: pick(size.and_then(|size| size.scale), target.scale),
            width_scale: pick(size.and_then(|size| size.width_scale), target.width_scale),
            baseline_shift: size
                .and_then(|size| size.baseline_shift)
                .unwrap_or(target.baseline_shift),
            tracking: pick(size.and_then(|size| size.tracking), target.tracking),
            case_mapping: target.case_mapping,
        })
    }

    /// The canonical file contents: two-space indented JSON with a final LF.
    ///
    /// # Panics
    /// Never: serializing these plain structs cannot fail.
    #[must_use]
    pub fn to_canonical_json(&self) -> String {
        let json = SettingsJson {
            format_version: FORMAT_VERSION,
            characters: self.characters.clone(),
            sources: self.sources.clone(),
            fonts: self.fonts.clone(),
        };
        let mut text =
            serde_json::to_string_pretty(&json).expect("settings serialization cannot fail");
        text.push('\n');
        text
    }

    /// Validates and writes the canonical file into `project_root`.
    ///
    /// # Errors
    /// Returns [`FontError::Settings`] for invalid settings or the I/O error.
    pub fn save(&self, project_root: &Path) -> Result<(), FontError> {
        self.validate()?;
        write_atomically(
            &project_root.join(FONT_SETTINGS_FILE),
            self.to_canonical_json().as_bytes(),
        )
    }
}

fn check_parameters(
    what: &str,
    ids: &BTreeSet<&str>,
    source: &str,
    axes: &BTreeMap<String, f64>,
    scales: [f64; 2],
    shift: i32,
    tracking: f64,
) -> Result<(), FontError> {
    let fail = |reason: String| Err(FontError::Settings(format!("{what}: {reason}")));
    if !ids.contains(source) {
        return fail(format!("unknown source {source}"));
    }
    for (tag, value) in axes {
        if tag.len() != 4 || !tag.bytes().all(|b| b.is_ascii_graphic()) {
            return fail(format!("axis tag {tag:?} is invalid"));
        }
        if !value.is_finite() {
            return fail(format!("axis {tag} must be finite"));
        }
    }
    if scales
        .iter()
        .any(|scale| !(MIN_SCALE..=MAX_SCALE).contains(scale))
    {
        return fail(format!(
            "scale and widthScale must be within {MIN_SCALE}..={MAX_SCALE}"
        ));
    }
    if !(-MAX_BASELINE_SHIFT..=MAX_BASELINE_SHIFT).contains(&shift) {
        return fail(format!(
            "baselineShift must be within ±{MAX_BASELINE_SHIFT}"
        ));
    }
    if !(MIN_TRACKING..=MAX_TRACKING).contains(&tracking) {
        return fail(format!(
            "tracking must be within {MIN_TRACKING}..={MAX_TRACKING}"
        ));
    }
    Ok(())
}

/// Resolves a validated project-relative font path.
#[must_use]
pub fn project_path(project_root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(project_root.to_owned(), |path, part| path.join(part))
}

fn is_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// `fonts/<name>` with plain file name segments only.
pub(crate) fn is_font_path(value: &str) -> bool {
    let mut parts = value.split('/');
    parts.next() == Some(FONTS_DIR)
        && parts.clone().next().is_some()
        && parts.all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.chars().all(|c| {
                    !c.is_control() && !matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                })
        })
}

pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), FontError> {
    let io = |source| FontError::Io {
        path: path.to_string_lossy().into_owned(),
        source,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io)?;
    }
    let temp = path.with_extension("aeria-tmp");
    fs::write(&temp, bytes).map_err(io)?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        io(error)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> FontSettings {
        FontSettings {
            characters: "АБЁ№".to_owned(),
            sources: vec![FontSource {
                id: "oswald".to_owned(),
                file: "fonts/Oswald-Variable.ttf".to_owned(),
                family: "Oswald".to_owned(),
                copyright: "Copyright 2016 The Oswald Project Authors".to_owned(),
                license: "OFL-1.1".to_owned(),
                license_file: "fonts/OFL-oswald.txt".to_owned(),
            }],
            fonts: vec![FontTarget {
                font: "TrumpGothic".to_owned(),
                source: "oswald".to_owned(),
                axes: BTreeMap::from([("wght".to_owned(), 500.0)]),
                scale: 1.0,
                width_scale: 0.85,
                baseline_shift: 0,
                tracking: 0.0,
                case_mapping: CaseMapping::None,
                sizes: BTreeMap::from([(
                    "184".to_owned(),
                    SizeOverride {
                        scale: Some(1.1),
                        ..SizeOverride::default()
                    },
                )]),
            }],
        }
    }

    #[test]
    fn settings_round_trip_canonically() {
        let text = settings().to_canonical_json();
        assert!(text.starts_with("{\n  \"formatVersion\": 1,\n  \"characters\": \"АБЁ№\",\n"));
        assert!(text.ends_with("}\n"));
        assert_eq!(FontSettings::parse(&text).expect("parse"), settings());
        assert_eq!(
            FontSettings::parse(&format!("\u{feff}{text}")).expect("parse with BOM"),
            settings()
        );
    }

    #[test]
    fn overrides_fall_back_to_the_font_entry() {
        let settings = settings();
        let target = &settings.fonts[0];
        let small = settings.parameters(target, "184").expect("parameters");
        assert!((small.scale - 1.1).abs() < f64::EPSILON);
        assert!((small.width_scale - 0.85).abs() < f64::EPSILON);
        let large = settings.parameters(target, "68").expect("parameters");
        assert!((large.scale - 1.0).abs() < f64::EPSILON);
        assert_eq!(large.axes.get("wght"), Some(&500.0));
    }

    #[test]
    fn invalid_or_newer_settings_are_rejected() {
        let valid = settings().to_canonical_json();
        for text in [
            valid.replace("\"formatVersion\": 1", "\"formatVersion\": 2"),
            valid.replace("АБЁ№", "АБA"),
            valid.replace("АБЁ№", "ААБ"),
            valid.replace("АБЁ№", ""),
            valid.replace("\"TrumpGothic\"", "\"AXIS\""),
            valid.replace("\"184\"", "\"185\""),
            valid.replace("\"source\": \"oswald\"", "\"source\": \"missing\""),
            valid.replace("fonts/Oswald-Variable.ttf", "../Oswald.ttf"),
            valid.replace("fonts/Oswald-Variable.ttf", "fonts/../x.ttf"),
            valid.replace("\"widthScale\": 0.85", "\"widthScale\": 3.0"),
            valid.replace("\"scale\": 1.1", "\"scale\": 0.1"),
            valid.replace("\"tracking\": 0.0", "\"tracking\": 2.0"),
            valid.replace("\"wght\"", "\"weight\""),
            valid.replace("\"license\"", "\"extra\": 1,\n      \"license\""),
            "[]".to_owned(),
        ] {
            assert!(FontSettings::parse(&text).is_err(), "{text}");
        }
    }

    #[test]
    fn missing_file_means_no_settings() {
        let temp = tempfile::tempdir().expect("temp");
        assert_eq!(FontSettings::load(temp.path()).expect("load"), None);
        settings().save(temp.path()).expect("save");
        assert_eq!(
            FontSettings::load(temp.path()).expect("load"),
            Some(settings())
        );
    }
}
