# Search and translation memory

`aeria-po` owns search and replace over the project's files (see
[Project search](#project-search)). `aeria-search` owns local source search
and translation memory. Its data is
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
  they are searched in the project's files rather than indexed.

## Glossary candidates

`aeria_po::term_candidates` finds where a project disagrees with itself: names
of its English sources that its translations render in several ways and
its glossary does not have, such as `Kojin` as «Кодзин», «Кудзин», and
«Койдзин». It reads every file of `po/`, several at a time, and takes about
seven seconds on a project of the whole game. A person decides what becomes
a term.

1. A *candidate* is a capitalized phrase of the plain text of the sources:
   up to four words, hyphenated names whole (`Radz-at-Han`), joined by `of`,
   `the`, `de`, or `del` (`Students of Baldesion`). The first word of a
   sentence and leading function words (`The`, `You`) are not part of it.
   It is in at least 12 strings, at least 6 of them outside interface
   sheets (whose labels are capitalized anyway), not in more than four in
   five of them inside a longer candidate, and not a term of the glossary
   (compared without case and a leading `the`).
2. A *rendering* is a Russian word of the translations of the candidate's
   translated strings that is in at least 4 and a twentieth of them, at
   least 30 times more frequent there than in all translations, and with
   at least three tenths of all its strings among them: the candidate's
   translation, not a word around it. Words are compared without their
   endings, and bases that differ only at the end are forms of one word
   (Кодзина, Кодзину); other spellings differ in the middle (Шарлаян,
   Шаллаян).
3. Words that share at least three fifths of their strings are one
   rendering (Лимсы Ломинсы). The rendering with most strings is the main
   one; another with at least 4 strings and a twentieth, sharing at most
   three twentieths of its strings with the main one, is its rival.
4. A candidate with rivals is reported, with each rendering's words as
   translations most often write them, its strings, and up to three
   examples. A rendering's word that sounds like the name comes first: its
   first consonants are the same, and three of its first four, or all of
   the shorter, follow each other in both (`Kojin`, «Кодзин», «Кудзин»:
   `kdzn`). When every rendering has such a word, the candidate is
   *spellings* of one name, almost always a real disagreement; otherwise its
   renderings are other words, translations of the name or words around it,
   for the person to check. Spellings come first, then by the strings of the
   rivals.

The result is a heuristic: a rendering can still be a word that goes with a
name rather than translate it, so the person reviews each candidate.

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
the translator.

## Project search

`aeria_po::search` searches the project's files, the only state of a
project, so it always sees the current translations; there is no index of
`po/`. A search reads every chosen file, several at a time, and lists its
entries in file order (files sorted by path):

- **Pattern**: text, whole words (`\b` around the text), or a regular
  expression of the `regex` crate, case-insensitive unless asked otherwise.
  A pattern without regular-expression syntax and without a character the
  PO format escapes (`"`, `\`, a line break, a tab) first tests the raw
  file and skips files without a match before parsing them.
- **Fields**: the translation, the source, the translator's notes, and the
  `msgctxt`. In translations and sources only the text a translator writes
  matches: text nodes and translatable macro arguments, such as the branches
  of `<if $gn4>готова<else>готов</if>`. Macro names and arguments
  (`<sheet Item $n1 0>`) never match. Malformed macro text is one text.
- **Filters**: files and folders relative to `po/` (`quest/`, `Addon`), the
  entry state (untranslated, translated, fuzzy), and the checks a translation
  is saved with (see [`po-project.md`](./po-project.md#checking)): only
  translations with a problem, which are not exported, or only translations
  with advice, such as a term of the source whose translation does not seem
  to be used. A search may have no pattern and only filters.

Findings of the checks are data (`aeria_po::Issue`: the kind with its
term, variant, or word), so an interface words them in its own language; each
has the English message Aeria has always written, and a group (the kind, with
the term for term issues). With a checks filter a search also counts, over
every entry found, the entries with an issue of each group, most first, and
can keep only the entries with an issue of one group.

A search returns at most 2,000 entries with the byte ranges of their matches,
every file with an entry found and its count, and counts the rest; a new search in the desktop cancels the one in
progress. `search_all` returns every entry found, for an action on a whole
result; it is never cancelled, so it never returns part of one. On the full game (about 7,300 files) a text search takes about half
a second, a regular expression about one, and checking every translation
about two seconds. The glossary finds a string's terms with one automaton of
all its terms in one pass, and a check finds the source's terms once for
every term check.

### Replacing

A replacement changes translations only, and only the text ranges a search
matches in: game data cannot change. The replacement is text, or with a
regular expression a template with `$1` and `${name}`; with *preserve case*,
a match in capitals or with a capital first letter keeps that case.
Inserted text is escaped as macro text (`\<`, `\{`, `\\`), so it never
becomes a macro.

`preview_replace` lists every change with the translation before and after
and the problems of the new one; nothing is written. `Session::apply_edits`
writes a list of edits, each file once:

- an edit is made only while its entry still has the translation and fuzzy
  mark it was made from; an entry changed since is skipped;
- a replacement is checked like a saved translation and skipped when it has
  a problem, so an invalid translation is never written;
- a replacement keeps the entry's fuzzy mark: replacing text is not a review;
- skipped edits are reported with the reason, never fatal.

Every applied edit returns its entry's state before and after, from which
the inverse edit is made. The desktop keeps the inverse of the last bulk
edit in memory: **Undo** restores exactly the earlier state of the entries
that did not change since and reports the others. Git keeps everything else.

### Term exceptions

A found string can take an exception for a term of its findings, and with
one group of term findings chosen, every string found can (see
[`po-project.md`](./po-project.md#term-exceptions)). The exception is a bulk
edit like a replacement: made only while the entry is unchanged, and the one
Undo reverts.

### Translating again

A found translation can be machine-translated again with the current terms
and style: its translation is cleared (with its fuzzy mark and previous
source) as one bulk edit that Undo reverts, and a machine translation run of
exactly those strings starts (see
[`translate.md`](./translate.md#what-is-translated)). A run that stops leaves
the rest untranslated, so any later run takes them.

## Desktop use

The Search tool and the `#` prefix of the command palette use project search
(see [`desktop-editor-ui.md`](./desktop-editor-ui.md#search-tool)). No
desktop feature uses the source index yet. Indexes are kept per source
language and game version in application data; those of game versions no
longer in use stay on disk until the user clears application data.

Translation memory is the similar sources that have a translation in `po/`,
with the translation and whether it is fuzzy.
