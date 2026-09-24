# Translation workspace

A translation repository is a sparse overlay over immutable HXS source data.

## Sparse overlay

The repository stores entries only for translated or otherwise explicitly
project-managed units. It does not contain one record for every source
string.

A translation entry contains enough information for stable identity, current
binding verification, target content, review state, notes, and safe source
updates without duplicating the complete source corpus. A unit stores its
source binding, the verified macro/raw/row-technical hashes, the source
layout (sheet schema hash and String column offset) of that binding, and the
row key of its row when the sheet is keyed, but never the full source macro
text.

The workspace is sparse in memory as well as on disk: creating a workspace
from a verified HXS snapshot does not enumerate source cells. A unit is
created only when a caller requests one verified String cell, and source data
is read on demand through `aeria-hxs`.

The in-memory workspace keeps a deterministic ordered index by
`TranslationUnitId` and a secondary deterministic index from `SourceBinding`
to `TranslationUnitId` for bound units. Both are updated together when a unit
is inserted; ordinary mutations do not change source facts.

## Bound and detached units

Every unit has a `SourceStatus`:

- **Bound**: its binding, fingerprint, and layout describe the current source.
  Only bound units appear at a source cell, count toward translation
  progress, can be edited, and will be exported.
- **Detached**: a source update found no current occurrence for it. The
  reason is one of sheet removed, row removed, cell removed, column
  unresolved, not translatable, or binding conflict. The unit keeps its last
  bound facts, target, note, and review state, owns no binding, and cannot be
  edited through the ordinary editor. Every later source update evaluates it
  again and binds it when its occurrence is established.

Detached units exist so that a source update can always complete without
discarding a translation or attaching it to an unrelated string. The
workspace exposes them through `Workspace::detached_units` and
`ProjectSession::detached_units`.

## Project/source binding

Workspace metadata records one source language, one target language, and the
verified HXS `contentId` of the source the bound units describe. Exactly one
target language is canonical per workspace; target-language selection is not
repeated on individual units.

The source is identified by content only. The HXS `snapshotId` and game
version are properties of the opened source package, not of the project, so a
game version with identical extracted content opens the project unchanged.

The workspace source adapter accepts an already verified `HxsSnapshot`. It
checks that the requested sheet, row, subrow, and String column exist, copies
the verified cell and row hashes into a `SourceFingerprint`, copies the sheet
schema hash and column offset into a `SourceLayout`, and derives a new v1
`TranslationUnitId`. It does not reopen or independently verify SQLite
internals. `ProjectSession` also records the row key when the sheet has a row
key column; it detects the column once per sheet with the session's guidance
and caches it for the immutable source.

Source facts change only through a source update, described in
[`rebase.md`](./rebase.md).

## Target validity

Manual target edits use `aeria-se` intrinsic validation. Understood syntax and
opaque-but-preservable syntax are valid targets; malformed or unsafe syntax is
rejected. Manual translation does not require source and target protected
structures to match. Strict structure compatibility remains a safety mechanism
for future assisted or AI translation.

## Project scope

Exactly one target language is canonical per project.

Project-shared data may include:

- workspace manifest and source identity
- translated units, bound or detached
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

- `draft`
- `reviewed`
- `needs-review`

`reviewed` means another human has reviewed the translation. AI output is
`draft`. A source update that changes a unit's source content moves it to
`needs-review`; a deterministic `Unchanged` or `EncodingChanged` outcome keeps
its review state.

Changing a target always resets its unit to `draft`. Marking a unit
`reviewed` is an explicit workspace operation. Review is never inferred from
Git commits, approvals, or other repository state.

## Persistence

`aeria-workspace` provides the Workspace Format persistence adapter. It writes
[Workspace Format v2](../formats/workspace-v2.md) and reads
[Workspace Format v1](../formats/workspace-v1.md) only to migrate it.

`WorkspaceStore` binds to a repository root and offers:

- `read_stored` (crate-internal): read and validate every managed file in
  either format, counting bound units whose binding another bound unit
  already claims (a merge or interrupted-update state that a source update
  resolves);
- `load`: read, require the current format and unique bound bindings, and
  activate the workspace for editing;
- `read_metadata`: read only the manifest, for example to learn the source
  language before building a source package;
- `initialize`: publish a new `.aeria/` directory;
- `persist_unit`: rewrite the one shard selected by a changed unit;
- `publish_source_update` (crate-internal): rewrite the given shards and then
  the manifest after a source update.

After `load` or initialization the store keeps a session-scoped validated
layout, manifest, and unit index for interactive mutations. `persist_unit`
uses that cache to avoid repeating directory enumeration and JSONL parsing,
while preserving the same invariant checks; callers that have not loaded
through the store take the full validation path. The cache is updated only
after atomic publication and is invalidated on publication failure. Before a
cached publish, the store checks the managed `.aeria` paths against the cache
snapshot using path metadata and content hashes for the manifest and target
shard. If an external change is detected, the mutation fails closed; the
caller must reload, so an external unit is never silently discarded.

On the ordinary target/note/review path, an existing unit's status, binding,
fingerprint, layout, and row key are immutable. A new unit must be bound and use a
binding not owned by another bound unit.

Canonical file replacements are written to a temporary file outside the
managed `.aeria/` namespace and published with a cross-platform atomic file
replacement. Initialization stages the complete directory and publishes it
only after serialization succeeds. The atomicity guarantee is per file. A
source update publishes the manifest last, so an interruption leaves the
previous content ID in place and the update is planned again on the next
open; see [`rebase.md`](./rebase.md#applying).

## Compatibility

The workspace has an explicit `formatVersion`. New Aeria versions either open
an older workspace directly or migrate it without data loss. Workspace Format
v1 is migrated by the source update workflow; unsupported versions are
reported and never reinterpreted.
