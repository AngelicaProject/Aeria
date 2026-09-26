# Harmonia Pack Format v1

Status: **implemented, not yet released**. `aeria-export` writes this format
and the Harmonia plugin reads it; the format is frozen with the first published
release. The export pipeline that produces this file is described in
[`../architecture/export.md`](../architecture/export.md); the update feed that
distributes it is [`feed-v1.md`](./feed-v1.md).

`crates/aeria-export/tests/fixtures/harmonia-interop.hpk` (format minor 0) and
`harmonia-interop-fonts.hpk` (minor 1, with a [`FONTS`](#fonts-section)
section) are signed packs the Aeria tests regenerate byte for byte and the
Harmonia tests read. Any change to them is a format change.

A Harmonia pack (`.hpk`) is a compiled, read-only runtime artifact. It is not
an editable collaboration format and is never read back by Aeria as project
data.

## Goals

- Lookup by `(sheet, rowId, subrowId, stringOrdinal)` directly from a
  memory-mapped file, without parsing strings or allocating per value.
- A translation is applied only to the exact source string it was made for,
  even when the game version string did not change.
- One self-contained file carries content, metadata, integrity digest, and an
  optional publisher signature, so manual import is a single file.
- Identical project, source, policy, release parameters, and signing key
  produce byte-identical output.

## Container

All integers are little-endian. The file has three parts:

```text
[0, bodyLength - 32)          header, section table, sections
[bodyLength - 32, bodyLength) packHash = SHA-256 of [0, bodyLength - 32)
[bodyLength, EOF)             optional signature block
```

`packHash` is the pack's identity. The feed, the installed-pack record, and the
signature all refer to it.

### Header (64 bytes)

| Offset | Type | Field | Value |
| --- | --- | --- | --- |
| 0 | `u8[8]` | magic | ASCII `AERIAHPK` |
| 8 | `u16` | formatMajor | `1` |
| 10 | `u16` | formatMinor | `0`, or `1` when the pack has a `FONTS` section |
| 12 | `u32` | headerSize | `64` |
| 16 | `u64` | bodyLength | see above |
| 24 | `u32` | sectionCount | number of section table entries |
| 28 | `u32` | flags | `0`; any other value is rejected |
| 32 | `u8[32]` | reserved | zero |

A reader rejects any `formatMajor` other than `1`. A reader accepts a higher
`formatMinor` only because minor versions may add optional sections (below)
and nothing else.

### Section table

`sectionCount` entries of 24 bytes starting at offset 64:

| Type | Field |
| --- | --- |
| `u32` | kind |
| `u32` | reserved, zero |
| `u64` | offset from the start of the file |
| `u64` | length in bytes |

Sections are listed in ascending offset order, start on 8-byte boundaries,
do not overlap, lie entirely before `bodyLength - 32`, and are separated only
by zero padding.

| Kind | Section | Required |
| --- | --- | --- |
| 1 | `MANIFEST` | yes |
| 2 | `NAMES` | yes |
| 3 | `SHEETS` | yes |
| 4 | `LAYOUT` | yes |
| 5 | `ROWS` | yes |
| 6 | `CELLS` | yes |
| 7 | `STRINGS` | yes |
| `0x10000` | [`FONTS`](#fonts-section), minor 1 | no |
| > `0x10000` | optional, added by a later minor version | no; ignored by readers that do not know them |

Each required kind appears exactly once and `FONTS` at most once. Any other
unknown kind below `0x10000` is rejected. A reader that implements minor 0 only
ignores `FONTS` and still applies the translations.

### `MANIFEST`

UTF-8 JSON without BOM, two-space indentation, fields in the order below,
followed by one LF. Readers reject unknown, missing, and duplicate fields.

```json
{
  "packId": "ru-main",
  "title": "Russian translation",
  "publisher": { "name": "Example team", "url": "https://example.com" },
  "license": "CC-BY-NC-SA-4.0",
  "release": { "sequence": 42, "version": "2026.09.25", "channel": "stable" },
  "target": { "language": "ru" },
  "source": {
    "language": "en",
    "gameVersion": "2026.08.12.0000.0000",
    "contentId": "sha256:<hex>",
    "snapshotId": "sha256:<hex>"
  },
  "contentPolicy": "reviewed",
  "project": { "commit": "<40 hex>" },
  "exporter": { "aeria": "0.9.0", "atlas": "0.4.0" },
  "minHarmonia": "1.4.0",
  "counts": { "sheets": 0, "rows": 0, "cells": 0, "reviewedCells": 0, "strings": 0 }
}
```

| Field | Rule |
| --- | --- |
| `packId` | `[a-z0-9][a-z0-9-]{0,63}`; stable for the project's whole release history |
| `title`, `publisher.name` | non-empty display strings |
| `publisher.url`, `license` | optional; `null` when absent |
| `release.sequence` | positive integer, strictly increasing across releases of one `packId` |
| `release.version` | display string; never compared |
| `release.channel` | `stable` or `testing` |
| `target.language`, `source.language` | BCP 47 language tags; `source.language` is the HXS source language |
| `source.*` | copied from the verified HXS the pack was built from |
| `contentPolicy` | `reviewed` or `all` (see [Cell state](#cell-state)) |
| `project.commit` | Git commit of the exported workspace state |
| `exporter` | producing Aeria version, and in `atlas` the string dialect the cells were encoded in, such as `lumina-7.7.0`; packs exported before Aeria encoded strings itself hold the Harmonia Atlas version that encoded them. Readers treat `atlas` as a non-empty display string |
| `minHarmonia` | lowest Harmonia version that implements this format minor |
| `counts` | exact counts; readers verify them. `strings` is the number of distinct stored strings |

The manifest carries no timestamps, local paths, or credentials.

### `NAMES`

Concatenated UTF-8 sheet names without separators. Referenced only by `SHEETS`.

### `SHEETS`

32-byte records, sorted by name in bytewise UTF-8 order, names unique:

| Type | Field |
| --- | --- |
| `u32` | nameOffset into `NAMES` |
| `u32` | nameLength |
| `u8` | variant: `0` default rows, `1` subrows (HXS `SheetVariant`) |
| `u8[3]` | reserved, zero |
| `u32` | layoutStart (index of the first `LAYOUT` record) |
| `u32` | layoutCount (≥ 1) |
| `u32` | rowStart (index of the first `ROWS` record) |
| `u32` | rowCount (≥ 1) |
| `u32` | reserved, zero |

Sheet layout and row ranges are contiguous, in sheet order, and cover their
sections exactly.

### `LAYOUT`

8-byte records. For each sheet, **every** String column of the source sheet
in ascending column index order, whether or not it has translations:

| Type | Field |
| --- | --- |
| `u32` | columnIndex (HXS `columns.index`) |
| `u32` | offset (HXS `columns.offset`, ≤ 65535) |

The position of a record within its sheet's layout is the **string ordinal**,
the index Harmonia uses when it enumerates String column definitions in order.

### `ROWS`

16-byte records, sorted by `(rowId, subrowId)` within a sheet, unique:

| Type | Field |
| --- | --- |
| `u32` | rowId |
| `u16` | subrowId (`0` for variant `0`) |
| `u16` | cellCount (≥ 1) |
| `u32` | cellStart (index of the first `CELLS` record) |
| `u32` | reserved, zero |

### `CELLS`

24-byte records, sorted by ordinal within a row, unique:

| Type | Field |
| --- | --- |
| `u16` | ordinal (< the sheet's layoutCount) |
| `u8` | state: `1` reviewed, `2` unreviewed |
| `u8` | reserved, zero |
| `u32` | stringLength (1..=65535, excluding the terminator) |
| `u32` | stringOffset into `STRINGS` |
| `u32` | reserved, zero |
| `u8[8]` | sourceGuard |

### `STRINGS`

Encoded SeString bytes. Each referenced range `[stringOffset, stringOffset +
stringLength)` contains no `0x00` byte and is followed by one `0x00`. Byte-equal
strings are stored once, in order of first reference when cells are visited in
canonical `(sheet, rowId, subrowId, ordinal)` order. The stored strings and
their terminators cover the section exactly; there are no unreferenced bytes.

The bytes are produced by encoding the validated target macro text in the
Lumina 7.7.0 dialect of the HXS source (see
[`../architecture/export.md`](../architecture/export.md)). Harmonia writes
them into the game row buffer unchanged.

## `FONTS` section

Minor 1 adds an optional section with glyphs for game fonts that lack
characters of the target language (the Latin display fonts have no Cyrillic).
Aeria renders the glyphs from source fonts chosen in the project (see
[`font-settings-v1.md`](./font-settings-v1.md)); Harmonia adds them to the
running game's own font files. Nothing in the game's fonts is replaced.

All integers are little-endian. The section is:

```text
[0, 32)                     header
[32, 32 + metadataLength)   metadata JSON
                            zero padding to an 8-byte boundary
glyphCount × 16 bytes       glyph records
bitmapLength bytes          bitmaps; the section ends here
```

### Header

| Offset | Type | Field |
| --- | --- | --- |
| 0 | `u8[8]` | magic ASCII `HPKFONT1` |
| 8 | `u32` | metadataLength |
| 12 | `u32` | glyphCount |
| 16 | `u32` | bitmapLength |
| 20 | `u8[12]` | reserved, zero |

### Metadata

UTF-8 JSON without BOM, two-space indentation, fields in the order below,
followed by one LF. Readers reject unknown, missing, and duplicate fields.

```json
{
  "sources": [
    {
      "family": "Oswald",
      "copyright": "Copyright 2016 The Oswald Project Authors (https://github.com/googlefonts/OswaldFont)",
      "license": "OFL-1.1",
      "licenseText": "Copyright 2016 …
",
      "sha256": "<64 lowercase hex>"
    }
  ],
  "targets": [
    {
      "font": "TrumpGothic",
      "size": "184",
      "lineHeight": 24,
      "ascent": 19,
      "source": 0,
      "glyphStart": 0,
      "glyphCount": 71
    }
  ]
}
```

| Field | Rule |
| --- | --- |
| `sources` | the source fonts the glyphs were rendered from; each is used by at least one target |
| `family`, `copyright`, `license` | non-empty; `license` is an SPDX identifier |
| `licenseText` | the full license text with LF line endings; shown by Harmonia |
| `sha256` | SHA-256 of the source font file |
| `font`, `size` | `[A-Za-z0-9]+` and `[0-9]+`: the target is `common/font/<font>_<size>.fdt` |
| `lineHeight`, `ascent` | the `fthd` line height and ascent the glyphs were fitted to; `1 ≤ ascent ≤ lineHeight ≤ 255` |
| `source` | index into `sources` |
| `glyphStart`, `glyphCount` | the target's glyph records; `glyphCount ≥ 1` |

Targets are sorted by `(font, size)` in bytewise order and unique. Their glyph
ranges are contiguous, in target order, and cover all glyph records.

### Glyph records

16 bytes each, sorted by codepoint within a target, unique:

| Type | Field |
| --- | --- |
| `u32` | codepoint: a Unicode scalar value ≥ `U+0080` |
| `u8` | width |
| `u8` | height |
| `u8` | offsetY: first bitmap row, counted from the top of the line |
| `u8` | advance |
| `u32` | bitmapOffset into the bitmap area |
| `u32` | reserved, zero |

A glyph's bitmap is `width × height` bytes of 8-bit coverage (`0` transparent,
`255` opaque), row-major. Column 0 is the pen position: left side bearing is
part of the bitmap, as in game `.fdt` glyphs. Rules:

- `offsetY + height ≤ lineHeight`;
- `advance − width` fits in `i8` (the `.fdt` next-offset field);
- bitmaps are stored in glyph record order, each at the end of the previous
  one, and cover the bitmap area exactly. A glyph without ink has width or
  height `0` and an empty bitmap.

### Application by Harmonia

Harmonia applies the section at game start, only for the active pack:

1. For each target and for each of the two font sets, the main set
   (`common/font/<font>_<size>.fdt` with `common/font/fontN.tex`) and the
   title-screen set (`<font>_<size>_lobby.fdt` with `font_lobbyN.tex`), it
   reads the game's own files. A target whose `.fdt` is missing or whose
   `fthd` line height or ascent differ from the section is skipped and
   reported.
2. Glyphs whose codepoint the `.fdt` already has are skipped; existing glyphs,
   kerning, and texture pages are never changed.
3. The remaining glyphs are packed into atlas pages that no game font uses and
   that are entirely empty in the game's texture. A page is texture
   `(page / 4) + 1`, channel `page % 4`. A target that does not fit is skipped
   as a whole and reported.
4. Coverage is rounded to the 4-bit channels of the `0x1440` textures
   (`round(value × 15 / 255)`), glyph records are inserted into the `.fdt`
   keeping codepoint order, and the modified files are served to the game
   through a Penumbra temporary mod. Without Penumbra translations still apply
   and fonts do not.

The exact packing and file rules are Harmonia's; they are described in its
`docs/packs.md`.

## Source guard

`sourceGuard` is the first 8 bytes of the HXS `raw_hash` of the source string
the translation was made for:

```text
SHA-256("HARMONIA-HXS-V1-RAW-STRING" || u32le(len(raw)) || raw)[0..8]
```

`raw` is the source string's bytes as stored in the EXD row, without the
terminating `0x00`. Harmonia computes the same value from the string it is
about to replace and writes the translation only on an exact match.

A translation unit whose source occurrence has no `raw_hash` is not exported.

> Open item: an in-game check must confirm that HXS `raw_value`
> (`ReadOnlySeString.Data`) equals the in-memory row string without its
> terminator for every String column. Until then a mismatch is safe but shows
> up as untranslated text and as source-changed counts in Harmonia diagnostics.

## Cell state

`contentPolicy: "reviewed"` means every cell has state `1`. `all` also exports
`draft` and `needs-review` units as state `2`. Harmonia may let the player
choose whether state-`2` cells are applied; the pack never mixes in other
states.

## Signature block

Present only in signed packs; an unsigned pack ends at `bodyLength`.

| Type | Field |
| --- | --- |
| `u8[8]` | magic ASCII `HPKSIG01` |
| `u16` | algorithm: `1` = ECDSA P-256 with SHA-256 |
| `u16` | reserved, zero |
| `u8[65]` | publicKey, SEC1 uncompressed point |
| `u8[64]` | signature, IEEE P1363 `r ‖ s` |
| `u8` | hasEndorsement: `0` or `1` |
| `u8[65]` | previousPublicKey, only when hasEndorsement = 1 |
| `u8[64]` | endorsement, only when hasEndorsement = 1 |

- `signature` signs `"AERIA-HPK-V1-SIGNATURE" || packHash` with `publicKey`.
  Signatures are deterministic (RFC 6979).
- `endorsement` signs `"AERIA-HPK-V1-KEY-ROTATION" || publicKey` with
  `previousPublicKey`. It lets a publisher rotate keys without users
  re-trusting manually.
- Key fingerprint: SHA-256 of the 65-byte public key, shown in hex.
- Nothing may follow the block.

The signature covers `packHash`, which covers the manifest, so `packId`,
`release.sequence`, `source`, and `minHarmonia` are all authenticated.

## Transport encoding

For download a pack may be wrapped as `.hpk.br`: a Brotli stream whose
decompressed bytes are exactly the `.hpk` file, including the signature
block. Pack identity is always `packHash` of the decompressed file; the
compressed bytes are not an identity.

## Reader requirements (Harmonia)

Before a pack is installed or loaded, the reader verifies:

1. header, section table, alignment, bounds, and required sections;
2. `packHash`;
3. the signature and endorsement when present;
4. manifest schema and that `counts` match the sections;
5. when present, every `FONTS` rule above;
6. every ordering, uniqueness, range, and cross-reference rule above;
7. every string range, its terminator, and that it parses as a well-formed
   SeString.

Any failure rejects the whole pack.

At runtime, for each sheet row Harmonia is about to rewrite:

1. The pack is used only when `source.language` equals the game client
   language.
2. A sheet is used only when the running sheet's String column definitions,
   in order, equal its `LAYOUT` records exactly (same count, index, and
   offset). The result is cached per loaded sheet.
3. Lookup uses `(rowId, subrowId)`; subrows are never collapsed.
4. A cell is written only when the current source string's guard equals
   `sourceGuard`. Otherwise the original text stays and a per-sheet mismatch
   counter is incremented.

A game version different from `source.gameVersion` does not reject the pack.
The per-cell guard and layout check keep application safe, and the consumer
reports the pack as built for a different game version together with the
share of guarded cells that matched.

## Limits

A pack is at most 1 GiB. A string is at most 65535 bytes. Column indices and
offsets are at most 65535.

## Open decisions

- Whether HXS `gameVersion` (the text of `ffxivgame.ver`) and the client's
  `GameVersionString` always use the same representation. A difference only
  makes Harmonia report every pack as built for another game version.
