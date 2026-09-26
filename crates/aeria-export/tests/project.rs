//! Collecting a project's translations from a synthetic game.

use std::sync::Arc;

use aeria_core::ReviewState;
use aeria_export::{
    Channel, ContentPolicy, ExportError, PackManifest, Publisher, StringEncoder, collect_project,
    pack_source, source_guard, write_pack,
};
use aeria_source::{GameSource, SheetLookup, SourceLanguage};
use aeria_sqpack::testing::{FakeGame, TextSheet};
use aeria_workspace::Workspace;
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

fn workspace(source: &GameSource, review_seven: bool) -> Workspace {
    let SheetLookup::Present(sheet) = source.sheet("Synthetic").expect("sheet") else {
        panic!("sheet");
    };
    let mut workspace = Workspace::for_source(source, "ru").expect("workspace");
    workspace
        .create_unit(sheet.facts(42, 0, 0).expect("facts"), "Раз")
        .expect("unit");
    let seven = workspace
        .create_unit(sheet.facts(7, 0, 0).expect("facts"), "Два")
        .expect("unit");
    if review_seven {
        workspace
            .update_review_state(seven, ReviewState::Reviewed)
            .expect("review");
    }
    workspace
}

#[test]
fn the_reviewed_policy_exports_reviewed_units_with_the_games_layout_and_guard() {
    let game = game(&sheet());
    let workspace = workspace(&game.source, true);
    let export = collect_project(
        &workspace,
        &game.source,
        ContentPolicy::Reviewed,
        &mut encoder(),
    )
    .expect("export");
    assert_eq!(export.report.exported, 1);
    assert_eq!(export.report.skipped_unreviewed, 1);
    let sheet = &export.sheets[0];
    assert_eq!(sheet.name, "Synthetic");
    let layout: Vec<_> = sheet
        .layout
        .iter()
        .map(|column| (column.column_index, column.offset))
        .collect();
    assert_eq!(layout, [(0, 0), (2, 8)], "every String column of the game");
    assert_eq!(sheet.cells.len(), 1);
    assert_eq!(sheet.cells[0].row_id, 7);
    assert_eq!(sheet.cells[0].text, "Два".as_bytes());
    assert_eq!(sheet.cells[0].source_guard, source_guard(b"Two"));
}

#[test]
fn a_collected_project_writes_a_pack_for_the_games_version() {
    let game = game(&sheet());
    let workspace = workspace(&game.source, false);
    let export = collect_project(&workspace, &game.source, ContentPolicy::All, &mut encoder())
        .expect("export");
    assert_eq!(export.report.exported, 2);
    let source = pack_source(&game.source);
    assert_eq!(source.language, "en");
    assert_eq!(source.game_version, "2026.09.15.0000.0000");
    let manifest = PackManifest {
        pack_id: "synthetic".to_owned(),
        title: "Synthetic".to_owned(),
        publisher: Publisher {
            name: "Tests".to_owned(),
            url: None,
        },
        license: None,
        sequence: 1,
        version: "1".to_owned(),
        channel: Channel::Testing,
        target_language: "ru".to_owned(),
        source,
        content_policy: ContentPolicy::All,
        project_commit: "0".repeat(40),
        exporter_aeria: "0.1.0".to_owned(),
        min_harmonia: "0.1.0".to_owned(),
    };
    let pack = write_pack(&manifest, export.sheets, None, None).expect("pack");
    assert_eq!(pack.counts.cells, 2);
    assert_eq!(pack.counts.reviewed_cells, 0);
}

#[test]
fn a_unit_that_no_longer_describes_the_game_fails_the_export() {
    let old = game(&sheet());
    let workspace = workspace(&old.source, true);
    let patched = game(
        &TextSheet::new(3, &[0, 2])
            .row(7, &[(0, "Two, revised"), (2, "Other")])
            .row(42, &[(0, "One")]),
    );
    let result = collect_project(
        &workspace,
        &patched.source,
        ContentPolicy::All,
        &mut encoder(),
    );
    assert!(matches!(result, Err(ExportError::Cell { row_id: 7, .. })));
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
    let workspace = workspace(&game.source, true);
    let mut rejecting = FakeEncoder {
        calls: 0,
        reject: Some("Два"),
    };
    let result = collect_project(&workspace, &game.source, ContentPolicy::All, &mut rejecting);
    assert!(matches!(result, Err(ExportError::Cell { row_id: 7, .. })));
    assert_eq!(rejecting.calls, 1);

    let result = collect_project(&workspace, &game.source, ContentPolicy::All, &mut Short);
    assert!(matches!(result, Err(ExportError::Encoder(_))));
}
