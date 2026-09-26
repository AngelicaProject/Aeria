# Translation workspace

A translation repository is a sparse overlay over the installed game's text.

## Sparse overlay

The repository stores entries only for translated or otherwise explicitly
project-managed units. It does not contain one record for every source
string.

A translation entry contains enough information for stable identity, current
binding verification, target content, review state, notes, and safe source
updates without duplicating the complete source corpus. A unit stores its
source facts: the binding, the layout hash of its sheet, the source text it
was translated from, and the row key of its row when the sheet is keyed (see
[`../formats/workspace-v3.md`](../formats/workspace-v3.md)).

The workspace is sparse in memory as well as on disk: creating a workspace
does not enumerate source cells. A unit is created only when a caller
requests one String cell, and source data is read on demand through
`aeria-source`.

The in-memory workspace keeps a deterministic ordered index by
`TranslationUnitId` and a secondary deterministic index from `SourceBinding`
to `TranslationUnitId` for bound units. Both are updated together when a unit
is inserted; ordinary mutations do not change source facts.

## Bound and detached units

Every unit has a `SourceStatus`:

- **Bound**: its source facts describe the game at the project's version.
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
game version the bound units describe. Exactly one target language is
canonical per workspace; target-language selection is not repeated on
individual units.

A unit is created from `SourceSheet::facts`, which reads the cell's text,
the sheet's layout hash, and the row key from the game; the unit's
`TranslationUnitId` is derived from the binding and the text.

Source facts change only through a source update, described in
[`rebase.md`](./rebase.md).

## Target validity

Manual target edits use `aeria-se` intrinsic validation. Understood syntax and
opaque-but-preservable syntax are valid targets; malformed or unsafe syntax is
rejected. Manual translation does not require source and target protected
structures to match. Assisted translation must also satisfy the assisted
structure policy in [`strings.md`](./strings.md#assisted-structure-policy).

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
- the game installation path and the disposable sheet catalog cache
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

`aeria-workspace` provides the Workspace Format persistence adapter. It reads
and writes [Workspace Format v3](../formats/workspace-v3.md); earlier formats
are reported as unsupported.

`WorkspaceStore` binds to a repository root and offers:

- `read_stored` (crate-internal): read and validate every managed file,
  counting bound units whose binding another bound unit already claims (a
  merge or interrupted-update state that a source update resolves);
- `load`: read, require unique bound bindings, and activate the workspace for
  editing;
- `read_metadata`: read only the manifest, for example to learn the source
  language before opening the game;
- `initialize`: publish a new `.aeria/` directory;
- `persist_unit`: rewrite the one shard selected by a changed unit;
- `publish_source_update` (crate-internal): rewrite the given shards and then
  the manifest after a source update.

After `load` or initialization the store keeps a session-scoped validated
layout, manifest, and unit index for interactive mutations. `persist_unit`
uses that cache to avoid repeating directory enumeration and JSONL parsing,
while preserving the same invariant checks; callers that have not loaded
through the store take the full validation path. The cache is updated only
after atomic publication and is invalidated on publication failure. After a
publish only the written shard is hashed again and the directories are read
again; the other files keep the state verified just before it, so one write
costs one shard rather than the whole workspace. Before a
cached publish, the store checks the managed `.aeria` paths against the cache
snapshot using path metadata and content hashes for the manifest and target
shard. If an external change is detected, the mutation fails closed; the
caller must reload, so an external unit is never silently discarded.

On the ordinary target/note/review path, an existing unit's status and source
facts are immutable. A new unit must be bound and use a
binding not owned by another bound unit.

Canonical file replacements are written to a temporary file outside the
managed `.aeria/` namespace and published with a cross-platform atomic file
replacement. Initialization stages the complete directory and publishes it
only after serialization succeeds. The atomicity guarantee is per file. A
source update publishes the manifest last, so an interruption leaves the
previous game version in place and the update is planned again on the next
open; see [`rebase.md`](./rebase.md#applying).

## Compatibility

The workspace has an explicit `formatVersion`. Future Aeria versions either
open an older workspace directly or migrate it without data loss.
Unsupported versions, including Workspace Format v1 and v2, are reported and
never reinterpreted.
