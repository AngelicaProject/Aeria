# Localization with agents

Aeria has no built-in model or agent. A project is localized by external agent
harnesses that can run commands, which plan the work, split it among their own
subagents, and translate. Aeria gives them the game the way a codebase is given
to them: the project is files they read, search, and edit with their own tools,
kept in Git, and one command checks them.

- the **project's PO files** in `po/`: every string of the game with the other
  client languages, speakers, and macro legends, and its translation, and
  `po/README.md` with the layout, the rules, and working with Git (see
  [`po-project.md`](./po-project.md));
- the **project knowledge**: files in the project that say how the project
  translates (see [Project knowledge](#project-knowledge));
- the **rules of a translation** every string follows, whoever writes it (see
  [Translation rules](#translation-rules));
- the **`aeria` command**, which makes a project, checks its files, and brings
  them to a new game version (see [The `aeria` command](#the-aeria-command)).

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
reach agents through `po/README.md`, which also carries the macro
authoring reference of [`strings.md`](./strings.md) and the layout of the
knowledge files.

## The `aeria` command

`aeria` is a console command that people and agents run in a project
directory. It is the `aeria-cli` binary of the desktop package
(`apps/desktop/src-tauri/src/cli`), which shares the desktop's game detection;
it opens no window. It finds the project root by walking up from the current
directory (or `--project`) to the directory with `aeria.json`. Output is
compact text by default and one JSON value with `--json`. The tool and every
command have `--help`. Exit status: 0 done, 1 done with problems, 2 error.

| Command | Purpose |
| --- | --- |
| `init --language <tag>` | Makes the directory a project: `aeria.json`, `po/` with every translatable string of the installed game, `po/README.md`, `aeria-knowledge/` with its files and no entries, `.gitattributes` keeping PO files LF, and Aeria's sections of `AGENTS.md` and `CLAUDE.md`; makes it a Git repository if it is not one and commits the project. In an existing project it only adds the files around `po/` that are missing, rewrites Aeria's sections and `po/README.md`, and commits those. About a minute for the whole game (7,337 files, 430 MB; 117 MB in Git). |
| `check [--all]` | The linter of [`po-project.md`](./po-project.md#checking) over the files of `po/` changed since the last commit (every file with `--all`, or outside a repository) and over the project knowledge; writes `po/README.md` when its text changed and no other file. About 1 s for a few files and 10 s for the whole game. |
| `update` | The game update of [`po-project.md`](./po-project.md#game-updates): refuses while `po/` or `aeria.json` has changes that are not committed, makes every file again, carries the translations over, and commits the result as one commit. |

Everything else an agent does with its own tools on the files: searching the
game, counting what is left, comparing translations across the project.

### The project server

A command is a thin client of the project's server, a background process of
the same executable (`__serve`) that keeps the installed game and the
knowledge open between commands.

- The first command for a project starts the server and waits until it has
  opened the game; `agents/<key>.server` in application data names its
  loopback port, a random token every request carries, its process, and the
  build of the executable. A command from another build makes the old server
  stop and starts a new one, and so does a request after the game on disk was
  updated. A server stops after 15 minutes without requests. One command at a
  time starts a server (`agents/<key>.spawn` is locked while it does).
- The server runs from a copy of the executable in `agents/`, so a rebuild or
  an update can always replace `aeria`. It is started through
  `Start-Process` on Windows: a process started directly would inherit the
  command's standard output and error, and the agent's harness, which reads
  them until they close, would wait for the server to exit.
- The server's working directory is `agents/`, never the project, so the
  project folder can be moved or deleted while a server runs.
- `init` never uses a server. Help and version need none. When no server can
  be started, and with `AERIA_NO_SERVER` set, a command opens the project
  itself.

The desktop does not open projects of this format yet; until it does, a
project is edited through its files and Git.

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
  `AGENTS.md` (what the project is, that `po/` is the project, how `aeria
  check` is used, and which Git commands never to run on it) and
  `CLAUDE.md` (which imports `AGENTS.md` for Claude Code), keeping any other
  text. Checkpoints commit both files, so collaborators and their agents get
  them.

Aeria installs no skills. What an agent needs to know is in `po/README.md`,
which `aeria check` keeps current, so it always matches the installed version,
and `AGENTS.md` points there. Connecting removes the
`aeria-localization` skill earlier versions put into harnesses' skill folders,
where the file is still the one they wrote. Skills a project writes for
itself go into `.agents/skills/` (Codex, Hermes Agent) or `.claude/skills/`
(Claude Code) and are committed with the project like its knowledge.
