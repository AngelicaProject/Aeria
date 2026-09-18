# Translation identity

Aeria distinguishes snapshot coordinates from durable translation identity.

## Source coordinate

Within an HXS snapshot, a string occurrence is addressed by:

- sheet
- row
- subrow
- column

This binding is also the runtime lookup shape expected by the in-game consumer.

## Translation unit identity

A translation unit has an Aeria-owned stable identity that can survive a coordinate change across game versions. The current source coordinate is a binding of that unit, not the durable identity itself.

New identities are generated deterministically from source facts so that independent branches encountering the same new source occurrence do not invent incompatible random IDs.

### TranslationUnitId v1

Newly created units use the canonical textual form:

```text
tu1:<64 lowercase hexadecimal characters>
```

The digest is SHA-256 over these bytes, in order:

1. the unframed UTF-8 bytes of the domain separator `aeria.translation-unit.v1`;
2. a source-language string framed as a little-endian `u32` byte length followed by UTF-8 bytes;
3. the sheet-name string with the same framing;
4. the row ID as a little-endian `u32`;
5. the subrow ID as a little-endian `u16`;
6. the column index as a little-endian `u32`;
7. the raw 32-byte source macro-text hash.

The raw-value hash, row technical hash, target language, target macro string, review state, game version, `contentId`, and `snapshotId` do not participate in the digest. This keeps equivalent source occurrences independent of a particular snapshot or project target.

`TranslationUnitId` is derived only when a unit is first created. A source update or rebase retains the existing ID even when the coordinate moves, source text changes, or any source fingerprint field changes. Rebase must update the current binding and fingerprint in place; it must never recompute an existing unit ID.

## Source occurrence rule

One managed String cell is one translation unit. During a verified source
transition, the previous `SourceBinding` is the authoritative continuity key
when that binding exists in the new snapshot. The planner then compares only
the cell's `macroTextHash` and `rawValueHash`:

- equal content is `Unchanged`;
- changed content is `SourceChanged` at the same binding;
- a changed `rowTechnicalHash` is context diagnostics, not identity evidence.

If the old binding is missing, the unit is `Ambiguous`. Similarity, partial
hashes, coordinate movement, and a unique candidate at another binding never
establish cross-binding identity. The existing `TranslationUnitId` is kept;
future apply semantics may update the source facts in place and require review
for `SourceChanged`, but must not recompute the ID. See
[`rebase-safety.md`](./rebase-safety.md) for the complete transition contract.
