//! Consecutive game patches applied to one project. A translation detached by
//! one patch must survive every later patch unchanged and bind again as soon
//! as its line returns, and a chain of updates must end in the same state as
//! updating directly to the last version.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use aeria_core::{DetachReason, ReviewState, SourceBinding, SourceStatus, TranslationUnitId};
use aeria_hsp::SourcePackage;
use aeria_workspace::ProjectSession;
use tempfile::TempDir;

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{ProjectionFixture, write_projection_fixture_with};

type Rows<'a> = &'a [(u32, [&'a str; 4])];

/// Keyed dialogue: column 0 holds the line key, column 1 the text.
fn dialogue(_: &str, _: u32, _: u16, column_index: u32, _: &str) -> bool {
    column_index == 1
}

fn version(game_version: &str, rows: Rows<'_>) -> ProjectionFixture {
    write_projection_fixture_with(game_version, rows, dialogue)
}

fn text(row_id: u32) -> SourceBinding {
    SourceBinding::new("Projection", row_id, 0, 1)
}

fn update(repository: &TempDir, fixture: &ProjectionFixture) -> ProjectSession {
    let package = SourcePackage::open(&fixture.package_path, repository.path().join("cache"))
        .expect("package");
    ProjectSession::open_with_source_update(repository.path(), package)
        .expect("source update")
        .0
}

fn managed_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let aeria = root.join(".aeria");
    files.insert(
        PathBuf::from("manifest.json"),
        fs::read(aeria.join("manifest.json")).expect("manifest"),
    );
    for entry in fs::read_dir(aeria.join("units")).expect("units") {
        let path = entry.expect("entry").path();
        files.insert(
            PathBuf::from(path.file_name().expect("name")),
            fs::read(&path).expect("shard"),
        );
    }
    files
}

fn copy_project(from: &TempDir) -> TempDir {
    let root = tempfile::tempdir().expect("repository");
    for directory in [".aeria", ".aeria/units"] {
        fs::create_dir_all(root.path().join(directory)).expect("directory");
    }
    for (name, bytes) in managed_files(from.path()) {
        let path = if name == Path::new("manifest.json") {
            root.path().join(".aeria/manifest.json")
        } else {
            root.path().join(".aeria/units").join(name)
        };
        fs::write(path, bytes).expect("copy");
    }
    root
}

struct Unit {
    id: TranslationUnitId,
    target: &'static str,
}

fn assert_kept(session: &ProjectSession, units: &[Unit]) {
    assert_eq!(session.workspace().units().count(), units.len());
    for unit in units {
        let stored = session
            .workspace()
            .unit(unit.id)
            .expect("unit is never removed");
        assert_eq!(stored.target_macro(), unit.target);
        assert_eq!(stored.translator_note(), Some("note"));
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_chain_of_patches_detaches_keeps_and_reattaches_translations() {
    let v1 = version(
        "7.0",
        &[
            (1, ["Q1", "Hello", "", ""]),
            (2, ["Q2", "Goodbye", "", ""]),
            (3, ["Q3", "Later", "", ""]),
        ],
    );
    // 7.1 removes Q2 and inserts a line above everything.
    let v2 = version(
        "7.1",
        &[
            (1, ["Q0", "Inserted", "", ""]),
            (2, ["Q1", "Hello", "", ""]),
            (4, ["Q3", "Later", "", ""]),
        ],
    );
    // 7.2 changes Q3's text and keeps Q2 away.
    let v3 = version(
        "7.2",
        &[
            (1, ["Q0", "Inserted", "", ""]),
            (2, ["Q1", "Hello", "", ""]),
            (4, ["Q3", "Later!", "", ""]),
        ],
    );
    // 7.3 brings Q2 back at a new row with its original text.
    let v4 = version(
        "7.3",
        &[
            (1, ["Q0", "Inserted", "", ""]),
            (2, ["Q1", "Hello", "", ""]),
            (3, ["Q2", "Goodbye", "", ""]),
            (4, ["Q3", "Later!", "", ""]),
        ],
    );

    let repository = tempfile::tempdir().expect("repository");
    let mut session = ProjectSession::initialize(
        repository.path(),
        &v1.package_path,
        repository.path().join("cache"),
        "fr",
    )
    .expect("initialize");
    let mut units = Vec::new();
    for (row_id, target) in [(1, "Bonjour"), (2, "Au revoir"), (3, "Plus tard")] {
        let id = session.set_target(&text(row_id), target).expect("unit");
        session.set_note(id, Some("note".to_owned())).expect("note");
        session
            .set_review_state(id, ReviewState::Reviewed)
            .expect("review");
        units.push(Unit { id, target });
    }
    let [hello, goodbye, later] = [units[0].id, units[1].id, units[2].id];
    drop(session);
    let before_chain = copy_project(&repository);

    let session = update(&repository, &v2);
    assert_kept(&session, &units);
    let unit = |id| session.workspace().unit(id).expect("unit").clone();
    assert_eq!(unit(hello).source_binding(), &text(2), "Q1 follows its key");
    assert_eq!(unit(hello).review_state(), ReviewState::Reviewed);
    assert_eq!(
        unit(goodbye).source_status(),
        SourceStatus::Detached(DetachReason::RowRemoved)
    );
    assert_eq!(unit(later).source_binding(), &text(4));
    drop(session);

    let session = update(&repository, &v3);
    assert_kept(&session, &units);
    let unit = |id| session.workspace().unit(id).expect("unit").clone();
    assert_eq!(
        unit(goodbye).source_status(),
        SourceStatus::Detached(DetachReason::RowRemoved),
        "a detached unit stays detached, untouched, while its line is absent"
    );
    assert_eq!(unit(later).review_state(), ReviewState::NeedsReview);
    drop(session);

    let session = update(&repository, &v4);
    assert_kept(&session, &units);
    let unit = |id| session.workspace().unit(id).expect("unit").clone();
    let returned = unit(goodbye);
    assert!(
        returned.is_bound(),
        "the returning line binds its translation"
    );
    assert_eq!(returned.source_binding(), &text(3));
    assert_eq!(returned.review_state(), ReviewState::Reviewed);
    assert_eq!(unit(hello).source_binding(), &text(2));
    assert_eq!(unit(later).review_state(), ReviewState::NeedsReview);
    drop(session);

    // Updating 7.0 directly to 7.3 reaches exactly the same files as the
    // chain: skipping patches never changes where a translation ends up.
    let direct = copy_project(&before_chain);
    drop(update(&direct, &v4));
    assert_eq!(
        managed_files(direct.path()),
        managed_files(repository.path())
    );

    // Every version stays reachable: going back to 7.0 is an update too.
    let session = update(&repository, &v1);
    assert_kept(&session, &units);
    for (id, row_id) in [(hello, 1), (goodbye, 2), (later, 3)] {
        assert_eq!(
            session.workspace().unit(id).expect("unit").source_binding(),
            &text(row_id)
        );
    }
}
