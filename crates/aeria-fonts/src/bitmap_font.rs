//! A TrueType font made from bitmap glyphs, for showing game symbols in the
//! interface.
//!
//! Each glyph becomes outlines of its covered pixels: runs of pixels in a
//! row, merged down while they repeat, as rectangles that fill with the
//! nonzero rule. At interface sizes the font is drawn far below its source
//! size, so the pixel steps do not show.

/// A glyph as coverage, one byte per pixel, row by row.
#[derive(Clone, Copy, Debug)]
pub struct BitmapGlyph<'a> {
    pub character: char,
    pub width: u32,
    pub height: u32,
    /// Pixels from the baseline up to the top row; negative below it.
    pub top: i32,
    /// How far the pen moves after the glyph, in pixels.
    pub advance: i32,
    pub alpha: &'a [u8],
}

/// Font units per source pixel.
const UNITS_PER_PIXEL: i32 = 32;
/// Coverage from which a pixel is part of the glyph.
const THRESHOLD: u8 = 128;

/// A rectangle in font units: left, bottom, right, top.
type Rect = (i32, i32, i32, i32);

fn rectangles(glyph: &BitmapGlyph<'_>) -> Vec<Rect> {
    let width = glyph.width as usize;
    let mut done = Vec::new();
    // Runs of the previous row with the row they started on.
    let mut open: Vec<(usize, usize, i32)> = Vec::new();
    let unit = UNITS_PER_PIXEL;
    let row_top = |row: i32| (glyph.top - row) * unit;
    for row in 0..=glyph.height as usize {
        let mut runs = Vec::new();
        if row < glyph.height as usize {
            let pixels = &glyph.alpha[row * width..(row + 1) * width];
            let mut start = None;
            for (column, alpha) in pixels.iter().chain(std::iter::once(&0)).enumerate() {
                match (start, *alpha >= THRESHOLD) {
                    (None, true) => start = Some(column),
                    (Some(first), false) => {
                        runs.push((first, column));
                        start = None;
                    }
                    _ => {}
                }
            }
        }
        let row = i32::try_from(row).unwrap_or(i32::MAX);
        let mut next = Vec::new();
        for (left, right, started) in open.drain(..) {
            if runs.contains(&(left, right)) {
                next.push((left, right, started));
            } else {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                done.push((
                    left as i32 * unit,
                    row_top(row),
                    right as i32 * unit,
                    row_top(started),
                ));
            }
        }
        for run in runs {
            if !next.iter().any(|(left, right, _)| (*left, *right) == run) {
                next.push((run.0, run.1, row));
            }
        }
        open = next;
    }
    done
}

struct Outline {
    rects: Vec<Rect>,
    advance: u16,
}

impl Outline {
    fn bounds(&self) -> Option<Rect> {
        let first = *self.rects.first()?;
        Some(self.rects.iter().fold(first, |bounds, rect| {
            (
                bounds.0.min(rect.0),
                bounds.1.min(rect.1),
                bounds.2.max(rect.2),
                bounds.3.max(rect.3),
            )
        }))
    }

    /// The glyph's `glyf` record: one clockwise contour of four on-curve
    /// points per rectangle, coordinates as 16-bit deltas.
    fn glyf(&self) -> Vec<u8> {
        let Some(bounds) = self.bounds() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        push_i16(&mut out, self.rects.len());
        for value in [bounds.0, bounds.1, bounds.2, bounds.3] {
            push_i16(&mut out, value);
        }
        for index in 0..self.rects.len() {
            push_u16(&mut out, index * 4 + 3);
        }
        push_u16(&mut out, 0_usize);
        let points: Vec<(i32, i32)> = self
            .rects
            .iter()
            .flat_map(|&(left, bottom, right, top)| {
                [(left, bottom), (left, top), (right, top), (right, bottom)]
            })
            .collect();
        out.extend(std::iter::repeat_n(0x01, points.len()));
        let mut previous = (0, 0);
        let mut ys = Vec::new();
        for &(x, y) in &points {
            push_i16(&mut out, x - previous.0);
            push_i16(&mut ys, y - previous.1);
            previous = (x, y);
        }
        out.extend(ys);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }
}

fn push_u16(out: &mut Vec<u8>, value: impl TryInto<u16>) {
    out.extend_from_slice(&value.try_into().unwrap_or(u16::MAX).to_be_bytes());
}

fn push_i16(out: &mut Vec<u8>, value: impl TryInto<i16>) {
    out.extend_from_slice(&value.try_into().unwrap_or(0).to_be_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0_u32, |sum, chunk| {
        let mut word = [0; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

fn name_table(family: &str) -> Vec<u8> {
    let postscript: String = family
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    let names = [
        (1, family.to_owned()),
        (2, "Regular".to_owned()),
        (3, format!("{postscript}-Regular")),
        (4, family.to_owned()),
        (6, format!("{postscript}-Regular")),
    ];
    let mut records = Vec::new();
    let mut strings = Vec::new();
    for (id, text) in &names {
        let encoded = utf16(text);
        for value in [3_usize, 1, 0x0409, *id, encoded.len(), strings.len()] {
            push_u16(&mut records, value);
        }
        strings.extend(encoded);
    }
    let mut out = Vec::new();
    push_u16(&mut out, 0_usize);
    push_u16(&mut out, names.len());
    push_u16(&mut out, 6 + records.len());
    out.extend(records);
    out.extend(strings);
    out
}

/// A format 4 `cmap` for characters mapped to glyphs 1, 2, … in order.
fn cmap_table(characters: &[u16]) -> Vec<u8> {
    let mut segments: Vec<(u16, u16, u16)> = Vec::new();
    for (index, &code) in characters.iter().enumerate() {
        let glyph = u16::try_from(index + 1).unwrap_or(u16::MAX);
        match segments.last_mut() {
            // Characters in a row map to glyphs in a row: one segment.
            Some((_, end, _)) if end.checked_add(1) == Some(code) => *end = code,
            _ => segments.push((code, code, glyph)),
        }
    }
    segments.push((0xFFFF, 0xFFFF, 0));
    let count = segments.len();
    let mut table = Vec::new();
    let length = 16 + count * 8;
    for value in [4, length, 0] {
        push_u16(&mut table, value);
    }
    let search = 2 * (1_usize << count.ilog2());
    for value in [
        count * 2,
        search,
        count.ilog2() as usize,
        count * 2 - search,
    ] {
        push_u16(&mut table, value);
    }
    for (_, end, _) in &segments {
        push_u16(&mut table, *end);
    }
    push_u16(&mut table, 0_usize);
    for (start, _, _) in &segments {
        push_u16(&mut table, *start);
    }
    for (start, _, glyph) in &segments {
        // The last segment maps 0xFFFF to glyph 0: a delta of 1.
        let delta = if *start == 0xFFFF {
            1
        } else {
            glyph.wrapping_sub(*start)
        };
        push_u16(&mut table, delta);
    }
    for _ in &segments {
        push_u16(&mut table, 0_usize);
    }
    let mut out = Vec::new();
    for value in [0_usize, 1, 3, 1] {
        push_u16(&mut out, value);
    }
    push_u32(&mut out, 12);
    out.extend(table);
    out
}

/// Writes a TrueType font of the glyphs. `ascent` and `descent` are the
/// line's extent above and below the baseline in pixels, and
/// `pixels_per_em` the size the glyphs were drawn at. Characters outside
/// the Basic Multilingual Plane are left out.
#[must_use]
pub fn bitmap_font(
    family: &str,
    pixels_per_em: u32,
    ascent: i32,
    descent: i32,
    glyphs: &[BitmapGlyph<'_>],
) -> Vec<u8> {
    let mut glyphs: Vec<&BitmapGlyph<'_>> = glyphs
        .iter()
        .filter(|glyph| u16::try_from(u32::from(glyph.character)).is_ok())
        .filter(|glyph| glyph.alpha.len() == (glyph.width * glyph.height) as usize)
        .collect();
    glyphs.sort_by_key(|glyph| glyph.character);
    glyphs.dedup_by_key(|glyph| glyph.character);
    let unit = UNITS_PER_PIXEL;
    let units_per_em = i32::try_from(pixels_per_em).unwrap_or(36).max(1) * unit;
    let outlines: Vec<Outline> = std::iter::once(Outline {
        rects: Vec::new(),
        advance: u16::try_from(units_per_em / 2).unwrap_or(0),
    })
    .chain(glyphs.iter().map(|glyph| Outline {
        rects: rectangles(glyph),
        advance: u16::try_from(glyph.advance.max(0) * unit).unwrap_or(u16::MAX),
    }))
    .collect();
    let characters: Vec<u16> = glyphs
        .iter()
        .filter_map(|glyph| u16::try_from(u32::from(glyph.character)).ok())
        .collect();

    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for outline in &outlines {
        push_u32(&mut loca, u32::try_from(glyf.len()).unwrap_or(u32::MAX));
        glyf.extend(outline.glyf());
    }
    push_u32(&mut loca, u32::try_from(glyf.len()).unwrap_or(u32::MAX));
    let bounds = outlines
        .iter()
        .filter_map(Outline::bounds)
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
        .unwrap_or((0, 0, 0, 0));
    let max_contours = outlines
        .iter()
        .map(|outline| outline.rects.len())
        .max()
        .unwrap_or(0);
    let advance_max = outlines
        .iter()
        .map(|outline| outline.advance)
        .max()
        .unwrap_or(0);
    let summary = Summary {
        units_per_em,
        bounds,
        max_contours,
        advance_max,
        ascender: ascent * unit,
        descender: -descent.abs() * unit,
        glyph_count: outlines.len(),
        first: characters.first().copied().unwrap_or(0),
        last: characters.last().copied().unwrap_or(0),
    };
    let mut hmtx = Vec::new();
    for outline in &outlines {
        push_u16(&mut hmtx, outline.advance);
        push_i16(&mut hmtx, outline.bounds().map_or(0, |bounds| bounds.0));
    }
    assemble(&[
        (b"OS/2", os2_table(&summary)),
        (b"cmap", cmap_table(&characters)),
        (b"glyf", glyf),
        (b"head", head_table(&summary)),
        (b"hhea", hhea_table(&summary)),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp_table(&summary)),
        (b"name", name_table(family)),
        (b"post", post_table(&summary)),
    ])
}

/// What the font-wide tables say about the glyphs.
struct Summary {
    units_per_em: i32,
    bounds: Rect,
    max_contours: usize,
    advance_max: u16,
    ascender: i32,
    descender: i32,
    glyph_count: usize,
    first: u16,
    last: u16,
}

fn head_table(summary: &Summary) -> Vec<u8> {
    let mut head = Vec::new();
    // Version, revision, checksum adjustment (set last), magic number.
    for value in [0x0001_0000_u32, 0x0001_0000, 0, 0x5F0F_3CF5] {
        push_u32(&mut head, value);
    }
    push_u16(&mut head, 0x0003_usize);
    push_u16(&mut head, summary.units_per_em);
    head.extend_from_slice(&[0; 16]);
    let bounds = summary.bounds;
    for value in [bounds.0, bounds.1, bounds.2, bounds.3] {
        push_i16(&mut head, value);
    }
    // Style, smallest size, direction hint, long offsets, glyph format.
    for value in [0_usize, 8, 2, 1, 0] {
        push_u16(&mut head, value);
    }
    head
}

fn hhea_table(summary: &Summary) -> Vec<u8> {
    let mut hhea = Vec::new();
    push_u32(&mut hhea, 0x0001_0000);
    push_i16(&mut hhea, summary.ascender);
    push_i16(&mut hhea, summary.descender);
    push_i16(&mut hhea, 0);
    push_u16(&mut hhea, summary.advance_max);
    push_i16(&mut hhea, summary.bounds.0);
    push_i16(&mut hhea, 0);
    push_i16(&mut hhea, summary.bounds.2);
    // Caret slope, caret offset, reserved words, metric format.
    for value in [1_usize, 0, 0, 0, 0, 0, 0, 0] {
        push_u16(&mut hhea, value);
    }
    push_u16(&mut hhea, summary.glyph_count);
    hhea
}

fn maxp_table(summary: &Summary) -> Vec<u8> {
    let mut maxp = Vec::new();
    push_u32(&mut maxp, 0x0001_0000);
    push_u16(&mut maxp, summary.glyph_count);
    push_u16(&mut maxp, summary.max_contours * 4);
    push_u16(&mut maxp, summary.max_contours);
    // No composites or instructions; two zones.
    for value in [0_usize, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        push_u16(&mut maxp, value);
    }
    maxp
}

fn os2_table(summary: &Summary) -> Vec<u8> {
    let em = summary.units_per_em;
    let mut os2 = Vec::new();
    push_u16(&mut os2, 4_usize);
    push_i16(&mut os2, i32::from(summary.advance_max) / 2);
    // Weight, width, embedding.
    for value in [400_usize, 5, 0] {
        push_u16(&mut os2, value);
    }
    // Subscript, superscript, strikeout, family class.
    for value in [
        em / 2,
        em / 2,
        0,
        em / 8,
        em / 2,
        em / 2,
        0,
        em / 3,
        em / 20,
        em / 4,
        0,
    ] {
        push_i16(&mut os2, value);
    }
    os2.extend_from_slice(&[0; 10]);
    // Unicode ranges: bit 60, the private use area.
    for value in [0, 1 << 28, 0, 0] {
        push_u32(&mut os2, value);
    }
    os2.extend_from_slice(b"AERI");
    push_u16(&mut os2, 0x0040_usize);
    push_u16(&mut os2, summary.first);
    push_u16(&mut os2, summary.last);
    push_i16(&mut os2, summary.ascender);
    push_i16(&mut os2, summary.descender);
    push_i16(&mut os2, 0);
    push_u16(&mut os2, summary.bounds.3.max(summary.ascender));
    push_u16(&mut os2, (-summary.bounds.1).max(-summary.descender));
    push_u32(&mut os2, 1);
    push_u32(&mut os2, 0);
    push_i16(&mut os2, em / 2);
    push_i16(&mut os2, em * 7 / 10);
    // Default character, break character, context.
    for value in [0_usize, 32, 0] {
        push_u16(&mut os2, value);
    }
    os2
}

fn post_table(summary: &Summary) -> Vec<u8> {
    let em = summary.units_per_em;
    let mut post = Vec::new();
    for value in [0x0003_0000_u32, 0] {
        push_u32(&mut post, value);
    }
    push_i16(&mut post, -(em / 10));
    push_i16(&mut post, em / 20);
    for _ in 0..5 {
        push_u32(&mut post, 0);
    }
    post
}

/// Writes the table directory and the tables, sorted by tag, and sets the
/// font checksum.
fn assemble(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let count = tables.len();
    let mut font = Vec::new();
    push_u32(&mut font, 0x0001_0000);
    push_u16(&mut font, count);
    let search = 16 * (1_usize << count.ilog2());
    for value in [search, count.ilog2() as usize, count * 16 - search] {
        push_u16(&mut font, value);
    }
    let mut offset = 12 + count * 16;
    let mut data = Vec::new();
    let mut head_offset = 0;
    for (tag, table) in tables {
        if *tag == b"head" {
            head_offset = offset;
        }
        font.extend_from_slice(*tag);
        push_u32(&mut font, checksum(table));
        push_u32(&mut font, u32::try_from(offset).unwrap_or(u32::MAX));
        push_u32(&mut font, u32::try_from(table.len()).unwrap_or(u32::MAX));
        data.extend_from_slice(table);
        while data.len() % 4 != 0 {
            data.push(0);
        }
        offset = 12 + count * 16 + data.len();
    }
    font.extend(data);
    let adjustment = 0xB1B0_AFBA_u32.wrapping_sub(checksum(&font));
    font[head_offset + 8..head_offset + 12].copy_from_slice(&adjustment.to_be_bytes());
    font
}

#[cfg(test)]
mod tests {
    use swash::FontRef;

    use super::*;

    #[test]
    fn glyphs_map_to_outlines_with_their_advance() {
        // A 3×2 glyph: a full top row and one pixel below it on the left.
        let alpha = [255, 255, 255, 255, 0, 0];
        let glyphs = [
            BitmapGlyph {
                character: '\u{E03C}',
                width: 3,
                height: 2,
                top: 10,
                advance: 4,
                alpha: &alpha,
            },
            BitmapGlyph {
                character: '\u{E03D}',
                width: 0,
                height: 0,
                top: 0,
                advance: 2,
                alpha: &[],
            },
        ];
        let bytes = bitmap_font("Aeria Test", 36, 38, 10, &glyphs);
        let font = FontRef::from_index(&bytes, 0).expect("a font");
        let charmap = font.charmap();
        assert_eq!(charmap.map('\u{E03C}'), 1);
        assert_eq!(charmap.map('\u{E03D}'), 2);
        assert_eq!(charmap.map('A'), 0);
        let metrics = font.glyph_metrics(&[]);
        assert_eq!(metrics.units_per_em(), 36 * 32);
        assert!((metrics.advance_width(1) - 128.0).abs() < f32::EPSILON);
        assert_eq!(
            rectangles(&glyphs[0]),
            [(0, 256, 32, 288), (0, 288, 96, 320)]
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
        );
    }
}
