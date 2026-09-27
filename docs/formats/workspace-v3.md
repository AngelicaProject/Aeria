# Workspace Format v3

Status: **current contract; written by `aeria-workspace`**.

Workspace Format v3 is a sparse, Git-tracked translation overlay over the
installed game. It stores project metadata and explicitly managed translation
units, never the complete source corpus or derived indexes.

Each unit carries the source text it was translated from, so a reviewer can
see the original next to the translation, a source update can tell whether
the string changed, and a changed string shows what it was before. Source
and target text are macro text as defined in
[`../architecture/strings.md`](../architecture/strings.md#macro-text).

Aeria reads no earlier format. Workspaces written in Workspace Format v1 or
v2 are reported as unsupported.

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

A unit's shard depends only on its ID, never on the sheet name, so file names
do not depend on names chosen by the game (a sheet named `Aux` or two names
that differ only by case would otherwise break a Windows checkout).

`.aeria/manifest.json` and `.aeria/units/*.jsonl` are canonical Aeria-managed
project state. They are Git-tracked text files so Git can diff and merge them
and history remains inspectable. They are not a supported hand-editing
interface: Aeria produces canonical bytes and validates loaded state.

## Manifest

`.aeria/manifest.json` is UTF-8 JSON formatted with two-space indentation, the
field order below, and one LF after the closing `}`:

```json
{
  "formatVersion": 3,
  "sourceLanguage": "en",
  "targetLanguage": "ru",
  "gameVersion": "2026.09.15.0000.0000"
}
```

| Field | Representation |
| --- | --- |
| `formatVersion` | the JSON integer `3` |
| `sourceLanguage` | `ja`, `en`, `de`, or `fr` |
| `targetLanguage` | a non-empty UTF-8 string |
| `gameVersion` | the game version the bound units describe, in the form described in [`../architecture/source.md`](../architecture/source.md#opening-a-source) |

All fields are required. The manifest contains no game paths, AI settings,
credentials, UI state, or other machine-local data.

## Unit shards

Every translation unit is serialized as one compact JSON object followed by
one LF. A shard is named with exactly two lowercase hexadecimal characters
and the `.jsonl` suffix: the first byte of the unit's ID. For example, an ID
beginning `7a` is stored in `.aeria/units/7a.jsonl`. Records in a shard are
sorted by ID in ascending bytewise order. A shard with no records is absent.

Each record has all fields below, in exactly this order:

```json
{"id":"<32 hex>","status":"bound","sheet":"Addon","row":12,"subrow":0,"column":0,"layout":"<16 hex>","source":"Retainer","key":null,"target":"Помощник","review":"draft","note":null}
```

| Field | Representation |
| --- | --- |
| `id` | 32 lowercase hexadecimal characters; see [`../architecture/identity.md`](../architecture/identity.md) |
| `status` | `bound`, or one detach reason from the table below |
| `sheet` | sheet name, a non-empty UTF-8 string |
| `row` | JSON integer in `0..=u32::MAX` |
| `subrow` | JSON integer in `0..=u16::MAX` |
| `column` | JSON integer in `0..=u32::MAX`, the column index |
| `layout` | 16 lowercase hexadecimal characters, the layout hash of the sheet at the binding |
| `source` | the source text at the binding, a non-empty UTF-8 string |
| `key` | the row key at the binding, or `null` when the sheet is not keyed |
| `target` | the translation, a UTF-8 string, including `""` |
| `review` | `draft`, `reviewed`, or `needs-review` |
| `note` | translator note, a UTF-8 string or `null` |

All fields are required, including nullable ones.

`sheet`, `row`, `subrow`, `column`, `layout`, `source`, and `key` are the
unit's *source facts*: where the unit was last bound and what the source said
there. They change only through a source update
([`../architecture/rebase.md`](../architecture/rebase.md)); ordinary edits
change only `target`, `review`, and `note`. A source update never changes
`id`.

`source` and `key` are texts printed by `aeria_se::codec::decode`, as
described in [`../architecture/source.md`](../architecture/source.md#string-text).
The row key is defined in
[`../architecture/source.md`](../architecture/source.md#row-keys).

### Status

| `status` | Meaning |
| --- | --- |
| `bound` | The source facts describe the game at `gameVersion`. |
| `sheet-removed` | The sheet no longer exists. |
| `sheet-unavailable` | The sheet exists in the game but cannot be read. |
| `row-removed` | The row or subrow no longer exists, or its row key was removed. |
| `cell-removed` | The column is no longer a String column. |
| `column-unresolved` | The sheet layout changed and the column could not be mapped. |
| `not-translatable` | The cell is no longer translatable. |
| `binding-conflict` | Another unit owns the resolved cell. |

Every value other than `bound` marks a *detached* unit. A detached unit keeps
the source facts it was last bound with, together with its target, review
state, and note. It owns no current cell and is not exported.

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
- A `formatVersion` other than `3` is reported as unsupported and never
  reinterpreted.
- The manifest and records must have every required field and no unknown or
  duplicate fields. Wrong JSON types, non-canonical ID or layout spellings,
  fractional numbers, out-of-range coordinates, empty `sheet` or `source`,
  an empty `key` string, and unknown `status`, `review`, or `sourceLanguage`
  values are invalid. `targetLanguage` must not be empty or whitespace-only,
  and `gameVersion` must be a valid game version.
- Every record is in the shard derived from its ID, and IDs are strictly
  increasing within a shard. Duplicate IDs are invalid.
- `target` must pass intrinsic `aeria-se` validation; opaque but preservable
  syntax is valid.
- Aeria never writes two bound units with the same `(sheet, row, subrow,
  column)`. Such state can still be read: it occurs when a Git merge combines
  units that different branches bound to one cell, and in the intermediate
  state of an interrupted source update. A workspace is editable only when
  every bound binding is unique and every bound unit describes the game at
  `gameVersion`; otherwise a source update first reconciles it.
- JSONL records may end with LF or CRLF, and the final record may omit its
  terminator. Blank or whitespace-only lines and bare CR are invalid.
- The stored ID is loaded as is and never derived again.

Reader acceptance of a BOM, CRLF, field order, or a missing final LF does not
make those bytes canonical.

## Compatibility

Future incompatible changes require a new format version and a lossless
migration. Game paths and caches are machine-local and never stored in the
workspace.
