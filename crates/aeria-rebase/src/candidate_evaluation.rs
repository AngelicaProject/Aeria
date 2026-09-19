//! Test-only deterministic evaluation corpus for candidate blocking/ranking.
//!
//! This file is included only by the candidate module's unit tests. The
//! logical-origin labels stay in this oracle and are never passed to pool
//! generation or ranking. The `EvaluationPair` shape is intentionally source
//! oriented so a later adapter can feed real old/new HXS snapshots and a
//! ground-truth reconciliation set.

use aeria_core::{Sha256Hash, SourceBinding, SourceFingerprint};
use sha2::{Digest, Sha256};

use super::*;

struct EvaluationPair {
    name: &'static str,
    old_binding: SourceBinding,
    old_fingerprint: SourceFingerprint,
    old_macro_text: &'static str,
    candidates: Vec<LabeledCandidate>,
    valid_origin: Option<&'static str>,
}

struct LabeledCandidate {
    logical_origin: &'static str,
    occurrence: IndexedOccurrence,
    macro_text: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Metrics {
    valid_cases: usize,
    no_valid_candidate_cases: usize,
    pool_hits: usize,
    recall_at_1: usize,
    recall_at_3: usize,
    recall_at_5: usize,
    recall_at_10: usize,
    reciprocal_rank_sum: f64,
}

impl Metrics {
    fn report(self) -> String {
        let denominator = as_f64(self.valid_cases);
        let ratio = |value: usize| {
            if self.valid_cases == 0 {
                0.0
            } else {
                as_f64(value) / denominator
            }
        };
        format!(
            "valid={} no_valid={} pool_recall={:.3} recall@1={:.3} recall@3={:.3} recall@5={:.3} recall@10={:.3} mrr={:.3}",
            self.valid_cases,
            self.no_valid_candidate_cases,
            ratio(self.pool_hits),
            ratio(self.recall_at_1),
            ratio(self.recall_at_3),
            ratio(self.recall_at_5),
            ratio(self.recall_at_10),
            if self.valid_cases == 0 {
                0.0
            } else {
                self.reciprocal_rank_sum / denominator
            }
        )
    }
}

#[test]
fn synthetic_corpus_reports_blocking_and_ranking_metrics_deterministically() {
    let corpus = synthetic_corpus();
    assert_eq!(corpus.len(), 28);
    let first = evaluate(&corpus);
    let second = evaluate(&corpus);
    assert_eq!(first, second);
    println!("candidate evaluation: {}", first.report());
    assert_eq!(first.no_valid_candidate_cases, 1);
    assert!(first.pool_hits <= first.valid_cases);
    assert!(first.recall_at_1 <= first.recall_at_3);
    assert!(first.recall_at_3 <= first.recall_at_5);
    assert!(first.recall_at_5 <= first.recall_at_10);
}

fn evaluate(corpus: &[EvaluationPair]) -> Metrics {
    let mut metrics = Metrics::default();
    for case in corpus {
        assert!(!case.name.is_empty());
        let old_fingerprint = case.old_fingerprint;
        let occurrences: Vec<_> = case
            .candidates
            .iter()
            .map(|candidate| candidate.occurrence.clone())
            .collect();
        let mut index = CandidateIndex {
            occurrences,
            ..CandidateIndex::default()
        };
        index.rebuild_for_test();
        let pool = index.generate_pool(&case.old_binding, &old_fingerprint);
        let Some(valid_origin) = case.valid_origin else {
            metrics.no_valid_candidate_cases += 1;
            continue;
        };
        metrics.valid_cases += 1;
        let pool_contains_truth = pool
            .iter()
            .any(|&index| case.candidates[index].logical_origin == valid_origin);
        metrics.pool_hits += usize::from(pool_contains_truth);

        let old_source = PreparedSource::new(case.old_macro_text);
        let prepared = pool
            .iter()
            .map(|&index| {
                let candidate = &case.candidates[index];
                (
                    candidate.occurrence.clone(),
                    PreparedSource::new(candidate.macro_text),
                )
            })
            .collect();
        let ranked = rank_candidates(&case.old_binding, &old_fingerprint, &old_source, prepared);
        let Some(rank) = ranked.iter().position(|candidate| {
            case.candidates.iter().any(|labeled| {
                labeled.logical_origin == valid_origin
                    && labeled.occurrence.binding == candidate.binding
            })
        }) else {
            continue;
        };
        metrics.reciprocal_rank_sum += 1.0 / as_f64(rank + 1);
        metrics.recall_at_1 += usize::from(rank < 1);
        metrics.recall_at_3 += usize::from(rank < 3);
        metrics.recall_at_5 += usize::from(rank < 5);
        metrics.recall_at_10 += usize::from(rank < 10);
    }
    metrics
}

impl CandidateIndex {
    fn rebuild_for_test(&mut self) {
        for (index, occurrence) in self.occurrences.iter().enumerate() {
            self.by_binding.insert(occurrence.binding.clone(), index);
            self.by_fingerprint
                .entry(occurrence.fingerprint)
                .or_default()
                .push(index);
            if let Some(raw) = occurrence.fingerprint.raw_value_hash() {
                self.by_macro_and_raw
                    .entry((occurrence.fingerprint.macro_text_hash(), raw))
                    .or_default()
                    .push(index);
            }
            self.by_macro_text
                .entry(occurrence.fingerprint.macro_text_hash())
                .or_default()
                .push(index);
            self.by_sheet_column
                .entry((
                    occurrence.binding.sheet_name().to_owned(),
                    occurrence.binding.column_index(),
                ))
                .or_default()
                .push(index);
        }
    }
}

#[allow(clippy::too_many_lines)]
fn synthetic_corpus() -> Vec<EvaluationPair> {
    let mut cases = vec![
        pair(
            "exact relocation",
            "Attack",
            candidate(1, "Attack", "truth"),
            Some("truth"),
        ),
        pair(
            "tiny text edit",
            "Attack",
            candidate(1, "Attakc", "truth"),
            Some("truth"),
        ),
        pair(
            "large text edit",
            "The quick brown fox jumps",
            candidate(1, "A distant moon rises", "truth"),
            Some("truth"),
        ),
        pair(
            "prefix insertion",
            "Attack",
            candidate(1, "Heavy Attack", "truth"),
            Some("truth"),
        ),
        pair(
            "suffix insertion",
            "Attack",
            candidate(1, "Attack now", "truth"),
            Some("truth"),
        ),
        pair(
            "middle insertion",
            "A B",
            candidate(1, "A careful B", "truth"),
            Some("truth"),
        ),
        pair(
            "deletion",
            "Attack now",
            candidate(1, "Attack", "truth"),
            Some("truth"),
        ),
        pair(
            "number changes",
            "Potion 10",
            candidate(1, "Potion 11", "truth"),
            Some("truth"),
        ),
        pair(
            "punctuation changes",
            "Wait!",
            candidate(1, "Wait?", "truth"),
            Some("truth"),
        ),
        pair(
            "whitespace changes",
            "One two",
            candidate(1, "One   two", "truth"),
            Some("truth"),
        ),
        pair(
            "macro-preserving visible-text edit",
            "<Bold>Attack",
            candidate(1, "<Bold>Attacks", "truth"),
            Some("truth"),
        ),
        pair(
            "macro structure change",
            "<Bold>Attack",
            candidate(1, "<Italic>Attack", "truth"),
            Some("truth"),
        ),
        pair(
            "opaque valid macros",
            "before<UnknownFuture(1)>after",
            candidate(1, "before<UnknownFuture(1)>after now", "truth"),
            Some("truth"),
        ),
        pair(
            "empty strings",
            "",
            candidate(1, "", "truth"),
            Some("truth"),
        ),
        pair(
            "very short strings",
            "Yes",
            candidate(1, "No", "truth"),
            Some("truth"),
        ),
        pair(
            "duplicate strings",
            "Cancel",
            candidate(1, "Cancel", "truth"),
            Some("truth"),
        ),
        pair(
            "one old to many identical new",
            "OK",
            labeled_candidates(&[
                (1, "OK", "truth"),
                (2, "OK", "other"),
                (3, "OK", "other"),
                (4, "OK", "other"),
            ]),
            Some("truth"),
        ),
        pair(
            "many old to one similar new",
            "Alpha",
            candidates("truth", &[(1, "Alpha plus"), (2, "Beta"), (3, "Gamma")]),
            Some("truth"),
        ),
        pair(
            "row movement",
            "Row text",
            candidate(1, "Row text", "truth"),
            Some("truth"),
        ),
        pair(
            "subrow movement",
            "Subrow text",
            candidate_subrow(1, 1, "Subrow text", "truth"),
            Some("truth"),
        ),
        pair(
            "column movement",
            "Column text",
            candidate_column(1, 1, "Column text", "truth"),
            Some("truth"),
        ),
        pair_sheet(
            "cross-sheet movement",
            "Sheet text",
            candidate_sheet("Other", 1, "Sheet text", "truth"),
            Some("truth"),
        ),
        pair(
            "moved plus edited",
            "Move me",
            candidate(1, "Moved me", "truth"),
            Some("truth"),
        ),
        pair(
            "unrelated identical text",
            "Yes",
            labeled_candidates(&[(1, "Yes", "other"), (2, "Yes", "truth")]),
            Some("truth"),
        ),
        pair(
            "coordinate-near unrelated text",
            "Attack",
            labeled_candidates(&[(1, "Unrelated", "other"), (100, "Attack harder", "truth")]),
            Some("truth"),
        ),
        pair(
            "coordinate-far correct text",
            "Far text",
            far_candidates(),
            Some("truth"),
        ),
        pair(
            "no valid candidate",
            "Missing",
            candidate(1, "Unrelated", "other"),
            None,
        ),
    ];
    cases.push(pair(
        "coordinate-near duplicate short string",
        "No",
        labeled_candidates(&[(1, "No", "truth"), (2, "No", "other"), (3, "No", "other")]),
        Some("truth"),
    ));
    cases
}

fn as_f64(value: usize) -> f64 {
    f64::from(u32::try_from(value).expect("evaluation corpus values fit u32"))
}

fn pair(
    name: &'static str,
    old_macro_text: &'static str,
    candidates: Vec<LabeledCandidate>,
    valid_origin: Option<&'static str>,
) -> EvaluationPair {
    let old_binding = SourceBinding::new("Sheet", 10, 0, 0);
    EvaluationPair {
        name,
        old_fingerprint: fingerprint(&old_binding, old_macro_text, None),
        old_binding,
        old_macro_text,
        candidates,
        valid_origin,
    }
}

fn pair_sheet(
    name: &'static str,
    old_macro_text: &'static str,
    candidate: LabeledCandidate,
    valid_origin: Option<&'static str>,
) -> EvaluationPair {
    let old_binding = SourceBinding::new("Sheet", 10, 0, 0);
    EvaluationPair {
        name,
        old_fingerprint: fingerprint(&old_binding, old_macro_text, None),
        old_binding,
        old_macro_text,
        candidates: vec![candidate],
        valid_origin,
    }
}

fn candidates(truth: &'static str, values: &[(u32, &'static str)]) -> Vec<LabeledCandidate> {
    values
        .iter()
        .map(|&(row, text)| candidate_with_origin(row, 0, 0, "Sheet", text, truth))
        .collect()
}

fn labeled_candidates(values: &[(u32, &'static str, &'static str)]) -> Vec<LabeledCandidate> {
    values
        .iter()
        .map(|&(row, text, origin)| candidate_with_origin(row, 0, 0, "Sheet", text, origin))
        .collect()
}

fn far_candidates() -> Vec<LabeledCandidate> {
    let mut candidates: Vec<_> = (1..=40)
        .map(|row| candidate_with_origin(row, 0, 0, "Sheet", "Unrelated", "other"))
        .collect();
    candidates.push(candidate_with_origin(
        1000,
        0,
        0,
        "Sheet",
        "Far text moved",
        "truth",
    ));
    candidates
}

fn candidate(row: u32, macro_text: &'static str, origin: &'static str) -> Vec<LabeledCandidate> {
    vec![candidate_with_origin(
        row, 0, 0, "Sheet", macro_text, origin,
    )]
}

fn candidate_subrow(
    row: u32,
    subrow: u16,
    macro_text: &'static str,
    origin: &'static str,
) -> Vec<LabeledCandidate> {
    vec![candidate_with_origin(
        row, subrow, 0, "Sheet", macro_text, origin,
    )]
}

fn candidate_column(
    row: u32,
    column: u32,
    macro_text: &'static str,
    origin: &'static str,
) -> Vec<LabeledCandidate> {
    vec![candidate_with_origin(
        row, 0, column, "Sheet", macro_text, origin,
    )]
}

fn candidate_sheet(
    sheet: &'static str,
    row: u32,
    macro_text: &'static str,
    origin: &'static str,
) -> LabeledCandidate {
    candidate_with_origin(row, 0, 0, sheet, macro_text, origin)
}

fn candidate_with_origin(
    row: u32,
    subrow: u16,
    column: u32,
    sheet: &'static str,
    macro_text: &'static str,
    logical_origin: &'static str,
) -> LabeledCandidate {
    let binding = SourceBinding::new(sheet, row + 10, subrow, column);
    LabeledCandidate {
        logical_origin,
        occurrence: IndexedOccurrence {
            binding: binding.clone(),
            fingerprint: fingerprint(&binding, macro_text, None),
        },
        macro_text,
    }
}

fn fingerprint(
    binding: &SourceBinding,
    macro_text: &str,
    raw_value: Option<&[u8]>,
) -> SourceFingerprint {
    let technical = format!(
        "{}:{}:{}:{}",
        binding.sheet_name(),
        binding.row_id(),
        binding.subrow_id(),
        binding.column_index()
    );
    SourceFingerprint::new(
        hash(macro_text.as_bytes()),
        raw_value.map(hash),
        hash(technical.as_bytes()),
    )
}

fn hash(value: &[u8]) -> Sha256Hash {
    Sha256Hash::from_bytes(Sha256::digest(value).into())
}
