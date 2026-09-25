//! Font settings for the glyphs Harmonia adds to game fonts.
//!
//! The settings live in the committed `aeria-fonts.json` and the source fonts
//! in `fonts/`; see `docs/formats/font-settings-v1.md` and
//! `docs/architecture/export.md`.

use std::path::{Path, PathBuf};

use aeria_fonts::{
    FONT_SETTINGS_FILE, FONTS_DIR, FontSettings, GAME_FONTS, character_set, describe_font,
    game_font, import_project_file, install_recommended_files, load_source, preview_line,
    project_path, render_size,
};
use aeria_git::GitRepository;
use serde::Serialize;
use tauri::Manager;

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontsOverviewDto {
    pub settings: Option<FontSettings>,
    /// Why `aeria-fonts.json` exists but cannot be used.
    pub settings_error: Option<String>,
    /// `aeria-fonts.json` or `fonts/` has uncommitted changes.
    pub uncommitted: bool,
    pub game_fonts: Vec<GameFontDto>,
    /// What the files of the configured sources contain.
    pub sources: Vec<SourceFileDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameFontDto {
    pub name: String,
    pub sizes: Vec<GameFontSizeDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameFontSizeDto {
    pub size: String,
    pub line_height: u8,
    pub ascent: u8,
    pub cap_height: u8,
    pub cap_advance: f32,
    pub space_advance: u8,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFileDto {
    pub id: String,
    /// Why the file cannot be used, or `None`.
    pub error: Option<String>,
    pub axes: Vec<AxisDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AxisDto {
    pub tag: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

/// A font file copied into `fonts/`, with what could be read from it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedFileDto {
    pub file: String,
    pub family: Option<String>,
    pub copyright: Option<String>,
    /// The file is a font.
    pub is_font: bool,
}

/// One size of the preview.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSizeDto {
    pub size: String,
    pub line_height: u8,
    pub ascent: u8,
    pub cap_height: u8,
    pub cap_advance: f32,
    /// Mean advance of the generated capitals А–Я, when there are any.
    pub generated_cap_advance: Option<f32>,
    pub width: u32,
    pub height: u32,
    /// 8-bit coverage, row-major.
    pub pixels: Vec<u8>,
    pub missing: Vec<String>,
    pub error: Option<String>,
}

fn project_root(state: &DesktopState) -> CommandResult<PathBuf> {
    let project = state.lock_project()?;
    let session = project.as_ref().ok_or_else(CommandError::no_project)?;
    Ok(session.repository_root().to_owned())
}

fn font_error(error: &aeria_fonts::FontError) -> CommandError {
    let code = match error {
        aeria_fonts::FontError::Settings(_) => "fontSettingsInvalid",
        aeria_fonts::FontError::Io { .. } => "exportIo",
        _ => "fontsFailed",
    };
    CommandError::new(code, error.to_string())
}

/// Whether `path` from `git status` belongs to the font settings.
pub(crate) fn is_font_path(path: &str) -> bool {
    path == FONT_SETTINGS_FILE || path.starts_with(&format!("{FONTS_DIR}/"))
}

fn source_files(root: &Path, settings: &FontSettings) -> Vec<SourceFileDto> {
    settings
        .sources
        .iter()
        .map(|source| {
            let loaded = load_source(root, source);
            let description = loaded
                .as_ref()
                .ok()
                .and_then(|loaded| describe_font(&loaded.bytes));
            SourceFileDto {
                id: source.id.clone(),
                error: loaded.err().map(|error| error.to_string()),
                axes: description
                    .map(|description| {
                        description
                            .axes
                            .into_iter()
                            .map(|(tag, min, default, max)| AxisDto {
                                tag,
                                min,
                                default,
                                max,
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect()
}

fn overview(state: &DesktopState) -> CommandResult<FontsOverviewDto> {
    let root = project_root(state)?;
    let (settings, settings_error) = match FontSettings::load(&root) {
        Ok(settings) => (settings, None),
        Err(error) => (None, Some(error.to_string())),
    };
    let uncommitted = GitRepository::open(&root, state.git())
        .ok()
        .and_then(|repository| repository.status().ok())
        .is_some_and(|status| status.files.iter().any(|file| is_font_path(&file.path)));
    Ok(FontsOverviewDto {
        sources: settings
            .as_ref()
            .map(|settings| source_files(&root, settings))
            .unwrap_or_default(),
        settings,
        settings_error,
        uncommitted,
        game_fonts: GAME_FONTS
            .iter()
            .map(|font| GameFontDto {
                name: font.name.to_owned(),
                sizes: font
                    .sizes
                    .iter()
                    .map(|size| GameFontSizeDto {
                        size: size.size.to_owned(),
                        line_height: size.line_height,
                        ascent: size.ascent,
                        cap_height: size.cap_height,
                        cap_advance: f32::from(size.cap_advance_centi) / 100.0,
                        space_advance: size.space_advance,
                    })
                    .collect(),
            })
            .collect(),
    })
}

fn preview(
    root: &Path,
    settings: &FontSettings,
    font: &str,
    text: &str,
) -> CommandResult<Vec<PreviewSizeDto>> {
    settings.validate().map_err(|error| font_error(&error))?;
    let target = settings
        .fonts
        .iter()
        .find(|target| target.font == font)
        .ok_or_else(|| {
            CommandError::new("fontSettingsInvalid", format!("{font} is not configured"))
        })?;
    let game = game_font(font).expect("validated font");
    let characters = character_set(settings);
    Ok(game
        .sizes
        .iter()
        .map(|size| {
            let parameters = settings
                .parameters(target, size.size)
                .expect("validated source");
            let glyphs = load_source(root, parameters.source)
                .and_then(|source| render_size(game, size, &parameters, &source, &characters));
            let (line, generated_cap_advance, error) = match glyphs {
                Ok(glyphs) => {
                    let capitals: Vec<f32> = glyphs
                        .iter()
                        .filter(|glyph| ('А'..='Я').contains(&glyph.character))
                        .map(|glyph| f32::from(glyph.advance))
                        .collect();
                    #[allow(clippy::cast_precision_loss)]
                    let mean = (!capitals.is_empty())
                        .then(|| capitals.iter().sum::<f32>() / capitals.len() as f32);
                    (Some(preview_line(size, &glyphs, text)), mean, None)
                }
                Err(error) => (None, None, Some(error.to_string())),
            };
            PreviewSizeDto {
                size: size.size.to_owned(),
                line_height: size.line_height,
                ascent: size.ascent,
                cap_height: size.cap_height,
                cap_advance: f32::from(size.cap_advance_centi) / 100.0,
                generated_cap_advance,
                width: line.as_ref().map_or(0, |line| line.width),
                height: line.as_ref().map_or(0, |line| line.height),
                missing: line
                    .as_ref()
                    .map(|line| line.missing.iter().map(char::to_string).collect())
                    .unwrap_or_default(),
                pixels: line.map(|line| line.pixels).unwrap_or_default(),
                error,
            }
        })
        .collect())
}

#[tauri::command(rename_all = "camelCase")]
/// Reads the font settings, the supported game fonts, and the source files.
///
/// # Errors
///
/// Returns `noProjectOpen`.
pub async fn fonts_overview(app: tauri::AppHandle) -> CommandResult<FontsOverviewDto> {
    run_blocking(move || overview(&app.state::<DesktopState>())).await
}

#[tauri::command(rename_all = "camelCase")]
/// Writes the bundled recommended fonts into `fonts/` and saves the
/// recommended settings. Nothing is committed; the change is committed with
/// the next checkpoint.
///
/// # Errors
///
/// Returns `exportIo` when a file cannot be written.
pub async fn fonts_use_recommended(app: tauri::AppHandle) -> CommandResult<FontsOverviewDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let root = project_root(&state)?;
        let settings = install_recommended_files(&root).map_err(|error| font_error(&error))?;
        settings.save(&root).map_err(|error| font_error(&error))?;
        overview(&state)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Validates and writes `aeria-fonts.json`. Nothing is committed.
///
/// # Errors
///
/// Returns `fontSettingsInvalid` for invalid settings.
pub async fn fonts_save(
    app: tauri::AppHandle,
    settings: FontSettings,
) -> CommandResult<FontsOverviewDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let root = project_root(&state)?;
        settings.save(&root).map_err(|error| font_error(&error))?;
        overview(&state)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Copies a font or license file the user chose into `fonts/`.
///
/// # Errors
///
/// Returns `exportIo` when the file cannot be read or written.
pub async fn fonts_import_file(
    app: tauri::AppHandle,
    path: String,
) -> CommandResult<ImportedFileDto> {
    run_blocking(move || {
        let root = project_root(&app.state::<DesktopState>())?;
        let file =
            import_project_file(&root, Path::new(&path)).map_err(|error| font_error(&error))?;
        let bytes = std::fs::read(project_path(&root, &file))
            .map_err(|error| CommandError::new("exportIo", error.to_string()))?;
        let description = describe_font(&bytes);
        Ok(ImportedFileDto {
            file,
            is_font: description.is_some(),
            family: description.as_ref().and_then(|d| d.family.clone()),
            copyright: description.and_then(|d| d.copyright),
        })
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Renders `text` with the (possibly unsaved) settings for every size of one
/// game font.
///
/// # Errors
///
/// Returns `fontSettingsInvalid` for invalid settings or a game font that is
/// not configured. A size that fails to render reports its error instead.
pub async fn fonts_preview(
    app: tauri::AppHandle,
    settings: FontSettings,
    font: String,
    text: String,
) -> CommandResult<Vec<PreviewSizeDto>> {
    run_blocking(move || {
        let root = project_root(&app.state::<DesktopState>())?;
        preview(&root, &settings, &font, &text)
    })
    .await
}

#[cfg(test)]
mod tests {
    use aeria_fonts::recommended_settings;

    use super::*;

    #[test]
    fn git_manages_the_font_paths() {
        assert_eq!(
            [FONT_SETTINGS_FILE, FONTS_DIR],
            [aeria_git::FONT_SETTINGS_FILE, aeria_git::FONTS_DIR]
        );
        assert!(is_font_path("fonts/Oswald-Variable.ttf"));
        assert!(is_font_path("aeria-fonts.json"));
        assert!(!is_font_path("fontsx/a.ttf"));
    }

    #[test]
    fn preview_renders_every_size_of_a_font() {
        let temp = tempfile::tempdir().expect("temp");
        install_recommended_files(temp.path()).expect("install");
        let sizes =
            preview(temp.path(), &recommended_settings(), "Jupiter", "Ёж Q").expect("preview");
        assert_eq!(sizes.len(), 4);
        for size in &sizes {
            assert_eq!(size.error, None);
            assert_eq!(size.height, u32::from(size.line_height));
            assert_eq!(size.pixels.len(), (size.width * size.height) as usize);
            assert_eq!(size.missing, vec!["Q".to_owned()]);
        }
        let error =
            preview(temp.path(), &recommended_settings(), "AXIS", "Ё").expect_err("unknown");
        assert_eq!(error.code, "fontSettingsInvalid");
    }
}
