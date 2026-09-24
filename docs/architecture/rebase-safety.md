# Source update rules

This document specifies how `aeria-rebase::plan_source_update` decides the
outcome of every managed translation unit. The workflow around it is
described in [`rebase.md`](./rebase.md).

## Inputs

For each unit the planner reads only persisted workspace facts:

| Fact | Source |
| --- | --- |
| `SourceStatus` | `bound`, or `detached` with a reason. |
| `SourceBinding` | Sheet, row, subrow, and column index the unit was last bound to. |
| `SourceFingerprint` | Macro-text hash, optional raw-value hash, and row technical hash at that binding. |
| `SourceLayout` | Sheet schema hash and String column offset at that binding; absent only for units read from Workspace Format v1. |
| Row key | Macro-text hash of the row key at that binding, when the sheet was keyed. |

From the new source it reads the verified sheet catalog (schema hash and
column definitions) and the hash-only String occurrences of every sheet that
contains managed units, and detects each such sheet's row key column. The
permission predicate comes from the new HSG. The previous snapshot is never
read.

Detached units are planned exactly like bound units, from the facts they were
last bound with, so a unit whose occurrence returns is attached again.

## Why column indexes need a layout

An HXS column index is a physical position in one sheet schema. When a patch
adds, removes, or reorders columns, the same index can name a different
column, and the same logical column can move to another index. A binding is
therefore meaningful only together with the schema it was resolved in. The
sheet schema hash identifies that schema generation; the column offset
records the physical position of the String column within it.

Row and subrow IDs are kept as they are, except in keyed sheets; see
[Row keys](#row-keys).

## Row keys

A String column is a sheet's *row key column* when, in the snapshot:

1. the sheet has at least two rows;
2. every row has a non-empty macro text in the column;
3. the values are unique across rows; and
4. the HSG permits none of the column's occurrences, which means the text is
   identical in every official evidence language.

When several columns qualify, the lowest column index is used. Keys are
compared by macro-text hash. Condition 4 excludes translatable text: a
translated name that is merely unique must never act as an identity key,
because editing it would look like removing the row.

Keyed resolution applies to a sheet when the new sheet has a row key column
and at least one unit's persisted key is found in it. Then:

- a unit whose key is found resolves to the row that holds the key, which may
  differ from its previous row (row continuity `RowKey`);
- a unit whose key is not found is `Detached(RowRemoved)`, even when its
  previous row ID still exists and holds another line;
- a unit without a persisted key keeps its row ID.

When no persisted key is found at all, for example because the key column
changed, every unit keeps its row ID. A bound outcome always proposes the key
of its new row when the new sheet is keyed, so the next update can use it.
Only the row is resolved by key; the column is resolved as described below.

## Schema generations

The planner groups each sheet's units by the schema hash in their layout.

- **Removed sheet.** The sheet is not in the new source. Every unit is
  `Detached(SheetRemoved)`.
- **Unavailable sheet.** The sheet is not stored but the new source lists it
  as excluded (HXS v2): the game still has it but it could not be read or
  represented. Every unit is `Detached(SheetUnavailable)` and its schema
  update is marked unavailable. Like every detached unit, it is re-evaluated
  by the next source update and reattaches once the sheet is readable.
- **Same generation.** The unit's schema hash equals the current schema hash.
  Its column index is interpreted directly (column continuity `SameColumn`).
- **Same content, no layout.** A bound Workspace Format v1 unit without a
  layout is treated as the same generation when the workspace content ID
  equals the new content ID, because identical content implies an identical
  schema.
- **Other generation.** Every other group of units, including v1 units after
  a content change and detached units last bound in an older schema, is
  resolved through a column mapping.

## Column mapping

A mapping is computed per sheet and per previous schema generation, from the
units of that generation only:

1. **Votes.** For each unit, find the String columns of its resolved row in
   the new sheet whose macro-text hash equals the unit's persisted
   macro-text hash. If exactly one column matches, the unit votes for it.
   Several matches or none cast no vote, and a unit whose row key was
   removed casts no vote.
2. **Content evidence.** A previous column maps to the column that received
   a strict majority of the votes cast by its units
   (`ExactContent { supporting, cast }`).
3. **Unchanged position.** A previous column whose units cast no vote at all
   maps to itself only when the new schema has a String column at the same
   index and the same offset as the units' layout (`UnchangedPosition`).
4. **Unresolved.** Split votes without a strict majority, missing layout
   evidence, or no matching column leave the previous column unresolved.
5. **Injectivity.** If two previous columns map to one current column, both
   become unresolved.

Every unit of an unresolved column is `Detached(ColumnUnresolved)`. A mapping
never changes the sheet, row, or subrow; it only reinterprets the column.

Exact-hash votes are deterministic source facts, not similarity. A single
coincidental match cannot override a majority, and ties, splits, and
collisions are never broken by order or proximity.

## Occurrence resolution

After the row and column are known, the unit resolves at
`(sheet, row, subrow, column)`:

| Condition | Outcome |
| --- | --- |
| The column is not a String column in the new schema. | `Detached(CellRemoved)` |
| The row/subrow has no occurrence in that column. | `Detached(RowRemoved)` |
| The macro-text hash differs. | `SourceChanged` |
| The macro text is equal, but the raw-value hash or its presence differs. | `EncodingChanged` |
| Macro-text and raw-value hashes are equal. | `Unchanged` |

The translation is written and exported as macro text, and HXS macro text
represents the complete structured string, so a raw-only difference is an
encoding change, not a content change.

The row technical hash is compared separately and reported as
`SourceContextStatus`; it never changes the outcome. A bound outcome proposes
the new binding, fingerprint, layout, and row key.

## Permission

A bound outcome whose binding is not permitted by the new HSG becomes
`Detached(NotTranslatable)`. This happens when the game makes a string
identical in every evidence language, for example by blanking removed
content.

## Binding conflicts

At most one unit may own a binding. When several bound outcomes propose the
same binding, the planner keeps one, preferring in order:

1. a unit that is bound and keeps its exact binding and layout;
2. a unit with unchanged content;
3. a unit that was bound rather than detached;
4. the smallest `TranslationUnitId`.

Every other claimant is `Detached(BindingConflict)`.

## Truth matrix

| Sheet | Schema generation | Column | Row | Content | Permission | Outcome |
| --- | --- | --- | --- | --- | --- | --- |
| removed | any | any | any | any | any | `Detached(SheetRemoved)` |
| excluded as unreadable | any | any | any | any | any | `Detached(SheetUnavailable)` |
| present | any | any | keyed, key removed | any | any | `Detached(RowRemoved)` |
| present | same | not a String column | any | any | any | `Detached(CellRemoved)` |
| present | same | String | missing | any | any | `Detached(RowRemoved)` |
| present | same | String | present | equal | permitted | `Unchanged` |
| present | same | String | present | raw bytes only | permitted | `EncodingChanged` |
| present | same | String | present | macro text changed | permitted | `SourceChanged` |
| present | other, mapped | mapped column | present | as above | permitted | as above |
| present | other, unresolved | any | any | any | any | `Detached(ColumnUnresolved)` |
| present | any | resolved | present | any | blocked | `Detached(NotTranslatable)` |

A binding conflict can then detach any bound outcome with `BindingConflict`.

## Applying outcomes

- `Unchanged` and `EncodingChanged`: keep ID, target, note, and review
  state; set status `bound` and replace binding, fingerprint, layout, and row
  key with the proposed facts. A context-only change, a new layout, or a new
  row key is still written.
- `SourceChanged`: as above, and set review state `needs-review`.
- `Detached(reason)`: set the status to the reason; keep binding,
  fingerprint, layout, row key, target, note, and review state.

No outcome removes a unit or recomputes its `TranslationUnitId`.

## Coordinate reuse and row shifts

For

```text
previous: A at X
new:      A at Y, unrelated B at X      (same schema)
```

the unit at X stays at X and is `SourceChanged` with B's content. It is never
moved to Y. For a row shift

```text
previous: Alpha@100, Beta@101
new:      Inserted@100, Alpha@101, Beta@102
```

in a sheet without a row key column the unit at 100 is `SourceChanged` with
"Inserted", the unit at 101 is `SourceChanged` with "Alpha", and neither is
rebound. Both are visible review work, never a silent move.

In a keyed sheet

```text
previous: K1 Alpha@100, K2 Beta@101
new:      K0 Inserted@100, K1 Alpha@101, K2 Beta@102
```

the units follow K1 to 101 and K2 to 102 and stay `Unchanged`. If K2 had been
removed and row 101 reused for a new line K3, the unit last bound at 101
would be `Detached(RowRemoved)` instead of being attached to K3's text.

For a column insertion

```text
previous columns: 0 name, 2 description
new columns:      2 name, 4 description
```

the units of column 0 vote for column 2 and the units of column 2 vote for
column 4. Both columns map, and every translation follows its logical column.
Without the mapping, description translations would have landed on the name
column.

## Required coverage

The `aeria-rebase` integration tests cover:

- identical sources, pure and order-independent planning;
- macro-text, raw-only (`EncodingChanged`), and context-only changes at a
  surviving binding;
- removed rows and sheets, with last facts preserved;
- row shifts that stay at their binding and require review in unkeyed
  sheets;
- keyed rows that follow their line after an insertion, removed keyed lines
  whose row ID is reused, and units without keys that keep their rows and
  receive keys;
- column insertion mapped by content evidence, including a changed cell in a
  mapped column;
- schema changes without evidence, mapped only by unchanged position;
- split and colliding evidence that stays unresolved;
- permission loss and later reattachment;
- binding conflicts and their deterministic winner;
- Workspace Format v1 units with and without a content change;
- language mismatch and repeated IDs as errors;
- equivalent physical storage order producing the same plan.

`aeria-rebase` unit tests cover row key column detection. `aeria-workspace`
integration tests cover applying updates, permission-driven detachment,
hotfix content reuse, Workspace Format v1 migration, repeating an update
interrupted before the manifest was written, and a keyed dialogue sheet whose
translations follow their lines through a session.

## Model-based check

A bounded model runs in CI. It places three logical rows into four row slots
in every injective way (73 placements), applies all eight content-mutation
masks, and optionally inserts an unrelated duplicate: `73 * 8 * 2 = 1,168`
verified HXS transitions in one schema. For every transition it checks that

1. a unit whose binding survives stays at that binding with same-row,
   same-column continuity;
2. its outcome is `Unchanged`, `EncodingChanged`, or `SourceChanged` by
   macro/raw content only;
3. its context status follows the row technical hash;
4. a unit whose binding is missing is detached with no proposed facts;
5. every unit appears exactly once with its ID unchanged.

The model prints its state count and the number of bound, source-changed,
detached, and wrong mappings; wrong mappings must be zero.
