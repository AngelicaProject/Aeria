//! `brief` and `guide`: what agents need to know, as text.

use std::fmt::Write as _;

use aeria_knowledge::rules::{
    MACRO_TEXT, ORIGINAL_TEXT, PLAYER_CHARACTER, TRANSLATION_STYLE, living_language,
};

use super::project::Project;

/// How translations reach the project with the command.
const WRITING: &str = "\
Writing translations:
- Read a scene with `aeria read <sheet>`: every string of a quest or cutscene in play \
order, or of another sheet in row order, with its address (`@sheet:row:subrow:column`), \
speaker, source, the other client languages, macros, the current translation, and the \
project knowledge the lines need. Translate a quest or cutscene as one scene, so voices, \
address, and jokes stay consistent across its lines: read all its parts first.
- Write as you go, about 50 strings per write: translate a part, write it, go on. \
Written translations are saved at once, so nothing is lost if the work stops, and \
`aeria read <sheet> --untranslated` shows what is left. Do not hold a large range back \
to write it in one go. For a long list sheet, repeat `aeria read <sheet> --rows <a>-<b> \
--untranslated --limit 50`, translate, write, until nothing untranslated is left.
- Write translations with `aeria write`, one block per string: the `@address` line, \
then the translation on the next line, written as it is, with no escaping. Several \
blocks go in one call. Never put translations inside a shell command line, where \
quoting corrupts apostrophes and quotes: write the blocks to a file in the system's \
temporary folder with a file-writing tool and run `aeria write <file>`, or pipe them \
on standard input (`aeria write -`). The file is deleted after the write; never put \
one in the project. Every translation is checked as it is written; a rejected one \
comes back with what to fix. `aeria check` runs the same checks without writing.
- You may write an untranslated string, or replace a translation an agent wrote. A \
translation a person wrote or changed, and a reviewed one, is marked `(keep)` by `aeria \
read`, and `aeria write` skips it: tell the user if you think it should change.
- Advice that comes back with a written translation (a term that seems missing, a word \
that may need a gender condition, phrasing that reads machine-written) is worth a \
second look; fix it with another write when it is right.";

/// The rules of a translation for this project, for any agent translating.
pub(crate) fn brief(project: &Project) -> String {
    let target = project.target_language();
    let mut text = format!(
        "# Translating FINAL FANTASY XIV for this project\n\nSource language: {}. Target \
         language: {}. Game version: {}.\n\n",
        project.source_language(),
        target.as_deref().unwrap_or("not set yet"),
        project.session.source().version()
    );
    for section in [ORIGINAL_TEXT, TRANSLATION_STYLE, PLAYER_CHARACTER] {
        text.push_str(section);
        text.push_str("\n\n");
    }
    if let Some(living) = target.as_deref().and_then(living_language) {
        text.push_str(living);
        text.push_str("\n\n");
    }
    text.push_str(MACRO_TEXT);
    text.push('\n');
    text.push_str(&aeria_se::authoring_reference());
    text.push_str("\n\n");
    text.push_str(WRITING);
    text.push_str(
        "\n\nThe project knowledge `aeria read` shows with the lines takes precedence over \
         the style defaults above. Entries marked settled were decided by a person.\n",
    );
    text
}

/// How the project is organized and how to work on it, for the agent that
/// leads the work.
pub(crate) fn guide(project: &Project) -> String {
    let mut text = String::new();
    let _ = write!(
        text,
        "# Localizing this project with the aeria command\n\n\
         This is an Aeria project: a fan localization of FINAL FANTASY XIV from {} into {}. \
         The game's text is read from the installed game; the project stores only \
         translations and decisions. Aeria, the desktop editor, shows every translation you \
         write as soon as it is written.\n\n",
        project.source_language(),
        project
            .target_language()
            .as_deref()
            .unwrap_or("a target language not set yet")
    );
    text.push_str(GUIDE);
    text
}

const GUIDE: &str = "\
## The project

- `.aeria/` holds the translations. Never edit it directly: write with `aeria write`.
- Keep the project clean: pass translations to `aeria write` on standard input, and put \
any temporary file (batches, notes, scripts) in the system's temporary folder, never in \
the project; delete it when done. Everything in the project ends up in its repository.
- `aeria-knowledge/` is the project knowledge, the documentation every translation \
follows. Read it, follow it, and keep it current; edit its files directly:
  - `style.md`: how each kind of text reads, one `## <kind>` section per kind: \
general, journal, objective, system, dialogue, names, items, actions, interface, lore.
  - `terms.csv`: terms every translation renders the same way; columns term, \
translation, note, forbidden (variants separated by `;`), settled.
  - `characters.md`: how characters speak, one `## LABEL` section per character, named \
by the speaker labels `aeria read` shows (several labels separated by commas).
  - `story.md`: what happened so far, one `## <sheet>` section per quest or cutscene \
sheet, so later scenes stay consistent with earlier ones.
  - `lessons.md`: recurring problems and what to do instead, one `## <id>` section per \
lesson; `<!-- aeria: domain=<kind> -->` under the heading limits one to a kind of text.
- An entry a person decided is settled: `yes` in the settled column of a term, or \
`<!-- aeria: settled=yes -->` as the first line under a heading. Follow settled entries \
and do not change them without asking the user. Entries you write are not settled.
- `aeria knowledge` checks the files and reports every problem with its line.

## Commands

- `aeria overview` — the project's areas and progress; `aeria overview <pattern>` lists \
sheets, such as `quest/*` or `*item*`; `--folders` lists folders such as quest/000.
- `aeria read <sheet>` — a scene with everything needed to translate it. A long scene \
comes in parts that fit a terminal; the end of each part gives the command for the next \
(`--from`).
- `aeria brief` — the translation rules; give it to every agent that translates.
- `aeria write` / `aeria check` — write translations, or only check them.
- `aeria find <text>` — search the source text, or translations with `--in translation`.
- `aeria audit [<pattern>]` — deterministic checks of the translations: the same source translated differently, forbidden terms, broken macros, gender, machine phrasing, length. Fix agents' findings with `aeria write`; flag a person's.
- `aeria review` — what waits for review; `aeria flag <address>… --reason <text>` marks translations for a person to decide.
- Every command has `--help`, and `--json` for machine-readable output.

## Working on a large scope

- Plan by scenes: a quest or cutscene sheet is one unit of work; other sheets split by \
rows (`--rows`). Scenes that do not share characters or terms can be translated in \
parallel by several agents; writes from parallel agents are safe.
- Settle what many scenes share before translating them: names and terms first (names, \
places, items, actions), then the style of each kind of text, then quests and cutscenes \
in the game's order, so later scenes build on earlier ones.
- Record decisions as you make them: a term in `terms.csv`, a character's voice in \
`characters.md`, what a scene established in `story.md`, a correction that will recur \
in `lessons.md`. Agents that translate later read them through `aeria read`.
- Ask the user about matters of taste that are theirs to decide, such as how formal \
the translation is or how a well-known name is rendered, and record the answer as a \
settled entry.
- Give each translating agent `aeria brief`, its scene, and the instruction to write \
with `aeria write` as it goes, about 50 strings per write, and fix what comes back \
rejected.
- The user reviews and commits the work in Aeria; do not commit or push unless asked.
";

const BLOCK_BEGIN: &str = "<!-- aeria:begin -->";
const BLOCK_END: &str = "<!-- aeria:end -->";

/// Aeria's part of `AGENTS.md`: what any agent harness needs to find the
/// command.
const AGENTS_BLOCK: &str = "\
## Aeria localization project

This directory is an [Aeria](https://angelicaproject.github.io/Aeria/) project: a fan
localization of FINAL FANTASY XIV. The game's text comes from the installed game; the
project keeps the translations in `.aeria/` and its knowledge in `aeria-knowledge/`.
Work on it with the `aeria` command:

- Start with `aeria guide`: how the project is organized and how to work on it,
  alone or with several agents.
- Every agent that translates reads `aeria brief` first.
- Read a scene with `aeria read <sheet>` and write translations with `aeria write`;
  never edit `.aeria/` directly.
- `aeria-knowledge/` is the project's documentation: follow it and keep it current.
  Entries marked settled are a person's decisions; ask before changing them.
- `aeria --help` lists every command.";

/// Aeria's part of `CLAUDE.md`: Claude Code reads `CLAUDE.md` and imports
/// `AGENTS.md` from it.
const CLAUDE_BLOCK: &str = "@AGENTS.md";

/// The skill that tells agent harnesses with skills how to localize with
/// the command, as `SKILL.md`.
pub(crate) const SKILL: &str = "\
---
name: aeria-localization
description: Localize FINAL FANTASY XIV in an Aeria project with the aeria command: read scenes with every client language, write checked translations, keep the project knowledge, and split large scopes among subagents. Use in any directory with an .aeria folder, or when asked to translate or localize FFXIV text with Aeria.
---

# Localizing with Aeria

An Aeria project is a directory with `.aeria/` (the translations) and
`aeria-knowledge/` (style, terms, character voices, story, lessons). The `aeria`
command reads the game and the project and writes checked translations; it works
in the project directory, and an open Aeria window shows every write at once.

1. Run `aeria guide` in the project and follow it; `aeria overview` shows what is
   translated.
2. Give every agent that translates the text of `aeria brief`, and one scene: a
   quest or cutscene sheet, or a range of rows of another sheet (`--rows`).
3. A translating agent reads its scene with `aeria read <sheet>` (add
   `--untranslated` to see only what is left; a long scene comes in parts, and the end
   of each gives the command for the next), and writes as it goes, about 50 strings
   per write, in blocks of an `@address` line and the translation as it is (no
   escaping): a file written with a file-writing tool and passed as `aeria write
   <file>`, or `aeria write -` on standard input, never translations inside a shell
   command line. It fixes what comes back REJECTED. Written work is saved at once;
   `--untranslated` shows what is left. Temporary files go into the system's
   temporary folder, never into the project.
4. Record decisions in `aeria-knowledge/` as you go; ask the user about matters of
   taste and settled entries.
5. Check the work with `aeria audit` and `aeria review`; flag what a person must
   decide with `aeria flag`.

Every command has `--help` and `--json`.
";

/// Puts Aeria's block into a file between its markers, keeping everything
/// else. Returns whether the file changed.
fn upsert_block(path: &std::path::Path, block: &str) -> Result<bool, String> {
    let wrapped = format!("{BLOCK_BEGIN}\n{block}\n{BLOCK_END}");
    let current = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let next = match &current {
        None => format!("{wrapped}\n"),
        Some(text) => match (text.find(BLOCK_BEGIN), text.find(BLOCK_END)) {
            (Some(begin), Some(end)) if begin < end => format!(
                "{}{wrapped}{}",
                &text[..begin],
                &text[end + BLOCK_END.len()..]
            ),
            _ => format!("{}\n\n{wrapped}\n", text.trim_end()),
        },
    };
    if current.as_deref() == Some(next.as_str()) {
        return Ok(false);
    }
    std::fs::write(path, next).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(true)
}

/// Writes Aeria's blocks into `AGENTS.md` and `CLAUDE.md` at the project
/// root, and says what changed.
pub(crate) fn init(root: &std::path::Path) -> Result<String, String> {
    let mut report = String::new();
    for (name, block) in [("AGENTS.md", AGENTS_BLOCK), ("CLAUDE.md", CLAUDE_BLOCK)] {
        let changed = upsert_block(&root.join(name), block)?;
        let _ = writeln!(
            report,
            "{name}: {}",
            if changed {
                "written"
            } else {
                "already current"
            }
        );
    }
    report.push_str(
        "Agent harnesses that start in this directory now find the aeria command. Commit these files with the project so collaborators get them too.\n",
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_keeps_the_user_text_and_replaces_its_own_block() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path();
        std::fs::write(root.join("AGENTS.md"), "# Our rules\nBe kind.\n").expect("agents");
        init(root).expect("init");
        let agents = std::fs::read_to_string(root.join("AGENTS.md")).expect("read");
        assert!(agents.starts_with("# Our rules\nBe kind.\n\n<!-- aeria:begin -->"));
        assert!(agents.contains("aeria guide"));
        let claude = std::fs::read_to_string(root.join("CLAUDE.md")).expect("read");
        assert_eq!(
            claude,
            "<!-- aeria:begin -->\n@AGENTS.md\n<!-- aeria:end -->\n"
        );
        assert!(
            init(root)
                .expect("again")
                .contains("AGENTS.md: already current")
        );
        assert!(SKILL.starts_with("---\nname: aeria-localization\n"));
    }
}
