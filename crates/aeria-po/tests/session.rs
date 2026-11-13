//! An open project: pages of rows with translations, saving and checking a
//! translation, notes, progress, and changes made outside the session.

use std::sync::Arc;

use aeria_po::{EditError, OpenError, Session};
use aeria_source::{GameSource, SourceLanguage};
use aeria_sqpack::testing::{FakeGame, TextSheet};

const QUEST: &str = "quest/000/Test_00001";

fn game(root: &std::path::Path, version: &str, hello: &str) -> Arc<GameSource> {
    FakeGame::new(version)
        .with_text(
            "Addon",
            &TextSheet::new(1, &[0])
                .row(1, &[(0, "OK")])
                .row(2, &[(0, "Cancel")])
                .row(1500, &[(0, "Far away")]),
        )
        .with_text(
            QUEST,
            &TextSheet::new(2, &[0, 1])
                .keyed(0)
                .row(0, &[(0, "TEXT_TEST_00001_SEQ_00"), (1, "Meet Alice.")])
                .row(1, &[(0, "TEXT_TEST_00001_ALICE_000_000"), (1, hello)]),
        )
        .write(root)
        .expect("write");
    Arc::new(GameSource::open(root, SourceLanguage::English).expect("game"))
}

#[test]
fn cached_glossary_saves_preserve_live_versions_and_refuse_hidden_external_changes() {
    use aeria_po::glossary_project::{self as glossary, EditedTerm};
    let directory = tempfile::tempdir().unwrap();
    let source = game(
        &directory.path().join("game"),
        "2026.01.01.0000.0000",
        "Hello.",
    );
    let root = directory.path().join("project");
    let session = aeria_po::session::create(&root, source, "ru", 2).unwrap();
    session.set_translation("Addon", 1, 0, 0, "ОК").unwrap();
    std::fs::create_dir_all(root.join("aeria-knowledge")).unwrap();
    std::fs::write(
        root.join("aeria-knowledge/terms.csv"),
        "term,translation,category\nCustom,Особый,понятие\n",
    )
    .unwrap();
    std::fs::write(
        root.join("aeria-knowledge/term-sheets.csv"),
        "sheet,column,category\nAddon,0,понятие\n",
    )
    .unwrap();
    let mut loaded = session.migrate_glossary().unwrap();
    let edits = |loaded: &glossary::Resolved| -> Vec<EditedTerm> {
        loaded
            .term_ids
            .iter()
            .zip(&loaded.knowledge.terms.entries)
            .map(|(id, e)| EditedTerm {
                id: Some(id.clone()),
                term: e.term.clone(),
                translation: e.translation.clone(),
                category: e.category.clone(),
                forms: e.forms.clone(),
                note: e.note.clone(),
                match_case: e.match_case,
            })
            .collect()
    };
    for translation in ["ДА", "ОК"] {
        let mut changes = edits(&loaded);
        changes
            .iter_mut()
            .find(|e| e.term == "OK")
            .unwrap()
            .translation = translation.to_owned();
        loaded = session
            .save_glossary(&glossary::token(&loaded), changes, None, None, &[])
            .unwrap();
        let reread = glossary::load(&root).unwrap();
        assert_eq!(glossary::token(&loaded), glossary::token(&reread));
        assert_eq!(
            loaded.knowledge.terms.entries,
            session.knowledge().terms.entries
        );
    }
    let path = root.join("po/Addon/0.po");
    let before = std::fs::metadata(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("msgstr \"ОК\"", "msgstr \"ДА\"")).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(before.modified().unwrap()))
        .unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), before.len());
    let catalog = root.join("aeria-knowledge/glossary.json");
    let original = std::fs::read(&catalog).unwrap();
    let mut changes = edits(&loaded);
    changes
        .iter_mut()
        .find(|e| e.term == "Custom")
        .unwrap()
        .note = Some("New note".to_owned());
    assert!(
        session
            .save_glossary(&glossary::token(&loaded), changes, None, None, &[])
            .is_err()
    );
    assert_eq!(std::fs::read(catalog).unwrap(), original);
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("msgstr \"ДА\"")
    );
    assert_eq!(
        session
            .glossary()
            .unwrap()
            .knowledge
            .terms
            .find("OK")
            .unwrap()
            .translation,
        "ДА"
    );
}

#[test]
#[allow(clippy::too_many_lines)] // one scenario
fn a_session_reads_writes_and_checks_strings() {
    let directory = tempfile::tempdir().expect("directory");
    let game_dir = directory.path().join("game");
    let root = directory.path().join("project");
    let source = game(&game_dir, "2026.01.01.0000.0000", "Hello.");
    let session = aeria_po::session::create(&root, Arc::clone(&source), "ru", 2).expect("create");
    assert_eq!(session.settings().game_version, "2026.01.01.0000.0000");

    let page = session.page("Addon", None, 256).expect("page");
    assert_eq!(page.rows.len(), 3);
    assert!(
        page.rows
            .iter()
            .all(|row| row.cells[0].translation.is_none())
    );

    let saved = session
        .set_translation("Addon", 1, 0, 0, "ОК")
        .expect("save")
        .expect("translation");
    assert_eq!(saved.text, "ОК");
    let far = session
        .set_translation("Addon", 1500, 0, 0, "Далеко")
        .expect("save");
    assert!(far.is_some());
    let text = std::fs::read_to_string(root.join("po/Addon/1000.po")).expect("file");
    assert!(text.contains("msgstr \"Далеко\""), "{text}");

    // Keyed rows are saved under their key.
    session
        .set_translation(QUEST, 1, 0, 1, "Привет.")
        .expect("save quest");
    let text = std::fs::read_to_string(root.join("po/quest/000/Test_00001.po")).expect("quest");
    assert!(
        text.contains("msgctxt \"quest/000/Test_00001:TEXT_TEST_00001_ALICE_000_000:1\"\nmsgid \"Hello.\"\nmsgstr \"Привет.\""),
        "{text}"
    );

    // A translation with a problem is refused and nothing is written.
    let before = std::fs::read(root.join("po/Addon/0.po")).expect("before");
    let refused = session.set_translation("Addon", 2, 0, 0, "Отмена\n");
    assert!(
        matches!(refused, Err(EditError::Invalid(ref problems)) if problems.len() == 1),
        "{refused:?}"
    );
    assert_eq!(
        std::fs::read(root.join("po/Addon/0.po")).expect("after"),
        before
    );
    assert!(matches!(
        session.set_translation("Addon", 9, 0, 0, "x"),
        Err(EditError::NotAnEntry(_))
    ));

    // A note keeps the translation; an empty text clears it.
    let noted = session
        .set_note("Addon", 1, 0, 0, Some("кнопка\nв окнах"))
        .expect("note")
        .expect("translation");
    assert_eq!(noted.note.as_deref(), Some("кнопка\nв окнах"));
    assert_eq!(noted.text, "ОК");
    let cleared = session
        .set_translation("Addon", 1, 0, 0, "")
        .expect("clear");
    assert_eq!(cleared.map(|t| t.text), Some(String::new()));

    let progress = session.progress().expect("progress");
    let addon = progress
        .iter()
        .find(|sheet| sheet.sheet == "Addon")
        .expect("addon");
    assert_eq!((addon.entries, addon.translated), (3, 1));

    // A change made outside the session is read on the next read.
    let path = root.join("po/Addon/0.po");
    let text = std::fs::read_to_string(&path).expect("read");
    std::fs::write(
        &path,
        text.replace(
            "msgctxt \"Addon:2:0:0\"\nmsgid \"Cancel\"\nmsgstr \"\"",
            "msgctxt \"Addon:2:0:0\"\nmsgid \"Cancel\"\nmsgstr \"Отмена\"",
        ),
    )
    .expect("edit");
    let cancel = session.translation("Addon", 2, 0, 0).expect("read");
    assert_eq!(cancel.map(|t| t.text).as_deref(), Some("Отмена"));

    // A newer game asks for an update; an older one is refused.
    drop(session);
    let newer = game(
        &directory.path().join("newer"),
        "2026.02.01.0000.0000",
        "Hi.",
    );
    assert!(matches!(
        Session::open(&root, newer),
        Err(OpenError::UpdateRequired { .. })
    ));
    let older = game(
        &directory.path().join("older"),
        "2025.12.01.0000.0000",
        "Hello.",
    );
    assert!(matches!(
        Session::open(&root, older),
        Err(OpenError::GameOutdated { .. })
    ));
    let session = Session::open(&root, source).expect("reopen");
    assert_eq!(
        session
            .translation(QUEST, 1, 0, 1)
            .expect("quest")
            .map(|t| t.text)
            .as_deref(),
        Some("Привет.")
    );
}

#[test]
#[allow(clippy::too_many_lines)] // one scenario
fn a_reviewed_translation_is_left_to_people() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path().join("project");
    let source = game(
        &directory.path().join("game"),
        "2026.01.01.0000.0000",
        "Hello.",
    );
    aeria_po::session::create(&root, Arc::clone(&source), "ru", 2).expect("create");
    // A project of the first format takes the second with its first review.
    let settings = std::fs::read_to_string(root.join("aeria.json")).expect("settings");
    std::fs::write(
        root.join("aeria.json"),
        settings.replace(aeria_po::FORMAT, "aeria-po/1"),
    )
    .expect("old format");
    let session = Session::open(&root, Arc::clone(&source)).expect("open");
    assert_eq!(session.settings().format, "aeria-po/1");

    session
        .set_translation("Addon", 1, 0, 0, "ОК")
        .expect("save");
    assert_eq!(
        session.settings().format,
        "aeria-po/1",
        "a save marks nothing"
    );
    let marked = session
        .set_reviewed("Addon", 1, 0, 0, true)
        .expect("review")
        .expect("translation");
    assert!(marked.reviewed && !marked.review_stale);
    assert_eq!(session.settings().format, aeria_po::FORMAT);
    let written = std::fs::read_to_string(root.join("aeria.json")).expect("settings");
    assert!(written.contains(aeria_po::FORMAT), "{written}");

    // A person's edit keeps the review; machine translation leaves it.
    let edited = session
        .set_translation("Addon", 1, 0, 0, "Ок")
        .expect("edit")
        .expect("translation");
    assert!(edited.reviewed);
    let filled = session
        .fill(
            "Addon/0.po",
            &[("Addon:1:0:0".to_owned(), "Хорошо".to_owned())],
            true,
            &aeria_knowledge::Glossary::default(),
        )
        .expect("fill");
    assert!(filled.is_empty());

    // Translating again skips it, with the reason.
    let clear = |context: &str, text: &str| aeria_po::EntryEdit {
        path: "Addon/0.po".to_owned(),
        context: context.to_owned(),
        expected_text: text.to_owned(),
        expected_fuzzy: false,
        kind: aeria_po::EditKind::Clear,
    };
    let applied = session
        .apply_edits(&[clear("Addon:1:0:0", "Ок")])
        .expect("apply");
    assert!(applied.done.is_empty());
    assert_eq!(applied.skipped[0].reason, aeria_po::SkipReason::Reviewed);

    // A review removed gives the string back to machine translation.
    let unmarked = session
        .set_reviewed("Addon", 1, 0, 0, false)
        .expect("unreview")
        .expect("translation");
    assert!(!unmarked.reviewed);
    assert_eq!(
        session
            .apply_edits(&[clear("Addon:1:0:0", "Ок")])
            .expect("apply")
            .done
            .len(),
        1
    );

    // Saving and marking at once; a mark of another text holds no longer.
    let both = session
        .set_translation_reviewed("Addon", 2, 0, 0, "Отмена")
        .expect("save and review")
        .expect("translation");
    assert!(both.reviewed);
    let path = root.join("po/Addon/0.po");
    let text = std::fs::read_to_string(&path).expect("file");
    std::fs::write(
        &path,
        text.replace("msgstr \"Отмена\"", "msgstr \"Отменить\""),
    )
    .expect("hand edit");
    let stale = session
        .translation("Addon", 2, 0, 0)
        .expect("read")
        .expect("translation");
    assert!(!stale.reviewed && stale.review_stale);
    let filled = session
        .fill(
            "Addon/0.po",
            &[("Addon:2:0:0".to_owned(), "Отказ".to_owned())],
            false,
            &aeria_knowledge::Glossary::default(),
        )
        .expect("fill");
    assert!(filled.is_empty(), "a translated string is not filled");
    session
        .set_translation("Addon", 2, 0, 0, "")
        .expect("clear");
    let cleared = session.translation("Addon", 2, 0, 0).expect("read");
    assert!(
        cleared.is_none(),
        "an empty translation keeps no mark: {cleared:?}"
    );
}
