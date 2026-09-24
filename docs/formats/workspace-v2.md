# Workspace Format v2

Status: **current contract; written by `aeria-workspace`**.

Workspace Format v2 is a sparse, Git-tracked translation overlay. It stores
project metadata and explicitly managed translation units, never the complete
source corpus or the derived in-memory source-binding index.

Compared with [Workspace Format v1](./workspace-v1.md), v2 identifies the
source by content only, records the source layout each binding was resolved
in, and can keep a translation that has no current source occurrence. These
additions let a source update complete for any game change without losing a
translation; see [`../architecture/rebase.md`](../architecture/rebase.md).
The repository layout and canonical encoding are unchanged from v1 and remain
supported by the layout evaluation in
[`workspace-v1-evaluation.md`](./workspace-v1-evaluation.md).

## Repository layout

The repository root is the project container. It may contain arbitrary
project-owned files, including documentation, collaboration configuration,
glossaries, translation guidance, and future project-shared Aeria data. The
reader must not reject unrelated repository-root content.

The reserved data root is `.aeria/`. Its canonical layout is:

```text
.aeria/
  manifest.json
  units/
    00.jsonl
    ...
    ff.jsonl
```

`.aeria/manifest.json` is required. `.aeria/units/` is present only when there
are translation units to store; an absent directory is the empty sparse
overlay. When present, it contains at most 256 non-empty shard files. The
canonical writer does not create placeholder files such as `.gitkeep`.

## Aeria-managed project state

`.aeria/manifest.json` and `.aeria/units/*.jsonl` are canonical Aeria-managed
project state. They are Git-tracked text files so Git can diff and merge them,
project history remains inspectable, and recovery and external auditing
remain possible. They are not a supported hand-editing interface. Aeria
produces canonical bytes and validates loaded state; validation is not
weakened to accommodate arbitrary manual modifications.

## Manifest

`.aeria/manifest.json` is UTF-8 JSON formatted with two-space indentation, the
field order below, and one LF after the closing `}`:

```json
{
  "formatVersion": 2,
  "sourceLanguage": "en",
  "targetLanguage": "fr",
  "contentId": "sha256:<64 lowercase hexadecimal characters>"
}
```

All four fields are required. `formatVersion` is the JSON integer `2`.
Language values are UTF-8 strings. `contentId` is the verified HXS content ID
the bound units describe, in its canonical form.

The manifest records no `snapshotId` or game version. Game versions whose
extracted content is identical share a content ID, so a hotfix without source
changes does not change the project. The manifest also contains no local HXS
paths, AI settings, credentials, UI state, or other machine-local data.

## Unit shards

Every translation unit is serialized as one compact JSON object followed by
one LF. A shard is named with exactly two lowercase hexadecimal characters
and the `.jsonl` suffix: the first byte of the raw 32-byte
`TranslationUnitId` digest. For example, an ID beginning `tu1:7a...` is stored
in `.aeria/units/7a.jsonl`. Records in a shard are sorted by the canonical
textual ID in ascending bytewise order. A shard with no records is absent.

Each record has all fields below, in exactly this order:

```json
{"id":"tu1:<hex>","sourceStatus":"bound","sourceBinding":{"sheetName":"...","rowId":0,"subrowId":0,"columnIndex":0},"sourceFingerprint":{"macroTextHash":"<hex>","rawValueHash":null,"rowTechnicalHash":"<hex>"},"sourceLayout":{"sheetSchemaHash":"<hex>","columnOffset":0},"sourceRowKey":null,"targetMacro":"...","reviewState":"draft","translatorNote":null}
```

| Field | Representation |
| --- | --- |
| `id` | `tu1:<64 lowercase hexadecimal characters>` |
| `sourceStatus` | `bound`, or one detach reason from the table below |
| `sourceBinding.sheetName` | UTF-8 string |
| `sourceBinding.rowId` | JSON integer in `0..=u32::MAX` |
| `sourceBinding.subrowId` | JSON integer in `0..=u16::MAX` |
| `sourceBinding.columnIndex` | JSON integer in `0..=u32::MAX` |
| `sourceFingerprint.macroTextHash` | 64 lowercase hexadecimal characters |
| `sourceFingerprint.rawValueHash` | the same hash form, or `null` |
| `sourceFingerprint.rowTechnicalHash` | 64 lowercase hexadecimal characters |
| `sourceLayout` | object, or `null` only for a detached unit (see below) |
| `sourceLayout.sheetSchemaHash` | 64 lowercase hexadecimal characters |
| `sourceLayout.columnOffset` | JSON integer in `0..=u32::MAX` |
| `sourceRowKey` | 64 lowercase hexadecimal characters, or `null` |
| `targetMacro` | UTF-8 string, including `""` |
| `reviewState` | `draft`, `reviewed`, or `needs-review` |
| `translatorNote` | UTF-8 string or `null` |

All fields are required, including nullable ones. Hashes carry no prefix.

### Source status

| `sourceStatus` | Meaning |
| --- | --- |
| `bound` | The binding, fingerprint, and layout describe the current source. |
| `sheet-removed` | The sheet no longer exists. |
| `sheet-unavailable` | The sheet exists in the game but the source could not read or represent it. |
| `row-removed` | The row/subrow no longer exists in its sheet. |
| `cell-removed` | The column is no longer a String column. |
| `column-unresolved` | The sheet schema changed and the column could not be mapped. |
| `not-translatable` | Source guidance no longer permits the occurrence. |
| `binding-conflict` | Another unit owns the resolved occurrence. |

Every value other than `bound` marks a *detached* unit. A detached unit keeps
the binding, fingerprint, layout, and row key it was last bound with,
together with its target, review state, and note. It owns no current occurrence and is not
exported.

### Source layout

`sheetSchemaHash` is the verified HXS schema hash of the unit's sheet at its
binding, and `columnOffset` is the HXS offset of the bound String column in
that schema. Together they state which column layout `columnIndex` refers to.

`sourceLayout` is `null` only for a detached unit that was migrated from
Workspace Format v1 without ever being bound in v2, because v1 did not record
the layout. A bound unit with a `null` layout is invalid.

### Source row key

`sourceRowKey` is the macro-text hash of the row key at the unit's binding
when the sheet has a row key column, as defined in
[`rebase-safety.md`](../architecture/rebase-safety.md#row-keys), and `null`
otherwise. It lets a source update follow a line whose row ID changed. It is
a source fact like the fingerprint: it is set when the unit is created and by
every source update, and it never participates in the `TranslationUnitId`.

The `id`, source facts, target, review state, and note represent exactly the
corresponding `aeria-core` domain facts. Full source macro text is not stored.
A missing unit and a unit with an empty `targetMacro` remain distinct.

`TranslationUnitId` derivation is defined by
[`../architecture/identity.md`](../architecture/identity.md). A source update
rewrites status, binding, fingerprint, layout, and row key in place; it never
recomputes `id`.

## Canonical JSON and newline rules

- Every `.aeria/` data file is UTF-8 with LF line endings, including the
  final LF.
- JSONL has no insignificant whitespace outside JSON strings and one complete
  record per physical line.
- A JSON string escapes `"`, `\\`, and the JSON control escapes `\b`, `\f`,
  `\n`, `\r`, and `\t`. Other code points from `U+0000` through `U+001F` use
  lowercase `\u00xx` escapes. Solidus and non-ASCII characters are not
  escaped.
- JSON numbers are decimal integers without quotes or leading zeroes.
- Hashes and IDs use lowercase hexadecimal.
- Field order is semantic canonical order.
- An unchanged logical workspace serializes byte-for-byte identically.

Comments, duplicate keys, omitted nullable fields, record-breaking newlines,
and alternate pretty-printing are not part of the format.

## Reader and validation contract

The canonical rules above define writer output. A reader may accept the
harmless variations listed here and normalizes them in memory. Any other
violation fails the whole workspace; the reader never skips invalid data or
returns a partially validated workspace.

- One UTF-8 BOM at the start of the manifest or a shard may be stripped. A
  BOM anywhere else is invalid.
- Inside `.aeria/`, only `manifest.json` and the optional `units/` directory
  are defined. `units/` may contain only files named `[0-9a-f]{2}.jsonl`.
  Empty shard files, nested directories, other entries, and symlinks are
  invalid. Unrelated repository-root content is ignored.
- The manifest's `formatVersion` selects the contract. Version `2` requires
  the v2 manifest and v2 records. Version `1` is read under the
  [v1 contract](./workspace-v1.md) for migration only. Other versions are
  reported as unsupported and never reinterpreted.
- The manifest and records must have every required field and no unknown or
  duplicate fields. Wrong JSON types, non-canonical ID/hash spellings,
  fractional numbers, out-of-range coordinates, unknown `sourceStatus` or
  `reviewState` values, a malformed `sourceRowKey`, and a `null` layout on a
  bound unit are invalid.
  Languages must not be empty or whitespace-only. `contentId` must be a
  canonical `sha256:` HXS ID.
- Every record is in the shard derived from its ID, and IDs are strictly
  increasing within a shard. Duplicate IDs are invalid.
- Aeria never writes two bound units with the same `sourceBinding`.
  Detached units do not take part. Such state can still be read: it occurs
  in the intermediate state of an interrupted source update, and when a Git
  merge combines units that different branches bound to one occurrence.
- A workspace is editable only when every bound unit's binding is unique and
  every bound unit's source facts describe the source recorded by
  `contentId`; otherwise it must first be reconciled by a source update,
  which keeps one deterministic owner per binding and detaches the others.
- `targetMacro` must pass intrinsic `aeria-se` validation; opaque but
  preservable syntax is valid.
- JSONL records may end with LF or CRLF, and the final record may omit its
  terminator. Blank or whitespace-only lines and bare CR are invalid.
- The stored `TranslationUnitId` is loaded as is and never re-derived.

Reader acceptance of a BOM, CRLF, field order, or a missing final LF does not
make those bytes canonical.

## History and merge inputs

Git history and merge stages may predate v2. The public helpers
`decode_unit_shard` and `decode_unit_record` therefore accept each record in
either the v2 or the v1 shape; a v1 record decodes as a bound unit without a
layout. `encode_unit_shard` writes v2 only and rejects a bound unit without a
layout.

## Migration from v1

A v1 workspace is never activated for editing. Opening it with any source
requires a source update, which rewrites every shard in the v2 shape and then
writes the v2 manifest. Units receive their layout from the new source; see
[`../architecture/rebase-safety.md`](../architecture/rebase-safety.md#schema-generations).
The migration is lossless: no unit, target, note, review state, or ID is
removed or changed by the format change itself.

## Compatibility

Future incompatible changes require a new format version and a lossless
migration. HSP paths, materialized HXS cache paths, and HSP package IDs are
runtime source configuration and are not stored in the workspace.
