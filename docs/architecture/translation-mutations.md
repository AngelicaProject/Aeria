# Transactional translation mutations

`ProjectSession` owns the application-level mutation API for ordinary editor
changes. The session keeps the game source, sparse `Workspace`, and
`WorkspaceStore` together so a successful mutation commits the same state to
memory and to the workspace files.

```text
ProjectSession mutation
        ↓
translation permission of the game cell
        ↓
verify the unit against the game cell
        ↓
Workspace domain mutation
        ↓
WorkspaceStore canonical shard persistence
        ↓
committed live session state
```

## Target mutations

`ProjectSession::set_target` is the create-or-update application operation.
Before any Workspace operation, it requires the exact `SourceBinding` to be a
translatable String cell of the game (see
[`source.md`](./source.md#translation-permission)). Any other binding returns
the typed `SourceNotTranslatable` mutation error and cannot create or modify a
translation unit or write a shard.

When no unit owns the supplied binding, the session reads the cell's source
facts (binding, layout hash, text, and row key) from the game, derives the
stable `TranslationUnitId`, and creates the sparse unit. When a unit already
exists, its durable ID and source facts are preserved while the existing
workspace target semantics apply.

`set_target` rejects empty or whitespace-only targets before changing the
in-memory workspace or writing a shard. Existing records with an empty target
remain readable under the persisted contract, but ordinary target edits cannot
create or update a unit to an empty value.

Only bound units own a binding. A detached unit keeps its last binding but is
not found by `set_target`, and `set_note` and `set_review_state` reject it
with `DetachedUnit`; detached units change only through a source update. If a
new unit's derived ID equals a detached unit's ID (the same source text at the
same coordinate), creation fails with `DuplicateUnitId`; the next source
update attaches the detached unit again when its occurrence is established.

Target changes reset review state to `Draft` through
`Workspace::update_target`. Setting the identical target is a successful
no-op and does not rewrite the canonical shard.

## Assisted targets

`ProjectSession::set_assisted_target(binding, target, expected,
replace_reviewed)` writes a target produced by assisted translation. In
addition to every `set_target` check, it:

- reads the source macro of the binding (`source_macro`) and
  requires the target to satisfy the assisted structure policy in
  [`strings.md`](./strings.md#assisted-structure-policy);
- compares the unit's current target and review state with `expected`, the
  state captured when the translation was produced (`assisted_state`
  returns it; `None` fields mean untranslated), and refuses with `Conflict`
  when they differ, so a translation never replaces work saved after it was
  produced;
- refuses to replace a `reviewed` unit unless `replace_reviewed` records the
  user's explicit approval.

The written target is a draft, as for any target change. A refused write
changes nothing in memory or on disk.

`ProjectSession::set_assisted_targets(writes, replace_reviewed)` writes a
batch of `AssistedWrite`s (binding, target, expected state, and an optional
review state to set with the target). Each is checked as by
`set_assisted_target` and applied in memory; a refused one is left out with
its error. The changed units are then persisted with
`WorkspaceStore::persist_units`: each changed shard is serialized once and
the shards are published in parallel, each atomically. When a shard cannot be
published, every unit of it is restored in memory and reported as
`NotSaved`; the other shards stay written. Results are in the order of
`writes`.

## Notes and review state

`set_note` and `set_review_state` operate only on existing units. Notes may be
set or cleared without changing review state. Review transitions use the
existing `ReviewState` values and semantics. Identical note or review-state
requests are successful no-ops and do not rewrite canonical files.

## Source integrity

Before an ordinary mutation of an existing unit, the session reads the source
facts of the unit's `SourceBinding` from the game and compares them with the
persisted facts. A mismatch is a typed source-integrity error containing the
unit ID, binding, persisted facts, and the game's facts. The mutation does not
repair, update source facts, change review state, or write files.

First-unit creation reads the same facts through `Workspace::create_unit`.
Callers do not supply or control source facts or durable IDs.

## Transaction and persistence behavior

Each mutation affects one unit and one canonical ID shard. The session stages
the domain change in the live workspace, calls `WorkspaceStore::persist_unit`,
and treats the pair as one caller-visible transaction. If persistence fails,
an existing unit is restored from its saved unit value; a newly created unit
is removed. Thus a failed mutation does not leave the live session ahead of
the previously committed files on disk. `WorkspaceStore` remains the owner of
manifest validation, canonical serialization, identity/binding checks, and
atomic shard publication.

The session does not clone the full workspace, materialize the source corpus,
enumerate all units, rebuild indexes, or rewrite every shard. Normal work is
bounded to one source lookup, one sparse-unit lookup, and one shard
persistence operation. The store may perform its existing global binding
check when inserting a new durable unit.

Successful mutations update the live workspace before returning, so
`page_translation_rows` immediately reads the committed state from the
same session. The desktop command returns the resulting overlay for the
changed unit, allowing the renderer to patch one loaded cell without
reloading pages. The persisted format, canonical JSON/JSONL encoding, identity
derivation, source-binding contract, and target validation rules are
unchanged. Translation permission adds only the gate for target mutations.

Deletion/reset-to-untranslated, bulk or multi-shard transactions, source
updates, export, Git, AI, Tauri commands, and UI state are outside this
layer's scope.

The desktop application boundary delegates ordinary target, note, and review
commands to this API and maps its typed failures to the IPC error contract.
