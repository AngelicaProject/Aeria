use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use aeria_core::{DetachReason, ReviewState, Sha256Hash, SourceBinding, SourceStatus};
use aeria_hxs::HxsSnapshot;
use aeria_rebase::{Continuity, UnitUpdateOutcome};
use aeria_workspace::{
    AssistedExpectation, AssistedWriteError, MAX_TRANSLATION_PAGE_SIZE, ProjectSession,
    ProjectSessionError, SourceUpdateRequirement, TranslationMutationError, TranslationReadError,
    TranslationRowCursor, Workspace, WorkspaceError, WorkspaceStore, WorkspaceStoreError,
};
use tempfile::TempDir;

#[path = "support/fixture.rs"]
mod fixture;

use fixture::*;

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
fn open_and_open_from_source_package_produce_equivalent_sessions() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let cache_root = repository.path().join("cache");
    ProjectSession::initialize(repository.path(), &fixture.package_path, &cache_root, "fr")
        .expect("project should initialize");

    let opened = ProjectSession::open(repository.path(), &fixture.package_path, &cache_root)
        .expect("normal constructor should open");
    let source_package = aeria_hsp::SourcePackage::open(&fixture.package_path, &cache_root)
        .expect("source package should validate");
    let opened_from_package =
        ProjectSession::open_from_source_package(repository.path(), source_package)
            .expect("validated-package constructor should open");

    assert_eq!(
        opened.repository_root(),
        opened_from_package.repository_root()
    );
    assert_eq!(
        opened.source_package_path(),
        opened_from_package.source_package_path()
    );
    assert_eq!(
        opened.source_package().package_id(),
        opened_from_package.source_package().package_id()
    );
    assert_eq!(
        opened.workspace().metadata(),
        opened_from_package.workspace().metadata()
    );
    assert_eq!(
        opened.source().metadata().snapshot_id,
        opened_from_package.source().metadata().snapshot_id
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
fn a_permission_loss_requires_an_update_that_detaches_without_losing_the_translation() {
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

    let cache_root = repository.path().join("cache");
    let Err(error) = ProjectSession::open(repository.path(), &blocked_package, &cache_root) else {
        panic!("blocked existing unit must require a source update")
    };
    assert!(matches!(
        error,
        ProjectSessionError::SourceUpdateRequired {
            requirement: SourceUpdateRequirement::PermissionChanged { blocked_units: 1 },
            ..
        }
    ));
    assert_eq!(before, managed_files(repository.path()));

    let package =
        aeria_hsp::SourcePackage::open(&blocked_package, &cache_root).expect("blocked package");
    let preview =
        ProjectSession::preview_source_update(repository.path(), &package).expect("preview");
    assert_eq!(preview.plan.summary.newly_detached, 1);
    assert_eq!(before, managed_files(repository.path()));

    let (session, report) = ProjectSession::open_with_source_update(repository.path(), package)
        .expect("update detaches the blocked unit");
    assert_eq!(report.expect("update applied").plan, preview.plan);
    let unit = session.workspace().unit(id).expect("unit is preserved");
    assert_eq!(
        unit.source_status(),
        SourceStatus::Detached(DetachReason::NotTranslatable)
    );
    assert_eq!(unit.target_macro(), "Bonjour");
    assert_eq!(
        unit.source_binding(),
        &SourceBinding::new("Synthetic", 42, 0, 0)
    );
    assert_eq!(session.detached_units().count(), 1);
    assert!(session.translation_progress().is_empty());
    drop(session);

    let reopened = ProjectSession::open(repository.path(), &blocked_package, &cache_root)
        .expect("updated project opens without another update");
    assert_eq!(reopened.detached_units().count(), 1);
}

#[test]
fn identical_content_from_another_game_version_opens_without_an_update() {
    let original_fixture = write_fixture();
    let hotfix = write_fixture_with("en", "hotfix-game");
    let repository = tempfile::tempdir().expect("temporary repository");
    let mut session = initialize_project(&repository, &original_fixture.package_path, "fr");
    session
        .set_target(&SourceBinding::new("Synthetic", 42, 0, 0), "Bonjour")
        .expect("target");
    drop(session);
    let before = managed_files(repository.path());

    let session = ProjectSession::open(
        repository.path(),
        &hotfix.package_path,
        repository.path().join("cache"),
    )
    .expect("same content from another game version opens directly");
    assert_eq!(session.source().metadata().game_version, "hotfix-game");
    assert_eq!(session.workspace().units().count(), 1);
    assert_eq!(before, managed_files(repository.path()));
}

#[test]
fn a_content_update_rebinds_marks_review_and_is_idempotent_after_interruption() {
    let original_fixture = write_fixture();
    let updated_fixture = write_fixture_with_text("en", "next-game", "uno");
    let repository = tempfile::tempdir().expect("temporary repository");
    let cache_root = repository.path().join("cache");
    let mut session = initialize_project(&repository, &original_fixture.package_path, "fr");
    let changed = session
        .set_target(&SourceBinding::new("Synthetic", 42, 0, 0), "Bonjour")
        .expect("changed unit");
    let unchanged = session
        .set_target(&SourceBinding::new("Synthetic", 7, 0, 0), "Deux")
        .expect("unchanged unit");
    session
        .set_review_state(changed, ReviewState::Reviewed)
        .expect("review");
    session
        .set_review_state(unchanged, ReviewState::Reviewed)
        .expect("review");
    drop(session);
    let before_update = managed_files(repository.path());

    let Err(error) = ProjectSession::open(
        repository.path(),
        &updated_fixture.package_path,
        &cache_root,
    ) else {
        panic!("changed content must require a source update")
    };
    assert!(matches!(
        error,
        ProjectSessionError::SourceUpdateRequired {
            requirement: SourceUpdateRequirement::ContentChanged { .. },
            ..
        }
    ));
    assert_eq!(before_update, managed_files(repository.path()));

    let package = aeria_hsp::SourcePackage::open(&updated_fixture.package_path, &cache_root)
        .expect("updated package");
    let (session, report) = ProjectSession::open_with_source_update(repository.path(), package)
        .expect("update applies");
    let report = report.expect("an update was required");
    assert_eq!(report.plan.summary.unchanged, 1);
    assert_eq!(report.plan.summary.source_changed, 1);
    assert_eq!(report.plan.summary.detached, 0);
    assert!(
        report
            .plan
            .entries()
            .iter()
            .all(|entry| entry.continuity == Some(Continuity::SAME_BINDING)
                && entry.outcome != UnitUpdateOutcome::Detached(DetachReason::RowRemoved))
    );
    let changed_unit = session.workspace().unit(changed).expect("changed unit");
    assert_eq!(changed_unit.target_macro(), "Bonjour");
    assert_eq!(changed_unit.review_state(), ReviewState::NeedsReview);
    let unchanged_unit = session.workspace().unit(unchanged).expect("unchanged unit");
    assert_eq!(unchanged_unit.review_state(), ReviewState::Reviewed);
    assert_eq!(
        session.workspace().metadata().source_content_id(),
        session.source().metadata().content_id
    );
    drop(session);
    let after_update = managed_files(repository.path());

    let manifest_path = repository.path().join(".aeria/manifest.json");
    fs::write(&manifest_path, &before_update[&manifest_path])
        .expect("simulate an update interrupted before the manifest");
    let package = aeria_hsp::SourcePackage::open(&updated_fixture.package_path, &cache_root)
        .expect("updated package");
    let (_, report) = ProjectSession::open_with_source_update(repository.path(), package)
        .expect("interrupted update is planned again");
    assert_eq!(
        report
            .expect("update re-applied")
            .plan
            .summary
            .changed_units,
        0
    );
    assert_eq!(after_update, managed_files(repository.path()));

    ProjectSession::open(
        repository.path(),
        &updated_fixture.package_path,
        &cache_root,
    )
    .expect("updated project opens");
}

#[test]
fn a_keyed_sheet_update_moves_translations_with_their_lines() {
    let dialogue = |_: &str, _: u32, _: u16, column_index: u32, _: &str| column_index == 1;
    let original = write_projection_fixture_with(
        "quest-game",
        &[
            (1, ["TEXT_Q_001", "Hello", "", ""]),
            (2, ["TEXT_Q_002", "Goodbye", "", ""]),
            (3, ["TEXT_Q_003", "Removed line", "", ""]),
        ],
        dialogue,
    );
    let patched = write_projection_fixture_with(
        "quest-patch",
        &[
            (1, ["TEXT_Q_000", "Inserted", "", ""]),
            (2, ["TEXT_Q_001", "Hello", "", ""]),
            (3, ["TEXT_Q_002", "Goodbye!", "", ""]),
            (4, ["TEXT_Q_004", "Replacement", "", ""]),
        ],
        dialogue,
    );
    let repository = tempfile::tempdir().expect("temporary repository");
    let cache_root = repository.path().join("cache");
    let mut session = initialize_project(&repository, &original.package_path, "fr");
    let hello = session
        .set_target(&SourceBinding::new("Projection", 1, 0, 1), "Bonjour")
        .expect("hello");
    let goodbye = session
        .set_target(&SourceBinding::new("Projection", 2, 0, 1), "Au revoir")
        .expect("goodbye");
    let removed = session
        .set_target(&SourceBinding::new("Projection", 3, 0, 1), "Supprimé")
        .expect("removed");
    session
        .set_review_state(hello, ReviewState::Reviewed)
        .expect("review");
    assert!(
        [hello, goodbye, removed].iter().all(|id| session
            .workspace()
            .unit(*id)
            .is_some_and(|unit| unit.source_row_key().is_some())),
        "units in a keyed sheet record their row key"
    );
    drop(session);

    let package =
        aeria_hsp::SourcePackage::open(&patched.package_path, &cache_root).expect("package");
    let (session, report) = ProjectSession::open_with_source_update(repository.path(), package)
        .expect("update applies");
    let report = report.expect("update required");
    assert_eq!(report.plan.summary.row_moved, 2);
    let hello_unit = session.workspace().unit(hello).expect("hello unit");
    assert_eq!(
        hello_unit.source_binding(),
        &SourceBinding::new("Projection", 2, 0, 1)
    );
    assert_eq!(hello_unit.review_state(), ReviewState::Reviewed);
    let goodbye_unit = session.workspace().unit(goodbye).expect("goodbye unit");
    assert_eq!(
        goodbye_unit.source_binding(),
        &SourceBinding::new("Projection", 3, 0, 1)
    );
    assert_eq!(goodbye_unit.review_state(), ReviewState::NeedsReview);
    let removed_unit = session.workspace().unit(removed).expect("removed unit");
    assert_eq!(
        removed_unit.source_status(),
        SourceStatus::Detached(DetachReason::RowRemoved)
    );
    assert_eq!(removed_unit.target_macro(), "Supprimé");
    drop(session);

    ProjectSession::open(repository.path(), &patched.package_path, &cache_root)
        .expect("updated project opens");
}

#[test]
fn a_workspace_format_v1_project_is_migrated_by_a_source_update() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let cache_root = repository.path().join("cache");
    let mut session = initialize_project(&repository, &fixture.package_path, "fr");
    let id = session
        .set_target(&SourceBinding::new("Synthetic", 42, 0, 0), "Bonjour")
        .expect("unit");
    let snapshot_id = session.source().metadata().snapshot_id;
    drop(session);
    let current = managed_files(repository.path());

    let manifest_path = repository.path().join(".aeria/manifest.json");
    let manifest = String::from_utf8(current[&manifest_path].clone()).expect("UTF-8");
    let legacy_manifest = manifest
        .replace("\"formatVersion\": 2", "\"formatVersion\": 1")
        .replace(
            "\"\n}\n",
            &format!("\",\n  \"snapshotId\": \"{snapshot_id}\"\n}}\n"),
        );
    fs::write(&manifest_path, legacy_manifest).expect("v1 manifest");
    let shard_path = repository
        .path()
        .join(".aeria/units")
        .join(format!("{:02x}.jsonl", id.as_bytes()[0]));
    let shard = String::from_utf8(current[&shard_path].clone()).expect("UTF-8");
    let layout_start = shard.find(",\"sourceLayout\":").expect("layout field");
    let layout_end = shard.find(",\"targetMacro\":").expect("target field");
    let legacy_shard = format!("{}{}", &shard[..layout_start], &shard[layout_end..])
        .replace("\"sourceStatus\":\"bound\",", "");
    fs::write(&shard_path, legacy_shard).expect("v1 shard");

    let Err(error) = ProjectSession::open(repository.path(), &fixture.package_path, &cache_root)
    else {
        panic!("v1 must be migrated before editing")
    };
    assert!(matches!(
        error,
        ProjectSessionError::SourceUpdateRequired {
            requirement: SourceUpdateRequirement::FormatMigration { version: 1 },
            ..
        }
    ));
    assert!(matches!(
        WorkspaceStore::new(repository.path()).load(),
        Err(WorkspaceStoreError::MigrationRequired { version: 1, .. })
    ));

    let package =
        aeria_hsp::SourcePackage::open(&fixture.package_path, &cache_root).expect("package");
    let (_, report) = ProjectSession::open_with_source_update(repository.path(), package)
        .expect("v1 is migrated");
    let report = report.expect("migration applied");
    assert_eq!(report.previous_format_version, 1);
    assert_eq!(report.plan.summary.unchanged, 1);
    assert_eq!(current, managed_files(repository.path()));
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
fn a_stale_sparse_unit_fingerprint_is_refused_on_open_and_reconciled_without_losing_it() {
    let fixture = write_fixture();
    let source = HxsSnapshot::open(&fixture.path).expect("source");
    let mut workspace = Workspace::from_verified_snapshot(&source, "fr").expect("workspace");
    let unit_id = workspace
        .create_unit_from_hxs(&source, "Synthetic", 42, 0, 0, "Bonjour")
        .expect("unit");
    workspace
        .update_review_state(unit_id, ReviewState::Reviewed)
        .expect("review");
    let repository = tempfile::tempdir().expect("temporary repository");
    WorkspaceStore::new(repository.path())
        .initialize(&workspace)
        .expect("workspace should initialize");

    let shard = fs::read_dir(repository.path().join(".aeria/units"))
        .expect("unit shards")
        .map(|entry| entry.expect("unit shard entry").path())
        .next()
        .expect("one unit shard");
    let text = fs::read_to_string(&shard).expect("unit shard text");
    let persisted_hash = hex(&fixture.one_macro_hash);
    let stale_hash = "00".repeat(32);
    let updated = text.replace(
        &format!("\"macroTextHash\":\"{persisted_hash}\""),
        &format!("\"macroTextHash\":\"{stale_hash}\""),
    );
    assert_ne!(updated, text, "test fixture must change the persisted hash");
    fs::write(&shard, updated).expect("stale unit fixture");
    let before_open = managed_files(repository.path());

    // The unit is never read, overlaid, or mutated as if it were current.
    let Err(error) = ProjectSession::open(
        repository.path(),
        &fixture.package_path,
        repository.path().join("cache"),
    ) else {
        panic!("a stale unit must not open as current");
    };
    assert!(matches!(
        error,
        ProjectSessionError::SourceUpdateRequired {
            requirement: SourceUpdateRequirement::SourceFactsMismatch { units: 1 },
            ..
        }
    ));
    assert_eq!(before_open, managed_files(repository.path()));

    let package =
        aeria_hsp::SourcePackage::open(&fixture.package_path, repository.path().join("cache"))
            .expect("package");
    let (session, report) = ProjectSession::open_with_source_update(repository.path(), package)
        .expect("reconciliation");
    assert_eq!(report.expect("reconciled").plan.summary.source_changed, 1);
    let unit = session.workspace().unit(unit_id).expect("unit kept");
    assert_eq!(unit.target_macro(), "Bonjour");
    assert_eq!(unit.review_state(), ReviewState::NeedsReview);
    assert_eq!(
        unit.source_binding(),
        &SourceBinding::new("Synthetic", 42, 0, 0)
    );
    let page = session
        .page_translation_rows(
            "Synthetic",
            Some(&TranslationRowCursor::new("Synthetic", 7, 0)),
            2,
        )
        .expect("reconciled unit reads");
    let overlay = page.rows[0].cells[0].translation.as_ref().expect("overlay");
    assert_eq!(overlay.translation_unit_id, unit_id);
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
fn assisted_targets_follow_the_structure_policy_and_compare_and_set() {
    let fixture = write_fixture_with_text("en", "test-game", "Hi <pcname(lnum1)>!");
    let repository = tempfile::tempdir().expect("temporary repository");
    let binding = SourceBinding::new("Synthetic", 42, 0, 0);
    let mut session = initialize_project(&repository, &fixture.package_path, "ru");
    assert_eq!(
        session.source_macro(&binding).expect("source"),
        "Hi <pcname(lnum1)>!"
    );
    let untranslated = session.assisted_state(&binding);
    assert_eq!(
        untranslated,
        AssistedExpectation {
            target: None,
            review_state: None
        }
    );
    let files_before = managed_files(repository.path());

    let dropped = session
        .set_assisted_target(&binding, "Привет!", &untranslated, false)
        .expect_err("a dropped runtime value is refused");
    assert!(matches!(dropped, AssistedWriteError::Structure { .. }));
    assert_eq!(managed_files(repository.path()), files_before);

    let id = session
        .set_assisted_target(
            &binding,
            "Привет, <pcname(lnum1)>, <pcname(lnum1)>!",
            &untranslated,
            false,
        )
        .expect("a repeated runtime value is allowed");
    let written = session.assisted_state(&binding);
    assert_eq!(written.review_state, Some(ReviewState::Draft));

    let stale = session
        .set_assisted_target(
            &binding,
            "Здравствуй, <pcname(lnum1)>!",
            &untranslated,
            false,
        )
        .expect_err("the unit changed since the translation was produced");
    assert!(matches!(stale, AssistedWriteError::Conflict { ref current } if current == &written));

    session
        .set_review_state(id, ReviewState::Reviewed)
        .expect("review");
    let reviewed = session.assisted_state(&binding);
    assert!(matches!(
        session.set_assisted_target(&binding, "Здравствуй, <pcname(lnum1)>!", &reviewed, false),
        Err(AssistedWriteError::Reviewed)
    ));
    session
        .set_assisted_target(&binding, "Здравствуй, <pcname(lnum1)>!", &reviewed, true)
        .expect("an approved replacement of a reviewed string");
    assert_eq!(
        session.assisted_state(&binding),
        AssistedExpectation {
            target: Some("Здравствуй, <pcname(lnum1)>!".to_owned()),
            review_state: Some(ReviewState::Draft),
        }
    );
    assert!(matches!(
        session.source_macro(&SourceBinding::new("Synthetic", 42, 0, 9)),
        Err(TranslationMutationError::SourceNotTranslatable { .. })
    ));
}

#[test]
fn empty_and_whitespace_targets_are_rejected_without_mutating_workspace_or_files() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let binding = SourceBinding::new("Synthetic", 42, 0, 0);
    let mut session = initialize_project(&repository, &fixture.package_path, "fr");
    let before = managed_files(repository.path());

    for target in ["", " \t\n", "\u{2003}"] {
        assert!(matches!(
            session.set_target(&binding, target),
            Err(TranslationMutationError::EmptyTarget)
        ));
        assert!(
            session
                .workspace()
                .unit_by_source_binding(&binding)
                .is_none()
        );
        assert_eq!(before, managed_files(repository.path()));
    }

    let id = session
        .set_target(&binding, "Bonjour")
        .expect("non-empty target should create a unit");
    let before_existing = managed_files(repository.path());
    for target in ["", " \t\n", "\u{2003}"] {
        assert!(matches!(
            session.set_target(&binding, target),
            Err(TranslationMutationError::EmptyTarget)
        ));
        assert_eq!(
            session
                .workspace()
                .unit(id)
                .expect("unit remains")
                .target_macro(),
            "Bonjour"
        );
        assert_eq!(before_existing, managed_files(repository.path()));
    }
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
fn files_changed_behind_an_open_session_block_mutations_until_reloaded() {
    let fixture = write_fixture();
    let repository = tempfile::tempdir().expect("temporary repository");
    let binding = SourceBinding::new("Synthetic", 42, 0, 0);
    let mut session = initialize_project(&repository, &fixture.package_path, "fr");
    let id = session
        .set_target(&binding, "Bonjour")
        .expect("target should create a unit");

    // Another process (for example a Git merge) replaces the shard with a
    // stale version of the unit while this session is open.
    let shard = fs::read_dir(repository.path().join(".aeria/units"))
        .expect("unit shards")
        .map(|entry| entry.expect("unit shard entry").path())
        .next()
        .expect("one unit shard");
    let text = fs::read_to_string(&shard).expect("unit shard text");
    let stale = text
        .replace(
            &format!("\"macroTextHash\":\"{}\"", hex(&fixture.one_macro_hash)),
            &format!("\"macroTextHash\":\"{}\"", "00".repeat(32)),
        )
        .replace("Bonjour", "Salut");
    fs::write(&shard, stale).expect("stale unit fixture");
    let before = managed_files(repository.path());

    for result in [
        session.set_target(&binding, "Coucou").map(|_| ()),
        session.set_note(id, Some("note".to_owned())),
        session.set_review_state(id, ReviewState::Reviewed),
    ] {
        assert!(
            result.is_err(),
            "a mutation must not overwrite unseen files"
        );
    }
    assert_eq!(before, managed_files(repository.path()));
    assert_eq!(
        session.workspace().unit(id).unwrap().target_macro(),
        "Bonjour"
    );

    assert!(matches!(
        session.reload_workspace(),
        Err(ProjectSessionError::SourceUpdateRequired {
            requirement: SourceUpdateRequirement::SourceFactsMismatch { units: 1 },
            ..
        })
    ));
    session
        .reload_and_reconcile_workspace()
        .expect("reconcile")
        .expect("reconciled");
    let unit = session.workspace().unit(id).expect("unit kept");
    assert_eq!(unit.target_macro(), "Salut", "the on-disk edit is kept");
    assert_eq!(unit.review_state(), ReviewState::NeedsReview);
    session
        .set_target(&binding, "Coucou")
        .expect("mutations work again after reconciliation");
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
