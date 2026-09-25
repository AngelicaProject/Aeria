# Search and translation memory

`aeria-search` owns local source search and translation memory. Its data is
machine-local, rebuildable cache: deleting it never loses translation work,
and it is never written to a repository.

## Source index

`SourceIndex` is one SQLite file per source package, stored by the desktop in
`<app-data>/search/<key>.sqlite3`, where the key is derived from a SHA-256
hash of the package ID. It holds every non-empty String cell that HSG grants
for translation, with its coordinate, macro text, and plain text, and an FTS5
index over the plain text.

The plain text of a macro string is its text ranges from `aeria-se` semantic
analysis, in order, with a space where a macro separates two ranges; escapes
contribute their character. Searches therefore match what players read, not
macro names or arguments.

The tokenizer follows the package's source language: three-character
substrings (`trigram`) for Japanese, Chinese, and Korean, which are written
without spaces, and case- and diacritic-insensitive words (`unicode61` with
`remove_diacritics 2`) otherwise.

The index is built from a verified HXS handle and the HSG guidance index,
written to a `.partial` file, and published by rename only when complete. A
file is used only when its recorded format (`1`) and package ID match; any
other file is rebuilt. A cancelled or failed build leaves no index.

## Queries

- **Source search**: with the word tokenizer, every word of the query must
  match as a prefix; with trigrams, the query must occur as a substring.
  Queries without words, and trigram queries shorter than three characters,
  scan the plain text for a case-insensitive (ASCII) substring instead. Hits
  are ranked by BM25, then by index order, up to 200 per page, optionally for
  one sheet.
- **Similar sources**: up to 16 of the text's longest distinct words (or
  evenly spread trigrams) find up to 300 candidates, which are ranked by
  similarity. Similarity is the Dice coefficient of the plain texts'
  lowercased character bigrams, with whitespace runs as one space; identical
  texts score 1. Candidates below 0.5 are dropped.
- **Translation text**: `text_contains` matches a translation's plain text
  against a query, ignoring case. Translations change with every edit, so
  they are searched in the workspace rather than indexed.

## Desktop use

The desktop builds the active package's index in a background worker the
first time Angelica needs it or a message is sent to her; requests made
meanwhile report that the index is being built. A failed build is reported
once and retried on the next request. Building opens its own HXS handle, so it
never holds the project lock; searches take the lock only to add workspace
translations, and are refused when the project changed in the meantime.
Indexes of source packages no longer in use stay on disk until the user
clears application data.

Translation memory is the similar sources that have a non-empty bound
translation in the workspace, with the translation and its review state.
