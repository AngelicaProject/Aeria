# Game source

The source of an Aeria project is the installed game. Aeria reads the Excel
sheets of the game directly through `aeria-sqpack` and derives every source
fact it needs from them: strings, sheet layouts, translation permission, and
row keys. Nothing about the source is extracted into an intermediate
artifact, and nothing about it is cached across processes.

`aeria-source` owns this model. It depends on `aeria-sqpack` for the file
formats and on `aeria-se` to print string bytes as macro text.

## Opening a source

`GameSource::open(game_path, language)` takes the game folder, the one that
contains `game/sqpack`, and a source language (`ja`, `en`, `de`, or `fr`).
It reads:

- the game version, the trimmed text of `game/ffxivgame.ver`. All Excel
  files are in the base `ffxiv` repository, so expansion version files do
  not describe the source;
- the sheet list, `exd/root.exl`, restricted to sheets that have a header.

A game version is a dot-separated list of decimal numbers, such as
`2026.09.15.0000.0000`. Versions are compared component by component; a
version that does not have this form is rejected.

`GameSource::current_version` reads the version file again. Callers that
keep a source open while the game can be patched, such as export, compare it
with the version the source was opened with and stop when it changed.

## Sheets

`GameSource::sheet(name)` reads one sheet in the source language and returns
one of:

- `Missing`: the sheet is not in the sheet list;
- `Unavailable(reason)`: the sheet is listed but cannot be read, because its
  variant or a column type is unknown, its pages are malformed, or it has
  neither data in the source language nor language-neutral data;
- a `SourceSheet`.

A sheet in a language it does not provide is read from its language-neutral
data, as the game does.

A `SourceSheet` holds its rows in `(row, subrow)` order and, for every row,
the bytes of each String column. A subrow is numbered by its position in its
row. Recently read sheets are kept in memory and read again when evicted.

### Layout

A sheet's *layout* is the ordered list of its String columns, each as its
column index and its byte offset in the row. It is exactly what Harmonia
compares before it touches a sheet (see
[`../formats/pack-v1.md`](../formats/pack-v1.md#reader-requirements-harmonia)).

The layout hash identifies a layout in persisted data. It is the first 8
bytes of

```text
SHA-256("aeria.sheet-layout.v1" || for each String column: u32le(index) || u16le(offset))
```

written as 16 lowercase hexadecimal characters. Non-String columns do not
take part: a patch that adds or moves a number column leaves the layout, and
every translation of the sheet, unchanged.

### String text

`SourceSheet::text(row, subrow, column)` prints the String cell with
`aeria_se::codec::decode`. The printed macro text is what translators see
and what translation units store (see
[`../formats/workspace-v3.md`](../formats/workspace-v3.md)). Two cells whose
texts are equal are the same source for Aeria even when their bytes differ.
Export and Harmonia compare bytes instead (see
[`export.md`](./export.md)).

## Translation permission

A String cell may be translated only when its text is language-dependent.
Keys, file names, and other identifiers are the same in every language, and
changing them breaks the game.

For one sheet, the *evidence languages* are the global client languages
Japanese, English, German, and French. The sheet is *comparable* when it
can be read in every evidence language and, in all of them, has the same
variant, the same columns (type and offset), and the same list of
`(row, subrow)` coordinates. A sheet with only language-neutral data reads
the same in every language, so none of its cells is translatable. In a
comparable sheet, a cell is translatable when its source bytes are not empty
and differ from the bytes of the same cell in at least one other evidence
language. Every cell of a sheet that is not comparable is not translatable.

Permission is computed for a whole sheet the first time it is needed and
kept with the sheet. It depends only on the game data, so every collaborator
on the same game version computes the same permission.

A translatable cell whose text contains no letters outside protected
structure, such as `...`, a `0` placeholder, or only number formatting, is
still translatable. The read path marks it `formatting_only` so the editor
can label and filter it; see `MacroString::is_formatting_only` in
`aeria-se`.

## Row keys

Some sheets number their rows as a sequence and carry a stable text key in a
String column, for example `TEXT_..._000_000` in quest dialogue. Inserting a
line renumbers every later row, so the row ID is not a durable coordinate
there, but the key is.

A String column is the sheet's *row key column* when:

1. the sheet has at least two rows;
2. every row has a non-empty text in the column;
3. the texts are unique across rows; and
4. none of the column's cells is translatable, which means the text is the
   same in every evidence language.

When several columns qualify, the lowest column index is used. The key of a
row is the text of its row key cell. Condition 4 excludes translatable text:
a translated name that is merely unique must never act as an identity key.

## Performance

Reading every sheet of one language takes about one to two seconds on one
thread. A sheet is read on demand and takes milliseconds. Permission reads
the sheet in the three other evidence languages as well. Opening a project
reads every sheet that holds translation units, once.

## Degraded workspace

A project whose game is older than the project's game version, or that has
no game folder configured, cannot be edited. Git history, the manifest, the
glossary, and translations remain inspectable in the repository, but editing,
preview, assisted translation, and export require the game at the project's
version or newer.

## Additional source languages

Additional official languages are local context for a translator. They are
read from the same installation and are never recorded in the project.
