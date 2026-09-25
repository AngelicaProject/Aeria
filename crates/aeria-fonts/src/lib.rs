//! Cyrillic glyphs for the game fonts that have none.
//!
//! Aeria renders glyphs from project-chosen source fonts at export time and
//! stores them in the optional `FONTS` pack section; Harmonia adds them to the
//! game's own `.fdt` files and atlas pages at startup. The contracts are
//! `docs/formats/font-settings-v1.md` and the `FONTS` part of
//! `docs/formats/pack-v1.md`.

mod error;
mod import;
mod preset;
mod preview;
mod render;
mod section;
mod settings;
mod targets;

pub use error::FontError;
pub use import::{FontDescription, describe_font, import_project_file};
pub use preset::{DEFAULT_CHARACTERS, install_recommended_files, recommended_settings};
pub use preview::{PreviewLine, preview_line};
pub use render::{LoadedSource, character_set, generate, load_source, render_size};
pub use section::{FONTS_SECTION_KIND, FontSection, SectionGlyph, SectionSource, SectionTarget};
pub use settings::{
    CaseMapping, FONT_SETTINGS_FILE, FONTS_DIR, FontSettings, FontSource, FontTarget, SizeOverride,
    SizeParameters, project_path,
};
pub use targets::{GAME_FONTS, GameFont, GameFontSize, game_font};
