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

## Version history

A workspace is bound to one current source snapshot. Historical game versions are represented naturally by Git history/branches/tags rather than by multiple simultaneous active source versions inside one workspace.
