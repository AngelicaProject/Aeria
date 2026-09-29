# Search and translation memory

`aeria-search` owns local source search and translation memory. Its data is
machine-local, rebuildable cache: deleting it never loses translation work,
and it is never written to a repository.

## Source index

`SourceIndex` is one SQLite file per source language and game version,
stored by the desktop in `<app-data>/search/<key>.sqlite3`, where the key is
derived from a SHA-256 hash of `<language>/<game version>`. It holds every
translatable String cell of the game (see
[`source.md`](./source.md#translation-permission)), with its coordinate, macro
text, and plain text, and an FTS5 index over the plain text.

The plain text of a macro string is its text ranges from `aeria-se` semantic
analysis, in order, with a space where a macro separates two ranges; escapes
contribute their character. Searches therefore match what players read, not
macro names or arguments.

The tokenizer follows the source language: three-character
substrings (`trigram`) for Japanese, Chinese, and Korean, which are written
without spaces, and case- and diacritic-insensitive words (`unicode61` with
`remove_diacritics 2`) otherwise.

The index is built by reading every sheet of the game once, written to a
`.partial` file, and published by rename only when complete. A file is used
only when its recorded format (`2`) and source key match; any other file is
rebuilt. A cancelled or failed build leaves no index.

## Queries

- **Source search**: with the word tokenizer, every word of the query must
  match as a prefix; with trigrams, the query must occur as a substring.
  Queries without words, and trigram queries shorter than three characters,
  scan the plain text for a case-insensitive (ASCII) substring instead. Hits
  are ranked by BM25, then by index order, up to 200 per page, optionally for
  one sheet.
- **Similar sources**: of the text's 12 longest distinct words, the 4 in the
  fewest indexed strings find up to 40 candidates; words in more than 20,000
  strings are left out unless the text has nothing else, since ranking their
  matches costs far more than they tell (an OR over "the" takes about 170 ms,
  over rare words 1–7 ms, and the cost grows with the number of words).
  Texts in languages indexed by trigrams use up to 16 evenly spread
  trigrams. `SourceIndex::similar_search` keeps one connection for many
  texts; `SourceIndex::word_counts` reads how many strings contain each word
  from the index's `fts5vocab` once (about 75,000 words in 0.4 s), and
  `similar_search_with` answers every word from it instead of a query per
  word. Candidates are ranked by similarity. Similarity is the Dice coefficient of the plain texts'
  lowercased character bigrams, with whitespace runs as one space; identical
  texts score 1. Candidates below 0.5 are dropped.
- **Translation text**: `text_contains` matches a translation's plain text
  against a query, ignoring case. Translations change with every edit, so
  they are searched in the workspace rather than indexed.

## Terminology candidates

`SourceIndex::term_candidates` finds names that recur in the game's text,
for filling a project glossary. It reads the index twice and takes about two
seconds for the English client.

1. A *name* is an indexed cell of a data sheet (a sheet name without `/`, so
   not quest, cutscene, or other script text) whose text has no macros, is 2
   to 48 characters and at most 5 words, contains a letter, and has only
   letters, digits, spaces, and `'`, `’`, `-`, `&`, `・`, or `ー`. With the
   word tokenizer it must start with an uppercase letter. Cells with the same
   text are one name.
2. Every indexed string is scanned for all names at once (Aho-Corasick,
   leftmost-longest, so `Fire Shard` is found instead of `Fire`), with the
   same case. With the word tokenizer a match must stand as whole words and
   must not start a sentence or a line, where any word is capitalized. A
   string whose whole text is the name does not count.
3. A candidate is a name found in at least the requested number of other
   strings. Candidates are ordered by those strings, then occurrences, then
   text, and keep up to three of their name cells.

The result is a heuristic: ordinary capitalized words, such as interface
labels or the pronoun *I*, can rank high. Choosing terminology is left to
the translator or an agent.

## Desktop use

No desktop feature uses the index yet; the `aeria` command will (see
[`agents.md`](./agents.md#the-aeria-command)). Indexes are kept per source
language and game version in application data; those of game versions no
longer in use stay on disk until the user clears application data.

Translation memory is the similar sources that have a non-empty bound
translation in the workspace, with the translation and its review state.
