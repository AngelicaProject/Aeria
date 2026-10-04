use aeria_fonts::{
    DEFAULT_CHARACTERS, FONT_SETTINGS_FILE, FontSection, FontSettings, GAME_FONTS,
    REPLACED_CHARACTERS, SectionGlyph, game_font, generate, install_recommended_files,
    preview_line,
};

#[test]
fn recommended_fonts_cover_every_target_deterministically() {
    let temp = tempfile::tempdir().expect("temp");
    let settings = install_recommended_files(temp.path()).expect("install");
    settings.save(temp.path()).expect("save");
    assert!(temp.path().join(FONT_SETTINGS_FILE).exists());
    let loaded = FontSettings::load(temp.path())
        .expect("load")
        .expect("settings");

    let fonts = generate(temp.path(), &loaded).expect("generate");
    let section = fonts.added.clone().expect("added glyphs");
    let replaced = fonts.replaced.clone().expect("replaced glyphs");
    let sizes = |replaces: bool| -> usize {
        GAME_FONTS
            .iter()
            .filter(|font| font.replaces == replaces)
            .map(|font| font.sizes.len())
            .sum()
    };
    assert_eq!(section.targets.len(), sizes(false));
    assert_eq!(replaced.targets.len(), sizes(true));
    assert!(replaced.targets.iter().all(|target| target.font == "AXIS"));
    let characters = DEFAULT_CHARACTERS.chars().count();
    // A replacing font gets only the Cyrillic block; `№` and quotes stay the
    // game's own.
    let cyrillic = DEFAULT_CHARACTERS
        .chars()
        .filter(|c| REPLACED_CHARACTERS.contains(c))
        .count();
    assert!(cyrillic < characters);
    for (target, expected) in section
        .targets
        .iter()
        .map(|target| (target, characters))
        .chain(replaced.targets.iter().map(|target| (target, cyrillic)))
    {
        assert_eq!(
            target.glyphs.len(),
            expected,
            "{}_{}",
            target.font,
            target.size
        );
        let size = game_font(&target.font)
            .and_then(|font| font.size(&target.size))
            .expect("size");
        assert_eq!(
            (target.line_height, target.ascent),
            (size.line_height, size.ascent)
        );
        // Capitals sit on the native baseline.
        let capital = target
            .glyphs
            .iter()
            .find(|glyph| glyph.character == 'Н')
            .expect("Н");
        let (top, bottom) = ink_rows(capital);
        assert_eq!(bottom + 1, size.ascent, "{}_{}", target.font, target.size);
        assert_eq!(
            bottom + 1 - top,
            size.cap_height,
            "{}_{}",
            target.font,
            target.size
        );
    }
    // MiedingerMid draws small letters with the capital glyphs.
    let miedinger = section
        .targets
        .iter()
        .find(|target| target.font == "MiedingerMid")
        .expect("target");
    let glyph = |c: char| {
        miedinger
            .glyphs
            .iter()
            .find(|glyph| glyph.character == c)
            .expect("glyph")
    };
    assert_eq!(glyph('ж').bitmap, glyph('Ж').bitmap);

    let again = generate(temp.path(), &loaded).expect("again");
    for (first, second) in [
        (&section, again.added.as_ref().expect("added")),
        (&replaced, again.replaced.as_ref().expect("replaced")),
    ] {
        let bytes = first.encode().expect("encode");
        assert_eq!(bytes, second.encode().expect("encode"));
        assert_eq!(&FontSection::decode(&bytes).expect("decode"), first);
    }

    let size = game_font("Jupiter")
        .and_then(|font| font.size("16"))
        .expect("size");
    let line = preview_line(size, &section.targets[0].glyphs, "Ёж и ё");
    assert_eq!(line.height, 26);
    assert!(line.missing.is_empty());
    assert!(line.pixels.iter().any(|p| *p > 0));
}

/// The first and last line rows of a glyph with ink the game's 4-bit pages
/// keep.
fn ink_rows(glyph: &SectionGlyph) -> (u8, u8) {
    let rows: Vec<u8> = (glyph.offset_y..glyph.offset_y + glyph.height)
        .filter(|line| row(glyph, *line).iter().any(|value| *value > 0))
        .collect();
    (rows[0], rows[rows.len() - 1])
}

/// Coverage of one row of a glyph, as the game's 4-bit pages keep it.
fn row(glyph: &SectionGlyph, line: u8) -> Vec<u8> {
    let width = usize::from(glyph.width);
    let row = usize::from(line - glyph.offset_y);
    glyph.bitmap[row * width..(row + 1) * width]
        .iter()
        .map(|value| u8::try_from((u16::from(*value) * 15 + 127) / 255).expect("4 bits"))
        .collect()
}

/// Runs of ink in a row: (first column, coverage of the first pixel, sum).
fn stems(row: &[u8]) -> Vec<(usize, u8, u32)> {
    let mut runs = Vec::new();
    let mut column = 0;
    while column < row.len() {
        if row[column] == 0 {
            column += 1;
            continue;
        }
        let start = column;
        while column < row.len() && row[column] != 0 {
            column += 1;
        }
        let sum = row[start..column].iter().map(|v| u32::from(*v)).sum();
        runs.push((start, row[start], sum));
    }
    runs
}

#[test]
fn replacements_sit_on_the_native_grid() {
    let temp = tempfile::tempdir().expect("temp");
    let settings = install_recommended_files(temp.path()).expect("install");
    let replaced = generate(temp.path(), &settings)
        .expect("generate")
        .replaced
        .expect("replaced");
    let target = replaced
        .targets
        .iter()
        .find(|target| target.font == "AXIS" && target.size == "14")
        .expect("AXIS_14");
    let grid = game_font("AXIS")
        .and_then(|font| font.size("14"))
        .and_then(|size| size.grid)
        .expect("grid");
    let glyph = |c: char| {
        target
            .glyphs
            .iter()
            .find(|glyph| glyph.character == c)
            .expect("glyph")
    };
    // A row in the middle of the small letters: every vertical stem of
    // н, п, ш, ц starts with the same coverage and holds the native width.
    let middle = target.ascent - grid.x_height / 2;
    let mut first_pixels = Vec::new();
    for c in ['н', 'п', 'ш', 'ц'] {
        let runs = stems(&row(glyph(c), middle));
        assert!(runs.len() >= 2, "{c}: {runs:?}");
        for (_, first, sum) in [runs[0], runs[runs.len() - 1]] {
            first_pixels.push(first);
            // 1.4 px of ink, within a 4-bit step per pixel.
            assert!((19..=23).contains(&sum), "{c}: {runs:?}");
        }
    }
    assert!(
        first_pixels
            .iter()
            .all(|first| first.abs_diff(first_pixels[0]) <= 1),
        "{first_pixels:?}"
    );
    // The small letters end on the native x-height and baseline.
    // Replacements span the line like the game's own glyphs, so the game's
    // italic slant is the same for them.
    for target in &replaced.targets {
        assert!(
            target
                .glyphs
                .iter()
                .all(|glyph| glyph.offset_y == 0 && glyph.height == target.line_height),
            "{}_{}",
            target.font,
            target.size
        );
    }
    let (top, bottom) = ink_rows(glyph('н'));
    assert_eq!(
        (top, bottom + 1),
        (target.ascent - grid.x_height, target.ascent)
    );
}

#[test]
fn a_missing_glyph_fails_generation() {
    let temp = tempfile::tempdir().expect("temp");
    let mut settings = install_recommended_files(temp.path()).expect("install");
    settings.characters.push('\u{A640}');
    let error = generate(temp.path(), &settings).expect_err("missing glyph");
    assert!(error.to_string().contains("U+A640"), "{error}");
}
