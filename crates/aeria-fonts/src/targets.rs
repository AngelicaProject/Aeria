//! The game fonts Aeria can extend and the native metrics glyphs are fitted
//! to.
//!
//! The values were measured from the game's own `.fdt` files and atlas
//! textures (`lineHeight` and `ascent` from the `fthd` header, `capHeight`
//! and `capAdvance` from the Latin capitals, `spaceAdvance` from the space).
//! `lineHeight` and `ascent` are also written into the pack so Harmonia
//! applies a size only when the running game still has the same metrics.

/// Native metrics of one size of a game font.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GameFontSize {
    /// The size part of the `.fdt` name, e.g. `"184"` for `TrumpGothic_184`.
    pub size: &'static str,
    /// `fthd` line height in pixels.
    pub line_height: u8,
    /// `fthd` ascent: the baseline row counted from the top of the line.
    pub ascent: u8,
    /// Ink height of the Latin capitals in pixels.
    pub cap_height: u8,
    /// Mean advance of `A`–`Z` in hundredths of a pixel.
    pub cap_advance_centi: u16,
    /// Advance of the space character in pixels.
    pub space_advance: u8,
    /// How the native Latin glyphs sit on the pixel grid, for game fonts
    /// whose own glyphs are replaced; `None` draws outlines unfitted.
    pub grid: Option<PixelGrid>,
}

/// The pixel grid of the native Latin glyphs of one size, measured like
/// the other metrics. Replacement glyphs are fitted to it so that their stems
/// look like the Latin ones in the same line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelGrid {
    /// Ink height of the Latin small `x` in pixels.
    pub x_height: u8,
    /// Width of a vertical stem (`n`, `l`, `H`) in hundredths of a pixel.
    pub stem_centi: u16,
    /// Where the left edge of a vertical stem falls inside its pixel, in
    /// hundredths of a pixel from the pixel's left edge.
    pub stem_phase_centi: u16,
    /// Thickness of a horizontal bar (`H`, `e`) in hundredths of a pixel.
    pub bar_centi: u16,
    /// Half the difference of the mean left and right side bearings of the
    /// small Latin letters, in hundredths of a pixel: the native glyphs keep
    /// most of the space between letters on their left, and replacements are
    /// shifted to do the same, so they sit evenly next to native punctuation.
    pub bearing_split_centi: i16,
}

/// A game font Aeria renders glyphs for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GameFont {
    /// The name part of the `.fdt` name, e.g. `"Jupiter"`.
    pub name: &'static str,
    /// The font has its own Cyrillic, and the rendered glyphs replace it
    /// instead of filling gaps: only the configured characters of
    /// [`REPLACED_CHARACTERS`] are rendered, into the pack's
    /// font-replacements section.
    pub replaces: bool,
    pub sizes: &'static [GameFontSize],
}

/// The characters a replacing game font gets from the source font: the
/// Cyrillic block. Other configured characters (`№`, quotes) keep the
/// game's glyphs.
pub const REPLACED_CHARACTERS: std::ops::RangeInclusive<char> = '\u{0400}'..='\u{04FF}';

const fn size(
    size: &'static str,
    line_height: u8,
    ascent: u8,
    cap_height: u8,
    cap_advance_centi: u16,
    space_advance: u8,
) -> GameFontSize {
    GameFontSize {
        size,
        line_height,
        ascent,
        cap_height,
        cap_advance_centi,
        space_advance,
        grid: None,
    }
}

const fn fitted(
    base: GameFontSize,
    x_height: u8,
    stem_centi: u16,
    stem_phase_centi: u16,
    bar_centi: u16,
    bearing_split_centi: i16,
) -> GameFontSize {
    GameFontSize {
        grid: Some(PixelGrid {
            x_height,
            stem_centi,
            stem_phase_centi,
            bar_centi,
            bearing_split_centi,
        }),
        ..base
    }
}

/// Every game font and size Aeria generates glyphs for. `Jupiter_45` and
/// `Jupiter_90` hold digits only and `Meidinger` holds digits and signs, so
/// they are not listed. `AXIS`, the font of dialogue, menus, and chat, has
/// Cyrillic whose stems sit unevenly on the pixel grid; its replacements are
/// fitted to the grid of its Latin (`AXIS_96` is the 9.6 px size).
pub const GAME_FONTS: &[GameFont] = &[
    GameFont {
        name: "AXIS",
        replaces: true,
        sizes: &[
            fitted(size("96", 15, 11, 8, 688, 3), 6, 106, 87, 100, 82),
            fitted(size("12", 17, 13, 9, 842, 3), 7, 140, 7, 120, 98),
            fitted(size("14", 19, 15, 11, 969, 4), 8, 140, 40, 140, 111),
            fitted(size("18", 24, 19, 14, 1273, 5), 11, 193, 80, 180, 111),
            fitted(size("36", 48, 38, 28, 2527, 10), 21, 333, 67, 300, 284),
        ],
    },
    GameFont {
        name: "Jupiter",
        replaces: false,
        sizes: &[
            size("16", 26, 19, 13, 981, 5),
            size("20", 32, 25, 16, 1227, 6),
            size("23", 35, 26, 18, 1423, 7),
            size("46", 70, 52, 37, 2808, 14),
        ],
    },
    GameFont {
        name: "MiedingerMid",
        replaces: false,
        sizes: &[
            size("10", 14, 11, 7, 1058, 5),
            size("12", 16, 13, 9, 1258, 6),
            size("14", 18, 14, 10, 1462, 6),
            size("18", 22, 17, 13, 1900, 8),
            size("36", 44, 34, 26, 3777, 16),
        ],
    },
    GameFont {
        name: "TrumpGothic",
        replaces: false,
        sizes: &[
            size("184", 24, 19, 14, 600, 3),
            size("23", 30, 24, 18, 758, 4),
            size("34", 42, 34, 26, 1123, 5),
            size("68", 84, 68, 52, 2231, 10),
        ],
    },
];

/// Looks up a game font by its `.fdt` name part.
#[must_use]
pub fn game_font(name: &str) -> Option<&'static GameFont> {
    GAME_FONTS.iter().find(|font| font.name == name)
}

impl GameFont {
    /// Looks up one size by its `.fdt` size part.
    #[must_use]
    pub fn size(&self, size: &str) -> Option<&'static GameFontSize> {
        self.sizes.iter().find(|candidate| candidate.size == size)
    }
}
