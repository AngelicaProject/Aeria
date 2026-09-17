# Workspace Format v1

Status: **frozen contract; production persistence is not implemented yet**.

Workspace Format v1 is a sparse, Git-tracked translation overlay. It stores
project metadata and explicitly managed translation units, never the complete
source corpus or the derived in-memory source-binding index.

The supporting layout evaluation is documented in
[`workspace-v1-evaluation.md`](./workspace-v1-evaluation.md). It compared
one-file, single-JSONL, sharded-JSONL, and sharded pretty-JSON layouts with
real Git diff and three-way merge experiments at representative scales.

## Repository layout

The canonical layout is:

```text
manifest.json
units/
  00.jsonl
  ...
  ff.jsonl
```

Only non-empty shard files are present. The directory contains at most 256
unit files. The manifest and unit files are the complete v1 canonical project
state for the facts defined below.

## Manifest

`manifest.json` is UTF-8 JSON formatted with two-space indentation, the field
order below, and one LF after the closing `}`:

```json
{
  "formatVersion": 1,
  "sourceLanguage": "en",
  "targetLanguage": "fr",
  "contentId": "sha256:<64 lowercase hexadecimal characters>",
  "snapshotId": "sha256:<64 lowercase hexadecimal characters>"
}
```

All five fields are required. `formatVersion` is the JSON integer `1`, not a
string. Language values are UTF-8 strings. `contentId` and `snapshotId` are
the current verified HXS identifiers, preserved in their canonical
`sha256:<64 lowercase hexadecimal characters>` form.

The manifest does not contain local HXS paths, AI provider or model settings,
credentials, UI state, local cache/index paths, or other machine-local data.

## Unit shards

Every translation unit is serialized as one compact JSON object followed by
one LF. A shard is named with exactly two lowercase hexadecimal characters
and the `.jsonl` suffix. The name is the first byte of the raw 32-byte
`TranslationUnitId` digest, equivalently the first two hexadecimal characters
after `tu1:` in the canonical textual ID. For example, an ID beginning
`tu1:7a...` is stored in `units/7a.jsonl`.

Records in every shard are sorted by the complete canonical textual
`TranslationUnitId` in ascending bytewise lexicographic order. This is also
the order of the raw digest for v1 IDs. A shard with no records is absent.

Each record has all fields below, in exactly this order:

```json
{"id":"tu1:<64 lowercase hexadecimal characters>","sourceBinding":{"sheetName":"...","rowId":0,"subrowId":0,"columnIndex":0},"sourceFingerprint":{"macroTextHash":"<64 lowercase hexadecimal characters>","rawValueHash":null,"rowTechnicalHash":"<64 lowercase hexadecimal characters>"},"targetMacro":"...","reviewState":"draft","translatorNote":null}
```

The fields are:

| Field | Representation | Required |
| --- | --- | --- |
| `id` | `tu1:<64 lowercase hexadecimal characters>` | yes |
| `sourceBinding.sheetName` | UTF-8 string | yes |
| `sourceBinding.rowId` | non-negative JSON integer | yes |
| `sourceBinding.subrowId` | non-negative JSON integer | yes |
| `sourceBinding.columnIndex` | non-negative JSON integer | yes |
| `sourceFingerprint.macroTextHash` | 64 lowercase hexadecimal characters, without a prefix | yes |
| `sourceFingerprint.rawValueHash` | the same hash string or JSON `null` | yes |
| `sourceFingerprint.rowTechnicalHash` | 64 lowercase hexadecimal characters, without a prefix | yes |
| `targetMacro` | UTF-8 string, including `""` for an explicit empty target | yes |
| `reviewState` | one of `draft`, `reviewed`, `needs-review` | yes |
| `translatorNote` | UTF-8 string or JSON `null` | yes |

The `id`, binding, fingerprint, target, review state, and note represent
exactly the corresponding `aeria-core` domain facts. Full source macro text is
not stored. A missing unit and a unit with an empty `targetMacro` remain
distinct.

`TranslationUnitId` derivation and source-update identity rules are defined
by the finalized identity contract in
[`../architecture/identity.md`](../architecture/identity.md). A source
binding or fingerprint update retains `id`; it never recomputes it.

## Canonical JSON and newline rules

- Every file is UTF-8 with LF (`U+000A`) line endings, including the final LF.
- JSONL has no insignificant whitespace outside JSON strings. There is one
  complete logical record per physical line.
- A JSON string escapes `"`, `\\`, and the JSON control escapes `\b`, `\f`,
  `\n`, `\r`, and `\t`. Other code points from `U+0000` through `U+001F`
  use lowercase `\u00xx` escapes. Solidus and non-ASCII Unicode characters
  are not escaped.
- JSON numbers are decimal integers without quotes or leading zeroes.
- Hashes and IDs use lowercase hexadecimal. No alternate casing or hash
  prefixes are permitted in unit records.
- Field order is semantic canonical order, not an implementation detail.
- An unchanged logical workspace must serialize byte-for-byte identically.

The v1 contract does not include comments, duplicate JSON keys, omitted
nullable fields, literal record-breaking newlines inside JSONL, or alternate
pretty-printing of unit records.

## Domain scope and compatibility

The workspace is sparse and has exactly one canonical target language. The
persisted project-level facts are the format version, source language, target
language, current HXS `contentId`, and current HXS `snapshotId`. The persisted
unit-level facts are the stable ID, current source binding, source fingerprint,
target macro string, review state, and optional translator note.

The secondary `SourceBinding` index is derived in memory and is not persisted.
Local source paths, credentials, provider choices, UI state, caches, and the
full HXS source corpus are outside the format.

Future incompatible changes require a new format version or an explicit,
lossless migration. The next implementation change may add a reader/writer
for this frozen representation, but must not make new layout or syntax
decisions.
