# Source update rules

This document specifies how `aeria-rebase::plan_source_update` decides the
outcome of every translation unit. The workflow around it is described in
[`rebase.md`](./rebase.md).

## Inputs

For each unit the planner reads only its persisted facts (see
[`../formats/workspace-v3.md`](../formats/workspace-v3.md)):

| Fact | Meaning |
| --- | --- |
| `status` | `bound`, or detached with a reason. |
| `sheet`, `row`, `subrow`, `column` | Where the unit was last bound. |
| `layout` | The layout hash of the sheet at that binding. |
| `source` | The source text at that binding. |
| `key` | The row key at that binding, when the sheet was keyed. |

From the game it reads, for every sheet that holds units, the sheet, its
layout, permission, and row key column (see [`source.md`](./source.md)). The
previous game version is never read: the patch has already replaced it.

Detached units are planned exactly like bound units, from the facts they were
last bound with, so a unit whose cell returns is attached again.

## Why column indexes need a layout

A column index is a position in one sheet layout. When a patch adds, removes,
or reorders String columns, the same index can name a different column, and
the same logical column can move to another index. A binding is therefore
meaningful only together with the layout it was resolved in.

Row and subrow IDs are kept as they are, except in keyed sheets; see
[Row keys](#row-keys).

## Row keys

The row key column of a sheet is defined in
[`source.md`](./source.md#row-keys). Keyed resolution applies to a sheet when
the current sheet has a row key column and at least one unit's persisted key
is found in it. Then:

- a unit whose key is found resolves to the row that holds the key, which may
  differ from its previous row (row continuity `RowKey`);
- a unit whose key is not found is `Detached(RowRemoved)`, even when its
  previous row ID still exists and holds another line;
- a unit without a persisted key keeps its row ID.

When no persisted key is found at all, for example because the key column
changed, every unit keeps its row ID. A bound outcome always proposes the key
of its new row when the current sheet is keyed. Only the row is resolved by
key; the column is resolved as described below.

## Layout generations

The planner groups each sheet's units by the layout hash they record.

- **Removed sheet.** The sheet is not in the sheet list. Every unit is
  `Detached(SheetRemoved)`.
- **Unavailable sheet.** The sheet is listed but cannot be read. Every unit
  is `Detached(SheetUnavailable)`. Like every detached unit, it is evaluated
  again by the next source update and reattaches once the sheet is readable.
- **Same layout.** The unit's layout equals the current layout. Its column
  index is interpreted directly (column continuity `SameColumn`).
- **Other layout.** Every other group of units is resolved through a column
  mapping.

## Column mapping

A mapping is computed per sheet and per previous layout, from the units of
that layout only:

1. **Votes.** For each unit, find the String columns of its resolved row
   whose text equals the unit's `source`. If exactly one column matches, the
   unit votes for it. Several matches or none cast no vote, and a unit whose
   row key was removed casts no vote.
2. **Content evidence.** A previous column maps to the column that received
   a strict majority of the votes cast by its units
   (`ExactContent { supporting, cast }`).
3. **Unresolved.** A column without votes or with split votes without a
   strict majority is unresolved.
4. **Injectivity.** If two previous columns map to one current column, both
   become unresolved.

Every unit of an unresolved column is `Detached(ColumnUnresolved)`. A mapping
never changes the sheet, row, or subrow; it only reinterprets the column.

Exact-text votes are deterministic source facts, not similarity. A single
coincidental match cannot override a majority, and ties, splits, and
collisions are never broken by order or proximity.

## Cell resolution

After the row and column are known, the unit resolves at
`(sheet, row, subrow, column)`:

| Condition | Outcome |
| --- | --- |
| The column is not a String column in the current layout. | `Detached(CellRemoved)` |
| The row or subrow does not exist. | `Detached(RowRemoved)` |
| The text differs from `source`. | `SourceChanged` |
| The text equals `source`. | `Unchanged` |

Only the printed text is compared. When a patch re-encodes a string without
changing its text, the unit is `Unchanged`; export takes the current bytes
from the game (see [`export.md`](./export.md)).

A bound outcome proposes the new binding, layout, text, and row key.

## Permission

A bound outcome whose cell is not translatable is
`Detached(NotTranslatable)`. This happens when the game makes a string the
same in every language, for example by blanking removed content.

## Binding conflicts

At most one unit may own a binding. When several bound outcomes propose the
same binding, the planner keeps one, preferring in order:

1. a unit that is bound and keeps its exact binding and layout;
2. a unit with unchanged text;
3. a unit that was bound rather than detached;
4. the smallest ID.

Every other claimant is `Detached(BindingConflict)`.

## Truth matrix

| Sheet | Layout | Column | Row | Text | Permission | Outcome |
| --- | --- | --- | --- | --- | --- | --- |
| removed | any | any | any | any | any | `Detached(SheetRemoved)` |
| unreadable | any | any | any | any | any | `Detached(SheetUnavailable)` |
| present | any | any | keyed, key removed | any | any | `Detached(RowRemoved)` |
| present | same | not a String column | any | any | any | `Detached(CellRemoved)` |
| present | same | String | missing | any | any | `Detached(RowRemoved)` |
| present | same | String | present | equal | translatable | `Unchanged` |
| present | same | String | present | changed | translatable | `SourceChanged` |
| present | other, mapped | mapped column | present | as above | translatable | as above |
| present | other, unresolved | any | any | any | any | `Detached(ColumnUnresolved)` |
| present | any | resolved | present | any | not translatable | `Detached(NotTranslatable)` |

A binding conflict can then detach any bound outcome with `BindingConflict`.

## Applying outcomes

- `Unchanged`: keep ID, target, note, and review state; set status `bound`
  and replace the source facts with the proposed ones. A new row, column,
  layout, or key is still written.
- `SourceChanged`: as above, and set review state `needs-review`. The
  previous text stays visible in the Git history of the record.
- `Detached(reason)`: set the status to the reason; keep the source facts,
  target, note, and review state.

No outcome removes a unit or derives its ID again.

## Coordinate reuse and row shifts

For

```text
previous: A at X
new:      A at Y, unrelated B at X      (same layout)
```

the unit at X stays at X and is `SourceChanged` with B's text. It is never
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
- changed text, and re-encoded bytes with the same text, at a surviving
  binding;
- removed rows and sheets, and unreadable sheets, with last facts preserved;
- row shifts that stay at their binding and require review in unkeyed
  sheets;
- keyed rows that follow their line after an insertion, removed keyed lines
  whose row ID is reused, and units without keys that keep their rows and
  receive keys;
- column insertion mapped by content evidence, including a changed cell in a
  mapped column;
- layout changes without evidence, and split and colliding evidence, that
  stay unresolved;
- permission loss and later reattachment;
- binding conflicts and their deterministic winner;
- language mismatch and repeated IDs as errors.

`aeria-source` unit tests cover row key column detection and permission.
`aeria-workspace` integration tests cover applying updates, repeating an
update interrupted before the manifest was written, and a keyed dialogue sheet
whose translations follow their lines through a session.

## Model-based check

A bounded model runs in CI. It places three logical rows into four row slots
in every injective way (73 placements), applies all eight content-mutation
masks, and optionally inserts an unrelated duplicate: `73 * 8 * 2 = 1,168`
transitions in one layout. For every transition it checks that

1. a unit whose binding survives stays at that binding with same-row,
   same-column continuity;
2. its outcome is `Unchanged` or `SourceChanged` by text only;
3. a unit whose binding is missing is detached with no proposed facts;
4. every unit appears exactly once with its ID unchanged.

The model prints its state count and the number of bound, source-changed,
detached, and wrong mappings; wrong mappings must be zero.
