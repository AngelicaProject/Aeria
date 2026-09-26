# Translation identity

Aeria distinguishes source coordinates from durable translation identity.

## Source coordinate

A String cell of the game is addressed by:

- sheet
- row
- subrow
- column index

This is also the runtime lookup shape Harmonia uses. A column index is a
position in one sheet layout (see [`source.md`](./source.md#layout)), and a
row ID is a position in one version of a sheet, so a coordinate alone is not
durable across game versions.

## Translation unit identity

A translation unit has an Aeria-owned identity that survives a coordinate
change across game versions. The current coordinate is a binding of that
unit, not its identity.

New identities are derived deterministically from source facts, so
independent branches that translate the same new source cell create the same
ID.

### TranslationUnitId

The ID is 16 bytes, written as 32 lowercase hexadecimal characters. It is the
first 16 bytes of SHA-256 over, in order:

1. the UTF-8 bytes of the domain separator `aeria.translation-unit.v2`;
2. the sheet name, framed as a little-endian `u32` byte length followed by
   its UTF-8 bytes;
3. the row ID as a little-endian `u32`;
4. the subrow ID as a little-endian `u16`;
5. the column index as a little-endian `u32`;
6. the source text, framed like the sheet name.

The target, review state, note, layout, row key, game version, and languages
do not take part.

The ID is derived only when a unit is created. A source update keeps the
existing ID when the row or column moves, the source text changes, or the unit
is detached; it updates the unit's source facts in place and never derives
the ID again.

## Source occurrence rule

One String cell is one translation unit. A unit's binding is interpreted in
the layout recorded with the unit. During a source update:

- in an unchanged layout, the previous column index is kept; in a keyed sheet
  the row is found by the unit's row key, and a removed key detaches the
  unit;
- in a changed layout, the sheet, row, and subrow are kept as above and only
  the column is reinterpreted through a deterministic sheet-level column
  mapping established by exact text evidence;
- a different text at the resolved cell marks the unit for review;
- a unit without an established cell is detached, not guessed.

Similarity, coordinate proximity, and a unique candidate at another row never
establish identity. A row key is an exact, language-invariant source value,
not a similarity measure. See [`rebase-safety.md`](./rebase-safety.md) for
the complete rules.
