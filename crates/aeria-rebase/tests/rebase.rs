use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use aeria_core::{
    ReviewState, Sha256Hash, SourceBinding, SourceFingerprint, TranslationUnit, TranslationUnitId,
};
use aeria_hxs::HxsSnapshot;
use aeria_rebase::candidates::{
    CandidateQuery, CandidateSuggester, CandidateSuggestionError, MAX_GENERATED_CANDIDATES,
    MAX_RESULT_LIMIT,
};
use aeria_rebase::{
    AutomaticEvidence, CandidateEvidence, CandidateEvidenceSummary, RebaseError, RebaseOutcome,
    RebasePlanner, SourceContextStatus, UnitRebasePlan, plan_rebase,
};
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
    let mut reversed_units: Vec<_> = workspace.units().collect();
    reversed_units.reverse();
    let third = plan_rebase(workspace.metadata(), reversed_units, &old, &new)
        .expect("reversed units plan succeeds");

    assert_eq!(first, second);
    assert_eq!(first, third);
    assert_eq!(workspace, before);
    assert_eq!(first.summary.unchanged, 1);
    assert_eq!(first.unit_entries[0].translation_unit_id, id);
    assert_eq!(first.unit_entries[0].outcome, RebaseOutcome::Unchanged);
    assert_eq!(
        first.unit_entries[0].automatic_evidence,
        Some(AutomaticEvidence::SameBinding)
    );
    assert_eq!(
        first.unit_entries[0].context_status,
        Some(SourceContextStatus::Unchanged)
    );
    assert!(first.unit_entries[0].candidate_evidence.is_empty());
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
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
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
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
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
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
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
        .create_unit_from_hxs(&managed_snapshot, "台詞", 1, 0, 0, "target")
        .expect("unit");
    let unit = workspace.units().next().expect("unit view");
    let error = CandidateSuggester::from_snapshot(&new)
        .expect("candidate index")
        .suggest(CandidateQuery::new(id), unit, &supplied_snapshot, &new)
        .expect_err("stale old payload must be rejected");
    assert!(matches!(
        error,
        CandidateSuggestionError::OldBaselineVerification(
            RebaseError::OldFingerprintMismatch { .. }
        )
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
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
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
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
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
fn same_binding_content_and_context_changes_are_classified_separately() {
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
        assert_eq!(
            entry.automatic_evidence,
            Some(AutomaticEvidence::SameBinding)
        );
        assert_eq!(
            entry.proposed_source_binding,
            Some(SourceBinding::new("台詞", 1, 0, 0))
        );
        assert!(entry.proposed_source_fingerprint.is_some());
        assert_eq!(
            entry.context_status,
            Some(match changed {
                ChangedField::RowTechnical => SourceContextStatus::Changed,
                ChangedField::Macro | ChangedField::Raw => SourceContextStatus::Unchanged,
            })
        );
        assert_eq!(
            entry.outcome,
            match changed {
                ChangedField::RowTechnical => RebaseOutcome::Unchanged,
                ChangedField::Macro | ChangedField::Raw => RebaseOutcome::SourceChanged,
            }
        );
    }
}

#[test]
fn neighboring_string_changes_only_affect_their_own_translation_unit() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row_with_cells(
            1,
            1,
            vec![cell(0, "stable", None), cell(2, "old", None)],
        )],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row_with_cells(
            1,
            1,
            vec![cell(0, "stable", None), cell(2, "new", None)],
        )],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "stable target")
        .expect("stable unit");
    workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 2, "changed target")
        .expect("changed unit");

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    let stable = plan
        .unit_entries
        .iter()
        .find(|entry| entry.previous_source_binding.column_index() == 0)
        .expect("stable entry");
    assert_eq!(stable.outcome, RebaseOutcome::Unchanged);
    assert_eq!(stable.candidate_evidence, Vec::new());
    let changed = plan
        .unit_entries
        .iter()
        .find(|entry| entry.previous_source_binding.column_index() == 2)
        .expect("changed entry");
    assert_eq!(changed.outcome, RebaseOutcome::SourceChanged);
    assert_eq!(changed.candidate_evidence, Vec::new());
    assert_eq!(plan.summary.unchanged, 1);
    assert_eq!(plan.summary.source_changed, 1);
    assert_eq!(plan.summary.ambiguous, 0);
}

#[test]
fn duplicate_content_elsewhere_does_not_disqualify_a_surviving_binding() {
    let old_fixture = write_snapshot(&snapshot("old", vec![row(1, "same", None, &[1])]));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row(1, "same", None, &[1]), row(2, "same", None, &[2])],
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
    assert_eq!(entry.outcome, RebaseOutcome::Unchanged);
    assert_eq!(
        entry.automatic_evidence,
        Some(AutomaticEvidence::SameBinding)
    );
    assert!(entry.candidate_evidence.is_empty());
    assert_eq!(plan.summary.unchanged, 1);
    assert_eq!(plan.summary.source_changed, 0);
    assert_eq!(plan.summary.ambiguous, 0);
}

#[test]
fn raw_value_presence_changes_are_source_changed_at_the_same_binding() {
    for (old_raw, new_raw) in [(Some(b"raw".to_vec()), None), (None, Some(b"raw".to_vec()))] {
        let old_fixture = write_snapshot(&snapshot("old", vec![row(1, "same", old_raw, &[1])]));
        let new_fixture = write_snapshot(&snapshot("new", vec![row(1, "same", new_raw, &[1])]));
        let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
        let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
        let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
        workspace
            .create_unit_from_hxs(&old, "台詞", 1, 0, 0, "target")
            .expect("unit");

        let plan = plan_rebase(workspace.metadata(), workspace.units(), &old, &new)
            .expect("plan succeeds");
        let entry = &plan.unit_entries[0];
        assert_eq!(entry.outcome, RebaseOutcome::SourceChanged);
        assert_eq!(
            entry.proposed_source_binding,
            Some(SourceBinding::new("台詞", 1, 0, 0))
        );
        assert_eq!(entry.context_status, Some(SourceContextStatus::Unchanged));
    }
}

#[test]
fn exact_fingerprint_at_another_binding_is_only_a_candidate() {
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
    assert_eq!(entry.outcome, RebaseOutcome::Ambiguous);
    assert_eq!(entry.automatic_evidence, None);
    assert_eq!(
        entry.candidate_evidence,
        vec![
            CandidateEvidenceSummary {
                evidence: CandidateEvidence::CompleteFingerprint,
                candidate_count: 1,
            },
            CandidateEvidenceSummary {
                evidence: CandidateEvidence::MacroAndRawValue,
                candidate_count: 1,
            },
            CandidateEvidenceSummary {
                evidence: CandidateEvidence::MacroAndRowTechnical,
                candidate_count: 1,
            },
            CandidateEvidenceSummary {
                evidence: CandidateEvidence::ExactMacroText,
                candidate_count: 1,
            },
        ]
    );
    assert_eq!(entry.proposed_source_binding, None);
}

#[test]
fn exact_content_relocation_with_changed_context_is_ambiguous() {
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
    assert_eq!(entry.outcome, RebaseOutcome::Ambiguous);
    assert_eq!(entry.automatic_evidence, None);
    assert_eq!(
        entry.candidate_evidence,
        vec![CandidateEvidenceSummary {
            evidence: CandidateEvidence::ExactMacroText,
            candidate_count: 1,
        }]
    );
    assert_eq!(entry.proposed_source_binding, None);
}

#[test]
fn exact_macro_and_raw_relocation_with_changed_context_is_ambiguous() {
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
    assert_eq!(entry.outcome, RebaseOutcome::Ambiguous);
    assert_eq!(entry.automatic_evidence, None);
    assert_eq!(
        entry.candidate_evidence,
        vec![
            CandidateEvidenceSummary {
                evidence: CandidateEvidence::MacroAndRawValue,
                candidate_count: 1,
            },
            CandidateEvidenceSummary {
                evidence: CandidateEvidence::ExactMacroText,
                candidate_count: 1,
            },
        ]
    );
    assert_eq!(entry.proposed_source_binding, None);
}

#[test]
fn ambiguous_unit_reports_multiple_candidate_evidence_summaries() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row_at(1, 4, "same", Some(b"raw".to_vec()), &[1])],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![
            row(2, "same", Some(b"raw".to_vec()), &[2]),
            row(3, "same", Some(b"raw".to_vec()), &[3]),
            row_at(1, 2, "same", Some(b"other".to_vec()), &[1]),
        ],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 4, "target")
        .expect("unit");

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    let entry = &plan.unit_entries[0];
    assert_eq!(entry.outcome, RebaseOutcome::Ambiguous);
    assert_eq!(
        entry.candidate_evidence,
        vec![
            CandidateEvidenceSummary {
                evidence: CandidateEvidence::MacroAndRawValue,
                candidate_count: 2,
            },
            CandidateEvidenceSummary {
                evidence: CandidateEvidence::MacroAndRowTechnical,
                candidate_count: 1,
            },
            CandidateEvidenceSummary {
                evidence: CandidateEvidence::ExactMacroText,
                candidate_count: 3,
            },
        ]
    );
    assert_eq!(entry.proposed_source_binding, None);
}

#[test]
fn missing_unit_reports_all_matching_candidate_evidence_summaries() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row_at(1, 4, "same", Some(b"raw".to_vec()), &[1])],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row_at(1, 2, "same", Some(b"other".to_vec()), &[1])],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, "台詞", 1, 0, 4, "target")
        .expect("unit");

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    let entry = &plan.unit_entries[0];
    assert_eq!(entry.outcome, RebaseOutcome::Ambiguous);
    assert_eq!(
        entry.candidate_evidence,
        vec![
            CandidateEvidenceSummary {
                evidence: CandidateEvidence::MacroAndRowTechnical,
                candidate_count: 1,
            },
            CandidateEvidenceSummary {
                evidence: CandidateEvidence::ExactMacroText,
                candidate_count: 1,
            },
        ]
    );
    assert_eq!(entry.proposed_source_binding, None);
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
    assert!(plan.unit_entries.iter().all(|entry| {
        entry.candidate_evidence
            == vec![CandidateEvidenceSummary {
                evidence: CandidateEvidence::ExactMacroText,
                candidate_count: 2,
            }]
    }));
}

#[test]
fn duplicate_heavy_ambiguous_plan_keeps_candidate_payload_small() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        (1..=8)
            .map(|row_id| {
                row(
                    row_id,
                    "repeated",
                    None,
                    &[u8::try_from(row_id).expect("old duplicate row fits in a byte")],
                )
            })
            .collect(),
    ));
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
    for row_id in 1..=8 {
        workspace
            .create_unit_from_hxs(&old, "台詞", row_id, 0, 0, "target")
            .expect("unit");
    }

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    assert_eq!(plan.unit_entries.len(), 8);
    assert_eq!(plan.summary.ambiguous, 8);
    assert!(plan.unit_entries.iter().all(|entry| {
        entry.outcome == RebaseOutcome::Ambiguous
            && entry.candidate_evidence
                == vec![CandidateEvidenceSummary {
                    evidence: CandidateEvidence::ExactMacroText,
                    candidate_count: 100,
                }]
    }));
    assert!(
        std::mem::size_of::<UnitRebasePlan>() < 64 * std::mem::size_of::<SourceBinding>(),
        "authoritative entry must not contain 64 candidate bindings"
    );
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
fn surviving_binding_continuity_wins_over_relocation_candidates() {
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
    assert_eq!(plan.summary.unchanged, 0);
    assert_eq!(plan.summary.source_changed, 1);
    assert_eq!(plan.summary.ambiguous, 1);
    let changed = plan
        .unit_entries
        .iter()
        .find(|entry| entry.previous_source_binding.row_id() == 1)
        .expect("surviving binding");
    assert_eq!(changed.outcome, RebaseOutcome::SourceChanged);
    assert_eq!(
        changed.proposed_source_binding,
        Some(SourceBinding::new("台詞", 1, 0, 0))
    );
    assert_eq!(
        changed.automatic_evidence,
        Some(AutomaticEvidence::SameBinding)
    );
    let missing = plan
        .unit_entries
        .iter()
        .find(|entry| entry.previous_source_binding.row_id() == 2)
        .expect("missing binding");
    assert_eq!(missing.outcome, RebaseOutcome::Ambiguous);
    assert_eq!(missing.proposed_source_binding, None);
}

#[test]
#[allow(clippy::too_many_lines)]
fn coordinate_shifts_and_reused_coordinates_never_rebind_units() {
    let old_rows = vec![
        row(100, "Alpha", None, &[1]),
        row(101, "Beta", None, &[2]),
        row(102, "Gamma", None, &[3]),
    ];
    let cases = [
        (
            "plus-one shift",
            vec![
                row(101, "Alpha", None, &[1]),
                row(102, "Beta", None, &[2]),
                row(103, "Gamma", None, &[3]),
            ],
            (0, 2, 1),
        ),
        (
            "minus-one shift",
            vec![
                row(99, "Alpha", None, &[1]),
                row(100, "Beta", None, &[2]),
                row(101, "Gamma", None, &[3]),
            ],
            (0, 2, 1),
        ),
        (
            "large shift",
            vec![
                row(200, "Alpha", None, &[1]),
                row(201, "Beta", None, &[2]),
                row(202, "Gamma", None, &[3]),
            ],
            (0, 0, 3),
        ),
        (
            "middle insertion",
            vec![
                row(100, "Alpha", None, &[1]),
                row(101, "Inserted", None, &[9]),
                row(102, "Beta", None, &[2]),
                row(103, "Gamma", None, &[3]),
            ],
            (1, 2, 0),
        ),
        (
            "middle deletion",
            vec![row(100, "Alpha", None, &[1]), row(102, "Gamma", None, &[3])],
            (2, 0, 1),
        ),
    ];

    for (name, new_rows, (expected_unchanged, expected_source_changed, expected_ambiguous)) in cases
    {
        let old_fixture = write_snapshot(&snapshot("old", old_rows.clone()));
        let new_fixture = write_snapshot(&snapshot("new", new_rows));
        let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
        let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
        let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
        for (row_id, target) in [(100, "a"), (101, "b"), (102, "c")] {
            workspace
                .create_unit_from_hxs(&old, "台詞", row_id, 0, 0, target)
                .expect("unit");
        }

        let plan = plan_rebase(workspace.metadata(), workspace.units(), &old, &new)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(plan.summary.unchanged, expected_unchanged, "{name}");
        assert_eq!(
            plan.summary.source_changed, expected_source_changed,
            "{name}"
        );
        assert_eq!(plan.summary.ambiguous, expected_ambiguous, "{name}");
        assert!(
            plan.unit_entries.iter().all(|entry| {
                (matches!(
                    entry.outcome,
                    RebaseOutcome::Unchanged | RebaseOutcome::SourceChanged
                ) && entry.proposed_source_binding.as_ref()
                    == Some(&entry.previous_source_binding)
                    && entry.proposed_source_fingerprint.is_some()
                    && entry.automatic_evidence == Some(AutomaticEvidence::SameBinding))
                    || (entry.outcome == RebaseOutcome::Ambiguous
                        && entry.proposed_source_binding.is_none()
                        && entry.proposed_source_fingerprint.is_none()
                        && entry.automatic_evidence.is_none())
            }),
            "{name}"
        );
        if name == "plus-one shift" {
            for (old_row, descendant_row) in [(101, 102), (102, 103)] {
                let entry = plan
                    .unit_entries
                    .iter()
                    .find(|entry| entry.previous_source_binding.row_id() == old_row)
                    .expect("shift entry");
                assert_eq!(entry.outcome, RebaseOutcome::SourceChanged);
                assert_eq!(
                    entry.proposed_source_binding,
                    Some(SourceBinding::new("台詞", old_row, 0, 0))
                );
                assert!(entry.candidate_evidence.is_empty());
                assert_ne!(
                    entry.proposed_source_binding,
                    Some(SourceBinding::new("台詞", descendant_row, 0, 0))
                );
            }
        }
    }
}

#[test]
fn reused_coordinate_does_not_bind_the_old_occurrence_to_an_unrelated_value() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![row(10, "A", None, &[1]), row(11, "B", None, &[2])],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![row(10, "B", None, &[2]), row(20, "A", None, &[1])],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    workspace
        .create_unit_from_hxs(&old, "台詞", 10, 0, 0, "a")
        .expect("A unit");
    workspace
        .create_unit_from_hxs(&old, "台詞", 11, 0, 0, "b")
        .expect("B unit");

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    let reused = plan
        .unit_entries
        .iter()
        .find(|entry| entry.previous_source_binding.row_id() == 10)
        .expect("reused binding");
    assert_eq!(reused.outcome, RebaseOutcome::SourceChanged);
    assert_eq!(
        reused.proposed_source_binding,
        Some(SourceBinding::new("台詞", 10, 0, 0))
    );
    let missing = plan
        .unit_entries
        .iter()
        .find(|entry| entry.previous_source_binding.row_id() == 11)
        .expect("missing binding");
    assert_eq!(missing.outcome, RebaseOutcome::Ambiguous);
    assert_eq!(missing.proposed_source_binding, None);
}

#[test]
fn duplicate_values_under_a_row_shift_preserve_surviving_bindings() {
    let old_fixture = write_snapshot(&snapshot(
        "old",
        vec![
            row(100, "Same", Some(b"raw".to_vec()), &[1]),
            row(101, "Same", Some(b"raw".to_vec()), &[2]),
            row(102, "Tail", None, &[3]),
        ],
    ));
    let new_fixture = write_snapshot(&snapshot(
        "new",
        vec![
            row(101, "Same", Some(b"raw".to_vec()), &[1]),
            row(102, "Same", Some(b"raw".to_vec()), &[2]),
            row(103, "Tail", None, &[3]),
        ],
    ));
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let new = HxsSnapshot::open(&new_fixture.path).expect("new HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    for row_id in [100, 101, 102] {
        workspace
            .create_unit_from_hxs(&old, "台詞", row_id, 0, 0, "target")
            .expect("unit");
    }

    let plan =
        plan_rebase(workspace.metadata(), workspace.units(), &old, &new).expect("plan succeeds");
    assert_eq!(plan.summary.unchanged, 1);
    assert_eq!(plan.summary.source_changed, 1);
    assert_eq!(plan.summary.ambiguous, 1);
    let ambiguous = plan
        .unit_entries
        .iter()
        .find(|entry| entry.outcome == RebaseOutcome::Ambiguous)
        .expect("removed shifted binding");
    assert!(ambiguous.proposed_source_binding.is_none());
    assert!(plan.unit_entries.iter().any(|entry| {
        entry.outcome == RebaseOutcome::Unchanged
            && entry.automatic_evidence == Some(AutomaticEvidence::SameBinding)
    }));
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
    let old = HxsSnapshot::open(&old_fixture.path).expect("old HXS");
    let mut workspace = Workspace::from_verified_snapshot(&old, "fr").expect("workspace");
    for (row_id, column_index, target) in [(10, 0, "a"), (11, 0, "b"), (12, 0, "c")] {
        workspace
            .create_unit_from_hxs(&old, "台詞", row_id, 0, column_index, target)
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
    let mut automatically_resolved = 0_usize;
    let mut source_changed = 0_usize;
    let mut unresolved = 0_usize;
    let mut wrong_automatic_mappings = 0_usize;

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
                    let binding = SourceBinding::new("台詞", row_id, 0, column_index);
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
                        let binding = SourceBinding::new("台詞", row_id, 0, column_index);
                        new_content
                            .insert(binding, (macro_hash("Beta"), Some(raw_hash(b"beta")), 7));
                    }
                }

                let new_fixture = write_snapshot(&snapshot("new", new_rows));
                let new = HxsSnapshot::open(&new_fixture.path).expect("model new HXS");
                let plan = plan_rebase(workspace.metadata(), workspace.units(), &old, &new)
                    .expect("model plan succeeds");
                states += 1;
                let actual_ids: BTreeSet<_> = plan
                    .unit_entries
                    .iter()
                    .map(|entry| entry.translation_unit_id)
                    .collect();
                let expected_ids: BTreeSet<_> = unit_origins.keys().copied().collect();
                assert_eq!(actual_ids, expected_ids);
                for entry in &plan.unit_entries {
                    let origin = unit_origins[&entry.translation_unit_id];
                    if let Some((new_macro, new_raw, new_technical)) =
                        new_content.get(&entry.previous_source_binding)
                    {
                        automatically_resolved += 1;
                        assert_eq!(
                            entry.proposed_source_binding.as_ref(),
                            Some(&entry.previous_source_binding)
                        );
                        assert_eq!(
                            entry.automatic_evidence,
                            Some(AutomaticEvidence::SameBinding)
                        );
                        assert!(entry.proposed_source_fingerprint.is_some());
                        let (old_macro, old_raw, old_technical) = old_content[origin];
                        let expected_outcome = if old_macro == *new_macro && old_raw == *new_raw {
                            RebaseOutcome::Unchanged
                        } else {
                            source_changed += 1;
                            RebaseOutcome::SourceChanged
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
                        let binding = entry
                            .proposed_source_binding
                            .as_ref()
                            .expect("surviving entry has authoritative binding");
                        if binding != &entry.previous_source_binding {
                            wrong_automatic_mappings += 1;
                        }
                    } else {
                        unresolved += 1;
                        assert_eq!(entry.outcome, RebaseOutcome::Ambiguous);
                        assert!(entry.proposed_source_binding.is_none());
                        assert!(entry.proposed_source_fingerprint.is_none());
                        assert!(entry.automatic_evidence.is_none());
                        assert!(entry.context_status.is_none());
                    }
                }
            }
        }
    }

    println!(
        "model states: {states}; automatically resolved entries: {automatically_resolved}; source-changed entries: {source_changed}; unresolved entries: {unresolved}; wrong automatic mappings: {wrong_automatic_mappings}"
    );
    assert_eq!(wrong_automatic_mappings, 0);
    assert_eq!(states, 73 * 8 * 2);
    assert!(automatically_resolved > 0);
    assert!(source_changed > 0);
    assert!(unresolved > 0);
}

#[test]
fn duplicate_bindings_are_rejected_across_the_complete_borrowed_input() {
    let fixture = write_snapshot(&snapshot(
        "old",
        vec![row(1, "first", None, &[1]), row(2, "second", None, &[2])],
    ));
    let snapshot = HxsSnapshot::open(&fixture.path).expect("HXS");
    let mut workspace = Workspace::from_verified_snapshot(&snapshot, "fr").expect("workspace");

    let first_binding = SourceBinding::new("台詞", 1, 0, 0);
    let second_binding = SourceBinding::new("台詞", 2, 0, 0);
    let first_fingerprint = workspace
        .create_unit_from_hxs(&snapshot, "台詞", 1, 0, 0, "first")
        .and_then(|id| {
            workspace
                .unit(id)
                .map(|unit| *unit.source_fingerprint())
                .ok_or(aeria_workspace::WorkspaceError::UnitNotFound { id })
        })
        .expect("first fingerprint");
    let second_fingerprint = workspace
        .create_unit_from_hxs(&snapshot, "台詞", 2, 0, 0, "second")
        .and_then(|id| {
            workspace
                .unit(id)
                .map(|unit| *unit.source_fingerprint())
                .ok_or(aeria_workspace::WorkspaceError::UnitNotFound { id })
        })
        .expect("second fingerprint");

    let units = [
        TranslationUnit::new(
            TranslationUnitId::from_bytes([0; 32]),
            first_binding.clone(),
            first_fingerprint,
            "",
        ),
        TranslationUnit::new(
            TranslationUnitId::from_bytes([1; 32]),
            second_binding,
            second_fingerprint,
            "",
        ),
        TranslationUnit::new(
            TranslationUnitId::from_bytes([2; 32]),
            first_binding,
            first_fingerprint,
            "",
        ),
    ];
    let error = plan_rebase(workspace.metadata(), units.iter(), &snapshot, &snapshot)
        .expect_err("duplicate binding must be rejected before matching");
    assert!(
        matches!(error, RebaseError::InvalidWorkspace { message } if message.contains("duplicate current source binding"))
    );

    let duplicate_id_units = [
        TranslationUnit::new(
            TranslationUnitId::from_bytes([0; 32]),
            SourceBinding::new("台詞", 1, 0, 0),
            first_fingerprint,
            "",
        ),
        TranslationUnit::new(
            TranslationUnitId::from_bytes([0; 32]),
            SourceBinding::new("台詞", 2, 0, 0),
            second_fingerprint,
            "",
        ),
    ];
    let error = plan_rebase(
        workspace.metadata(),
        duplicate_id_units.iter(),
        &snapshot,
        &snapshot,
    )
    .expect_err("duplicate ID must be rejected before matching");
    assert!(
        matches!(error, RebaseError::InvalidWorkspace { message } if message.contains("duplicate translation-unit ID"))
    );
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

fn cell(column_index: u32, macro_text: &str, raw_value: Option<Vec<u8>>) -> CellSpec {
    CellSpec {
        column_index,
        macro_text: macro_text.into(),
        raw_value,
    }
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
