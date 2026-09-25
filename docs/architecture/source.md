# Source packages and HXS

HSP is Aeria's source handoff artifact. HXS is the immutable source contract
embedded inside it, and HSG is the deterministic translation-permission
allowlist derived from multilingual evidence.

The Rust core consumes a validated HSP and does not depend on how the package
was produced. Atlas process integration is a desktop workflow outside the
package-consumption boundary.

When creating a project from an installed game, Aeria launches the pinned
Harmonia Atlas v0.3.0 sidecar with its `package --events jsonl` command. The
JSONL stream is the process boundary: stdout is typed protocol data and stderr
is bounded diagnostics. Cancellation terminates and awaits the child, then
remains authoritative through package validation, workspace initialization, and
active-project publication. Atlas writes the generated HSP to the deterministic
app-data staging path `source-packages/staging/source.hsp`; Aeria validates that
staging artifact and checks its package ID against Atlas before atomically moving
it to the immutable `source-packages/<package-id-hex>.hsp` path. A validated
package is transferred into `ProjectSession` without reopening it. Existing
valid immutable collisions are reused, while invalid collisions are replaced
only after the new staging package is validated. On Windows Atlas runs without
a console window.

Beside each package it publishes, Aeria writes a local build record
`source-packages/<package-id-hex>.build.json` with the SHA-256 of the Atlas
executable, the source language, and every game version file
(`game/ffxivgame.ver` and each `game/sqpack/exN/exN.ver`). Before running
Atlas, Aeria looks for a record with exactly these inputs and, when the
package it names still verifies, uses that package instead of building again.
Game version alone is not enough: two Atlas releases can produce different
content from the same game version. Packages without a record, such as ones
published before records existed, are never reused for a build; opening a
project can still use them when their content ID matches the workspace.
Records are cache metadata and never enter project data.

Aeria keeps a package while it knows it needs it: a recent project or the
open project uses its package ID, or its build record matches the current
Atlas and game installation. Any other package is removable from Settings;
deleting it also deletes its build record and, when no remaining package has
the same snapshot, its materialized HXS. A project that is not in the recent
list and still used a deleted package is updated from the game the next time
it opens. When the recent-project registry cannot be read, nothing is
removable. Atlas is acquired only at
build time from this pinned release with a verified checksum; runtime downloads
are not used. Materialized HXS files remain disposable app-cache artifacts, and
the workspace format remains independent of game paths, Atlas paths, package
IDs, and cache paths.

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

The production HXS reader remains owned by `aeria-hxs`. It opens HXS v1 and v2
as an immutable SQLite artifact in read-only mode, validates the HXS SQLite identity
and required schema, runs SQLite integrity checks, and recomputes the
canonical row, sheet, `contentId`, and `snapshotId` hashes before exposing
source data. `aeria-hsp` owns package/HSG validation and consumes a dedicated
row-complete, String-only evidence stream. Its bounded keyset pages return
physical row/subrow groups, preserve rows with zero String cells, and select
only `row_id`, `subrow_id`, `column_index`, and exact macro text. Evidence
matching therefore requires no raw source bytes, technical payload, row hashes,
or per-cell `string_cell` lookups.

For deterministic source update planning, the reader also exposes bounded
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
presentation path is separate from the occurrence-level update/scanning APIs;
it does not change String-cell identity or any HXS hash contract.

The reader verifies rows as a stream and does not require the complete snapshot to be resident in memory. This is a source inspection interface only: it does not persist workspace state, edit translations, or export runtime data.

Within one desktop process, a successfully fully verified HSP and its
materialized HXS may be reopened through an in-memory immutable-byte cache.
The fast path rehashes both files, re-parses the manifest/guidance, and opens
only the HXS metadata/sheet catalog. The accepted verification is also
recorded as a rebuildable JSON record under
`<cache-root>/hsp-verification/`; the record contains its version,
canonical package path, HSP/HXS sizes and SHA-256 hashes, package/source
identities, cache path, and validated package metadata. On a later process
start, Aeria rehashes both artifacts, checks the canonical paths and
identities, validates the manifest-owned component descriptors, rechecks
archive membership and guidance relationships, and opens only the cached HXS
catalog. In particular, the required `sourceHxs` descriptor's size and
SHA-256 are authoritative for the cached HXS bytes; the persistent record's
copies must also match but cannot replace that manifest contract. Missing,
malformed, unsupported, tampered, or mismatched records are deleted and the
complete ZIP/component, SQLite integrity/schema, row/hash, and relationship
validation runs instead. This cache is disposable and does not change HSP/HXS
or Workspace Format contracts.
When a validated staging HSP is atomically published to its immutable
package-ID path, the relocation operation updates both the process-local cache
and the persistent record key and canonical `packagePath`, then removes the
staging record. A failure to write disposable cache state does not roll back
the published HSP; it removes the stale staging record so the next open uses
full validation.

Set `AERIA_PERF_TRACE=1` to emit both per-phase duration and cumulative
elapsed time for HSP identity/hash work, HSP archive/component verification,
HXS identity and full validation, guidance relationships,
persistent/in-memory cache paths, workspace loading and compatibility, row
paging, and unit persistence. This is an optional diagnostic trace, not a
runtime behavior switch for validation.

## HXS versions and excluded sheets

Aeria reads HXS v1 and v2. The required `sourceHxs` HSP component declares the
HXS version, and it must equal the embedded file's SQLite `user_version` and
`hxs_meta.format_version`; any other version is rejected.

HXS v2 lets Atlas keep producing a snapshot when individual sheets cannot be
read after a game patch. Every catalog sheet is either stored or listed in the
`excluded_sheets` table with a reason (unsupported variant, unsupported column
type, or unreadable data). Aeria verifies the v2 schema, the exclusion reason
codes, `hxs_meta.excluded_sheet_count`, that no sheet is both stored and
excluded, and the v2 `contentId`, which covers the stored sheets and the
exclusion list. Excluding a sheet therefore changes project compatibility like
any other content change. `HxsSnapshot::excluded_sheets` exposes the list; it
is always empty for v1. Row, sheet, and snapshot hashes are identical in both
versions.

A source update treats an excluded sheet as unavailable rather than removed:
its units are detached with `SheetUnavailable` and reattach when a later
source can read the sheet again (see [`rebase-safety.md`](./rebase-safety.md)).

## Translation permission and formatting-only strings

HSG remains the only translation-permission authority. Atlas grants an
occurrence when its source text is not empty and its exact macro text differs
in at least one evidence language. An empty source text is never translatable.
A sheet an evidence input could not read is reported incompatible with the
reason `unreadableInInput`; a sheet no input could read is omitted from HSG.

Some granted occurrences contain no letters outside protected structure:
punctuation such as `...`, digits such as a `0` placeholder, spacing, or only
number-formatting and icon macros. Their language variants differ by
localization convention (ellipsis form, thousands separator, full-width
punctuation) rather than prose. Aeria does not reinterpret HSG for them; the
read path marks them `formatting_only` so the editor can label and filter them.
The classification is `MacroString::is_formatting_only` in `aeria-se`: a well
formed document whose user-facing text ranges, including user-facing macro
arguments, contain no Unicode alphabetic character.

## Hash contract used by source updates

HXS hashes are canonical source facts, not relocation-invariant identity
keys. A workspace unit persists the String cell's `macroTextHash` and optional
`rawValueHash`, the coordinate-sensitive `rows.technical_hash`, and a
`SourceLayout` made of the sheet `schema_hash` and the column `offset`:

| HXS fact | Source update use |
| --- | --- |
| `string_cells.macro_hash` | Content classification, column-mapping votes, and row keys. |
| `string_cells.raw_hash` | Distinguishes an encoding-only change from unchanged content. |
| `rows.technical_hash` | Context status only; it includes the coordinate and never decides identity. |
| `sheets.schema_hash` | Identifies the schema generation a column index belongs to. |
| `columns.offset` | Confirms an unchanged String column position when content gives no evidence. |
| `content_id` | Project compatibility; the workspace records it. |
| `snapshot_id`, game version | Package identity only; not recorded by the workspace. |

The complete rules are in [`rebase-safety.md`](./rebase-safety.md).

## Source cache

Embedded HXS snapshots live outside the translation repository in a
machine-local disposable cache. The package reader uses:

```text
<cache-root>/hxs/<snapshot-id-without-sha256-prefix>/source.hxs
```

The canonical `sha256:` prefix is not used in the Windows filename. A failed
replacement is staged beside the destination and published only after the
component stream has been flushed and verified.

Source updates need only the new snapshot; the previous snapshot is used only for optional review suggestions for detached units. Cache cleanup can therefore never block an update. Retention of older snapshots is governed by a user-configurable cache budget.

## Degraded workspace

A project may be opened without its source snapshot. Git history, manifest, glossary, and persisted translations remain inspectable, but editing, reliable preview, AI translation, and export require a compatible verified source. A source update needs only a new verified source, which can always be built from the installed game.

## Additional source languages

Additional official source languages are local user preferences. They are context, not project identity, and are not required to be committed to the translation repository.
