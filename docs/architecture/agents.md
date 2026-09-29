# Localization with agents

Aeria has no built-in model or agent. A project is localized by external agent
harnesses, such as Claude Code, Codex, or Hermes Agent, which plan the work,
split it among their own subagents, and translate. Aeria gives them three
things:

- the **project knowledge**: files in the project that say how the project
  translates (see [Project knowledge](#project-knowledge));
- the **rules of a translation** every string follows, whoever writes it (see
  [Translation rules](#translation-rules));
- the **`aeria` command**, which reads the game and the project for an agent
  and writes checked translations (see [The `aeria` command](#the-aeria-command)).

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
reach agents through `aeria brief`.

## The `aeria` command

`aeria` is a console command that an agent runs in a project directory. It is
the `aeria-cli` binary of the desktop package (`apps/desktop/src-tauri/src/cli`),
which shares the desktop's game detection, sheet catalog cache, and
workspace code; it opens no window. It works on the project files directly;
the desktop does not need to be open. It finds the project root by walking up
from the current directory (or `--project`) to the directory with `.aeria/`,
and the game installation, caches, and application data the desktop uses.
Output is compact text by default and one JSON value with `--json`. The tool
and every command have `--help`. Exit status: 0 done, 1 done with rejected
or failed translations or knowledge problems, 2 error.

### The project server

A command is a thin client of the project's server, a background process of
the same executable (`__serve`) that keeps the game, the workspace, and the
caches open: the sheets' strings, the other client languages, the knowledge
(read again when a file changes), the search index with its word counts, and
similar strings per source text. With a running server a command takes about
30 ms; `read` of a 126-line quest with similar translations not yet looked up
takes about 0.6 s, as the lookups run on four threads, and a batch of 100
translations is written in about 0.4 s.

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
- Reads run in parallel. Writes take the project's write lock and run one at
  a time. Before each request the server compares the stamp with the last one
  it took in and reloads the workspace when another process wrote.
- Help and version need no server. When no server can be started, and with
  `AERIA_NO_SERVER` set, a command opens the project itself.
- `AERIA_TRACE` prints how long opening the project takes.

| Command | Purpose |
| --- | --- |
| `guide` | How the project is organized, the knowledge files, the commands, and how to work on a large scope with several agents. |
| `brief` | The [translation rules](#translation-rules), the project's languages, the macro authoring reference of [`strings.md`](./strings.md), and how to write with the command, for every agent that translates. |
| `overview` | Languages, game version, progress, and areas of the project (quests, cutscenes, and the non-dialogue domains), and the state of the knowledge; with a pattern (`quest/*`, `item`), the matching sheets and their progress; `--folders` groups sheets by folder. |
| `read <sheet>` | A scene: the translatable strings of a quest or cutscene in the order of its dialogue, or of another sheet in row order, with `--rows` and `--untranslated` to narrow it. Each string has its address `@sheet:row:subrow:column`, its kind or speaker, its state (untranslated, or whose translation it is and its review state; `(keep)` marks one an agent may not replace), the source, the other client languages, the legends of its macros, the row's other cells, a mark where the French or German line varies with the player character's gender and the source does not, and, for strings an agent may write, up to two similar translated strings from the search index when it exists. Before the lines comes the knowledge slice they need: style of their domains, lessons, the terms in them, their speakers' voices, and the story of the sheet. |
| `write` | Writes translations from a file, standard input, or `--at`/`--text`: blocks of an `@address` line and the translation, or JSON Lines. See the invariants below. `--needs-review` marks them as needing review; otherwise they are drafts. |
| `check` | The checks of `write`, without writing. |
| `find <text>` | Strings whose source (the default; the first search builds the search index of the game version) or translation (`--in translation`) contains the text, ignoring case and tags; `--sheet` takes a pattern. |
| `knowledge` | Checks the knowledge files and lists every problem with its file and line. |

Not implemented yet: `audit` (project-wide deterministic checks such as
inconsistent terms or address), `changes` (what a game update changed), and
`flag` (mark strings for a person).

### Writing

- Every translation is checked before anything is stored. It is rejected when
  it is empty, has a line break its source does not have (the game's line
  break is `<br>`), breaks the assisted structure policy of
  [`strings.md`](./strings.md), uses a forbidden variant of a term of its
  source, or, for Russian, writes both genders at once such as `готов(а)`.
  The write itself is compare-and-set against the state the command read. An
  invalid or structurally unsafe translation is never stored.
- A translation that passed its checks but could not be saved, for example
  because the disk refused the write, is reported as failed rather than
  rejected; writing it again may succeed. A stamp that cannot be replaced
  after a write is a warning: the translations are written.
- Advice does not block a write: a term of the source whose translation does
  not seem to be used, a source that varies with the player character's
  gender and a translation that does not, a French or German line that varies
  where the source does not, and `machine_phrasing`.
- An agent writes an untranslated string or replaces a translation an agent
  wrote that is not reviewed. A translation a person wrote or changed, and a
  reviewed one, is skipped and reported. Which translations agents wrote is
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
- A command that wrote translations, and the desktop after each of its
  writes, replaces the stamp before releasing the lock. The desktop compares the stamp with the one its session took in,
  before each of its writes and every 1.5 seconds; after a change it reloads
  the workspace under the lock and emits `project://workspace-reloaded`, on
  which the editor reloads the open sheet and the progress.
- The ledger is SQLite (the `written` table: location, target, time).

Git operations in the desktop do not take the lock; synchronizing while agents
write can make the desktop's next write fail on the changed workspace until it
reloads.

Identity, source updates, merges, and export do not depend on agents.

Discovery (not implemented yet): Aeria writes `AGENTS.md` and `CLAUDE.md` at
the project root, which agent harnesses read, and can install an
`aeria-localization` skill for harnesses that use skills and put `aeria` on
`PATH`.
