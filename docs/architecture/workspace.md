# Translation workspace

A translation repository is a sparse overlay over immutable HXS source data.

## Sparse overlay

The repository stores entries only for translated or otherwise explicitly project-managed units. It does not contain one record for every source string.

A translation entry contains enough information for stable identity, current binding verification, target content, review state, notes, and safe migration without duplicating the complete source corpus. The first in-memory workspace slice stores the source binding and verified macro/raw/row-technical hashes, but never the full source macro text.

The workspace is sparse in memory as well as on disk: creating a workspace from a verified HXS snapshot does not enumerate source cells. A unit is created only when a caller requests one verified String cell, and source data is read on demand through `aeria-hxs`.

The in-memory workspace keeps a deterministic ordered index by `TranslationUnitId` and a secondary deterministic ordered index from `SourceBinding` to `TranslationUnitId`. Both indexes are updated together when a unit is inserted; current workspace mutations do not change source bindings.

## Project/source binding

Workspace metadata records one source language, one target language, the current verified HXS `contentId`, and the current verified HXS `snapshotId`. Exactly one target language is canonical per workspace; target-language selection is not repeated on individual units.

The workspace source adapter accepts an already verified `HxsSnapshot`. It checks that the requested sheet, row, subrow, and String column exist, copies only the verified cell and row hashes into a `SourceFingerprint`, and derives a new v1 `TranslationUnitId`. It does not reopen or independently verify SQLite internals.

The current workspace interface has no per-unit source transition. A future atomic rebase operation must update workspace source metadata and unit bindings together; the core `TranslationUnit` primitive retains the stable-ID source-change transition for that operation.

## Target validity

Manual target edits use `aeria-se` intrinsic validation. Understood syntax and opaque-but-preservable syntax are valid targets; malformed or unsafe syntax is rejected. Manual translation does not require source and target protected structures to match. Strict structure compatibility remains a safety mechanism for future assisted or AI translation.

## Project scope

Exactly one target language is canonical per project.

Project-shared data may include:

- workspace manifest and source binding
- translated units
- glossary and translation guidance
- shared QA/configuration rules
- collaboration policy
- optionally shared saved collections/queries

Machine/user-local data includes:

- secondary source-language preferences
- source-package paths and disposable HXS cache
- AI credentials/model preferences
- local UI layout
- local search/index databases
- AI job state

## Review states

Initial core states:

- `draft`
- `reviewed`
- `needs-review`

`reviewed` means another human has reviewed the translation. AI output is `draft`. A source change invalidates prior review and moves the translation to `needs-review` unless a deterministic rule proves that the source content did not change.

Changing a target always resets its unit to `draft`. Marking a unit `reviewed` is an explicit workspace operation. The core `TranslationUnit` primitive can record a known meaningful source update as `needs-review` for a future atomic rebase; the current workspace interface does not accept arbitrary source binding or fingerprint replacements. Review is never inferred from Git commits, approvals, or other repository state.

## Production persistence

`aeria-workspace` now provides the production Workspace Format v1 persistence
adapter. `WorkspaceStore` binds to a repository root, loads the validated
state from `.aeria/manifest.json` and `.aeria/units/*.jsonl`, initializes a
new `.aeria/` directory from an in-memory workspace, and rewrites only the
shard selected by an affected `TranslationUnitId`. `persist_unit()` requires
that unit to be present in memory, fully validates the currently persisted
selected shard, replaces only that typed unit, and preserves every other
persisted unit in the shard. For an existing ID, its persisted
`SourceBinding` and `SourceFingerprint` are immutable on this ordinary
target/note/review path. A new ID must use a `SourceBinding` not owned anywhere
else in the persisted workspace; that insertion-only check may scan all
shards. Source transitions remain the responsibility of a future atomic rebase
operation.

Canonical shard replacements are written to a temporary file outside the
managed `.aeria/` namespace and published with a cross-platform atomic file
replacement. Initialization stages the complete directory under the
repository root and publishes it only after serialization succeeds. The
atomicity guarantee is per canonical file; multi-shard bulk operations do not
yet have one filesystem transaction or recovery protocol.

## Compatibility

The workspace has an explicit `formatVersion` from the first public version. New Aeria versions must either open an older public workspace directly or migrate it without data loss.

The concrete v1 layout and serialization contract are frozen in
[`../formats/workspace-v1.md`](../formats/workspace-v1.md). The persistence
adapter accepts only v1 repositories and reports unsupported versions rather
than applying an implicit migration.
