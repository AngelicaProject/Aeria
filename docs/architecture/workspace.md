# Translation workspace

A translation repository is a sparse overlay over immutable HXS source data.

## Sparse overlay

The repository stores entries only for translated or otherwise explicitly project-managed units. It does not contain one record for every source string.

A translation entry contains enough information for stable identity, current binding verification, target content, review state, notes, and safe migration without duplicating the complete source corpus. The first in-memory workspace slice stores the source binding and verified macro/raw/row-technical hashes, but never the full source macro text.

The workspace is sparse in memory as well as on disk: creating a workspace from a verified HXS snapshot does not enumerate source cells. A unit is created only when a caller requests one verified String cell, and source data is read on demand through `aeria-hxs`.

## Project/source binding

Workspace metadata records one source language, one target language, the current verified HXS `contentId`, and the current verified HXS `snapshotId`. Exactly one target language is canonical per workspace; target-language selection is not repeated on individual units.

The workspace source adapter accepts an already verified `HxsSnapshot`. It checks that the requested sheet, row, subrow, and String column exist, copies only the verified cell and row hashes into a `SourceFingerprint`, and derives a new v1 `TranslationUnitId`. It does not reopen or independently verify SQLite internals.

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
- source snapshot paths/cache
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

Changing a target always resets its unit to `draft`. Marking a unit `reviewed` and marking a known meaningful source update as `needs-review` are explicit domain operations; review is never inferred from Git commits, approvals, or other repository state.

## Compatibility

The workspace has an explicit `formatVersion` from the first public version. New Aeria versions must either open an older public workspace directly or migrate it without data loss.

The concrete workspace serialization and file layout remain draft. This domain slice does not select JSON, JSONL, TOML, YAML, SQLite, or a custom extension.
