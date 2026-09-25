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
apply the verified HSG GuidanceIndex
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

HSG is the only editability authority. The `GuidanceIndex` stores compatible
sheet allowlists as sorted `(row_id, subrow_id, column_index)` vectors and
uses binary search for exact occurrence lookup. It contains no source text or
text classifier.

Context cells are returned separately from editable cells. Rows with no
guidance-allowed cells are omitted from the visible result. HSG never grants an
empty source text. A blocked non-empty String is returned as read-only
context, while a blocked empty String may be omitted.

Each editable cell carries `formatting_only`, derived from its source macro by
`aeria_se::MacroString::is_formatting_only`: the source has no letters outside
protected structure, for example `...`, `0`, or a number-formatting macro. It
is a presentation hint only; it never changes permission, identity, or
validation (see [`source.md`](./source.md)).
A bound Workspace unit that guidance no longer permits requires a source
update before the session opens, which detaches it as `NotTranslatable` (see
[`rebase.md`](./rebase.md)); the read path never repairs or hides bound units.

This is not the future semantic-schema system. EXDSchema integration is
deferred; labels remain `Column N`. Later schema metadata may refine roles and
display labels without changing String-cell identity or TranslationUnit IDs.

## HXS source page

`aeria-hxs::HxsSnapshot::page_string_rows` is an additional source-browsing
API. The existing `page_string_occurrences` and
`page_string_occurrence_records` APIs remain available for source updates and scanning
contracts.

The row reader uses one bounded SQL query per page. A CTE first selects at most
`limit + 1` distinct `(row_id, subrow_id)` coordinates containing String cells,
using an exclusive keyset predicate and ordering by row/subrow. The query then
joins those coordinates to `string_cells` and `rows` with the page groups
pinned as the outer loop (`CROSS JOIN`), so each page reads only its own cells;
a join driven from `string_cells` by sheet alone would rescan the whole sheet
for every page. It returns
`row_id`, `subrow_id`, `column_index`, macro text, macro hash, optional raw
hash, and row technical hash. It does not select `raw_value` and does not call
`page_rows` or `string_cell` repeatedly.

The joined result is ordered by row ID, subrow ID, and column index. The extra
row group is removed after the query; it is never split between pages. The
cursor points to the final returned row only when another row group exists.

## Application paging and overlay composition

`MAX_TRANSLATION_PAGE_SIZE` remains 256, meaning up to 256 HXS row/subrow
groups scanned, not 256 String cells. Classification can therefore make a
visible page shorter, including zero visible rows with a non-null `next_after`.
The reader does not loop to fill a visible page. The desktop requests full
256-row pages and follows `next_after` until the sheet is complete, so paging
is a bounded transport detail rather than something the user navigates.

The application cursor is an owned `TranslationRowCursor` containing only
`sheet_name`, `row_id`, and `subrow_id`. Cross-sheet cursors are rejected.

For each translatable cell, Rust constructs the existing cell-level
`SourceBinding`, reconstructs the verified `SourceFingerprint` from macro hash,
optional raw hash, and row technical hash, derives the `SourceLayout` from the
sheet schema hash and column offset, then looks up the bound unit at that
binding. Detached units are never overlaid. A present overlay is returned even
when `target_macro` is empty; a missing overlay means untranslated. A stale
fingerprint or layout is an integrity error. Reads never repair or mutate
Workspace state.

The desktop maps this page to owned row DTOs and keeps page-size bounds and
integrity failures intact. Mutation commands remain cell-level:
`set_translation_target`, `set_translation_note`, and
`set_translation_review_state`.

## Translation progress

`ProjectSession::translation_progress` summarizes in-memory Workspace units per
sheet: `translated` counts units (including explicitly empty targets), and
`reviewed` and `needs_review` are subsets of it. Only bound units whose
binding the HSG index permits are counted, so every count is bounded by the
sheet's translatable cell count; detached units are not counted. Sheets
without units are omitted, and results are ordered by sheet name.

The summary reads no HXS rows and does not re-verify fingerprints; a stale unit
still surfaces as an integrity error when its row is paged. It is presentation
data for progress indicators, never an input to identity, source update,
merge, or export decisions.
