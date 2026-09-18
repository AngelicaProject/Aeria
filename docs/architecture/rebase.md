# Source update and rebase

Game updates are a primary product workflow. Rebase must be deterministic, conservative, auditable, and safe.

## Rules

- AI does not make identity or migration decisions.
- The same old snapshot, new snapshot, workspace, and rebase-engine version must produce the same plan.
- False automatic carry-over is considered worse than an ambiguous item requiring human attention.
- Existing translation text is never silently discarded.

## Core outcomes

A first stable vocabulary is:

- `unchanged`
- `relocated`
- `source-changed`
- `new`
- `removed`
- `ambiguous`

A source change results in `needs-review`; linguistic judgement about whether the existing target is still good belongs to humans or optional AI-review tooling, not the mechanical rebase engine.

## Rebase plan

Rebase first produces a deterministic plan/report. Applying it updates the project source binding and safe translation bindings transactionally. The report is local/exportable diagnostic data by default rather than required Git history.

The first production planner is implemented in `aeria-rebase` and is pure:
it borrows the workspace metadata and managed units plus verified old and new
HXS snapshots, returns an owned plan, and does not mutate the workspace, HXS,
or persistence. The old snapshot must match the workspace source language,
`contentId`, and `snapshotId`. The old and new snapshots must use the same
source language and HXS scope; a changed game version is valid.

The planner verifies every managed unit against the old snapshot before it
examines the new one. It enumerates verified sheet names in canonical order,
then walks each sheet with bounded keyset pages over
`(row_id, subrow_id, column_index)`. This preserves the global
`sheet name/rowId/subrowId/columnIndex` order without repeatedly sorting the
joined source corpus. It then applies these fixed mechanical stages:

1. the same current source binding;
2. an identical complete source fingerprint;
3. exact macro-text plus raw-value hash, when the old raw hash exists;
4. exact macro-text plus row-technical hash;
5. exact macro-text hash alone when the relationship is globally one-to-one.

Each stage resolves candidates as a batch. A new occurrence can be assigned to
at most one existing unit, and a candidate is automatically selected only
when one old unit and one currently unclaimed new occurrence remain under that
stage. Deterministic sorting never acts as identity evidence. Unresolved or
competing candidates remain `ambiguous`; they are not classified as terminal
`new` or `removed` outcomes.

An exact complete fingerprint proves that tracked source/context state did not
change. Partial exact-content evidence may establish a binding while still
producing `source-changed` when the complete fingerprint differs. A later
apply operation will move that unit to `needs-review`.

This first planner intentionally does not implement fuzzy matching, terminal
`new`/`removed` source-corpus classification, or plan application. Those are
subsequent milestones. The remaining conservative ambiguity is therefore an
expected result, not a failed rebase.

## Version history

A workspace is bound to one current source snapshot. Historical game versions are represented naturally by Git history/branches/tags rather than by multiple simultaneous active source versions inside one workspace.
