# Source and HXS

HXS is Aeria's immutable source contract.

The Rust core consumes HXS and does not depend on how a snapshot was produced. The desktop source manager may invoke Harmonia Atlas to provide a one-click `Generate Source from FFXIV installation` experience.

## Import verification

Before a snapshot is accepted, Aeria should verify its supported format, schema/invariants, logical metadata, and hashes/identifiers. A previously verified file may be trusted through a local cache keyed by file identity/hash when safe to do so.

The first production source slice is implemented by `aeria-hxs`. It opens HXS v1 as an immutable SQLite artifact in read-only mode, validates the HXS SQLite identity and required schema, runs SQLite integrity checks, and recomputes the canonical row, sheet, `contentId`, and `snapshotId` hashes before exposing any source data. Its public interface returns owned source DTOs for metadata, sheet schemas/hashes, bounded row pages, row technical payloads, and String-cell macro/raw representations; SQLite types remain private to the crate. A single row page is capped at the crate-level `MAX_ROW_PAGE_SIZE` of 4096 rows.

For deterministic source rebase indexing, the reader also exposes bounded
keyset pages for one verified sheet at a time. Each page is constrained by
that sheet's ID and advances by the existing String-cell primary-key
coordinate `(row_id, subrow_id, column_index)`, returning only verified
macro/raw/row-technical hashes without loading the corresponding source
values. Callers enumerate sheet names canonically to obtain snapshot order;
the reader does not require a new cross-sheet index or a global temporary
sort.

For desktop source browsing, `aeria-hxs` also exposes a bounded
`page_string_rows` path. It selects row/subrow groups with a keyset cursor and
joins all String cells in those groups in one bounded query, returning macro
text plus verified hashes but never `raw_value` bytes. This row-oriented
presentation path is separate from the occurrence-level rebase/scanning APIs;
it does not change String-cell identity or any HXS hash contract.

The reader verifies rows as a stream and does not require the complete snapshot to be resident in memory. This is a source inspection interface only: it does not persist workspace state, edit translations, or export runtime data.

## Hash contract used by rebase

HXS v1 hashes are canonical source facts, not relocation-invariant identity
keys. In particular, `rows.technical_hash` includes `sheet_name`, `row_id`,
and `subrow_id` as well as the non-String technical payload. The persisted
rebase `SourceFingerprint` keeps the String-cell macro hash, optional raw-value
hash, and this coordinate-sensitive row technical hash without changing the
workspace format. Rebase identity is established by the surviving
`SourceBinding`; `macroTextHash` and `rawValueHash` classify the String content
as unchanged or changed, while `rowTechnicalHash` is reported as separate
context status. A complete fingerprint must not be used to infer a moved
occurrence at another binding. The complete rebase transition contract is documented in
[`rebase-safety.md`](./rebase-safety.md).

## Source cache

Snapshots live in a machine-local content-addressed source store, not in the translation repository.

The current and immediately previous snapshots are protected because rebase requires them. Older snapshots are eligible for LRU cleanup under a user-configurable cache budget.

## Degraded workspace

A project may be opened without its source snapshot. Git history, manifest, glossary, and persisted translations remain inspectable, but editing, reliable preview, rebase, AI translation, and export require a compatible verified source.

## Additional source languages

Additional official source languages are local user preferences. They are context, not project identity, and are not required to be committed to the translation repository.
