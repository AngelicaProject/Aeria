//! Copying a chosen font or license file into the project.

use std::path::Path;

use swash::{FontRef, StringId};

use crate::error::FontError;
use crate::settings::{FONTS_DIR, is_font_path, project_path, write_atomically};

/// Names read from a font file, to prefill a new source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontDescription {
    pub family: Option<String>,
    pub copyright: Option<String>,
    /// Variation axes as `(tag, min, default, max)`.
    pub axes: Vec<(String, f32, f32, f32)>,
}

/// Reads the family, copyright notice, and axes of a font; `None` when the
/// bytes are not a font.
#[must_use]
pub fn describe_font(bytes: &[u8]) -> Option<FontDescription> {
    let font = FontRef::from_index(bytes, 0)?;
    let name = |id: StringId| {
        font.localized_strings()
            .find_by_id(id, Some("en"))
            .or_else(|| font.localized_strings().find_by_id(id, None))
            .map(|string| string.to_string().trim().to_owned())
            .filter(|string| !string.is_empty())
    };
    Some(FontDescription {
        family: name(StringId::TypographicFamily).or_else(|| name(StringId::Family)),
        copyright: name(StringId::Copyright),
        axes: font
            .variations()
            .map(|axis| {
                let tag = axis.tag().to_be_bytes();
                (
                    String::from_utf8_lossy(&tag).into_owned(),
                    axis.min_value(),
                    axis.default_value(),
                    axis.max_value(),
                )
            })
            .collect(),
    })
}

/// Copies `file` into `fonts/` of the project under its own file name and
/// returns the project-relative path. An existing file of that name is
/// replaced.
///
/// # Errors
/// Returns [`FontError::Settings`] for a file name that cannot be a project
/// path, or [`FontError::Io`].
pub fn import_project_file(project_root: &Path, file: &Path) -> Result<String, FontError> {
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| FontError::Settings("the file name is not valid Unicode".to_owned()))?;
    let relative = format!("{FONTS_DIR}/{name}");
    if !is_font_path(&relative) {
        return Err(FontError::Settings(format!(
            "{name:?} cannot be used as a file name"
        )));
    }
    let bytes = std::fs::read(file).map_err(|source| FontError::Io {
        path: file.to_string_lossy().into_owned(),
        source,
    })?;
    write_atomically(&project_path(project_root, &relative), &bytes)?;
    Ok(relative)
}
