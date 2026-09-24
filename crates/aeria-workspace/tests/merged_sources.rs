//! Workspace state produced by Git merges of work done against different game
//! versions. A merge can combine units whose stored source facts describe an
//! older source, or two units that claim one occurrence. Such state must be
//! detected on open and reconciled by the deterministic source update: no
//! translation is shown against source text it was not made for, and no
//! unit, target, note, or identity is lost.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use aeria_core::{DetachReason, ReviewState, SourceBinding, SourceStatus, TranslationUnit};
use aeria_hsp::SourcePackage;
use aeria_workspace::{
    ProjectSession, ProjectSessionError, SourceUpdateRequirement, decode_unit_shard,
    encode_unit_shard,
};
use tempfile::TempDir;

#[path = "support/fixture.rs"]
mod fixture;

use fixture::{Fixture, write_fixture_with_text};

const ROW_CHANGED: u32 = 42;
const ROW_STABLE: u32 = 7;

fn binding(row_id: u32) -> SourceBinding {
    SourceBinding::new("Synthetic", row_id, 0, 0)
}

/// The game version before and after a patch that changes the text of row
/// 42 ("one" becomes "uno") and keeps row 7.
fn versions() -> (Fixture, Fixture) {
    (
        write_fixture_with_text("en", "7.0", "one"),
        write_fixture_with_text("en", "7.1", "uno"),
    )
}

fn cache(root: &TempDir) -> PathBuf {
    root.path().join("cache")
}

fn package(root: &TempDir, fixture: &Fixture) -> SourcePackage {
    SourcePackage::open(&fixture.package_path, cache(root)).expect("package")
}

fn open(root: &TempDir, fixture: &Fixture) -> Result<ProjectSession, Box<ProjectSessionError>> {
    ProjectSession::open(root.path(), &fixture.package_path, cache(root)).map_err(Box::new)
}

fn update(root: &TempDir, fixture: &Fixture) -> ProjectSession {
    ProjectSession::open_with_source_update(root.path(), package(root, fixture))
        .expect("source update")
        .0
}

fn initialize(fixture: &Fixture) -> TempDir {
    let root = tempfile::tempdir().expect("repository");
    ProjectSession::initialize(root.path(), &fixture.package_path, cache(&root), "fr")
        .expect("initialize");
    root
}

/// Copies the managed `.aeria` state into a new repository, like a clone.
fn clone(from: &TempDir) -> TempDir {
    let root = tempfile::tempdir().expect("repository");
    copy_dir(&from.path().join(".aeria"), &root.path().join(".aeria"));
    root
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("directory");
    for entry in fs::read_dir(from).expect("read directory") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy");
        }
    }
}

fn units(root: &TempDir) -> BTreeMap<String, Vec<TranslationUnit>> {
    let directory = root.path().join(".aeria/units");
    let mut shards = BTreeMap::new();
    for entry in fs::read_dir(&directory).expect("units") {
        let path = entry.expect("entry").path();
        let bytes = fs::read(&path).expect("shard");
        shards.insert(
            path.file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned(),
            decode_unit_shard(&bytes, &path).expect("decode"),
        );
    }
    shards
}

/// Writes what a Git merge produces from `ours` and `theirs`: our manifest
/// and the union of both sides' units, taking their version of a unit both
/// sides hold (the explicit "theirs" resolution of a same-unit conflict).
fn merged(ours: &TempDir, theirs: &TempDir) -> TempDir {
    let root = clone(ours);
    let mut shards = units(ours);
    for (name, their_units) in units(theirs) {
        let shard = shards.entry(name).or_default();
        for unit in their_units {
            shard.retain(|existing| existing.id() != unit.id());
            shard.push(unit);
        }
    }
    for (name, mut shard) in shards {
        shard.sort_by_key(TranslationUnit::id);
        let path = root.path().join(".aeria/units").join(&name);
        fs::write(&path, encode_unit_shard(&shard, &path).expect("encode")).expect("write");
    }
    root
}

/// Every unit's identity, target, and note: what no reconciliation may change.
fn translations(root: &TempDir) -> BTreeMap<String, (String, Option<String>)> {
    units(root)
        .into_values()
        .flatten()
        .map(|unit| {
            (
                unit.id().to_string(),
                (
                    unit.target_macro().to_owned(),
                    unit.translator_note().map(str::to_owned),
                ),
            )
        })
        .collect()
}

fn managed_files(root: &TempDir) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let aeria = root.path().join(".aeria");
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

fn requirement(error: ProjectSessionError) -> SourceUpdateRequirement {
    match error {
        ProjectSessionError::SourceUpdateRequired { requirement, .. } => requirement,
        other => panic!("expected a required source update, got {other}"),
    }
}

#[test]
fn a_translation_merged_from_an_older_game_version_is_reviewed_never_misattributed() {
    let (old, new) = versions();
    let base = initialize(&old);
    let mut session = open(&base, &old).expect("base");
    let changed = session
        .set_target(&binding(ROW_CHANGED), "Un")
        .expect("unit on the changed row");
    drop(session);

    // A applies the game patch; B keeps translating on the old version.
    let a = clone(&base);
    drop(update(&a, &new));
    let b = clone(&base);
    let mut session = open(&b, &old).expect("B on the old version");
    session
        .set_target(&binding(ROW_CHANGED), "Un (B)")
        .expect("B edits the changed row");
    let stable = session
        .set_target(&binding(ROW_STABLE), "Deux (B)")
        .expect("B adds a unit on a stable row");
    session
        .set_review_state(stable, ReviewState::Reviewed)
        .expect("B reviews it");
    drop(session);

    // The merge takes A's manifest (new version) and B's units.
    let merged = merged(&a, &b);
    let before = managed_files(&merged);
    let merged_translations = translations(&merged);
    let Err(error) = open(&merged, &new) else {
        panic!("stale merged units must not open as current");
    };
    assert_eq!(
        requirement(*error),
        SourceUpdateRequirement::SourceFactsMismatch { units: 1 }
    );
    assert_eq!(
        before,
        managed_files(&merged),
        "a failed open writes nothing"
    );

    let (session, report) =
        ProjectSession::open_with_source_update(merged.path(), package(&merged, &new))
            .expect("reconcile");
    let summary = report.expect("reconciled").plan.summary;
    assert_eq!(summary.source_changed, 1);
    assert_eq!(summary.detached, 0);

    let changed_unit = session
        .workspace()
        .unit(changed)
        .expect("changed unit kept");
    assert_eq!(changed_unit.target_macro(), "Un (B)");
    assert_eq!(changed_unit.review_state(), ReviewState::NeedsReview);
    assert_eq!(changed_unit.source_binding(), &binding(ROW_CHANGED));
    let stable_unit = session.workspace().unit(stable).expect("stable unit kept");
    assert_eq!(stable_unit.target_macro(), "Deux (B)");
    assert_eq!(stable_unit.review_state(), ReviewState::Reviewed);

    // The overlay is shown against the new text only after review marking.
    let page = session
        .page_translation_rows("Synthetic", None, 10)
        .expect("rows read after reconciliation");
    let cell = page
        .rows
        .iter()
        .find(|row| row.row_id == ROW_CHANGED)
        .expect("changed row")
        .cells[0]
        .clone();
    assert_eq!(cell.source_macro, "uno");
    assert_eq!(cell.translation.expect("overlay").target_macro, "Un (B)");
    drop(session);

    // Reconciling the merge equals B applying the patch itself, so every
    // collaborator converges on the same bytes.
    let b_updated = clone(&b);
    drop(update(&b_updated, &new));
    assert_eq!(managed_files(&b_updated), managed_files(&merged));
    assert_eq!(merged_translations, translations(&merged));
    open(&merged, &new).expect("reconciled project opens");
}

#[test]
fn a_binding_claimed_by_two_merged_units_keeps_the_current_owner_and_preserves_the_other() {
    let (old, new) = versions();
    let base = initialize(&old);

    let a = clone(&base);
    let mut session = update(&a, &new);
    let current = session
        .set_target(&binding(ROW_CHANGED), "Uno (A)")
        .expect("A translates the new text");
    drop(session);

    let b = clone(&base);
    let mut session = open(&b, &old).expect("B on the old version");
    let stale = session
        .set_target(&binding(ROW_CHANGED), "One (B)")
        .expect("B translates the old text");
    session
        .set_note(stale, Some("B's context".to_owned()))
        .expect("B adds a note");
    drop(session);
    assert_ne!(current, stale, "identity includes the source text");

    let merged = merged(&a, &b);
    let merged_translations = translations(&merged);
    let Err(error) = open(&merged, &new) else {
        panic!("two owners of one occurrence must not open");
    };
    // One duplicate claim and one stale fingerprint.
    assert_eq!(
        requirement(*error),
        SourceUpdateRequirement::SourceFactsMismatch { units: 2 }
    );

    let session = update(&merged, &new);
    let owner = session.workspace().unit(current).expect("current owner");
    assert!(owner.is_bound());
    assert_eq!(owner.target_macro(), "Uno (A)");
    let other = session.workspace().unit(stale).expect("other unit kept");
    assert_eq!(
        other.source_status(),
        SourceStatus::Detached(DetachReason::BindingConflict)
    );
    assert_eq!(other.target_macro(), "One (B)");
    assert_eq!(other.translator_note(), Some("B's context"));
    assert_eq!(session.detached_units().count(), 1);
    drop(session);
    assert_eq!(merged_translations, translations(&merged));

    // The merge direction does not change the outcome.
    let reverse = merged_reverse(&b, &a, &new);
    assert_eq!(managed_files(&reverse), managed_files(&merged));
}

/// Merges with B as "ours"; B's manifest still records the old version, so
/// the merged project is first updated to the new version like any other.
fn merged_reverse(b: &TempDir, a: &TempDir, new: &Fixture) -> TempDir {
    let root = merged(b, a);
    let manifest = fs::read(a.path().join(".aeria/manifest.json")).expect("manifest");
    fs::write(root.path().join(".aeria/manifest.json"), manifest).expect("manifest");
    drop(update(&root, new));
    root
}

#[test]
fn a_session_reconciles_after_a_merge_but_never_across_source_versions() {
    let (old, new) = versions();
    let base = initialize(&old);
    let mut session = open(&base, &old).expect("base");
    let changed = session
        .set_target(&binding(ROW_CHANGED), "Un")
        .expect("unit");
    drop(session);
    let a = clone(&base);
    let b = clone(&base);
    let mut session = update(&a, &new);

    let mut b_session = open(&b, &old).expect("B");
    b_session
        .set_target(&binding(ROW_CHANGED), "Un (B)")
        .expect("B edit");
    drop(b_session);

    // Simulate the merge arriving in A's repository while A's session is open.
    let merged = merged(&a, &b);
    for (name, bytes) in managed_files(&merged) {
        let path = if name == Path::new("manifest.json") {
            a.path().join(".aeria/manifest.json")
        } else {
            a.path().join(".aeria/units").join(name)
        };
        fs::write(path, bytes).expect("merged file");
    }
    assert!(matches!(
        session.reload_workspace(),
        Err(ProjectSessionError::SourceUpdateRequired {
            requirement: SourceUpdateRequirement::SourceFactsMismatch { units: 1 },
            ..
        })
    ));
    let report = session
        .reload_and_reconcile_workspace()
        .expect("reconcile")
        .expect("reconciled");
    assert_eq!(report.plan.summary.source_changed, 1);
    let unit = session.workspace().unit(changed).expect("unit");
    assert_eq!(unit.target_macro(), "Un (B)");
    assert_eq!(unit.review_state(), ReviewState::NeedsReview);
    assert!(
        session
            .reload_and_reconcile_workspace()
            .expect("idempotent")
            .is_none()
    );

    // A merge that brings another source version is never reconciled by
    // this session: it needs that version's package.
    fs::copy(
        b.path().join(".aeria/manifest.json"),
        a.path().join(".aeria/manifest.json"),
    )
    .expect("older manifest");
    let before = managed_files(&a);
    assert!(matches!(
        session.reload_and_reconcile_workspace(),
        Err(ProjectSessionError::SourceUpdateRequired {
            requirement: SourceUpdateRequirement::ContentChanged { .. },
            ..
        })
    ));
    assert_eq!(before, managed_files(&a));
    assert_eq!(
        session
            .workspace()
            .unit(changed)
            .expect("state kept")
            .target_macro(),
        "Un (B)"
    );
}

#[test]
fn a_stale_unit_is_never_shown_against_other_source_text() {
    let (old, new) = versions();
    let base = initialize(&old);
    let mut session = open(&base, &old).expect("base");
    session
        .set_target(&binding(ROW_CHANGED), "Un")
        .expect("unit");
    drop(session);
    let a = clone(&base);
    drop(update(&a, &new));
    // A merge that only brought the new manifest: the unit still records
    // the old text.
    let stale = clone(&base);
    fs::copy(
        a.path().join(".aeria/manifest.json"),
        stale.path().join(".aeria/manifest.json"),
    )
    .expect("new manifest");
    assert!(matches!(
        open(&stale, &new).map(|_| ()).map_err(|error| *error),
        Err(ProjectSessionError::SourceUpdateRequired {
            requirement: SourceUpdateRequirement::SourceFactsMismatch { units: 1 },
            ..
        })
    ));
}
