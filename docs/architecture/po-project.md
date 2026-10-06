# The project as PO files

Status: **implemented** in `aeria-po` (the format, identity, files, game
updates, checks, and the session the desktop edits through), the desktop,
`aeria-git`, `aeria-export`, and `aeria-check`.

## Why

A project's text is the game's text and its translation, kept as files that
people, the desktop, machine translation, and Git all work on directly: one
truth, with no second store to keep in step with it. Principle 7 of
[`../product/principles.md`](../product/principles.md) asks that friendly
operations and Git work on the same project state; the PO files are that
state.

Review states and authorship marks do not exist. Git answers their questions
better: who changed a string and when is the history, what changed is the
diff, and accepting work is committing it.

## The project

A project is a folder, usually a Git repository:

```text
aeria.json                  format, source and target languages, game version
po/                         the game's text and its translation, as PO files
  quest/000/ClsArc000_00021.po
  BNpcName/3000.po
  ...
aeria-knowledge/            project knowledge: style.md, terms.csv
aeria-pack.json, aeria-fonts.json, fonts/, ...
```

Every translatable string of the game is an entry in `po/`, translated or
not. There is no separate store of translations and no state outside the
files and Git. `aeria.json`:

```json
{
  "format": "aeria-po/2",
  "sourceLanguage": "en",
  "targetLanguage": "ru",
  "gameVersion": "2026.09.15.0000.0000"
}
```

`gameVersion` is the game version the files of `po/` are for; a project that
does not record it takes the `X-Game-Version` of its first file.

`format` is `aeria-po/2`, or `aeria-po/1` for a project without review
marks (see [Reviews](#reviews)); Aeria reads both. A version of Aeria that
reads only `aeria-po/1` would drop review marks from every file it writes,
so a project takes `aeria-po/2` when Aeria writes its first review mark, and
such a version then refuses to open it. New projects start as
`aeria-po/2`. `aeria.json` is committed with the translations, so
collaborators receive the new format with the first marks.

The repository holds the game's text in the source language and the other
client languages (Japanese, English, German, French, as the client ships
them): for the whole game about 7,300 files and 430 MB, about 117 MB
compressed in Git; a patch adds the changed files. Public community
repositories already publish the complete text of the game in every client
language.

### Files

- A quest (`quest/…`) or cutscene (`cut_scene/…`) sheet is one file, its
  entries in play order.
- Any other sheet is one file when its row IDs are below 1,000, otherwise a
  folder of files by row ID range: `BNpcName/3000.po` holds rows 3000–3999.
  A range is a range of IDs, not of positions, so an added row never moves
  another row to a different file.
- Paths are safe on systems that ignore case: a segment Windows reserves
  (`CON`, `AUX`, `NUL`, `COM1`, …) gets `~`; each folder takes the casing it
  first has in the game's sheet list; a sheet whose path is also a folder of
  other sheets gets `~` (the data sheet `Quest` is `Quest~/`, beside the
  dialogue in `quest/`); a sheet whose path differs from an earlier one's
  only by case gets `~` and its number among them.

A file starts with a header entry:

```po
# quest/000/ClsArc000_00021 — «Way of the Archer» · in play order
msgid ""
msgstr ""
"Language: ru\n"
"Content-Type: text/plain; charset=UTF-8\n"
"X-Game-Version: 2026.09.15.0000.0000\n"
"X-Source-Language: en\n"
```

`X-Game-Version` is the game version the file was made for.

### Entries

```po
#. ja: 弓術士ギルドの受付、アセリナは、…
#. de: Athelyna kann dich Gildenmeisterin Luciane vorstellen, …
#. fr: Athelyna a préparé votre admission à la guilde des archers.
#. kind: journal
# Ателина — не «Афелина»: так в глоссарии.
msgctxt "quest/000/ClsArc000_00021:TEXT_CLSARC000_00021_SEQ_00:1"
msgid "Athelyna wishes you to reaffirm your desire to join the Archers' Guild."
msgstr "Ателина из Гильдии лучников хочет убедиться, что намерение вступить в гильдию твёрдое."
```

| Part | Meaning | Written by |
| --- | --- | --- |
| `#.` lines | the other client languages, the speaker or kind of line, the row's other cells, what the macros do | Aeria, from the game; never edited by hand |
| `# ` lines | translator notes | people |
| `#, fuzzy` and `#\| msgid` | a game update changed the source; `#\| msgid` is the source the translation was written for | Aeria, in a game update |
| `#, aeria-term-exception: <term>` | a term of the glossary does not apply to this string; see [Term exceptions](#term-exceptions) | people, through Aeria |
| `#, aeria-reviewed: <fingerprint>` | a person reviewed the translation whose fingerprint this is; see [Reviews](#reviews) | people, through Aeria |
| `msgctxt` | the entry's identity; see [Identity](#identity) | Aeria |
| `msgid` | the source text as macro text ([`strings.md`](./strings.md#macro-text)) | Aeria |
| `msgstr` | the translation as macro text; empty while there is none | people and machine translation |

`fuzzy` states a fact of the game, not an opinion of a translator: the
source changed after the translation was written. Term exceptions and the
review mark are the decisions of a person that Aeria reads: all are flags of
the same `#,` line, `fuzzy` first, then one `aeria-term-exception` flag per
term in the order they were added, then the review mark
(`#, fuzzy, aeria-term-exception: the Maelstrom, aeria-reviewed: 3f9a1c0b7d2e`).
Aeria keeps no other flag; a file written by another tool loses its other
flags when Aeria writes it.

### Term exceptions

A glossary term can mean something else in a string: `maelstrom` is the
Grand Company in one string and a whirlpool in another. A person marks such a
string with an exception for the term. In that string the checks neither ask
for the term's translation nor forbid its variants, and machine translation
is told the term does not apply (see
[`translate.md`](./translate.md#the-request)). Only a person adds or removes
an exception, from the editor or from search; machine translation never
does, so which terms apply to a string never depends on AI judgment.

The flag holds the term as the glossary writes it and is compared ignoring
case. A term with a comma cannot be an exception, since flags are separated
by commas. An exception stays when the translation changes, since it is about
the meaning of the source; it is dropped when a game update changes the
source (see [Game updates](#game-updates)). An exception that names no term
of the source, because the term left the glossary or matches no longer, is
advice.

Entries are ordered as the game orders them: play order in a scene, row and
column order elsewhere. The `#.` lines are derived only from the game, so
every collaborator with the same game version produces the same bytes for
them and Git never sees them conflict. Files are written in one canonical
form, one line per field, so a save changes only the lines of the string it
changed.

### Reviews

A person marks a translation they reviewed, in the editor (see
[`desktop-editor-ui.md`](./desktop-editor-ui.md#translation-editor)); no
save, bulk replacement, or machine translation marks one. Machine
translation never changes a reviewed translation: a run leaves it, also when
its source changed, and translating found strings again skips it (see
[`translate.md`](./translate.md#what-is-translated) and
[`search.md`](./search.md#translating-again)). A person removes the mark to
give the string back to machine translation.

The mark holds the translation's fingerprint: the first twelve hexadecimal
digits of the SHA-256 of its `msgstr` (`aeria_po::fingerprint`). It holds
only while the translation is still that text (`Entry::is_reviewed`). Git
merges files line by line, and the flag and `msgstr` are different lines: a
branch that marks a translation reviewed and another that changes it merge
without a conflict into a mark over a text nobody reviewed. With the
fingerprint that mark no longer holds, so a review never passes to a text
nobody reviewed, whatever merged it; the editor says the translation changed
after its review, and machine translation may change it again. A hand edit
of the file does the same.

- A person's edit of a reviewed translation in Aeria, in the editor or by a
  bulk replacement, keeps it reviewed: the mark takes the new text's
  fingerprint.
- A review confirms the translation for its source as it is, so marking a
  fuzzy translation reviewed also clears `fuzzy`.
- A game update that changes the source keeps the mark with the
  translation, which becomes fuzzy: it stays the person's while they look
  at it again.
- Clearing a translation removes its mark, and machine translation writing
  over a mark of another text removes that mark.

### Identity

A string of the game is a cell: a sheet (a table such as `BNpcName`), a row
ID, a subrow (for the few sheets whose rows have several subrecords; 0
otherwise), and a column (the position of the String column in the row, such
as 0 for the singular and 2 for the plural of an NPC name). In data sheets
the row ID is the ID of a game object and does not change. In quest and
cutscene dialogue the row ID is only the line's position, and inserting a line
renumbers the rest; there the game stores a stable key in each row (see
[`source.md`](./source.md#row-keys)).

`msgctxt` is therefore:

- in a sheet with a row key column whose keys are all non-empty and free of
  `:`: `sheet:key:column`;
- in any other sheet: `sheet:row:subrow:column`.

No other identity is derived or stored. See [Stability data](#stability-data)
for how the game keeps these across patches.

## Why PO

The format must let people edit one string at a time with ordinary tools,
carry context next to each string, merge line by line in Git without false
conflicts, and hold macro text, which is full of `<` and `>`.

- **gettext PO** keeps one entry per string with comments above it; merges
  line by line; has the marks this model needs as part of the format (`fuzzy`,
  the previous source `#|`, obsolete entries `#~`); is known to every model
  from countless projects; and opens in PO editors and translation platforms.
  Its cost is escaping `\"` and `\\` inside strings.
- **XLIFF** is XML: every `<` of a macro would be written `&lt;`.
- **JSON** has no comments and turns commas and escaping into conflicts.
- **YAML** is familiar, but quoting and indentation errors are common, and a
  colon in the text changes the meaning of a line.
- **A format of our own** is known to no tool and no model.

## Editing

The desktop opens a project with the installed game (`aeria_po::Session`):

- **Opening** reads `aeria.json` and requires the game's source language and
  version. A game older than the project is refused; a newer one asks for a
  [game update](#game-updates). Opening never writes.
- **Reading** a sheet takes its rows from the game and the translations of
  their strings from the sheet's files, parsed when first read and read again
  only when a file's size or time changed. Rows come in pages of at most 256
  source rows; a row with no string of the project is left out, and its
  other non-empty cells are returned as context.
- **Saving** a string reads its file again, checks the translation (see
  [Checking](#checking)), sets its `msgstr` or note, clears `fuzzy` and the
  previous source, and replaces the file through a temporary file and a
  rename. A translation with a problem is refused and nothing is written. An
  entry whose `msgid` is not the game's text refuses the save: the project
  needs a game update. An empty translation leaves the string untranslated.
  Saves are made one at a time.
- **Changes made elsewhere**, by Git, by hand, or by a machine translation
  run, are seen on the next read. The desktop checks the files of the sheets
  it has shown every 1.5 seconds and reloads a sheet whose file changed.
- **Progress** per sheet counts entries, translated entries, and fuzzy
  entries; the first count reads every file, later ones only changed files.

A person's decision wins through Git (principle 5): any translation can be
changed, and no change reaches the project's history without the person
committing it in Aeria or merging it, where the diff shows it; every change
can be reverted. Git is ordinary Git: no merge driver, no hooks. Two people
who translate different strings of one file usually merge cleanly, because
unchanged `msgctxt`, `msgid`, and comment lines separate their edits; when Git
reports a conflict in a PO file, Aeria joins the file per string (see
[`git.md`](./git.md#per-string-merge)).

A project is created from the installed game with a target language: every
file of `po/` with empty translations, `aeria.json`, and the knowledge files
without entries (about a minute for the whole game).

## Checking

The checks are Aeria's own; they run when a translation is saved, on every
answer of [machine translation](./translate.md), and in CI (`aeria-check`,
see [`git.md`](./git.md#merge-check-ci)). A translation has a **problem**
when it:

- breaks the assisted structure policy of [`strings.md`](./strings.md): the
  source's macros, as game data, must stay;
- has a line break its source does not have (the game breaks lines with
  `<br>`);
- uses a forbidden variant of a term of its source (see
  [`knowledge.md`](./knowledge.md));
- for Russian, writes both genders at once (`готов(а)`);
- for a language written in Cyrillic, has a slip of letters the source does
  not have: a stress or other combining mark (`эле́зен`), a word that mixes
  Cyrillic with Latin or Greek letters (`Танalanе`), or letters of another
  writing system (`цели无属性`). A letter the source spells with, such as the
  æ of Pandæmonium, is not a slip.

Each problem and piece of advice is an `Issue` with its data and its English
message, so an interface can word it in its own language (see
[`search.md`](./search.md#project-search)).

Advice does not make a translation wrong: a term whose translation does not
seem to be used, a word written twice in a row with only spaces between in
either reading of the conditions (each condition's first branch, or each
one's last; not a word the source repeats, nor a name written twice with
capitals, as a Lalafell's: Гун Гун), a condition on the player character's gender that may be
missing, machine phrasing, a term exception that names no term of the
source, an interface label or a name of a world object longer than its
budget (see [`translate.md`](./translate.md#interface-labels) and
[names in the world](./translate.md#names-in-the-world)). Advice is for the
person who reviews a translation: saving, export, `aeria-check`, and the
checks of machine translation ignore it, and it is sent to a model only
when a person asks machine translation to correct a translation with it (see
[`translate.md`](./translate.md#correcting-translations)); machine
translation holds its own answers to the length budget. A file has
a problem when a line breaks the PO
format or is a Git conflict marker, or a `msgctxt` is not an identity or
appears twice.

A translation is *valid* when its entry passes these checks. Invariant: only
valid translations whose entry is not `fuzzy` are exported. A fuzzy
translation stays in its file as visible work and never reaches players.

## Game updates

A game update brings the project from the game version its files were made
for to the installed one. It is deterministic, refuses to run while `po/` or
`aeria.json` has uncommitted changes, and produces one commit, "Update to
game version …", so everything it did is visible in the diff and can be
reverted. The desktop offers it when it opens a project for an older version.

1. Aeria makes every file again from the installed game.
2. It joins each entry of the previous files to the new entry with the same
   `msgctxt`, across all files, so a sheet whose file is split or joined
   keeps every translation:

   | Previous entry | New entry with the same `msgctxt` | Result |
   | --- | --- | --- |
   | same `msgid` | exists | translation, notes, and marks are kept |
   | other `msgid` | exists | translation, notes, and review mark are kept; `#, fuzzy` and `#\| msgid` with the previous source; term exceptions are dropped |
   | any | none | the entry becomes obsolete (`#~`) at the end of its file, with its translation and notes |
   | none | exists | a new entry with an empty `msgstr` |

   An obsolete entry goes into the new file of its previous path, else the
   first file of its sheet, else a file of its previous path that keeps only
   obsolete entries (a sheet the game no longer has). An obsolete entry whose
   `msgctxt` exists again in a later version joins as above.
3. `X-Game-Version` and `aeria.json` `gameVersion` are set to the installed
   version.

Running it again on the same game changes nothing. A fuzzy entry keeps its
previous `#| msgid` until its translation is saved, which removes the mark;
the editor shows the old source beside the new one. Nothing else happens: no
text similarity, no moving translations between rows, no column mapping. A
string whose meaning moved (a renumbered list, a reordered column) becomes
fuzzy and is fixed as ordinary work; since fuzzy entries are not exported, no
player sees a translation meant for another string.

## Earlier projects

There is no migration. A project of the earlier format has no `aeria.json`
and is reported as not a project; its translations are started again in a
new project.

## Stability data

The English sheets of `xivapi/ffxiv-datamining` for eight consecutive game
versions, 7.41 to 7.56, about one million text cells each:

| Update | Text changed at the same ID | Rows removed | Text moved to another ID | Sheets whose column count changed |
| --- | --- | --- | --- | --- |
| 7.41 | 92 | 6 | 0 | 0 |
| 7.45 | 521 | 1 | 0 | 1 |
| 7.50 | about 8,000 | 743 | 7,574 | 31 |
| 7.50h1 | 3 | 0 | 0 | 2 |
| 7.51 | 367 | 2 | 0 | 0 |
| 7.55 | 675 | 78 | 7 | 1 |
| 7.55h2 | 0 | 0 | 0 | 0 |
| 7.56 | 53 | 16 | 1 | 1 |

Keyed dialogue sheets were compared by key. At 7.50 the export tool changed
how it writes macros, so about 30,000 apparent changes in dialogue are not
changes of the game and are left out; part of the 8,000 in other sheets may
be the same. Of the moves at 7.50, 7,335 are `CompleteJournal` and 195
`JournalGenre`, list sheets the game renumbered; the rest are single digits
per sheet. Most column count changes are in numeric columns.

## Open question

- Translation memory over the files of `po/` (`aeria-search` indexes the
  game only; project search reads the files, see
  [`search.md`](./search.md#project-search)).
