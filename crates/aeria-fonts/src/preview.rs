//! A line of generated glyphs laid out the way the game draws them, for the
//! font settings preview.

use crate::section::SectionGlyph;
use crate::targets::GameFontSize;

/// One line of text, `line_height` pixels tall, as 8-bit coverage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewLine {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// Characters of the text that have no generated glyph and were left out.
    pub missing: Vec<char>,
}

/// Lays out `text` with `glyphs`: each glyph at the pen position and its
/// `offset_y`, the pen advancing by the glyph advance. A space advances by the
/// native space width.
#[must_use]
pub fn preview_line(size: &GameFontSize, glyphs: &[SectionGlyph], text: &str) -> PreviewLine {
    let mut placed = Vec::new();
    let mut missing = Vec::new();
    let mut pen = 0usize;
    let mut width = 0usize;
    for character in text.chars() {
        if character == ' ' {
            pen += usize::from(size.space_advance);
            continue;
        }
        match glyphs.binary_search_by_key(&character, |glyph| glyph.character) {
            Ok(index) => {
                let glyph = &glyphs[index];
                placed.push((pen, glyph));
                width = width.max(pen + usize::from(glyph.width));
                pen += usize::from(glyph.advance);
            }
            Err(_) => {
                if !missing.contains(&character) {
                    missing.push(character);
                }
            }
        }
    }
    let width = width.max(pen);
    let height = usize::from(size.line_height);
    let mut pixels = vec![0u8; width * height];
    for (x, glyph) in placed {
        for row in 0..usize::from(glyph.height) {
            let y = usize::from(glyph.offset_y) + row;
            for column in 0..usize::from(glyph.width) {
                let value = glyph.bitmap[row * usize::from(glyph.width) + column];
                let pixel = &mut pixels[y * width + x + column];
                *pixel = (*pixel).max(value);
            }
        }
    }
    PreviewLine {
        width: u32::try_from(width).unwrap_or(u32::MAX),
        height: u32::try_from(height).unwrap_or(u32::MAX),
        pixels,
        missing,
    }
}
