# Bounded translation reads

`ProjectSession::page_translation_rows` is the application-level read API over
verified HXS source and sparse Workspace state. Rust performs the complete
source/overlay composition so the desktop renderer does not separately join
HXS and Workspace data.

```text
ProjectSession
    ↓
bounded HXS String-row page
    ↓
classify String cells
    ↓
sparse Workspace lookup by SourceBinding
    ↓
TranslationRowView page
```

## Row and cell ownership

A source row is the logical browsing and editing group. A String cell remains
the durable translation identity and persistence unit. Each translatable cell
retains its own `SourceBinding`, source fingerprint, sparse overlay, target,
review state, translator note, and stable `TranslationUnitId`.

Grouping does not change identity. For example, four String cells in one
`Item` row produce one `TranslationRowView` with four independent
`TranslationCellView` values. Mutations still address one `SourceBinding` at a
time.

The current classification is deliberately narrow because EXDSchema is not
yet integrated:

- empty macro text is ignored;
- non-empty macro text beginning with the exact uppercase `TEXT_` prefix is
  read-only context;
- every other non-empty String cell is translatable.

Context cells are returned separately from editable cells. Rows with no
translatable cells are omitted from the visible result. Existing Workspace
units bound to a newly classified context cell are not deleted or repaired by
this read path.

This is not the future semantic-schema system. EXDSchema integration is
deferred; labels remain `Column N`. Later schema metadata may refine roles and
display labels without changing String-cell identity or TranslationUnit IDs.

## HXS source page

`aeria-hxs::HxsSnapshot::page_string_rows` is an additional source-browsing
API. The existing `page_string_occurrences` and
`page_string_occurrence_records` APIs remain available for rebase and scanning
contracts.

The row reader uses one bounded SQL query per page. A CTE first selects at most
`limit + 1` distinct `(row_id, subrow_id)` coordinates containing String cells,
using an exclusive keyset predicate and ordering by row/subrow. The query then
joins those coordinates to `string_cells` and `rows`, returning
`row_id`, `subrow_id`, `column_index`, macro text, macro hash, optional raw
hash, and row technical hash. It does not select `raw_value` and does not call
`page_rows` or `string_cell` repeatedly.

The joined result is ordered by row ID, subrow ID, and column index. The extra
row group is removed after the query; it is never split between pages. The
cursor points to the final returned row only when another row group exists.

## Application paging and overlay composition

`MAX_TRANSLATION_PAGE_SIZE` remains 256. The desktop requests 100, now meaning
up to 100 HXS row/subrow groups scanned, not 100 String cells. Classification
can therefore make a visible page shorter, including zero visible rows with a
non-null `next_after`. The reader does not loop to fill a visible page.

The application cursor is an owned `TranslationRowCursor` containing only
`sheet_name`, `row_id`, and `subrow_id`. Cross-sheet cursors are rejected.

For each translatable cell, Rust constructs the existing cell-level
`SourceBinding`, reconstructs the verified `SourceFingerprint` from macro hash,
optional raw hash, and row technical hash, then looks up the exact sparse
Workspace binding. A present overlay is returned even when `target_macro` is
empty; a missing overlay means untranslated. A stale fingerprint is an
integrity error. Reads never repair or mutate Workspace state.

The desktop maps this page to owned row DTOs and keeps page-size bounds and
integrity failures intact. Mutation commands remain cell-level:
`set_translation_target`, `set_translation_note`, and
`set_translation_review_state`.
