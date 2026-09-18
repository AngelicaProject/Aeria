# Source update and rebase

Game updates are a primary product workflow. Rebase is deterministic,
conservative, auditable, and safe: preserving a translation is less important
than avoiding a false source identity.

## Current planner contract

The first production planner in `aeria-rebase` is pure. It borrows workspace
metadata and managed units plus verified old and new HXS snapshots, verifies
the old baseline, and returns an owned diagnostic plan without mutating the
workspace, HXS, or persistence. A changed game version is valid when the
source language and HXS scope remain compatible.

The planner treats each persisted String cell as one translation unit. It
establishes continuity only when the previous `SourceBinding` still exists in
the new verified snapshot. The binding is authoritative; it is not disqualified
because another occurrence has the same content or because the row's technical
context changed.

```text
same SourceBinding
```

The planner compares the String-cell content at that binding using
`macroTextHash` and `rawValueHash`, excluding `rowTechnicalHash`:

- unchanged content produces `Unchanged`;
- changed content produces `SourceChanged` while retaining the same binding;
- changed row technical context is reported separately as
  `SourceContextStatus::Changed` and does not change either outcome.

If the old binding is missing, the result is `Ambiguous`. Exact macro/raw
matches, exact macro matches, row evidence, coordinate observations, and
future fuzzy rankings are candidate diagnostics only. They never populate an
authoritative proposed binding and never claim a new occurrence. Candidate
discovery is performed only for missing bindings, so surviving bindings do not
pay the cost of broad candidate diagnostics.

The plan never recomputes `TranslationUnitId`. It currently does not apply a
plan or automatically classify terminal `new`, `removed`, or `relocated`
states; those require later explicit reconciliation. A future apply operation
may carry a `SourceChanged` unit forward at its surviving binding, update its
current source facts, and mark it for review without changing its durable ID.

The complete source-transition matrix, HXS v1 hash contract, adversarial cases,
model-based safety proof, and apply blocker live in
[`rebase-safety.md`](./rebase-safety.md).

## Product rules

- AI does not make identity or migration decisions.
- The same verified inputs and planner version produce the same logical plan.
- Existing translation text is never silently discarded.
- Ambiguity is visible work, not a fallback to ordering or similarity.
- RebasePlan apply is blocked until the safety contract is merged and the
  exhaustive model-based tests are green.

## Historical source versions

A workspace is bound to one current source snapshot. Historical game versions
are represented by Git history, branches, or tags rather than by multiple
simultaneous active source versions inside one workspace.
