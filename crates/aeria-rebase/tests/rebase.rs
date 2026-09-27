//! Source update planning over synthetic game installations.

use std::collections::{BTreeMap, BTreeSet};

use std::sync::Arc;

use aeria_core::{
    DetachReason, GameVersion, ReviewState, SourceBinding, SourceFacts, TranslationUnit,
    TranslationUnitId, WorkspaceMetadata,
};
use aeria_rebase::{
    ColumnContinuity, ColumnMappingEvidence, Continuity, RowContinuity, SourceUpdateError,
    SourceUpdatePlan, UnitUpdate, UnitUpdateOutcome, plan_source_update,
};
use aeria_source::{GameSource, SheetLookup, SourceLanguage, SourceSheet};
use aeria_sqpack::excel::{ColumnKind, Language};
use aeria_sqpack::testing::{FakeGame, FakeRow, FakeSheet, TextSheet as Spec};
use tempfile::TempDir;

const SHEET: &str = "台詞";
const OLD: &str = "2026.09.15.0000.0000";
const NEW: &str = "2026.10.01.0000.0000";

struct Game {
    _folder: TempDir,
    source: GameSource,
}

fn game(version: &str, sheets: &[(&str, &Spec)]) -> Game {
    let folder = tempfile::tempdir().expect("folder");
    let mut fake = FakeGame::new(version);
    for (name, spec) in sheets {
        fake = fake.with_sheet(*name, spec.fake());
    }
    fake.write(folder.path()).expect("game");
    let source = GameSource::open(folder.path(), SourceLanguage::English).expect("source");
    Game {
        _folder: folder,
        source,
    }
}

fn sheet(source: &GameSource, name: &str) -> Arc<SourceSheet> {
    match source.sheet(name).expect("readable") {
        SheetLookup::Present(sheet) => sheet,
        other => panic!("{name} is {other:?}"),
    }
}

fn metadata(version: &str) -> WorkspaceMetadata {
    WorkspaceMetadata::new("en", "fr", version.parse().expect("version")).expect("metadata")
}

/// A bound unit at one cell of `source`, as the workspace creates it.
fn unit(source: &GameSource, row_id: u32, column: u32) -> TranslationUnit {
    unit_in(source, SHEET, row_id, column)
}

fn unit_in(source: &GameSource, name: &str, row_id: u32, column: u32) -> TranslationUnit {
    let facts = sheet(source, name)
        .facts(row_id, 0, column)
        .expect("a String cell");
    let id = TranslationUnitId::derive(facts.binding(), facts.text()).expect("ID");
    TranslationUnit::new(id, facts, "target")
}

fn plan(units: &[TranslationUnit], game: &Game) -> SourceUpdatePlan {
    plan_source_update(&metadata(OLD), units, &game.source).expect("plan")
}

fn entry(plan: &SourceUpdatePlan, id: TranslationUnitId) -> &UnitUpdate {
    plan.entries()
        .iter()
        .find(|entry| entry.translation_unit_id == id)
        .expect("plan entry")
}

fn proposed(entry: &UnitUpdate) -> &SourceBinding {
    entry.proposed.as_ref().expect("bound outcome").binding()
}

fn binding(row_id: u32, column: u32) -> SourceBinding {
    SourceBinding::new(SHEET, row_id, 0, column)
}

#[test]
fn an_identical_game_keeps_every_unit_and_planning_is_pure_and_deterministic() {
    let spec = Spec::new(1, &[0])
        .row(1, &[(0, "Привет")])
        .row(2, &[(0, "Пока")]);
    let old = game(OLD, &[(SHEET, &spec)]);
    let new = game(NEW, &[(SHEET, &spec)]);
    let mut units = vec![unit(&old.source, 1, 0), unit(&old.source, 2, 0)];
    units[0].set_review_state(ReviewState::Reviewed);
    let before = units.clone();

    let first = plan(&units, &new);
    let reversed: Vec<_> = units.iter().rev().collect();
    let second =
        plan_source_update(&metadata(OLD), reversed, &new.source).expect("reversed units plan");
    assert_eq!(first, second);
    assert_eq!(units, before);
    assert!(first.changes_game_version());
    assert!(first.sheet_layout_updates.is_empty());
    assert_eq!(first.summary.unchanged, 2);
    assert_eq!(first.summary.changed_units, 0);
    for unit in &units {
        let entry = entry(&first, unit.id());
        assert_eq!(entry.outcome, UnitUpdateOutcome::Unchanged);
        assert_eq!(entry.continuity, Some(Continuity::SAME_BINDING));
        assert!(!entry.changes_unit());
    }

    let same_version = game(OLD, &[(SHEET, &spec)]);
    assert!(!plan(&units, &same_version).changes_workspace());
}

#[test]
fn changed_text_needs_review_and_unchanged_bytes_do_not() {
    let old = game(
        OLD,
        &[(
            SHEET,
            &Spec::new(1, &[0])
                .row(1, &[(0, "Alpha")])
                .row(2, &[(0, "Beta")])
                .row_bytes(3, 0, b"Gamma\xff"),
        )],
    );
    let new = game(
        NEW,
        &[(
            SHEET,
            &Spec::new(1, &[0])
                .row(1, &[(0, "Alpha")])
                .row(2, &[(0, "Beta revised")])
                .row_bytes(3, 0, b"Gamma\xff"),
        )],
    );
    let units: Vec<_> = (1..=3).map(|row| unit(&old.source, row, 0)).collect();
    assert_eq!(units[2].source().text(), "Gamma<raw FF>");

    let plan = plan(&units, &new);
    let outcomes: Vec<_> = units
        .iter()
        .map(|unit| entry(&plan, unit.id()).outcome)
        .collect();
    assert_eq!(
        outcomes,
        [
            UnitUpdateOutcome::Unchanged,
            UnitUpdateOutcome::SourceChanged,
            UnitUpdateOutcome::Unchanged,
        ]
    );
    let changed = entry(&plan, units[1].id());
    assert_eq!(
        changed.proposed.as_ref().expect("bound").text(),
        "Beta revised"
    );
    assert_eq!(changed.previous.text(), "Beta");
    assert_eq!(plan.summary.source_changed, 1);
    assert_eq!(plan.summary.changed_units, 1);
}

#[test]
fn removed_rows_removed_sheets_and_unreadable_sheets_detach_units_with_their_facts() {
    let spec = Spec::new(1, &[0])
        .row(1, &[(0, "kept")])
        .row(2, &[(0, "removed")]);
    let old = game(OLD, &[(SHEET, &spec), ("Gone", &spec), ("Broken", &spec)]);
    let units = vec![
        unit(&old.source, 1, 0),
        unit(&old.source, 2, 0),
        unit_in(&old.source, "Gone", 1, 0),
        unit_in(&old.source, "Broken", 1, 0),
    ];

    let folder = tempfile::tempdir().expect("folder");
    let japanese_only = FakeSheet::new(vec![ColumnKind::String])
        .with_rows(Language::Japanese, vec![FakeRow::new(1, vec!["x".into()])]);
    FakeGame::new(NEW)
        .with_sheet(SHEET, Spec::new(1, &[0]).row(1, &[(0, "kept")]).fake())
        .with_sheet("Broken", japanese_only)
        .write(folder.path())
        .expect("game");
    let new = Game {
        source: GameSource::open(folder.path(), SourceLanguage::English).expect("source"),
        _folder: folder,
    };

    let plan = plan(&units, &new);
    let outcomes: Vec<_> = units
        .iter()
        .map(|unit| entry(&plan, unit.id()).outcome)
        .collect();
    assert_eq!(
        outcomes,
        [
            UnitUpdateOutcome::Unchanged,
            UnitUpdateOutcome::Detached(DetachReason::RowRemoved),
            UnitUpdateOutcome::Detached(DetachReason::SheetRemoved),
            UnitUpdateOutcome::Detached(DetachReason::SheetUnavailable),
        ]
    );
    for unit in &units[1..] {
        let entry = entry(&plan, unit.id());
        assert_eq!(&entry.previous, unit.source());
        assert!(entry.proposed.is_none());
        assert!(entry.changes_unit());
    }
    let unavailable: Vec<_> = plan
        .sheet_layout_updates
        .iter()
        .map(|update| {
            (
                update.sheet_name.as_str(),
                update.unavailable,
                update.layout,
            )
        })
        .collect();
    assert_eq!(unavailable, [("Broken", true, None), ("Gone", false, None)]);
}

#[test]
fn a_shifted_row_keeps_its_binding_and_needs_review_in_an_unkeyed_sheet() {
    let old = game(
        OLD,
        &[(
            SHEET,
            &Spec::new(1, &[0])
                .row(100, &[(0, "Alpha")])
                .row(101, &[(0, "Beta")]),
        )],
    );
    let new = game(
        NEW,
        &[(
            SHEET,
            &Spec::new(1, &[0])
                .row(100, &[(0, "Inserted")])
                .row(101, &[(0, "Alpha")])
                .row(102, &[(0, "Beta")]),
        )],
    );
    let units = vec![unit(&old.source, 100, 0), unit(&old.source, 101, 0)];
    let plan = plan(&units, &new);
    for (unit, text) in units.iter().zip(["Inserted", "Alpha"]) {
        let entry = entry(&plan, unit.id());
        assert_eq!(entry.outcome, UnitUpdateOutcome::SourceChanged);
        assert_eq!(proposed(entry), unit.source_binding());
        assert_eq!(entry.proposed.as_ref().expect("bound").text(), text);
    }
}

#[test]
fn an_inserted_column_is_mapped_by_exact_text_instead_of_shifting_translations() {
    // Columns 0 and 2 hold the name and description; the patch inserts a
    // String column at 0, moving them to 1 and 3.
    let old = game(
        OLD,
        &[(
            SHEET,
            &Spec::new(3, &[0, 2])
                .row(1, &[(0, "Sword"), (2, "A blade")])
                .row(2, &[(0, "Shield"), (2, "A board")])
                .row(3, &[(0, "Bow"), (2, "A stick")]),
        )],
    );
    let new = game(
        NEW,
        &[(
            SHEET,
            &Spec::new(4, &[0, 1, 3])
                .row(1, &[(0, "new"), (1, "Sword"), (3, "A blade")])
                .row(2, &[(0, "new"), (1, "Shield"), (3, "A board, revised")])
                .row(3, &[(0, "new"), (1, "Bow"), (3, "A stick")]),
        )],
    );
    let names: Vec<_> = (1..=3).map(|row| unit(&old.source, row, 0)).collect();
    let descriptions: Vec<_> = (1..=3).map(|row| unit(&old.source, row, 2)).collect();
    let units: Vec<_> = names.iter().chain(&descriptions).cloned().collect();

    let plan = plan(&units, &new);
    for (unit, column) in names
        .iter()
        .map(|unit| (unit, 1))
        .chain(descriptions.iter().map(|unit| (unit, 3)))
    {
        let entry = entry(&plan, unit.id());
        assert_eq!(proposed(entry).column_index(), column);
        assert!(matches!(
            entry.continuity.expect("bound").column,
            ColumnContinuity::Mapped {
                evidence: ColumnMappingEvidence::ExactContent { .. },
                ..
            }
        ));
    }
    assert_eq!(
        entry(&plan, descriptions[1].id()).outcome,
        UnitUpdateOutcome::SourceChanged,
        "a changed cell in a mapped column needs review"
    );
    assert_eq!(plan.summary.column_mapped, 6);
    assert_eq!(plan.summary.source_changed, 1);
    let update = &plan.sheet_layout_updates[0];
    let mapped: Vec<_> = update
        .columns
        .iter()
        .map(|mapping| (mapping.previous_column, mapping.column))
        .collect();
    assert_eq!(mapped, [(0, Some(1)), (2, Some(3))]);
    assert_eq!(
        update.columns[1].evidence,
        Some(ColumnMappingEvidence::ExactContent {
            supporting: 2,
            cast: 2
        })
    );
}

#[test]
fn a_number_column_change_keeps_the_layout_and_every_binding() {
    let old = game(OLD, &[(SHEET, &Spec::new(2, &[0]).row(1, &[(0, "Alpha")]))]);
    let new = game(NEW, &[(SHEET, &Spec::new(3, &[0]).row(1, &[(0, "Alpha")]))]);
    let units = vec![unit(&old.source, 1, 0)];
    let plan = plan(&units, &new);
    assert!(plan.sheet_layout_updates.is_empty());
    assert_eq!(
        entry(&plan, units[0].id()).continuity,
        Some(Continuity::SAME_BINDING)
    );
}

#[test]
fn a_layout_change_without_evidence_or_with_split_or_colliding_evidence_is_never_guessed() {
    let old = game(
        OLD,
        &[(
            SHEET,
            &Spec::new(2, &[0, 1])
                .row(1, &[(0, "A1"), (1, "B1")])
                .row(2, &[(0, "A2"), (1, "B2")])
                .row(3, &[(0, "A3"), (1, "Same")]),
        )],
    );
    // Column 0's texts all changed (no votes); column 1's votes split
    // between the new columns 1 and 2.
    let split = game(
        NEW,
        &[(
            SHEET,
            &Spec::new(3, &[0, 1, 2])
                .row(1, &[(0, "x"), (1, "B1"), (2, "y")])
                .row(2, &[(0, "x"), (1, "y"), (2, "B2")])
                .row(3, &[(0, "x"), (1, "Same"), (2, "Same")]),
        )],
    );
    let units: Vec<_> = [(1, 0), (2, 0), (1, 1), (2, 1), (3, 1)]
        .into_iter()
        .map(|(row, column)| unit(&old.source, row, column))
        .collect();
    let plan = plan(&units, &split);
    for unit in &units {
        assert_eq!(
            entry(&plan, unit.id()).outcome,
            UnitUpdateOutcome::Detached(DetachReason::ColumnUnresolved)
        );
    }

    // Both previous columns find their texts in the same new column.
    let colliding = game(
        NEW,
        &[(
            SHEET,
            &Spec::new(3, &[0, 2])
                .row(1, &[(0, "x"), (2, "A1")])
                .row(2, &[(0, "x"), (2, "B2")]),
        )],
    );
    let units = vec![unit(&old.source, 1, 0), unit(&old.source, 2, 1)];
    let plan = self::plan(&units, &colliding);
    let mapped: Vec<_> = plan.sheet_layout_updates[0]
        .columns
        .iter()
        .map(|mapping| mapping.column)
        .collect();
    assert_eq!(
        mapped,
        [None, None],
        "two columns mapping to one are unresolved"
    );
}

#[test]
fn a_removed_string_column_detaches_as_cell_removed_in_an_unchanged_row() {
    let old = game(
        OLD,
        &[(SHEET, &Spec::new(2, &[0, 1]).row(1, &[(0, "A"), (1, "B")]))],
    );
    let units = vec![unit(&old.source, 1, 1)];
    // Same layout hash is impossible when a String column is removed, so the
    // unit goes through mapping; its text is gone and the column unresolved.
    let new = game(NEW, &[(SHEET, &Spec::new(2, &[0]).row(1, &[(0, "A")]))]);
    assert_eq!(
        entry(&plan(&units, &new), units[0].id()).outcome,
        UnitUpdateOutcome::Detached(DetachReason::ColumnUnresolved)
    );
    // A unit whose recorded layout matches but whose column is no longer a
    // String column (a merged unit with a stale column) is cell-removed.
    let current = sheet(&new.source, SHEET);
    let stale = SourceFacts::new(binding(1, 1), current.layout(), "B", None);
    let stale = TranslationUnit::new(
        TranslationUnitId::derive(stale.binding(), "B").expect("ID"),
        stale,
        "t",
    );
    assert_eq!(
        entry(&plan(std::slice::from_ref(&stale), &new), stale.id()).outcome,
        UnitUpdateOutcome::Detached(DetachReason::CellRemoved)
    );
}

#[test]
fn lost_permission_detaches_and_a_later_game_reattaches() {
    let spec = Spec::new(1, &[0]).row(1, &[(0, "Name")]);
    let old = game(OLD, &[(SHEET, &spec)]);
    let units = vec![unit(&old.source, 1, 0)];
    let blanked = game(
        NEW,
        &[(SHEET, &Spec::new(1, &[0]).keyed(0).row(1, &[(0, "Name")]))],
    );
    let plan = plan(&units, &blanked);
    assert_eq!(
        entry(&plan, units[0].id()).outcome,
        UnitUpdateOutcome::Detached(DetachReason::NotTranslatable)
    );

    let mut detached = units[0].clone();
    detached.detach(DetachReason::NotTranslatable);
    let restored = game("2026.11.01.0000.0000", &[(SHEET, &spec)]);
    let plan = plan_source_update(&metadata(NEW), [&detached], &restored.source).expect("plan");
    let entry = entry(&plan, detached.id());
    assert_eq!(entry.outcome, UnitUpdateOutcome::Unchanged);
    assert!(entry.reattaches());
    assert_eq!(plan.summary.reattached, 1);
}

#[test]
fn a_detached_unit_reattaches_when_its_sheet_or_row_returns() {
    let spec = Spec::new(1, &[0]).row(1, &[(0, "Back")]);
    let old = game(OLD, &[(SHEET, &spec)]);
    let mut removed_row = unit(&old.source, 1, 0);
    removed_row.detach(DetachReason::RowRemoved);
    let mut removed_sheet = removed_row.clone();
    removed_sheet.detach(DetachReason::SheetRemoved);
    let new = game(NEW, &[(SHEET, &spec)]);
    for unit in [removed_row, removed_sheet] {
        let plan = plan(std::slice::from_ref(&unit), &new);
        let entry = entry(&plan, unit.id());
        assert_eq!(entry.outcome, UnitUpdateOutcome::Unchanged);
        assert!(entry.reattaches() && entry.changes_unit());
    }
}

fn dialogue(rows: &[(u32, &str, &str)]) -> Spec {
    rows.iter()
        .fold(Spec::new(2, &[0, 1]).keyed(0), |spec, (row, key, text)| {
            spec.row(*row, &[(0, key), (1, text)])
        })
}

#[test]
fn keyed_lines_follow_their_key_when_a_line_is_inserted() {
    let old = game(
        OLD,
        &[(
            SHEET,
            &dialogue(&[(100, "K1", "Alpha"), (101, "K2", "Beta")]),
        )],
    );
    let new = game(
        NEW,
        &[(
            SHEET,
            &dialogue(&[
                (100, "K0", "Inserted"),
                (101, "K1", "Alpha"),
                (102, "K2", "Beta"),
            ]),
        )],
    );
    let units = vec![unit(&old.source, 100, 1), unit(&old.source, 101, 1)];
    assert_eq!(units[0].source().row_key(), Some("K1"));
    let plan = plan(&units, &new);
    for (unit, row) in units.iter().zip([101, 102]) {
        let entry = entry(&plan, unit.id());
        assert_eq!(entry.outcome, UnitUpdateOutcome::Unchanged);
        assert_eq!(proposed(entry), &binding(row, 1));
        assert!(matches!(
            entry.continuity.expect("bound").row,
            RowContinuity::RowKey { .. }
        ));
    }
    assert_eq!(plan.summary.row_moved, 2);
}

#[test]
fn a_removed_keyed_line_is_detached_even_when_its_row_id_is_reused() {
    let old = game(
        OLD,
        &[(
            SHEET,
            &dialogue(&[(100, "K1", "Alpha"), (101, "K2", "Beta")]),
        )],
    );
    let new = game(
        NEW,
        &[(
            SHEET,
            &dialogue(&[(100, "K1", "Alpha"), (101, "K3", "Gamma")]),
        )],
    );
    let units = vec![unit(&old.source, 100, 1), unit(&old.source, 101, 1)];
    let plan = plan(&units, &new);
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
fn units_without_keys_keep_their_rows_and_receive_keys() {
    let old = game(
        OLD,
        &[(
            SHEET,
            &Spec::new(2, &[0, 1])
                .row(1, &[(0, "a"), (1, "Alpha")])
                .row(2, &[(0, "b"), (1, "Beta")]),
        )],
    );
    let units = vec![unit(&old.source, 1, 1)];
    assert_eq!(units[0].source().row_key(), None);
    let new = game(
        NEW,
        &[(SHEET, &dialogue(&[(1, "K1", "Alpha"), (2, "K2", "Beta")]))],
    );
    let plan = plan(&units, &new);
    let entry = entry(&plan, units[0].id());
    assert_eq!(entry.outcome, UnitUpdateOutcome::Unchanged);
    assert_eq!(proposed(entry), &binding(1, 1));
    assert_eq!(
        entry.proposed.as_ref().expect("bound").row_key(),
        Some("K1")
    );
    assert!(entry.changes_unit(), "a new key is written");
}

#[test]
fn when_keys_stop_being_unique_rows_keep_their_ids() {
    let old = game(
        OLD,
        &[(SHEET, &dialogue(&[(1, "K1", "Alpha"), (2, "K2", "Beta")]))],
    );
    let new = game(
        NEW,
        &[(
            SHEET,
            &dialogue(&[(1, "K", "Beta"), (2, "K", "Alpha"), (3, "K3", "Gamma")]),
        )],
    );
    let units = vec![unit(&old.source, 1, 1), unit(&old.source, 2, 1)];
    let plan = plan(&units, &new);
    for unit in &units {
        let entry = entry(&plan, unit.id());
        assert_eq!(entry.outcome, UnitUpdateOutcome::SourceChanged);
        assert_eq!(proposed(entry), unit.source_binding());
        assert_eq!(entry.proposed.as_ref().expect("bound").row_key(), None);
    }
}

#[test]
fn a_binding_claimed_twice_keeps_its_current_owner_and_detaches_the_other() {
    let spec = Spec::new(1, &[0]).row(1, &[(0, "Line")]);
    let old = game(OLD, &[(SHEET, &spec)]);
    let owner = unit(&old.source, 1, 0);
    // A unit merged from another branch that was bound to the same cell
    // under an older text.
    let facts = SourceFacts::new(binding(1, 0), owner.source().layout(), "Old line", None);
    let other = TranslationUnit::new(
        TranslationUnitId::derive(facts.binding(), facts.text()).expect("ID"),
        facts,
        "other",
    );
    let new = game(NEW, &[(SHEET, &spec)]);
    let units = vec![owner.clone(), other.clone()];
    let plan = plan(&units, &new);
    assert_eq!(
        entry(&plan, owner.id()).outcome,
        UnitUpdateOutcome::Unchanged
    );
    assert_eq!(
        entry(&plan, other.id()).outcome,
        UnitUpdateOutcome::Detached(DetachReason::BindingConflict)
    );

    // Between two units with the same claim, the smallest ID wins.
    let mut twin = owner.clone();
    twin.set_target_macro("twin");
    let twin = TranslationUnit::new(
        TranslationUnitId::from_bytes([0; 16]),
        owner.source().clone(),
        "twin",
    );
    let plan = self::plan(&[owner.clone(), twin.clone()], &new);
    assert_eq!(
        entry(&plan, twin.id()).outcome,
        UnitUpdateOutcome::Unchanged
    );
    assert_eq!(
        entry(&plan, owner.id()).outcome,
        UnitUpdateOutcome::Detached(DetachReason::BindingConflict)
    );
}

#[test]
fn another_language_an_older_game_and_repeated_ids_are_errors() {
    let spec = Spec::new(1, &[0]).row(1, &[(0, "Line")]);
    let old = game(OLD, &[(SHEET, &spec)]);
    let units = vec![unit(&old.source, 1, 0)];
    let german =
        WorkspaceMetadata::new("de", "fr", OLD.parse().expect("version")).expect("metadata");
    assert!(matches!(
        plan_source_update(&german, &units, &old.source),
        Err(SourceUpdateError::SourceLanguageMismatch { .. })
    ));
    assert!(matches!(
        plan_source_update(&metadata(NEW), &units, &old.source),
        Err(SourceUpdateError::GameOutdated { .. })
    ));
    let repeated = vec![units[0].clone(), units[0].clone()];
    assert!(matches!(
        plan_source_update(&metadata(OLD), &repeated, &old.source),
        Err(SourceUpdateError::InvalidWorkspace { .. })
    ));
}

/// Row slots of the bounded model: two rows and two String columns.
fn model_slot(slot: usize) -> (u32, usize) {
    (10 + u32::try_from(slot / 2).expect("small"), slot % 2)
}

/// Every injective placement of three logical rows into four slots, where a
/// row may also be removed.
fn model_placements() -> Vec<[Option<usize>; 3]> {
    let choices: Vec<Option<usize>> = std::iter::once(None).chain((0..4).map(Some)).collect();
    let mut placements = Vec::new();
    for &a in &choices {
        for &b in &choices {
            for &c in &choices {
                let used: Vec<usize> = [a, b, c].into_iter().flatten().collect();
                let unique: BTreeSet<usize> = used.iter().copied().collect();
                if used.len() == unique.len() {
                    placements.push([a, b, c]);
                }
            }
        }
    }
    placements
}

#[test]
#[allow(clippy::single_match_else)]
fn bounded_exhaustive_transition_model_obeys_the_binding_continuity_oracle() {
    let texts = ["Alpha", "Beta", "Gamma"];
    let old_spec = Spec::new(2, &[0, 1])
        .row(10, &[(0, "Alpha"), (1, "Beta")])
        .row(11, &[(0, "Gamma")]);
    let old = game(OLD, &[(SHEET, &old_spec)]);
    let origins = [(10, 0), (10, 1), (11, 0)];
    let units: Vec<_> = origins
        .iter()
        .map(|(row, column)| unit(&old.source, *row, *column))
        .collect();

    let (mut states, mut bound, mut source_changed, mut detached, mut wrong) = (0, 0, 0, 0, 0);
    let placements = model_placements();
    assert_eq!(placements.len(), 73);
    for placement in placements {
        for mask in 0_u8..8 {
            for inserted in [false, true] {
                let mut cells: BTreeMap<(u32, usize), String> = BTreeMap::new();
                for (origin, slot) in placement.iter().enumerate() {
                    if let Some(slot) = slot {
                        let text = if mask & (1 << origin) == 0 {
                            texts[origin].to_owned()
                        } else {
                            format!("changed-{origin}")
                        };
                        cells.insert(model_slot(*slot), text);
                    }
                }
                if inserted {
                    let occupied: BTreeSet<usize> = placement.iter().flatten().copied().collect();
                    if let Some(slot) = (0..4).find(|slot| !occupied.contains(slot)) {
                        cells.insert(model_slot(slot), "Beta".to_owned());
                    }
                }
                let mut spec = Spec::new(2, &[0, 1]);
                for row in [10, 11] {
                    let row_cells: Vec<(usize, &str)> = cells
                        .iter()
                        .filter(|((cell_row, _), _)| *cell_row == row)
                        .map(|((_, column), text)| (*column, text.as_str()))
                        .collect();
                    if !row_cells.is_empty() {
                        spec = spec.row(row, &row_cells);
                    }
                }
                let new = game(NEW, &[(SHEET, &spec)]);
                let plan = plan(&units, &new);
                states += 1;
                assert_eq!(plan.entries().len(), units.len());
                for (origin, unit) in units.iter().enumerate() {
                    let entry = entry(&plan, unit.id());
                    let previous = unit.source_binding();
                    let key = (previous.row_id(), previous.column_index() as usize);
                    // An empty cell is not translatable, so a surviving
                    // binding is one that holds text.
                    match cells.get(&key) {
                        Some(text) => {
                            bound += 1;
                            assert_eq!(entry.continuity, Some(Continuity::SAME_BINDING));
                            let expected = if text == texts[origin] {
                                UnitUpdateOutcome::Unchanged
                            } else {
                                source_changed += 1;
                                UnitUpdateOutcome::SourceChanged
                            };
                            assert_eq!(entry.outcome, expected);
                            if proposed(entry) != previous {
                                wrong += 1;
                            }
                        }
                        None => {
                            detached += 1;
                            assert!(matches!(entry.outcome, UnitUpdateOutcome::Detached(_)));
                            assert!(entry.proposed.is_none() && entry.continuity.is_none());
                        }
                    }
                }
            }
        }
    }
    println!(
        "model states: {states}; bound: {bound}; source changed: {source_changed}; detached: {detached}; wrong mappings: {wrong}"
    );
    assert_eq!(states, 73 * 8 * 2);
    assert_eq!(wrong, 0);
    assert!(bound > 0 && source_changed > 0 && detached > 0);
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

type RandomRow = (u32, Vec<(usize, String)>);

struct RandomPatch {
    old: Spec,
    new: Spec,
}

fn random_spec(width: usize, text_columns: &[usize], keyed: bool, rows: &[RandomRow]) -> Spec {
    let mut columns = text_columns.to_vec();
    if keyed {
        columns.push(0);
    }
    let mut spec = Spec::new(width, &columns);
    if keyed {
        spec = spec.keyed(0);
    }
    for (row_id, cells) in rows {
        let cells: Vec<(usize, &str)> = cells
            .iter()
            .map(|(column, text)| (*column, text.as_str()))
            .collect();
        spec = spec.row(*row_id, &cells);
    }
    spec
}

/// Builds an old sheet and a patched one: removed lines, lines moved to other
/// row IDs (keyed sheets only), edited texts, inserted lines, and sometimes
/// an inserted column that shifts every text column.
fn random_patch(random: &mut Lcg) -> RandomPatch {
    let keyed = random.chance(50);
    let first = if keyed { 2 } else { 0 };
    let text_columns = [first, first + 2];
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
        let mut cells: Vec<(usize, String)> = text_columns
            .iter()
            .map(|&column| (column, random.text().to_owned()))
            .collect();
        if keyed {
            cells.insert(0, (0, next_key()));
        }
        old_rows.push((row_id, cells));
    }
    old_rows.sort_by_key(|(row_id, _)| *row_id);

    let mut new_rows: Vec<RandomRow> = Vec::new();
    let taken = |rows: &[RandomRow], row_id: u32| rows.iter().any(|(id, _)| *id == row_id);
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
            .map(|(column, text)| {
                if keyed && *column == 0 {
                    return (0, text.clone());
                }
                let text = if random.chance(25) {
                    random.text().to_owned()
                } else {
                    text.clone()
                };
                (column + shift, text)
            })
            .collect();
        new_rows.push((target, cells));
    }
    for _ in 0..random.below(3) {
        let row_id = u32::try_from(41 + random.below(20)).expect("small");
        if taken(&new_rows, row_id) {
            continue;
        }
        let mut cells: Vec<(usize, String)> = text_columns
            .iter()
            .map(|&column| (column + shift, random.text().to_owned()))
            .collect();
        if keyed {
            cells.insert(0, (0, next_key()));
        }
        new_rows.push((row_id, cells));
    }
    if new_rows.is_empty() {
        new_rows.push(old_rows[0].clone());
    }
    new_rows.sort_by_key(|(row_id, _)| *row_id);
    let width = first + 3;
    let new_columns: Vec<usize> = text_columns.iter().map(|column| column + shift).collect();
    RandomPatch {
        old: random_spec(width, &text_columns, keyed, &old_rows),
        new: random_spec(width + shift, &new_columns, keyed, &new_rows),
    }
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
/// decides, a translation stays unchanged only on identical text, every
/// other bound translation is flagged as changed, proposed facts describe the
/// new game exactly, no two units share a cell, keyed lines follow their key,
/// and planning is deterministic.
#[test]
#[allow(clippy::too_many_lines)]
fn randomized_patches_never_attach_a_translation_to_different_text() {
    let mut random = Lcg(0x00A3_71A5_EED5);
    let mut totals = RandomTotals::default();
    for case in 0..150 {
        let patch = random_patch(&mut random);
        let old = game(OLD, &[(SHEET, &patch.old)]);
        let new = game(NEW, &[(SHEET, &patch.new)]);
        let old_sheet = sheet(&old.source, SHEET);
        let new_sheet = sheet(&new.source, SHEET);

        let mut units = Vec::new();
        for row in old_sheet.rows() {
            for cell in old_sheet.cells(row) {
                if !cell.translatable || random.chance(30) {
                    continue;
                }
                let mut facts = old_sheet
                    .facts(row.row_id, row.subrow_id, cell.column)
                    .expect("cell");
                // Some units predate row keys, and some are already detached;
                // both can then compete with other units for one cell.
                if random.chance(20) {
                    facts = SourceFacts::new(
                        facts.binding().clone(),
                        facts.layout(),
                        facts.text(),
                        None,
                    );
                }
                let id = TranslationUnitId::derive(facts.binding(), facts.text()).expect("ID");
                let mut unit = TranslationUnit::new(id, facts, "t");
                if random.chance(15) {
                    unit.detach(DetachReason::RowRemoved);
                }
                units.push(unit);
            }
        }

        let plan = plan(&units, &new);
        assert_eq!(plan, self::plan(&units, &new), "case {case}: deterministic");
        assert_eq!(plan.entries().len(), units.len(), "case {case}");
        let mut claimed = BTreeSet::new();
        for unit in &units {
            let entry = entry(&plan, unit.id());
            let Some(facts) = &entry.proposed else {
                assert!(
                    matches!(entry.outcome, UnitUpdateOutcome::Detached(_)),
                    "case {case}"
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
            let binding = facts.binding();
            assert!(
                claimed.insert(binding.clone()),
                "case {case}: one owner per cell"
            );
            let current = new_sheet
                .facts(
                    binding.row_id(),
                    binding.subrow_id(),
                    binding.column_index(),
                )
                .expect("the proposed cell exists");
            assert_eq!(
                &current, facts,
                "case {case}: proposed facts describe the game"
            );
            match entry.outcome {
                UnitUpdateOutcome::Unchanged => {
                    assert_eq!(facts.text(), unit.source().text(), "case {case}");
                    totals.unchanged += 1;
                }
                UnitUpdateOutcome::SourceChanged => {
                    assert_ne!(facts.text(), unit.source().text(), "case {case}");
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
                let keys = new_sheet.row_keys().expect("row keys were used");
                let key = unit.source().row_key().expect("a keyed unit");
                assert_eq!(
                    keys.row_of(key),
                    Some((binding.row_id(), binding.subrow_id())),
                    "case {case}: a keyed line follows its key"
                );
                totals.followed_keys += 1;
            }
        }
    }
    println!("{totals:?}");
    assert!(totals.unchanged > 100, "{totals:?}");
    assert!(totals.changed > 30, "{totals:?}");
    assert!(totals.followed_keys > 10, "{totals:?}");
    assert!(totals.column_mapped > 30, "{totals:?}");
    assert!(totals.detached > 30, "{totals:?}");
    assert!(totals.reattached > 20, "{totals:?}");
    assert!(totals.binding_conflicts > 5, "{totals:?}");
}

#[test]
fn game_versions_are_recorded_in_the_plan() {
    let spec = Spec::new(1, &[0]).row(1, &[(0, "Line")]);
    let new = game(NEW, &[(SHEET, &spec)]);
    let units = vec![unit(&new.source, 1, 0)];
    let plan = plan(&units, &new);
    assert_eq!(
        plan.previous_game_version,
        OLD.parse::<GameVersion>().expect("version")
    );
    assert_eq!(plan.game_version.as_str(), NEW);
}
