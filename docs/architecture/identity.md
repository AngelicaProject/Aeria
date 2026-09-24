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

`TranslationUnitId` is derived only when a unit is first created. A source update retains the existing ID even when the column moves, the source text changes, any source fingerprint or layout field changes, or the unit is detached. It updates the source facts in place and never recomputes an existing unit ID. The source layout does not participate in the digest.

## Source occurrence rule

One managed String cell is one translation unit. A binding is interpreted in
the sheet schema generation recorded by the unit's `SourceLayout`, because an
HXS column index is only a position within one schema.

During a source update:

- in an unchanged schema generation, the previous `SourceBinding` is the
  continuity key, except that in a keyed sheet the row is found by the
  persisted row key, and a removed key detaches the unit;
- in a changed generation, the sheet, row, and subrow are kept and only the
  column is reinterpreted through a deterministic sheet-level column mapping
  established by exact content evidence or an unchanged column position;
- `macroTextHash` classifies the resolved occurrence as `SourceChanged`; a
  change of `rawValueHash` alone is `EncodingChanged`; otherwise it is
  `Unchanged`; `rowTechnicalHash` is context only;
- a unit without an established occurrence is detached, not guessed.

Similarity, partial hashes, coordinate proximity, and a unique candidate at
another row never establish identity. A row key is an exact,
language-invariant source value, not a similarity measure. The existing `TranslationUnitId` is
kept in every case and is never recomputed. See
[`rebase-safety.md`](./rebase-safety.md) for the complete rules.
