//! Building and querying the source index from a synthetic game.

use aeria_search::{SourceIndex, SourceQuery, Tokenizer};
use aeria_source::{GameSource, SourceLanguage};
use aeria_sqpack::testing::{FakeGame, TextSheet};

#[test]
fn the_index_holds_translatable_strings_only_and_is_keyed_by_the_game_data() {
    let game = tempfile::tempdir().expect("game");
    FakeGame::new("2026.09.15.0000.0000")
        .with_text(
            "Addon",
            &TextSheet::new(2, &[0, 1])
                .keyed(0)
                .row(1, &[(0, "KEY_HELLO"), (1, "Hello there")])
                .row(2, &[(0, "KEY_BYE"), (1, "Goodbye")]),
        )
        .write(game.path())
        .expect("write");
    let source = GameSource::open(game.path(), SourceLanguage::English).expect("source");
    let folder = tempfile::tempdir().expect("index folder");
    let path = folder.path().join("index.sqlite");
    let key = "en/2026.09.15.0000.0000";
    let index = SourceIndex::build(&path, key, Tokenizer::Words, &source, &|| true).expect("build");

    let query = |text| SourceQuery {
        text,
        sheet: None,
        offset: 0,
        limit: 10,
    };
    let hits = index.search(&query("hello")).expect("search").hits;
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].row, hits[0].column), (1, 1));
    assert_eq!(hits[0].source, "Hello there");
    assert!(
        index
            .search(&query("KEY_BYE"))
            .expect("search")
            .hits
            .is_empty(),
        "keys are not indexed"
    );

    assert!(SourceIndex::open(&path, key).expect("open").is_some());
    assert!(
        SourceIndex::open(&path, "en/2026.10.01.0000.0000")
            .expect("open")
            .is_none()
    );
    assert!(matches!(
        SourceIndex::build(
            folder.path().join("cancelled.sqlite"),
            key,
            Tokenizer::Words,
            &source,
            &|| false
        ),
        Err(aeria_search::SearchError::Cancelled)
    ));
    assert!(!folder.path().join("cancelled.sqlite").exists());
}
