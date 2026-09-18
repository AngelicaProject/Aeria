//! Deterministic and conservative cross-snapshot migration planning.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use aeria_core::{
    Sha256Hash, SourceBinding, SourceFingerprint, TranslationUnit, TranslationUnitId,
    WorkspaceMetadata,
};
use aeria_hxs::{
    HxsError, HxsSnapshot, MAX_STRING_OCCURRENCE_PAGE_SIZE, SnapshotMetadata,
    StringOccurrenceCoordinate, StringOccurrenceFingerprint,
};
use thiserror::Error;

const MAX_REPORTED_CANDIDATES: usize = 64;

/// Errors raised when a deterministic rebase plan cannot be built safely.
#[derive(Debug, Error)]
pub enum RebaseError {
    /// The old snapshot does not match the workspace source language.
    #[error(
        "old HXS source language does not match the workspace: expected {expected:?}, found {found:?}"
    )]
    OldSourceLanguageMismatch { expected: String, found: String },

    /// The old snapshot does not match the workspace content identity.
    #[error(
        "old HXS content ID does not match the workspace: expected {expected:?}, found {found:?}"
    )]
    OldContentIdMismatch { expected: String, found: String },

    /// The old snapshot does not match the workspace snapshot identity.
    #[error(
        "old HXS snapshot ID does not match the workspace: expected {expected:?}, found {found:?}"
    )]
    OldSnapshotIdMismatch { expected: String, found: String },

    /// The new snapshot has a different source language.
    #[error(
        "new HXS source language does not match the workspace: expected {expected:?}, found {found:?}"
    )]
    NewSourceLanguageMismatch { expected: String, found: String },

    /// The old and new snapshots cover different HXS scopes.
    #[error("old and new HXS scopes are incompatible: old {old:?}, new {new:?}")]
    IncompatibleScope { old: String, new: String },

    /// A source read required to verify the old baseline failed.
    #[error(
        "could not read old HXS occurrence for translation unit {unit_id} at {binding:?}: {source}"
    )]
    OldSourceRead {
        unit_id: TranslationUnitId,
        binding: SourceBinding,
        #[source]
        source: HxsError,
    },

    /// A managed unit's current binding is absent from the old snapshot.
    #[error(
        "old HXS baseline is missing translation unit {unit_id}'s String occurrence at {binding:?}"
    )]
    OldOccurrenceMissing {
        unit_id: TranslationUnitId,
        binding: SourceBinding,
    },

    /// A managed unit's persisted fingerprint disagrees with the old snapshot.
    #[error("old HXS baseline fingerprint disagrees for translation unit {unit_id} at {binding:?}")]
    OldFingerprintMismatch {
        unit_id: TranslationUnitId,
        binding: SourceBinding,
        expected: Box<SourceFingerprint>,
        found: Box<SourceFingerprint>,
    },

    /// The new occurrence scan failed.
    #[error("could not enumerate new HXS String occurrence fingerprints: {0}")]
    NewSourceRead(#[source] HxsError),

    /// The borrowed workspace view did not satisfy its own uniqueness rules.
    #[error("invalid workspace input: {message}")]
    InvalidWorkspace { message: String },
}

/// The stable identity facts of one HXS snapshot included in a plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSnapshotIdentity {
    pub game_version: String,
    pub source_language: String,
    pub scope: String,
    pub content_id: String,
    pub snapshot_id: String,
}

impl SourceSnapshotIdentity {
    fn from_metadata(metadata: &SnapshotMetadata) -> Self {
        Self {
            game_version: metadata.game_version.clone(),
            source_language: metadata.source_language.clone(),
            scope: metadata.scope.clone(),
            content_id: metadata.content_id.clone(),
            snapshot_id: metadata.snapshot_id.clone(),
        }
    }
}

/// The deterministic result for one already managed translation unit.
///
/// The hardening contract intentionally has only two planner outcomes. A
/// source change, relocation, removal, or addition is not established by
/// this planner; those require candidate review or explicit reconciliation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RebaseOutcome {
    /// The previous binding and String-cell source content are unchanged.
    Unchanged,
    /// The previous binding exists, but the String-cell source content changed.
    SourceChanged,
    /// No authoritative identity decision was established.
    Ambiguous,
}

/// The only evidence that can establish automatic source continuity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AutomaticEvidence {
    /// The previous binding exists in the new verified snapshot.
    SameBinding,
}

/// The source-content facts of one managed String cell.
///
/// This intentionally excludes row technical context. The persisted
/// [`SourceFingerprint`] still contains that context for compatibility, but it
/// is not part of same-binding identity classification.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceContent {
    pub macro_text_hash: Sha256Hash,
    pub raw_value_hash: Option<Sha256Hash>,
}

impl SourceContent {
    /// Extracts String-cell content facts from a persisted source fingerprint.
    #[must_use]
    pub const fn from_fingerprint(fingerprint: &SourceFingerprint) -> Self {
        Self {
            macro_text_hash: fingerprint.macro_text_hash(),
            raw_value_hash: fingerprint.raw_value_hash(),
        }
    }
}

/// Diagnostic status for row technical context at a surviving binding.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SourceContextStatus {
    /// The persisted row technical hash is unchanged.
    Unchanged,
    /// The persisted row technical hash changed; this does not change identity.
    Changed,
}

/// Deterministic evidence for a non-authoritative candidate suggestion.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CandidateEvidence {
    /// The complete persisted fingerprint matches at another binding.
    CompleteFingerprint,
    /// Macro-text and the old optional raw-value hash matched.
    MacroAndRawValue,
    /// Macro-text and row technical hashes matched.
    MacroAndRowTechnical,
    /// The macro-text hash matched.
    ExactMacroText,
}

/// One bounded, deterministic, non-authoritative candidate diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateEvidenceSet {
    /// The evidence that produced this candidate set. This is never an
    /// identity decision and never authorizes a source claim.
    pub evidence: CandidateEvidence,
    /// Candidate bindings in canonical source order.
    pub candidate_bindings: Vec<SourceBinding>,
    /// Number of available candidates before diagnostic truncation.
    pub candidate_count: usize,
    /// Whether `candidate_bindings` was truncated to the diagnostic limit.
    pub candidates_truncated: bool,
}

/// One deterministic plan entry for an existing managed unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitRebasePlan {
    pub translation_unit_id: TranslationUnitId,
    pub previous_source_binding: SourceBinding,
    pub previous_source_fingerprint: SourceFingerprint,
    pub outcome: RebaseOutcome,
    pub proposed_source_binding: Option<SourceBinding>,
    pub proposed_source_fingerprint: Option<SourceFingerprint>,
    /// Present for authoritative same-binding continuity, including
    /// `SourceChanged`.
    pub automatic_evidence: Option<AutomaticEvidence>,
    /// Technical-context comparison for a surviving old binding. It is absent
    /// when the old binding is missing.
    pub context_status: Option<SourceContextStatus>,
    /// Bounded, canonical, non-authoritative diagnostics. These suggestions
    /// never claim a new occurrence and never populate the proposed fields.
    pub candidate_evidence: Vec<CandidateEvidenceSet>,
}

/// Backward-friendly name for consumers that call an entry a unit plan entry.
pub type UnitPlanEntry = UnitRebasePlan;

/// Deterministic counts derived from the plan entries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RebaseSummary {
    pub unchanged: usize,
    pub source_changed: usize,
    pub ambiguous: usize,
}

/// A pure, owned plan for rebasing managed units from one verified snapshot to another.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebasePlan {
    pub old_source_snapshot: SourceSnapshotIdentity,
    pub new_source_snapshot: SourceSnapshotIdentity,
    pub unit_entries: Vec<UnitRebasePlan>,
    pub summary: RebaseSummary,
}

impl RebasePlan {
    /// Returns entries in ascending [`TranslationUnitId`] order.
    #[must_use]
    pub fn entries(&self) -> &[UnitRebasePlan] {
        &self.unit_entries
    }
}

/// Stateless entry point for deterministic source rebase planning.
#[derive(Clone, Copy, Debug, Default)]
pub struct RebasePlanner;

impl RebasePlanner {
    /// Builds a pure plan from a workspace metadata view, managed units, and
    /// verified old/new HXS snapshots.
    ///
    /// The iterator is collected and sorted by durable unit ID, so callers may
    /// provide units from any map or storage iteration order. The workspace and
    /// snapshots are borrowed and never mutated.
    ///
    /// # Errors
    ///
    /// Returns [`RebaseError`] when source compatibility or old-baseline
    /// verification fails.
    pub fn plan<'a, I>(
        workspace_metadata: &WorkspaceMetadata,
        units: I,
        old_snapshot: &HxsSnapshot,
        new_snapshot: &HxsSnapshot,
    ) -> Result<RebasePlan, RebaseError>
    where
        I: IntoIterator<Item = &'a TranslationUnit>,
    {
        plan_rebase(workspace_metadata, units, old_snapshot, new_snapshot)
    }
}

/// Builds a deterministic source rebase plan.
///
/// This is the crate boundary used by `aeria-workspace` callers: pass
/// `workspace.metadata()` and `workspace.units()` without making the workspace
/// crate depend on rebase policy.
///
/// # Errors
///
/// Returns [`RebaseError`] when source compatibility or old-baseline
/// verification fails.
pub fn plan_rebase<'a, I>(
    workspace_metadata: &WorkspaceMetadata,
    units: I,
    old_snapshot: &HxsSnapshot,
    new_snapshot: &HxsSnapshot,
) -> Result<RebasePlan, RebaseError>
where
    I: IntoIterator<Item = &'a TranslationUnit>,
{
    let old_metadata = old_snapshot.metadata();
    let new_metadata = new_snapshot.metadata();
    validate_snapshot_preconditions(workspace_metadata, &old_metadata, &new_metadata)?;

    let mut units: Vec<_> = units.into_iter().collect();
    units.sort_unstable_by_key(|unit| unit.id());
    validate_workspace_view(&units)?;

    let mut states = Vec::with_capacity(units.len());
    for unit in units {
        verify_old_baseline(unit, old_snapshot)?;
        states.push(UnitState::new(unit));
    }

    // The old baseline is fully verified before this scan begins. The new
    // snapshot is read once through its bounded keyset API.
    let new_index = NewSourceIndex::read(new_snapshot)?;

    resolve_same_binding(&mut states, &new_index);
    record_candidate_stage(
        &mut states,
        &new_index,
        &new_index.by_fingerprint,
        CandidateEvidence::CompleteFingerprint,
        |fingerprint| Some(*fingerprint),
    );
    record_candidate_stage(
        &mut states,
        &new_index,
        &new_index.by_macro_and_raw,
        CandidateEvidence::MacroAndRawValue,
        |fingerprint| {
            fingerprint
                .raw_value_hash()
                .map(|raw| (fingerprint.macro_text_hash(), raw))
        },
    );
    record_candidate_stage(
        &mut states,
        &new_index,
        &new_index.by_macro_and_row,
        CandidateEvidence::MacroAndRowTechnical,
        |fingerprint| {
            Some((
                fingerprint.macro_text_hash(),
                fingerprint.row_technical_hash(),
            ))
        },
    );
    record_candidate_stage(
        &mut states,
        &new_index,
        &new_index.by_macro,
        CandidateEvidence::ExactMacroText,
        |fingerprint| Some(fingerprint.macro_text_hash()),
    );
    let entries: Vec<_> = states.into_iter().map(UnitState::finish).collect();
    let summary = summarize(&entries);
    Ok(RebasePlan {
        old_source_snapshot: SourceSnapshotIdentity::from_metadata(&old_metadata),
        new_source_snapshot: SourceSnapshotIdentity::from_metadata(&new_metadata),
        unit_entries: entries,
        summary,
    })
}

/// Short alias for callers that prefer a free `plan` function.
///
/// # Errors
///
/// Returns [`RebaseError`] when source compatibility or old-baseline
/// verification fails.
pub fn plan<'a, I>(
    workspace_metadata: &WorkspaceMetadata,
    units: I,
    old_snapshot: &HxsSnapshot,
    new_snapshot: &HxsSnapshot,
) -> Result<RebasePlan, RebaseError>
where
    I: IntoIterator<Item = &'a TranslationUnit>,
{
    plan_rebase(workspace_metadata, units, old_snapshot, new_snapshot)
}

fn validate_snapshot_preconditions(
    workspace_metadata: &WorkspaceMetadata,
    old_metadata: &SnapshotMetadata,
    new_metadata: &SnapshotMetadata,
) -> Result<(), RebaseError> {
    if old_metadata.source_language != workspace_metadata.source_language() {
        return Err(RebaseError::OldSourceLanguageMismatch {
            expected: workspace_metadata.source_language().to_owned(),
            found: old_metadata.source_language.clone(),
        });
    }
    if old_metadata.content_id != workspace_metadata.source_content_id() {
        return Err(RebaseError::OldContentIdMismatch {
            expected: workspace_metadata.source_content_id().to_owned(),
            found: old_metadata.content_id.clone(),
        });
    }
    if old_metadata.snapshot_id != workspace_metadata.source_snapshot_id() {
        return Err(RebaseError::OldSnapshotIdMismatch {
            expected: workspace_metadata.source_snapshot_id().to_owned(),
            found: old_metadata.snapshot_id.clone(),
        });
    }
    if new_metadata.source_language != workspace_metadata.source_language() {
        return Err(RebaseError::NewSourceLanguageMismatch {
            expected: workspace_metadata.source_language().to_owned(),
            found: new_metadata.source_language.clone(),
        });
    }
    if old_metadata.scope != new_metadata.scope {
        return Err(RebaseError::IncompatibleScope {
            old: old_metadata.scope.clone(),
            new: new_metadata.scope.clone(),
        });
    }
    Ok(())
}

fn validate_workspace_view(units: &[&TranslationUnit]) -> Result<(), RebaseError> {
    let mut ids = BTreeSet::new();
    let mut bindings = BTreeSet::new();
    for unit in units {
        if !ids.insert(unit.id()) {
            return Err(RebaseError::InvalidWorkspace {
                message: format!("duplicate translation-unit ID {}", unit.id()),
            });
        }
        if !bindings.insert(unit.source_binding().clone()) {
            return Err(RebaseError::InvalidWorkspace {
                message: format!(
                    "duplicate current source binding {:?}",
                    unit.source_binding()
                ),
            });
        }
    }
    Ok(())
}

fn verify_old_baseline(
    unit: &TranslationUnit,
    old_snapshot: &HxsSnapshot,
) -> Result<(), RebaseError> {
    let binding = unit.source_binding();
    let cell = old_snapshot
        .string_cell(
            binding.sheet_name(),
            binding.row_id(),
            binding.subrow_id(),
            binding.column_index(),
        )
        .map_err(|source| RebaseError::OldSourceRead {
            unit_id: unit.id(),
            binding: binding.clone(),
            source,
        })?
        .ok_or_else(|| RebaseError::OldOccurrenceMissing {
            unit_id: unit.id(),
            binding: binding.clone(),
        })?;
    let row = old_snapshot
        .row(binding.sheet_name(), binding.row_id(), binding.subrow_id())
        .map_err(|source| RebaseError::OldSourceRead {
            unit_id: unit.id(),
            binding: binding.clone(),
            source,
        })?
        .ok_or_else(|| RebaseError::OldOccurrenceMissing {
            unit_id: unit.id(),
            binding: binding.clone(),
        })?;
    let found = SourceFingerprint::new(
        Sha256Hash::from_bytes(*cell.hashes.macro_text.as_bytes()),
        cell.hashes
            .raw_value
            .map(|hash| Sha256Hash::from_bytes(*hash.as_bytes())),
        Sha256Hash::from_bytes(*row.hashes.technical.as_bytes()),
    );
    if found != *unit.source_fingerprint() {
        return Err(RebaseError::OldFingerprintMismatch {
            unit_id: unit.id(),
            binding: binding.clone(),
            expected: Box::new(*unit.source_fingerprint()),
            found: Box::new(found),
        });
    }
    Ok(())
}

#[derive(Clone)]
struct NewOccurrence {
    binding: SourceBinding,
    fingerprint: SourceFingerprint,
}

struct NewSourceIndex {
    occurrences: Vec<NewOccurrence>,
    by_binding: BTreeMap<SourceBinding, usize>,
    by_fingerprint: BTreeMap<SourceFingerprint, Vec<usize>>,
    by_macro_and_raw: BTreeMap<(Sha256Hash, Sha256Hash), Vec<usize>>,
    by_macro_and_row: BTreeMap<(Sha256Hash, Sha256Hash), Vec<usize>>,
    by_macro: BTreeMap<Sha256Hash, Vec<usize>>,
}

impl NewSourceIndex {
    fn read(snapshot: &HxsSnapshot) -> Result<Self, RebaseError> {
        let mut occurrences = Vec::new();
        let mut sheet_names: Vec<_> = snapshot
            .sheets()
            .into_iter()
            .map(|sheet| sheet.name)
            .collect();
        sheet_names.sort_unstable();

        for sheet_name in sheet_names {
            let mut after: Option<StringOccurrenceCoordinate> = None;
            loop {
                let page = snapshot
                    .page_string_occurrences(
                        &sheet_name,
                        after.as_ref(),
                        MAX_STRING_OCCURRENCE_PAGE_SIZE,
                    )
                    .map_err(RebaseError::NewSourceRead)?;
                for occurrence in page.occurrences {
                    occurrences.push(convert_occurrence(&occurrence));
                }
                match page.next_after {
                    Some(next) => after = Some(next),
                    None => break,
                }
            }
        }

        let mut index = Self {
            occurrences,
            by_binding: BTreeMap::new(),
            by_fingerprint: BTreeMap::new(),
            by_macro_and_raw: BTreeMap::new(),
            by_macro_and_row: BTreeMap::new(),
            by_macro: BTreeMap::new(),
        };
        for (index_number, occurrence) in index.occurrences.iter().enumerate() {
            index
                .by_binding
                .insert(occurrence.binding.clone(), index_number);
            index
                .by_fingerprint
                .entry(occurrence.fingerprint)
                .or_default()
                .push(index_number);
            if let Some(raw) = occurrence.fingerprint.raw_value_hash() {
                index
                    .by_macro_and_raw
                    .entry((occurrence.fingerprint.macro_text_hash(), raw))
                    .or_default()
                    .push(index_number);
            }
            index
                .by_macro_and_row
                .entry((
                    occurrence.fingerprint.macro_text_hash(),
                    occurrence.fingerprint.row_technical_hash(),
                ))
                .or_default()
                .push(index_number);
            index
                .by_macro
                .entry(occurrence.fingerprint.macro_text_hash())
                .or_default()
                .push(index_number);
        }
        Ok(index)
    }
}

fn convert_occurrence(occurrence: &StringOccurrenceFingerprint) -> NewOccurrence {
    NewOccurrence {
        binding: SourceBinding::new(
            occurrence.coordinate.sheet_name.clone(),
            occurrence.coordinate.row_id,
            occurrence.coordinate.subrow_id,
            occurrence.coordinate.column_index,
        ),
        fingerprint: SourceFingerprint::new(
            Sha256Hash::from_bytes(*occurrence.macro_text_hash.as_bytes()),
            occurrence
                .raw_value_hash
                .as_ref()
                .map(|hash| Sha256Hash::from_bytes(*hash.as_bytes())),
            Sha256Hash::from_bytes(*occurrence.row_technical_hash.as_bytes()),
        ),
    }
}

struct UnitState {
    id: TranslationUnitId,
    previous_binding: SourceBinding,
    previous_fingerprint: SourceFingerprint,
    outcome: Option<RebaseOutcome>,
    proposed: Option<(SourceBinding, SourceFingerprint)>,
    automatic_evidence: Option<AutomaticEvidence>,
    context_status: Option<SourceContextStatus>,
    candidate_evidence: Vec<CandidateEvidenceSet>,
}

impl UnitState {
    fn new(unit: &TranslationUnit) -> Self {
        Self {
            id: unit.id(),
            previous_binding: unit.source_binding().clone(),
            previous_fingerprint: *unit.source_fingerprint(),
            outcome: None,
            proposed: None,
            automatic_evidence: None,
            context_status: None,
            candidate_evidence: Vec::new(),
        }
    }

    fn resolve(&mut self, occurrence: &NewOccurrence) {
        debug_assert_eq!(self.previous_binding, occurrence.binding);
        let content_unchanged =
            source_content_equal(&self.previous_fingerprint, &occurrence.fingerprint);
        self.outcome = Some(if content_unchanged {
            RebaseOutcome::Unchanged
        } else {
            RebaseOutcome::SourceChanged
        });
        self.proposed = Some((occurrence.binding.clone(), occurrence.fingerprint));
        self.automatic_evidence = Some(AutomaticEvidence::SameBinding);
        self.context_status = Some(source_context_status(
            &self.previous_fingerprint,
            &occurrence.fingerprint,
        ));
    }

    fn record_candidates(
        &mut self,
        candidates: &[usize],
        index: &NewSourceIndex,
        evidence: CandidateEvidence,
    ) {
        if candidates.is_empty() {
            return;
        }
        let retained = candidates.iter().take(MAX_REPORTED_CANDIDATES);
        let candidate_bindings = retained
            .map(|candidate| index.occurrences[*candidate].binding.clone())
            .collect();
        self.candidate_evidence.push(CandidateEvidenceSet {
            evidence,
            candidate_bindings,
            candidate_count: candidates.len(),
            candidates_truncated: candidates.len() > MAX_REPORTED_CANDIDATES,
        });
    }

    fn finish(self) -> UnitRebasePlan {
        let outcome = self.outcome.unwrap_or(RebaseOutcome::Ambiguous);
        let (proposed_source_binding, proposed_source_fingerprint) = self
            .proposed
            .map_or((None, None), |(binding, fingerprint)| {
                (Some(binding), Some(fingerprint))
            });
        UnitRebasePlan {
            translation_unit_id: self.id,
            previous_source_binding: self.previous_binding,
            previous_source_fingerprint: self.previous_fingerprint,
            outcome,
            proposed_source_binding,
            proposed_source_fingerprint,
            automatic_evidence: self.automatic_evidence,
            context_status: self.context_status,
            candidate_evidence: self.candidate_evidence,
        }
    }
}

fn resolve_same_binding(states: &mut [UnitState], index: &NewSourceIndex) {
    for state in states.iter_mut() {
        let Some(&occurrence_index) = index.by_binding.get(&state.previous_binding) else {
            continue;
        };
        let occurrence = &index.occurrences[occurrence_index];
        state.resolve(occurrence);
    }
}

fn source_content_equal(old: &SourceFingerprint, new: &SourceFingerprint) -> bool {
    SourceContent::from_fingerprint(old) == SourceContent::from_fingerprint(new)
}

fn source_context_status(old: &SourceFingerprint, new: &SourceFingerprint) -> SourceContextStatus {
    if old.row_technical_hash() == new.row_technical_hash() {
        SourceContextStatus::Unchanged
    } else {
        SourceContextStatus::Changed
    }
}

fn record_candidate_stage<K, F>(
    states: &mut [UnitState],
    new_index: &NewSourceIndex,
    candidates_by_key: &BTreeMap<K, Vec<usize>>,
    evidence: CandidateEvidence,
    key_for_unit: F,
) where
    K: Ord,
    F: Fn(&SourceFingerprint) -> Option<K>,
{
    let mut groups: BTreeMap<K, Vec<usize>> = BTreeMap::new();
    for (state_index, state) in states.iter().enumerate() {
        if state.outcome.is_none()
            && let Some(key) = key_for_unit(&state.previous_fingerprint)
        {
            groups.entry(key).or_default().push(state_index);
        }
    }

    for (key, old_units) in groups {
        let available: Vec<_> = candidates_by_key
            .get(&key)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        for state_index in old_units {
            states[state_index].record_candidates(&available, new_index, evidence);
        }
    }
}

fn summarize(entries: &[UnitRebasePlan]) -> RebaseSummary {
    let mut summary = RebaseSummary::default();
    for entry in entries {
        match entry.outcome {
            RebaseOutcome::Unchanged => summary.unchanged += 1,
            RebaseOutcome::SourceChanged => summary.source_changed += 1,
            RebaseOutcome::Ambiguous => summary.ambiguous += 1,
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeria_hxs::{ProducerMetadata, SnapshotCounts};

    #[test]
    fn incompatible_scopes_are_rejected_before_matching() {
        let workspace_metadata =
            WorkspaceMetadata::new("en", "fr", "content", "snapshot").expect("metadata");
        let metadata = |scope: &str| SnapshotMetadata {
            format_version: 1,
            game_version: "game".into(),
            source_language: "en".into(),
            scope: scope.into(),
            content_id: "content".into(),
            snapshot_id: "snapshot".into(),
            producer: ProducerMetadata {
                extractor_version: "test".into(),
                lumina_version: "test".into(),
            },
            counts: SnapshotCounts {
                sheets: 0,
                rows: 0,
                string_cells: 0,
            },
        };
        assert!(matches!(
            validate_snapshot_preconditions(
                &workspace_metadata,
                &metadata("full"),
                &metadata("partial"),
            ),
            Err(RebaseError::IncompatibleScope { .. })
        ));
    }

    #[test]
    fn candidate_evidence_is_non_authoritative_and_keeps_multiple_strengths() {
        let binding = SourceBinding::new("Sheet", 1, 0, 0);
        let macro_hash = Sha256Hash::from_bytes([1; 32]);
        let raw_hash = Sha256Hash::from_bytes([2; 32]);
        let fingerprint =
            SourceFingerprint::new(macro_hash, Some(raw_hash), Sha256Hash::from_bytes([3; 32]));
        let unit = TranslationUnit::new(
            TranslationUnitId::from_bytes([4; 32]),
            binding.clone(),
            fingerprint,
            "",
        );
        let mut states = vec![UnitState::new(&unit)];
        let index = NewSourceIndex {
            occurrences: vec![
                NewOccurrence {
                    binding: SourceBinding::new("Sheet", 1, 0, 1),
                    fingerprint,
                },
                NewOccurrence {
                    binding: SourceBinding::new("Sheet", 1, 0, 2),
                    fingerprint,
                },
                NewOccurrence {
                    binding: SourceBinding::new("Sheet", 1, 0, 3),
                    fingerprint: SourceFingerprint::new(
                        macro_hash,
                        Some(Sha256Hash::from_bytes([9; 32])),
                        fingerprint.row_technical_hash(),
                    ),
                },
            ],
            by_binding: BTreeMap::new(),
            by_fingerprint: BTreeMap::new(),
            by_macro_and_raw: BTreeMap::new(),
            by_macro_and_row: BTreeMap::new(),
            by_macro: BTreeMap::new(),
        };
        let mut complete_candidates = BTreeMap::new();
        complete_candidates.insert(fingerprint, vec![0, 1]);

        record_candidate_stage(
            &mut states,
            &index,
            &complete_candidates,
            CandidateEvidence::CompleteFingerprint,
            |previous| Some(*previous),
        );

        let mut weaker_candidates = BTreeMap::new();
        weaker_candidates.insert(macro_hash, vec![2]);
        record_candidate_stage(
            &mut states,
            &index,
            &weaker_candidates,
            CandidateEvidence::ExactMacroText,
            |previous| Some(previous.macro_text_hash()),
        );

        assert_eq!(states[0].outcome, None);
        assert_eq!(states[0].automatic_evidence, None);
        assert_eq!(states[0].candidate_evidence.len(), 2);
        assert_eq!(
            states[0].candidate_evidence[0].candidate_bindings,
            vec![
                SourceBinding::new("Sheet", 1, 0, 1),
                SourceBinding::new("Sheet", 1, 0, 2),
            ]
        );
        assert_eq!(
            states[0].candidate_evidence[1].evidence,
            CandidateEvidence::ExactMacroText
        );
    }
}
