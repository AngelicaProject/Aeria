use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use aeria_core::{ReviewState, Sha256Hash, SourceBinding};
use aeria_hsp::{
    GuidanceEvidenceInput, GuidanceOccurrence, GuidanceSheet, GuidanceSheetStatus,
    HspComponentDescriptor, HspManifest, HspSourceIdentity, SourceGuidance,
    compute_guidance_bundle_id, compute_package_id, compute_source_evidence_id,
};
use aeria_hxs::HxsSnapshot;
use aeria_workspace::{
    MAX_TRANSLATION_PAGE_SIZE, ProjectSession, ProjectSessionError, TranslationMutationError,
    TranslationReadError, TranslationRowCursor, Workspace, WorkspaceError, WorkspaceStore,
};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use zip::ZipWriter;

const SYNTHETIC_SCHEMA: &str = include_str!("../../aeria-hxs/tests/fixtures/synthetic_v1.sql");
const APPLICATION_ID: i64 = 0x4841_544c;

struct Fixture {
    _directory: TempDir,
    path: PathBuf,
    package_path: PathBuf,
    one_macro_hash: [u8; 32],
    one_row_technical_hash: [u8; 32],
    two_macro_hash: [u8; 32],
    two_raw_hash: [u8; 32],
}

struct ProjectionFixture {
    _directory: TempDir,
    path: PathBuf,
    package_path: PathBuf,
}

struct ProjectionString {
    column_index: u32,
    macro_text: String,
    macro_hash: [u8; 32],
}

struct ProjectionRow {
    row_id: u32,
    row_hash: [u8; 32],
    technical_hash: [u8; 32],
    string_hash: [u8; 32],
    strings: Vec<ProjectionString>,
}

type StringHashSpec = (u32, [u8; 32], Option<[u8; 32]>);

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
    let source_package_path = fixture.package_path.clone();

    let session = ProjectSession::initialize(
        repository.path(),
        &source_package_path,
        repository.path().join("cache"),
        "fr",
    )
    .expect("new project should initialize");
    let source_metadata = session.source().metadata();
    assert_eq!(session.repository_root(), repository.path());
    assert_eq!(session.source_package_path(), source_package_path.as_path());
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
    assert!(!manifest.contains(source_package_path.to_string_lossy().as_ref()));
    assert_managed_files_omit_path(repository.path(), &source_package_path);
    assert_eq!(
        fs::read(&unrelated_path).expect("unrelated file"),
        b"project-owned file\n"
    );

    let expected_metadata = session.workspace().metadata().clone();
    drop(session);
    let reopened = ProjectSession::open(
        repository.path(),
        &source_package_path,
        repository.path().join("cache"),
    )
    .expect("initialized project should reopen");
    assert_eq!(reopened.workspace().metadata(), &expected_metadata);
    assert_eq!(reopened.workspace().units().count(), 0);
    assert_eq!(
        reopened.source_package_path(),
        source_package_path.as_path()
    );
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

    let session = ProjectSession::open(
        repository.path(),
        &fixture.package_path,
        repository.path().join("cache"),
    )
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
    assert_managed_files_omit_path(repository.path(), &fixture.package_path);
    assert_eq!(before, managed_files(repository.path()));
}

#[test]
fn opening_existing_workspace_rejects_a_binding_blocked_by_guidance() {
    let fixture = write_fixture();
    let blocked_package = write_hsp_package(&fixture.path, |_, row_id, _, column_index, _| {
        !(row_id == 42 && column_index == 0)
    });
    let source = HxsSnapshot::open(&fixture.path).expect("fixture should verify");
    let mut workspace = Workspace::from_verified_snapshot(&source, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&source, "Synthetic", 42, 0, 0, "Bonjour")
        .expect("unit");
    let repository = tempfile::tempdir().expect("repository");
    WorkspaceStore::new(repository.path())
        .initialize(&workspace)
        .expect("workspace should initialize");
    let before = managed_files(repository.path());

    let Err(error) = ProjectSession::open(
        repository.path(),
        blocked_package,
        repository.path().join("cache"),
    ) else {
        panic!("blocked existing unit must fail opening")
    };
    assert!(matches!(
        error,
        ProjectSessionError::BlockedWorkspaceUnit {
            translation_unit_id,
            source_binding,
            ..
        } if translation_unit_id == id
            && source_binding == SourceBinding::new("Synthetic", 42, 0, 0)
    ));
    assert_eq!(before, managed_files(repository.path()));
}

#[test]
fn blocked_target_mutation_does_not_create_a_unit_or_write_workspace_files() {
    let fixture = write_fixture();
    let blocked_package = write_hsp_package(&fixture.path, |_, row_id, _, column_index, _| {
        !(row_id == 42 && column_index == 0)
    });
    let repository = tempfile::tempdir().expect("repository");
    let mut session = initialize_project(&repository, &blocked_package, "fr");
    let before = managed_files(repository.path());

    let error = session
        .set_target(&SourceBinding::new("Synthetic", 42, 0, 0), "Bonjour")
        .expect_err("blocked source must be rejected");
    assert!(matches!(
        error,
        TranslationMutationError::SourceNotTranslatable { source_binding }
            if source_binding == SourceBinding::new("Synthetic", 42, 0, 0)
    ));
    assert_eq!(session.workspace().units().count(), 0);
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
    let session = ProjectSession::open(
        repository.path(),
        &fixture.package_path,
        repository.path().join("cache"),
    )
    .expect("session");

    let first = session
        .page_translation_rows("Synthetic", None, 1)
        .expect("first translation page");
    assert_eq!(first.rows.len(), 1);
    assert_eq!(first.rows[0].row_id, 7);
    assert_eq!(first.rows[0].cells[0].source_macro, "two");
    assert!(first.rows[0].cells[0].translation.is_none());
    let after = first.next_after.expect("second page cursor");

    let second = session
        .page_translation_rows("Synthetic", Some(&after), 1)
        .expect("second translation page");
    assert_eq!(second.rows.len(), 1);
    assert_eq!(second.rows[0].row_id, 42);
    assert_eq!(second.rows[0].cells[0].source_macro, "one");
    let overlay = second.rows[0].cells[0]
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
fn translation_rows_follow_guidance_for_context_empty_and_regular_multi_string_rows() {
    let fixture = write_projection_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let session = initialize_project(&repository, &fixture.package_path, "fr");

    let page = session
        .page_translation_rows("Projection", None, 5)
        .expect("row page");
    assert_eq!(page.rows.len(), 3);

    let path_row = &page.rows[0];
    assert_eq!((path_row.row_id, path_row.subrow_id), (1, 0));
    assert_eq!(
        path_row
            .context
            .iter()
            .map(|cell| (cell.column_index, cell.source_macro.as_str()))
            .collect::<Vec<_>>(),
        [(0, "Context field")]
    );
    assert_eq!(path_row.cells.len(), 1);
    assert_eq!(path_row.cells[0].source_binding.column_index(), 1);
    assert_eq!(path_row.cells[0].source_macro, "Greetings and welcome");

    let empty_row = &page.rows[1];
    assert_eq!((empty_row.row_id, empty_row.subrow_id), (2, 0));
    assert_eq!(
        empty_row
            .context
            .iter()
            .map(|cell| (cell.column_index, cell.source_macro.as_str()))
            .collect::<Vec<_>>(),
        [(0, "Empty context")]
    );
    assert_eq!(empty_row.cells.len(), 1);
    assert_eq!(empty_row.cells[0].source_binding.column_index(), 1);
    assert_eq!(empty_row.cells[0].source_macro, "");

    let item_row = &page.rows[2];
    assert_eq!((item_row.row_id, item_row.subrow_id), (4, 0));
    assert!(item_row.context.is_empty());
    assert_eq!(item_row.cells.len(), 4);
    assert_eq!(
        item_row
            .cells
            .iter()
            .map(|cell| cell.source_binding.column_index())
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert_eq!(item_row.cells[0].source_macro, "fire shard");
    assert_eq!(item_row.cells[3].source_macro, "Fire Shard");
}

#[test]
fn context_only_and_empty_rows_advance_the_translation_row_cursor() {
    let fixture = write_projection_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let session = initialize_project(&repository, &fixture.package_path, "fr");

    let first = session
        .page_translation_rows("Projection", None, 1)
        .expect("first row page");
    assert_eq!(
        first.rows.iter().map(|row| row.row_id).collect::<Vec<_>>(),
        [1]
    );
    let mut after = first.next_after;
    let mut visible = vec![1];
    let mut empty_pages = 0;
    while let Some(cursor) = after {
        let page = session
            .page_translation_rows("Projection", Some(&cursor), 1)
            .expect("following row page");
        if page.rows.is_empty() {
            empty_pages += 1;
        } else {
            visible.extend(page.rows.iter().map(|row| row.row_id));
        }
        after = page.next_after;
    }

    assert_eq!(visible, [1, 2, 4]);
    assert_eq!(
        empty_pages, 2,
        "context/empty source rows still advanced paging"
    );
}

#[test]
fn translation_rows_overlay_cells_by_binding_and_preserve_explicit_empty_targets() {
    let fixture = write_projection_fixture();
    let source = HxsSnapshot::open(&fixture.path).expect("source");
    let mut workspace = Workspace::from_verified_snapshot(&source, "fr").expect("workspace");
    let translated_id = workspace
        .create_unit_from_hxs(&source, "Projection", 4, 0, 2, "")
        .expect("explicit empty target unit");
    let unrelated_id = workspace
        .create_unit_from_hxs(&source, "Projection", 1, 0, 1, "Bonjour")
        .expect("path unit");
    let repository = tempfile::tempdir().expect("temporary repository");
    WorkspaceStore::new(repository.path())
        .initialize(&workspace)
        .expect("workspace should initialize");
    let session = open_project(&repository, &fixture.package_path);

    let page = session
        .page_translation_rows(
            "Projection",
            Some(&TranslationRowCursor::new("Projection", 3, 0)),
            2,
        )
        .expect("item row page");
    let row = &page.rows[0];
    assert_eq!(row.row_id, 4);
    assert_eq!(
        row.cells[2]
            .translation
            .as_ref()
            .expect("explicit empty target overlay")
            .translation_unit_id,
        translated_id
    );
    assert_eq!(row.cells[2].translation.as_ref().unwrap().target_macro, "");
    assert!(row.cells[0].translation.is_none());
    assert_eq!(
        page.rows[0].cells.len(),
        4,
        "the unrelated cell-level overlay does not collapse the row"
    );
    let path_row = session
        .page_translation_rows("Projection", None, 1)
        .expect("path row page")
        .rows
        .remove(0);
    assert_eq!(
        path_row.cells[0]
            .translation
            .as_ref()
            .expect("path overlay")
            .translation_unit_id,
        unrelated_id
    );
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
    let session = open_project(&repository, &fixture.package_path);
    let page = session
        .page_translation_rows("Synthetic", None, 2)
        .expect("translation page");

    let empty = &page.rows[0].cells[0];
    let overlay = empty
        .translation
        .as_ref()
        .expect("an explicit empty target still has a unit");
    assert_eq!(overlay.translation_unit_id, empty_id);
    assert_eq!(overlay.target_macro, "");
    assert!(page.rows[1].cells[0].translation.is_none());
}

#[test]
fn translation_read_rejects_invalid_limits_and_cross_sheet_cursors() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    initialize_project(&repository, &fixture.package_path, "fr");
    let session = open_project(&repository, &fixture.package_path);

    assert!(matches!(
        session.page_translation_rows("Synthetic", None, 0),
        Err(TranslationReadError::InvalidPageLimit { limit: 0, max })
            if max == MAX_TRANSLATION_PAGE_SIZE
    ));
    assert!(matches!(
        session.page_translation_rows("Synthetic", None, MAX_TRANSLATION_PAGE_SIZE + 1),
        Err(TranslationReadError::InvalidPageLimit { limit, max })
            if limit == MAX_TRANSLATION_PAGE_SIZE + 1 && max == MAX_TRANSLATION_PAGE_SIZE
    ));
    assert!(matches!(
        session.page_translation_rows(
            "Synthetic",
            Some(&TranslationRowCursor::new("Other", 7, 0)),
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

    let session = open_project(&repository, &fixture.package_path);
    let error = session
        .page_translation_rows(
            "Synthetic",
            Some(&TranslationRowCursor::new("Synthetic", 7, 0)),
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
    let session = initialize_project(&repository, &fixture.package_path, "fr");
    let before = managed_files(repository.path());

    let mut after = None;
    loop {
        let page = session
            .page_translation_rows("Synthetic", after.as_ref(), 1)
            .expect("translation page");
        assert!(page.rows.len() <= 1);
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(before, managed_files(repository.path()));
}

#[test]
fn session_mutations_create_update_and_read_back_the_committed_state() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let source_binding = SourceBinding::new("Synthetic", 42, 0, 0);
    let unrelated_path = repository.path().join("README.md");
    fs::write(&unrelated_path, b"project-owned\n").expect("unrelated file");
    let mut session = initialize_project(&repository, &fixture.package_path, "fr");

    let id = session
        .set_target(&source_binding, "Bonjour")
        .expect("first target should create a unit");
    let unit = session.workspace().unit(id).expect("created unit");
    assert_eq!(unit.source_binding(), &source_binding);
    assert_eq!(unit.target_macro(), "Bonjour");
    assert_eq!(unit.review_state(), ReviewState::Draft);
    assert_eq!(session.workspace().units().count(), 1);

    let page = session
        .page_translation_rows("Synthetic", None, 2)
        .expect("read after write");
    let overlay = page.rows[1].cells[0]
        .translation
        .as_ref()
        .expect("new target is visible immediately");
    assert_eq!(overlay.translation_unit_id, id);
    assert_eq!(overlay.target_macro, "Bonjour");

    session
        .set_review_state(id, ReviewState::Reviewed)
        .expect("review state");
    let before_update = session.workspace().unit(id).expect("unit").clone();
    session
        .set_target(&source_binding, "Salut")
        .expect("existing target should update");
    let updated = session.workspace().unit(id).expect("updated unit");
    assert_eq!(updated.id(), before_update.id());
    assert_eq!(updated.source_binding(), before_update.source_binding());
    assert_eq!(
        updated.source_fingerprint(),
        before_update.source_fingerprint()
    );
    assert_eq!(updated.target_macro(), "Salut");
    assert_eq!(updated.review_state(), ReviewState::Draft);
    assert_eq!(
        fs::read(&unrelated_path).expect("unrelated file"),
        b"project-owned\n"
    );

    drop(session);
    let reopened = open_project(&repository, &fixture.package_path);
    let reopened_unit = reopened.workspace().unit(id).expect("persisted unit");
    assert_eq!(reopened_unit.target_macro(), "Salut");
    assert_eq!(reopened_unit.source_binding(), &source_binding);
}

#[test]
fn empty_target_creates_explicit_sparse_state() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let binding = SourceBinding::new("Synthetic", 7, 0, 0);
    let mut session = initialize_project(&repository, &fixture.package_path, "fr");

    let id = session
        .set_target(&binding, "")
        .expect("empty target should create a unit");
    let page = session
        .page_translation_rows("Synthetic", None, 2)
        .expect("read explicit empty target");
    let overlay = page.rows[0].cells[0]
        .translation
        .as_ref()
        .expect("empty target remains present");
    assert_eq!(overlay.translation_unit_id, id);
    assert_eq!(overlay.target_macro, "");

    drop(session);
    let reopened = open_project(&repository, &fixture.package_path);
    assert_eq!(
        reopened
            .workspace()
            .unit(id)
            .expect("empty unit")
            .target_macro(),
        ""
    );
}

#[test]
fn identical_mutations_do_not_rewrite_canonical_files() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let binding = SourceBinding::new("Synthetic", 42, 0, 0);
    let mut session = initialize_project(&repository, &fixture.package_path, "fr");
    let id = session
        .set_target(&binding, "Bonjour")
        .expect("target should create a unit");
    session.set_note(id, Some("note".to_owned())).expect("note");
    session
        .set_review_state(id, ReviewState::Reviewed)
        .expect("review state");

    let before = managed_files(repository.path());
    session
        .set_target(&binding, "Bonjour")
        .expect("same target is a no-op");
    session
        .set_note(id, Some("note".to_owned()))
        .expect("same note is a no-op");
    session
        .set_review_state(id, ReviewState::Reviewed)
        .expect("same review state is a no-op");
    assert_eq!(before, managed_files(repository.path()));
    assert_eq!(
        session.workspace().unit(id).unwrap().review_state(),
        ReviewState::Reviewed
    );
}

#[test]
fn note_and_review_mutations_preserve_their_existing_semantics() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let binding = SourceBinding::new("Synthetic", 42, 0, 0);
    let mut session = initialize_project(&repository, &fixture.package_path, "fr");
    let id = session
        .set_target(&binding, "Bonjour")
        .expect("target should create a unit");
    session
        .set_review_state(id, ReviewState::Reviewed)
        .expect("review state");

    session
        .set_note(id, Some("note".to_owned()))
        .expect("set note");
    assert_eq!(
        session.workspace().unit(id).unwrap().translator_note(),
        Some("note")
    );
    assert_eq!(
        session.workspace().unit(id).unwrap().review_state(),
        ReviewState::Reviewed
    );
    session.set_note(id, None).expect("clear note");
    assert_eq!(
        session.workspace().unit(id).unwrap().translator_note(),
        None
    );
    assert_eq!(
        session.workspace().unit(id).unwrap().review_state(),
        ReviewState::Reviewed
    );

    session
        .set_review_state(id, ReviewState::NeedsReview)
        .expect("needs review");
    assert_eq!(
        session.workspace().unit(id).unwrap().review_state(),
        ReviewState::NeedsReview
    );
    drop(session);
    let reopened = open_project(&repository, &fixture.package_path);
    let unit = reopened.workspace().unit(id).expect("persisted unit");
    assert_eq!(unit.translator_note(), None);
    assert_eq!(unit.review_state(), ReviewState::NeedsReview);
}

#[test]
fn invalid_or_missing_targets_do_not_change_session_or_files() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let binding = SourceBinding::new("Synthetic", 42, 0, 0);
    let mut session = initialize_project(&repository, &fixture.package_path, "fr");
    let before = managed_files(repository.path());
    assert!(matches!(
        session.set_target(&binding, "<if(1,2,3>"),
        Err(TranslationMutationError::Workspace(
            WorkspaceError::InvalidTarget { .. }
        ))
    ));
    assert!(
        session
            .workspace()
            .unit_by_source_binding(&binding)
            .is_none()
    );
    assert_eq!(before, managed_files(repository.path()));

    assert!(matches!(
        session.set_target(&SourceBinding::new("Synthetic", 42, 0, 1), "target"),
        Err(TranslationMutationError::SourceNotTranslatable { .. })
    ));
    assert_eq!(before, managed_files(repository.path()));

    let id = session
        .set_target(&binding, "Bonjour")
        .expect("valid target should create a unit");
    let before_existing = managed_files(repository.path());
    assert!(matches!(
        session.set_target(&binding, "<if(1,2,3>"),
        Err(TranslationMutationError::Workspace(
            WorkspaceError::InvalidTarget { .. }
        ))
    ));
    assert_eq!(
        session.workspace().unit(id).unwrap().target_macro(),
        "Bonjour"
    );
    assert_eq!(before_existing, managed_files(repository.path()));

    let unknown = aeria_core::TranslationUnitId::from_bytes([0xff; 32]);
    assert!(matches!(
        session.set_note(unknown, Some("note".to_owned())),
        Err(TranslationMutationError::Workspace(WorkspaceError::UnitNotFound { id })) if id == unknown
    ));
    assert!(matches!(
        session.set_review_state(unknown, ReviewState::Reviewed),
        Err(TranslationMutationError::Workspace(WorkspaceError::UnitNotFound { id })) if id == unknown
    ));
    assert_eq!(before_existing, managed_files(repository.path()));
}

#[test]
fn stale_source_fingerprint_blocks_all_ordinary_mutations() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let binding = SourceBinding::new("Synthetic", 42, 0, 0);
    let mut initial = initialize_project(&repository, &fixture.package_path, "fr");
    let id = initial
        .set_target(&binding, "Bonjour")
        .expect("target should create a unit");
    drop(initial);

    let shard = fs::read_dir(repository.path().join(".aeria/units"))
        .expect("unit shards")
        .map(|entry| entry.expect("unit shard entry").path())
        .next()
        .expect("one unit shard");
    let text = fs::read_to_string(&shard).expect("unit shard text");
    let stale = text.replace(
        &format!("\"macroTextHash\":\"{}\"", hex(&fixture.one_macro_hash)),
        &format!("\"macroTextHash\":\"{}\"", "00".repeat(32)),
    );
    fs::write(&shard, stale).expect("stale unit fixture");
    let before = managed_files(repository.path());

    let mut session = open_project(&repository, &fixture.package_path);
    for result in [
        session.set_target(&binding, "Salut").map(|_| ()),
        session.set_note(id, Some("note".to_owned())),
        session.set_review_state(id, ReviewState::Reviewed),
    ] {
        assert!(
            matches!(result, Err(TranslationMutationError::SourceIntegrity { translation_unit_id, source_binding, .. })
            if translation_unit_id == id && *source_binding == binding)
        );
    }
    assert_eq!(before, managed_files(repository.path()));
    assert_eq!(
        session.workspace().unit(id).unwrap().target_macro(),
        "Bonjour"
    );
    assert_eq!(
        session.workspace().unit(id).unwrap().translator_note(),
        None
    );
    assert_eq!(
        session.workspace().unit(id).unwrap().review_state(),
        ReviewState::Draft
    );
}

#[test]
fn only_the_affected_shard_changes() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let mut session = initialize_project(&repository, &fixture.package_path, "fr");
    let first_binding = SourceBinding::new("Synthetic", 42, 0, 0);
    let second_binding = SourceBinding::new("Synthetic", 7, 0, 0);
    let first = session
        .set_target(&first_binding, "one")
        .expect("first target");
    let second = session
        .set_target(&second_binding, "two")
        .expect("second target");
    assert_ne!(first.as_bytes()[0], second.as_bytes()[0]);
    let before = managed_files(repository.path());
    session
        .set_target(&first_binding, "updated")
        .expect("update first target");
    let after = managed_files(repository.path());
    assert_eq!(
        before[&repository.path().join(".aeria/manifest.json")],
        after[&repository.path().join(".aeria/manifest.json")]
    );
    let first_path = repository
        .path()
        .join(".aeria/units")
        .join(format!("{:02x}.jsonl", first.as_bytes()[0]));
    let second_path = repository
        .path()
        .join(".aeria/units")
        .join(format!("{:02x}.jsonl", second.as_bytes()[0]));
    assert_ne!(before[&first_path], after[&first_path]);
    assert_eq!(before[&second_path], after[&second_path]);
}

#[test]
fn rejects_a_different_verified_snapshot_without_modifying_the_workspace() {
    let original_fixture = write_fixture();
    let different_snapshot = write_fixture_with("en", "different-game");
    let repository = tempfile::tempdir().expect("temporary repository");
    initialize_project(&repository, &original_fixture.package_path, "fr");
    let before = managed_files(repository.path());

    let Err(error) = ProjectSession::open(
        repository.path(),
        &different_snapshot.package_path,
        repository.path().join("cache"),
    ) else {
        panic!("different snapshot must be rejected")
    };
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
    initialize_project(&repository, &original_fixture.package_path, "fr");
    let before = managed_files(repository.path());

    let Err(error) = ProjectSession::open(
        repository.path(),
        &different_language.package_path,
        repository.path().join("cache"),
    ) else {
        panic!("different source language must be rejected")
    };
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

    let error = ProjectSession::initialize(
        invalid_source_repository.path(),
        &invalid_source,
        invalid_source_repository.path().join("cache"),
        "fr",
    )
    .err()
    .expect("invalid HXS must be rejected");
    assert!(matches!(error, ProjectSessionError::Source { .. }));
    assert!(!invalid_source_repository.path().join(".aeria").exists());
    let error = ProjectSession::open(
        invalid_source_repository.path(),
        &invalid_source,
        invalid_source_repository.path().join("cache"),
    )
    .err()
    .expect("invalid HXS must be rejected while opening");
    assert!(matches!(error, ProjectSessionError::Source { .. }));

    let invalid_target_repository = tempfile::tempdir().expect("temporary repository");
    let error = ProjectSession::initialize(
        invalid_target_repository.path(),
        &fixture.package_path,
        invalid_target_repository.path().join("cache"),
        " ",
    )
    .err()
    .expect("invalid target language must be rejected");
    assert!(matches!(error, ProjectSessionError::Workspace { .. }));
    assert!(!invalid_target_repository.path().join(".aeria").exists());
}

#[test]
fn initialization_protects_existing_project_state() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    initialize_project(&repository, &fixture.package_path, "fr");
    let before = managed_files(repository.path());

    let Err(error) = ProjectSession::initialize(
        repository.path(),
        &fixture.package_path,
        repository.path().join("cache"),
        "fr",
    ) else {
        panic!("existing project must not be replaced")
    };
    assert!(matches!(
        error,
        ProjectSessionError::Store {
            source: aeria_workspace::WorkspaceStoreError::AlreadyInitialized { .. },
            ..
        }
    ));
    assert_eq!(before, managed_files(repository.path()));
}

fn initialize_project(
    repository: &TempDir,
    package_path: &Path,
    target_language: &str,
) -> ProjectSession {
    ProjectSession::initialize(
        repository.path(),
        package_path,
        repository.path().join("cache"),
        target_language,
    )
    .expect("project should initialize")
}

fn open_project(repository: &TempDir, package_path: &Path) -> ProjectSession {
    ProjectSession::open(
        repository.path(),
        package_path,
        repository.path().join("cache"),
    )
    .expect("project should open")
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

    let package_path = write_hsp_package(&path, |_, _, _, _, _| true);
    Fixture {
        _directory: directory,
        path,
        package_path,
        one_macro_hash,
        one_row_technical_hash,
        two_macro_hash,
        two_raw_hash,
    }
}

#[allow(clippy::too_many_lines)]
fn write_projection_fixture() -> ProjectionFixture {
    let directory = tempfile::tempdir().expect("create projection fixture directory");
    let path = directory.path().join("projection.hxs");
    let connection = Connection::open(&path).expect("create projection fixture database");
    connection
        .execute_batch(SYNTHETIC_SCHEMA)
        .expect("create projection fixture schema");
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = 1; PRAGMA foreign_keys = ON;"
        ))
        .expect("set HXS identity");

    let definitions = vec![
        (
            1,
            ["Context field", "Greetings and welcome", "", ""]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
        (
            2,
            ["Empty context", "", "", ""]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
        (
            3,
            ["", "", "", ""]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
        (
            4,
            [
                "fire shard",
                "fire shards",
                "A tiny crystalline manifestation",
                "Fire Shard",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        ),
        (
            5,
            ["Blocked only", "", "", ""]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
    ];
    let rows = definitions
        .into_iter()
        .map(|(row_id, texts)| {
            let string_hashes = texts
                .iter()
                .map(|text| macro_hash(text))
                .collect::<Vec<_>>();
            let string_hash = row_strings_hash(
                "Projection",
                row_id,
                0,
                &(0..4)
                    .map(|column| (column, string_hashes[column as usize], None))
                    .collect::<Vec<_>>(),
            );
            let technical_hash = row_technical_hash("Projection", row_id, 0);
            let row_hash = row_hash("Projection", row_id, 0, &technical_hash, &string_hash);
            ProjectionRow {
                row_id,
                row_hash,
                technical_hash,
                string_hash,
                strings: texts
                    .into_iter()
                    .enumerate()
                    .map(|(column_index, macro_text)| ProjectionString {
                        column_index: u32::try_from(column_index).expect("column fits"),
                        macro_hash: macro_hash(&macro_text),
                        macro_text,
                    })
                    .collect(),
            }
        })
        .collect::<Vec<_>>();

    let schema_hash =
        schema_hash_for_columns("Projection", &[(0, 0, 1), (1, 4, 1), (2, 8, 1), (3, 12, 1)]);
    let sheet_technical_hash = sheet_rows_hash(
        "HARMONIA-HXS-V1-SHEET-TECHNICAL",
        "Projection",
        &rows
            .iter()
            .map(|row| (row.row_id, 0, row.technical_hash))
            .collect::<Vec<_>>(),
    );
    let sheet_string_hash = sheet_rows_hash(
        "HARMONIA-HXS-V1-SHEET-STRINGS",
        "Projection",
        &rows
            .iter()
            .map(|row| (row.row_id, 0, row.string_hash))
            .collect::<Vec<_>>(),
    );
    let content_hash = digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-SHEET");
        framed_text(hasher, "Projection");
        hasher.update(0_u32.to_le_bytes());
        hasher.update(schema_hash);
        hasher.update(sheet_technical_hash);
        hasher.update(sheet_string_hash);
    });
    let content_id = format!(
        "sha256:{}",
        hex(&digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-CONTENT-v1");
            framed_text(hasher, "en");
            framed_text(hasher, "Projection");
            framed_text(hasher, "en");
            hasher.update(schema_hash);
            hasher.update(content_hash);
        }))
    );
    let snapshot_id = format!(
        "sha256:{}",
        hex(&digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-SNAPSHOT-v1");
            framed_text(hasher, "projection");
            framed_text(hasher, "en");
            framed_text(hasher, &content_id);
        }))
    );

    connection
        .execute(
            "INSERT INTO sheets (id, name, variant, effective_language, column_count, row_count, schema_hash, technical_hash, string_hash, content_hash) VALUES (1, 'Projection', 0, 'en', 4, 5, ?1, ?2, ?3, ?4)",
            params![schema_hash.as_slice(), sheet_technical_hash.as_slice(), sheet_string_hash.as_slice(), content_hash.as_slice()],
        )
        .expect("insert projection sheet");
    for (column_index, offset) in [(0_u32, 0_u32), (1, 4), (2, 8), (3, 12)] {
        connection
            .execute(
                "INSERT INTO columns (sheet_id, column_index, offset, type) VALUES (1, ?1, ?2, 1)",
                params![column_index, offset],
            )
            .expect("insert projection column");
    }
    for row in &rows {
        connection
            .execute(
                "INSERT INTO rows (sheet_id, row_id, subrow_id, technical_payload, row_hash, technical_hash, string_hash) VALUES (1, ?1, 0, ?2, ?3, ?4, ?5)",
                params![row.row_id, Vec::<u8>::new(), row.row_hash.as_slice(), row.technical_hash.as_slice(), row.string_hash.as_slice()],
            )
            .expect("insert projection row");
        for string in &row.strings {
            connection
                .execute(
                    "INSERT INTO string_cells (sheet_id, row_id, subrow_id, column_index, macro_text, raw_value, macro_hash, raw_hash) VALUES (1, ?1, 0, ?2, ?3, NULL, ?4, NULL)",
                    params![row.row_id, string.column_index, string.macro_text, string.macro_hash.as_slice()],
                )
                .expect("insert projection String cell");
        }
    }
    connection
        .execute(
            "INSERT INTO hxs_meta (id, format_version, game_version, language, scope, content_id, snapshot_id, extractor_version, lumina_version, sheet_count, row_count, string_cell_count) VALUES (1, 1, 'projection', 'en', 'full', ?1, ?2, 'test', '7.7.0', 1, 5, 20)",
            params![content_id, snapshot_id],
        )
        .expect("insert projection metadata");

    let package_path = write_hsp_package(&path, |sheet, row_id, _, column_index, _| {
        sheet == "Projection" && matches!((row_id, column_index), (1 | 2, 1) | (4, 0..=3))
    });
    ProjectionFixture {
        _directory: directory,
        path,
        package_path,
    }
}

fn write_hsp_package(
    source_path: &Path,
    allow: impl Fn(&str, u32, u16, u32, &str) -> bool,
) -> PathBuf {
    let snapshot = HxsSnapshot::open(source_path).expect("source fixture verifies");
    let metadata = snapshot.metadata();
    let guidance = build_guidance(&snapshot, &allow);
    let mut guidance_bytes = serde_json::to_vec(&guidance).expect("guidance JSON");
    guidance_bytes.push(b'\n');
    let source_bytes = fs::read(source_path).expect("source bytes");
    let source_component = HspComponentDescriptor {
        id: "source".to_owned(),
        kind: "sourceHxs".to_owned(),
        format_version: 1,
        required: true,
        path: "source/source.hxs".to_owned(),
        size: i64::try_from(source_bytes.len()).expect("source size"),
        sha256: hash_bytes(&source_bytes),
    };
    let guidance_component = HspComponentDescriptor {
        id: "guidance".to_owned(),
        kind: "sourceGuidance".to_owned(),
        format_version: 1,
        required: true,
        path: "guidance/source-guidance.json".to_owned(),
        size: i64::try_from(guidance_bytes.len()).expect("guidance size"),
        sha256: hash_bytes(&guidance_bytes),
    };
    let manifest_without_id = HspManifest {
        format_version: 1,
        package_id: String::new(),
        game_version: metadata.game_version,
        scope: metadata.scope,
        source: HspSourceIdentity {
            language: metadata.source_language,
            content_id: metadata.content_id,
            snapshot_id: metadata.snapshot_id,
        },
        components: vec![guidance_component, source_component],
    };
    let manifest = HspManifest {
        package_id: compute_package_id(&manifest_without_id).expect("package hash"),
        ..manifest_without_id
    };
    let package_path = source_path.with_extension("hsp");
    let file = fs::File::create(&package_path).expect("package file");
    let mut archive = ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    archive
        .start_file("manifest.json", options)
        .expect("manifest entry");
    let mut manifest_bytes = serde_json::to_vec(&manifest).expect("manifest JSON");
    manifest_bytes.push(b'\n');
    archive.write_all(&manifest_bytes).expect("manifest bytes");
    archive
        .start_file("guidance/source-guidance.json", options)
        .expect("guidance entry");
    archive.write_all(&guidance_bytes).expect("guidance bytes");
    archive
        .start_file("source/source.hxs", options)
        .expect("source entry");
    archive.write_all(&source_bytes).expect("source bytes");
    archive.finish().expect("package archive");
    package_path
}

fn build_guidance(
    snapshot: &HxsSnapshot,
    allow: &impl Fn(&str, u32, u16, u32, &str) -> bool,
) -> SourceGuidance {
    let metadata = snapshot.metadata();
    let source_evidence_id = compute_source_evidence_id(snapshot).expect("source evidence");
    let comparison_language = if metadata.source_language == "en" {
        "ja"
    } else {
        "en"
    };
    let mut evidence_inputs = vec![
        GuidanceEvidenceInput {
            language: metadata.source_language.clone(),
            evidence_id: source_evidence_id,
        },
        GuidanceEvidenceInput {
            language: comparison_language.to_owned(),
            evidence_id: format!("sha256:{}", "1".repeat(64)),
        },
    ];
    evidence_inputs.sort_by(|left, right| left.language.cmp(&right.language));

    let mut sheets = snapshot.sheets();
    sheets.sort_by(|left, right| left.name.cmp(&right.name));
    let guidance_sheets = sheets
        .iter()
        .map(|sheet| {
            let mut occurrences = Vec::new();
            let mut after = None;
            loop {
                let page = snapshot
                    .page_string_rows(
                        &sheet.name,
                        after.as_ref(),
                        aeria_hxs::MAX_STRING_ROW_PAGE_SIZE,
                    )
                    .expect("String rows");
                for row in &page.rows {
                    for occurrence in &row.occurrences {
                        let coordinate = &occurrence.fingerprint.coordinate;
                        if allow(
                            &coordinate.sheet_name,
                            coordinate.row_id,
                            coordinate.subrow_id,
                            coordinate.column_index,
                            &occurrence.macro_text,
                        ) {
                            occurrences.push(GuidanceOccurrence {
                                row_id: coordinate.row_id,
                                subrow_id: coordinate.subrow_id,
                                column_index: coordinate.column_index,
                            });
                        }
                    }
                }
                let Some(next) = page.next_after else {
                    break;
                };
                after = Some(next);
            }
            GuidanceSheet {
                name: sheet.name.clone(),
                schema_hash: format!("sha256:{}", sheet.hashes.schema.to_hex()),
                status: GuidanceSheetStatus::Compatible,
                translatable: occurrences,
                incompatibility_reasons: Vec::new(),
            }
        })
        .collect::<Vec<_>>();

    let guidance_without_id = SourceGuidance {
        format_version: 1,
        game_version: metadata.game_version.clone(),
        scope: metadata.scope.clone(),
        bundle_id: String::new(),
        source: aeria_hsp::GuidanceSourceIdentity {
            language: metadata.source_language.clone(),
            content_id: metadata.content_id.clone(),
            snapshot_id: metadata.snapshot_id.clone(),
        },
        evidence_inputs,
        sheets: guidance_sheets,
    };
    SourceGuidance {
        bundle_id: compute_guidance_bundle_id(&guidance_without_id).expect("guidance hash"),
        ..guidance_without_id
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    format!("sha256:{}", hex(&digest))
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

fn schema_hash_for_columns(sheet_name: &str, columns: &[(u32, u32, u32)]) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-SCHEMA");
        framed_text(hasher, sheet_name);
        hasher.update(0_u32.to_le_bytes());
        for (index, offset, type_code) in columns {
            hasher.update(index.to_le_bytes());
            hasher.update(offset.to_le_bytes());
            hasher.update(type_code.to_le_bytes());
        }
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

fn row_strings_hash(
    sheet_name: &str,
    row_id: u32,
    subrow_id: u16,
    cells: &[StringHashSpec],
) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-ROW-STRINGS");
        row_identity(hasher, sheet_name, row_id, subrow_id);
        for (column_index, macro_hash, raw_hash) in cells {
            hasher.update(column_index.to_le_bytes());
            hasher.update(macro_hash);
            hasher.update([u8::from(raw_hash.is_some())]);
            if let Some(raw_hash) = raw_hash {
                hasher.update(raw_hash);
            }
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
