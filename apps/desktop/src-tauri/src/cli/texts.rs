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
address, and jokes stay consistent across its lines.
- Write translations with `aeria write`, one block per string: the `@address` line, \
then the translation on the next line. Several blocks go in one call, from a file or \
standard input. Every translation is checked; a rejected one comes back with what to \
fix. `aeria check` runs the same checks without writing.
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
- `aeria read <sheet>` — a scene with everything needed to translate it.
- `aeria brief` — the translation rules; give it to every agent that translates.
- `aeria write` / `aeria check` — write translations, or only check them.
- `aeria find <text>` — search the source text, or translations with `--in translation`.
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
with `aeria write` and fix what comes back rejected.
- The user reviews and commits the work in Aeria; do not commit or push unless asked.
";
