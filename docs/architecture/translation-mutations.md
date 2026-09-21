# Transactional translation mutations

`ProjectSession` owns the application-level mutation API for ordinary editor
changes. The session keeps the verified HXS snapshot, sparse `Workspace`, and
`WorkspaceStore` together so a successful mutation commits the same state to
memory and to Workspace Format v1.

```text
ProjectSession mutation
        ↓
exact HSG allowlist check
        ↓
verify exact current HXS source
        ↓
Workspace domain mutation
        ↓
WorkspaceStore canonical shard persistence
        ↓
committed live session state
```

## Target mutations

`ProjectSession::set_target` is the create-or-update application operation.
Before any Workspace operation, it requires the exact `SourceBinding` to be
present in the compatible HSG allowlist. A blocked binding returns the typed
`SourceNotTranslatable` mutation error and cannot create or modify a
translation unit or write a shard.

When no unit owns the supplied binding, the session verifies that the current
HXS coordinate is a String occurrence, derives its source fingerprint and
stable `TranslationUnitId` through the existing workspace/domain rules, and
creates the sparse unit. When a unit already exists, its durable ID, binding,
and source fingerprint are preserved while the existing workspace target
semantics apply.

An empty target is explicit state. The first `set_target(binding, "")`
creates a real `TranslationUnit` with an empty `target_macro`; it does not
mean that the source is untranslated and does not delete a unit.

Target changes reset review state to `Draft` through
`Workspace::update_target`. Setting the identical target is a successful
no-op and does not rewrite the canonical shard.

## Notes and review state

`set_note` and `set_review_state` operate only on existing units. Notes may be
set or cleared without changing review state. Review transitions use the
existing `ReviewState` values and semantics. Identical note or review-state
requests are successful no-ops and do not rewrite canonical files.

## Source integrity

Before an ordinary mutation of an existing unit, the session resolves the
unit's current `SourceBinding` against its verified HXS snapshot and compares
the resulting `SourceFingerprint` with the persisted unit fingerprint. A
mismatch is a typed source-integrity error containing the unit ID, binding,
persisted fingerprint, and verified fingerprint. The mutation does not repair,
rebase, update source facts, change review state, or write files.

First-unit creation performs the same verified source lookup through the
existing `Workspace::create_unit_from_hxs` domain seam. Callers do not supply
or control source hashes or durable IDs.

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
bounded to one verified source lookup, one sparse-unit lookup, and one shard
persistence operation. The store may perform its existing global binding
check when inserting a new durable unit.

Successful mutations update the live workspace before returning, so
`page_translation_rows` immediately reads the committed state from the
same session. The desktop command returns the resulting overlay for the
changed unit, allowing the renderer to patch one loaded cell without
reloading pages. The persisted format, canonical JSON/JSONL encoding, identity
derivation, source-binding contract, and target validation rules are
unchanged. HSG adds only the permission gate for target mutations.

Deletion/reset-to-untranslated, bulk or multi-shard transactions, source
update/rebase, export, Git, AI, Tauri commands, and UI state are outside this
layer's scope.

The desktop application boundary delegates ordinary target, note, and review
commands to this API and maps its typed failures to the IPC error contract.
