//! Glossary candidates over a synthetic project: a name translated two ways
//! is found, one translated one way is not, nor is a term of the glossary.

use std::sync::Arc;

use aeria_knowledge::{Knowledge, KnowledgeTexts};
use aeria_po::term_candidates;
use aeria_source::{GameSource, SourceLanguage};
use aeria_sqpack::testing::{FakeGame, TextSheet};

/// Rows of other text with the same words around the names, so that only
/// a name's own words are rare in the project, as in a real one.
const FILLER: u32 = 600;

/// A project of strings naming the Kojin two ways, Gerolt one way, and the
/// Maelstrom two ways, among filler.
fn project() -> (tempfile::TempDir, aeria_po::Session) {
    let directory = tempfile::tempdir().expect("directory");
    let game_dir = directory.path().join("game");
    let mut sheet = TextSheet::new(1, &[0]);
    for row in 1..=16 {
        sheet = sheet.row(row, &[(0, "We met the Kojin at the river.")]);
    }
    for row in 101..=116 {
        sheet = sheet.row(row, &[(0, "We spoke with Gerolt at the forge.")]);
    }
    for row in 201..=216 {
        sheet = sheet.row(row, &[(0, "We saw the Maelstrom fleet.")]);
    }
    for row in 1001..1001 + FILLER {
        sheet = sheet.row(row, &[(0, "We met a friend at the river.")]);
    }
    FakeGame::new("2026.01.01.0000.0000")
        .with_text("DefaultTalk", &sheet)
        .write(&game_dir)
        .expect("write");
    let source = Arc::new(GameSource::open(&game_dir, SourceLanguage::English).expect("game"));
    let session = aeria_po::session::create(&directory.path().join("project"), source, "ru", 2)
        .expect("create");
    for row in 1..=16 {
        let tribe = if row % 2 == 0 {
            "кодзинами"
        } else {
            "кудзинами"
        };
        session
            .set_translation(
                "DefaultTalk",
                row,
                0,
                0,
                &format!("Мы встретились с {tribe} у реки."),
            )
            .expect("save");
    }
    for row in 101..=116 {
        session
            .set_translation(
                "DefaultTalk",
                row,
                0,
                0,
                "Мы поговорили с Геролтом у кузницы.",
            )
            .expect("save");
    }
    for row in 201..=216 {
        let fleet = if row % 2 == 0 {
            "Водоворота"
        } else {
            "Мальстрёма"
        };
        session
            .set_translation(
                "DefaultTalk",
                row,
                0,
                0,
                &format!("Мы увидели флот {fleet}."),
            )
            .expect("save");
    }
    for row in 1001..1001 + FILLER {
        session
            .set_translation("DefaultTalk", row, 0, 0, "Мы встретились с другом у реки.")
            .expect("save");
    }
    (directory, session)
}

#[test]
fn names_translated_in_several_ways_are_candidates() {
    let (_directory, session) = project();
    let knowledge = Knowledge::from_texts(&KnowledgeTexts {
        style: None,
        terms: Some(
            "term,translation,note,forbidden,settled\nthe Maelstrom,Мальстрём,,,yes\n".to_owned(),
        ),
    });

    let found = term_candidates(session.root(), &knowledge).expect("candidates");

    let phrases: Vec<&str> = found
        .iter()
        .map(|candidate| candidate.phrase.as_str())
        .collect();
    assert_eq!(
        phrases,
        ["Kojin"],
        "Gerolt is translated one way, the Maelstrom is a term"
    );
    let kojin = &found[0];
    assert_eq!(kojin.translated, 16);
    let mut renderings: Vec<(String, usize)> = kojin
        .renderings
        .iter()
        .map(|rendering| (rendering.words.join(" "), rendering.strings))
        .collect();
    renderings.sort();
    assert_eq!(
        renderings,
        [("кодзинами".to_owned(), 8), ("кудзинами".to_owned(), 8)]
    );
    let example = &kojin.renderings[0].examples[0];
    assert!(example.source.contains("Kojin"), "{example:?}");
    assert!(example.translation.contains("дзинами"), "{example:?}");
    assert_eq!(kojin.sheets, [("DefaultTalk".to_owned(), 16)]);
}
