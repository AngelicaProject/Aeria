use std::fmt::Write as _;
use std::path::PathBuf;

use aeria_core::{ReviewState, Sha256Hash, SourceBinding, SourceFingerprint, TranslationUnit};
use aeria_hxs::HxsSnapshot;
use aeria_rebase::{MatchEvidence, RebaseError, RebaseOutcome, RebasePlanner, plan_rebase};
use aeria_workspace::Workspace;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const SYNTHETIC_SCHEMA: &str = include_str!("../../aeria-hxs/tests/fixtures/synthetic_v1.sql");
const APPLICATION_ID: i64 = 0x4841_544c;

#[derive(Clone)]
struct CellSpec {
    column_index: u32,
    macro_text: String,
    raw_value: Option<Vec<u8>>,
}

#[derive(Clone)]
struct RowSpec {
    row_id: u32,
    subrow_id: u16,
    technical: u8,
    cells: Vec<CellSpec>,
}

#[derive(Clone)]
struct SnapshotSpec {
    game_version: String,
    source_language: String,
    scope: String,
    sheet_name: String,
    rows: Vec<RowSpec>,
    reverse_insertion: bool,
}

struct Fixture {
    _directory: TempDir,
    path: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn identical_snapshot_is_unchanged_and_planning_is_pure_and_deterministic() {
    let spec = snapshot("old", vec![row(1, "Привет", Some(b"raw".to_vec()), &[1])]);
    let old_fixture = write_snapshot(&spec);
    let new_fixture = write_snapshot(&spec);
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "<future(1)>target")
        .expect("unit");
    workspace
        .update_review_state(id, ReviewState::Reviewed)
        .expect("review state");
    let before = workspace.clone();

    let first =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    let second = RebasePlanner::plan(workspace.metadata(), workspace.units(), &old, &new)
        .expect("same plan succeeds");

    assert_eq!(first, second);
    assert_eq!(workspace, before);
    assert_eq!(first.summary.unchanged, 1);
    assert_eq!(first.unit_entries[0].translation_unit_id, id);
    assert_eq!(first.unit_entries[0].outcome, RebaseOutcome::Unchanged);
    assert_eq!(
        first.unit_entries[0].matching_evidence,
        MatchEvidence::SameBinding
    );
}

#[test]
fn same_binding_fingerprint_changes_are_source_changed() {
    for changed in [
        ChangedField::Macro,
        ChangedField::Raw,
        ChangedField::RowTechnical,
    ] {
        let old_spec = snapshot("old", vec![row(1, "one", Some(b"raw".to_vec()), &[1])]);
        let mut new_row = row(1, "one", Some(b"raw".to_vec()), &[1]);
        match changed {
            ChangedField::Macro => new_row.cells[0].macro_text = "two".into(),
            ChangedField::Raw => new_row.cells[0].raw_value = Some(b"changed".to_vec()),
            ChangedField::RowTechnical => new_row.technical = 9,
        }
        let new_spec = snapshot("new", vec![new_row]);
        let old_fixture = write_snapshot(&old_spec);
        let new_fixture = write_snapshot(&new_spec);
        let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
        let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
        let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
        let id = workspace
            .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
            .expect("unit");
        let plan = plan_rebase(workspace.metadata(), workspace.units(), &old, &new)
            .expect("plan succeeds");
        let entry = &plan.unit_entries[0];
        assert_eq!(entry.translation_unit_id, id);
        assert_eq!(entry.outcome, RebaseOutcome::SourceChanged);
        assert_eq!(entry.matching_evidence, MatchEvidence::SameBinding);
        assert_eq!(
            entry.proposed_source_binding,
            Some(SourceBinding::new("台詞", 1, 0, 0))
        );
    }
}

#[test]
fn unique_full_fingerprint_relocation_preserves_id() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row(1, "one", Some(b"raw".to_vec()), &[1])],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row_at(1, 2, "one", Some(b"raw".to_vec()), &[1])],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
        .expect("unit");

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    let entry = &plan.unit_entries[0];
    assert_eq!(entry.translation_unit_id, id);
    assert_eq!(entry.outcome, RebaseOutcome::Relocated);
    assert_eq!(entry.matching_evidence, MatchEvidence::CompleteFingerprint);
    assert_eq!(
        entry.proposed_source_binding,
        Some(SourceBinding::new("台詞", 1, 0, 2))
    );
}

#[test]
fn exact_content_relocation_with_changed_context_is_source_changed() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row(1, "opaque <future(1)>", None, &[1])],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row(9, "opaque <future(1)>", None, &[9])],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
        .expect("unit");

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    let entry = &plan.unit_entries[0];
    assert_eq!(entry.translation_unit_id, id);
    assert_eq!(entry.outcome, RebaseOutcome::SourceChanged);
    assert_eq!(entry.matching_evidence, MatchEvidence::UniqueMacroText);
    assert_eq!(
        entry.proposed_source_binding,
        Some(SourceBinding::new("台詞", 9, 0, 0))
    );
}

#[test]
fn exact_macro_and_raw_relocation_with_changed_context_is_source_changed() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row(1, "same", Some(b"raw".to_vec()), &[1])],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row_at(9, 2, "same", Some(b"raw".to_vec()), &[9])],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
        .expect("unit");

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    let entry = &plan.unit_entries[0];
    assert_eq!(entry.outcome, RebaseOutcome::SourceChanged);
    assert_eq!(entry.matching_evidence, MatchEvidence::MacroAndRawValue);
    assert_eq!(
        entry.proposed_source_binding,
        Some(SourceBinding::new("台詞", 9, 0, 2))
    );
}

#[test]
fn duplicate_candidates_and_competing_units_remain_ambiguous() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row(1, "same", None, &[1]), row(2, "same", None, &[2])],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row(9, "same", None, &[9]), row(10, "same", None, &[10])],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "one")
        .expect("first unit");
    workspace
        .create_unit_from_hxs(&old, "台詞", 2, 0, 0, "two")
        .expect("second unit");

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    assert_eq!(plan.summary.ambiguous, 2);
    assert!(
        plan.unit_entries
            .iter()
            .all(|entry| entry.outcome == RebaseOutcome::Ambiguous)
    );
    assert!(
        plan.unit_entries
            .iter()
            .all(|entry| entry.candidate_count == 2)
    );
    assert_eq!(
        plan.unit_entries[0].candidate_bindings,
        vec![
            SourceBinding::new("台詞", 9, 0, 0),
            SourceBinding::new("台詞", 10, 0, 0)
        ]
    );
    assert_eq!(
        plan.unit_entries[1].candidate_bindings,
        plan.unit_entries[0].candidate_bindings
    );
}

#[test]
fn duplicate_heavy_macro_index_is_bounded_and_conservative() {
    let old_fixture = write_snapshot(&snapshot("old", vec![row(1, "repeated", None, &[1])]));
    let new_rows = (10..110)
        .map(|row_id| {
            row(
                row_id,
                "repeated",
                None,
                &[u8::try_from(row_id).expect("duplicate test row fits in a byte")],
            )
        })
        .collect();
    let new_fixture = write_snapshot(&snapshot("new", new_rows));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
        .expect("unit");

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    let entry = &plan.unit_entries[0];
    assert_eq!(entry.outcome, RebaseOutcome::Ambiguous);
    assert_eq!(entry.candidate_count, 100);
    assert_eq!(entry.candidate_bindings.len(), 64);
    assert!(entry.candidates_truncated);
}

#[test]
fn equivalent_physical_insertion_order_produces_the_same_plan() {
    let old_spec = snapshot(
        "old",
        vec![row(1, "one", None, &[1]), row(2, "two", None, &[2])],
    );
    let mut new_spec = snapshot(
        "new",
        vec![row(9, "one", None, &[9]), row(10, "two", None, &[10])],
    );
    new_spec.reverse_insertion = true;
    let old_fixture = write_snapshot(&old_spec);
    let new_fixture = write_snapshot(&new_spec);
    let equivalent_new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row(10, "two", None, &[10]), row(9, "one", None, &[9])],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let equivalent_new = HxsSnapshot::open(&equivalent_new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "one")
        .expect("first unit");
    workspace
        .create_unit_from_hxs(&old, "台詞", 2, 0, 0, "two")
        .expect("second unit");

    let first =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    let second = plan_rebase(
        workspace.metadata(),
        workspace.units(),
        &old,
        &equivalent_new,
    )
    .expect("equivalent plan succeeds");
    assert_eq!(first, second);
}

#[test]
fn same_binding_claims_before_relocation_and_competing_units_never_duplicate() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row(1, "first", None, &[1]), row(2, "second", None, &[2])],
    ));
    let new_fixture = write_snapshot(&snapshot("new", vec![row(1, "second", None, &[1])]));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "first")
        .expect("first unit");
    workspace
        .create_unit_from_hxs(&old, "台詞", 2, 0, 0, "second")
        .expect("second unit");

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    assert_eq!(plan.summary.source_changed, 1);
    assert_eq!(plan.summary.ambiguous, 1);
    let bindings: Vec<_> = plan
        .unit_entries
        .iter()
        .filter_map(|entry| entry.proposed_source_binding.as_ref())
        .collect();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0], &SourceBinding::new("台詞", 1, 0, 0));
}

#[test]
fn baseline_and_snapshot_preconditions_are_errors() {
    let old_fixture = write_snapshot(&snapshot("old", vec![row(1, "one", None, &[1])]));
    let new_fixture = write_snapshot(&snapshot("new", vec![row(1, "one", None, &[1])]));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
        .expect("unit");

    let mut wrong_language_metadata = workspace.metadata().clone();
    wrong_language_metadata = aeria_core::WorkspaceMetadata::new(
        "ja",
        wrong_language_metadata.target_language(),
        wrong_language_metadata.source_content_id(),
        wrong_language_metadata.source_snapshot_id(),
    )
    .expect("metadata");
    assert!(matches!(
        plan_rebase(&wrong_language_metadata, workspace.units(), &old, &new),
        Err(RebaseError::OldSourceLanguageMismatch { .. })
    ));

    let wrong_content_metadata = aeria_core::WorkspaceMetadata::new(
        "en",
        "fr",
        "wrong-content",
        workspace.metadata().source_snapshot_id(),
    )
    .expect("metadata");
    assert!(matches!(
        plan_rebase(&wrong_content_metadata, workspace.units(), &old, &new),
        Err(RebaseError::OldContentIdMismatch { .. })
    ));

    let new_language_fixture = write_snapshot(&snapshot_with_language(
        "new-language",
        "ja",
        vec![row(1, "one", None, &[1])],
    ));
    let new_language = HxsSnapshot::open(&new_language_fixture.path).expect("new language HXS");
    assert!(matches!(
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new_language),
        Err(RebaseError::NewSourceLanguageMismatch { .. })
    ));

    let changed_old_fixture =
        write_snapshot(&snapshot("changed-old", vec![row(1, "one", None, &[1])]));
    let changed_old = HxsSnapshot::open(&changed_old_fixture.path).expect("changed old HXS");
    assert!(matches!(
        plan_rebase(workspace.metadata(), workspace.units(), &changed_old, &new),
        Err(RebaseError::OldSnapshotIdMismatch { .. })
    ));

    let bad_fingerprint = SourceFingerprint::new(
        Sha256Hash::from_bytes([0xa5; 32]),
        None,
        Sha256Hash::from_bytes([0x5a; 32]),
    );
    let bad_binding = SourceBinding::new("台詞", 1, 0, 0);
    let bad_id = aeria_core::TranslationUnitId::derive("en", &bad_binding, &bad_fingerprint)
        .expect("test identity");
    let bad_unit = TranslationUnit::new(bad_id, bad_binding, bad_fingerprint, "target");
    assert!(matches!(
        plan_rebase(workspace.metadata(), [&bad_unit], &old, &new),
        Err(RebaseError::OldFingerprintMismatch { .. })
    ));
}

#[derive(Clone, Copy)]
enum ChangedField {
    Macro,
    Raw,
    RowTechnical,
}

fn snapshot(game_version: &str, rows: Vec<RowSpec>) -> SnapshotSpec {
    snapshot_with_language(game_version, "en", rows)
}

fn snapshot_with_language(
    game_version: &str,
    source_language: &str,
    rows: Vec<RowSpec>,
) -> SnapshotSpec {
    SnapshotSpec {
        game_version: game_version.into(),
        source_language: source_language.into(),
        scope: "full".into(),
        sheet_name: "台詞".into(),
        rows,
        reverse_insertion: false,
    }
}

fn row(row_id: u32, macro_text: &str, raw_value: Option<Vec<u8>>, technical: &[u8]) -> RowSpec {
    row_at(row_id, 0, macro_text, raw_value, technical)
}

fn row_at(
    row_id: u32,
    column_index: u32,
    macro_text: &str,
    raw_value: Option<Vec<u8>>,
    technical: &[u8],
) -> RowSpec {
    RowSpec {
        row_id,
        subrow_id: 0,
        technical: technical.first().copied().unwrap_or_default(),
        cells: vec![CellSpec {
            column_index,
            macro_text: macro_text.into(),
            raw_value,
        }],
    }
}

#[allow(clippy::too_many_lines)]
fn write_snapshot(spec: &SnapshotSpec) -> Fixture {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("snapshot.hxs");
    let connection = Connection::open(&path).expect("fixture database");
    connection.execute_batch(SYNTHETIC_SCHEMA).expect("schema");
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = 1; PRAGMA foreign_keys = ON;"
        ))
        .expect("identity");

    let mut string_columns: Vec<_> = spec
        .rows
        .iter()
        .flat_map(|row| row.cells.iter().map(|cell| cell.column_index))
        .collect();
    string_columns.sort_unstable();
    string_columns.dedup();
    let mut built_rows: Vec<_> = spec
        .rows
        .iter()
        .map(|row| build_row(spec, row, &string_columns))
        .collect();
    built_rows.sort_by_key(|row| (row.spec.row_id, row.spec.subrow_id));
    let schema_hash = schema_hash(&spec.sheet_name, &string_columns);
    let technical_hash = sheet_rows_hash(
        "HARMONIA-HXS-V1-SHEET-TECHNICAL",
        &spec.sheet_name,
        &built_rows,
        |row| &row.technical_hash,
    );
    let string_hash = sheet_rows_hash(
        "HARMONIA-HXS-V1-SHEET-STRINGS",
        &spec.sheet_name,
        &built_rows,
        |row| &row.string_hash,
    );
    let content_hash = digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-SHEET");
        framed_text(hasher, &spec.sheet_name);
        hasher.update(0_u32.to_le_bytes());
        hasher.update(schema_hash);
        hasher.update(technical_hash);
        hasher.update(string_hash);
    });
    let content_id = format!(
        "sha256:{}",
        hex(&digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-CONTENT-v1");
            framed_text(hasher, &spec.source_language);
            framed_text(hasher, &spec.sheet_name);
            framed_text(hasher, &spec.source_language);
            hasher.update(schema_hash);
            hasher.update(content_hash);
        }))
    );
    let snapshot_id = format!(
        "sha256:{}",
        hex(&digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-SNAPSHOT-v1");
            framed_text(hasher, &spec.game_version);
            framed_text(hasher, &spec.source_language);
            framed_text(hasher, &content_id);
        }))
    );

    connection
        .execute(
            "INSERT INTO sheets (id, name, variant, effective_language, column_count, row_count, schema_hash, technical_hash, string_hash, content_hash) VALUES (1, ?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                spec.sheet_name,
                spec.source_language,
                i64::try_from(string_columns.len() + 1).expect("column count"),
                i64::try_from(built_rows.len()).expect("row count"),
                schema_hash.as_slice(),
                technical_hash.as_slice(),
                string_hash.as_slice(),
                content_hash.as_slice()
            ],
        )
        .expect("sheet");
    connection
        .execute(
            "INSERT INTO columns (sheet_id, column_index, offset, type) VALUES (1, 1, 4, 14)",
            [],
        )
        .expect("String column");
    for column_index in &string_columns {
        connection
            .execute(
                "INSERT INTO columns (sheet_id, column_index, offset, type) VALUES (1, ?1, ?2, 1)",
                params![column_index, column_index * 4],
            )
            .expect("String column");
    }
    let row_iter: Box<dyn Iterator<Item = &BuiltRow>> = if spec.reverse_insertion {
        Box::new(built_rows.iter().rev())
    } else {
        Box::new(built_rows.iter())
    };
    for built in row_iter {
        connection
            .execute(
                "INSERT INTO rows (sheet_id, row_id, subrow_id, technical_payload, row_hash, technical_hash, string_hash) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    built.spec.row_id,
                    built.spec.subrow_id,
                    built.technical_payload,
                    built.row_hash.as_slice(),
                    built.technical_hash.as_slice(),
                    built.string_hash.as_slice()
                ],
            )
            .expect("row");
        let cells: Box<dyn Iterator<Item = &BuiltCell>> = if spec.reverse_insertion {
            Box::new(built.cells.iter().rev())
        } else {
            Box::new(built.cells.iter())
        };
        for cell in cells {
            connection
                .execute(
                    "INSERT INTO string_cells (sheet_id, row_id, subrow_id, column_index, macro_text, raw_value, macro_hash, raw_hash) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        built.spec.row_id,
                        built.spec.subrow_id,
                        cell.spec.column_index,
                        cell.spec.macro_text,
                        cell.spec.raw_value,
                        cell.macro_hash.as_slice(),
                        cell.raw_hash.as_ref().map(<[u8; 32]>::as_slice)
                    ],
                )
                .expect("String cell");
        }
    }
    connection
        .execute(
            "INSERT INTO hxs_meta (id, format_version, game_version, language, scope, content_id, snapshot_id, extractor_version, lumina_version, sheet_count, row_count, string_cell_count) VALUES (1, 1, ?1, ?2, ?3, ?4, ?5, 'test', '7.7.0', 1, ?6, ?7)",
            params![
                spec.game_version,
                spec.source_language,
                spec.scope,
                content_id,
                snapshot_id,
                i64::try_from(built_rows.len()).expect("row count"),
                i64::try_from(built_rows.iter().map(|row| row.cells.len()).sum::<usize>())
                    .expect("String-cell count")
            ],
        )
        .expect("metadata");
    drop(connection);
    Fixture {
        _directory: directory,
        path,
    }
}

struct BuiltCell {
    spec: CellSpec,
    macro_hash: [u8; 32],
    raw_hash: Option<[u8; 32]>,
}

struct BuiltRow {
    spec: RowSpec,
    technical_payload: Vec<u8>,
    row_hash: [u8; 32],
    technical_hash: [u8; 32],
    string_hash: [u8; 32],
    cells: Vec<BuiltCell>,
}

fn build_row(snapshot: &SnapshotSpec, row: &RowSpec, string_columns: &[u32]) -> BuiltRow {
    let mut cells: Vec<_> = row
        .cells
        .iter()
        .cloned()
        .map(|cell| BuiltCell {
            macro_hash: macro_hash(&cell.macro_text),
            raw_hash: cell.raw_value.as_deref().map(raw_hash),
            spec: cell,
        })
        .collect();
    for column_index in string_columns {
        if !cells
            .iter()
            .any(|cell| cell.spec.column_index == *column_index)
        {
            let spec = CellSpec {
                column_index: *column_index,
                macro_text: String::new(),
                raw_value: None,
            };
            cells.push(BuiltCell {
                macro_hash: macro_hash(&spec.macro_text),
                raw_hash: None,
                spec,
            });
        }
    }
    cells.sort_by_key(|cell| cell.spec.column_index);
    let mut string_parts = Vec::new();
    for cell in &cells {
        string_parts.extend(string_part(
            cell.spec.column_index,
            &cell.macro_hash,
            cell.raw_hash.as_ref(),
        ));
    }
    let technical_value = [row.technical, 0, 0, 0];
    let technical_payload = [
        1_u32.to_le_bytes().as_slice(),
        14_u32.to_le_bytes().as_slice(),
        4_u32.to_le_bytes().as_slice(),
        &technical_value,
    ]
    .concat();
    let technical_hash = digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-ROW-TECHNICAL");
        row_identity(hasher, &snapshot.sheet_name, row.row_id, row.subrow_id);
        hasher.update(1_u32.to_le_bytes());
        hasher.update(14_u32.to_le_bytes());
        framed_bytes(hasher, &technical_value);
    });
    let string_hash = digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-ROW-STRINGS");
        row_identity(hasher, &snapshot.sheet_name, row.row_id, row.subrow_id);
        hasher.update(string_parts);
    });
    let row_hash = digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-ROW");
        row_identity(hasher, &snapshot.sheet_name, row.row_id, row.subrow_id);
        hasher.update(technical_hash);
        hasher.update(string_hash);
    });
    BuiltRow {
        spec: row.clone(),
        technical_payload,
        row_hash,
        technical_hash,
        string_hash,
        cells,
    }
}

fn schema_hash(sheet_name: &str, string_columns: &[u32]) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-SCHEMA");
        framed_text(hasher, sheet_name);
        hasher.update(0_u32.to_le_bytes());
        let mut columns: Vec<_> = string_columns
            .iter()
            .map(|column_index| (*column_index, column_index * 4, 1_u32))
            .chain([(1_u32, 4_u32, 14_u32)])
            .collect();
        columns.sort_unstable_by_key(|column| column.0);
        for (index, offset, type_code) in columns {
            hasher.update(index.to_le_bytes());
            hasher.update(offset.to_le_bytes());
            hasher.update(type_code.to_le_bytes());
        }
    })
}

fn sheet_rows_hash(
    domain: &str,
    sheet_name: &str,
    rows: &[BuiltRow],
    select: impl Fn(&BuiltRow) -> &[u8; 32],
) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(domain.as_bytes());
        framed_text(hasher, sheet_name);
        for row in rows {
            hasher.update(row.spec.row_id.to_le_bytes());
            hasher.update(u32::from(row.spec.subrow_id).to_le_bytes());
            hasher.update(select(row));
        }
    })
}

fn string_part(column_index: u32, macro_hash: &[u8; 32], raw_hash: Option<&[u8; 32]>) -> Vec<u8> {
    let mut result = column_index.to_le_bytes().to_vec();
    result.extend(macro_hash);
    result.push(u8::from(raw_hash.is_some()));
    if let Some(raw_hash) = raw_hash {
        result.extend(raw_hash);
    }
    result
}

fn row_identity(hasher: &mut Sha256, sheet_name: &str, row_id: u32, subrow_id: u16) {
    framed_text(hasher, sheet_name);
    hasher.update(row_id.to_le_bytes());
    hasher.update(u32::from(subrow_id).to_le_bytes());
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

fn framed_text(hasher: &mut Sha256, value: &str) {
    hasher.update(
        u32::try_from(value.len())
            .expect("fixture text fits HXS framing")
            .to_le_bytes(),
    );
    hasher.update(value.as_bytes());
}

fn framed_bytes(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(
        u32::try_from(value.len())
            .expect("fixture bytes fit HXS framing")
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
        write!(&mut result, "{byte:02x}").expect("String writing cannot fail");
    }
    result
}
