# Rebase safety contract

Status: **required before any rebase apply or workspace mutation**.

`aeria-rebase` produces a pure diagnostic plan. A managed String cell is one
translation unit. The planner reads verified old and new HXS snapshots and
never mutates the workspace, source files, or persisted translation text.

## Authority boundary

The previous `SourceBinding` is the authoritative continuity key when that
binding exists in the new verified snapshot. This rule is intentionally
independent of candidate uniqueness and complete-fingerprint equality:

```text
old binding exists in new snapshot
    -> same binding is authoritative
old binding is missing
    -> no automatic cross-binding identity
```

For a surviving binding, the planner compares the String-cell content facts:

- equal `macroTextHash` and `rawValueHash` produce `Unchanged`;
- either content fact changing produces `SourceChanged`;
- a changed `rowTechnicalHash` is reported separately as
  `SourceContextStatus::Changed` and does not change identity.

For a missing binding, the outcome is `Ambiguous`. Candidate diagnostics may
show exact or partial matches, but they never establish a proposed binding.
Candidate discovery is restricted to missing-binding units. Surviving units do
not receive broad candidate diagnostics, even if the same source content is
duplicated elsewhere.

The plan never recomputes `TranslationUnitId`. `automatic_evidence` is
`AutomaticEvidence::SameBinding` only for surviving bindings. Proposed binding
and fingerprint are populated for both `Unchanged` and `SourceChanged`; they
are absent for `Ambiguous`.

## HXS v1 hash contract

The planner consumes verified HXS v1 data and does not redefine the persisted
workspace format. The persisted `SourceFingerprint` remains the unchanged
three-field tuple:

```text
(macroTextHash, rawValueHash, rowTechnicalHash)
```

The planner uses explicit concepts for the two comparisons:

| Concept | Facts | Use |
| --- | --- | --- |
| `SourceContent` | `macroTextHash`, optional `rawValueHash` | Classifies a surviving binding as `Unchanged` or `SourceChanged`. |
| `SourceContextStatus` | equality of `rowTechnicalHash` | Reports technical context change without changing identity. |
| complete `SourceFingerprint` | all three persisted fields | Compatibility and candidate evidence; never a cross-binding identity key. |

Relevant HXS inputs are:

| HXS fact | Includes | Rebase consequence |
| --- | --- | --- |
| `string_cells.macro_hash` | Complete macro text, including structured macro spelling | Cell content comparison and candidate evidence. |
| `string_cells.raw_hash` | Presence and hash of raw source value | `Some`/`None` and value changes are `SourceChanged` at a surviving binding. |
| `rows.technical_hash` | Sheet name, row ID, subrow ID, and canonical non-String technical payload | Context-only status; coordinate-sensitive changes do not break surviving identity. |
| `rows.string_hash` | Row coordinates, String columns, macro hashes, and raw hashes | Verified aggregate; not a persisted identity key. |
| `rows.row_hash` | Row coordinates, technical hash, and String hash | Verified aggregate; not a persisted identity key. |
| sheet/schema/content/snapshot IDs | Canonical source and metadata identity | Snapshot validation; never a rebind decision. |

Because row technical hashes include coordinates, a row shift can change the
complete fingerprint even when the String cell's content is identical. That
must be reported as context change, not mistaken for loss of identity.

## Planner truth matrix

“Binding present” means the exact previous `SourceBinding` exists in the new
snapshot. Candidate evidence is shown only for a missing binding.

| Binding present | String content | Technical context | Candidate result | Planner result |
| --- | --- | --- | --- | --- |
| yes | same | same | none | `Unchanged`, same binding, `SameBinding` evidence, context unchanged |
| yes | same | changed | none | `Unchanged`, same binding, context changed |
| yes | changed | same | none | `SourceChanged`, same binding, context unchanged |
| yes | changed | changed | none | `SourceChanged`, same binding, context changed |
| no | same or changed | same or changed | zero, unique, or duplicate | `Ambiguous`, no proposed binding; candidates remain non-authoritative |

The number of equal complete fingerprints elsewhere is irrelevant to every
surviving binding. A duplicate elsewhere cannot turn a surviving binding into
`Ambiguous`.

## Required transition coverage

The deterministic planner and tests cover these transition classes:

### Snapshot and sheet transitions

- identical old/new snapshots;
- game-version changes with compatible source language and scope;
- source-language, scope, old-content, old-snapshot, malformed, and
  unverified-input failures;
- empty source corpora and empty managed workspaces;
- added, removed, renamed, and variant-changed sheets;
- unchanged and changed schemas, including String and technical column edits;
- all cross-sheet relationships remain non-authoritative.

### Binding and coordinate transitions

- row/subrow/column shifts, insertions, deletions, and global offsets;
- row and column movement together, block movement, permutations, swaps, and
  cycles;
- coordinate reuse where an unrelated value occupies the old coordinate;
- the same binding surviving while its content changes;
- duplicate values at the surviving binding and elsewhere;
- no binding-presence check is replaced by ordering or nearest-coordinate logic.

### Content and context transitions

- macro-only, raw-only, and macro-plus-raw changes;
- raw `Some -> None` and `None -> Some` changes;
- technical-only changes;
- a neighboring String cell changing without affecting another cell's unit;
- technical changes in the same row reported as context changes;
- empty, opaque, macro-bearing, and long valid macro text;
- duplicate macro, macro-plus-raw, and complete-fingerprint candidates.

### Candidate transitions

Only a missing old binding enters candidate discovery. Exact complete,
macro-plus-raw, macro-plus-row, and exact-macro evidence is bounded,
deterministic, and non-authoritative. Unique evidence is still a suggestion;
duplicates remain ambiguous. Candidate discovery is not run for surviving
bindings, which keeps ordinary rebase transitions proportional to the managed
units that actually lost their binding.

## Coordinate reuse and row shifts

For:

```text
old: A at X
new: A at Y, unrelated B at X
```

the old unit at X follows X if X survives, even when X now contains B. It is
`Unchanged` or `SourceChanged` according to B's content at X, with the
surviving binding X proposed. A unit whose old binding is absent at Y is
`Ambiguous`; A at Y may be a candidate but is never automatically proposed.

For a row shift:

```text
old: Alpha@100, Beta@101, Gamma@102
new: Alpha@101, Beta@102, Gamma@103
```

old Alpha@100 is missing and ambiguous. Old Beta@101 and Gamma@102 retain
their bindings and are `SourceChanged` because the surviving cells contain the
shifted neighbor's content. Neither is rebound to its descendant row. The
same rule handles insertions, deletions, coordinate reuse, duplicate values,
and swaps.

## Model-based safety proof

The integration suite runs a deterministic bounded reference model in CI; it
is not ignored. The model enumerates 73 injective placements of three logical
old occurrences into four slots, all eight source-mutation masks, and both
inserted/non-inserted corpus states: `73 * 8 * 2 = 1,168` verified HXS
transitions.

For every generated state, the oracle checks:

1. any authoritative proposed binding equals the previous binding;
2. every surviving binding is never ambiguous and is `Unchanged` or
   `SourceChanged` according to macro/raw content only;
3. every missing binding is `Ambiguous` with no proposed binding or
   fingerprint;
4. content classification ignores row technical context;
5. output is deterministic for equivalent physical insertion order; and
6. the `TranslationUnitId` remains stable.

The model never passes logical-origin labels to production. Its exhaustive
state count, automatic continuity count, source-changed count, unresolved
count, and wrong-mapping count are printed by the test for auditability.

## Future apply semantics

Apply is intentionally not implemented by this planner. Once the safety
contract is merged and CI is green, a future apply operation may:

- carry `Unchanged` forward at the same binding;
- carry `SourceChanged` forward at the same binding, replace the current
  persisted source facts, and mark the unit for review;
- reject `Ambiguous` until a human explicitly reconciles a candidate; and
- preserve the existing `TranslationUnitId` in every case.

Apply must update workspace source facts and review state atomically. It must
not silently discard translated text or infer cross-binding identity from a
unique candidate.

Fuzzy matching, ranking, and structural shift detection are out of scope for
identity authority. They may provide review UI later, but cannot create an
automatic source mapping.
