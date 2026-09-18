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
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RebaseOutcome {
    /// The coordinate and complete source fingerprint are unchanged.
    Unchanged,
    /// The coordinate moved but the complete source fingerprint is unchanged.
    Relocated,
    /// A safe coordinate was found, but the complete source fingerprint changed.
    SourceChanged,
    /// No safe one-to-one identity decision was established.
    Ambiguous,
}

/// The exact deterministic evidence used for a proposed binding.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MatchEvidence {
    /// The new snapshot contains the old coordinate.
    SameBinding,
    /// The complete source fingerprint matched.
    CompleteFingerprint,
    /// Macro-text and the old optional raw-value hash matched.
    MacroAndRawValue,
    /// Macro-text and row technical hashes matched.
    MacroAndRowTechnical,
    /// The macro-text hash was globally unique for this stage.
    UniqueMacroText,
    /// No candidate evidence established a safe mapping.
    None,
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
    pub matching_evidence: MatchEvidence,
    /// A bounded, canonical diagnostic list. Matching semantics never depend
    /// on this list being complete.
    pub candidate_bindings: Vec<SourceBinding>,
    /// Number of available candidates before diagnostic truncation.
    pub candidate_count: usize,
    pub candidates_truncated: bool,
}

/// Backward-friendly name for consumers that call an entry a unit plan entry.
pub type UnitPlanEntry = UnitRebasePlan;

/// Deterministic counts derived from the plan entries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RebaseSummary {
    pub unchanged: usize,
    pub relocated: usize,
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
    let mut claimed = BTreeSet::new();

    resolve_same_binding(&mut states, &new_index, &mut claimed);
    resolve_stage(
        &mut states,
        &mut claimed,
        &new_index,
        &new_index.by_fingerprint,
        MatchEvidence::CompleteFingerprint,
        |fingerprint| Some(*fingerprint),
    );
    resolve_stage(
        &mut states,
        &mut claimed,
        &new_index,
        &new_index.by_macro_and_raw,
        MatchEvidence::MacroAndRawValue,
        |fingerprint| {
            fingerprint
                .raw_value_hash()
                .map(|raw| (fingerprint.macro_text_hash(), raw))
        },
    );
    resolve_stage(
        &mut states,
        &mut claimed,
        &new_index,
        &new_index.by_macro_and_row,
        MatchEvidence::MacroAndRowTechnical,
        |fingerprint| {
            Some((
                fingerprint.macro_text_hash(),
                fingerprint.row_technical_hash(),
            ))
        },
    );
    resolve_stage(
        &mut states,
        &mut claimed,
        &new_index,
        &new_index.by_macro,
        MatchEvidence::UniqueMacroText,
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
    evidence: MatchEvidence,
    candidate_bindings: Vec<SourceBinding>,
    candidate_count: usize,
    candidates_truncated: bool,
}

impl UnitState {
    fn new(unit: &TranslationUnit) -> Self {
        Self {
            id: unit.id(),
            previous_binding: unit.source_binding().clone(),
            previous_fingerprint: *unit.source_fingerprint(),
            outcome: None,
            proposed: None,
            evidence: MatchEvidence::None,
            candidate_bindings: Vec::new(),
            candidate_count: 0,
            candidates_truncated: false,
        }
    }

    fn resolve(&mut self, occurrence: &NewOccurrence, evidence: MatchEvidence) {
        self.outcome = Some(if self.previous_fingerprint == occurrence.fingerprint {
            if self.previous_binding == occurrence.binding {
                RebaseOutcome::Unchanged
            } else {
                RebaseOutcome::Relocated
            }
        } else {
            RebaseOutcome::SourceChanged
        });
        self.proposed = Some((occurrence.binding.clone(), occurrence.fingerprint));
        self.evidence = evidence;
    }

    fn record_candidates(
        &mut self,
        candidates: &[usize],
        index: &NewSourceIndex,
        evidence: MatchEvidence,
    ) {
        if self.evidence != MatchEvidence::None || candidates.is_empty() {
            return;
        }
        self.evidence = evidence;
        self.candidate_count = candidates.len();
        let retained = candidates.iter().take(MAX_REPORTED_CANDIDATES);
        self.candidate_bindings = retained
            .map(|candidate| index.occurrences[*candidate].binding.clone())
            .collect();
        self.candidates_truncated = self.candidate_count > self.candidate_bindings.len();
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
            matching_evidence: self.evidence,
            candidate_bindings: self.candidate_bindings,
            candidate_count: self.candidate_count,
            candidates_truncated: self.candidates_truncated,
        }
    }
}

fn resolve_same_binding(
    states: &mut [UnitState],
    index: &NewSourceIndex,
    claimed: &mut BTreeSet<usize>,
) {
    let mut groups: BTreeMap<SourceBinding, Vec<usize>> = BTreeMap::new();
    for (state_index, state) in states.iter().enumerate() {
        if state.outcome.is_none() {
            groups
                .entry(state.previous_binding.clone())
                .or_default()
                .push(state_index);
        }
    }

    for (binding, old_units) in groups {
        let available: Vec<_> = index
            .by_binding
            .get(&binding)
            .into_iter()
            .copied()
            .filter(|candidate| !claimed.contains(candidate))
            .collect();
        if old_units.len() == 1 && available.len() == 1 {
            let state_index = old_units[0];
            let new_index = available[0];
            claimed.insert(new_index);
            states[state_index].resolve(&index.occurrences[new_index], MatchEvidence::SameBinding);
        } else {
            for state_index in old_units {
                states[state_index].record_candidates(
                    &available,
                    index,
                    MatchEvidence::SameBinding,
                );
            }
        }
    }
}

fn resolve_stage<K, F>(
    states: &mut [UnitState],
    claimed: &mut BTreeSet<usize>,
    new_index: &NewSourceIndex,
    candidates_by_key: &BTreeMap<K, Vec<usize>>,
    evidence: MatchEvidence,
    key_for_unit: F,
) where
    K: Ord,
    F: Fn(&SourceFingerprint) -> Option<K>,
{
    let mut groups: BTreeMap<K, Vec<usize>> = BTreeMap::new();
    for (state_index, state) in states.iter().enumerate() {
        if state.outcome.is_none()
            && state.evidence == MatchEvidence::None
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
            .filter(|candidate| !claimed.contains(candidate))
            .collect();
        if old_units.len() == 1 && available.len() == 1 {
            let state_index = old_units[0];
            let occurrence_index = available[0];
            claimed.insert(occurrence_index);
            states[state_index].resolve(&new_index.occurrences[occurrence_index], evidence);
        } else {
            for state_index in old_units {
                states[state_index].record_candidates(&available, new_index, evidence);
            }
        }
    }
}

fn summarize(entries: &[UnitRebasePlan]) -> RebaseSummary {
    let mut summary = RebaseSummary::default();
    for entry in entries {
        match entry.outcome {
            RebaseOutcome::Unchanged => summary.unchanged += 1,
            RebaseOutcome::Relocated => summary.relocated += 1,
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
    fn same_binding_stage_does_not_claim_one_occurrence_twice() {
        let binding = SourceBinding::new("Sheet", 1, 0, 0);
        let fingerprint = SourceFingerprint::new(
            Sha256Hash::from_bytes([1; 32]),
            None,
            Sha256Hash::from_bytes([2; 32]),
        );
        let units = [
            TranslationUnit::new(
                TranslationUnitId::from_bytes([1; 32]),
                binding.clone(),
                fingerprint,
                "",
            ),
            TranslationUnit::new(
                TranslationUnitId::from_bytes([2; 32]),
                binding.clone(),
                fingerprint,
                "",
            ),
        ];
        let mut states: Vec<_> = units.iter().map(UnitState::new).collect();
        let index = NewSourceIndex {
            occurrences: vec![NewOccurrence {
                binding: binding.clone(),
                fingerprint,
            }],
            by_binding: BTreeMap::from([(binding, 0)]),
            by_fingerprint: BTreeMap::new(),
            by_macro_and_raw: BTreeMap::new(),
            by_macro_and_row: BTreeMap::new(),
            by_macro: BTreeMap::new(),
        };
        let mut claimed = BTreeSet::new();

        resolve_same_binding(&mut states, &index, &mut claimed);

        assert!(claimed.is_empty());
        assert!(states.iter().all(|state| state.outcome.is_none()));
        assert!(states.iter().all(|state| {
            state.evidence == MatchEvidence::SameBinding
                && state.candidate_count == 1
                && state.proposed.is_none()
        }));
    }

    #[test]
    fn ambiguous_complete_fingerprint_is_frozen_before_weaker_stages() {
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
        let mut claimed = BTreeSet::new();
        let mut complete_candidates = BTreeMap::new();
        complete_candidates.insert(fingerprint, vec![0, 1]);

        resolve_stage(
            &mut states,
            &mut claimed,
            &index,
            &complete_candidates,
            MatchEvidence::CompleteFingerprint,
            |previous| Some(*previous),
        );

        let mut weaker_candidates = BTreeMap::new();
        weaker_candidates.insert(macro_hash, vec![2]);
        resolve_stage(
            &mut states,
            &mut claimed,
            &index,
            &weaker_candidates,
            MatchEvidence::UniqueMacroText,
            |previous| Some(previous.macro_text_hash()),
        );

        assert!(claimed.is_empty());
        assert_eq!(states[0].outcome, None);
        assert_eq!(states[0].evidence, MatchEvidence::CompleteFingerprint);
        assert_eq!(states[0].candidate_count, 2);
        assert_eq!(
            states[0].candidate_bindings,
            vec![
                SourceBinding::new("Sheet", 1, 0, 1),
                SourceBinding::new("Sheet", 1, 0, 2),
            ]
        );
    }
}
