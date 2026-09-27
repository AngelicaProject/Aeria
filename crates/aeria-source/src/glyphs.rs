//! Game glyphs for the interface: the symbols of the game font's private
//! use area, such as `U+E03C` (the high-quality mark), and the inline icons
//! of `<icon>` macros.
//!
//! Game text writes some symbols as private use characters that only the
//! game font draws, and shows others as icons from a texture. Aeria reads
//! both so the editor can show them as the game does.

/// The game font the private use glyphs are read from: its largest size,
/// so the glyphs stay sharp when scaled down.
pub(crate) const GLYPH_FONT: &str = "common/font/axis_36.fdt";
/// The table of inline icons.
pub(crate) const ICON_TABLE: &str = "common/font/gfdata.gfd";
/// The inline icons for keyboard and mouse or an Xbox controller.
pub(crate) const ICON_TEXTURE: &str = "common/font/fonticon_xinput.tex";
/// Where the double-size icons start in the icon texture: at twice an
/// icon's position, this many rows down.
const HIGH_RESOLUTION_TOP: u32 = 341;

/// Texture formats Aeria decodes.
const FORMAT_B4G4R4A4: u32 = 0x1440;
const FORMAT_B8G8R8A8: u32 = 0x1450;
/// Size of a `.tex` header; the first surface follows at the offset it
/// names.
const TEX_HEADER: usize = 80;

/// A glyph of the game font as coverage: `alpha` holds one byte, 0–255, per
/// pixel, row by row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontGlyph {
    pub character: char,
    pub width: u32,
    pub height: u32,
    /// Rows from the top of the line to the top of the glyph.
    pub offset_y: i32,
    /// How far the pen moves after the glyph.
    pub advance: i32,
    pub alpha: Vec<u8>,
}

/// The private use glyphs of the game font with the font's metrics.
#[derive(Clone, Debug, PartialEq)]
pub struct FontGlyphs {
    /// The font size in pixels.
    pub size: f32,
    pub line_height: i32,
    /// The baseline, counted from the top of the line.
    pub ascent: i32,
    pub glyphs: Vec<FontGlyph>,
}

/// An inline icon as RGBA pixels, row by row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// A decoded texture surface.
pub(crate) struct Texture<'a> {
    format: u32,
    width: u32,
    height: u32,
    pixels: &'a [u8],
}

impl<'a> Texture<'a> {
    /// Reads the first surface of a `.tex` file in a format Aeria decodes.
    pub(crate) fn parse(bytes: &'a [u8]) -> Option<Self> {
        let format = u32_at(bytes, 4)?;
        let width = u32::from(u16_at(bytes, 8)?);
        let height = u32::from(u16_at(bytes, 10)?);
        let surface = usize::try_from(u32_at(bytes, 28)?).ok()?;
        let pixel_size = match format {
            FORMAT_B4G4R4A4 => 2,
            FORMAT_B8G8R8A8 => 4,
            _ => return None,
        };
        let length = usize::try_from(width * height).ok()? * pixel_size;
        let pixels = bytes.get(surface.max(TEX_HEADER)..surface.max(TEX_HEADER) + length)?;
        Some(Self {
            format,
            width,
            height,
            pixels,
        })
    }

    fn contains(&self, x: u32, y: u32, width: u32, height: u32) -> bool {
        x + width <= self.width && y + height <= self.height
    }

    /// One 4-bit channel of a `B4G4R4A4` font page as coverage. Channels
    /// 0–3 are the bits at 8, 4, 0, and 12, the order the game fonts use.
    fn channel(&self, channel: u16, x: u32, y: u32, width: u32, height: u32) -> Option<Vec<u8>> {
        if self.format != FORMAT_B4G4R4A4 || !self.contains(x, y, width, height) {
            return None;
        }
        let shift = [8, 4, 0, 12][usize::from(channel % 4)];
        let mut alpha = Vec::with_capacity((width * height) as usize);
        for row in y..y + height {
            for column in x..x + width {
                let at = ((row * self.width + column) * 2) as usize;
                let value = u16::from_le_bytes([self.pixels[at], self.pixels[at + 1]]);
                #[allow(clippy::cast_possible_truncation)]
                alpha.push(((value >> shift) & 0xF) as u8 * 17);
            }
        }
        Some(alpha)
    }

    /// A region of a `B8G8R8A8` texture as RGBA.
    fn rgba(&self, x: u32, y: u32, width: u32, height: u32) -> Option<Vec<u8>> {
        if self.format != FORMAT_B8G8R8A8 || !self.contains(x, y, width, height) {
            return None;
        }
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for row in y..y + height {
            for column in x..x + width {
                let at = ((row * self.width + column) * 4) as usize;
                let [blue, green, red, alpha] = self.pixels[at..at + 4] else {
                    return None;
                };
                rgba.extend_from_slice(&[red, green, blue, alpha]);
            }
        }
        Some(rgba)
    }
}

/// A glyph record of a game `.fdt` font.
pub(crate) struct FdtGlyph {
    pub(crate) character: char,
    /// The font page: `texture / 4` selects the file, `texture % 4` the
    /// channel.
    pub(crate) texture: u16,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    next_offset_x: i32,
    offset_y: i32,
}

/// The metrics and glyph records of a game `.fdt` font: `fcsv0100`, then at
/// 0x20 the `fthd` header (glyph count, texture size, font size, line
/// height, ascent) and 16-byte glyph records, each the character's UTF-8
/// bytes packed big-endian, its Shift-JIS code, its page, position, size,
/// and offsets.
pub(crate) fn parse_fdt(bytes: &[u8]) -> Option<(f32, i32, i32, Vec<FdtGlyph>)> {
    if bytes.get(..8)? != b"fcsv0100" || bytes.get(0x20..0x24)? != b"fthd" {
        return None;
    }
    let count = usize::try_from(u32_at(bytes, 0x24)?).ok()?;
    let size = f32::from_le_bytes(bytes.get(0x34..0x38)?.try_into().ok()?);
    let line_height = i32::from_le_bytes(bytes.get(0x38..0x3C)?.try_into().ok()?);
    let ascent = i32::from_le_bytes(bytes.get(0x3C..0x40)?.try_into().ok()?);
    let mut glyphs = Vec::new();
    for index in 0..count {
        let at = 0x40 + index * 16;
        let record = bytes.get(at..at + 16)?;
        // The UTF-8 bytes, packed most significant first, without leading zeros.
        let packed = u32::from_le_bytes(record[0..4].try_into().ok()?).to_be_bytes();
        let mut utf8: Vec<u8> = packed
            .iter()
            .copied()
            .skip_while(|byte| *byte == 0)
            .collect();
        if utf8.is_empty() {
            utf8.push(0);
        }
        let Some(character) = std::str::from_utf8(&utf8)
            .ok()
            .and_then(|text| text.chars().next())
        else {
            continue;
        };
        glyphs.push(FdtGlyph {
            character,
            texture: u16::from_le_bytes([record[6], record[7]]),
            x: u32::from(u16::from_le_bytes([record[8], record[9]])),
            y: u32::from(u16::from_le_bytes([record[10], record[11]])),
            width: u32::from(record[12]),
            height: u32::from(record[13]),
            next_offset_x: i32::from(record[14].cast_signed()),
            offset_y: i32::from(record[15].cast_signed()),
        });
    }
    Some((size, line_height, ascent, glyphs))
}

/// Whether a character is in the private use area the game font draws.
pub(crate) fn is_private_use(character: char) -> bool {
    ('\u{E000}'..='\u{F8FF}').contains(&character)
}

/// Cuts the private use glyphs out of their font pages. `page` returns the
/// `.tex` file of a page number, `font1.tex` for page 0.
pub(crate) fn private_glyphs(
    fdt: &[u8],
    mut page: impl FnMut(u16) -> Option<Vec<u8>>,
) -> Option<FontGlyphs> {
    let (size, line_height, ascent, records) = parse_fdt(fdt)?;
    let mut pages: std::collections::BTreeMap<u16, Option<Vec<u8>>> =
        std::collections::BTreeMap::new();
    let mut glyphs = Vec::new();
    for record in records
        .iter()
        .filter(|record| is_private_use(record.character))
    {
        let file = pages
            .entry(record.texture / 4)
            .or_insert_with(|| page(record.texture / 4));
        let Some(texture) = file.as_deref().and_then(Texture::parse) else {
            continue;
        };
        let Some(alpha) = texture.channel(
            record.texture,
            record.x,
            record.y,
            record.width,
            record.height,
        ) else {
            continue;
        };
        glyphs.push(FontGlyph {
            character: record.character,
            width: record.width,
            height: record.height,
            offset_y: record.offset_y,
            advance: i32::try_from(record.width).unwrap_or(0) + record.next_offset_x,
            alpha,
        });
    }
    Some(FontGlyphs {
        size,
        line_height,
        ascent,
        glyphs,
    })
}

/// An icon of the `gfdata.gfd` table: `gftd0100`, a count, then 16-byte
/// entries of id, left, top, width, height, an unknown word, the id this
/// entry stands for when it has no size, and an unknown word. Positions
/// are in the normal-size half of the icon texture.
pub(crate) fn icon_region(gfd: &[u8], id: u32) -> Option<(u32, u32, u32, u32)> {
    if gfd.get(..8)? != b"gftd0100" {
        return None;
    }
    let count = usize::try_from(u32_at(gfd, 8)?).ok()?;
    let entry = |id: u32| {
        (0..count).find_map(|index| {
            let at = 16 + index * 16;
            (u32::from(u16_at(gfd, at)?) == id).then(|| {
                let word = |offset: usize| u32::from(u16_at(gfd, at + offset).unwrap_or(0));
                (word(2), word(4), word(6), word(8), word(12))
            })
        })
    };
    let mut current = id;
    // A few steps of redirection at most; a loop is not an icon.
    for _ in 0..4 {
        let (left, top, width, height, redirect) = entry(current)?;
        if width > 0 && height > 0 {
            return Some((left, top, width, height));
        }
        if redirect == 0 || redirect == current {
            return None;
        }
        current = redirect;
    }
    None
}

/// Cuts an icon out of the icon texture at double size.
pub(crate) fn icon(gfd: &[u8], texture: &[u8], id: u32) -> Option<Icon> {
    let (left, top, width, height) = icon_region(gfd, id)?;
    let texture = Texture::parse(texture)?;
    let (width, height) = (width * 2, height * 2);
    let rgba = texture.rgba(left * 2, top * 2 + HIGH_RESOLUTION_TOP, width, height)?;
    Some(Icon {
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tex(format: u32, width: u16, height: u16, pixels: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; TEX_HEADER];
        bytes[4..8].copy_from_slice(&format.to_le_bytes());
        bytes[8..10].copy_from_slice(&width.to_le_bytes());
        bytes[10..12].copy_from_slice(&height.to_le_bytes());
        bytes[28..32].copy_from_slice(&80_u32.to_le_bytes());
        bytes.extend_from_slice(pixels);
        bytes
    }

    #[test]
    fn font_glyphs_come_from_their_page_and_channel() {
        let mut fdt = vec![0; 0x40];
        fdt[..8].copy_from_slice(b"fcsv0100");
        fdt[0x20..0x24].copy_from_slice(b"fthd");
        fdt[0x24..0x28].copy_from_slice(&2_u32.to_le_bytes());
        fdt[0x34..0x38].copy_from_slice(&36.0_f32.to_le_bytes());
        fdt[0x38..0x3C].copy_from_slice(&48_i32.to_le_bytes());
        fdt[0x3C..0x40].copy_from_slice(&38_i32.to_le_bytes());
        // 'A', then U+E03C on page 0, channel 1 (bits 4–7), at (1, 0), 1×1.
        for (utf8, texture, x) in [(0x41_u32, 0_u16, 0_u16), (0x00EE_80BC, 1, 1)] {
            let mut record = [0_u8; 16];
            record[0..4].copy_from_slice(&utf8.to_le_bytes());
            record[6..8].copy_from_slice(&texture.to_le_bytes());
            record[8..10].copy_from_slice(&x.to_le_bytes());
            record[12] = 1;
            record[13] = 1;
            record[14] = 2;
            record[15] = 3;
            fdt.extend_from_slice(&record);
        }
        let page = tex(FORMAT_B4G4R4A4, 2, 1, &[0, 0, 0xF0, 0x00]);
        let glyphs =
            private_glyphs(&fdt, |index| (index == 0).then(|| page.clone())).expect("glyphs");
        assert_eq!((glyphs.line_height, glyphs.ascent), (48, 38));
        assert_eq!(
            glyphs.glyphs,
            [FontGlyph {
                character: '\u{E03C}',
                width: 1,
                height: 1,
                offset_y: 3,
                advance: 3,
                alpha: vec![255],
            }]
        );
    }

    #[test]
    fn icons_follow_redirects_and_use_the_double_size_half() {
        let mut gfd = b"gftd0100".to_vec();
        gfd.extend_from_slice(&2_u32.to_le_bytes());
        gfd.extend_from_slice(&[0; 4]);
        for words in [[1_u16, 0, 0, 1, 1, 0, 0, 0], [26, 0, 0, 0, 0, 0, 1, 0]] {
            for word in words {
                gfd.extend_from_slice(&word.to_le_bytes());
            }
        }
        assert_eq!(icon_region(&gfd, 26), Some((0, 0, 1, 1)));
        assert_eq!(icon_region(&gfd, 5), None);
        let width = 2_u16;
        let height = u16::try_from(HIGH_RESOLUTION_TOP).expect("small") + 2;
        let mut pixels = vec![0_u8; usize::from(width) * usize::from(height) * 4];
        let top = HIGH_RESOLUTION_TOP as usize * usize::from(width) * 4;
        // B, G, R, A of the first double-size pixel.
        pixels[top..top + 4].copy_from_slice(&[3, 2, 1, 255]);
        let texture = tex(FORMAT_B8G8R8A8, width, height, &pixels);
        let icon = icon(&gfd, &texture, 1).expect("icon");
        assert_eq!((icon.width, icon.height), (2, 2));
        assert_eq!(&icon.rgba[..4], &[1, 2, 3, 255]);
    }
}
