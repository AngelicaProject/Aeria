# The project as PO files

Status: **proposal, not implemented.** When implemented it replaces Workspace
Format v3 (`.aeria/`), the translation unit identity of
[`identity.md`](./identity.md), the source update rules of
[`rebase-safety.md`](./rebase-safety.md), the `aeria-units` merge driver of
[`git.md`](./git.md), review states, the agents' ledger, and the `game/`
corpus view of [`agents.md`](./agents.md#the-corpus). Those documents describe
the current system until then.

## Why

Agents localize the game best when they work on it the way they work on a
codebase: files they read, search, and edit with their own tools, and a
checker that reports problems by file and line. The `game/` corpus showed it.
It also showed the weakness of a corpus that is only a view: the project's
truth stayed in `.aeria/units`, the files agents edited were kept in step with
it by a two-way synchronization (a baseline, digests, file times, a watcher),
and `git diff` showed changes the agent had not made. An agent took them for
side effects of the check and reset `.aeria/units` with `git restore`, which
discarded every uncommitted translation.

Principle 7 of [`../product/principles.md`](../product/principles.md) asks that
friendly operations and Git work on the same project state. This proposal
makes the PO files that state: one truth, edited by agents, people, the
desktop, and Git alike.

Review states and authorship marks go too. They were made for people
translating with a machine's help. When agents do most of the work, Git
answers the same questions better: who changed a string and when is the
history and `git blame`, what changed is the diff, and accepting work is
committing it.

## The project

A project is a Git repository:

```text
aeria.json                  project settings: languages, pack, fonts
po/                         the game's text and its translation, as PO files
  README.md                 what agents need to know, written by Aeria
  quest/000/ClsArc000_00021.po
  BNpcName/003.po
  ...
aeria-knowledge/            project knowledge, unchanged
fonts/, AGENTS.md, CLAUDE.md, .github/workflows/, ...
```

Every translatable string of the game is an entry in `po/`, translated or
not. There is no `.aeria/` directory, no separate store of translations, and
no state outside the files and Git. `aeria.json` holds the source and target
languages and what `aeria-pack.json` and `aeria-fonts.json` hold today.

The repository holds the game's text in the source language and the evidence
languages (Japanese, English, German, French, as the client ships them). For
Project Prima that is about 427 MB of files, about 111 MB compressed in Git; a
patch adds the changed files. Public community repositories already publish
the complete text of the game in every client language.

### Files

- A quest (`quest/…`) or cutscene (`cut_scene/…`) sheet is one file, its
  entries in play order.
- Any other sheet is one file when its row IDs are below 1,000, otherwise a
  folder of files by row ID range: `BNpcName/003.po` holds rows 3000–3999.
  A range is a range of IDs, not of positions, so an added row never moves
  another row to a different file.
- A path segment that Windows reserves (`CON`, `AUX`, `NUL`, `COM1`, …) or
  that differs from another only by case gets the suffix `~` and, for a case
  collision, the number of its place in the sheet list. The game has no such
  sheet today; the rule keeps a checkout possible on every system.

A file starts with a header entry:

```po
# quest/000/ClsArc000_00021 — «Way of the Archer» · 44 strings in play order
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
| `# ` lines | translator notes | people and agents |
| `#, fuzzy` and `#\| msgid` | a game update changed the source; `#\| msgid` is the source the translation was written for | Aeria, in a game update |
| `msgctxt` | the entry's identity; see [Identity](#identity) | Aeria |
| `msgid` | the source text as macro text ([`strings.md`](./strings.md#macro-text)) | Aeria |
| `msgstr` | the translation as macro text; empty while there is none | people and agents |

`fuzzy` is the only mark. It states a fact of the game, not an opinion of a
translator: the source changed after the translation was written. Anything a
translator wants a person to decide goes into a note and to the user.

Entries are ordered as the game orders them: play order in a scene, row and
column order elsewhere. The `#.` lines are derived only from the game, so
every collaborator with the same game version produces the same bytes for
them and Git never sees them conflict.

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

- in a sheet with a row key column: `sheet:key:column`;
- in any other sheet: `sheet:row:subrow:column`.

No other identity is derived or stored. See [Stability data](#stability-data)
for how the game keeps these across patches.

## Why PO

The format must let agents and people edit one string at a time with ordinary
tools, carry context next to each string, merge line by line in Git without
false conflicts, and hold macro text, which is full of `<` and `>`.

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

Agents and people edit `msgstr` and notes with any tool. The desktop edits the
same files: it re-reads a file before each save, changes only the entry being
edited, and replaces the file atomically; it watches `po/` and shows edits
made elsewhere, with each entry's changes since the last commit and its
history from Git. Git is ordinary Git: no merge driver, no hooks. Two people
who translate different strings of one file merge cleanly, because unchanged
`msgctxt`, `msgid`, and comment lines separate their edits; two people who
translate the same string get a conflict in that entry, which `aeria check`
reports until it is resolved.

A person's decision wins through Git (principle 5): any translation can be
changed, and no change reaches the project's history without the person
committing it in Aeria or merging it, where the diff shows it; every change
can be reverted.

## Checking

`aeria check` is a linter over `po/` and `aeria-knowledge/`; it changes no
file. It reports, each as `file:line: what to fix`:

- a line that breaks the PO format, or a Git conflict marker;
- a `msgctxt` that the installed game does not have, or a `msgid` that is not
  its current text;
- a translation that breaks the assisted structure policy of
  [`strings.md`](./strings.md), uses a forbidden variant of a term of its
  source, has a line break its source does not have, or, for Russian, writes
  both genders at once;
- a problem of the knowledge files.

Advice that does not make a translation wrong (a term whose translation does
not seem to be used, a gender condition that may be missing, machine
phrasing) is listed separately. Exit status 1 when there are problems.

A translation is *valid* when its entry passes these checks. Invariant: only
valid translations whose entry is not `fuzzy` are exported. An invalid or
fuzzy translation stays in its file as visible work and never reaches players.

## Game updates

A game update brings the project from the game version its files were made
for to the installed one. It is deterministic, runs only on a working tree
without uncommitted changes, and produces one commit, so everything it did is
visible in the diff and can be reverted.

1. Aeria makes every file again from the installed game.
2. It joins each entry of the previous files to the new entry with the same
   `msgctxt`:

   | Previous entry | New entry with the same `msgctxt` | Result |
   | --- | --- | --- |
   | same `msgid` | exists | translation, notes, and marks are kept |
   | other `msgid` | exists | translation and notes are kept; `#, fuzzy` and `#\| msgid` with the previous source |
   | any | none | the entry becomes obsolete (`#~`) at the end of its file, with its translation and notes |
   | none | exists | a new entry with an empty `msgstr` |

   An obsolete entry whose `msgctxt` exists again in a later version joins as
   above. A file of a sheet the game no longer has keeps only obsolete
   entries.
3. `X-Game-Version` is set to the installed version.

A fuzzy entry keeps its previous `#| msgid` until its translation changes; the
change removes the mark. A person who finds the translation still right
removes the mark in Aeria. Nothing else happens: no text similarity, no
moving translations between rows, no column mapping. A string whose meaning
moved (a renumbered list, a reordered column) becomes fuzzy and is fixed as
ordinary work; since fuzzy entries are not exported, no player sees a
translation meant for another string.

After a merge of branches made for different game versions, files carry
different `X-Game-Version` values; the same update brings the older files to
the installed version, and the result does not depend on the order of the
merges.

## Earlier projects

There is no migration. A project in Workspace Format v3 is reported as
unsupported; its translations are started again in a new project.

## What goes away

- `.aeria/` and Workspace Format v3, translation unit IDs, layouts, statuses,
  detach reasons, and review states;
- the source update planner of `aeria-rebase` with its column mapping and
  binding conflicts;
- the `aeria-units` merge driver and the merge check it needed;
- the agents' ledger, stamp, and write lock, and the `game/` view with its
  baseline, digests, and watcher;
- `aeria corpus`: `po/` is the project.

## What agents are told

`AGENTS.md` and `po/README.md` say, besides the layout and the rules of a
translation:

- the files in `po/` are the project; edit `msgstr` and notes, nothing else;
- run `aeria check` after editing and fix what it reports;
- `#, fuzzy` marks strings a game update changed; bring the translation in
  line with the new source;
- `git diff` shows your work; never run `git restore`, `git checkout`,
  `git reset`, `git clean`, or `git stash` on project files, and do not
  commit or push unless asked: the user reviews and commits in Aeria;
- ask the user about matters of taste and anything you are not sure to change.

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

- How the desktop's search and translation memory read the files (today they
  read the workspace).
