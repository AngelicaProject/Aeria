# Source packages and HXS

HSP is Aeria's source handoff artifact. HXS is the immutable source contract
embedded inside it, and HSG is the deterministic translation-permission
allowlist derived from multilingual evidence.

The Rust core consumes a validated HSP and does not depend on how the package
was produced. Atlas process integration is a desktop workflow outside the
package-consumption boundary.

## Import verification

Before a package is accepted, Aeria verifies its ZIP structure, manifest,
component paths, sizes, hashes, logical package identity, embedded HXS,
embedded HSG, source evidence identity, and all HSP/HXS/HSG relationships.
An existing cache file is reused only after it is verified against the
manifest component size and SHA-256.

The package reader streams `source/source.hxs` directly from the archive while
hashing it. Every manifest-listed ZIP entry is checked against its declared
uncompressed size before decompression, and its actual decompressed byte count
and SHA-256 are bounded and verified while streaming. Manifest JSON is capped
at 1 MiB; HSG JSON is capped at 64 MiB; unknown optional components are
integrity-checked and discarded without a payload allocation. The source is
materialized only into a caller-provided disposable cache, then opened through
`HxsSnapshot::open`; rejected replacements remove their partial cache file.

The production HXS reader remains owned by `aeria-hxs`. It opens HXS v1 as an
immutable SQLite artifact in read-only mode, validates the HXS SQLite identity
and required schema, runs SQLite integrity checks, and recomputes the
canonical row, sheet, `contentId`, and `snapshotId` hashes before exposing
source data. `aeria-hsp` owns package/HSG validation and consumes a dedicated
row-complete, String-only evidence stream. Its bounded keyset pages return
physical row/subrow groups, preserve rows with zero String cells, and select
only `row_id`, `subrow_id`, `column_index`, and exact macro text. Evidence
matching therefore requires no raw source bytes, technical payload, row hashes,
or per-cell `string_cell` lookups.

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

Embedded HXS snapshots live outside the translation repository in a
machine-local disposable cache. The package reader uses:

```text
<cache-root>/hxs/<snapshot-id-without-sha256-prefix>/source.hxs
```

The canonical `sha256:` prefix is not used in the Windows filename. A failed
replacement is staged beside the destination and published only after the
component stream has been flushed and verified.

The current and immediately previous snapshots are protected because rebase requires them. Older snapshots are eligible for LRU cleanup under a user-configurable cache budget.

## Degraded workspace

A project may be opened without its source snapshot. Git history, manifest, glossary, and persisted translations remain inspectable, but editing, reliable preview, rebase, AI translation, and export require a compatible verified source.

## Additional source languages

Additional official source languages are local user preferences. They are context, not project identity, and are not required to be committed to the translation repository.
