//! The optional pack section `FONTS` (kind `0x10000`, Pack Format v1.1).
//!
//! The contract is the `FONTS` part of `docs/formats/pack-v1.md`.

use serde::{Deserialize, Serialize};

use crate::error::FontError;

/// Section kind in the pack section table.
pub const FONTS_SECTION_KIND: u32 = 0x1_0000;
const MAGIC: &[u8; 8] = b"HPKFONT1";
const HEADER_SIZE: usize = 32;
const GLYPH_RECORD_SIZE: usize = 16;

/// Authorship and license of one source font, copied into the pack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SectionSource {
    pub family: String,
    pub copyright: String,
    /// SPDX license identifier.
    pub license: String,
    /// Full license text with LF line endings.
    pub license_text: String,
    /// SHA-256 of the font file the glyphs were rendered from.
    pub sha256: [u8; 32],
}

/// One glyph, ready for an `.fdt` record: the bitmap starts at the pen
/// position and at row `offset_y` of the line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SectionGlyph {
    pub character: char,
    pub width: u8,
    pub height: u8,
    pub offset_y: u8,
    pub advance: u8,
    /// `width * height` coverage bytes, row-major, 0 transparent.
    pub bitmap: Vec<u8>,
}

/// The glyphs added to one game font size.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SectionTarget {
    pub font: String,
    pub size: String,
    /// Native `fthd` line height and ascent the glyphs were fitted to.
    pub line_height: u8,
    pub ascent: u8,
    /// Index into [`FontSection::sources`].
    pub source: usize,
    pub glyphs: Vec<SectionGlyph>,
}

/// The whole `FONTS` section before encoding.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FontSection {
    pub sources: Vec<SectionSource>,
    pub targets: Vec<SectionTarget>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MetadataJson {
    sources: Vec<SourceJson>,
    targets: Vec<TargetJson>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceJson {
    family: String,
    copyright: String,
    license: String,
    license_text: String,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TargetJson {
    font: String,
    size: String,
    line_height: u8,
    ascent: u8,
    source: usize,
    glyph_start: u32,
    glyph_count: u32,
}

impl FontSection {
    /// Total number of glyphs.
    #[must_use]
    pub fn glyph_count(&self) -> usize {
        self.targets.iter().map(|target| target.glyphs.len()).sum()
    }

    /// Checks every rule of the section contract.
    ///
    /// # Errors
    /// Returns [`FontError::Section`] naming the first violation.
    pub fn validate(&self) -> Result<(), FontError> {
        let fail = |reason: String| Err(FontError::Section(reason));
        if self.targets.is_empty() {
            return fail("a FONTS section needs at least one target".to_owned());
        }
        let mut used = vec![false; self.sources.len()];
        for source in &self.sources {
            if [&source.family, &source.copyright, &source.license]
                .iter()
                .any(|value| value.trim().is_empty())
                || source.license_text.trim().is_empty()
                || source.license_text.contains('\r')
            {
                return fail(format!(
                    "source {} needs family, copyright, license and an LF license text",
                    source.family
                ));
            }
        }
        for pair in self.targets.windows(2) {
            if target_key(&pair[0]) >= target_key(&pair[1]) {
                return fail("targets must be sorted by font and size and unique".to_owned());
            }
        }
        for target in &self.targets {
            let name = format!("{}_{}", target.font, target.size);
            if target.font.is_empty()
                || !target.font.bytes().all(|b| b.is_ascii_alphanumeric())
                || target.size.is_empty()
                || !target.size.bytes().all(|b| b.is_ascii_digit())
            {
                return fail(format!("{name}: font must be [A-Za-z0-9]+ and size [0-9]+"));
            }
            if target.ascent == 0 || target.ascent > target.line_height {
                return fail(format!("{name}: ascent must be within 1..=lineHeight"));
            }
            let Some(slot) = used.get_mut(target.source) else {
                return fail(format!("{name}: unknown source {}", target.source));
            };
            *slot = true;
            if target.glyphs.is_empty() {
                return fail(format!("{name}: a target needs at least one glyph"));
            }
            for pair in target.glyphs.windows(2) {
                if pair[0].character >= pair[1].character {
                    return fail(format!("{name}: glyphs must be sorted and unique"));
                }
            }
            for glyph in &target.glyphs {
                let at = format!("{name} U+{:04X}", u32::from(glyph.character));
                if u32::from(glyph.character) < 0x80 {
                    return fail(format!("{at}: ASCII characters cannot be added"));
                }
                if glyph.bitmap.len() != usize::from(glyph.width) * usize::from(glyph.height) {
                    return fail(format!("{at}: bitmap size does not match width × height"));
                }
                if u16::from(glyph.offset_y) + u16::from(glyph.height)
                    > u16::from(target.line_height)
                {
                    return fail(format!("{at}: glyph extends below the line"));
                }
                if i8::try_from(i16::from(glyph.advance) - i16::from(glyph.width)).is_err() {
                    return fail(format!("{at}: advance − width must fit in i8"));
                }
            }
        }
        if used.iter().any(|used| !used) {
            return fail("every source must be used by a target".to_owned());
        }
        Ok(())
    }

    /// Validates and encodes the section body.
    ///
    /// # Errors
    /// Returns [`FontError::Section`] for invalid content or a size limit.
    ///
    /// # Panics
    /// Never: serializing plain structs cannot fail.
    pub fn encode(&self) -> Result<Vec<u8>, FontError> {
        self.validate()?;
        let limit = |what: &str| FontError::Section(format!("{what} exceeds u32"));
        let mut glyphs = Vec::new();
        let mut bitmaps = Vec::new();
        let mut targets = Vec::with_capacity(self.targets.len());
        let mut glyph_start = 0u32;
        for target in &self.targets {
            let glyph_count = u32::try_from(target.glyphs.len()).map_err(|_| limit("glyphs"))?;
            for glyph in &target.glyphs {
                glyphs.extend_from_slice(&u32::from(glyph.character).to_le_bytes());
                glyphs.extend_from_slice(&[
                    glyph.width,
                    glyph.height,
                    glyph.offset_y,
                    glyph.advance,
                ]);
                let offset = u32::try_from(bitmaps.len()).map_err(|_| limit("bitmaps"))?;
                glyphs.extend_from_slice(&offset.to_le_bytes());
                glyphs.extend_from_slice(&0u32.to_le_bytes());
                bitmaps.extend_from_slice(&glyph.bitmap);
            }
            targets.push(TargetJson {
                font: target.font.clone(),
                size: target.size.clone(),
                line_height: target.line_height,
                ascent: target.ascent,
                source: target.source,
                glyph_start,
                glyph_count,
            });
            glyph_start = glyph_start
                .checked_add(glyph_count)
                .ok_or_else(|| limit("glyphs"))?;
        }
        let metadata = MetadataJson {
            sources: self
                .sources
                .iter()
                .map(|source| SourceJson {
                    family: source.family.clone(),
                    copyright: source.copyright.clone(),
                    license: source.license.clone(),
                    license_text: source.license_text.clone(),
                    sha256: hex(&source.sha256),
                })
                .collect(),
            targets,
        };
        let mut metadata = serde_json::to_vec_pretty(&metadata).expect("metadata serializes");
        metadata.push(b'\n');

        let mut bytes =
            Vec::with_capacity(HEADER_SIZE + metadata.len() + glyphs.len() + bitmaps.len() + 8);
        bytes.extend_from_slice(MAGIC);
        for value in [metadata.len(), self.glyph_count(), bitmaps.len()] {
            let value = u32::try_from(value).map_err(|_| limit("FONTS section"))?;
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.resize(HEADER_SIZE, 0);
        bytes.extend_from_slice(&metadata);
        while !bytes.len().is_multiple_of(8) {
            bytes.push(0);
        }
        bytes.extend_from_slice(&glyphs);
        bytes.extend_from_slice(&bitmaps);
        Ok(bytes)
    }

    /// Decodes and validates a section body produced by [`FontSection::encode`].
    ///
    /// # Errors
    /// Returns [`FontError::Section`] for any deviation from the contract.
    ///
    /// # Panics
    /// Never: every slice is bounds-checked against the header first.
    pub fn decode(bytes: &[u8]) -> Result<Self, FontError> {
        let fail = |reason: &str| FontError::Section(reason.to_owned());
        if bytes.len() < HEADER_SIZE
            || &bytes[..8] != MAGIC
            || bytes[20..32].iter().any(|b| *b != 0)
        {
            return Err(fail("bad header"));
        }
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        let usize_at = |at: usize| u32_at(at) as usize;
        let (metadata_length, glyph_count, bitmap_length) =
            (usize_at(8), usize_at(12), usize_at(16));
        let metadata_end = HEADER_SIZE + metadata_length;
        let glyphs_start = metadata_end.next_multiple_of(8);
        let bitmaps_start = glyphs_start + glyph_count * GLYPH_RECORD_SIZE;
        if bytes.len() != bitmaps_start + bitmap_length
            || bytes[metadata_end.min(bytes.len())..glyphs_start.min(bytes.len())]
                .iter()
                .any(|b| *b != 0)
        {
            return Err(fail("section lengths do not match the header"));
        }
        let metadata: MetadataJson = serde_json::from_slice(&bytes[HEADER_SIZE..metadata_end])
            .map_err(|error| FontError::Section(error.to_string()))?;
        let bitmaps = &bytes[bitmaps_start..];
        let mut section = Self::default();
        for source in metadata.sources {
            section.sources.push(SectionSource {
                family: source.family,
                copyright: source.copyright,
                license: source.license,
                license_text: source.license_text,
                sha256: unhex(&source.sha256).ok_or_else(|| fail("bad source sha256"))?,
            });
        }
        let mut next_glyph = 0usize;
        let mut next_bitmap = 0usize;
        for target in metadata.targets {
            let start = target.glyph_start as usize;
            let count = target.glyph_count as usize;
            if start != next_glyph || start + count > glyph_count {
                return Err(fail("target glyph ranges must be contiguous"));
            }
            let mut glyphs = Vec::with_capacity(count);
            for index in start..start + count {
                let at = glyphs_start + index * GLYPH_RECORD_SIZE;
                let record = &bytes[at..at + GLYPH_RECORD_SIZE];
                let character = char::from_u32(u32_at(at))
                    .ok_or_else(|| fail("glyph is not a Unicode scalar value"))?;
                let (width, height) = (record[4], record[5]);
                let offset = usize_at(at + 8);
                let size = usize::from(width) * usize::from(height);
                if offset != next_bitmap || offset + size > bitmaps.len() || u32_at(at + 12) != 0 {
                    return Err(fail("glyph bitmaps must tile the bitmap area in order"));
                }
                next_bitmap += size;
                glyphs.push(SectionGlyph {
                    character,
                    width,
                    height,
                    offset_y: record[6],
                    advance: record[7],
                    bitmap: bitmaps[offset..offset + size].to_vec(),
                });
            }
            next_glyph += count;
            section.targets.push(SectionTarget {
                font: target.font,
                size: target.size,
                line_height: target.line_height,
                ascent: target.ascent,
                source: target.source,
                glyphs,
            });
        }
        if next_glyph != glyph_count || next_bitmap != bitmaps.len() {
            return Err(fail("glyphs or bitmaps are not referenced by a target"));
        }
        section.validate()?;
        Ok(section)
    }
}

fn target_key(target: &SectionTarget) -> (&[u8], &[u8]) {
    (target.font.as_bytes(), target.size.as_bytes())
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section() -> FontSection {
        FontSection {
            sources: vec![SectionSource {
                family: "Oswald".to_owned(),
                copyright: "Copyright 2016 The Oswald Project Authors".to_owned(),
                license: "OFL-1.1".to_owned(),
                license_text: "SIL OPEN FONT LICENSE\n".to_owned(),
                sha256: [7; 32],
            }],
            targets: vec![
                SectionTarget {
                    font: "TrumpGothic".to_owned(),
                    size: "184".to_owned(),
                    line_height: 24,
                    ascent: 19,
                    source: 0,
                    glyphs: vec![
                        SectionGlyph {
                            character: 'А',
                            width: 2,
                            height: 2,
                            offset_y: 5,
                            advance: 3,
                            bitmap: vec![1, 2, 3, 4],
                        },
                        SectionGlyph {
                            character: 'Б',
                            width: 1,
                            height: 3,
                            offset_y: 5,
                            advance: 2,
                            bitmap: vec![5, 6, 7],
                        },
                    ],
                },
                SectionTarget {
                    font: "TrumpGothic".to_owned(),
                    size: "23".to_owned(),
                    line_height: 30,
                    ascent: 24,
                    source: 0,
                    glyphs: vec![SectionGlyph {
                        character: 'Ё',
                        width: 0,
                        height: 0,
                        offset_y: 0,
                        advance: 4,
                        bitmap: vec![],
                    }],
                },
            ],
        }
    }

    #[test]
    fn section_round_trips_and_is_aligned() {
        let bytes = section().encode().expect("encode");
        assert_eq!(&bytes[..8], MAGIC);
        let metadata_length = u32::from_le_bytes(bytes[8..12].try_into().expect("4")) as usize;
        let glyphs = (HEADER_SIZE + metadata_length).next_multiple_of(8);
        assert_eq!(bytes.len(), glyphs + 3 * GLYPH_RECORD_SIZE + 7);
        assert_eq!(&bytes[bytes.len() - 7..], &[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(FontSection::decode(&bytes).expect("decode"), section());
    }

    #[test]
    fn invalid_sections_are_rejected() {
        let mut unsorted = section();
        unsorted.targets.swap(0, 1);
        let mut glyph_order = section();
        glyph_order.targets[0].glyphs.swap(0, 1);
        let mut ascii = section();
        ascii.targets[0].glyphs[0].character = 'A';
        let mut bitmap = section();
        bitmap.targets[0].glyphs[0].bitmap.pop();
        let mut below = section();
        below.targets[0].glyphs[0].offset_y = 23;
        let mut unused = section();
        unused.sources.push(unused.sources[0].clone());
        let mut crlf = section();
        crlf.sources[0].license_text = "a\r\n".to_owned();
        for invalid in [unsorted, glyph_order, ascii, bitmap, below, unused, crlf] {
            assert!(invalid.encode().is_err(), "{invalid:?}");
        }
        let mut bytes = section().encode().expect("encode");
        bytes.push(0);
        assert!(FontSection::decode(&bytes).is_err());
    }
}
