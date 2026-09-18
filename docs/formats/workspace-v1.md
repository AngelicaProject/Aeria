# Workspace Format v1

Status: **frozen contract; production persistence is implemented in
`aeria-workspace`**.

Workspace Format v1 is a sparse, Git-tracked translation overlay. It stores
project metadata and explicitly managed translation units, never the complete
source corpus or the derived in-memory source-binding index.

The supporting layout evaluation is documented in
[`workspace-v1-evaluation.md`](./workspace-v1-evaluation.md). It compared
one-file, single-JSONL, sharded-JSONL, and sharded pretty-JSON layouts with
real Git diff and three-way merge experiments at representative scales.

## Repository layout

The repository root is the project container. It may contain arbitrary
project-owned files, including documentation, collaboration configuration,
glossaries, translation guidance, and future project-shared Aeria data. The
Workspace Format v1 reader must not reject unrelated repository-root content.

The reserved Workspace Format v1 data root is `.aeria/`. Its canonical layout
is:

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
manifest and unit files under `.aeria/` are the complete v1 canonical project
state for the facts defined below. The canonical writer does not create
placeholder files such as `.gitkeep` merely to preserve an empty directory.

Validation is strict inside the reserved `.aeria/` Workspace Format area. The
reader does not validate or reject unrelated entries in the repository root.

## Aeria-managed project state

`.aeria/manifest.json` and `.aeria/units/*.jsonl` are canonical Aeria-managed
project state. They are Git-tracked text files so Git can diff and merge them,
project history remains inspectable, and recovery and external auditing remain
possible. They are not a supported hand-editing interface or a normal manual
project workflow. Aeria is responsible for producing canonical bytes and for
validating loaded state; validation is not weakened to accommodate arbitrary
manual modifications. The files remain ordinary UTF-8 text rather than opaque
or binary data, but the persisted contract is machine-managed.

## Manifest

`.aeria/manifest.json` is UTF-8 JSON formatted with two-space indentation, the
field order below, and one LF after the closing `}`:

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
`tu1:7a...` is stored in `.aeria/units/7a.jsonl`.

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
| `sourceBinding.rowId` | JSON integer in `0..=u32::MAX` | yes |
| `sourceBinding.subrowId` | JSON integer in `0..=u16::MAX` | yes |
| `sourceBinding.columnIndex` | JSON integer in `0..=u32::MAX` | yes |
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

- Every `.aeria/` data file is UTF-8 with LF (`U+000A`) line endings, including
  the final LF.
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

## Reader and validation contract

The canonical JSON and newline rules above define writer output. They are
separate from semantic reader acceptance: a reader may accept harmless input
variations listed here and must normalize them in memory. A reader must fail
the whole workspace with a validation error; it must not silently skip invalid
data or return a partially validated workspace.

- The reader decodes UTF-8. It may accept exactly one UTF-8 BOM at the start
  of `.aeria/manifest.json` or a unit shard and strips it before JSON parsing.
  A BOM anywhere else is invalid. The canonical writer never emits a BOM.
- The reader opens the repository root as a project container and requires a
  `.aeria/` directory containing `manifest.json`. Unrelated repository-root
  files and directories are ignored by Workspace Format validation. Inside the
  reserved `.aeria/` area, only `manifest.json` and the optional `units/`
  directory are defined by v1; any other entry is invalid. A missing `units/`
  directory means an empty sparse overlay. If it is present, it may contain
  only files named exactly `[0-9a-f]{2}.jsonl`; nested directories, other
  extensions, and unexpected unit files are invalid. An empty shard file is
  invalid because canonical writers omit empty shards. A present empty
  `units/` directory is read as an empty overlay, but the canonical writer
  omits the directory when there are no units.
- The manifest must be one JSON object with all five required fields and no
  unknown or duplicate fields. Unit records must have every required field
  shown above, including both nested objects and nullable fields, with no
  unknown or duplicate fields. Field order and insignificant JSON whitespace
  are not semantic requirements for reading. Wrong JSON types, non-canonical
  ID/hash spellings, arrays, fractional numbers, and values outside the
  documented coordinate ranges are invalid. Source and target languages must
  satisfy the current `WorkspaceMetadata` domain contract: UTF-8 strings that
  are not empty or whitespace-only. `contentId` and `snapshotId` must be
  non-empty valid canonical HXS IDs in the
  `sha256:<64 lowercase hexadecimal characters>` form.
- Every record's `id` must be placed in the shard derived from its raw ID
  digest. A record in the wrong shard is invalid. IDs must be strictly
  increasing within each shard. Duplicate `TranslationUnitId` values are
  invalid even before considering whether the records' other fields match.
- Duplicate current `SourceBinding` values are invalid across the entire
  workspace, including duplicates in different shards. The reader must build
  the secondary binding index only after this uniqueness check succeeds.
- `targetMacro` must pass intrinsic `aeria-se` validation. Malformed or unsafe
  target strings make loading the whole workspace fail. A target string that
  is opaque but preservable is valid.
- A JSONL reader may accept LF or CRLF record terminators, and may accept a
  final record without a terminating line ending. A single terminal LF is a
  terminator, not a blank record. Any blank or whitespace-only record line,
  including an extra terminal LF, is invalid. A bare CR is not a record
  terminator. The manifest may likewise omit its final LF; the canonical
  writer always emits exactly one LF at EOF.
- The reader validates the stored canonical `TranslationUnitId` and loads that
  exact ID. It must never re-derive an existing ID from its current
  `SourceBinding` or `SourceFingerprint` during load. Rebased units retain
  their original ID even when those current source facts no longer produce
  the original digest. ID derivation is a unit-creation operation, not a load
  validation or repair operation.

Reader acceptance of field order, insignificant whitespace, a leading BOM,
CRLF, or a missing final LF does not make those byte sequences canonical. A
writer must still emit only the byte form specified in the canonical rules
above.

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
