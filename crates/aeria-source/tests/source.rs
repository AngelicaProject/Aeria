//! `GameSource` over synthetic installations.

use aeria_source::{
    DialogueKind, GameSource, LineRole, RowAddress, SheetLookup, SourceLanguage, SourceSheet,
};
use aeria_sqpack::excel::{ColumnKind, Language};
use aeria_sqpack::testing::{FakeGame, FakeRow, FakeSheet, FakeValue};

const VERSION: &str = "2026.09.15.0000.0000";
const LANGUAGES: [Language; 4] = [
    Language::Japanese,
    Language::English,
    Language::German,
    Language::French,
];

/// A sheet with a key column, a name column, and a number column, whose
/// names differ by language except in row 3 and whose row 4 name is empty.
fn dialogue() -> FakeSheet {
    let mut sheet = FakeSheet::new(vec![
        ColumnKind::String,
        ColumnKind::String,
        ColumnKind::UInt32,
    ]);
    for language in LANGUAGES {
        let suffix = language.suffix();
        let rows = vec![
            FakeRow::new(
                1,
                vec![
                    "KEY_1".into(),
                    format!("Hello {suffix}").as_str().into(),
                    1.into(),
                ],
            ),
            FakeRow::new(
                2,
                vec![
                    "KEY_2".into(),
                    format!("Bye {suffix}").as_str().into(),
                    2.into(),
                ],
            ),
            FakeRow::new(3, vec!["KEY_3".into(), "...".into(), 3.into()]),
            FakeRow::new(4, vec!["KEY_4".into(), "".into(), 4.into()]),
        ];
        sheet.rows.insert(language, rows);
    }
    sheet
}

fn present(source: &GameSource, name: &str) -> std::sync::Arc<SourceSheet> {
    match source.sheet(name).expect("readable game") {
        SheetLookup::Present(sheet) => sheet,
        other => panic!("{name} is {other:?}"),
    }
}

fn translatable(sheet: &SourceSheet) -> Vec<(u32, u32)> {
    sheet
        .rows()
        .iter()
        .flat_map(|row| {
            sheet
                .cells(row)
                .filter(|cell| cell.translatable)
                .map(|cell| (row.row_id, cell.column))
        })
        .collect()
}

#[test]
fn a_source_reads_its_version_sheets_and_cells() {
    let folder = tempfile::tempdir().expect("folder");
    FakeGame::new(VERSION)
        .with_sheet("Quest", dialogue())
        .with_sheet("Addon", dialogue())
        .write(folder.path())
        .expect("game");
    let source = GameSource::open(folder.path(), SourceLanguage::German).expect("source");
    assert_eq!(source.version().as_str(), VERSION);
    assert_eq!(source.sheet_names(), ["Addon", "Quest"]);
    assert!(matches!(source.sheet("Nope"), Ok(SheetLookup::Missing)));

    let sheet = present(&source, "Quest");
    assert_eq!(
        sheet
            .columns()
            .iter()
            .map(|column| (column.index, column.offset))
            .collect::<Vec<_>>(),
        [(0, 0), (1, 4)]
    );
    assert_eq!(sheet.cell(2, 0, 1).expect("cell").text(), "Bye de");
    assert!(
        sheet.cell(2, 0, 2).is_none(),
        "a number column has no String cell"
    );
    assert!(sheet.cell(9, 0, 1).is_none());
    assert_eq!(
        sheet
            .rows_after(Some((2, 0)))
            .iter()
            .map(|row| row.row_id)
            .collect::<Vec<_>>(),
        [3, 4]
    );
    assert!(
        std::sync::Arc::ptr_eq(&sheet, &present(&source, "Quest")),
        "sheets are cached"
    );
}

#[test]
fn permission_needs_non_empty_text_that_differs_by_language() {
    let folder = tempfile::tempdir().expect("folder");
    let mut missing_row = dialogue();
    missing_row
        .rows
        .get_mut(&Language::French)
        .expect("French rows")
        .pop();
    let mut neutral = FakeSheet::new(vec![ColumnKind::String]);
    neutral.rows.insert(
        Language::None,
        vec![
            FakeRow::new(1, vec!["A".into()]),
            FakeRow::new(2, vec!["B".into()]),
        ],
    );
    FakeGame::new(VERSION)
        .with_sheet("Quest", dialogue())
        .with_sheet("Uneven", missing_row)
        .with_sheet("Neutral", neutral)
        .write(folder.path())
        .expect("game");
    let source = GameSource::open(folder.path(), SourceLanguage::English).expect("source");

    let quest = present(&source, "Quest");
    assert_eq!(translatable(&quest), [(1, 1), (2, 1)]);
    let keys = quest.row_keys().expect("keyed");
    assert_eq!(keys.column(), 0);
    assert_eq!(keys.key_of(3, 0), Some("KEY_3"));
    assert_eq!(keys.row_of("KEY_2"), Some((2, 0)));

    assert!(
        translatable(&present(&source, "Uneven")).is_empty(),
        "rows differ by language"
    );
    let neutral = present(&source, "Neutral");
    assert!(
        translatable(&neutral).is_empty(),
        "neutral data is the same in every language"
    );
    assert_eq!(neutral.cell(2, 0, 0).expect("cell").text(), "B");
}

#[test]
fn unreadable_sheets_are_unavailable_and_a_patch_shows_in_the_version() {
    let folder = tempfile::tempdir().expect("folder");
    let japanese_only = FakeSheet::new(vec![ColumnKind::String]).with_rows(
        Language::Japanese,
        vec![FakeRow::new(1, vec![FakeValue::from("日本")])],
    );
    let game = FakeGame::new(VERSION).with_sheet("Japanese", japanese_only);
    game.write(folder.path()).expect("game");
    let source = GameSource::open(folder.path(), SourceLanguage::English).expect("source");
    assert!(matches!(
        source.sheet("Japanese"),
        Ok(SheetLookup::Unavailable(_))
    ));

    let mut patched = game;
    patched.version = "2026.10.01.0000.0000".to_owned();
    patched.write(folder.path()).expect("patch");
    assert_eq!(source.version().as_str(), VERSION);
    assert_eq!(
        source.current_version().expect("version").as_str(),
        "2026.10.01.0000.0000"
    );
}

#[test]
fn a_cell_reads_in_the_other_client_languages() {
    let folder = tempfile::tempdir().expect("folder");
    let english_and_german = FakeSheet::new(vec![ColumnKind::String])
        .with_rows(
            Language::English,
            vec![
                FakeRow::new(1, vec![FakeValue::from("Hello")]),
                FakeRow::new(2, vec![FakeValue::from("Bye")]),
            ],
        )
        .with_rows(
            Language::German,
            vec![FakeRow::new(1, vec![FakeValue::from("Hallo")])],
        );
    FakeGame::new(VERSION)
        .with_sheet("Quest", dialogue())
        .with_sheet("Partial", english_and_german)
        .write(folder.path())
        .expect("game");
    let source = GameSource::open(folder.path(), SourceLanguage::English).expect("source");

    let texts = |sheet: &str, row: u32, column: u32| {
        source
            .cell_in_other_languages(sheet, row, 0, column)
            .expect("readable game")
            .into_iter()
            .map(|(language, text)| (language.code(), text))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        texts("Quest", 1, 1),
        [
            ("ja", Some("Hello ja".to_owned())),
            ("de", Some("Hello de".to_owned())),
            ("fr", Some("Hello fr".to_owned())),
        ]
    );
    assert_eq!(
        texts("Partial", 1, 0),
        [("ja", None), ("de", Some("Hallo".to_owned())), ("fr", None)],
        "a language the sheet lacks has no text"
    );
    assert_eq!(
        texts("Partial", 2, 0),
        [("ja", None), ("de", None), ("fr", None)],
        "a row one language lacks has no text there"
    );
    assert_eq!(
        texts("Missing", 1, 0),
        [("ja", None), ("de", None), ("fr", None)]
    );
    assert!(texts("Quest", 1, 2).iter().all(|(_, text)| text.is_none()));
}

#[test]
fn a_folder_without_the_game_is_rejected() {
    let folder = tempfile::tempdir().expect("folder");
    assert!(GameSource::open(folder.path(), SourceLanguage::English).is_err());
}

/// A keyed two-column sheet whose texts differ by language, except that a
/// text of `-` is empty in every language.
fn keyed(rows: &[(u32, &str, &str)]) -> FakeSheet {
    let mut sheet = FakeSheet::new(vec![ColumnKind::String, ColumnKind::String]);
    for language in LANGUAGES {
        let suffix = language.suffix();
        let rows = rows
            .iter()
            .map(|(row, key, text)| {
                let text = if *text == "-" {
                    String::new()
                } else {
                    format!("{text} {suffix}")
                };
                FakeRow::new(*row, vec![(*key).into(), text.as_str().into()])
            })
            .collect();
        sheet.rows.insert(language, rows);
    }
    sheet
}

/// A game with a quest sheet and another version of it with the same name,
/// a quest with another name, a quest whose ID two `Quest` rows hold, a
/// cutscene sheet, and `Quest`.
fn dialogue_game(folder: &std::path::Path) -> GameSource {
    let quest = keyed(&[
        (0, "TEXT_MANFST004_00124_SEQ_00", "Miounne has tasks."),
        (1, "TEXT_MANFST004_00124_SEQ_01", "-"),
        (2, "TEXT_MANFST004_00124_TODO_00", "Visit the guild."),
        (3, "TEXT_MANFST004_00124_MIOUNNE_000_1", "Let us begin."),
        (4, "TEXT_MANFST004_00124_QIB_BATTLETALK_05", "Hah!"),
        (5, "TEXT_MANFST004_00124_URIANGER_000_2", "Well met."),
    ]);
    let cutscene = keyed(&[
        (0, "TEXT_VOICEMAN_02400_000010_URIANGER", "Thou art come."),
        (1, "TEXT_VOICEMAN_02400_000020_ILBERD", "Hail."),
    ]);
    let mut names = FakeSheet::new(vec![ColumnKind::String, ColumnKind::String]);
    for language in LANGUAGES {
        let suffix = language.suffix();
        names.rows.insert(
            language,
            vec![
                FakeRow::new(
                    7,
                    vec![
                        format!("Close to Home {suffix}").as_str().into(),
                        "ManFst004_00124".into(),
                    ],
                ),
                FakeRow::new(
                    8,
                    vec![
                        format!("Twice {suffix}").as_str().into(),
                        "Twice_00001".into(),
                    ],
                ),
                FakeRow::new(
                    9,
                    vec![
                        format!("Again {suffix}").as_str().into(),
                        "Twice_00001".into(),
                    ],
                ),
                FakeRow::new(
                    10,
                    vec![
                        format!("Close to Home {suffix}").as_str().into(),
                        "ManFst998_00125".into(),
                    ],
                ),
                FakeRow::new(
                    11,
                    vec![
                        format!("Far from Home {suffix}").as_str().into(),
                        "ManFst999_00126".into(),
                    ],
                ),
            ],
        );
    }
    FakeGame::new(VERSION)
        .with_sheet("quest/001/ManFst004_00124", quest)
        .with_sheet(
            "quest/000/Twice_00001",
            keyed(&[(0, "K0", "a"), (1, "K1", "b")]),
        )
        .with_sheet(
            "quest/001/ManFst998_00125",
            keyed(&[(0, "K0", "a"), (1, "K1", "b")]),
        )
        .with_sheet(
            "quest/001/ManFst999_00126",
            keyed(&[(0, "K0", "a"), (1, "K1", "b")]),
        )
        .with_sheet("cut_scene/024/VoiceMan_02400", cutscene)
        .with_sheet("Quest", names)
        .write(folder)
        .expect("game");
    GameSource::open(folder, SourceLanguage::English).expect("source")
}

#[test]
fn quest_keys_describe_journal_objectives_and_speech() {
    let folder = tempfile::tempdir().expect("folder");
    let source = dialogue_game(folder.path());
    let dialogue = source
        .dialogue("quest/001/ManFst004_00124")
        .expect("readable")
        .expect("dialogue");
    assert_eq!(dialogue.kind, DialogueKind::Quest);
    let roles: Vec<(u32, LineRole)> = dialogue
        .lines
        .iter()
        .map(|line| (line.row_id, line.role.clone()))
        .collect();
    let speech = |speaker: &str| LineRole::Speech {
        speaker: speaker.to_owned(),
    };
    assert_eq!(
        roles,
        [
            (0, LineRole::Journal),
            (2, LineRole::Objective),
            (3, speech("MIOUNNE")),
            (4, LineRole::Other),
            (5, speech("URIANGER")),
        ],
        "a row without text is left out"
    );
    let line = &dialogue.lines[2];
    assert_eq!((line.column, line.text.as_str()), (1, "Let us begin. en"));
    assert!(source.dialogue("Quest").expect("readable").is_none());
}

#[test]
fn quests_and_speakers_are_found_across_sheets() {
    let folder = tempfile::tempdir().expect("folder");
    let source = dialogue_game(folder.path());
    assert_eq!(
        source
            .quest_row("quest/001/ManFst004_00124")
            .expect("readable"),
        Some((7, 0))
    );
    assert_eq!(
        source.quest_row("quest/000/Twice_00001").expect("readable"),
        None,
        "an ID held by two rows names no quest"
    );
    assert_eq!(
        source
            .quest_row("cut_scene/024/VoiceMan_02400")
            .expect("readable"),
        None
    );

    assert_eq!(
        source.speakers("ri", 2).expect("readable"),
        [("URIANGER".to_owned(), 2)]
    );
    let (total, lines) = source.speaker_lines("URIANGER", 1, 5, 2).expect("readable");
    assert_eq!(total, 2);
    assert_eq!(
        lines,
        [RowAddress {
            sheet: "quest/001/ManFst004_00124".to_owned(),
            row_id: 5,
            subrow_id: 0,
        }],
        "lines follow sheet-name order, so the cutscene comes first"
    );
    assert_eq!(
        source.speaker_lines("NOBODY", 0, 5, 2).expect("readable").0,
        0
    );
}

#[test]
fn quests_with_the_same_name_are_versions_of_each_other() {
    let folder = tempfile::tempdir().expect("folder");
    let source = dialogue_game(folder.path());
    assert_eq!(
        source
            .quest_versions("quest/001/ManFst004_00124")
            .expect("readable"),
        ["quest/001/ManFst998_00125"]
    );
    assert_eq!(
        source
            .quest_versions("quest/001/ManFst998_00125")
            .expect("readable"),
        ["quest/001/ManFst004_00124"]
    );
    assert!(
        source
            .quest_versions("quest/001/ManFst999_00126")
            .expect("readable")
            .is_empty()
    );
    assert!(
        source
            .quest_versions("quest/000/Twice_00001")
            .expect("readable")
            .is_empty(),
        "a quest without one row has no name"
    );
    assert!(
        source
            .quest_versions("cut_scene/024/VoiceMan_02400")
            .expect("readable")
            .is_empty()
    );
}
