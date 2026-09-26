# Bounded translation reads

`ProjectSession::page_translation_rows` is the application-level read API over
the game and sparse Workspace state. Rust performs the complete
source/overlay composition so the desktop renderer does not separately join
game and Workspace data.

```text
ProjectSession
    ↓
bounded page of one game sheet's rows
    ↓
translatable cells and read-only context
    ↓
sparse Workspace lookup by SourceBinding
    ↓
TranslationRowView page
```

## Row and cell ownership

A source row is the logical browsing and editing group. A String cell remains
the durable translation identity and persistence unit. Each translatable cell
retains its own `SourceBinding`, source facts, sparse overlay, target, review
state, translator note, and stable `TranslationUnitId`.

Grouping does not change identity. For example, four String cells in one
`Item` row produce one `TranslationRowView` with four independent
`TranslationCellView` values. Mutations still address one `SourceBinding` at a
time.

Translation permission (see [`source.md`](./source.md#translation-permission))
is the only editability authority. Translatable cells are editable. A
non-empty cell that is not translatable is returned as read-only context; an
empty one is omitted. Rows with no translatable cells are omitted from the
visible result.

Each editable cell carries `formatting_only`, derived from its source macro by
`aeria_se::MacroString::is_formatting_only`: the source has no letters outside
protected structure, for example `...`, `0`, or a number-formatting macro. It
is a presentation hint only; it never changes permission, identity, or
validation. A bound unit whose cell is no longer translatable requires a
source update before the session opens, which detaches it as
`NotTranslatable` (see [`rebase.md`](./rebase.md)); the read path never
repairs or hides bound units.

This is not a semantic-schema system. Labels remain `Column N`. Later schema
metadata may refine roles and display labels without changing String-cell
identity or translation-unit IDs.

## Paging and overlay composition

`MAX_TRANSLATION_PAGE_SIZE` is 256, meaning up to 256 row/subrow groups
scanned, not 256 String cells. Classification can therefore make a visible
page shorter, including zero visible rows with a non-null `next_after`. The
reader does not loop to fill a visible page. The desktop requests full
256-row pages and follows `next_after` until the sheet is complete, so paging
is a bounded transport detail rather than something the user navigates. A
sheet that is missing or cannot be read has no rows.

The cursor is an owned `TranslationRowCursor` containing only `sheet_name`,
`row_id`, and `subrow_id`; it is exclusive and ordered by row ID then subrow
ID. Cross-sheet cursors are rejected.

For each translatable cell with a bound unit, Rust compares the unit's source
facts with the facts of the game's cell (`SourceSheet::facts`). Detached units
are never overlaid. A present overlay is returned even when `target_macro` is
empty; a missing overlay means untranslated. A unit whose facts differ is an
integrity error. Reads never repair or mutate Workspace state.

The desktop maps this page to owned row DTOs and keeps page-size bounds and
integrity failures intact. Mutation commands remain cell-level:
`set_translation_target`, `set_translation_note`, and
`set_translation_review_state`.

## Translation progress

`ProjectSession::translation_progress` summarizes in-memory Workspace units per
sheet: `translated` counts bound units (including explicitly empty targets),
and `reviewed` and `needs_review` are subsets of it. Detached units are not
counted; bound units were verified against the game when the project opened.
Sheets without units are omitted, and results are ordered by sheet name.

The summary reads no game data. It is presentation data for progress
indicators, never an input to identity, source update, merge, or export
decisions. The total per sheet comes from the sheet catalog (see
[`desktop-application-boundary.md`](./desktop-application-boundary.md)).
