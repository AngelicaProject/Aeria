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
}

/// A game font that has no Cyrillic glyphs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GameFont {
    /// The name part of the `.fdt` name, e.g. `"Jupiter"`.
    pub name: &'static str,
    pub sizes: &'static [GameFontSize],
}

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
    }
}

/// Every game font and size Aeria generates glyphs for. `Jupiter_45` and
/// `Jupiter_90` hold digits only and `Meidinger` holds digits and signs, so
/// they are not listed.
pub const GAME_FONTS: &[GameFont] = &[
    GameFont {
        name: "Jupiter",
        sizes: &[
            size("16", 26, 19, 13, 981, 5),
            size("20", 32, 25, 16, 1227, 6),
            size("23", 35, 26, 18, 1423, 7),
            size("46", 70, 52, 37, 2808, 14),
        ],
    },
    GameFont {
        name: "MiedingerMid",
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
