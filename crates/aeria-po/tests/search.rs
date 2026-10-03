//! Searching the project (macros are covered by the unit tests of
//! `aeria_po::search`), replacing in translations with a preview, applying
//! edits only to entries that did not change, undoing them, and clearing
//! translations to translate again.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use aeria_knowledge::Knowledge;
use aeria_po::{
    CheckFilter, EditKind, EntryEdit, Field, Fields, Issue, MatchKind, Pattern, Query, Replacement,
    SkipReason, State, preview_replace, search, search_all,
};
use aeria_source::{GameSource, SourceLanguage};
use aeria_sqpack::testing::{FakeGame, TextSheet};

fn project() -> (tempfile::TempDir, aeria_po::Session) {
    let directory = tempfile::tempdir().expect("directory");
    let game_dir = directory.path().join("game");
    FakeGame::new("2026.01.01.0000.0000")
        .with_text(
            "Addon",
            &TextSheet::new(1, &[0])
                .row(1, &[(0, "The cook arrives")])
                .row(2, &[(0, "Ask the cook")])
                .row(3, &[(0, "Cooking")]),
        )
        .with_text(
            "Item",
            &TextSheet::new(1, &[0]).row(1, &[(0, "cook's knife")]),
        )
        .write(&game_dir)
        .expect("write");
    let source = Arc::new(GameSource::open(&game_dir, SourceLanguage::English).expect("game"));
    let root = directory.path().join("project");
    let session = aeria_po::session::create(&root, source, "ru", 2).expect("create");
    session
        .set_translation("Addon", 1, 0, 0, "Повар пришёл")
        .expect("save");
    session
        .set_translation("Addon", 2, 0, 0, "Спроси повара")
        .expect("save");
    session
        .set_translation("Item", 1, 0, 0, "Поварской нож")
        .expect("save");
    (directory, session)
}

fn query(text: &str, kind: MatchKind) -> Query {
    Query {
        pattern: Some(Pattern {
            text: text.to_owned(),
            kind,
            case_sensitive: false,
        }),
        ..Query::default()
    }
}

fn find(session: &aeria_po::Session, query: &Query) -> aeria_po::Found {
    search(
        session.root(),
        query,
        &Knowledge::default(),
        "ru",
        &AtomicBool::new(false),
    )
    .expect("search")
}

#[test]
fn search_matches_text_in_translations_and_sources_but_never_macros() {
    let (_directory, session) = project();

    let found = find(&session, &query("повар", MatchKind::Text));
    assert_eq!(found.total, 3, "{found:?}");
    let files: Vec<(&str, &str, usize)> = found
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.sheet.as_str(), file.count))
        .collect();
    assert_eq!(files, [("Addon.po", "Addon", 2), ("Item.po", "Item", 1)]);
    let first = &found.hits[0];
    assert_eq!(first.context, "Addon:1:0:0");
    assert_eq!(first.matches[0].field, Field::Translation);
    assert_eq!(first.matches[0].ranges, vec![0.."Повар".len()]);

    // Whole words: «Поварской» is not «повар».
    assert_eq!(find(&session, &query("повар", MatchKind::Word)).total, 1);
    // The source matches too, and can be left out.
    let mut cook = query("cook", MatchKind::Word);
    assert_eq!(find(&session, &cook).total, 3);
    cook.fields = Fields {
        translation: true,
        source: false,
        note: false,
        context: false,
    };
    assert_eq!(find(&session, &cook).total, 0);
    // Paths and states narrow the search.
    let mut untranslated = Query {
        states: vec![State::Untranslated],
        ..Query::default()
    };
    assert_eq!(find(&session, &untranslated).total, 1);
    untranslated.paths = vec!["Item".to_owned()];
    assert_eq!(find(&session, &untranslated).total, 0);
    // A regular expression finds inflected forms: «Повар», «повара», «Поварской».
    assert_eq!(
        find(&session, &query(r"\bповар\w*", MatchKind::Regex)).total,
        3
    );
    assert_eq!(
        find(&session, &query(r"\bповар(а|ом)?\b", MatchKind::Regex)).total,
        2
    );
}

#[test]
fn problems_are_found_by_the_checks_a_translation_is_saved_with() {
    let (_directory, session) = project();
    let terms = "term,translation,forbidden\ncook,кулинар,повар\n";
    std::fs::create_dir_all(session.root().join("aeria-knowledge")).expect("dir");
    std::fs::write(session.root().join("aeria-knowledge/terms.csv"), terms).expect("terms");
    let knowledge = session.knowledge();
    let problems = search(
        session.root(),
        &Query {
            check: CheckFilter::Problems,
            ..Query::default()
        },
        &knowledge,
        "ru",
        &AtomicBool::new(false),
    )
    .expect("search");
    // «Повар» and its inflected form «повара»; «Поварской» is another word.
    assert_eq!(problems.total, 2);
    assert_eq!(
        problems.hits[0].findings[0],
        Issue::ForbiddenTerm {
            term: "cook".to_owned(),
            translation: "кулинар".to_owned(),
            variant: "повар".to_owned(),
        }
    );
    assert_eq!(problems.issues.len(), 1);
    assert_eq!(problems.issues[0].group, "forbiddenTerm:cook");
    assert_eq!(problems.issues[0].count, 2);
    let unused = search(
        session.root(),
        &Query {
            check: CheckFilter::Advice,
            ..Query::default()
        },
        &knowledge,
        "ru",
        &AtomicBool::new(false),
    )
    .expect("search");
    assert_eq!(unused.total, 3);
    assert_eq!(unused.issues[0].group, "termNotUsed:cook");
    assert_eq!(unused.issues[0].count, 3);
    // One group of the summary narrows the search to it.
    let narrowed = search(
        session.root(),
        &Query {
            check: CheckFilter::Advice,
            issue: Some("termNotUsed:nothing".to_owned()),
            ..Query::default()
        },
        &knowledge,
        "ru",
        &AtomicBool::new(false),
    )
    .expect("search");
    assert_eq!(narrowed.total, 0);
}

#[test]
fn a_replacement_is_previewed_applied_to_unchanged_entries_and_undone() {
    let (_directory, session) = project();
    let replacement = Replacement {
        text: "кулинар".to_owned(),
        preserve_case: true,
    };
    let changes = preview_replace(
        session.root(),
        &query("повар", MatchKind::Word),
        &replacement,
        &Knowledge::default(),
        "ru",
        &AtomicBool::new(false),
    )
    .expect("preview");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].after, "Кулинар пришёл");
    assert!(changes[0].problems.is_empty());

    let mut edits: Vec<EntryEdit> = changes
        .iter()
        .map(|change| EntryEdit {
            path: change.path.clone(),
            context: change.context.clone(),
            expected_text: change.before.clone(),
            expected_fuzzy: change.fuzzy,
            kind: EditKind::Replace(change.after.clone()),
        })
        .collect();
    // An entry changed since the preview is skipped.
    edits.push(EntryEdit {
        path: "Addon.po".to_owned(),
        context: "Addon:2:0:0".to_owned(),
        expected_text: "something else".to_owned(),
        expected_fuzzy: false,
        kind: EditKind::Replace("Спроси кулинара".to_owned()),
    });
    // A replacement that breaks the string is not written.
    edits.push(EntryEdit {
        path: "Item.po".to_owned(),
        context: "Item:1:0:0".to_owned(),
        expected_text: "Поварской нож".to_owned(),
        expected_fuzzy: false,
        kind: EditKind::Replace("Нож(а)".to_owned()),
    });
    let applied = session.apply_edits(&edits).expect("apply");
    assert_eq!(applied.done.len(), 1);
    assert_eq!(applied.skipped.len(), 2);
    assert_eq!(applied.skipped[0].reason, SkipReason::Changed);
    assert!(matches!(applied.skipped[1].reason, SkipReason::Invalid(_)));
    let translation = session
        .translation("Addon", 1, 0, 0)
        .expect("read")
        .expect("translated");
    assert_eq!(translation.text, "Кулинар пришёл");

    // Undo restores the entries that did not change since.
    let undo: Vec<EntryEdit> = applied.done.iter().map(aeria_po::EditDone::undo).collect();
    let undone = session.apply_edits(&undo).expect("undo");
    assert_eq!(undone.done.len(), 1);
    let translation = session
        .translation("Addon", 1, 0, 0)
        .expect("read")
        .expect("translated");
    assert_eq!(translation.text, "Повар пришёл");
    let again = session.apply_edits(&undo).expect("undo twice");
    assert_eq!(again.skipped[0].reason, SkipReason::Changed);
}

#[test]
fn clearing_leaves_strings_untranslated_for_machine_translation() {
    let (_directory, session) = project();
    let path = session.root().join("po/Addon.po");
    // A fuzzy entry: the clear removes its mark and previous source too.
    let text = std::fs::read_to_string(&path).expect("file");
    let text = text.replace(
        "msgctxt \"Addon:2:0:0\"",
        "#, fuzzy\n#| msgid \"Ask the cooks\"\nmsgctxt \"Addon:2:0:0\"",
    );
    std::fs::write(&path, text).expect("write");
    let applied = session
        .apply_edits(&[EntryEdit {
            path: "Addon.po".to_owned(),
            context: "Addon:2:0:0".to_owned(),
            expected_text: "Спроси повара".to_owned(),
            expected_fuzzy: true,
            kind: EditKind::Clear,
        }])
        .expect("clear");
    assert_eq!(applied.done.len(), 1);
    assert!(applied.done[0].before.fuzzy);
    let text = std::fs::read_to_string(&path).expect("file");
    assert!(!text.contains("#, fuzzy"), "{text}");
    assert!(!text.contains("Ask the cooks"), "{text}");
    assert_eq!(session.translation("Addon", 2, 0, 0).expect("read"), None);
}

#[test]
fn a_term_exception_lifts_the_term_and_undo_takes_it_back() {
    let (_directory, session) = project();
    let terms = "term,translation,forbidden\ncook,кулинар,повар\n";
    std::fs::create_dir_all(session.root().join("aeria-knowledge")).expect("dir");
    std::fs::write(session.root().join("aeria-knowledge/terms.csv"), terms).expect("terms");
    let problems = |session: &aeria_po::Session| {
        session
            .findings("Addon", 1, 0, 0)
            .expect("findings")
            .0
            .iter()
            .filter(|issue| issue.is_problem())
            .count()
    };
    assert_eq!(problems(&session), 1);

    let edit = EntryEdit {
        path: "Addon.po".to_owned(),
        context: "Addon:1:0:0".to_owned(),
        expected_text: "Повар пришёл".to_owned(),
        expected_fuzzy: false,
        kind: EditKind::TermException {
            term: "cook".to_owned(),
            add: true,
        },
    };
    let revision = session.revision();
    let applied = session.apply_edits(&[edit]).expect("apply");
    assert_eq!(applied.done.len(), 1);
    assert!(
        session.revision() > revision,
        "a write is a change of the files"
    );
    let text = std::fs::read_to_string(session.root().join("po/Addon.po")).expect("file");
    assert!(text.contains("#, aeria-term-exception: cook\nmsgctxt \"Addon:1:0:0\""));
    assert_eq!(problems(&session), 0);
    assert_eq!(
        session.findings("Addon", 1, 0, 0).expect("findings").1,
        ["cook"]
    );
    // The exception is the string's alone.
    assert_eq!(
        session
            .findings("Addon", 2, 0, 0)
            .expect("findings")
            .0
            .iter()
            .filter(|issue| issue.is_problem())
            .count(),
        1
    );

    let undo: Vec<EntryEdit> = applied.done.iter().map(aeria_po::EditDone::undo).collect();
    assert_eq!(session.apply_edits(&undo).expect("undo").done.len(), 1);
    assert_eq!(problems(&session), 1);
}

#[test]
fn search_all_returns_every_entry_past_the_hit_limit() {
    let directory = tempfile::tempdir().expect("directory");
    let game_dir = directory.path().join("game");
    let rows = u32::try_from(aeria_po::MAX_HITS).expect("rows") + 50;
    let mut sheet = TextSheet::new(1, &[0]);
    for row in 1..=rows {
        sheet = sheet.row(row, &[(0, "The cook arrives")]);
    }
    FakeGame::new("2026.01.01.0000.0000")
        .with_text("LogMessage", &sheet)
        .write(&game_dir)
        .expect("write");
    let source = Arc::new(GameSource::open(&game_dir, SourceLanguage::English).expect("game"));
    let session = aeria_po::session::create(&directory.path().join("project"), source, "ru", 2)
        .expect("create");
    let cook = query("cook", MatchKind::Text);

    let found = find(&session, &cook);
    assert_eq!(found.total, rows as usize);
    assert_eq!(found.hits.len(), aeria_po::MAX_HITS);

    let all = search_all(session.root(), &cook, &Knowledge::default(), "ru").expect("search");
    assert_eq!(all.len(), rows as usize);
    assert_eq!(
        all[..found.hits.len()],
        found.hits[..],
        "the same order as search"
    );
    let contexts: std::collections::HashSet<_> = all.iter().map(|hit| &hit.context).collect();
    assert_eq!(contexts.len(), all.len(), "every entry once");
}
