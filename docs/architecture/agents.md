# Localization with agents

Aeria has no built-in model or agent. A project is localized by external agent
harnesses that can run commands, which plan the work, split it among their own
subagents, and translate. Aeria gives them the game the way a codebase is given
to them: files to read, search, and edit with their own tools, and one command
that checks and saves the edits.

- the **corpus**: the whole game text in `game/` as gettext PO files, with the
  other client languages, speakers, and macro legends, and `game/README.md`
  with the layout and the rules (see [The corpus](#the-corpus));
- the **project knowledge**: files in the project that say how the project
  translates (see [Project knowledge](#project-knowledge));
- the **rules of a translation** every string follows, whoever writes it (see
  [Translation rules](#translation-rules));
- the **`aeria` command**, which makes the corpus and saves checked
  translations from it (see [The `aeria` command](#the-aeria-command)).

Aeria never distributes or schedules work; a harness decides what to
translate, in which order, and with how many agents.

## Project knowledge

The knowledge is the `aeria-knowledge` directory at the project root:
`style.md` (how each kind of text reads), `terms.csv` (terms every
translation renders the same way), `characters.md` (how characters speak),
`story.md` (what happened so far, per quest or cutscene sheet), and
`lessons.md` (recurring problems and what to do instead). The format is
[Project Knowledge Format v1](../formats/knowledge-v1.md). `aeria-knowledge`
owns reading, checking, and selecting it.

The knowledge is project documentation, like `docs/` of a code repository:
people and agents read and edit the files directly, and a checkpoint commits
them with the translations (see [`git.md`](./git.md#checkpoints)). There is one
layer. An entry a person decided is marked `settled`; agents follow it and do
not change it without asking a person.

The desktop edits the terms, the style, and the character voices in the
project knowledge dialog (see
[`desktop-editor-ui.md`](./desktop-editor-ui.md#project-knowledge-dialog)). A
term a person edits there is marked settled.

A file that cannot be read, and entries the format excludes, are reported
with their file and line and left out; the rest of the knowledge is used.

## Translation rules

`aeria_knowledge::rules` holds the rules every translation follows:

- what the game's texts are: the Japanese original and three finished
  localizations, what each is evidence for, and that content comes only from
  the source line;
- how a line reads as if written in the target language, and the known causes
  of dry, machine-written text;
- how the player character is addressed without assuming a gender, with
  conditions on `$gn4` in gendered languages;
- how translations are written as macro text;
- for Russian, notes on living language and `machine_phrasing`, a word-level
  check for officialese, bookish links, calques, and stacked explanations.

The project knowledge takes precedence over the style defaults. The rules
reach agents through `game/README.md`, which also carries the macro
authoring reference of [`strings.md`](./strings.md) and the layout of the
knowledge files.

## The `aeria` command

`aeria` is a console command that an agent runs in a project directory. It is
the `aeria-cli` binary of the desktop package (`apps/desktop/src-tauri/src/cli`),
which shares the desktop's game detection, sheet catalog cache, and
workspace code; it opens no window. It works on the project files directly;
the desktop does not need to be open. It finds the project root by walking up
from the current directory (or `--project`) to the directory with `.aeria/`,
and the game installation, caches, and application data the desktop uses.
Output is compact text by default and one JSON value with `--json`. The tool
and every command have `--help`. Exit status: 0 done, 1 done with
translations not saved or knowledge problems, 2 error.

| Command | Purpose |
| --- | --- |
| `corpus [<pattern>]` | Writes the game text into `game/` as PO files; see [The corpus](#the-corpus). `--force` also replaces files with unsaved changes. |
| `check` | Saves the translations changed in `game/`, reports every problem as `file:line`, including problems of the knowledge files, and brings `game/` up to date; see [The corpus](#the-corpus) and [Saving](#saving). |
| `init` | Writes Aeria's section into `AGENTS.md` and `CLAUDE.md` at the project root (see [Discovery](#discovery)). |

Everything else an agent does with its own tools on the files: searching the
game, counting what is left, comparing translations across the project.

### The corpus

`aeria corpus` writes the whole game text into `game/` in the project as
gettext PO files (`apps/desktop/src-tauri/src/cli/corpus.rs`), so agents read,
search, and translate it with the file tools they already use, the way they
work on source code. A quest or cutscene is one file in play order; another
sheet is one file, or a folder of files of 200 strings in row order, such as
`game/BNpcName/001.po`. Each string is an entry: `msgctxt` is its address,
`msgid` its source, `msgstr` its translation (empty while there is none), and
extracted comments hold the other client languages, the speaker or kind, the
row's other cells, and the legends of its macros; `#, keep` marks a translation
an agent may not replace. `game/README.md` describes the layout, how the
files are saved, the knowledge files, and how to work with the user, followed
by the [translation rules](#translation-rules).

- `game/` is the game's text: `aeria corpus` adds `/game/` to the project's
  `.gitignore`, and checkpoints never commit it. It is made from the installed
  game and the project. Files are rewritten only when their text changes, and
  making the whole corpus removes the files of sheets the game no longer has.
- `game/.aeria-state.json` keeps the game version the corpus was made for;
  for every translated string, the translation its file shows and a digest of
  how it shows it (translation, author, review state, note); and for each
  file its size and time when Aeria last read or wrote it, the game version
  it was made for, and whether it has problems left.
- `aeria check` reads the files that changed since, and those with problems
  left, and saves every `msgstr` that differs from what its file shows
  through the checks and the compare-and-set write of [Saving](#saving). Each
  problem is reported as `game/<file>:<line>: <what to fix>`: a line that
  breaks the PO format (the rest of that entry is skipped, the other entries
  are read), a rejected or skipped translation, a translation removed in the
  file, a `msgid` that is not the game's current text of that address (the
  file was made before a game update), and a translation that changed in the
  project after the file was made. Such a translation is never saved: a
  translation lands only on the text it was written for, and never replaces
  one made elsewhere meanwhile. A translation that was not saved stays in its
  file and is reported by the next check. `check` never reads standard input,
  which a harness may leave open.

#### Keeping `game/` current

`game/` always shows the project and the installed game. After every check,
through the project server or without one, and while a server runs, every 2 seconds after another process
such as the desktop wrote the project, Aeria rewrites the files whose strings
changed since they were made: a translation, its author, review state, or
note, detected from the digests without reading the files. Only files the
corpus has are rewritten; `aeria corpus` decides what it holds. After a game
update, once the project itself has been updated (see
[`rebase.md`](./rebase.md)), the first of these makes the whole corpus again
for the new version. A server that can no longer take the project in, as
after a game update, exits, and the next command starts one on the new
version.

A file with changes not saved yet is never replaced, by these updates or by
`aeria corpus` unless `--force` is given. A file kept through a game update
that way is made again for the new version once its problems are gone.

When a file is made again, a translation whose `msgid` changed from the one
the previous file showed, and that waits for review, is marked `#, fuzzy`
with the previous source as `#| msgid`, as `msgmerge` does. The source
update marks such translations `needs-review`; the previous source comes from
the file, since the workspace keeps only the current one. The mark stays
while the translation waits for review and goes once it changes or a person
reviews it.

### The project server

A command is a thin client of the project's server, a background process of
the same executable (`__serve`) that keeps the game, the workspace, and the
caches open: the sheets' strings, the other client languages, and the
knowledge (read again when a file changes). With a running server, `check`
of a project whose corpus holds the whole game (9,474 files) takes about 2 s,
most of it looking at every file's size and time; making the whole corpus
takes about 40 s.

- The first command for a project starts the server and waits until it has
  opened the project (about 2 s for a project of 25,000 translations);
  `agents/<key>.server` in application data names its loopback port, a
  random token every request carries, its process, and the build of the
  executable. A command from another build makes the old server stop and
  starts a new one. A server stops after 15 minutes without requests. One
  command at a time starts a server (`agents/<key>.spawn` is locked while it
  does).
- The server runs from a copy of the executable in `agents/`, so a rebuild or
  an update can always replace `aeria`. It is started through
  `Start-Process` on Windows: a process started directly would inherit the
  command's standard output and error, and the agent's harness, which reads
  them until they close, would wait for the server to exit.
- The server's working directory is `agents/`, never the project, so the
  project folder can be moved or deleted while a server runs.
- Reads run in parallel. Writes take the project's write lock and run one at
  a time. Before each request the server compares the stamp with the last one
  it took in and reloads the workspace when another process wrote.
- Help and version need no server. When no server can be started, and with
  `AERIA_NO_SERVER` set, a command opens the project itself.
- `AERIA_TRACE` prints how long opening the project takes.

### Saving

- Every translation is checked before anything is stored. It is rejected when
  it is empty, has a line break its source does not have (the game's line
  break is `<br>`), breaks the assisted structure policy of
  [`strings.md`](./strings.md), uses a forbidden variant of a term of its
  source, or, for Russian, writes both genders at once such as `готов(а)`.
  The write itself is compare-and-set against the state the command read. An
  invalid or structurally unsafe translation is never stored.
- A translation that passed its checks but could not be saved, for example
  because the disk refused the write, is reported as failed rather than
  rejected; the next check tries it again. A stamp that cannot be replaced
  after a write is a warning: the translations are written.
- Advice does not keep a translation from being saved: a term of the source whose translation does
  not seem to be used, a source that varies with the player character's
  gender and a translation that does not, a French or German line that varies
  where the source does not, and `machine_phrasing`.
- An agent saves an untranslated string or replaces a translation an agent
  wrote that is not reviewed. A translation a person wrote or changed, and a
  reviewed one, is marked `#, keep` in its file and is not saved when changed
  there. Which translations agents wrote is
  kept per project in a ledger (see
  [Sharing a project between processes](#sharing-a-project-between-processes)):
  a translation is an agent's while its text is the one recorded, so a
  person's later edit makes it theirs. Translations written before the ledger
  existed count as a person's.

### Sharing a project between processes

Per project, keyed by a hash of its canonical root, application data holds
`agents/<key>.lock`, `agents/<key>.stamp`, and `agents/<key>.sqlite3`:

- A write takes an exclusive lock on the lock file for its whole run: the
  server around each write, a command without a server before it opens the
  project, and the desktop around each translation, note, and review change. Writes of parallel agents and of the desktop never
  interleave, and each starts from the workspace the previous one left.
- A check that saved translations, and the desktop after each of its
  writes, replaces the stamp before releasing the lock. The desktop compares the stamp with the one its session took in,
  before each of its writes and every 1.5 seconds; after a change it reloads
  the workspace under the lock and emits `project://workspace-reloaded`, on
  which the editor reloads the open sheet and the progress.
- The ledger is SQLite (the `written` table: location, target, time).

Git operations in the desktop do not take the lock; synchronizing while agents
write can make the desktop's next write fail on the changed workspace until it
reloads.

Identity, source updates, merges, and export do not depend on agents.

## Discovery

Agent harnesses find the command in two ways; Settings → Agents →
**Connect agents** in the desktop sets up both and shows their state:

- **The command on `PATH`.** The installer and the portable build ship the
  command as `bin/aeria.exe` beside the application (a development build has
  `aeria-cli.exe` there). Connecting copies it to
  `%LOCALAPPDATA%\Aeria\bin\aeria.exe` (`~/.local/bin/aeria` on Linux) and adds
  that folder to the user's `PATH`; agents started afterwards find `aeria`. A
  desktop that starts after an update refreshes the copy when agents were
  connected.
- **Project files.** `aeria init`, and connecting with a project open, write
  Aeria's section between `<!-- aeria:begin -->` and `<!-- aeria:end -->` into
  `AGENTS.md` (what the project is, that the game text is in `game/`, and how
  `aeria check` saves translations) and
  `CLAUDE.md` (which imports `AGENTS.md` for Claude Code), keeping any other
  text. Checkpoints commit both files, so collaborators and their agents get
  them.

Aeria installs no skills. What an agent needs to know is in `game/README.md`,
which Aeria writes with the corpus, so it always matches the installed version,
and `AGENTS.md` points there. Connecting removes the
`aeria-localization` skill earlier versions put into harnesses' skill folders,
where the file is still the one they wrote. Skills a project writes for
itself go into `.agents/skills/` (Codex, Hermes Agent) or `.claude/skills/`
(Claude Code) and are committed with the project like its knowledge.
