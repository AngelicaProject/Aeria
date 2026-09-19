# Rebase candidate suggestions

Candidate suggestions are bounded review assistance for an `Ambiguous`
translation unit whose previous source binding is missing. They are separate
from the authoritative transition facts produced by
[`RebasePlanner`](./rebase.md):

```text
RebasePlanner       authoritative same-binding transition facts
CandidateSuggester  bounded review suggestions for missing bindings
FuzzyRanker         deterministic ordering only
Human reconciliation future explicit cross-binding decision
```

The public `aeria-rebase::candidates` interface returns owned
`SourceCandidate` values for one `CandidateQuery`. A candidate contains a
source binding, verified source fingerprint, evidence, and a
`CandidateScore`. Results never mutate a `TranslationUnit`, change its ID,
update workspace state, or produce a `RebaseOutcome`. There is no automatic
acceptance, rebinding, apply operation, or proposed authoritative binding.

## Candidate generation

`CandidateSuggester::from_snapshot` enumerates the verified new HXS once using
the bounded, canonical String-occurrence pages. It retains lightweight
coordinates and hashes in an owned index. A query first blocks on exact
complete fingerprint, macro-plus-raw hash, and macro-text hash groups. It then
adds a bounded same-sheet/same-column coordinate neighborhood. Duplicate
indexes are merged in canonical source order and capped at
`MAX_GENERATED_CANDIDATES` (currently 128).

The suggester retains the verified `SnapshotMetadata` from construction and
rejects any different snapshot passed for payload reads. Before reading the
old macro payload, it reuses the planner's old-baseline verification, including
the persisted macro/raw/row-technical fingerprint checks. Ranking therefore
cannot combine an index, new payload, or old payload from an unverified source
state.

Full macro payloads are read only for that bounded pool. A unit whose binding
survives in the prepared new snapshot receives no suggestions and does not
compete with other coordinates. The query result is capped by
`MAX_RESULT_LIMIT` (currently 50), with `DEFAULT_RESULT_LIMIT` set to 10.
These are conservative implementation defaults and are intentionally easy to
change after measurement.

Blocking and ranking are separate concerns. If a correct candidate is absent
from the bounded pool, ranking cannot recover it; evaluation reports that as a
blocking miss rather than a ranking miss.

## Ranking baseline

The baseline uses `aeria-se` to derive two source projections:

- the raw macro representation remains available for exact hash evidence;
- visible/translatable text ranges are joined and normalized by preserving
  Unicode, collapsing deterministic Unicode whitespace runs to one ASCII
  space, trimming leading/trailing whitespace, and making no locale-dependent
  case or punctuation rewrite. The projection is bounded at 4096 Unicode
  scalar values.

Ranking is lexicographic and deterministic: exact macro-plus-raw evidence,
exact macro text, protected-structure compatibility, normalized edit
similarity, same-sheet, same-column, and coordinate proximity. Protected
structure and coordinate facts are ranking evidence only. Opaque but valid
macros remain supported by the `aeria-se` projection; malformed input is not
silently treated as safe structure.

The expensive edit scorer has an explicit per-candidate budget of at most
65,536 edit-matrix cells. It reuses two dynamic-programming buffers for work
within that budget. Larger normalized inputs use a deterministic positional
overlap fallback, so the 128-candidate pool cannot create an unbounded
quadratic scoring cost. This is a computational bound, not a confidence
threshold.

Every equal score uses canonical source order as the final tie-break:
`sheet_name`, `row_id`, `subrow_id`, then `column_index`. The score is a
ranking value, not a probability or calibrated confidence, and the top result
is never automatic identity.

## Evaluation and persistence

The offline evaluation harness keeps logical-origin labels in the evaluation
oracle only. Production blocking and ranking receive source facts, not those
labels. It reports candidate-pool recall separately from final recall@1,
recall@3, recall@5, recall@10, and MRR; cases with no valid candidate are
reported separately. The current 28-case corpus freezes integer regression
baselines of 27 valid cases, 1 no-valid case, pool hits 26, recall@1 hits 25,
and recall@3/5/10 hits 26. These are deterministic regression gates, not
confidence claims or representative production accuracy for historical FFXIV
updates. Its source-pair abstraction can later be backed by real historical
HXS pairs without changing the production interface.

Suggestion lists, scores, normalized text, pool contents, and ranker versions
are disposable snapshot-dependent state. Workspace Format v1 is unchanged;
none of those values are persisted. UI, AI ranking, reconciliation, and
apply remain out of scope.
