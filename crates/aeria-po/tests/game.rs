//! Files made from a synthetic game, and a game update that carries their
//! translations to the next version.

use std::collections::BTreeMap;
use std::path::Path;

use aeria_po::{Languages, PoFile, SheetPaths, merge_files, sheet_files};
use aeria_source::{GameSource, SourceLanguage};
use aeria_sqpack::testing::{FakeGame, TextSheet};

const QUEST: &str = "quest/000/Test_00001";

fn quest(lines: &[(u32, &str, &str)]) -> TextSheet {
    lines.iter().fold(
        TextSheet::new(2, &[0, 1]).keyed(0),
        |sheet, (row, key, text)| sheet.row(*row, &[(0, key), (1, text)]),
    )
}

fn files(game: &Path) -> BTreeMap<String, PoFile> {
    let source = GameSource::open(game, SourceLanguage::English).expect("game");
    let paths = SheetPaths::new(source.sheet_names().iter().map(String::as_str));
    let languages = Languages {
        target: "ru".to_owned(),
    };
    let mut all = BTreeMap::new();
    for name in source.sheet_names() {
        for (path, file) in sheet_files(&source, &paths, &languages, name).expect("files") {
            all.insert(path, file);
        }
    }
    all
}

fn translate(files: &mut BTreeMap<String, PoFile>, context: &str, translation: &str) {
    let entry = files
        .values_mut()
        .flat_map(|file| file.entries.iter_mut())
        .find(|entry| entry.context == context)
        .unwrap_or_else(|| panic!("{context}"));
    translation.clone_into(&mut entry.translation);
}

fn entry<'a>(files: &'a BTreeMap<String, PoFile>, context: &str) -> &'a aeria_po::Entry {
    files
        .values()
        .flat_map(|file| &file.entries)
        .find(|entry| entry.context == context)
        .unwrap_or_else(|| panic!("{context}"))
}

#[test]
#[allow(clippy::too_many_lines)] // one scenario
fn files_follow_the_game_and_a_game_update_keeps_every_translation() {
    let first = tempfile::tempdir().expect("first");
    FakeGame::new("2026.01.01.0000.0000")
        .with_text(
            "Addon",
            &TextSheet::new(1, &[0])
                .row(1, &[(0, "OK")])
                .row(2, &[(0, "Cancel")])
                .row(1500, &[(0, "Far away")]),
        )
        .with_text(
            QUEST,
            &quest(&[
                (0, "TEXT_TEST_00001_SEQ_00", "Meet Alice."),
                (1, "TEXT_TEST_00001_ALICE_000_000", "Hello."),
                (2, "TEXT_TEST_00001_ALICE_000_001", "Goodbye."),
            ]),
        )
        .write(first.path())
        .expect("write");
    let mut made = files(first.path());
    assert_eq!(
        made.keys().map(String::as_str).collect::<Vec<_>>(),
        ["Addon/0.po", "Addon/1000.po", "quest/000/Test_00001.po"]
    );
    let hello = entry(
        &made,
        "quest/000/Test_00001:TEXT_TEST_00001_ALICE_000_000:1",
    );
    assert_eq!(hello.source, "Hello.");
    assert!(hello.extracted.contains(&"ja: Hello. [ja]".to_owned()));
    assert!(hello.extracted.contains(&"speaker: ALICE".to_owned()));
    let journal = entry(&made, "quest/000/Test_00001:TEXT_TEST_00001_SEQ_00:1");
    assert!(journal.extracted.contains(&"kind: journal".to_owned()));
    assert_eq!(
        made["Addon/0.po"].field("X-Game-Version"),
        Some("2026.01.01.0000.0000")
    );
    // Written files read back the same.
    for file in made.values() {
        let (read, problems) = PoFile::parse(&file.write());
        assert!(problems.is_empty());
        assert_eq!(read.write(), file.write());
    }

    translate(&mut made, "Addon:1:0:0", "ОК");
    translate(&mut made, "Addon:2:0:0", "Отмена");
    translate(&mut made, "Addon:1500:0:0", "Далеко");
    translate(
        &mut made,
        "quest/000/Test_00001:TEXT_TEST_00001_ALICE_000_000:1",
        "Привет.",
    );
    translate(
        &mut made,
        "quest/000/Test_00001:TEXT_TEST_00001_ALICE_000_001:1",
        "Пока.",
    );

    // The patch: a line is inserted into the quest, renumbering the rows
    // after it; a text changes; a row goes, and with it a file.
    let second = tempfile::tempdir().expect("second");
    FakeGame::new("2026.02.01.0000.0000")
        .with_text(
            "Addon",
            &TextSheet::new(1, &[0])
                .row(1, &[(0, "OK!")])
                .row(2, &[(0, "Cancel")]),
        )
        .with_text(
            QUEST,
            &quest(&[
                (0, "TEXT_TEST_00001_SEQ_00", "Meet Alice."),
                (1, "TEXT_TEST_00001_ALICE_000_005", "Welcome."),
                (2, "TEXT_TEST_00001_ALICE_000_000", "Hello."),
                (3, "TEXT_TEST_00001_ALICE_000_001", "Goodbye."),
            ]),
        )
        .write(second.path())
        .expect("write");
    let updated = merge_files(&made, files(second.path()), "2026.02.01.0000.0000");

    let hello = entry(
        &updated,
        "quest/000/Test_00001:TEXT_TEST_00001_ALICE_000_000:1",
    );
    assert_eq!(
        (hello.translation.as_str(), hello.fuzzy),
        ("Привет.", false)
    );
    assert_eq!(
        entry(
            &updated,
            "quest/000/Test_00001:TEXT_TEST_00001_ALICE_000_005:1"
        )
        .translation,
        ""
    );
    let ok = entry(&updated, "Addon:1:0:0");
    assert_eq!(ok.translation, "ОК");
    assert!(ok.fuzzy);
    assert_eq!(ok.previous.as_deref(), Some("OK"));
    assert!(!entry(&updated, "Addon:2:0:0").fuzzy);
    // Addon is one file again: the removed row's translation is kept there.
    assert!(!updated.contains_key("Addon/1000.po"));
    let addon = &updated["Addon.po"];
    assert_eq!(addon.obsolete.len(), 1);
    assert_eq!(addon.obsolete[0].translation, "Далеко");
    assert_eq!(addon.field("X-Game-Version"), Some("2026.02.01.0000.0000"));

    // Updating again changes nothing.
    assert_eq!(
        merge_files(&updated, files(second.path()), "2026.02.01.0000.0000"),
        updated
    );
}
