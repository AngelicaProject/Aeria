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
reach agents through the `aeria` command.

## The `aeria` command

Status: **proposal, being implemented.**

`aeria` is a console command that an agent runs in the project directory.
It works on the project files directly; the desktop does not need to be
open. Output is compact text by default and JSON on request. Every command and
the tool itself have `--help`.

| Command | Purpose |
| --- | --- |
| `overview` | Areas and sheets of the project with their progress; sheet search by name or pattern. |
| `read` | A scene of strings: speaker, source, the other client languages, macro legends, row context, the current translation and who wrote it, marks where a line varies with the player character's gender, similar translations, and the knowledge the scene needs. |
| `write` | Writes a batch of translations. Each is checked; rejected ones come back with the reason. |
| `check` | The checks of `write` without writing. |
| `find` | Searches the source, the translations, and the other client languages. |
| `brief` | The translation rules and the project facts, for subagents. |
| `guide` | How the project is organized and how to work on it. |

Later: `audit` (project-wide deterministic checks such as inconsistent terms or
address), `changes` (what a game update changed), and `flag` (mark strings for
a person).

Invariants:

- A write is checked before anything is stored: the assisted structure policy
  of [`strings.md`](./strings.md), forbidden term variants, and the unit's
  expected state (compare-and-set). An invalid or structurally unsafe
  translation is never stored.
- An agent writes an untranslated string or replaces a translation an agent
  wrote. A translation a person wrote or changed, and a reviewed one, is never
  replaced; it is reported instead. Which translations an agent wrote is kept
  per project in local application data, with the text it wrote: a person's
  later edit changes the text, so the translation becomes the person's.
- Writes from several processes, such as parallel subagents and the desktop,
  are serialized per project, and a writer reloads the workspace before it
  writes. The desktop reloads the open project when the files change on disk.
- Identity, source updates, merges, and export do not depend on agents.

Discovery: Aeria writes `AGENTS.md` and `CLAUDE.md` at the project root, which
agent harnesses read, and can install an `aeria-localization` skill for
harnesses that use skills and put `aeria` on `PATH`.
