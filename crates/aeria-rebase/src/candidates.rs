//! Bounded, deterministic candidate suggestions for missing source bindings.
//!
//! This module is review assistance only. It does not establish source
//! identity, mutate a [`TranslationUnit`], update workspace state, or produce
//! a [`RebaseOutcome`]. The rebase planner remains the only authority for
//! source transition facts.

#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use aeria_core::{
    Sha256Hash, SourceBinding, SourceFingerprint, TranslationUnit, TranslationUnitId,
};
use aeria_hxs::{
    HxsError, HxsSnapshot, MAX_STRING_OCCURRENCE_PAGE_SIZE, SnapshotMetadata,
    StringOccurrenceCoordinate,
};
use aeria_se::{SemanticAnalysis, StructureCompatibility, TextRangeKind};
use thiserror::Error;

use crate::CandidateEvidence;

/// The default number of suggestions returned by one query.
pub const DEFAULT_RESULT_LIMIT: usize = 10;

/// The largest result set a query can request.
pub const MAX_RESULT_LIMIT: usize = 50;

/// The largest bounded pool ranked for one query.
pub const MAX_GENERATED_CANDIDATES: usize = 128;

/// The maximum number of edit-distance matrix cells evaluated for one
/// candidate. Larger inputs use the deterministic bounded fallback scorer.
pub const MAX_EDIT_MATRIX_CELLS: usize = 65_536;

const COORDINATE_NEIGHBOR_RADIUS: usize = 32;
const MAX_NORMALIZED_TEXT_SCALARS: usize = 4096;

/// A request for suggestions for one existing, previously managed unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateQuery {
    /// The durable unit whose missing old binding needs review assistance.
    pub translation_unit_id: TranslationUnitId,
    /// Requested top-K result count. Values above [`MAX_RESULT_LIMIT`] are
    /// clamped to preserve the bounded interface.
    pub limit: usize,
}

impl CandidateQuery {
    /// Creates a query using the normal result limit.
    #[must_use]
    pub const fn new(translation_unit_id: TranslationUnitId) -> Self {
        Self {
            translation_unit_id,
            limit: DEFAULT_RESULT_LIMIT,
        }
    }

    /// Returns the bounded result limit used by the suggester.
    #[must_use]
    pub const fn effective_limit(self) -> usize {
        if self.limit > MAX_RESULT_LIMIT {
            MAX_RESULT_LIMIT
        } else {
            self.limit
        }
    }
}

/// Errors raised while preparing or evaluating candidate suggestions.
#[derive(Debug, Error)]
pub enum CandidateSuggestionError {
    /// The query ID and supplied managed unit do not agree.
    #[error("candidate query targets translation unit {query_id}, but supplied unit is {unit_id}")]
    QueryUnitMismatch {
        query_id: TranslationUnitId,
        unit_id: TranslationUnitId,
    },

    /// The old source cell needed for visible-text ranking could not be read.
    #[error("could not read old source for translation unit {unit_id} at {binding:?}: {source}")]
    OldSourceRead {
        unit_id: TranslationUnitId,
        binding: SourceBinding,
        #[source]
        source: HxsError,
    },

    /// The old source binding is absent from the source snapshot.
    #[error("old source is missing for translation unit {unit_id} at {binding:?}")]
    OldSourceMissing {
        unit_id: TranslationUnitId,
        binding: SourceBinding,
    },

    /// A verified new occurrence disappeared while its payload was fetched.
    #[error("new source occurrence is missing at {binding:?}")]
    NewSourceMissing { binding: SourceBinding },

    /// A payload read from the verified new snapshot failed.
    #[error("could not read new source at {binding:?}: {source}")]
    NewSourceRead {
        binding: SourceBinding,
        #[source]
        source: HxsError,
    },

    /// The payload snapshot does not match the snapshot used to build the
    /// lightweight candidate index.
    #[error(
        "candidate payload snapshot does not match index snapshot: expected {expected_snapshot_id:?}, found {found_snapshot_id:?}"
    )]
    NewSnapshotMismatch {
        expected_snapshot_id: String,
        found_snapshot_id: String,
    },

    /// The old snapshot does not satisfy the planner's persisted baseline
    /// fingerprint contract for the managed unit.
    #[error("old source baseline verification failed: {0}")]
    OldBaselineVerification(#[source] crate::RebaseError),

    /// Building the bounded source index failed.
    #[error("could not enumerate new HXS String occurrences: {0}")]
    IndexRead(#[source] HxsError),
}

/// A deterministic protected-structure comparison used as ranking evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ProtectedStructureMatch {
    /// Both macro strings are well formed and have equivalent protected data.
    Compatible,
    /// At least one macro string is malformed, so structure cannot be trusted.
    NotComparable,
    /// Both inputs are comparable but their protected data differs.
    Incompatible,
}

impl ProtectedStructureMatch {
    const fn rank(self) -> u8 {
        match self {
            Self::Compatible => 2,
            Self::NotComparable => 1,
            Self::Incompatible => 0,
        }
    }
}

/// The exact source-content evidence represented in a ranking score.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CandidateExactness {
    /// No exact macro-content evidence.
    None,
    /// The complete macro-text hash is equal.
    MacroText,
    /// Macro text and an old present raw-value hash are equal.
    MacroAndRaw,
}

/// A deterministic ranking score.
///
/// This is a ranking value only. It is not a probability, confidence value,
/// or automatic acceptance threshold, and it never establishes source
/// identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateScore {
    /// Exact source-content evidence, if any.
    pub exactness: CandidateExactness,
    /// Conservative protected-structure comparison result.
    pub protected_structure: ProtectedStructureMatch,
    /// Normalized visible-text edit similarity in the inclusive range 0..=1000.
    pub visible_text_similarity: u16,
    /// Whether both occurrences are in the same sheet.
    pub same_sheet: bool,
    /// Whether both occurrences use the same column index.
    pub same_column: bool,
    /// Canonical coordinate distance. Lower is better; it is only a ranking hint.
    pub coordinate_distance: u64,
}

impl CandidateScore {
    fn exact_rank(self) -> u8 {
        match self.exactness {
            CandidateExactness::None => 0,
            CandidateExactness::MacroText => 1,
            CandidateExactness::MacroAndRaw => 2,
        }
    }
}

impl Ord for CandidateScore {
    fn cmp(&self, other: &Self) -> Ordering {
        self.exact_rank()
            .cmp(&other.exact_rank())
            .then_with(|| self.exactness.cmp(&other.exactness))
            .then_with(|| {
                self.protected_structure
                    .rank()
                    .cmp(&other.protected_structure.rank())
            })
            .then_with(|| {
                self.visible_text_similarity
                    .cmp(&other.visible_text_similarity)
            })
            .then_with(|| self.same_sheet.cmp(&other.same_sheet))
            .then_with(|| self.same_column.cmp(&other.same_column))
            .then_with(|| other.coordinate_distance.cmp(&self.coordinate_distance))
    }
}

impl PartialOrd for CandidateScore {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One non-authoritative source candidate for human review.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCandidate {
    /// The candidate's source coordinate. This is not an authoritative binding
    /// for the queried translation unit.
    pub binding: SourceBinding,
    /// Verified facts at the candidate coordinate.
    pub fingerprint: SourceFingerprint,
    /// The strongest deterministic evidence represented by this candidate.
    pub evidence: CandidateEvidence,
    /// Disposable deterministic ranking value, never identity authority.
    pub score: CandidateScore,
}

/// An owned candidate suggester prepared from one verified new snapshot.
///
/// Preparing once and querying many times keeps source enumeration independent
/// of the number of ambiguous units. The prepared index stores lightweight
/// coordinates and hashes; full macro/raw payloads are fetched only for the
/// bounded pool selected for one query.
#[derive(Clone, Debug)]
pub struct CandidateSuggester {
    index: CandidateIndex,
    new_snapshot_metadata: SnapshotMetadata,
}

impl CandidateSuggester {
    /// Builds a bounded-query index from a verified new snapshot.
    ///
    /// The scan is paged by sheet and keyset coordinate. It is performed once
    /// for this suggester, not once per ambiguous translation unit.
    ///
    /// # Errors
    ///
    /// Returns an error when the verified snapshot cannot be enumerated.
    pub fn from_snapshot(snapshot: &HxsSnapshot) -> Result<Self, CandidateSuggestionError> {
        Ok(Self {
            index: CandidateIndex::read(snapshot).map_err(CandidateSuggestionError::IndexRead)?,
            new_snapshot_metadata: snapshot.metadata(),
        })
    }

    /// Suggests candidates for one managed unit without changing any state.
    ///
    /// A unit whose old binding survives in the prepared new snapshot receives
    /// no suggestions and never competes with other coordinates. The intended
    /// caller invokes this for an `Ambiguous` planner entry. The method accepts
    /// the unit directly so the candidate result cannot be mistaken for an
    /// authoritative planner transition.
    ///
    /// `old_snapshot` supplies the old macro representation for the fuzzy
    /// projection. `new_snapshot` is queried only for the bounded candidate
    /// payloads selected by the prepared index.
    ///
    /// # Errors
    ///
    /// Returns an error for a mismatched query/unit ID or an unexpected source
    /// read failure.
    pub fn suggest(
        &self,
        query: CandidateQuery,
        unit: &TranslationUnit,
        old_snapshot: &HxsSnapshot,
        new_snapshot: &HxsSnapshot,
    ) -> Result<Vec<SourceCandidate>, CandidateSuggestionError> {
        if query.translation_unit_id != unit.id() {
            return Err(CandidateSuggestionError::QueryUnitMismatch {
                query_id: query.translation_unit_id,
                unit_id: unit.id(),
            });
        }

        let found_snapshot_metadata = new_snapshot.metadata();
        if found_snapshot_metadata != self.new_snapshot_metadata {
            return Err(CandidateSuggestionError::NewSnapshotMismatch {
                expected_snapshot_id: self.new_snapshot_metadata.snapshot_id.clone(),
                found_snapshot_id: found_snapshot_metadata.snapshot_id,
            });
        }

        let limit = query.effective_limit();
        if limit == 0 || self.index.contains_binding(unit.source_binding()) {
            return Ok(Vec::new());
        }

        crate::verify_old_baseline(unit, old_snapshot)
            .map_err(CandidateSuggestionError::OldBaselineVerification)?;

        let old_cell = old_snapshot
            .string_cell(
                unit.source_binding().sheet_name(),
                unit.source_binding().row_id(),
                unit.source_binding().subrow_id(),
                unit.source_binding().column_index(),
            )
            .map_err(|source| CandidateSuggestionError::OldSourceRead {
                unit_id: unit.id(),
                binding: unit.source_binding().clone(),
                source,
            })?
            .ok_or_else(|| CandidateSuggestionError::OldSourceMissing {
                unit_id: unit.id(),
                binding: unit.source_binding().clone(),
            })?;
        let old_prepared = PreparedSource::new(&old_cell.macro_text);
        let pool = self
            .index
            .generate_pool(unit.source_binding(), unit.source_fingerprint());
        let mut prepared_candidates = Vec::with_capacity(pool.len());

        for occurrence_index in pool {
            let occurrence = &self.index.occurrences[occurrence_index];
            let cell = new_snapshot
                .string_cell(
                    occurrence.binding.sheet_name(),
                    occurrence.binding.row_id(),
                    occurrence.binding.subrow_id(),
                    occurrence.binding.column_index(),
                )
                .map_err(|source| CandidateSuggestionError::NewSourceRead {
                    binding: occurrence.binding.clone(),
                    source,
                })?
                .ok_or_else(|| CandidateSuggestionError::NewSourceMissing {
                    binding: occurrence.binding.clone(),
                })?;
            prepared_candidates.push((occurrence.clone(), PreparedSource::new(&cell.macro_text)));
        }

        let mut candidates = rank_candidates(
            unit.source_binding(),
            unit.source_fingerprint(),
            &old_prepared,
            prepared_candidates,
        );
        candidates.truncate(limit);
        Ok(candidates)
    }
}

#[derive(Clone, Debug)]
struct IndexedOccurrence {
    binding: SourceBinding,
    fingerprint: SourceFingerprint,
}

#[derive(Clone, Debug, Default)]
struct CandidateIndex {
    occurrences: Vec<IndexedOccurrence>,
    by_binding: BTreeMap<SourceBinding, usize>,
    by_fingerprint: BTreeMap<SourceFingerprint, Vec<usize>>,
    by_macro_and_raw: BTreeMap<(Sha256Hash, Sha256Hash), Vec<usize>>,
    by_macro_text: BTreeMap<Sha256Hash, Vec<usize>>,
    by_sheet_column: BTreeMap<(String, u32), Vec<usize>>,
}

impl CandidateIndex {
    fn read(snapshot: &HxsSnapshot) -> Result<Self, HxsError> {
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
                let page = snapshot.page_string_occurrences(
                    &sheet_name,
                    after.as_ref(),
                    MAX_STRING_OCCURRENCE_PAGE_SIZE,
                )?;
                occurrences.extend(page.occurrences.into_iter().map(|occurrence| {
                    IndexedOccurrence {
                        binding: SourceBinding::new(
                            occurrence.coordinate.sheet_name,
                            occurrence.coordinate.row_id,
                            occurrence.coordinate.subrow_id,
                            occurrence.coordinate.column_index,
                        ),
                        fingerprint: SourceFingerprint::new(
                            Sha256Hash::from_bytes(*occurrence.macro_text_hash.as_bytes()),
                            occurrence
                                .raw_value_hash
                                .map(|hash| Sha256Hash::from_bytes(*hash.as_bytes())),
                            Sha256Hash::from_bytes(*occurrence.row_technical_hash.as_bytes()),
                        ),
                    }
                }));
                match page.next_after {
                    Some(next) => after = Some(next),
                    None => break,
                }
            }
        }

        let mut index = Self {
            occurrences,
            ..Self::default()
        };
        for (occurrence_index, occurrence) in index.occurrences.iter().enumerate() {
            index
                .by_binding
                .insert(occurrence.binding.clone(), occurrence_index);
            index
                .by_fingerprint
                .entry(occurrence.fingerprint)
                .or_default()
                .push(occurrence_index);
            if let Some(raw_hash) = occurrence.fingerprint.raw_value_hash() {
                index
                    .by_macro_and_raw
                    .entry((occurrence.fingerprint.macro_text_hash(), raw_hash))
                    .or_default()
                    .push(occurrence_index);
            }
            index
                .by_macro_text
                .entry(occurrence.fingerprint.macro_text_hash())
                .or_default()
                .push(occurrence_index);
            index
                .by_sheet_column
                .entry((
                    occurrence.binding.sheet_name().to_owned(),
                    occurrence.binding.column_index(),
                ))
                .or_default()
                .push(occurrence_index);
        }
        Ok(index)
    }

    fn contains_binding(&self, binding: &SourceBinding) -> bool {
        self.by_binding.contains_key(binding)
    }

    fn generate_pool(
        &self,
        binding: &SourceBinding,
        fingerprint: &SourceFingerprint,
    ) -> Vec<usize> {
        let mut pool = Vec::with_capacity(MAX_GENERATED_CANDIDATES);
        let mut seen = BTreeSet::new();
        let mut add = |indices: &[usize]| {
            for &index in indices {
                if pool.len() == MAX_GENERATED_CANDIDATES {
                    return;
                }
                if seen.insert(index) {
                    pool.push(index);
                }
            }
        };

        if let Some(indices) = self.by_fingerprint.get(fingerprint) {
            add(indices);
        }
        if let Some(raw_hash) = fingerprint.raw_value_hash()
            && let Some(indices) = self
                .by_macro_and_raw
                .get(&(fingerprint.macro_text_hash(), raw_hash))
        {
            add(indices);
        }
        if let Some(indices) = self.by_macro_text.get(&fingerprint.macro_text_hash()) {
            add(indices);
        }

        if let Some(indices) = self
            .by_sheet_column
            .get(&(binding.sheet_name().to_owned(), binding.column_index()))
        {
            let pivot = indices.partition_point(|&index| {
                let candidate = &self.occurrences[index].binding;
                (candidate.row_id(), candidate.subrow_id())
                    < (binding.row_id(), binding.subrow_id())
            });
            let start = pivot.saturating_sub(COORDINATE_NEIGHBOR_RADIUS);
            let end = pivot
                .saturating_add(COORDINATE_NEIGHBOR_RADIUS)
                .min(indices.len());
            add(&indices[start..end]);
        }
        pool
    }
}

struct PreparedSource {
    analysis: SemanticAnalysis,
    visible_scalars: Vec<char>,
}

fn rank_candidates(
    old_binding: &SourceBinding,
    old_fingerprint: &SourceFingerprint,
    old_source: &PreparedSource,
    candidates: Vec<(IndexedOccurrence, PreparedSource)>,
) -> Vec<SourceCandidate> {
    let mut ranked = candidates
        .into_iter()
        .map(|(occurrence, prepared)| {
            let exact_macro_and_raw = old_fingerprint
                .raw_value_hash()
                .zip(occurrence.fingerprint.raw_value_hash())
                .is_some_and(|(old, new)| {
                    old == new
                        && old_fingerprint.macro_text_hash()
                            == occurrence.fingerprint.macro_text_hash()
                });
            let exact_macro_text =
                old_fingerprint.macro_text_hash() == occurrence.fingerprint.macro_text_hash();
            let structure = structure_match(&old_source.analysis, &prepared.analysis);
            let same_sheet = old_binding.sheet_name() == occurrence.binding.sheet_name();
            let same_column = old_binding.column_index() == occurrence.binding.column_index();
            let exactness = if exact_macro_and_raw {
                CandidateExactness::MacroAndRaw
            } else if exact_macro_text {
                CandidateExactness::MacroText
            } else {
                CandidateExactness::None
            };
            let score = CandidateScore {
                exactness,
                protected_structure: structure,
                visible_text_similarity: normalized_edit_similarity(
                    &old_source.visible_scalars,
                    &prepared.visible_scalars,
                ),
                same_sheet,
                same_column,
                coordinate_distance: coordinate_distance(old_binding, &occurrence.binding),
            };
            let evidence = if old_fingerprint == &occurrence.fingerprint {
                CandidateEvidence::CompleteFingerprint
            } else if exact_macro_and_raw {
                CandidateEvidence::MacroAndRawValue
            } else if exact_macro_text {
                CandidateEvidence::ExactMacroText
            } else if structure == ProtectedStructureMatch::Compatible {
                CandidateEvidence::ProtectedStructureCompatible
            } else {
                CandidateEvidence::VisibleTextSimilarity
            };
            SourceCandidate {
                binding: occurrence.binding,
                fingerprint: occurrence.fingerprint,
                evidence,
                score,
            }
        })
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.binding.cmp(&right.binding))
    });
    ranked
}

impl PreparedSource {
    fn new(macro_text: &str) -> Self {
        let document = aeria_se::parse(macro_text);
        let analysis = document.semantic_analysis();
        let mut visible = String::new();
        for range in analysis.translatable_text_ranges() {
            if let Some(spelling) = document.slice(range.span) {
                if matches!(range.kind, TextRangeKind::Escape) {
                    visible.push_str(spelling.strip_prefix('\\').unwrap_or(spelling));
                } else {
                    visible.push_str(spelling);
                }
            }
        }
        let visible_text = normalize_visible_text(&visible);
        let visible_scalars = visible_text.chars().collect();
        Self {
            analysis,
            visible_scalars,
        }
    }
}

fn normalize_visible_text(value: &str) -> String {
    let mut normalized = String::new();
    let mut pending_space = false;
    let mut scalar_count = 0;
    for character in value.chars() {
        if character.is_whitespace() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if scalar_count == MAX_NORMALIZED_TEXT_SCALARS {
            break;
        }
        if pending_space {
            normalized.push(' ');
            pending_space = false;
        }
        normalized.push(character);
        scalar_count += 1;
    }
    normalized
}

fn structure_match(old: &SemanticAnalysis, new: &SemanticAnalysis) -> ProtectedStructureMatch {
    match aeria_se::compare_analyses(old, new).compatibility {
        StructureCompatibility::Compatible => ProtectedStructureMatch::Compatible,
        StructureCompatibility::Incompatible => ProtectedStructureMatch::Incompatible,
        StructureCompatibility::CannotSafelyCompare => ProtectedStructureMatch::NotComparable,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditSimilarityPath {
    FullMatrix,
    BoundedFallback,
}

fn normalized_edit_similarity(old: &[char], new: &[char]) -> u16 {
    edit_similarity(old, new).0
}

fn edit_similarity(old: &[char], new: &[char]) -> (u16, EditSimilarityPath) {
    if old == new {
        return (1000, EditSimilarityPath::FullMatrix);
    }
    if old.is_empty() || new.is_empty() {
        return (0, EditSimilarityPath::FullMatrix);
    }

    let matrix_cells = old.len().saturating_mul(new.len());
    if matrix_cells > MAX_EDIT_MATRIX_CELLS {
        return (
            bounded_overlap_similarity(old, new),
            EditSimilarityPath::BoundedFallback,
        );
    }

    let (short, long) = if old.len() <= new.len() {
        (old, new)
    } else {
        (new, old)
    };
    let mut previous: Vec<usize> = (0..=short.len()).collect();
    let mut current = vec![0; short.len() + 1];
    for (long_index, long_character) in long.iter().enumerate() {
        current[0] = long_index + 1;
        for (short_index, short_character) in short.iter().enumerate() {
            let substitution =
                previous[short_index] + usize::from(long_character != short_character);
            current[short_index + 1] = (current[short_index] + 1)
                .min(previous[short_index + 1] + 1)
                .min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[short.len()];
    let longest = old.len().max(new.len());
    (
        u16::try_from(((longest - distance) * 1000) / longest).unwrap_or(0),
        EditSimilarityPath::FullMatrix,
    )
}

fn bounded_overlap_similarity(old: &[char], new: &[char]) -> u16 {
    let matching_scalars = old
        .iter()
        .zip(new)
        .filter(|(old_character, new_character)| old_character == new_character)
        .count();
    let longest = old.len().max(new.len());
    u16::try_from((matching_scalars * 1000) / longest).unwrap_or(0)
}

fn coordinate_distance(old: &SourceBinding, new: &SourceBinding) -> u64 {
    if old.sheet_name() != new.sheet_name() {
        return u64::MAX;
    }
    u64::from(old.row_id().abs_diff(new.row_id())) * 1_000_000
        + u64::from(old.subrow_id().abs_diff(new.subrow_id())) * 1_000
        + u64::from(old.column_index().abs_diff(new.column_index()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_preserves_unicode_and_punctuation_but_folds_whitespace() {
        assert_eq!(normalize_visible_text("  Привет,\tмир!  "), "Привет, мир!");
        assert_ne!(
            normalize_visible_text("Attack"),
            normalize_visible_text("attack")
        );
        assert_ne!(
            normalize_visible_text("Yes"),
            normalize_visible_text("Yes!")
        );
    }

    #[test]
    fn edit_similarity_is_deterministic_and_bounded() {
        let score = |old: &str, new: &str| {
            let old: Vec<_> = old.chars().collect();
            let new: Vec<_> = new.chars().collect();
            normalized_edit_similarity(&old, &new)
        };
        assert_eq!(score("same", "same"), 1000);
        assert!(score("same", "sane") > 700);
        assert!(score("same", "different") < 500);
        assert_eq!(score("", "value"), 0);
    }

    #[test]
    fn long_edit_similarity_uses_the_bounded_fallback() {
        let old = vec!['a'; 4096];
        let new = vec!['b'; 4096];
        let first = edit_similarity(&old, &new);
        let second = edit_similarity(&old, &new);
        assert_eq!(first.1, EditSimilarityPath::BoundedFallback);
        assert_eq!(first, second);
        assert_eq!(first.0, 0);
    }

    #[test]
    fn opaque_valid_macros_have_visible_text_and_are_not_rejected() {
        let prepared = PreparedSource::new("before<UnknownFuture(1)>after");
        let visible: String = prepared.visible_scalars.iter().collect();
        assert_eq!(visible, "beforeafter");
        assert!(!matches!(
            prepared.analysis.validation().status(),
            aeria_se::SemanticValidity::InvalidUnsafe
        ));
    }

    #[test]
    fn score_ties_are_total_and_coordinate_distance_prefers_nearby() {
        let near = CandidateScore {
            exactness: CandidateExactness::None,
            protected_structure: ProtectedStructureMatch::Compatible,
            visible_text_similarity: 900,
            same_sheet: true,
            same_column: true,
            coordinate_distance: 1,
        };
        let far = CandidateScore {
            coordinate_distance: 2,
            ..near
        };
        assert!(near > far);
        assert_eq!(near.cmp(&near), Ordering::Equal);
    }

    #[test]
    fn pool_generation_is_capped_and_canonical() {
        let fingerprint = SourceFingerprint::new(
            Sha256Hash::from_bytes([1; 32]),
            None,
            Sha256Hash::from_bytes([2; 32]),
        );
        let occurrences = (0..(MAX_GENERATED_CANDIDATES + 25))
            .map(|row_id| IndexedOccurrence {
                binding: SourceBinding::new(
                    "Sheet",
                    u32::try_from(row_id).expect("synthetic row fits HXS coordinate"),
                    0,
                    0,
                ),
                fingerprint,
            })
            .collect();
        let mut index = CandidateIndex {
            occurrences,
            ..CandidateIndex::default()
        };
        for (index_number, occurrence) in index.occurrences.iter().enumerate() {
            index
                .by_binding
                .insert(occurrence.binding.clone(), index_number);
            index
                .by_macro_text
                .entry(fingerprint.macro_text_hash())
                .or_default()
                .push(index_number);
            index
                .by_sheet_column
                .entry(("Sheet".to_owned(), 0))
                .or_default()
                .push(index_number);
        }
        let pool = index.generate_pool(&SourceBinding::new("Missing", 0, 0, 0), &fingerprint);
        assert_eq!(pool.len(), MAX_GENERATED_CANDIDATES);
        assert_eq!(pool, (0..MAX_GENERATED_CANDIDATES).collect::<Vec<_>>());
    }

    #[test]
    fn candidate_score_is_not_confidence() {
        let docs = CandidateScore {
            exactness: CandidateExactness::MacroAndRaw,
            protected_structure: ProtectedStructureMatch::Compatible,
            visible_text_similarity: 1000,
            same_sheet: true,
            same_column: true,
            coordinate_distance: 0,
        };
        assert_eq!(docs.visible_text_similarity, 1000);
    }
}

#[cfg(test)]
#[path = "candidate_evaluation.rs"]
mod candidate_evaluation;
