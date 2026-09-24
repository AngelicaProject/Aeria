use std::path::PathBuf;

use aeria_hsp::SourcePackage;
use aeria_search::{SearchError, SourceIndex, SourceQuery, Tokenizer, plain_text};

fn package(cache: &std::path::Path) -> SourcePackage {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../aeria-hsp/tests/fixtures/synthetic.hsp");
    SourcePackage::open(path, cache).expect("fixture package")
}

fn build(
    directory: &std::path::Path,
    package: &SourcePackage,
    tokenizer: Tokenizer,
) -> SourceIndex {
    SourceIndex::build(
        directory.join("index.sqlite3"),
        package.package_id(),
        tokenizer,
        package.source(),
        package.guidance_index(),
        &|| true,
    )
    .expect("index")
}

/// The first word of three or more characters in a string's plain text.
fn first_word(source: &str) -> Option<String> {
    plain_text(source)
        .split(|character: char| !character.is_alphanumeric())
        .find(|word| word.chars().count() >= 3)
        .map(str::to_owned)
}

#[test]
fn indexes_only_translatable_strings_and_finds_them_by_word() {
    let directory = tempfile::tempdir().expect("directory");
    let package = package(&directory.path().join("cache"));
    let index = build(directory.path(), &package, Tokenizer::Words);

    let mut found = 0;
    for sheet in package.source().sheets() {
        let mut after = None;
        loop {
            let page = package
                .source()
                .page_string_occurrence_records(&sheet.name, after.as_ref(), 4096)
                .expect("page");
            for record in &page.occurrences {
                let at = &record.fingerprint.coordinate;
                let translatable = package.guidance_index().is_translatable(
                    &at.sheet_name,
                    at.row_id,
                    at.subrow_id,
                    at.column_index,
                );
                let Some(word) = first_word(&record.macro_text) else {
                    continue;
                };
                let hits = index
                    .search(&SourceQuery {
                        text: &word.to_uppercase(),
                        sheet: Some(&at.sheet_name),
                        offset: 0,
                        limit: 200,
                    })
                    .expect("search")
                    .hits;
                let hit = hits.iter().any(|hit| {
                    (hit.sheet.as_str(), hit.row, hit.subrow, hit.column)
                        == (
                            at.sheet_name.as_str(),
                            at.row_id,
                            at.subrow_id,
                            at.column_index,
                        )
                });
                assert_eq!(hit, translatable, "{at:?} {word}");
                if hit {
                    found += 1;
                }
            }
            match page.next_after {
                Some(next) => after = Some(next),
                None => break,
            }
        }
    }
    assert!(
        found > 0,
        "the fixture needs a translatable string with a word"
    );

    let reopened = SourceIndex::open(index.path(), package.package_id()).expect("open");
    assert!(reopened.is_some());
    assert!(
        SourceIndex::open(index.path(), "another package")
            .expect("open")
            .is_none()
    );
    assert!(
        SourceIndex::open(
            directory.path().join("missing.sqlite3"),
            package.package_id()
        )
        .expect("open")
        .is_none()
    );
}

#[test]
fn similar_strings_rank_the_same_text_first_and_skip_the_excluded_one() {
    let directory = tempfile::tempdir().expect("directory");
    let package = package(&directory.path().join("cache"));
    for tokenizer in [Tokenizer::Words, Tokenizer::Trigram] {
        let target = directory.path().join(format!("{tokenizer:?}"));
        let index = build(&target, &package, tokenizer);
        let any = first_hit(&index, &package);
        let similar = index.similar(&any.source, None, 5).expect("similar");
        assert!(!similar.is_empty(), "{tokenizer:?}");
        assert!(
            (similar[0].score - 1.0).abs() < f64::EPSILON,
            "{tokenizer:?}"
        );
        let excluded = index
            .similar(
                &any.source,
                Some((&any.sheet, any.row, any.subrow, any.column)),
                5,
            )
            .expect("similar");
        assert!(excluded.iter().all(|similar| similar.hit != any));
    }
}

fn first_hit(index: &SourceIndex, package: &SourcePackage) -> aeria_search::SourceHit {
    for sheet in package.source().sheets() {
        let page = package
            .source()
            .page_string_occurrence_records(&sheet.name, None, 4096)
            .expect("page");
        for record in &page.occurrences {
            let Some(word) = first_word(&record.macro_text) else {
                continue;
            };
            if let Some(hit) = index
                .search(&SourceQuery {
                    text: &word,
                    sheet: None,
                    offset: 0,
                    limit: 1,
                })
                .expect("search")
                .hits
                .into_iter()
                .next()
            {
                return hit;
            }
        }
    }
    panic!("the fixture has no searchable string");
}

#[test]
fn a_cancelled_build_leaves_nothing_behind() {
    let directory = tempfile::tempdir().expect("directory");
    let package = package(&directory.path().join("cache"));
    let path = directory.path().join("index.sqlite3");
    let error = SourceIndex::build(
        &path,
        package.package_id(),
        Tokenizer::Words,
        package.source(),
        package.guidance_index(),
        &|| false,
    )
    .expect_err("cancelled");
    assert!(matches!(error, SearchError::Cancelled));
    assert!(!path.exists());
    assert!(!directory.path().join("index.sqlite3.partial").exists());
}

#[test]
fn tokenizers_follow_the_source_language() {
    assert_eq!(Tokenizer::for_language("ja"), Tokenizer::Trigram);
    assert_eq!(Tokenizer::for_language("zh-Hans"), Tokenizer::Trigram);
    assert_eq!(Tokenizer::for_language("en"), Tokenizer::Words);
    assert_eq!(Tokenizer::for_language("und"), Tokenizer::Words);
}

#[test]
fn short_or_wordless_queries_scan_substrings() {
    let directory = tempfile::tempdir().expect("directory");
    let package = package(&directory.path().join("cache"));
    let words = build(&directory.path().join("words"), &package, Tokenizer::Words);
    let trigram = build(
        &directory.path().join("trigram"),
        &package,
        Tokenizer::Trigram,
    );
    let any = first_hit(&words, &package);
    let plain = plain_text(&any.source);
    let pair: String = plain
        .chars()
        .filter(|character| character.is_alphanumeric())
        .take(2)
        .collect();
    assert_eq!(pair.chars().count(), 2);
    for index in [&words, &trigram] {
        let hits = index
            .search(&SourceQuery {
                text: &pair,
                sheet: Some(&any.sheet),
                offset: 0,
                limit: 200,
            })
            .expect("search")
            .hits;
        assert!(hits.iter().all(|hit| {
            plain_text(&hit.source)
                .to_lowercase()
                .contains(&pair.to_lowercase())
        }));
    }
    let short = trigram
        .search(&SourceQuery {
            text: &pair,
            sheet: Some(&any.sheet),
            offset: 0,
            limit: 200,
        })
        .expect("search");
    assert!(
        short.hits.contains(&any),
        "a two-character query scans substrings"
    );
    for wildcard in ["%", "_", "\\"] {
        let page = words
            .search(&SourceQuery {
                text: wildcard,
                sheet: None,
                offset: 0,
                limit: 200,
            })
            .expect("search");
        assert!(
            page.hits
                .iter()
                .all(|hit| plain_text(&hit.source).contains(wildcard)),
            "{wildcard}"
        );
    }
}
