use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use aeria_core::{ReviewState, Sha256Hash, SourceBinding};
use aeria_hxs::HxsSnapshot;
use aeria_workspace::{
    MAX_TRANSLATION_PAGE_SIZE, ProjectSession, ProjectSessionError, TranslationReadError,
    Workspace, WorkspaceError, WorkspaceStore,
};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const SYNTHETIC_SCHEMA: &str = include_str!("../../aeria-hxs/tests/fixtures/synthetic_v1.sql");
const APPLICATION_ID: i64 = 0x4841_544c;

struct Fixture {
    _directory: TempDir,
    path: PathBuf,
    one_macro_hash: [u8; 32],
    one_row_technical_hash: [u8; 32],
    two_macro_hash: [u8; 32],
    two_raw_hash: [u8; 32],
}

#[test]
fn creates_a_unit_from_a_verified_hxs_string_cell_without_copying_source_text() {
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).expect("fixture should verify");
    let mut workspace = Workspace::from_verified_snapshot(&snapshot, "fr").expect("workspace");

    let id = workspace
        .create_unit_from_hxs(&snapshot, "Synthetic", 42, 0, 0, "Bonjour")
        .expect("source cell should create a unit");
    let unit = workspace.unit(id).expect("created unit");

    assert_eq!(unit.source_binding().sheet_name(), "Synthetic");
    assert_eq!(unit.source_binding().row_id(), 42);
    assert_eq!(unit.source_binding().subrow_id(), 0);
    assert_eq!(unit.source_binding().column_index(), 0);
    assert_eq!(
        unit.source_fingerprint().macro_text_hash(),
        Sha256Hash::from_bytes(fixture.one_macro_hash)
    );
    assert_eq!(unit.source_fingerprint().raw_value_hash(), None);
    assert_eq!(
        unit.source_fingerprint().row_technical_hash(),
        Sha256Hash::from_bytes(fixture.one_row_technical_hash)
    );
    assert_eq!(unit.target_macro(), "Bonjour");
    assert_eq!(unit.review_state(), ReviewState::Draft);
    assert!(!format!("{unit:?}").contains("\"one\""));

    let raw_id = workspace
        .create_unit_from_hxs(&snapshot, "Synthetic", 7, 0, 0, "Deux")
        .expect("second source cell should create a unit");
    let raw_unit = workspace.unit(raw_id).expect("second unit");
    assert_eq!(
        raw_unit.source_fingerprint().raw_value_hash(),
        Some(Sha256Hash::from_bytes(fixture.two_raw_hash))
    );
    assert_eq!(
        raw_unit.source_fingerprint().macro_text_hash(),
        Sha256Hash::from_bytes(fixture.two_macro_hash)
    );
}

#[test]
fn workspace_is_sparse_and_explicit_empty_targets_are_present() {
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).expect("fixture should verify");
    let mut workspace = Workspace::from_verified_snapshot(&snapshot, "fr").expect("workspace");

    let id = workspace
        .create_unit_from_hxs(&snapshot, "Synthetic", 42, 0, 0, "")
        .expect("empty target is valid");

    assert_eq!(workspace.units().count(), 1);
    assert_eq!(workspace.unit(id).expect("unit").target_macro(), "");
    assert_eq!(
        workspace
            .unit_by_source_binding(&SourceBinding::new("Synthetic", 42, 0, 0))
            .map(aeria_core::TranslationUnit::id),
        Some(id)
    );
    assert!(
        workspace
            .unit_by_source_binding(&SourceBinding::new("Synthetic", 7, 0, 0))
            .is_none()
    );
}

#[test]
fn requested_missing_hxs_string_cells_are_rejected_without_inserting_units() {
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).expect("fixture should verify");
    let mut workspace = Workspace::from_verified_snapshot(&snapshot, "fr").expect("workspace");

    assert!(matches!(
        workspace.create_unit_from_hxs(&snapshot, "Synthetic", 999, 0, 0, "target"),
        Err(WorkspaceError::SourceCellNotFound { .. })
    ));
    assert!(matches!(
        workspace.create_unit_from_hxs(&snapshot, "Synthetic", 42, 0, 1, "target"),
        Err(WorkspaceError::SourceCellNotFound { .. })
    ));
    assert_eq!(workspace.units().count(), 0);
}

#[test]
fn duplicate_identity_is_rejected() {
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).expect("fixture should verify");
    let mut workspace = Workspace::from_verified_snapshot(&snapshot, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&snapshot, "Synthetic", 42, 0, 0, "first")
        .expect("first unit");

    assert!(matches!(
        workspace.create_unit_from_hxs(&snapshot, "Synthetic", 42, 0, 0, "second"),
        Err(WorkspaceError::DuplicateUnitId { id: duplicate }) if duplicate == id
    ));
}

#[test]
fn opaque_targets_are_valid_but_malformed_targets_are_rejected() {
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).expect("fixture should verify");
    let mut workspace = Workspace::from_verified_snapshot(&snapshot, "fr").expect("workspace");

    let opaque_id = workspace
        .create_unit_from_hxs(&snapshot, "Synthetic", 42, 0, 0, "<futuremacro(1)>opaque")
        .expect("opaque but preservable target is valid");
    assert_eq!(
        workspace
            .unit(opaque_id)
            .expect("opaque unit")
            .target_macro(),
        "<futuremacro(1)>opaque"
    );

    assert!(matches!(
        workspace.create_unit_from_hxs(&snapshot, "Synthetic", 7, 0, 0, "<if(1,2,3>"),
        Err(WorkspaceError::InvalidTarget { .. })
    ));
    assert_eq!(workspace.units().count(), 1);
}

#[test]
fn editing_target_resets_review() {
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).expect("fixture should verify");
    let mut workspace = Workspace::from_verified_snapshot(&snapshot, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&snapshot, "Synthetic", 42, 0, 0, "Bonjour")
        .expect("unit");

    workspace
        .update_review_state(id, ReviewState::Reviewed)
        .expect("explicit review");
    workspace
        .update_target(id, "<bold(1)>Salut")
        .expect("valid target edit");
    assert_eq!(
        workspace.unit(id).unwrap().review_state(),
        ReviewState::Draft
    );
}

#[test]
fn unit_iteration_is_deterministic() {
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).expect("fixture should verify");
    let mut workspace = Workspace::from_verified_snapshot(&snapshot, "fr").expect("workspace");
    let first = workspace
        .create_unit_from_hxs(&snapshot, "Synthetic", 42, 0, 0, "one target")
        .expect("first unit");
    let second = workspace
        .create_unit_from_hxs(&snapshot, "Synthetic", 7, 0, 0, "two target")
        .expect("second unit");

    let ids: Vec<_> = workspace
        .units()
        .map(aeria_core::TranslationUnit::id)
        .collect();
    assert_eq!(
        ids,
        if first < second {
            vec![first, second]
        } else {
            vec![second, first]
        }
    );
}

#[test]
fn initializes_and_reopens_a_new_project_without_persisting_the_source_path() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let unrelated_path = repository.path().join("README.md");
    fs::write(&unrelated_path, b"project-owned file\n").expect("unrelated file");
    let source_path = fixture.path.clone();

    let session = ProjectSession::initialize(repository.path(), &source_path, "fr")
        .expect("new project should initialize");
    let source_metadata = session.source().metadata();
    assert_eq!(session.repository_root(), repository.path());
    assert_eq!(session.source_path(), source_path.as_path());
    assert_eq!(session.workspace().units().count(), 0);
    assert_eq!(
        session.workspace().metadata().source_language(),
        source_metadata.source_language
    );
    assert_eq!(session.workspace().metadata().target_language(), "fr");
    assert_eq!(
        session.workspace().metadata().source_content_id(),
        source_metadata.content_id
    );
    assert_eq!(
        session.workspace().metadata().source_snapshot_id(),
        source_metadata.snapshot_id
    );
    assert!(repository.path().join(".aeria/manifest.json").is_file());
    assert!(!repository.path().join(".aeria/units").exists());
    let manifest =
        fs::read_to_string(repository.path().join(".aeria/manifest.json")).expect("manifest");
    assert!(!manifest.contains(source_path.to_string_lossy().as_ref()));
    assert_managed_files_omit_path(repository.path(), &source_path);
    assert_eq!(
        fs::read(&unrelated_path).expect("unrelated file"),
        b"project-owned file\n"
    );

    let expected_metadata = session.workspace().metadata().clone();
    drop(session);
    let reopened = ProjectSession::open(repository.path(), &source_path)
        .expect("initialized project should reopen");
    assert_eq!(reopened.workspace().metadata(), &expected_metadata);
    assert_eq!(reopened.workspace().units().count(), 0);
    assert_eq!(reopened.source_path(), source_path.as_path());
}

#[test]
fn opens_existing_project_and_preserves_managed_files() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let source = HxsSnapshot::open(&fixture.path).expect("fixture should verify");
    let mut workspace = Workspace::from_verified_snapshot(&source, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&source, "Synthetic", 42, 0, 0, "Bonjour")
        .expect("unit");
    WorkspaceStore::new(repository.path())
        .initialize(&workspace)
        .expect("workspace should initialize");
    let before = managed_files(repository.path());

    let session = ProjectSession::open(repository.path(), &fixture.path)
        .expect("existing project should open");
    assert_eq!(session.workspace().units().count(), 1);
    assert_eq!(
        session
            .source()
            .string_cell("Synthetic", 42, 0, 0)
            .expect("source read")
            .expect("source cell")
            .macro_text,
        "one"
    );
    assert_eq!(session.workspace().metadata(), workspace.metadata());
    drop(session);
    assert_managed_files_omit_path(repository.path(), &fixture.path);
    assert_eq!(before, managed_files(repository.path()));
}

#[test]
fn translation_read_composes_verified_source_with_sparse_workspace_overlay() {
    let fixture = write_fixture();
    let source = HxsSnapshot::open(&fixture.path).expect("source");
    let mut workspace = Workspace::from_verified_snapshot(&source, "fr").expect("workspace");
    let translated_id = workspace
        .create_unit_from_hxs(&source, "Synthetic", 42, 0, 0, "Bonjour")
        .expect("translated unit");
    workspace
        .update_review_state(translated_id, ReviewState::Reviewed)
        .expect("review state");
    workspace
        .update_note(translated_id, Some("Checked by editor".to_owned()))
        .expect("translator note");

    let repository = tempfile::tempdir().expect("temporary repository");
    WorkspaceStore::new(repository.path())
        .initialize(&workspace)
        .expect("workspace should initialize");
    let session = ProjectSession::open(repository.path(), &fixture.path).expect("session");

    let first = session
        .page_translation_entries("Synthetic", None, 1)
        .expect("first translation page");
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.entries[0].source_binding.row_id(), 7);
    assert_eq!(first.entries[0].source_macro, "two");
    assert!(first.entries[0].translation.is_none());
    let after = first.next_after.expect("second page cursor");

    let second = session
        .page_translation_entries("Synthetic", Some(&after), 1)
        .expect("second translation page");
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.entries[0].source_binding.row_id(), 42);
    assert_eq!(second.entries[0].source_macro, "one");
    let overlay = second.entries[0]
        .translation
        .as_ref()
        .expect("translated occurrence has an overlay");
    assert_eq!(overlay.translation_unit_id, translated_id);
    assert_eq!(overlay.target_macro, "Bonjour");
    assert_eq!(overlay.review_state, ReviewState::Reviewed);
    assert_eq!(
        overlay.translator_note.as_deref(),
        Some("Checked by editor")
    );
    assert!(second.next_after.is_none());
}

#[test]
fn translation_read_distinguishes_an_empty_target_from_missing_workspace_state() {
    let fixture = write_fixture();
    let source = HxsSnapshot::open(&fixture.path).expect("source");
    let mut workspace = Workspace::from_verified_snapshot(&source, "fr").expect("workspace");
    let empty_id = workspace
        .create_unit_from_hxs(&source, "Synthetic", 7, 0, 0, "")
        .expect("empty target unit");

    let repository = tempfile::tempdir().expect("temporary repository");
    WorkspaceStore::new(repository.path())
        .initialize(&workspace)
        .expect("workspace should initialize");
    let session = ProjectSession::open(repository.path(), &fixture.path).expect("session");
    let page = session
        .page_translation_entries("Synthetic", None, 2)
        .expect("translation page");

    let empty = &page.entries[0];
    let overlay = empty
        .translation
        .as_ref()
        .expect("an explicit empty target still has a unit");
    assert_eq!(overlay.translation_unit_id, empty_id);
    assert_eq!(overlay.target_macro, "");
    assert!(page.entries[1].translation.is_none());
}

#[test]
fn translation_read_rejects_invalid_limits_and_cross_sheet_cursors() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    ProjectSession::initialize(repository.path(), &fixture.path, "fr")
        .expect("project should initialize");
    let session = ProjectSession::open(repository.path(), &fixture.path).expect("session");

    assert!(matches!(
        session.page_translation_entries("Synthetic", None, 0),
        Err(TranslationReadError::InvalidPageLimit { limit: 0, max })
            if max == MAX_TRANSLATION_PAGE_SIZE
    ));
    assert!(matches!(
        session.page_translation_entries("Synthetic", None, MAX_TRANSLATION_PAGE_SIZE + 1),
        Err(TranslationReadError::InvalidPageLimit { limit, max })
            if limit == MAX_TRANSLATION_PAGE_SIZE + 1 && max == MAX_TRANSLATION_PAGE_SIZE
    ));
    assert!(matches!(
        session.page_translation_entries(
            "Synthetic",
            Some(&SourceBinding::new("Other", 7, 0, 0)),
            1,
        ),
        Err(TranslationReadError::CursorSheetMismatch {
            requested_sheet,
            cursor_sheet,
        }) if requested_sheet == "Synthetic" && cursor_sheet == "Other"
    ));
}

#[test]
fn translation_read_rejects_a_stale_sparse_unit_fingerprint_without_repairing_files() {
    let fixture = write_fixture();
    let source = HxsSnapshot::open(&fixture.path).expect("source");
    let mut workspace = Workspace::from_verified_snapshot(&source, "fr").expect("workspace");
    let unit_id = workspace
        .create_unit_from_hxs(&source, "Synthetic", 42, 0, 0, "Bonjour")
        .expect("unit");
    let repository = tempfile::tempdir().expect("temporary repository");
    WorkspaceStore::new(repository.path())
        .initialize(&workspace)
        .expect("workspace should initialize");

    let shard = fs::read_dir(repository.path().join(".aeria/units"))
        .expect("unit shards")
        .map(|entry| entry.expect("unit shard entry").path())
        .next()
        .expect("one unit shard");
    let before = managed_files(repository.path());
    let text = fs::read_to_string(&shard).expect("unit shard text");
    let persisted_hash = hex(&fixture.one_macro_hash);
    let stale_hash = "00".repeat(32);
    let updated = text.replace(
        &format!("\"macroTextHash\":\"{persisted_hash}\""),
        &format!("\"macroTextHash\":\"{stale_hash}\""),
    );
    assert_ne!(updated, text, "test fixture must change the persisted hash");
    fs::write(&shard, updated).expect("stale unit fixture");
    let before_read = managed_files(repository.path());

    let session = ProjectSession::open(repository.path(), &fixture.path).expect("session");
    let error = session
        .page_translation_entries(
            "Synthetic",
            Some(&SourceBinding::new("Synthetic", 7, 0, 0)),
            2,
        )
        .expect_err("stale unit must fail the read");
    assert!(matches!(
        error,
        TranslationReadError::WorkspaceSourceMismatch {
            translation_unit_id,
            source_binding,
            ..
        } if translation_unit_id == unit_id
            && source_binding == SourceBinding::new("Synthetic", 42, 0, 0)
    ));
    assert_eq!(before_read, managed_files(repository.path()));
    assert_ne!(
        before, before_read,
        "the test fixture should be observably stale"
    );
}

#[test]
fn translation_reads_do_not_modify_managed_workspace_files() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let session = ProjectSession::initialize(repository.path(), &fixture.path, "fr")
        .expect("project should initialize");
    let before = managed_files(repository.path());

    let mut after = None;
    loop {
        let page = session
            .page_translation_entries("Synthetic", after.as_ref(), 1)
            .expect("translation page");
        assert!(page.entries.len() <= 1);
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(before, managed_files(repository.path()));
}

#[test]
fn rejects_a_different_verified_snapshot_without_modifying_the_workspace() {
    let original_fixture = write_fixture();
    let different_snapshot = write_fixture_with("en", "different-game");
    let repository = tempfile::tempdir().expect("temporary repository");
    ProjectSession::initialize(repository.path(), &original_fixture.path, "fr")
        .expect("new project should initialize");
    let before = managed_files(repository.path());

    let error = ProjectSession::open(repository.path(), &different_snapshot.path)
        .err()
        .expect("different snapshot must be rejected");
    assert!(matches!(
        error,
        ProjectSessionError::Compatibility {
            source: WorkspaceError::SourceSnapshotMismatch { .. },
            ..
        }
    ));
    assert_eq!(before, managed_files(repository.path()));
}

#[test]
fn rejects_a_different_verified_source_language_without_modifying_the_workspace() {
    let original_fixture = write_fixture();
    let different_language = write_fixture_with("ja", "test-game");
    let repository = tempfile::tempdir().expect("temporary repository");
    ProjectSession::initialize(repository.path(), &original_fixture.path, "fr")
        .expect("new project should initialize");
    let before = managed_files(repository.path());

    let error = ProjectSession::open(repository.path(), &different_language.path)
        .err()
        .expect("different source language must be rejected");
    assert!(matches!(
        error,
        ProjectSessionError::Compatibility {
            source: WorkspaceError::SourceLanguageMismatch { .. },
            ..
        }
    ));
    assert_eq!(before, managed_files(repository.path()));
}

#[test]
fn invalid_hxs_and_target_language_fail_before_publishing_workspace_state() {
    let fixture = write_fixture();
    let invalid_source_directory = tempfile::tempdir().expect("invalid source directory");
    let invalid_source = invalid_source_directory.path().join("invalid.hxs");
    fs::write(&invalid_source, b"not an HXS database").expect("invalid source");
    let invalid_source_repository = tempfile::tempdir().expect("temporary repository");

    let error = ProjectSession::initialize(invalid_source_repository.path(), &invalid_source, "fr")
        .err()
        .expect("invalid HXS must be rejected");
    assert!(matches!(error, ProjectSessionError::Source { .. }));
    assert!(!invalid_source_repository.path().join(".aeria").exists());
    let error = ProjectSession::open(invalid_source_repository.path(), &invalid_source)
        .err()
        .expect("invalid HXS must be rejected while opening");
    assert!(matches!(error, ProjectSessionError::Source { .. }));

    let invalid_target_repository = tempfile::tempdir().expect("temporary repository");
    let error = ProjectSession::initialize(invalid_target_repository.path(), &fixture.path, " ")
        .err()
        .expect("invalid target language must be rejected");
    assert!(matches!(error, ProjectSessionError::Workspace { .. }));
    assert!(!invalid_target_repository.path().join(".aeria").exists());
}

#[test]
fn initialization_protects_existing_project_state() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    ProjectSession::initialize(repository.path(), &fixture.path, "fr")
        .expect("new project should initialize");
    let before = managed_files(repository.path());

    let error = ProjectSession::initialize(repository.path(), &fixture.path, "fr")
        .err()
        .expect("existing project must not be replaced");
    assert!(matches!(
        error,
        ProjectSessionError::Store {
            source: aeria_workspace::WorkspaceStoreError::AlreadyInitialized { .. },
            ..
        }
    ));
    assert_eq!(before, managed_files(repository.path()));
}

fn managed_files(repository_root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let aeria_root = repository_root.join(".aeria");
    let mut files = BTreeMap::new();
    let manifest = aeria_root.join("manifest.json");
    files.insert(manifest.clone(), fs::read(manifest).expect("manifest"));
    let units = aeria_root.join("units");
    if units.is_dir() {
        for entry in fs::read_dir(units).expect("units directory") {
            let entry = entry.expect("unit entry");
            let path = entry.path();
            if path.is_file() {
                files.insert(path.clone(), fs::read(path).expect("unit shard"));
            }
        }
    }
    files
}

fn assert_managed_files_omit_path(repository_root: &Path, source_path: &Path) {
    let source_path = source_path.to_string_lossy();
    for bytes in managed_files(repository_root).values() {
        assert!(!String::from_utf8_lossy(bytes).contains(source_path.as_ref()));
    }
}

#[allow(clippy::too_many_lines)]
fn write_fixture() -> Fixture {
    write_fixture_with("en", "test-game")
}

#[allow(clippy::too_many_lines)]
fn write_fixture_with(source_language: &str, game_version: &str) -> Fixture {
    let directory = tempfile::tempdir().expect("create fixture directory");
    let path = directory.path().join("fixture.hxs");
    let connection = Connection::open(&path).expect("create fixture database");
    connection
        .execute_batch(SYNTHETIC_SCHEMA)
        .expect("create fixture schema");
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = 1; PRAGMA foreign_keys = ON;"
        ))
        .expect("set HXS identity");

    let one_macro_hash = macro_hash("one");
    let two_macro_hash = macro_hash("two");
    let two_raw_hash = raw_hash(b"raw");
    let one_row_technical_hash = row_technical_hash("Synthetic", 42, 0);
    let two_row_technical_hash = row_technical_hash("Synthetic", 7, 0);
    let one_row_string_hash = row_string_hash("Synthetic", 42, 0, 0, &one_macro_hash, None);
    let two_row_string_hash =
        row_string_hash("Synthetic", 7, 0, 0, &two_macro_hash, Some(&two_raw_hash));
    let one_row_hash = row_hash(
        "Synthetic",
        42,
        0,
        &one_row_technical_hash,
        &one_row_string_hash,
    );
    let second_row_hash = row_hash(
        "Synthetic",
        7,
        0,
        &two_row_technical_hash,
        &two_row_string_hash,
    );
    let schema_hash = schema_hash("Synthetic");
    let sheet_technical_hash = sheet_rows_hash(
        "HARMONIA-HXS-V1-SHEET-TECHNICAL",
        "Synthetic",
        &[
            (7, 0, two_row_technical_hash),
            (42, 0, one_row_technical_hash),
        ],
    );
    let sheet_string_hash = sheet_rows_hash(
        "HARMONIA-HXS-V1-SHEET-STRINGS",
        "Synthetic",
        &[(7, 0, two_row_string_hash), (42, 0, one_row_string_hash)],
    );
    let content_hash = digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-SHEET");
        framed_text(hasher, "Synthetic");
        hasher.update(0_u32.to_le_bytes());
        hasher.update(schema_hash);
        hasher.update(sheet_technical_hash);
        hasher.update(sheet_string_hash);
    });
    let content_id = format!(
        "sha256:{}",
        hex(&digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-CONTENT-v1");
            framed_text(hasher, source_language);
            framed_text(hasher, "Synthetic");
            framed_text(hasher, source_language);
            hasher.update(schema_hash);
            hasher.update(content_hash);
        }))
    );
    let snapshot_id = format!(
        "sha256:{}",
        hex(&digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-SNAPSHOT-v1");
            framed_text(hasher, game_version);
            framed_text(hasher, source_language);
            framed_text(hasher, &content_id);
        }))
    );

    connection
        .execute(
            "INSERT INTO sheets (id, name, variant, effective_language, column_count, row_count, schema_hash, technical_hash, string_hash, content_hash) VALUES (1, 'Synthetic', 0, ?1, 1, 2, ?2, ?3, ?4, ?5)",
            params![source_language, schema_hash.as_slice(), sheet_technical_hash.as_slice(), sheet_string_hash.as_slice(), content_hash.as_slice()],
        )
        .expect("insert sheet");
    connection
        .execute(
            "INSERT INTO columns (sheet_id, column_index, offset, type) VALUES (1, 0, 0, 1)",
            [],
        )
        .expect("insert String column");
    for (row_id, row_hash, row_technical_hash, row_string_hash) in [
        (
            42_u32,
            one_row_hash,
            one_row_technical_hash,
            one_row_string_hash,
        ),
        (
            7_u32,
            second_row_hash,
            two_row_technical_hash,
            two_row_string_hash,
        ),
    ] {
        connection
            .execute(
                "INSERT INTO rows (sheet_id, row_id, subrow_id, technical_payload, row_hash, technical_hash, string_hash) VALUES (1, ?1, 0, ?2, ?3, ?4, ?5)",
            params![row_id, Vec::<u8>::new(), row_hash.as_slice(), row_technical_hash.as_slice(), row_string_hash.as_slice()],
            )
            .expect("insert row");
    }
    connection
        .execute(
            "INSERT INTO string_cells (sheet_id, row_id, subrow_id, column_index, macro_text, raw_value, macro_hash, raw_hash) VALUES (1, 42, 0, 0, 'one', NULL, ?1, NULL)",
            params![one_macro_hash.as_slice()],
        )
        .expect("insert first String cell");
    connection
        .execute(
            "INSERT INTO string_cells (sheet_id, row_id, subrow_id, column_index, macro_text, raw_value, macro_hash, raw_hash) VALUES (1, 7, 0, 0, 'two', ?1, ?2, ?3)",
            params![b"raw".as_slice(), two_macro_hash.as_slice(), two_raw_hash.as_slice()],
        )
        .expect("insert second String cell");
    connection
        .execute(
            "INSERT INTO hxs_meta (id, format_version, game_version, language, scope, content_id, snapshot_id, extractor_version, lumina_version, sheet_count, row_count, string_cell_count) VALUES (1, 1, ?1, ?2, 'full', ?3, ?4, 'test', '7.7.0', 1, 2, 2)",
            params![game_version, source_language, content_id, snapshot_id],
        )
        .expect("insert metadata");

    Fixture {
        _directory: directory,
        path,
        one_macro_hash,
        one_row_technical_hash,
        two_macro_hash,
        two_raw_hash,
    }
}

fn macro_hash(value: &str) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-MACRO");
        framed_text(hasher, value);
    })
}

fn raw_hash(value: &[u8]) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-RAW-STRING");
        framed_bytes(hasher, value);
    })
}

fn schema_hash(sheet_name: &str) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-SCHEMA");
        framed_text(hasher, sheet_name);
        hasher.update(0_u32.to_le_bytes());
        hasher.update(0_u32.to_le_bytes());
        hasher.update(0_u32.to_le_bytes());
        hasher.update(1_u32.to_le_bytes());
    })
}

fn row_technical_hash(sheet_name: &str, row_id: u32, subrow_id: u16) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-ROW-TECHNICAL");
        row_identity(hasher, sheet_name, row_id, subrow_id);
    })
}

fn row_string_hash(
    sheet_name: &str,
    row_id: u32,
    subrow_id: u16,
    column_index: u32,
    macro_hash: &[u8; 32],
    raw_hash: Option<&[u8; 32]>,
) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-ROW-STRINGS");
        row_identity(hasher, sheet_name, row_id, subrow_id);
        hasher.update(column_index.to_le_bytes());
        hasher.update(macro_hash);
        hasher.update([u8::from(raw_hash.is_some())]);
        if let Some(raw_hash) = raw_hash {
            hasher.update(raw_hash);
        }
    })
}

fn row_hash(
    sheet_name: &str,
    row_id: u32,
    subrow_id: u16,
    technical_hash: &[u8; 32],
    string_hash: &[u8; 32],
) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-ROW");
        row_identity(hasher, sheet_name, row_id, subrow_id);
        hasher.update(technical_hash);
        hasher.update(string_hash);
    })
}

fn sheet_rows_hash(domain: &str, sheet_name: &str, rows: &[(u32, u16, [u8; 32])]) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(domain.as_bytes());
        framed_text(hasher, sheet_name);
        for (row_id, subrow_id, row_hash) in rows {
            hasher.update(row_id.to_le_bytes());
            hasher.update(u32::from(*subrow_id).to_le_bytes());
            hasher.update(row_hash);
        }
    })
}

fn row_identity(hasher: &mut Sha256, sheet_name: &str, row_id: u32, subrow_id: u16) {
    framed_text(hasher, sheet_name);
    hasher.update(row_id.to_le_bytes());
    hasher.update(u32::from(subrow_id).to_le_bytes());
}

fn framed_text(hasher: &mut Sha256, value: &str) {
    hasher.update(
        u32::try_from(value.len())
            .expect("fixture text fits framing")
            .to_le_bytes(),
    );
    hasher.update(value.as_bytes());
}

fn framed_bytes(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(
        u32::try_from(value.len())
            .expect("fixture bytes fit framing")
            .to_le_bytes(),
    );
    hasher.update(value);
}

fn digest(update: impl FnOnce(&mut Sha256)) -> [u8; 32] {
    let mut hasher = Sha256::new();
    update(&mut hasher);
    hasher.finalize().into()
}

fn hex(bytes: &[u8; 32]) -> String {
    let mut result = String::with_capacity(64);
    for byte in bytes {
        write!(&mut result, "{byte:02x}").expect("writing to a String cannot fail");
    }
    result
}
