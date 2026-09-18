# Rebase safety contract

Status: **required before any rebase apply or workspace mutation**.

`aeria-rebase` produces a pure diagnostic plan. The planner may establish
automatic continuity only when both facts are true:

1. the previous `SourceBinding` exists in the new verified HXS snapshot; and
2. the complete persisted `SourceFingerprint` is byte-for-byte identical.

That one case is `Unchanged`. Every other managed unit is `Ambiguous`, even
when candidate evidence is unique, exact, or strongly suggestive. Candidate
evidence is review assistance, not an identity decision.

False continuity is more dangerous than additional review. In particular,
the planner never rebinds a `TranslationUnitId` because of similarity,
partial hashes, coordinate movement, ordering, or a remaining candidate.

## Authority boundary

The plan DTO makes the distinction explicit:

- `automatic_evidence` is present only for
  `SameBindingAndCompleteFingerprint`.
- `proposed_source_binding` and `proposed_source_fingerprint` are present
  only for that authoritative `Unchanged` result.
- `candidate_evidence` contains bounded diagnostic sets. It never claims a
  source occurrence, and several units may suggest the same binding.

The current planner emits only `Unchanged` and `Ambiguous`. Relocation,
source-change, new, and removed classifications remain reconciliation work.
`TranslationUnitId` is never recomputed.

## HXS v1 hash contract

The planner consumes verified HXS v1 data and does not redefine the persisted
workspace format. The relevant hash inputs are:

| HXS fact | Includes | Rebase consequence |
| --- | --- | --- |
| `string_cells.macro_hash` | The complete macro text, including its structured macro spelling | Exact macro evidence can suggest a candidate; it is not identity proof. |
| `string_cells.raw_hash` | Presence and hash of the raw source value | `Some` and `None` are different source facts. Exact macro-plus-raw evidence is still only a suggestion away from the old binding. |
| `rows.technical_hash` | The sheet name, `row_id`, `subrow_id`, and every non-String technical cell's canonical type/value payload | This hash is coordinate-sensitive. A row move or sheet move changes it even if the technical payload is otherwise logically unchanged. |
| `rows.string_hash` | The sheet name, `row_id`, `subrow_id`, String column indexes, macro hashes, and raw-hash presence/values | This is a row-content aggregate and is not persisted in `SourceFingerprint`. |
| `rows.row_hash` | The sheet name, `row_id`, `subrow_id`, technical hash, and String hash | It inherits coordinate sensitivity. |
| sheet schema hash | Sheet name, variant, column indexes, offsets, and column types | Schema changes are source transitions, never identity evidence. |
| content/snapshot IDs | Canonical sheet/schema/content and game/source metadata | They validate the snapshot relationship; they do not rebind units. |

Therefore the persisted `SourceFingerprint` tuple
`(macroTextHash, rawValueHash, rowTechnicalHash)` is suitable for detecting
whether the currently bound source facts are unchanged. It is not a
relocation-invariant identity fingerprint. In particular, `rowTechnicalHash`
must not be treated as binding-independent merely because a technical field's
value did not change.

## Planner truth matrix

“Old binding occupied” means that the previous binding exists in the new
snapshot. “Candidate” describes the diagnostic evidence available after the
authoritative check. A candidate is never a claim.

| Binding | Complete fingerprint | Candidate | Old binding occupied | Source value | Coordinate reused | Planner result |
| --- | --- | --- | --- | --- | --- | --- |
| same | same | unique | yes | same | no | `Unchanged`; authoritative proposed binding is the old binding |
| same | same | duplicate | yes | same | yes/unknown | `Ambiguous`; duplicate complete evidence is a conflict |
| same | different | none or any | yes | same or different | yes/no | `Ambiguous`; same-binding changed candidate may be reported |
| same | different | unique | no | same | no | `Ambiguous`; exact macro/raw/row candidates may be reported |
| different | same | unique | no | same | no | `Ambiguous`; complete-fingerprint candidate only |
| different | same | duplicate | no | same | yes/no | `Ambiguous`; all matching bindings remain suggestions |
| different | different | unique | no | same | yes/no | `Ambiguous`; weaker exact evidence remains non-authoritative |
| different | different | duplicate | no | same/different | yes/no | `Ambiguous`; no ordering or uniqueness heuristic resolves it |
| same/different | any | any | yes/no | different | yes/no | `Ambiguous`; source edits never become `SourceChanged` identity automatically |

The matrix is intentionally conservative. “Unique” does not mean “safe to
apply”; it only means the diagnostic set contains one item at that evidence
strength.

## Complete source-transition state space

The following cases define expected planner behavior. They are transition
classes, not a promise that the pure planner can identify the historical
logical origin in every case.

### Snapshot-level transitions

| Transition | Expected behavior |
| --- | --- |
| Identical old/new snapshot | Each managed unit at the same binding with the same complete fingerprint is `Unchanged`. |
| Game version changes only | Valid if source language, scope, content ID, and snapshot preconditions are compatible; matching still uses the safety rule. |
| Source language mismatch | Reject the plan as a source compatibility error. |
| Scope mismatch | Reject the plan; scope compatibility is not inferred. |
| Wrong old `contentId` | Reject before matching. |
| Wrong old `snapshotId` | Reject before matching. |
| Malformed or unverified old HXS | Reject at HXS verification/opening; the planner accepts verified snapshots only. |
| Malformed or unverified new HXS | Reject at HXS verification/opening; no partial plan is produced. |
| Empty source corpus | A valid new snapshot with no String cells supplies no automatic mappings; managed old units are unresolved. |
| Empty managed workspace | A valid compatible transition produces an empty plan and does not enumerate or create units. |
| Source with zero String cells | No source occurrence can be claimed; any managed unit whose old baseline is valid becomes unresolved when its binding disappears. |

### Sheet-level transitions

| Transition | Expected behavior |
| --- | --- |
| Sheet unchanged | Apply the binding/fingerprint rule per String occurrence. |
| Sheet added | New occurrences are not managed or claimed by this planner. |
| Sheet removed | Units formerly on it become unresolved; removal is not a terminal automatic classification. |
| Sheet renamed | Cross-sheet continuity is never automatic; exact candidates remain suggestions. |
| Sheet variant changed | Candidate diagnostics may exist, but schema/variant change never implies identity. |
| Effective language changed | Snapshot compatibility or review policy must handle it; the planner never infers continuity from language alone. |
| Schema unchanged | It is contextual validation, not a replacement for same-binding complete-fingerprint equality. |
| Schema changed | No identity follows. String-column additions/removals, technical-column additions/removals, column type changes, offset changes, and simultaneous schema changes all leave affected units unresolved unless their exact old binding and complete fingerprint remain valid. |

### Coordinate transitions

Independently or in combination, the following are always non-authoritative
when the binding differs:

| Change | Result |
| --- | --- |
| Sheet name | `Ambiguous`; cross-sheet exact evidence is a candidate. |
| `row_id` +1, -1, or +N | `Ambiguous`; row offset is not identity evidence. |
| `subrow_id` movement | `Ambiguous`; subrow continuity is not inferred. |
| `column_index` +1, -1, or +N | `Ambiguous`; column offset is not identity evidence. |
| Row + column movement | `Ambiguous`. |
| Sheet + row movement | `Ambiguous`. |
| Arbitrary permutation | `Ambiguous`; output cannot depend on permutation order. |
| Contiguous block movement | `Ambiguous`; a future batch proposal must be human approved. |
| Multiple moved blocks | `Ambiguous`; each proposed relationship remains review-only. |

The only automatic coordinate result is the unchanged old binding itself.

### Insertion and deletion transitions

One row inserted at the beginning, middle, or end; multiple row insertions;
one or multiple row deletions; alternating inserts/deletes; insertion followed
by a global offset; column insertion/deletion; and subrow insertion/deletion
are all handled as ordinary source transitions. A unit is `Unchanged` only if
its own binding and complete fingerprint are still exactly equal. All other
units are `Ambiguous`; no offset, nearest-coordinate, or block heuristic may
claim them.

### Source-value transitions

The planner treats each persisted fingerprint field independently when
forming candidate diagnostics, but never uses a changed value for automatic
continuity:

| Variation | Hash facts and result |
| --- | --- |
| All unchanged | At the same binding, all three persisted fields match and the result may be `Unchanged`. |
| Macro changed only | `macroTextHash` changes; the unit is unresolved. |
| Raw changed only | `rawValueHash` changes; `Some`/`None` transitions are changes. |
| Technical changed only | `rowTechnicalHash` changes. |
| Macro + raw, macro + technical, raw + technical, or all changed | The complete fingerprint changes; unresolved. |
| Raw `Some -> None`, `None -> Some`, or `None -> None` | Presence/value semantics remain exact; only an unchanged complete tuple can be authoritative. |
| Empty macro text | It is a normal exact macro value, not a wildcard. |
| Extremely long macro text | HXS verification/framing limits apply; valid text is handled as data, never as fuzzy identity. |
| Macro-bearing or opaque valid macro text | The complete macro hash is preserved as source data; macros do not become disposable similarity evidence. |

### Neighbor context

Because `rows.technical_hash` includes row coordinates and technical cells:

| Neighbor change | Expected hash effect |
| --- | --- |
| Our String unchanged; adjacent technical field changes | Our cell macro/raw hashes stay the same; our `rowTechnicalHash` changes, so the complete fingerprint changes and our unit is unresolved. |
| Our String unchanged; another String in the row changes | Our cell hashes stay the same; the row technical hash stays the same, but row/string aggregates may change. This does not create automatic continuity at a changed binding. |
| Several neighboring fields change | The row technical hash changes when technical payload changes; any complete fingerprint change is unresolved. |
| Our String changes; row technical data unchanged | Macro/raw facts change while `rowTechnicalHash` may stay equal; exact macro+row evidence is still only a candidate. |
| Our String moves while technical data is logically identical | The row technical hash changes because its coordinate is part of the hash; it is not relocation-invariant evidence. |

Never assume that `rowTechnicalHash` is independent of the binding.

### Coordinate reuse

For:

```text
old: A at X
new: A at Y, unrelated B at X
```

the planner never uses the fact that X is occupied to move A to B. If B has
a different source value, different macro/raw facts, or different technical
facts, the old unit is unresolved and may have candidate diagnostics for B or
for A at Y. If valid HXS facts are observationally identical at the same
binding, the frozen contract has no production-visible fact with which to
distinguish the logical labels A and B; the planner follows the exact
same-binding/complete-fingerprint rule rather than inventing an origin
identifier that HXS does not persist. Such observationally indistinguishable
facts are not a license to use similarity at other bindings.

The same rule covers Y present or absent, multiple reused coordinates, and
reused coordinates with duplicate macro or macro-plus-raw values. No
coordinate-presence check alone establishes identity.

### Swaps, cycles, and permutations

For `A <-> B`, `A -> B; B -> C; C -> A`, rotations of three or more
occurrences, reverse ordering, and whole-block permutations, every binding
change is unresolved. Iteration order, `SourceBinding` order,
`TranslationUnitId` order, and first-match behavior must not choose an origin.

### Duplicate-source transitions

Duplicate macro text, duplicate macro-plus-raw values, duplicate complete
fingerprints, duplicates in old only, new only, or both, adjacent/far-apart
duplicates, cross-sheet/cross-column duplicates, and identical empty strings
are all diagnostic situations. One-to-many, many-to-one, and many-to-many
candidate sets remain ambiguous. No tie is resolved by source binding order,
unit ID order, or first match. Multiple old units may carry the same suggested
new binding.

### Removal and new-occurrence transitions

Genuine removal, genuine addition, remove-plus-unrelated-add at the same
coordinate, identical-text additions elsewhere or at the old coordinate,
moves, moves plus edits, edits in place, and new text equal to old text never
become automatic `removed`, `new`, or cross-binding continuity decisions.
The current plan is intentionally limited to existing managed units and the
two outcomes above.

### Compound adversarial transitions

Row insertion plus technical change; insertion plus duplicate text; insertion
plus coordinate reuse; shift plus source edit; shift plus duplicate
candidates; schema change plus row movement; column insertion plus source
edit; swaps plus technical changes; remove plus add plus duplicate; several
unrelated transformations in one sheet; and simultaneous transformations
across sheets all preserve the same safety boundary. A compound transition
may add candidate diagnostics, but it cannot create an authoritative mapping
that was not already proven by the unchanged same-binding rule.

## Model-based safety proof

The integration suite contains a deterministic bounded reference model. The
model assigns logical-origin IDs to three old occurrences, applies every
injective placement into four coordinate slots (including removal), applies
all eight combinations of unchanged/edited source facts, and tests both an
inserted and non-inserted new corpus. It constructs verified synthetic HXS
snapshots; logical-origin IDs are never passed to production.

For every production `Unchanged` entry, the oracle checks that the
authoritative proposed binding contains the same logical origin. It also
checks that unresolved entries have no proposed binding or fingerprint. The
model is bounded and reproducible, not randomized. The exhaustive test is
run separately because it intentionally exercises 1,168 transition states.

## Future structural reconciliation

A later UX may detect a review proposal such as “15,000 occurrences appear to
shift by row +1.” The pattern can produce a human-reviewed batch proposal,
but the pattern itself does not establish identity. One explicit human
approval may establish a batch mapping in a future reconciliation format.

Fuzzy ranking, exact candidate suggestions, and structural observations all
belong below the safety boundary:

```text
deterministic planner   -> exact unchanged same-binding authority only
unresolved diagnostics  -> exact, structural, and fuzzy review suggestions
human reconciliation    -> explicit new identity decisions
```

## Apply blocker

**RebasePlan apply MUST NOT be implemented until this safety contract is
merged and the exhaustive tests are green.**

Any future apply operation must reject unresolved entries. It may apply only
authoritative unchanged mappings or explicit human reconciliation decisions,
and it must update workspace metadata and source facts atomically without
recomputing existing `TranslationUnitId` values.
