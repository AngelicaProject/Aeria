//! Glyph rasterization with `swash`.
//!
//! A size is fitted so the source capital `H` is as tall as the native
//! capitals (`capHeight × scale`), then placed on the native baseline
//! (`ascent + baselineShift`). Outlines and advances are scaled horizontally by
//! `widthScale`. The bitmap of a glyph starts at the pen position, as game
//! `.fdt` glyphs do, and is trimmed to its ink rows. A size with a native
//! pixel grid (the replaced `AXIS`) is fitted to it first; see `grid`.

use std::path::Path;

use sha2::{Digest, Sha256};
use swash::scale::{Render, ScaleContext, Scaler, Source};
use swash::zeno::{Command, Format, Mask, PathData, Placement, Transform, Vector};
use swash::{FontRef, Setting, tag_from_bytes};

use crate::error::FontError;
use crate::grid::{self, Zones};
use crate::section::{FontSection, SectionGlyph, SectionSource, SectionTarget};
use crate::settings::{CaseMapping, FontSettings, FontSource, SizeParameters, project_path};
use crate::targets::{GameFont, GameFontSize, PixelGrid, REPLACED_CHARACTERS, game_font};

/// A source font file read from the project.
#[derive(Clone, Debug)]
pub struct LoadedSource {
    pub bytes: Vec<u8>,
    pub sha256: [u8; 32],
    /// License text with LF line endings, whatever Git checked out.
    pub license_text: String,
}

/// Reads a source font and its license text from the project.
///
/// # Errors
/// Returns [`FontError::Io`] or [`FontError::Source`] for a missing or
/// unreadable file or a license text that is not UTF-8.
pub fn load_source(project_root: &Path, source: &FontSource) -> Result<LoadedSource, FontError> {
    let read = |relative: &str| {
        std::fs::read(project_path(project_root, relative)).map_err(|error| FontError::Io {
            path: relative.to_owned(),
            source: error,
        })
    };
    let bytes = read(&source.file)?;
    let license_text = String::from_utf8(read(&source.license_file)?)
        .map_err(|_| source_error(source, "the license file is not UTF-8"))?
        .replace("\r\n", "\n");
    if FontRef::from_index(&bytes, 0).is_none() {
        return Err(source_error(
            source,
            "the file is not a TrueType or OpenType font",
        ));
    }
    Ok(LoadedSource {
        sha256: Sha256::digest(&bytes).into(),
        bytes,
        license_text,
    })
}

/// Renders every glyph of `characters` for one game font size.
///
/// # Errors
/// Returns [`FontError`] when an axis is missing or out of range, the source
/// lacks a glyph, or a glyph does not fit the `.fdt` record limits.
pub fn render_size(
    font: &GameFont,
    size: &GameFontSize,
    parameters: &SizeParameters<'_>,
    source: &LoadedSource,
    characters: &[char],
) -> Result<Vec<SectionGlyph>, FontError> {
    let fail = |reason: String| FontError::Glyph {
        font: font.name.to_owned(),
        size: size.size.to_owned(),
        reason,
    };
    let font_ref = FontRef::from_index(&source.bytes, 0)
        .ok_or_else(|| source_error(parameters.source, "the file is not a font"))?;
    let coords = variation_coords(&font_ref, parameters)?;
    let units_per_em = f32::from(font_ref.metrics(&coords).units_per_em);
    let charmap = font_ref.charmap();

    let mut context = ScaleContext::new();
    let reference = ['H', 'Н']
        .into_iter()
        .map(|c| charmap.map(c))
        .find(|glyph| *glyph != 0)
        .ok_or_else(|| source_error(parameters.source, "the font has no capital H"))?;
    let mut unscaled = context
        .builder(font_ref)
        .size(units_per_em)
        .normalized_coords(&coords)
        .build();
    let cap_units = unscaled
        .scale_outline(reference)
        .map(|outline| outline.bounds().max.y)
        .filter(|height| *height > 0.0)
        .ok_or_else(|| source_error(parameters.source, "the capital H has no outline"))?;

    #[allow(clippy::cast_possible_truncation)]
    let (scale, width_scale) = (parameters.scale as f32, parameters.width_scale as f32);
    let ppem = f32::from(size.cap_height) * scale * units_per_em / cap_units;
    let mut scaler = context
        .builder(font_ref)
        .size(ppem)
        .hint(false)
        .normalized_coords(&coords)
        .build();
    let metrics = font_ref.glyph_metrics(&coords).scale(ppem);
    let mut render = Render::new(&[Source::Outline]);
    render
        .format(Format::Alpha)
        .transform(Some(Transform::scale(width_scale, 1.0)));
    let fitting = fitting(
        &mut scaler,
        &charmap,
        &metrics,
        size,
        (scale, width_scale),
        characters,
    )
    .map_err(|reason| source_error(parameters.source, reason))?;

    let baseline = i32::from(size.ascent) + parameters.baseline_shift;
    #[allow(clippy::cast_possible_truncation)]
    let tracking = (parameters.tracking * f64::from(size.cap_height)) as f32;
    let line_height = i32::from(size.line_height);
    let mut glyphs = Vec::with_capacity(characters.len());
    for &character in characters {
        let at = |reason: &str| {
            fail(format!(
                "U+{:04X} {character}: {reason}",
                u32::from(character)
            ))
        };
        let drawn = match parameters.case_mapping {
            CaseMapping::None => character,
            CaseMapping::Upper => single_upper(character),
        };
        let glyph_id = charmap.map(drawn);
        if glyph_id == 0 {
            return Err(at(&format!(
                "the source {} has no glyph for it",
                parameters.source.family
            )));
        }
        let advance = metrics.advance_width(glyph_id) * width_scale + tracking;
        let (placement, data, advance) = if let Some(fitting) = &fitting {
            fitted(&mut scaler, glyph_id, width_scale, fitting, advance)
                .ok_or_else(|| at("the glyph has no outline"))?
        } else {
            let image = render
                .render(&mut scaler, glyph_id)
                .ok_or_else(|| at("the glyph could not be rendered"))?;
            (image.placement, image.data, advance)
        };
        glyphs.push(
            place(&placement, &data, advance, baseline, line_height)
                .map(|placed| SectionGlyph {
                    character,
                    ..placed
                })
                .map_err(|reason| at(&reason))?,
        );
    }
    Ok(glyphs)
}

/// How the glyphs of a size with a native pixel grid are fitted to it.
struct Fitting {
    grid: PixelGrid,
    zones: Zones,
    /// How far the glyphs move right; see [`bearing_shift`].
    shift: f32,
}

/// The fitting of a size with a native pixel grid, `None` for a size
/// without one.
fn fitting(
    scaler: &mut Scaler<'_>,
    charmap: &swash::Charmap<'_>,
    metrics: &swash::GlyphMetrics<'_>,
    size: &GameFontSize,
    (scale, width_scale): (f32, f32),
    characters: &[char],
) -> Result<Option<Fitting>, &'static str> {
    let Some(grid) = size.grid else {
        return Ok(None);
    };
    let zones =
        zones(scaler, charmap, size, scale).ok_or("the font has no small x to fit to the grid")?;
    let shift = bearing_shift(scaler, charmap, metrics, characters, width_scale, &grid);
    Ok(Some(Fitting { grid, zones, shift }))
}

/// The zones of the source at this size and where the native grid puts
/// them: the x-height of the small `x` (or `х`) and the cap height, scaled
/// like the glyphs. `None` when the font has neither `x`.
fn zones(
    scaler: &mut Scaler<'_>,
    charmap: &swash::Charmap<'_>,
    size: &GameFontSize,
    scale: f32,
) -> Option<Zones> {
    let grid = size.grid?;
    let x_height = ['x', 'х']
        .into_iter()
        .map(|c| charmap.map(c))
        .find(|glyph| *glyph != 0)
        .and_then(|glyph| scaler.scale_outline(glyph))
        .map(|outline| outline.bounds().max.y)?;
    let cap_height = f32::from(size.cap_height) * scale;
    Some(Zones {
        x_height: (x_height, (f32::from(grid.x_height) * scale).round()),
        cap_height: (cap_height, cap_height.round()),
    })
}

/// How far to move the glyphs right so that the space between letters is
/// split between their sides as in the native Latin: the native split less
/// the mean split of the small letters of `characters` (all of them when
/// none is small).
fn bearing_shift(
    scaler: &mut Scaler<'_>,
    charmap: &swash::Charmap<'_>,
    metrics: &swash::GlyphMetrics<'_>,
    characters: &[char],
    width_scale: f32,
    grid: &PixelGrid,
) -> f32 {
    let small: Vec<char> = characters
        .iter()
        .copied()
        .filter(|c| c.is_lowercase())
        .collect();
    let measured = if small.is_empty() { characters } else { &small };
    let splits: Vec<f32> = measured
        .iter()
        .map(|c| charmap.map(*c))
        .filter(|glyph| *glyph != 0)
        .filter_map(|glyph| {
            let bounds = scaler.scale_outline(glyph)?.bounds();
            let left = bounds.min.x * width_scale;
            let right = (metrics.advance_width(glyph) - bounds.max.x) * width_scale;
            Some((left - right) / 2.0)
        })
        .collect();
    if splits.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let mean = splits.iter().sum::<f32>() / splits.len() as f32;
    f32::from(grid.bearing_split_centi) / 100.0 - mean
}

/// Renders a glyph fitted to the native grid: outline scaled by
/// `width_scale` and moved right by the bearing shift, fitted, and filled.
/// The advance follows the right side.
fn fitted(
    scaler: &mut Scaler<'_>,
    glyph_id: u16,
    width_scale: f32,
    fitting: &Fitting,
    advance: f32,
) -> Option<(Placement, Vec<u8>, f32)> {
    let shift = fitting.shift;
    let outline = scaler.scale_outline(glyph_id)?;
    let mut commands: Vec<Command> = outline
        .path()
        .commands()
        .map(|command| {
            grid::map_points(command, |point| {
                Vector::new(point.x * width_scale + shift, point.y)
            })
        })
        .collect();
    let moved = grid::fit(&mut commands, &fitting.grid, &fitting.zones, advance);
    // The mask is drawn with y down.
    let commands: Vec<Command> = commands
        .into_iter()
        .map(|command| grid::map_points(command, |point| Vector::new(point.x, -point.y)))
        .collect();
    let (data, placement) = Mask::new(&commands[..]).format(Format::Alpha).render();
    Some((
        Placement {
            top: -placement.top,
            ..placement
        },
        data,
        advance + moved,
    ))
}

/// Normalized variation coordinates of the configured axis values.
fn variation_coords(
    font_ref: &FontRef<'_>,
    parameters: &SizeParameters<'_>,
) -> Result<Vec<i16>, FontError> {
    let variations = font_ref.variations();
    let mut settings = Vec::new();
    for (tag, value) in parameters.axes {
        let bytes: [u8; 4] = tag
            .as_bytes()
            .try_into()
            .map_err(|_| source_error(parameters.source, &format!("axis tag {tag}")))?;
        let tag_value = tag_from_bytes(&bytes);
        let axis = variations.find_by_tag(tag_value).ok_or_else(|| {
            source_error(parameters.source, &format!("the font has no {tag} axis"))
        })?;
        #[allow(clippy::cast_possible_truncation)]
        let value = *value as f32;
        if value < axis.min_value() || value > axis.max_value() {
            return Err(source_error(
                parameters.source,
                &format!(
                    "{tag} {value} is outside the axis range {}..={}",
                    axis.min_value(),
                    axis.max_value()
                ),
            ));
        }
        settings.push(Setting {
            tag: tag_value,
            value,
        });
    }
    Ok(variations
        .normalized_coords(settings.iter().copied())
        .collect())
}

/// Turns a rendered coverage image into a pen-relative, row-trimmed glyph.
fn place(
    placement: &swash::zeno::Placement,
    data: &[u8],
    advance: f32,
    baseline: i32,
    line_height: i32,
) -> Result<SectionGlyph, String> {
    let (width, height) = (
        usize::try_from(placement.width).map_err(|_| "too wide")?,
        usize::try_from(placement.height).map_err(|_| "too tall")?,
    );
    // Ink left of the pen cannot be drawn by an .fdt glyph; move the glyph
    // right and widen its advance instead of cutting it.
    let shift = (-placement.left).max(0);
    let left = usize::try_from(placement.left + shift).map_err(|_| "negative bearing")?;
    let top = baseline - placement.top;
    let rows: Vec<(i32, &[u8])> = (0..height)
        .map(|row| {
            (
                top + i32::try_from(row).unwrap_or(i32::MAX),
                &data[row * width..(row + 1) * width],
            )
        })
        .filter(|(line, pixels)| (0..line_height).contains(line) && pixels.iter().any(|p| *p != 0))
        .collect();
    let first = rows.first().map_or(0, |(line, _)| *line);
    let last = rows.last().map_or(-1, |(line, _)| *line);
    let columns = rows
        .iter()
        .filter_map(|(_, pixels)| pixels.iter().rposition(|p| *p != 0))
        .max()
        .map_or(0, |column| left + column + 1);
    let row_count = usize::try_from(last - first + 1).unwrap_or(0);

    let mut bitmap = vec![0u8; columns * row_count];
    for (line, pixels) in &rows {
        let row = usize::try_from(line - first).expect("rows are ascending");
        for (column, pixel) in pixels.iter().enumerate() {
            if left + column < columns {
                bitmap[row * columns + left + column] = *pixel;
            }
        }
    }
    #[allow(clippy::cast_possible_truncation)]
    let advance = (advance.round() as i32) + shift;
    let width = u8::try_from(columns).map_err(|_| "the glyph is wider than 255 pixels")?;
    let advance = u8::try_from(advance).map_err(|_| "the advance is outside 0..=255")?;
    if i8::try_from(i16::from(advance) - i16::from(width)).is_err() {
        return Err("advance − width does not fit an .fdt record".to_owned());
    }
    Ok(SectionGlyph {
        character: '\0',
        width,
        height: u8::try_from(row_count).map_err(|_| "the glyph is taller than 255 pixels")?,
        offset_y: if row_count == 0 {
            0
        } else {
            u8::try_from(first).map_err(|_| "offset")?
        },
        advance,
        bitmap,
    })
}

fn single_upper(character: char) -> char {
    let mut upper = character.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => character,
    }
}

fn source_error(source: &FontSource, reason: &str) -> FontError {
    FontError::Source {
        source_id: source.id.clone(),
        reason: reason.to_owned(),
    }
}

/// The characters of the settings, sorted and unique.
#[must_use]
pub fn character_set(settings: &FontSettings) -> Vec<char> {
    let mut characters: Vec<char> = settings.characters.chars().collect();
    characters.sort_unstable();
    characters.dedup();
    characters
}

/// The characters rendered for `font`: all of `characters`, or for a
/// replacing font those of [`REPLACED_CHARACTERS`].
#[must_use]
pub fn characters_for(font: &GameFont, characters: &[char]) -> Vec<char> {
    characters
        .iter()
        .copied()
        .filter(|c| !font.replaces || REPLACED_CHARACTERS.contains(c))
        .collect()
}

/// The glyphs of a pack: those added to game fonts that lack them (the
/// `FONTS` section) and those that replace a game font's own (the
/// font-replacements section). A section without targets is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackFonts {
    pub added: Option<FontSection>,
    pub replaced: Option<FontSection>,
}

/// Renders every configured game font size. Targets of a replacing game
/// font go into [`PackFonts::replaced`], the others into
/// [`PackFonts::added`]; in each, targets are ordered by font and size bytes
/// and sources by first use.
///
/// # Errors
/// Returns the first settings, file, or rendering error.
///
/// # Panics
/// Never for validated settings.
pub fn generate(project_root: &Path, settings: &FontSettings) -> Result<PackFonts, FontError> {
    settings.validate()?;
    let characters = character_set(settings);
    let mut added = FontSection::default();
    let mut replaced = FontSection::default();
    // Source ids in the order of each section's sources.
    let (mut added_ids, mut replaced_ids) = (Vec::<String>::new(), Vec::<String>::new());
    let mut loaded: Vec<(String, LoadedSource)> = Vec::new();
    let mut targets: Vec<(&'static GameFont, &'static GameFontSize, SizeParameters<'_>)> =
        Vec::new();
    for target in &settings.fonts {
        let font = game_font(&target.font).expect("validated font");
        for size in font.sizes {
            let parameters = settings
                .parameters(target, size.size)
                .expect("validated source");
            targets.push((font, size, parameters));
        }
    }
    targets.sort_by(|a, b| {
        (a.0.name.as_bytes(), a.1.size.as_bytes()).cmp(&(b.0.name.as_bytes(), b.1.size.as_bytes()))
    });

    for (font, size, parameters) in targets {
        let file = if let Some(file) = loaded
            .iter()
            .position(|(id, _)| *id == parameters.source.id)
        {
            file
        } else {
            let source = load_source(project_root, parameters.source)?;
            loaded.push((parameters.source.id.clone(), source));
            loaded.len() - 1
        };
        let source = &loaded[file].1;
        let (section, ids) = if font.replaces {
            (&mut replaced, &mut replaced_ids)
        } else {
            (&mut added, &mut added_ids)
        };
        let index = if let Some(index) = ids.iter().position(|id| *id == parameters.source.id) {
            index
        } else {
            ids.push(parameters.source.id.clone());
            section.sources.push(SectionSource {
                family: parameters.source.family.clone(),
                copyright: parameters.source.copyright.clone(),
                license: parameters.source.license.clone(),
                license_text: source.license_text.clone(),
                sha256: source.sha256,
            });
            section.sources.len() - 1
        };
        let glyphs = render_size(
            font,
            size,
            &parameters,
            source,
            &characters_for(font, &characters),
        )?;
        section.targets.push(SectionTarget {
            font: font.name.to_owned(),
            size: size.size.to_owned(),
            line_height: size.line_height,
            ascent: size.ascent,
            source: index,
            glyphs,
        });
    }
    let finish = |section: FontSection| -> Result<Option<FontSection>, FontError> {
        if section.targets.is_empty() {
            return Ok(None);
        }
        section.validate()?;
        Ok(Some(section))
    };
    Ok(PackFonts {
        added: finish(added)?,
        replaced: finish(replaced)?,
    })
}
