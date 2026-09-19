# Bounded translation reads

`ProjectSession::page_translation_entries` is the first application-level
read API over source and workspace state. It keeps authoritative composition
in Rust so future UI code does not separately join HXS and Workspace data.

```text
ProjectSession
    ↓
bounded per-sheet HXS String occurrence page
    ↓
sparse Workspace lookup by SourceBinding
    ↓
TranslationEntryView page
```

One HXS String cell is one browsable source entry. An entry without Workspace
state has no `TranslationUnitId` yet. That absence is distinct from a real
Workspace unit whose `target_macro` is explicitly empty. Source macro text
comes from the verified HXS record page; target macro, review state, and
translator note come from the sparse Workspace overlay.

The HXS reader page is bounded, per-sheet, keyset-paginated, and ordered by
row ID, subrow ID, then column index. The application bound is
`MAX_TRANSLATION_PAGE_SIZE` (currently 256), and the cursor is exclusive.
When an existing Workspace unit's persisted source fingerprint differs from
the verified HXS occurrence at the same binding, the read returns an
integrity error. It does not repair, rebase, or reinterpret the unit.

The API is read-only. It does not write Workspace Format files, HXS data,
review state, targets, notes, or TranslationUnit IDs. Search, filtering,
indexes, caches, mutations, source update/rebase, export, and UI remain
separate future layers. No UI owns authoritative source/workspace
composition.
