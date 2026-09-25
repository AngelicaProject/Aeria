use aeria_fonts::{
    DEFAULT_CHARACTERS, FONT_SETTINGS_FILE, FontSection, FontSettings, GAME_FONTS, game_font,
    generate, install_recommended_files, preview_line,
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

    let section = generate(temp.path(), &loaded).expect("generate");
    let sizes: usize = GAME_FONTS.iter().map(|font| font.sizes.len()).sum();
    assert_eq!(section.targets.len(), sizes);
    let characters = DEFAULT_CHARACTERS.chars().count();
    for target in &section.targets {
        assert_eq!(
            target.glyphs.len(),
            characters,
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
        assert_eq!(
            capital.offset_y + capital.height,
            size.ascent,
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

    let bytes = section.encode().expect("encode");
    assert_eq!(
        bytes,
        generate(temp.path(), &loaded)
            .expect("again")
            .encode()
            .expect("encode")
    );
    assert_eq!(FontSection::decode(&bytes).expect("decode"), section);

    let size = game_font("Jupiter")
        .and_then(|font| font.size("16"))
        .expect("size");
    let line = preview_line(size, &section.targets[0].glyphs, "Ёж и ё");
    assert_eq!(line.height, 26);
    assert!(line.missing.is_empty());
    assert!(line.pixels.iter().any(|p| *p > 0));
}

#[test]
fn a_missing_glyph_fails_generation() {
    let temp = tempfile::tempdir().expect("temp");
    let mut settings = install_recommended_files(temp.path()).expect("install");
    settings.characters.push('\u{A640}');
    let error = generate(temp.path(), &settings).expect_err("missing glyph");
    assert!(error.to_string().contains("U+A640"), "{error}");
}
