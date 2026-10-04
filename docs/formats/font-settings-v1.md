# Font Settings v1

Status: **implemented in `aeria-fonts`**.

Font Settings v1 chooses the source fonts from which Aeria renders glyphs that
the game's own fonts lack, or that replace the game's own `AXIS` Cyrillic, and
how they are fitted. It is a single optional
file, `aeria-fonts.json`, in the project root next to `aeria-pack.json`. The
source font files and their license texts live in the project directory
`fonts/`. Both are committed with the project, so every maintainer exports the
same glyphs. How the glyphs reach the game is the `FONTS` and
font-replacements sections of [`pack-v1.md`](./pack-v1.md); the export
pipeline is [`../architecture/export.md`](../architecture/export.md).

## Absence

A missing file means the pack has no font sections. Aeria does not create
the file until the font settings are saved explicitly. Saving them only writes
the files; the next checkpoint commits them.

## Canonical form

UTF-8 JSON with two-space indentation, the field order below, and one LF after
the closing `}`:

```json
{
  "formatVersion": 1,
  "characters": "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя№«»„“",
  "sources": [
    {
      "id": "oswald",
      "file": "fonts/Oswald-Variable.ttf",
      "family": "Oswald",
      "copyright": "Copyright 2016 The Oswald Project Authors (https://github.com/googlefonts/OswaldFont)",
      "license": "OFL-1.1",
      "licenseFile": "fonts/OFL-oswald.txt"
    }
  ],
  "fonts": [
    {
      "font": "TrumpGothic",
      "source": "oswald",
      "axes": { "wght": 400.0 },
      "scale": 1.0,
      "widthScale": 0.8,
      "baselineShift": 0,
      "tracking": 0.0,
      "caseMapping": "none",
      "sizes": { "184": { "scale": 1.05 } }
    }
  ]
}
```

| Field | Rule |
| --- | --- |
| `formatVersion` | the JSON integer `1` |
| `characters` | the characters added to every size; each ≥ `U+0080`, not a control character, listed once. A replacing game font (`AXIS`) gets only those in the Cyrillic block `U+0400`–`U+04FF`; the others keep the game's glyphs |
| `sources[].id` | `[a-z0-9][a-z0-9-]{0,63}`, unique |
| `sources[].file`, `licenseFile` | `fonts/<name>` paths with `/` separators and no `.` or `..` segments |
| `sources[].family`, `copyright`, `license` | non-empty without surrounding whitespace; `license` is an SPDX identifier |
| `fonts[].font` | a supported game font: `AXIS`, `Jupiter`, `TrumpGothic`, or `MiedingerMid`; listed once |
| `fonts[].source` | id of a source |
| `fonts[].axes` | variation axis values by four-letter tag; each within the source's axis range at export. Default `{}` (the font's default instance) |
| `fonts[].scale` | 0.5–2.0, default 1. Multiplies the size at which the source capital `H` is as tall as the native capitals |
| `fonts[].widthScale` | 0.5–2.0, default 1. Horizontal scale of outlines and advances |
| `fonts[].baselineShift` | −16–16 pixels, default 0; positive moves glyphs down |
| `fonts[].tracking` | −0.5–1.0, default 0. Space added to every advance, as a fraction of the native capital height |
| `fonts[].caseMapping` | `none` (default) or `upper`: draw each character with its uppercase glyph, for game fonts with capitals only |
| `fonts[].sizes` | overrides keyed by the size part of the `.fdt` name (`"16"`, `"184"`); each may set `source`, `axes`, `scale`, `widthScale`, `baselineShift`, `tracking`, and falls back to the font entry otherwise |

Every size of a listed game font is generated. The supported sizes and their
native metrics are the table in `crates/aeria-fonts/src/targets.rs`:

| Game font | Sizes |
| --- | --- |
| AXIS (replaces the game's Cyrillic) | 96 (9.6 px), 12, 14, 18, 36 |
| Jupiter | 16, 20, 23, 46 (45 and 90 hold digits only) |
| TrumpGothic | 23, 34, 68, 184 (18.4 px) |
| MiedingerMid | 10, 12, 14, 18, 36 |

`Meidinger` holds only digits and signs and is not supported.

`AXIS` sizes are fitted to the pixel grid of the game's own Latin of the same
size, measured with the other metrics: vertical stems get the native stem width
and start at the same fraction of a pixel, horizontal bars get the native
thickness on whole pixels, the baseline, x-height, and cap height land on the
native rows, and the glyphs are shifted so the space between letters is split
between their sides as in the native Latin. With `scale` other than 1 the zones
follow the scaled heights. `widthScale`, `tracking`, and `baselineShift` apply
as for the other fonts.

## Reader contract

- A leading UTF-8 BOM is accepted and ignored.
- Unknown fields are rejected at every level. A `formatVersion` other than
  `1` is rejected as unsupported.
- Invalid settings are reported; Aeria never falls back to defaults for a file
  that exists but is invalid. A source that lacks a glyph for a listed
  character, or an axis value outside the font's range, fails the export.
- License files are read as UTF-8 and their CRLF line endings are normalized
  to LF, so a Git checkout with `core.autocrlf` exports the same pack.

## Recommended fonts

Aeria bundles four SIL Open Font License 1.1 fonts from
`github.com/google/fonts` and offers them as the default. *Use recommended
fonts* writes them and their licenses into `fonts/` and fills the settings:

| Game font | Source | Parameters |
| --- | --- | --- |
| AXIS | Open Sans (variable) | `wdth` 94, `wght` 520; `wdth`/`wght` for sizes 96: 86/560, 12: 96/560, 18: 95/455, 36: 96/440 |
| Jupiter | Cormorant SC SemiBold (Medium for 46) | `widthScale` 0.95; small letters are the font's small capitals, as in Jupiter |
| TrumpGothic | Oswald (variable) | `wght` 400, `widthScale` 0.8 |
| MiedingerMid | Unbounded (variable) | `wght` 600, 500 for sizes 10–14; `tracking` 0.05; `caseMapping` `upper` |

The parameters were chosen by comparing rendered Cyrillic against the native
Latin glyphs of each size: capital height, stroke weight, and advance. For
`AXIS`, the weight matches the ink of the native small Latin letters and the
width keeps the mean advance of Russian text at or just below the game's own
Cyrillic, within about 1.5 %, so lines wrap as they did.

The file contains no local paths, credentials, or machine-specific values.
