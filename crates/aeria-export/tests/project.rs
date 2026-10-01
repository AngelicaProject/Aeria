//! Collecting a project's translations from a synthetic game.

use std::path::Path;
use std::sync::Arc;

use aeria_export::{
    Channel, ExportError, PackManifest, StringEncoder, Team, collect_project, pack_game,
    source_guard, write_pack,
};
use aeria_source::{GameSource, SourceLanguage};
use aeria_sqpack::testing::{FakeGame, TextSheet};
use tempfile::TempDir;

// Plain text encodes to its UTF-8 bytes.
struct FakeEncoder {
    calls: usize,
    reject: Option<&'static str>,
}

impl StringEncoder for FakeEncoder {
    fn encode(&mut self, macros: &[&str]) -> Result<Vec<Result<Vec<u8>, String>>, String> {
        self.calls += 1;
        Ok(macros
            .iter()
            .map(|text| {
                if Some(*text) == self.reject {
                    Err("not round-trip".to_owned())
                } else {
                    Ok(text.as_bytes().to_vec())
                }
            })
            .collect())
    }
}

fn encoder() -> FakeEncoder {
    FakeEncoder {
        calls: 0,
        reject: None,
    }
}

struct Game {
    _folder: TempDir,
    source: Arc<GameSource>,
}

fn game(sheet: &TextSheet) -> Game {
    let folder = tempfile::tempdir().expect("game");
    FakeGame::new("2026.09.15.0000.0000")
        .with_text("Synthetic", sheet)
        .write(folder.path())
        .expect("write");
    let source = GameSource::open(folder.path(), SourceLanguage::English).expect("source");
    Game {
        _folder: folder,
        source: Arc::new(source),
    }
}

/// Two String columns at 0 and 2; rows 7 and 42.
fn sheet() -> TextSheet {
    TextSheet::new(3, &[0, 2])
        .row(7, &[(0, "Two"), (2, "Other")])
        .row(42, &[(0, "One")])
}

/// A project for `source` with row 42 translated, row 7 translated and
/// marked fuzzy when `fuzzy_seven`, and the other string untranslated.
fn project(source: &GameSource, fuzzy_seven: bool) -> TempDir {
    let folder = tempfile::tempdir().expect("project");
    aeria_po::create(folder.path(), source, "ru", 1).expect("create");
    let path = folder.path().join("po/Synthetic.po");
    let text = std::fs::read_to_string(&path).expect("file");
    let seven = if fuzzy_seven {
        "#, fuzzy\nmsgctxt \"Synthetic:7:0:0\"\nmsgid \"Two\"\nmsgstr \"Два\""
    } else {
        "msgctxt \"Synthetic:7:0:0\"\nmsgid \"Two\"\nmsgstr \"Два\""
    };
    let text = text
        .replace(
            "msgctxt \"Synthetic:7:0:0\"\nmsgid \"Two\"\nmsgstr \"\"",
            seven,
        )
        .replace(
            "msgctxt \"Synthetic:42:0:0\"\nmsgid \"One\"\nmsgstr \"\"",
            "msgctxt \"Synthetic:42:0:0\"\nmsgid \"One\"\nmsgstr \"Раз\"",
        );
    std::fs::write(&path, text).expect("write");
    folder
}

fn root(folder: &TempDir) -> &Path {
    folder.path()
}

#[test]
fn translations_are_exported_with_the_games_layout_and_guard_and_fuzzy_ones_are_not() {
    let game = game(&sheet());
    let folder = project(&game.source, true);
    let export = collect_project(root(&folder), &game.source, &mut encoder()).expect("export");
    assert_eq!(export.report.exported, 1);
    assert_eq!(export.report.skipped_fuzzy, 1);
    assert_eq!(export.report.skipped_untranslated, 1);
    let sheet = &export.sheets[0];
    assert_eq!(sheet.name, "Synthetic");
    let layout: Vec<_> = sheet
        .layout
        .iter()
        .map(|column| (column.column_index, column.offset))
        .collect();
    assert_eq!(layout, [(0, 0), (2, 8)], "every String column of the game");
    assert_eq!(sheet.cells.len(), 1);
    assert_eq!(sheet.cells[0].row_id, 42);
    assert_eq!(sheet.cells[0].text, "Раз".as_bytes());
    assert_eq!(sheet.cells[0].source_guard, source_guard(b"One"));
}

#[test]
fn a_collected_project_writes_a_pack_for_the_games_version() {
    let game = game(&sheet());
    let folder = project(&game.source, false);
    let export = collect_project(root(&folder), &game.source, &mut encoder()).expect("export");
    assert_eq!(export.report.exported, 2);
    let game_facts = pack_game(&game.source);
    assert_eq!(game_facts.language, "en");
    assert_eq!(game_facts.version, "2026.09.15.0000.0000");
    let manifest = PackManifest {
        title: "Synthetic".to_owned(),
        team: Team {
            name: "Tests".to_owned(),
            url: None,
        },
        authors: Vec::new(),
        license: None,
        version: "2026.10.01.0001".parse().expect("version"),
        channel: Channel::Testing,
        language: "ru".to_owned(),
        game: game_facts,
        aeria: "0.1.0".to_owned(),
        commit: "0".repeat(40),
        min_harmonia: "0.1.0".to_owned(),
    };
    let pack = write_pack(&manifest, export.sheets, None, None).expect("pack");
    assert_eq!(pack.counts.cells, 2);
}

#[test]
fn a_translation_whose_source_changed_in_the_game_fails_the_export() {
    let old = game(&sheet());
    let folder = project(&old.source, false);
    let patched = game(
        &TextSheet::new(3, &[0, 2])
            .row(7, &[(0, "Two, revised"), (2, "Other")])
            .row(42, &[(0, "One")]),
    );
    let result = collect_project(root(&folder), &patched.source, &mut encoder());
    assert!(
        matches!(&result, Err(ExportError::Project(message)) if message.contains("Synthetic:7:0:0")),
        "{result:?}"
    );
}

#[test]
fn a_failed_encoding_or_a_short_encoder_fails_the_export() {
    struct Short;
    impl StringEncoder for Short {
        fn encode(&mut self, _: &[&str]) -> Result<Vec<Result<Vec<u8>, String>>, String> {
            Ok(Vec::new())
        }
    }

    let game = game(&sheet());
    let folder = project(&game.source, false);
    let mut rejecting = FakeEncoder {
        calls: 0,
        reject: Some("Два"),
    };
    let result = collect_project(root(&folder), &game.source, &mut rejecting);
    assert!(matches!(result, Err(ExportError::Cell { row_id: 7, .. })));
    assert_eq!(rejecting.calls, 1);

    let result = collect_project(root(&folder), &game.source, &mut Short);
    assert!(matches!(result, Err(ExportError::Encoder(_))));
}
