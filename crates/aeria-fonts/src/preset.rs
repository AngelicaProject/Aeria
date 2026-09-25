//! The recommended source fonts, bundled with Aeria.
//!
//! All three are SIL Open Font License 1.1 fonts from `github.com/google/fonts`
//! (`ofl/cormorantsc`, `ofl/oswald`, `ofl/unbounded`). They were chosen and tuned
//! against the native Latin glyphs of each game font size.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::FontError;
use crate::settings::{
    CaseMapping, FONTS_DIR, FontSettings, FontSource, FontTarget, SizeOverride, project_path,
    write_atomically,
};

/// Russian capitals and small letters with Ё, plus `№ « » „ “`.
pub const DEFAULT_CHARACTERS: &str =
    "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя№«»„“";

struct PresetFile {
    name: &'static str,
    bytes: &'static [u8],
}

const FILES: &[PresetFile] = &[
    PresetFile {
        name: "CormorantSC-Medium.ttf",
        bytes: include_bytes!("../presets/CormorantSC-Medium.ttf"),
    },
    PresetFile {
        name: "CormorantSC-SemiBold.ttf",
        bytes: include_bytes!("../presets/CormorantSC-SemiBold.ttf"),
    },
    PresetFile {
        name: "OFL-cormorantsc.txt",
        bytes: include_bytes!("../presets/OFL-cormorantsc.txt"),
    },
    PresetFile {
        name: "Oswald-Variable.ttf",
        bytes: include_bytes!("../presets/Oswald-Variable.ttf"),
    },
    PresetFile {
        name: "OFL-oswald.txt",
        bytes: include_bytes!("../presets/OFL-oswald.txt"),
    },
    PresetFile {
        name: "Unbounded-Variable.ttf",
        bytes: include_bytes!("../presets/Unbounded-Variable.ttf"),
    },
    PresetFile {
        name: "OFL-unbounded.txt",
        bytes: include_bytes!("../presets/OFL-unbounded.txt"),
    },
];

fn source(id: &str, file: &str, family: &str, copyright: &str, license_file: &str) -> FontSource {
    FontSource {
        id: id.to_owned(),
        file: format!("{FONTS_DIR}/{file}"),
        family: family.to_owned(),
        copyright: copyright.to_owned(),
        license: "OFL-1.1".to_owned(),
        license_file: format!("{FONTS_DIR}/{license_file}"),
    }
}

fn axes(values: &[(&str, f64)]) -> BTreeMap<String, f64> {
    values
        .iter()
        .map(|(tag, value)| ((*tag).to_owned(), *value))
        .collect()
}

/// The recommended settings: Cormorant SC for `Jupiter`, Oswald for
/// `TrumpGothic`, and Unbounded for `MiedingerMid`.
#[must_use]
pub fn recommended_settings() -> FontSettings {
    const CORMORANT: &str =
        "Copyright 2015 The Cormorant Project Authors (github.com/CatharsisFonts/Cormorant)";
    FontSettings {
        characters: DEFAULT_CHARACTERS.to_owned(),
        sources: vec![
            source(
                "cormorant-sc-medium",
                "CormorantSC-Medium.ttf",
                "Cormorant SC Medium",
                CORMORANT,
                "OFL-cormorantsc.txt",
            ),
            source(
                "cormorant-sc-semibold",
                "CormorantSC-SemiBold.ttf",
                "Cormorant SC SemiBold",
                CORMORANT,
                "OFL-cormorantsc.txt",
            ),
            source(
                "oswald",
                "Oswald-Variable.ttf",
                "Oswald",
                "Copyright 2016 The Oswald Project Authors (https://github.com/googlefonts/OswaldFont)",
                "OFL-oswald.txt",
            ),
            source(
                "unbounded",
                "Unbounded-Variable.ttf",
                "Unbounded",
                "Copyright 2022 The Unbounded Project Authors (https://github.com/googlefonts/unbounded)",
                "OFL-unbounded.txt",
            ),
        ],
        fonts: vec![
            FontTarget {
                font: "Jupiter".to_owned(),
                source: "cormorant-sc-semibold".to_owned(),
                axes: BTreeMap::new(),
                scale: 1.0,
                width_scale: 0.95,
                baseline_shift: 0,
                tracking: 0.0,
                case_mapping: CaseMapping::None,
                sizes: BTreeMap::from([(
                    "46".to_owned(),
                    SizeOverride {
                        source: Some("cormorant-sc-medium".to_owned()),
                        ..SizeOverride::default()
                    },
                )]),
            },
            FontTarget {
                font: "MiedingerMid".to_owned(),
                source: "unbounded".to_owned(),
                axes: axes(&[("wght", 600.0)]),
                scale: 1.0,
                width_scale: 1.0,
                baseline_shift: 0,
                tracking: 0.05,
                case_mapping: CaseMapping::Upper,
                // The small native sizes are drawn lighter.
                sizes: ["10", "12", "14"]
                    .into_iter()
                    .map(|size| {
                        (
                            size.to_owned(),
                            SizeOverride {
                                axes: Some(axes(&[("wght", 500.0)])),
                                ..SizeOverride::default()
                            },
                        )
                    })
                    .collect(),
            },
            FontTarget {
                font: "TrumpGothic".to_owned(),
                source: "oswald".to_owned(),
                axes: axes(&[("wght", 400.0)]),
                scale: 1.0,
                width_scale: 0.8,
                baseline_shift: 0,
                tracking: 0.0,
                case_mapping: CaseMapping::None,
                sizes: BTreeMap::new(),
            },
        ],
    }
}

/// Writes the bundled font and license files into `fonts/` of the project,
/// replacing files of the same name, and returns the recommended settings.
/// The settings file itself is not written.
///
/// # Errors
/// Returns [`FontError::Io`] when a file cannot be written.
pub fn install_recommended_files(project_root: &Path) -> Result<FontSettings, FontError> {
    for file in FILES {
        write_atomically(
            &project_path(project_root, &format!("{FONTS_DIR}/{}", file.name)),
            file.bytes,
        )?;
    }
    Ok(recommended_settings())
}
