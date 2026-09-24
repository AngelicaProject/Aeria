use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use aeria_core::{
    DetachReason, ReviewState, SourceBinding, SourceStatus, TranslationUnit, TranslationUnitId,
};
use aeria_hxs::HxsSnapshot;
use aeria_rebase::candidates::{
    CandidateQuery, CandidateSuggester, CandidateSuggestionError, MAX_GENERATED_CANDIDATES,
    MAX_RESULT_LIMIT,
};
use aeria_rebase::{
    CandidateEvidence, ColumnContinuity, ColumnMappingEvidence, Continuity, RowContinuity, RowKeys,
    SourceContextStatus, SourceUpdateError, SourceUpdatePlan, UnitUpdate, UnitUpdateOutcome,
    plan_source_update,
};
use aeria_workspace::Workspace;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const SYNTHETIC_SCHEMA: &str = include_str!("../../aeria-hxs/tests/fixtures/synthetic_v1.sql");
const APPLICATION_ID: i64 = 0x4841_544c;
const SHEET: &str = "台詞";

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
    extractor_version: String,
    lumina_version: String,
    sheet_name: String,
    rows: Vec<RowSpec>,
    reverse_insertion: bool,
    /// Sheets listed as unreadable; a non-empty list writes HXS v2.
    excluded_sheets: Vec<String>,
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

fn plan(workspace: &Workspace, snapshot: &HxsSnapshot) -> SourceUpdatePlan {
    plan_source_update(workspace.metadata(), workspace.units(), snapshot, |_| true)
        .expect("plan succeeds")
}

fn entry(plan: &SourceUpdatePlan, id: TranslationUnitId) -> &UnitUpdate {
    plan.entries()
        .iter()
        .find(|entry| entry.translation_unit_id == id)
        .expect("plan entry")
}

fn proposed_binding(entry: &UnitUpdate) -> &SourceBinding {
    &entry.proposed.as_ref().expect("bound outcome").binding
}

fn open(fixture: &Fixture) -> HxsSnapshot {
    HxsSnapshot::open(&fixture.path).expect("verified HXS")
}

#[test]
fn identical_snapshot_keeps_every_unit_and_planning_is_pure_and_deterministic() {
    let spec = snapshot("old", vec![row(1, "Привет", Some(b"raw".to_vec()), &[1])]);
    let old_fixture = write_snapshot(&spec);
    let new_fixture = write_snapshot(&spec);
    let old = open(&old_fixture);
    let new = open(&new_fixture);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "<future(1)>target")
        .expect("unit");
    workspace
        .update_review_state(id, ReviewState::Reviewed)
        .expect("review state");
    let before = workspace.clone();

    let first = plan(&workspace, &new);
    let mut reversed_units: Vec<_> = workspace.units().collect();
    reversed_units.reverse();
    let second = plan_source_update(workspace.metadata(), reversed_units, &new, |_| true)
        .expect("reversed units plan succeeds");

    assert_eq!(first, second);
    assert_eq!(workspace, before);
    assert!(!first.changes_content_id());
    assert!(first.sheet_schema_updates.is_empty());
    assert_eq!(first.summary.unchanged, 1);
    assert_eq!(first.summary.changed_units, 0);
    let entry = entry(&first, id);
    assert_eq!(entry.outcome, UnitUpdateOutcome::Unchanged);
    assert_eq!(entry.continuity, Some(Continuity::SAME_BINDING));
    assert_eq!(entry.context_status, Some(SourceContextStatus::Unchanged));
    assert!(!entry.changes_unit());
}

#[test]
fn surviving_bindings_classify_content_and_context_separately() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![
            row(1, "Alpha", None, &[1]),
            row(2, "Beta", Some(b"beta".to_vec()), &[2]),
            row(3, "Gamma", None, &[3]),
            row(4, "Delta", Some(b"delta".to_vec()), &[4]),
        ],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![
            row(1, "Alpha", None, &[1]),
            row(2, "Beta", Some(b"beta".to_vec()), &[9]),
            row(3, "Gamma revised", None, &[3]),
            row(4, "Delta", Some(b"delta-bytes".to_vec()), &[4]),
        ],
    ));
    let old = open(&old_fixture);
    let new = open(&new_fixture);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let ids: Vec<_> = (1..=4)
        .map(|row_id| {
            workspace
                .create_unit_from_hxs(&old, SHEET, row_id, 0, 0, "target")
                .expect("unit")
        })
        .collect();

    let plan = plan(&workspace, &new);
    assert!(plan.changes_content_id());
    assert!(plan.sheet_schema_updates.is_empty());
    let expected = [
        (UnitUpdateOutcome::Unchanged, SourceContextStatus::Unchanged),
        (UnitUpdateOutcome::Unchanged, SourceContextStatus::Changed),
        (
            UnitUpdateOutcome::SourceChanged,
            SourceContextStatus::Unchanged,
        ),
        (
            UnitUpdateOutcome::EncodingChanged,
            SourceContextStatus::Unchanged,
        ),
    ];
    for (id, (outcome, context)) in ids.iter().zip(expected) {
        let entry = entry(&plan, *id);
        assert_eq!(entry.outcome, outcome);
        assert_eq!(entry.context_status, Some(context));
        assert_eq!(entry.continuity, Some(Continuity::SAME_BINDING));
        assert_eq!(proposed_binding(entry), &entry.previous_binding);
    }
    assert!(!entry(&plan, ids[0]).changes_unit());
    assert!(
        entry(&plan, ids[1]).changes_unit(),
        "a context-only change still refreshes the fingerprint"
    );
    assert_eq!(plan.summary.unchanged, 2);
    assert_eq!(plan.summary.source_changed, 1);
    assert_eq!(plan.summary.encoding_changed, 1);
    assert_eq!(plan.summary.changed_units, 3);
}

#[test]
fn missing_rows_and_sheets_detach_units_and_keep_their_last_facts() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row(1, "kept", None, &[1]), row(2, "removed", None, &[2])],
    ));
    let without_row = write_snapshot(&snapshot("new", vec![row(1, "kept", None, &[1])]));
    let renamed_sheet = write_snapshot(&renamed(
        snapshot(
            "new",
            vec![row(1, "kept", None, &[1]), row(2, "removed", None, &[2])],
        ),
        "Renamed",
    ));
    let old = open(&old_fixture);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let kept = workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "kept")
        .expect("unit");
    let removed = workspace
        .create_unit_from_hxs(&old, SHEET, 2, 0, 0, "removed")
        .expect("unit");

    let plan_without_row = plan(&workspace, &open(&without_row));
    assert_eq!(
        entry(&plan_without_row, kept).outcome,
        UnitUpdateOutcome::Unchanged
    );
    let detached = entry(&plan_without_row, removed);
    assert_eq!(
        detached.outcome,
        UnitUpdateOutcome::Detached(DetachReason::RowRemoved)
    );
    assert!(detached.proposed.is_none());
    assert!(detached.continuity.is_none());
    assert_eq!(
        detached.previous_binding,
        SourceBinding::new(SHEET, 2, 0, 0)
    );
    assert_eq!(plan_without_row.summary.newly_detached, 1);

    let plan_renamed = plan(&workspace, &open(&renamed_sheet));
    assert!(
        plan_renamed
            .entries()
            .iter()
            .all(|entry| entry.outcome == UnitUpdateOutcome::Detached(DetachReason::SheetRemoved))
    );
    assert_eq!(plan_renamed.sheet_schema_updates.len(), 1);
    assert_eq!(plan_renamed.sheet_schema_updates[0].sheet_name, SHEET);
    assert!(plan_renamed.sheet_schema_updates[0].schema_hash.is_none());
}

#[test]
fn a_sheet_the_new_source_cannot_read_is_unavailable_not_removed() {
    let rows = vec![row(1, "kept", None, &[1])];
    let old_fixture = write_snapshot(&snapshot("old", rows.clone()));
    let old = open(&old_fixture);
    let unreadable = write_snapshot(&with_excluded_sheet(
        renamed(snapshot("new", rows), "Other"),
        SHEET,
    ));
    let unreadable = open(&unreadable);
    assert_eq!(unreadable.excluded_sheets().len(), 1);

    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let unit = workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "kept")
        .expect("unit");

    let plan = plan(&workspace, &unreadable);
    let entry = entry(&plan, unit);
    assert_eq!(
        entry.outcome,
        UnitUpdateOutcome::Detached(DetachReason::SheetUnavailable)
    );
    assert!(entry.proposed.is_none());
    assert_eq!(entry.previous_binding, SourceBinding::new(SHEET, 1, 0, 0));
    assert_eq!(plan.summary.newly_detached, 1);
    assert_eq!(plan.sheet_schema_updates.len(), 1);
    assert!(plan.sheet_schema_updates[0].unavailable);
    assert!(plan.sheet_schema_updates[0].schema_hash.is_none());
}

#[test]
fn a_shifted_row_keeps_its_binding_and_is_marked_for_review_not_rebound() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row(100, "Alpha", None, &[1]), row(101, "Beta", None, &[2])],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![
            row(100, "Inserted", None, &[1]),
            row(101, "Alpha", None, &[2]),
            row(102, "Beta", None, &[3]),
        ],
    ));
    let old = open(&old_fixture);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let alpha = workspace
        .create_unit_from_hxs(&old, SHEET, 100, 0, 0, "alpha")
        .expect("unit");
    let beta = workspace
        .create_unit_from_hxs(&old, SHEET, 101, 0, 0, "beta")
        .expect("unit");

    let plan = plan(&workspace, &open(&new_fixture));
    for id in [alpha, beta] {
        let entry = entry(&plan, id);
        assert_eq!(entry.outcome, UnitUpdateOutcome::SourceChanged);
        assert_eq!(proposed_binding(entry), &entry.previous_binding);
    }
}

#[test]
fn an_inserted_column_is_mapped_by_exact_content_instead_of_shifting_translations() {
    let old_rows = (1..=4)
        .map(|row_id| {
            row_with_cells(
                row_id,
                1,
                vec![
                    cell(0, &format!("name {row_id}"), None),
                    cell(2, &format!("description {row_id}"), None),
                ],
            )
        })
        .collect();
    let new_rows = (1..=4)
        .map(|row_id| {
            let description = if row_id == 4 {
                "description 4 revised".to_owned()
            } else {
                format!("description {row_id}")
            };
            row_with_cells(
                row_id,
                1,
                vec![
                    cell(2, &format!("name {row_id}"), None),
                    cell(4, &description, None),
                ],
            )
        })
        .collect();
    let old_fixture = write_snapshot(&snapshot("old", old_rows));
    let new_fixture = write_snapshot(&snapshot("new", new_rows));
    let old = open(&old_fixture);
    let new = open(&new_fixture);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let mut units = Vec::new();
    for row_id in 1..=4 {
        for column in [0, 2] {
            let id = workspace
                .create_unit_from_hxs(&old, SHEET, row_id, 0, column, "target")
                .expect("unit");
            units.push((id, row_id, column));
        }
    }

    let plan = plan(&workspace, &new);
    assert_eq!(plan.sheet_schema_updates.len(), 1);
    let columns: Vec<_> = plan.sheet_schema_updates[0]
        .columns
        .iter()
        .map(|mapping| (mapping.previous_column, mapping.column))
        .collect();
    assert_eq!(columns, vec![(0, Some(2)), (2, Some(4))]);
    for (id, row_id, column) in units {
        let entry = entry(&plan, id);
        let (expected_column, supporting) = if column == 0 { (2, 4) } else { (4, 3) };
        assert_eq!(
            proposed_binding(entry),
            &SourceBinding::new(SHEET, row_id, 0, expected_column),
            "a translation never moves to a different logical column"
        );
        assert_eq!(
            entry.continuity,
            Some(Continuity::column_mapped(
                column,
                ColumnMappingEvidence::ExactContent {
                    supporting,
                    cast: supporting,
                }
            ))
        );
        let expected_outcome = if row_id == 4 && column == 2 {
            UnitUpdateOutcome::SourceChanged
        } else {
            UnitUpdateOutcome::Unchanged
        };
        assert_eq!(entry.outcome, expected_outcome);
    }
    assert_eq!(plan.summary.column_mapped, 8);
    assert_eq!(plan.summary.detached, 0);
}

#[test]
fn without_content_evidence_only_an_unchanged_column_position_is_trusted() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row(1, "a", None, &[1]), row(2, "b", None, &[2])],
    ));
    let appended_column = write_snapshot(&snapshot(
        "new",
        vec![
            row_with_cells(1, 1, vec![cell(0, "a revised", None), cell(6, "x", None)]),
            row_with_cells(2, 2, vec![cell(0, "b revised", None), cell(6, "y", None)]),
        ],
    ));
    let moved_column = write_snapshot(&snapshot(
        "new",
        vec![
            row_at(1, 2, "a revised", None, &[1]),
            row_at(2, 2, "b revised", None, &[2]),
        ],
    ));
    let old = open(&old_fixture);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    for row_id in [1, 2] {
        workspace
            .create_unit_from_hxs(&old, SHEET, row_id, 0, 0, "target")
            .expect("unit");
    }

    let appended = plan(&workspace, &open(&appended_column));
    for entry in appended.entries() {
        assert_eq!(entry.outcome, UnitUpdateOutcome::SourceChanged);
        assert_eq!(proposed_binding(entry), &entry.previous_binding);
        assert_eq!(
            entry.continuity,
            Some(Continuity::column_mapped(
                0,
                ColumnMappingEvidence::UnchangedPosition
            ))
        );
    }

    let moved = plan(&workspace, &open(&moved_column));
    for entry in moved.entries() {
        assert_eq!(
            entry.outcome,
            UnitUpdateOutcome::Detached(DetachReason::ColumnUnresolved)
        );
    }
    assert_eq!(moved.sheet_schema_updates[0].columns[0].column, None);
}

#[test]
fn split_or_colliding_column_evidence_is_never_guessed() {
    let split_old = write_snapshot(&snapshot(
        "old",
        vec![row(1, "p", None, &[1]), row(2, "q", None, &[2])],
    ));
    let split_new = write_snapshot(&snapshot(
        "new",
        vec![
            row_with_cells(1, 1, vec![cell(2, "p", None), cell(4, "other", None)]),
            row_with_cells(2, 2, vec![cell(2, "another", None), cell(4, "q", None)]),
        ],
    ));
    let old = open(&split_old);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    for row_id in [1, 2] {
        workspace
            .create_unit_from_hxs(&old, SHEET, row_id, 0, 0, "target")
            .expect("unit");
    }
    let split = plan(&workspace, &open(&split_new));
    assert!(
        split
            .entries()
            .iter()
            .all(|entry| entry.outcome
                == UnitUpdateOutcome::Detached(DetachReason::ColumnUnresolved))
    );

    let collision_old = write_snapshot(&snapshot(
        "old",
        vec![
            row_with_cells(1, 1, vec![cell(0, "first", None), cell(2, "x", None)]),
            row_with_cells(2, 2, vec![cell(0, "y", None), cell(2, "second", None)]),
        ],
    ));
    let collision_new = write_snapshot(&snapshot(
        "new",
        vec![
            row_at(1, 4, "first", None, &[1]),
            row_at(2, 4, "second", None, &[2]),
        ],
    ));
    let old = open(&collision_old);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "target")
        .expect("unit");
    workspace
        .create_unit_from_hxs(&old, SHEET, 2, 0, 2, "target")
        .expect("unit");
    let collision = plan(&workspace, &open(&collision_new));
    assert!(
        collision
            .entries()
            .iter()
            .all(|entry| entry.outcome
                == UnitUpdateOutcome::Detached(DetachReason::ColumnUnresolved))
    );
    assert!(
        collision.sheet_schema_updates[0]
            .columns
            .iter()
            .all(|mapping| mapping.column.is_none())
    );
}

#[test]
fn guidance_detaches_blocked_occurrences_and_detached_units_reattach_later() {
    let fixture = write_snapshot(&snapshot(
        "old",
        vec![row(1, "one", None, &[1]), row(2, "two", None, &[2])],
    ));
    let source = open(&fixture);
    let mut workspace = Workspace::from_verified_snapshot(&source, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&source, SHEET, 1, 0, 0, "one")
        .expect("unit");
    let blocked = workspace
        .create_unit_from_hxs(&source, SHEET, 2, 0, 0, "two")
        .expect("unit");
    workspace
        .update_review_state(blocked, ReviewState::Reviewed)
        .expect("review");

    let blocked_plan = plan_source_update(workspace.metadata(), workspace.units(), &source, |b| {
        b.row_id() != 2
    })
    .expect("plan");
    assert_eq!(
        entry(&blocked_plan, blocked).outcome,
        UnitUpdateOutcome::Detached(DetachReason::NotTranslatable)
    );
    assert_eq!(blocked_plan.summary.newly_detached, 1);

    let mut detached = workspace.unit(blocked).expect("unit").clone();
    detached.detach(DetachReason::NotTranslatable);
    let units = [
        workspace
            .units()
            .find(|unit| unit.id() != blocked)
            .expect("permitted unit")
            .clone(),
        detached,
    ];
    let permitted =
        plan_source_update(workspace.metadata(), units.iter(), &source, |_| true).expect("plan");
    let reattached = entry(&permitted, blocked);
    assert_eq!(reattached.outcome, UnitUpdateOutcome::Unchanged);
    assert!(reattached.reattaches());
    assert!(reattached.changes_unit());
    assert_eq!(permitted.summary.reattached, 1);
    assert_eq!(permitted.summary.detached, 0);
}

fn dialogue_row(row_id: u32, key: &str, text: &str) -> RowSpec {
    row_with_cells(
        row_id,
        u8::try_from(row_id).expect("small test row"),
        vec![cell(0, key, None), cell(2, text, None)],
    )
}

fn dialogue_guidance(binding: &SourceBinding) -> bool {
    binding.column_index() == 2
}

/// Units at column 2 of the given rows, carrying the row keys of `snapshot`.
fn keyed_units(snapshot: &HxsSnapshot, rows: &[u32]) -> (Workspace, Vec<TranslationUnit>) {
    let keys = RowKeys::read(snapshot, SHEET, dialogue_guidance)
        .expect("row keys read")
        .expect("dialogue sheet is keyed");
    assert_eq!(keys.column(), 0);
    let mut workspace = Workspace::from_verified_snapshot(snapshot, "fr").expect("workspace");
    let units = rows
        .iter()
        .map(|&row_id| {
            let id = workspace
                .create_unit_from_hxs(snapshot, SHEET, row_id, 0, 2, "target")
                .expect("unit");
            workspace
                .unit(id)
                .expect("unit")
                .clone()
                .with_source_row_key(keys.key_of(row_id, 0))
        })
        .collect();
    (workspace, units)
}

#[test]
fn keyed_rows_follow_their_line_when_a_line_is_inserted() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![
            dialogue_row(1, "TEXT_Q_001", "Hello"),
            dialogue_row(2, "TEXT_Q_002", "Goodbye"),
        ],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![
            dialogue_row(1, "TEXT_Q_000", "Inserted"),
            dialogue_row(2, "TEXT_Q_001", "Hello"),
            dialogue_row(3, "TEXT_Q_002", "Goodbye"),
        ],
    ));
    let old = open(&old_fixture);
    let new = open(&new_fixture);
    let (workspace, units) = keyed_units(&old, &[1, 2]);
    let new_keys = RowKeys::read(&new, SHEET, dialogue_guidance)
        .expect("row keys")
        .expect("keyed");

    let plan = plan_source_update(workspace.metadata(), units.iter(), &new, dialogue_guidance)
        .expect("plan");
    for (unit, (previous_row, new_row)) in units.iter().zip([(1, 2), (2, 3)]) {
        let entry = entry(&plan, unit.id());
        assert_eq!(entry.outcome, UnitUpdateOutcome::Unchanged);
        assert_eq!(
            proposed_binding(entry),
            &SourceBinding::new(SHEET, new_row, 0, 2)
        );
        assert_eq!(
            entry.continuity.map(|continuity| continuity.row),
            Some(RowContinuity::RowKey {
                previous_row_id: previous_row,
                previous_subrow_id: 0,
            })
        );
        assert_eq!(
            entry
                .proposed
                .as_ref()
                .and_then(|proposed| proposed.row_key),
            new_keys.key_of(new_row, 0)
        );
    }
    assert_eq!(plan.summary.row_moved, 2);
}

#[test]
fn a_removed_keyed_line_is_detached_even_when_its_row_id_is_reused() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![
            dialogue_row(1, "TEXT_Q_001", "Hello"),
            dialogue_row(2, "TEXT_Q_002", "Goodbye"),
        ],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![
            dialogue_row(1, "TEXT_Q_001", "Hello"),
            dialogue_row(2, "TEXT_Q_003", "A different line"),
        ],
    ));
    let old = open(&old_fixture);
    let new = open(&new_fixture);
    let (workspace, units) = keyed_units(&old, &[1, 2]);

    let plan = plan_source_update(workspace.metadata(), units.iter(), &new, dialogue_guidance)
        .expect("plan");
    assert_eq!(
        entry(&plan, units[0].id()).outcome,
        UnitUpdateOutcome::Unchanged
    );
    assert_eq!(
        entry(&plan, units[1].id()).outcome,
        UnitUpdateOutcome::Detached(DetachReason::RowRemoved)
    );
}

#[test]
fn units_without_row_keys_keep_their_row_ids_and_receive_keys() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![
            dialogue_row(1, "TEXT_Q_001", "Hello"),
            dialogue_row(2, "TEXT_Q_002", "Goodbye"),
        ],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![
            dialogue_row(1, "TEXT_Q_000", "Inserted"),
            dialogue_row(2, "TEXT_Q_001", "Hello"),
            dialogue_row(3, "TEXT_Q_002", "Goodbye"),
        ],
    ));
    let old = open(&old_fixture);
    let new = open(&new_fixture);
    let (workspace, units) = keyed_units(&old, &[1, 2]);
    let keyless: Vec<TranslationUnit> = units
        .iter()
        .map(|unit| unit.clone().with_source_row_key(None))
        .collect();
    let new_keys = RowKeys::read(&new, SHEET, dialogue_guidance)
        .expect("row keys")
        .expect("keyed");

    let plan = plan_source_update(
        workspace.metadata(),
        keyless.iter(),
        &new,
        dialogue_guidance,
    )
    .expect("plan");
    for unit in &keyless {
        let entry = entry(&plan, unit.id());
        assert_eq!(entry.outcome, UnitUpdateOutcome::SourceChanged);
        assert_eq!(proposed_binding(entry), &entry.previous_binding);
        assert_eq!(
            entry
                .proposed
                .as_ref()
                .and_then(|proposed| proposed.row_key),
            new_keys.key_of(entry.previous_binding.row_id(), 0),
            "the next update can follow the row by its key"
        );
    }

    let unkeyed = plan_source_update(workspace.metadata(), units.iter(), &new, |_| true)
        .expect("plan without a key column");
    assert!(unkeyed.entries().iter().all(|entry| {
        entry.outcome == UnitUpdateOutcome::SourceChanged
            && entry
                .proposed
                .as_ref()
                .is_some_and(|proposed| proposed.row_key.is_none())
    }));
}

#[test]
fn a_binding_claimed_twice_keeps_its_current_owner_and_detaches_the_other() {
    let fixture = write_snapshot(&snapshot("old", vec![row(1, "one", None, &[1])]));
    let source = open(&fixture);
    let mut workspace = Workspace::from_verified_snapshot(&source, "fr").expect("workspace");
    let owner = workspace
        .create_unit_from_hxs(&source, SHEET, 1, 0, 0, "owner")
        .expect("unit");
    let owner_unit = workspace.unit(owner).expect("unit").clone();
    let claimant_id = TranslationUnitId::from_bytes([0; 32]);
    assert!(claimant_id < owner);
    let claimant = TranslationUnit::new(
        claimant_id,
        owner_unit.source_binding().clone(),
        *owner_unit.source_fingerprint(),
        "older translation",
    )
    .with_source_layout(owner_unit.source_layout().expect("layout"))
    .with_source_status(SourceStatus::Detached(DetachReason::NotTranslatable));

    let plan = plan_source_update(
        workspace.metadata(),
        [&owner_unit, &claimant],
        &source,
        |_| true,
    )
    .expect("plan");
    assert_eq!(entry(&plan, owner).outcome, UnitUpdateOutcome::Unchanged);
    assert_eq!(
        entry(&plan, claimant_id).outcome,
        UnitUpdateOutcome::Detached(DetachReason::BindingConflict)
    );
}

#[test]
fn workspace_format_v1_units_use_unchanged_content_or_exact_content_evidence() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row(1, "one", None, &[1]), row(2, "two", None, &[2])],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row(1, "one", None, &[1]), row(2, "two revised", None, &[2])],
    ));
    let old = open(&old_fixture);
    let new = open(&new_fixture);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    for row_id in [1, 2] {
        workspace
            .create_unit_from_hxs(&old, SHEET, row_id, 0, 0, "target")
            .expect("unit");
    }
    let legacy: Vec<_> = workspace
        .units()
        .map(|unit| {
            let mut legacy = TranslationUnit::new(
                unit.id(),
                unit.source_binding().clone(),
                *unit.source_fingerprint(),
                unit.target_macro(),
            );
            legacy.set_review_state(unit.review_state());
            legacy
        })
        .collect();
    assert!(legacy.iter().all(|unit| unit.source_layout().is_none()));

    let same_content =
        plan_source_update(workspace.metadata(), legacy.iter(), &old, |_| true).expect("plan");
    for (entry, unit) in same_content.entries().iter().zip(workspace.units()) {
        assert_eq!(entry.outcome, UnitUpdateOutcome::Unchanged);
        assert_eq!(entry.continuity, Some(Continuity::SAME_BINDING));
        assert_eq!(
            entry.proposed.as_ref().map(|proposed| proposed.layout),
            unit.source_layout()
        );
    }

    let new_content =
        plan_source_update(workspace.metadata(), legacy.iter(), &new, |_| true).expect("plan");
    assert_eq!(new_content.sheet_schema_updates.len(), 1);
    assert!(
        new_content.sheet_schema_updates[0]
            .previous_schema_hash
            .is_none()
    );
    let outcomes: Vec<_> = new_content
        .entries()
        .iter()
        .map(|entry| (entry.previous_binding.row_id(), entry.outcome))
        .collect();
    assert!(outcomes.contains(&(1, UnitUpdateOutcome::Unchanged)));
    assert!(outcomes.contains(&(2, UnitUpdateOutcome::SourceChanged)));
    assert!(new_content.entries().iter().all(|entry| entry.continuity
        == Some(Continuity::column_mapped(
            0,
            ColumnMappingEvidence::ExactContent {
                supporting: 1,
                cast: 1,
            }
        ))));
}

#[test]
fn a_different_source_language_and_repeated_ids_are_errors() {
    let old_fixture = write_snapshot(&snapshot("old", vec![row(1, "one", None, &[1])]));
    let other_language = write_snapshot(&snapshot_with_language(
        "new",
        "ja",
        vec![row(1, "one", None, &[1])],
    ));
    let old = open(&old_fixture);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "target")
        .expect("unit");
    assert!(matches!(
        plan_source_update(
            workspace.metadata(),
            workspace.units(),
            &open(&other_language),
            |_| true
        ),
        Err(SourceUpdateError::SourceLanguageMismatch { .. })
    ));

    let unit = workspace.unit(id).expect("unit");
    assert!(matches!(
        plan_source_update(workspace.metadata(), [unit, unit], &old, |_| true),
        Err(SourceUpdateError::InvalidWorkspace { message }) if message.contains("duplicate")
    ));
}

#[test]
fn equivalent_physical_insertion_order_produces_the_same_plan() {
    let old_spec = snapshot(
        "old",
        vec![row(1, "one", None, &[1]), row(2, "two", None, &[2])],
    );
    let mut new_spec = snapshot(
        "new",
        vec![row(1, "one!", None, &[9]), row(2, "two", None, &[10])],
    );
    new_spec.reverse_insertion = true;
    let old_fixture = write_snapshot(&old_spec);
    let new_fixture = write_snapshot(&new_spec);
    let equivalent_new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row(2, "two", None, &[10]), row(1, "one!", None, &[9])],
    ));
    let old = open(&old_fixture);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "one")
        .expect("first unit");
    workspace
        .create_unit_from_hxs(&old, SHEET, 2, 0, 0, "two")
        .expect("second unit");

    assert_eq!(
        plan(&workspace, &open(&new_fixture)),
        plan(&workspace, &open(&equivalent_new_fixture))
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn bounded_exhaustive_transition_model_obeys_binding_continuity_oracle() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![
            row(10, "Alpha", None, &[1]),
            row(11, "Beta", Some(b"beta".to_vec()), &[2]),
            row(12, "Gamma", Some(b"gamma".to_vec()), &[3]),
        ],
    ));
    let old = open(&old_fixture);
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    for (row_id, column_index, target) in [(10, 0, "a"), (11, 0, "b"), (12, 0, "c")] {
        workspace
            .create_unit_from_hxs(&old, SHEET, row_id, 0, column_index, target)
            .expect("model unit");
    }
    let unit_origins: BTreeMap<_, _> = workspace
        .units()
        .map(|unit| {
            (
                unit.id(),
                usize::try_from(unit.source_binding().row_id() - 10).expect("model origin"),
            )
        })
        .collect();
    let old_content = [
        (macro_hash("Alpha"), None, 1_u8),
        (macro_hash("Beta"), Some(raw_hash(b"beta")), 2_u8),
        (macro_hash("Gamma"), Some(raw_hash(b"gamma")), 3_u8),
    ];
    let placements = model_placements();
    let mut states = 0_usize;
    let mut bound = 0_usize;
    let mut source_changed = 0_usize;
    let mut detached = 0_usize;
    let mut wrong_mappings = 0_usize;

    for placement in placements {
        for mutation_mask in 0_u8..8 {
            for include_inserted in [false, true] {
                let mut new_rows = Vec::new();
                let mut new_content = BTreeMap::new();
                for (origin, slot) in placement.iter().enumerate() {
                    let Some(slot) = slot else {
                        continue;
                    };
                    let (row_id, column_index) = model_slot(*slot);
                    let mutated = mutation_mask & (1 << origin) != 0;
                    let (macro_text, raw_value, technical) = if mutated {
                        (
                            format!("changed-{origin}"),
                            Some(vec![90 + u8::try_from(origin).expect("origin")]),
                            90 + u8::try_from(origin).expect("origin"),
                        )
                    } else {
                        match origin {
                            0 => ("Alpha".into(), None, 1),
                            1 => ("Beta".into(), Some(b"beta".to_vec()), 2),
                            2 => ("Gamma".into(), Some(b"gamma".to_vec()), 3),
                            _ => unreachable!("bounded model origin"),
                        }
                    };
                    new_rows.push(row_at(
                        row_id,
                        column_index,
                        &macro_text,
                        raw_value.clone(),
                        &[technical],
                    ));
                    let binding = SourceBinding::new(SHEET, row_id, 0, column_index);
                    new_content.insert(
                        binding,
                        (
                            macro_hash(&macro_text),
                            raw_value.as_deref().map(raw_hash),
                            technical,
                        ),
                    );
                }
                if include_inserted {
                    let occupied: BTreeSet<_> = placement.iter().flatten().copied().collect();
                    if let Some(slot) = (0..4).find(|slot| !occupied.contains(slot)) {
                        let (row_id, column_index) = model_slot(slot);
                        new_rows.push(row_at(
                            row_id,
                            column_index,
                            "Beta",
                            Some(b"beta".to_vec()),
                            &[7],
                        ));
                        let binding = SourceBinding::new(SHEET, row_id, 0, column_index);
                        new_content
                            .insert(binding, (macro_hash("Beta"), Some(raw_hash(b"beta")), 7));
                    }
                }

                let new_fixture = write_snapshot(&snapshot("new", new_rows));
                let new = open(&new_fixture);
                let plan = plan(&workspace, &new);
                states += 1;
                let actual_ids: BTreeSet<_> = plan
                    .entries()
                    .iter()
                    .map(|entry| entry.translation_unit_id)
                    .collect();
                let expected_ids: BTreeSet<_> = unit_origins.keys().copied().collect();
                assert_eq!(actual_ids, expected_ids);
                for entry in plan.entries() {
                    let origin = unit_origins[&entry.translation_unit_id];
                    if let Some((new_macro, new_raw, new_technical)) =
                        new_content.get(&entry.previous_binding)
                    {
                        bound += 1;
                        assert_eq!(entry.continuity, Some(Continuity::SAME_BINDING));
                        let (old_macro, old_raw, old_technical) = old_content[origin];
                        let expected_outcome = if old_macro != *new_macro {
                            source_changed += 1;
                            UnitUpdateOutcome::SourceChanged
                        } else if old_raw != *new_raw {
                            UnitUpdateOutcome::EncodingChanged
                        } else {
                            UnitUpdateOutcome::Unchanged
                        };
                        assert_eq!(entry.outcome, expected_outcome);
                        assert_eq!(
                            entry.context_status,
                            Some(if old_technical == *new_technical {
                                SourceContextStatus::Unchanged
                            } else {
                                SourceContextStatus::Changed
                            })
                        );
                        if proposed_binding(entry) != &entry.previous_binding {
                            wrong_mappings += 1;
                        }
                    } else {
                        detached += 1;
                        assert!(matches!(entry.outcome, UnitUpdateOutcome::Detached(_)));
                        assert!(entry.proposed.is_none());
                        assert!(entry.continuity.is_none());
                        assert!(entry.context_status.is_none());
                    }
                }
            }
        }
    }

    println!(
        "model states: {states}; bound entries: {bound}; source-changed entries: {source_changed}; detached entries: {detached}; wrong mappings: {wrong_mappings}"
    );
    assert_eq!(wrong_mappings, 0);
    assert_eq!(states, 73 * 8 * 2);
    assert!(bound > 0);
    assert!(source_changed > 0);
    assert!(detached > 0);
}

#[test]
fn candidate_suggestions_are_bounded_and_non_authoritative() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row(1, "Hello", Some(b"raw".to_vec()), &[1])],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![
            row(2, "Hello", Some(b"raw".to_vec()), &[1]),
            row(3, "Hella", None, &[1]),
            row(4, "Hello", Some(b"raw".to_vec()), &[1]),
        ],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "target")
        .expect("unit");
    let before = workspace.clone();
    let unit = workspace.units().next().expect("unit view");
    assert_eq!(id, unit.id());
    let suggester = CandidateSuggester::from_snapshot(&new).expect("candidate index");
    let suggestions = suggester
        .suggest(CandidateQuery::new(id), unit, &old, &new)
        .expect("suggestions");

    assert_eq!(suggestions.len(), 3);
    assert_eq!(suggestions[0].binding.row_id(), 2);
    assert_eq!(suggestions[0].evidence, CandidateEvidence::MacroAndRawValue);
    assert_eq!(suggestions[1].binding.row_id(), 4);
    assert_eq!(suggestions[2].binding.row_id(), 3);
    assert!(
        suggestions
            .iter()
            .all(|candidate| candidate.binding.row_id() != 1)
    );
    assert_eq!(workspace, before);
    let after = workspace.units().next().expect("unit view after");
    assert_eq!(after.id(), id);
    assert_eq!(after.source_binding().row_id(), 1);
}

#[test]
fn unique_exact_candidate_remains_only_a_review_suggestion() {
    let old_fixture = write_snapshot(&snapshot("old", vec![row(1, "Cancel", None, &[1])]));
    let new_fixture = write_snapshot(&snapshot("new", vec![row(2, "Cancel", None, &[1])]));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "target")
        .expect("unit");
    let before = workspace.clone();
    let unit = workspace.units().next().expect("unit view");
    let suggestions = CandidateSuggester::from_snapshot(&new)
        .expect("candidate index")
        .suggest(CandidateQuery::new(id), unit, &old, &new)
        .expect("suggestions");
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].binding.row_id(), 2);
    assert_eq!(workspace, before);
    assert_eq!(
        workspace
            .units()
            .next()
            .expect("unit view")
            .source_binding()
            .row_id(),
        1
    );
}

#[test]
fn candidate_payload_snapshot_must_match_the_index_snapshot() {
    let old_fixture = write_snapshot(&snapshot("old", vec![row(1, "Cancel", None, &[1])]));
    let index_fixture = write_snapshot(&snapshot("new-a", vec![row(2, "Cancel", None, &[1])]));
    let payload_fixture = write_snapshot(&snapshot("new-b", vec![row(3, "Cancel", None, &[1])]));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let index_snapshot = HxsSnapshot::open(&index_fixture.path).expect("new A HXS");
    let payload_snapshot = HxsSnapshot::open(&payload_fixture.path).expect("new B HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "target")
        .expect("unit");
    let unit = workspace.units().next().expect("unit view");
    let error = CandidateSuggester::from_snapshot(&index_snapshot)
        .expect("candidate index")
        .suggest(CandidateQuery::new(id), unit, &old, &payload_snapshot)
        .expect_err("different payload snapshot must be rejected");
    assert!(matches!(
        error,
        CandidateSuggestionError::NewSnapshotMismatch { .. }
    ));
}

#[test]
fn producer_metadata_does_not_change_candidate_payload_compatibility() {
    let old_fixture = write_snapshot(&snapshot("old", vec![row(1, "Cancel", None, &[1])]));
    let index_fixture = write_snapshot(&snapshot_with_producer(
        "new",
        "en",
        "extractor-a",
        "lumina-a",
        vec![row(2, "Cancel", None, &[1])],
    ));
    let payload_fixture = write_snapshot(&snapshot_with_producer(
        "new",
        "en",
        "extractor-b",
        "lumina-b",
        vec![row(2, "Cancel", None, &[1])],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let index_snapshot = HxsSnapshot::open(&index_fixture.path).expect("index HXS");
    let payload_snapshot = HxsSnapshot::open(&payload_fixture.path).expect("payload HXS");
    assert_eq!(
        index_snapshot.metadata().game_version,
        payload_snapshot.metadata().game_version
    );
    assert_eq!(
        index_snapshot.metadata().source_language,
        payload_snapshot.metadata().source_language
    );
    assert_eq!(
        index_snapshot.metadata().scope,
        payload_snapshot.metadata().scope
    );
    assert_eq!(
        index_snapshot.metadata().content_id,
        payload_snapshot.metadata().content_id
    );
    assert_eq!(
        index_snapshot.metadata().snapshot_id,
        payload_snapshot.metadata().snapshot_id
    );
    assert_ne!(
        index_snapshot.metadata().producer,
        payload_snapshot.metadata().producer
    );
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "target")
        .expect("unit");
    let unit = workspace.units().next().expect("unit view");
    let suggester = CandidateSuggester::from_snapshot(&index_snapshot).expect("candidate index");
    let with_producer_difference = suggester
        .suggest(CandidateQuery::new(id), unit, &old, &payload_snapshot)
        .expect("producer-only metadata difference must be accepted");
    let with_same_snapshot = suggester
        .suggest(CandidateQuery::new(id), unit, &old, &index_snapshot)
        .expect("index snapshot payload must be accepted");
    assert_eq!(with_producer_difference, with_same_snapshot);
}

#[test]
fn old_payload_must_match_the_managed_unit_baseline() {
    let managed_fixture = write_snapshot(&snapshot("old-a", vec![row(1, "Cancel", None, &[1])]));
    let supplied_fixture = write_snapshot(&snapshot("old-b", vec![row(1, "Changed", None, &[1])]));
    let new_fixture = write_snapshot(&snapshot("new", vec![row(2, "Cancel", None, &[1])]));
    let managed_snapshot = HxsSnapshot::open(&managed_fixture.path).expect("old A HXS");
    let supplied_snapshot = HxsSnapshot::open(&supplied_fixture.path).expect("old B HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace =
        Workspace::from_verified_snapshot(&managed_snapshot, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&managed_snapshot, SHEET, 1, 0, 0, "target")
        .expect("unit");
    let unit = workspace.units().next().expect("unit view");
    let error = CandidateSuggester::from_snapshot(&new)
        .expect("candidate index")
        .suggest(CandidateQuery::new(id), unit, &supplied_snapshot, &new)
        .expect_err("stale old payload must be rejected");
    assert!(matches!(
        error,
        CandidateSuggestionError::OldBaselineMismatch { .. }
    ));
}

#[test]
fn surviving_bindings_never_compete_for_candidate_suggestions() {
    let old_fixture = write_snapshot(&snapshot("old", vec![row(1, "Hello", None, &[1])]));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row(1, "Hello", None, &[1]), row(2, "Hello", None, &[1])],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "target")
        .expect("unit");
    let unit = workspace.units().next().expect("unit view");
    let suggestions = CandidateSuggester::from_snapshot(&new)
        .expect("candidate index")
        .suggest(CandidateQuery::new(id), unit, &old, &new)
        .expect("suggestions");
    assert!(suggestions.is_empty());
}

#[test]
fn duplicate_candidate_groups_are_capped_and_requested_top_k_is_clamped() {
    let old_fixture = write_snapshot(&snapshot("old", vec![row(1, "Yes", None, &[1])]));
    let generated_limit =
        u32::try_from(MAX_GENERATED_CANDIDATES).expect("test limit fits HXS coordinate");
    let new_rows = (100..(100 + generated_limit + 10))
        .map(|row_id| row(row_id, "Yes", None, &[1]))
        .collect();
    let new_fixture = write_snapshot(&snapshot("new", new_rows));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "target")
        .expect("unit");
    let unit = workspace.units().next().expect("unit view");
    let suggestions = CandidateSuggester::from_snapshot(&new)
        .expect("candidate index")
        .suggest(
            CandidateQuery {
                translation_unit_id: id,
                limit: usize::MAX,
            },
            unit,
            &old,
            &new,
        )
        .expect("suggestions");
    assert_eq!(suggestions.len(), MAX_RESULT_LIMIT);
    assert_eq!(suggestions.first().expect("first").binding.row_id(), 100);
    let result_limit = u32::try_from(MAX_RESULT_LIMIT).expect("test limit fits HXS coordinate");
    assert_eq!(
        suggestions.last().expect("last").binding.row_id(),
        100 + result_limit - 1
    );
}

#[test]
fn equivalent_physical_insertion_order_produces_identical_candidate_suggestions() {
    let old_fixture = write_snapshot(&snapshot("old", vec![row(1, "one", None, &[1])]));
    let mut indexed_spec = snapshot(
        "new",
        vec![
            row(9, "one", None, &[9]),
            row(10, "one", None, &[10]),
            row(11, "near", None, &[11]),
        ],
    );
    indexed_spec.reverse_insertion = true;
    let indexed_fixture = write_snapshot(&indexed_spec);
    let payload_fixture = write_snapshot(&snapshot(
        "new",
        vec![
            row(11, "near", None, &[11]),
            row(9, "one", None, &[9]),
            row(10, "one", None, &[10]),
        ],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let indexed = HxsSnapshot::open(&indexed_fixture.path).expect("indexed new HXS");
    let payload = HxsSnapshot::open(&payload_fixture.path).expect("payload new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&old, SHEET, 1, 0, 0, "target")
        .expect("unit");
    let unit = workspace.units().next().expect("unit view");
    let suggester = CandidateSuggester::from_snapshot(&indexed).expect("candidate index");
    let from_indexed_storage = suggester
        .suggest(CandidateQuery::new(id), unit, &old, &indexed)
        .expect("indexed payload");
    let from_reordered_storage = suggester
        .suggest(CandidateQuery::new(id), unit, &old, &payload)
        .expect("reordered payload");
    assert_eq!(from_indexed_storage, from_reordered_storage);
}

fn snapshot(game_version: &str, rows: Vec<RowSpec>) -> SnapshotSpec {
    snapshot_with_language(game_version, "en", rows)
}

fn model_slot(slot: usize) -> (u32, u32) {
    (10 + u32::try_from(slot).expect("bounded model slot"), 0)
}

fn model_placements() -> Vec<[Option<usize>; 3]> {
    fn visit(
        origin: usize,
        current: &mut [Option<usize>; 3],
        used_slots: u8,
        result: &mut Vec<[Option<usize>; 3]>,
    ) {
        if origin == current.len() {
            result.push(*current);
            return;
        }
        current[origin] = None;
        visit(origin + 1, current, used_slots, result);
        for slot in 0..4 {
            let bit = 1_u8 << slot;
            if used_slots & bit == 0 {
                current[origin] = Some(slot);
                visit(origin + 1, current, used_slots | bit, result);
            }
        }
    }

    let mut result = Vec::new();
    visit(0, &mut [None; 3], 0, &mut result);
    result
}

fn snapshot_with_language(
    game_version: &str,
    source_language: &str,
    rows: Vec<RowSpec>,
) -> SnapshotSpec {
    snapshot_with_producer(game_version, source_language, "test", "7.7.0", rows)
}

fn snapshot_with_producer(
    game_version: &str,
    source_language: &str,
    extractor_version: &str,
    lumina_version: &str,
    rows: Vec<RowSpec>,
) -> SnapshotSpec {
    SnapshotSpec {
        game_version: game_version.into(),
        source_language: source_language.into(),
        scope: "full".into(),
        extractor_version: extractor_version.into(),
        lumina_version: lumina_version.into(),
        sheet_name: "台詞".into(),
        rows,
        reverse_insertion: false,
        excluded_sheets: Vec::new(),
    }
}

fn row(row_id: u32, macro_text: &str, raw_value: Option<Vec<u8>>, technical: &[u8]) -> RowSpec {
    row_at(row_id, 0, macro_text, raw_value, technical)
}

fn cell(column_index: u32, macro_text: &str, raw_value: Option<Vec<u8>>) -> CellSpec {
    CellSpec {
        column_index,
        macro_text: macro_text.into(),
        raw_value,
    }
}

fn with_excluded_sheet(mut spec: SnapshotSpec, sheet_name: &str) -> SnapshotSpec {
    spec.excluded_sheets.push(sheet_name.into());
    spec.excluded_sheets.sort();
    spec
}

fn renamed(mut spec: SnapshotSpec, sheet_name: &str) -> SnapshotSpec {
    spec.sheet_name = sheet_name.into();
    spec
}

fn row_with_cells(row_id: u32, technical: u8, cells: Vec<CellSpec>) -> RowSpec {
    RowSpec {
        row_id,
        subrow_id: 0,
        technical,
        cells,
    }
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
        cells: vec![cell(column_index, macro_text, raw_value)],
    }
}

#[allow(clippy::too_many_lines)]
fn write_snapshot(spec: &SnapshotSpec) -> Fixture {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("snapshot.hxs");
    let connection = Connection::open(&path).expect("fixture database");
    connection.execute_batch(SYNTHETIC_SCHEMA).expect("schema");
    let format_version = if spec.excluded_sheets.is_empty() {
        1
    } else {
        2
    };
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = {format_version}; PRAGMA foreign_keys = ON;"
        ))
        .expect("identity");
    if format_version == 2 {
        connection
            .execute_batch(
                "ALTER TABLE hxs_meta ADD COLUMN excluded_sheet_count INTEGER NOT NULL DEFAULT 0;
                 CREATE TABLE excluded_sheets (
                     name TEXT PRIMARY KEY,
                     reason INTEGER NOT NULL CHECK (reason IN (1, 2, 3))
                 );",
            )
            .expect("HXS v2 schema");
        for name in &spec.excluded_sheets {
            connection
                .execute(
                    "INSERT INTO excluded_sheets (name, reason) VALUES (?1, 3)",
                    params![name],
                )
                .expect("excluded sheet");
        }
    }

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
            if format_version == 1 {
                hasher.update(b"HARMONIA-HXS-CONTENT-v1");
                framed_text(hasher, &spec.source_language);
            } else {
                hasher.update(b"HARMONIA-HXS-CONTENT-v2");
                framed_text(hasher, &spec.source_language);
                hasher.update(1_u32.to_le_bytes());
            }
            framed_text(hasher, &spec.sheet_name);
            framed_text(hasher, &spec.source_language);
            hasher.update(schema_hash);
            hasher.update(content_hash);
            if format_version == 2 {
                let count = u32::try_from(spec.excluded_sheets.len()).expect("count");
                hasher.update(count.to_le_bytes());
                for name in &spec.excluded_sheets {
                    framed_text(hasher, name);
                    hasher.update(3_u32.to_le_bytes());
                }
            }
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
            "INSERT INTO hxs_meta (id, format_version, game_version, language, scope, content_id, snapshot_id, extractor_version, lumina_version, sheet_count, row_count, string_cell_count) VALUES (1, 1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9)",
            params![
                spec.game_version,
                spec.source_language,
                spec.scope,
                content_id,
                snapshot_id,
                spec.extractor_version,
                spec.lumina_version,
                i64::try_from(built_rows.len()).expect("row count"),
                i64::try_from(built_rows.iter().map(|row| row.cells.len()).sum::<usize>())
                    .expect("String-cell count")
            ],
        )
        .expect("metadata");
    if format_version == 2 {
        connection
            .execute(
                "UPDATE hxs_meta SET format_version = 2, excluded_sheet_count = ?1",
                params![i64::try_from(spec.excluded_sheets.len()).expect("count")],
            )
            .expect("HXS v2 metadata");
    }
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

#[test]
fn a_legacy_unit_on_a_column_that_is_no_longer_text_is_detached_as_cell_removed() {
    let fixture = write_snapshot(&snapshot(
        "game",
        vec![row(1, "one", None, &[1]), row(2, "two", None, &[2])],
    ));
    let source = open(&fixture);
    let mut workspace = Workspace::from_verified_snapshot(&source, "fr").expect("workspace");
    let text_unit = workspace
        .create_unit_from_hxs(&source, SHEET, 1, 0, 0, "Un")
        .expect("unit");
    let text_unit = workspace.unit(text_unit).expect("unit").clone();
    // A Workspace Format v1 unit records no layout. Column 1 is the sheet's
    // Int32 column, so no String occurrence exists at this binding.
    let mut legacy = TranslationUnit::new(
        TranslationUnitId::from_bytes([9; 32]),
        SourceBinding::new(SHEET, 1, 0, 1),
        *text_unit.source_fingerprint(),
        "kept",
    );
    legacy.set_translator_note(Some("note".to_owned()));
    let units = [text_unit.clone(), legacy.clone()];

    let plan =
        plan_source_update(workspace.metadata(), units.iter(), &source, |_| true).expect("plan");
    assert_eq!(
        entry(&plan, legacy.id()).outcome,
        UnitUpdateOutcome::Detached(DetachReason::CellRemoved)
    );
    assert!(entry(&plan, legacy.id()).proposed.is_none());
    assert_eq!(
        entry(&plan, text_unit.id()).outcome,
        UnitUpdateOutcome::Unchanged
    );
}

#[test]
fn a_detached_unit_reattaches_when_its_sheet_or_row_returns() {
    let original = write_snapshot(&snapshot(
        "7.0",
        vec![row(1, "one", None, &[1]), row(2, "two", None, &[2])],
    ));
    let original = open(&original);
    let mut workspace = Workspace::from_verified_snapshot(&original, "fr").expect("workspace");
    let id = workspace
        .create_unit_from_hxs(&original, SHEET, 2, 0, 0, "Deux")
        .expect("unit");
    let bound = workspace.unit(id).expect("unit").clone();

    for (reason, returned_text, expected) in [
        (
            DetachReason::SheetUnavailable,
            "two",
            UnitUpdateOutcome::Unchanged,
        ),
        (
            DetachReason::SheetRemoved,
            "two",
            UnitUpdateOutcome::Unchanged,
        ),
        (
            DetachReason::RowRemoved,
            "two",
            UnitUpdateOutcome::Unchanged,
        ),
        (
            DetachReason::RowRemoved,
            "two, revised",
            UnitUpdateOutcome::SourceChanged,
        ),
    ] {
        let mut detached = bound.clone();
        detached.detach(reason);
        let returned = write_snapshot(&snapshot(
            "7.2",
            vec![row(1, "one", None, &[1]), row(2, returned_text, None, &[2])],
        ));
        let returned = open(&returned);
        let plan = plan_source_update(
            workspace.metadata(),
            std::iter::once(&detached),
            &returned,
            |_| true,
        )
        .expect("plan");
        let entry = entry(&plan, id);
        assert_eq!(entry.outcome, expected, "{reason:?} -> {returned_text}");
        assert!(entry.reattaches());
        assert_eq!(proposed_binding(entry), &SourceBinding::new(SHEET, 2, 0, 0));
        assert_eq!(plan.summary.reattached, 1);
    }
}

#[test]
fn one_patch_with_an_inserted_column_moved_lines_changed_and_removed_text_resolves_each_unit() {
    let old_fixture = write_snapshot(&snapshot(
        "7.0",
        vec![
            dialogue_row(1, "Q1", "Hello"),
            dialogue_row(2, "Q2", "Goodbye"),
            dialogue_row(3, "Q3", "Later"),
            dialogue_row(4, "Q4", "Farewell"),
        ],
    ));
    // A speaker column is inserted before the text, a line is inserted at
    // the top, Q2 and Q3 swap places, Q3's text changes, and Q4 is removed.
    let speaker_row = |row_id: u32, key: &str, speaker: &str, text: &str| {
        row_with_cells(
            row_id,
            u8::try_from(row_id).expect("small row"),
            vec![
                cell(0, key, None),
                cell(2, speaker, None),
                cell(4, text, None),
            ],
        )
    };
    let new_fixture = write_snapshot(&snapshot(
        "7.1",
        vec![
            speaker_row(1, "Q0", "Alisaie", "Inserted line"),
            speaker_row(2, "Q1", "Alphinaud", "Hello"),
            speaker_row(3, "Q3", "Alisaie", "Later, revised"),
            speaker_row(4, "Q2", "Alphinaud", "Goodbye"),
        ],
    ));
    let old = open(&old_fixture);
    let new = open(&new_fixture);
    let (workspace, units) = keyed_units(&old, &[1, 2, 3, 4]);
    let translatable = |binding: &SourceBinding| binding.column_index() >= 2;

    let plan =
        plan_source_update(workspace.metadata(), units.iter(), &new, translatable).expect("plan");
    let expected = [
        (1, Some((2, UnitUpdateOutcome::Unchanged))),
        (2, Some((4, UnitUpdateOutcome::Unchanged))),
        (3, Some((3, UnitUpdateOutcome::SourceChanged))),
        (4, None),
    ];
    for (unit, (previous_row, expectation)) in units.iter().zip(expected) {
        let entry = entry(&plan, unit.id());
        match expectation {
            Some((row_id, outcome)) => {
                assert_eq!(entry.outcome, outcome, "line from row {previous_row}");
                assert_eq!(
                    proposed_binding(entry),
                    &SourceBinding::new(SHEET, row_id, 0, 4),
                    "line from row {previous_row} follows its key into the text column"
                );
            }
            None => assert_eq!(
                entry.outcome,
                UnitUpdateOutcome::Detached(DetachReason::RowRemoved)
            ),
        }
    }
    let mapping: Vec<_> = plan.sheet_schema_updates[0]
        .columns
        .iter()
        .map(|mapping| (mapping.previous_column, mapping.column))
        .collect();
    assert_eq!(mapping, vec![(2, Some(4))]);
    assert_eq!(plan.summary.source_changed, 1);
    assert_eq!(plan.summary.newly_detached, 1);
}

#[test]
fn when_row_keys_stop_being_unique_rows_keep_their_ids_and_changed_lines_need_review() {
    let old_fixture = write_snapshot(&snapshot(
        "7.0",
        vec![
            dialogue_row(1, "Q1", "Hello"),
            dialogue_row(2, "Q2", "Goodbye"),
        ],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "7.1",
        vec![
            dialogue_row(1, "DUP", "Inserted"),
            dialogue_row(2, "DUP", "Hello"),
            dialogue_row(3, "Q2", "Goodbye"),
        ],
    ));
    let old = open(&old_fixture);
    let new = open(&new_fixture);
    assert!(
        RowKeys::read(&new, SHEET, dialogue_guidance)
            .expect("read")
            .is_none(),
        "a column with repeated values is not a row key"
    );
    let (workspace, units) = keyed_units(&old, &[1, 2]);

    let plan = plan_source_update(workspace.metadata(), units.iter(), &new, dialogue_guidance)
        .expect("plan");
    for (unit, row_id) in units.iter().zip([1, 2]) {
        let entry = entry(&plan, unit.id());
        // The line cannot be followed, so the unit keeps its row and is never
        // treated as unchanged against different text.
        assert_eq!(entry.outcome, UnitUpdateOutcome::SourceChanged);
        assert_eq!(
            proposed_binding(entry),
            &SourceBinding::new(SHEET, row_id, 0, 2)
        );
        assert_eq!(
            entry.continuity.map(|continuity| continuity.row),
            Some(RowContinuity::SameRow)
        );
        assert!(entry.proposed.as_ref().expect("bound").row_key.is_none());
    }
}

#[test]
fn a_row_key_column_that_moves_still_identifies_its_lines() {
    let old_fixture = write_snapshot(&snapshot(
        "7.0",
        vec![
            dialogue_row(1, "Q1", "Hello"),
            dialogue_row(2, "Q2", "Goodbye"),
        ],
    ));
    let moved_key_row = |row_id: u32, key: &str, text: &str| {
        row_with_cells(
            row_id,
            u8::try_from(row_id).expect("small row"),
            vec![cell(2, text, None), cell(4, key, None)],
        )
    };
    let new_fixture = write_snapshot(&snapshot(
        "7.1",
        vec![
            moved_key_row(1, "Q0", "Inserted"),
            moved_key_row(2, "Q1", "Hello"),
            moved_key_row(3, "Q2", "Goodbye"),
        ],
    ));
    let old = open(&old_fixture);
    let new = open(&new_fixture);
    let new_keys = RowKeys::read(&new, SHEET, dialogue_guidance)
        .expect("read")
        .expect("keyed");
    assert_eq!(new_keys.column(), 4);
    let (workspace, units) = keyed_units(&old, &[1, 2]);

    let plan = plan_source_update(workspace.metadata(), units.iter(), &new, dialogue_guidance)
        .expect("plan");
    for (unit, row_id) in units.iter().zip([2, 3]) {
        let entry = entry(&plan, unit.id());
        assert_eq!(entry.outcome, UnitUpdateOutcome::Unchanged);
        assert_eq!(
            proposed_binding(entry),
            &SourceBinding::new(SHEET, row_id, 0, 2)
        );
    }
}

/// A small deterministic generator for randomized patches.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(bound).expect("bound fits u64"))
            .expect("value below bound fits usize")
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn text(&mut self) -> &'static str {
        VOCABULARY[self.below(VOCABULARY.len())]
    }
}

/// Texts are drawn from a small vocabulary so that lines collide, repeat,
/// and move between rows and columns.
const VOCABULARY: [&str; 6] = ["Hello", "Goodbye", "Later", "Farewell", "Yes", "No"];

type RandomRow = (u32, Vec<CellSpec>);

struct RandomPatch {
    old: SnapshotSpec,
    new: SnapshotSpec,
    keyed: bool,
}

/// Builds an old sheet version and a patched version: removed lines, lines
/// moved to other row IDs (keyed sheets only), edited texts, inserted lines,
/// and sometimes an inserted column that shifts every text column.
fn random_patch(random: &mut Lcg) -> RandomPatch {
    let keyed = random.chance(50);
    let first_text_column = if keyed { 2 } else { 0 };
    let text_columns = [first_text_column, first_text_column + 2];
    let shift = if random.chance(30) { 2 } else { 0 };
    let mut keys = 0;
    let mut next_key = || {
        keys += 1;
        format!("K{keys:03}")
    };

    let mut free: Vec<u32> = (1..=20).collect();
    let mut old_rows: Vec<RandomRow> = Vec::new();
    for _ in 0..3 + random.below(5) {
        let row_id = free.remove(random.below(free.len()));
        let mut cells: Vec<CellSpec> = text_columns
            .iter()
            .map(|&column| cell(column, random.text(), None))
            .collect();
        if keyed {
            cells.insert(0, cell(0, &next_key(), None));
        }
        old_rows.push((row_id, cells));
    }
    old_rows.sort_by_key(|(row_id, _)| *row_id);

    let mut new_rows: Vec<RandomRow> = Vec::new();
    let taken = |rows: &[RandomRow], row_id: u32| rows.iter().any(|(id, _)| *id == row_id);
    // A line inserted above shifts every keyed line into the next line's row.
    let shifted = keyed && random.chance(40);
    let next_row: BTreeMap<u32, u32> = old_rows
        .iter()
        .zip(old_rows.iter().skip(1))
        .map(|((row_id, _), (next, _))| (*row_id, *next))
        .collect();
    for (row_id, cells) in old_rows.iter().rev() {
        if random.chance(20) {
            continue;
        }
        let mut target = *row_id;
        if shifted {
            target = next_row.get(row_id).copied().unwrap_or(row_id + 30);
        } else if keyed && random.chance(40) {
            // Any row ID, including one another line used before the patch.
            let candidate = u32::try_from(1 + random.below(40)).expect("small");
            if !taken(&new_rows, candidate) {
                target = candidate;
            }
        }
        if taken(&new_rows, target) {
            continue;
        }
        let cells = cells
            .iter()
            .map(|spec| {
                if keyed && spec.column_index == 0 {
                    return spec.clone();
                }
                let text = if random.chance(25) {
                    random.text().to_owned()
                } else {
                    spec.macro_text.clone()
                };
                cell(spec.column_index + shift, &text, None)
            })
            .collect();
        new_rows.push((target, cells));
    }
    for _ in 0..random.below(3) {
        let row_id = u32::try_from(41 + random.below(20)).expect("small");
        if taken(&new_rows, row_id) {
            continue;
        }
        let mut cells: Vec<CellSpec> = text_columns
            .iter()
            .map(|&column| cell(column + shift, random.text(), None))
            .collect();
        if keyed {
            cells.insert(0, cell(0, &next_key(), None));
        }
        new_rows.push((row_id, cells));
    }
    if new_rows.is_empty() {
        new_rows.push(old_rows[0].clone());
    }
    new_rows.sort_by_key(|(row_id, _)| *row_id);
    let specs = |rows: Vec<RandomRow>| {
        rows.into_iter()
            .map(|(row_id, cells)| {
                row_with_cells(row_id, u8::try_from(row_id % 7).expect("small"), cells)
            })
            .collect()
    };
    RandomPatch {
        old: snapshot("old", specs(old_rows)),
        new: snapshot("new", specs(new_rows)),
        keyed,
    }
}

fn macro_text(source: &HxsSnapshot, binding: &SourceBinding) -> Option<String> {
    source
        .string_cell(
            binding.sheet_name(),
            binding.row_id(),
            binding.subrow_id(),
            binding.column_index(),
        )
        .expect("cell lookup")
        .map(|cell| cell.macro_text)
}

#[derive(Debug, Default)]
struct RandomTotals {
    unchanged: usize,
    changed: usize,
    followed_keys: usize,
    column_mapped: usize,
    reattached: usize,
    detached: usize,
    binding_conflicts: usize,
}

/// Randomized patches over plain and keyed sheets. Whatever the planner
/// decides, a translation stays "unchanged" only on identical text, every
/// other bound translation is flagged as changed, proposed facts describe the
/// new source exactly, no two units share an occurrence, keyed lines follow
/// their key, and planning is deterministic.
#[test]
#[allow(clippy::too_many_lines)]
fn randomized_patches_never_attach_a_translation_to_different_text() {
    let mut random = Lcg(0x00A3_71A5_EED5);
    let mut totals = RandomTotals::default();
    for case in 0..150 {
        let patch = random_patch(&mut random);
        let old_fixture = write_snapshot(&patch.old);
        let new_fixture = write_snapshot(&patch.new);
        let old = open(&old_fixture);
        let new = open(&new_fixture);
        let keyed = patch.keyed;
        let translatable = |binding: &SourceBinding| !keyed || binding.column_index() != 0;

        let old_keys = if keyed {
            RowKeys::read(&old, SHEET, translatable).expect("keys")
        } else {
            None
        };
        let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
        let mut units = Vec::new();
        for spec in &patch.old.rows {
            for cell_spec in &spec.cells {
                let binding = SourceBinding::new(SHEET, spec.row_id, 0, cell_spec.column_index);
                if !translatable(&binding) || random.chance(30) {
                    continue;
                }
                let id = workspace
                    .create_unit_from_hxs(&old, SHEET, spec.row_id, 0, cell_spec.column_index, "t")
                    .expect("unit");
                // Some units predate row keys, and some are already detached;
                // both can then compete with other units for one occurrence.
                let key = old_keys
                    .as_ref()
                    .and_then(|keys| keys.key_of(spec.row_id, 0))
                    .filter(|_| !random.chance(20));
                let mut unit = workspace
                    .unit(id)
                    .expect("unit")
                    .clone()
                    .with_source_row_key(key);
                if random.chance(15) {
                    unit.detach(DetachReason::RowRemoved);
                }
                units.push(unit);
            }
        }

        let plan = plan_source_update(workspace.metadata(), units.iter(), &new, translatable)
            .expect("plan");
        let again = plan_source_update(workspace.metadata(), units.iter(), &new, translatable)
            .expect("plan");
        assert_eq!(plan, again, "case {case}: planning is deterministic");
        assert_eq!(plan.entries().len(), units.len(), "case {case}");

        let new_keys = if keyed {
            RowKeys::read(&new, SHEET, translatable).expect("keys")
        } else {
            None
        };
        let current: BTreeMap<(u32, u16, u32), [u8; 32]> = new
            .page_string_occurrences(SHEET, None, aeria_hxs::MAX_STRING_OCCURRENCE_PAGE_SIZE)
            .expect("page")
            .occurrences
            .into_iter()
            .map(|occurrence| {
                let coordinate = occurrence.coordinate;
                (
                    (
                        coordinate.row_id,
                        coordinate.subrow_id,
                        coordinate.column_index,
                    ),
                    *occurrence.macro_text_hash.as_bytes(),
                )
            })
            .collect();
        let mut claimed = BTreeSet::new();
        for unit in &units {
            let entry = entry(&plan, unit.id());
            let old_text = macro_text(&old, unit.source_binding()).expect("old text");
            let Some(proposed) = &entry.proposed else {
                assert!(
                    matches!(entry.outcome, UnitUpdateOutcome::Detached(_)),
                    "case {case}: only a detached outcome has no proposal"
                );
                totals.detached += 1;
                if entry.outcome == UnitUpdateOutcome::Detached(DetachReason::BindingConflict) {
                    totals.binding_conflicts += 1;
                }
                continue;
            };
            if entry.reattaches() {
                totals.reattached += 1;
            }
            let binding = &proposed.binding;
            assert!(
                claimed.insert(binding.clone()),
                "case {case}: an occurrence has one owner"
            );
            let key = (
                binding.row_id(),
                binding.subrow_id(),
                binding.column_index(),
            );
            assert_eq!(
                Some(proposed.fingerprint.macro_text_hash().as_bytes()),
                current.get(&key),
                "case {case}: proposed facts describe the new source"
            );
            let new_text = macro_text(&new, binding).expect("proposed occurrence exists");
            match entry.outcome {
                UnitUpdateOutcome::Unchanged | UnitUpdateOutcome::EncodingChanged => {
                    assert_eq!(
                        new_text, old_text,
                        "case {case}: unchanged means identical text"
                    );
                    totals.unchanged += 1;
                }
                UnitUpdateOutcome::SourceChanged => {
                    assert_ne!(new_text, old_text, "case {case}: changed text is flagged");
                    totals.changed += 1;
                }
                UnitUpdateOutcome::Detached(_) => {
                    unreachable!("a detached outcome has no proposal")
                }
            }
            let continuity = entry.continuity.expect("bound continuity");
            if matches!(continuity.column, ColumnContinuity::Mapped { .. }) {
                totals.column_mapped += 1;
            }
            if let RowContinuity::RowKey { .. } = continuity.row {
                let keys = new_keys.as_ref().expect("row keys were used");
                let unit_key = unit.source_row_key().expect("a keyed unit");
                assert_eq!(
                    keys.row_of(unit_key),
                    Some((binding.row_id(), binding.subrow_id())),
                    "case {case}: a keyed line follows its key"
                );
                totals.followed_keys += 1;
            }
        }
    }
    println!("{totals:?}");
    // The generator must exercise every kind of outcome.
    assert!(totals.unchanged > 100, "{totals:?}");
    assert!(totals.changed > 30, "{totals:?}");
    assert!(totals.followed_keys > 10, "{totals:?}");
    assert!(totals.column_mapped > 30, "{totals:?}");
    assert!(totals.detached > 30, "{totals:?}");
    assert!(totals.reattached > 20, "{totals:?}");
    assert!(totals.binding_conflicts > 5, "{totals:?}");
}
