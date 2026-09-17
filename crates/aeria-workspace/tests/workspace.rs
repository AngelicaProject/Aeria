use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use aeria_core::{ReviewState, Sha256Hash, SourceBinding, SourceFingerprint};
use aeria_hxs::HxsSnapshot;
use aeria_workspace::{Workspace, WorkspaceError};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

const SYNTHETIC_SCHEMA: &str = include_str!("../../aeria-hxs/tests/fixtures/synthetic_v1.sql");
const APPLICATION_ID: i64 = 0x4841_544c;

struct Fixture {
    path: PathBuf,
    one_macro_hash: [u8; 32],
    one_row_technical_hash: [u8; 32],
    two_macro_hash: [u8; 32],
    two_raw_hash: [u8; 32],
    two_row_technical_hash: [u8; 32],
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
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
fn duplicate_identity_and_binding_are_rejected() {
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

    let moved_binding = SourceBinding::new("Synthetic", 7, 0, 0);
    let moved_fingerprint = SourceFingerprint::new(
        Sha256Hash::from_bytes(fixture.two_macro_hash),
        Some(Sha256Hash::from_bytes(fixture.two_raw_hash)),
        Sha256Hash::from_bytes(fixture.two_row_technical_hash),
    );
    workspace
        .mark_source_changed(id, moved_binding.clone(), moved_fingerprint)
        .expect("known source change");
    assert!(matches!(
        workspace.create_unit_from_hxs(&snapshot, "Synthetic", 7, 0, 0, "second"),
        Err(WorkspaceError::DuplicateSourceBinding { binding }) if binding == moved_binding
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
fn editing_target_resets_review_and_known_source_change_needs_review() {
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

    workspace
        .update_review_state(id, ReviewState::Reviewed)
        .expect("explicit review");
    let new_binding = SourceBinding::new("Synthetic", 7, 0, 0);
    workspace
        .mark_source_changed(
            id,
            new_binding.clone(),
            SourceFingerprint::new(
                Sha256Hash::from_bytes(fixture.two_macro_hash),
                Some(Sha256Hash::from_bytes(fixture.two_raw_hash)),
                Sha256Hash::from_bytes(fixture.two_row_technical_hash),
            ),
        )
        .expect("known source change");
    let unit = workspace.unit(id).expect("unit retains its ID");
    assert_eq!(unit.id(), id);
    assert_eq!(unit.source_binding(), &new_binding);
    assert_eq!(unit.review_state(), ReviewState::NeedsReview);
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

#[allow(clippy::too_many_lines)]
fn write_fixture() -> Fixture {
    let path = std::env::temp_dir().join(format!(
        "aeria-workspace-{}-{}.hxs",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
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
            framed_text(hasher, "en");
            framed_text(hasher, "Synthetic");
            framed_text(hasher, "en");
            hasher.update(schema_hash);
            hasher.update(content_hash);
        }))
    );
    let snapshot_id = format!(
        "sha256:{}",
        hex(&digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-SNAPSHOT-v1");
            framed_text(hasher, "test-game");
            framed_text(hasher, "en");
            framed_text(hasher, &content_id);
        }))
    );

    connection
        .execute(
            "INSERT INTO sheets (id, name, variant, effective_language, column_count, row_count, schema_hash, technical_hash, string_hash, content_hash) VALUES (1, 'Synthetic', 0, 'en', 1, 2, ?1, ?2, ?3, ?4)",
            params![schema_hash.as_slice(), sheet_technical_hash.as_slice(), sheet_string_hash.as_slice(), content_hash.as_slice()],
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
            "INSERT INTO hxs_meta (id, format_version, game_version, language, scope, content_id, snapshot_id, extractor_version, lumina_version, sheet_count, row_count, string_cell_count) VALUES (1, 1, 'test-game', 'en', 'full', ?1, ?2, 'test', '7.7.0', 1, 2, 2)",
            params![content_id, snapshot_id],
        )
        .expect("insert metadata");

    Fixture {
        path,
        one_macro_hash,
        one_row_technical_hash,
        two_macro_hash,
        two_raw_hash,
        two_row_technical_hash,
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
