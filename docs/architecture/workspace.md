# Translation workspace

A translation repository is a sparse overlay over immutable HXS source data.

## Sparse overlay

The repository stores entries only for translated or otherwise explicitly project-managed units. It does not contain one record for every source string.

A persisted translation entry is expected to contain enough information for stable identity, current binding verification, target content, review state, notes, and safe migration without duplicating the complete source corpus.

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

## Compatibility

The workspace has an explicit `formatVersion` from the first public version. New Aeria versions must either open an older public workspace directly or migrate it without data loss.
