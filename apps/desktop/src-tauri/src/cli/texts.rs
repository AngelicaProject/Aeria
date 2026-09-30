//! What agents need to know, as text: the rules of a translation for
//! `game/README.md`, and Aeria's part of `AGENTS.md` and `CLAUDE.md`.

use std::fmt::Write as _;

use aeria_knowledge::rules::{
    MACRO_TEXT, ORIGINAL_TEXT, PLAYER_CHARACTER, TRANSLATION_STYLE, living_language,
};

use super::project::Project;

/// The rules of a translation and how macros are written, for
/// `game/README.md`.
pub(crate) fn rules(project: &Project) -> String {
    let target = Some(project.settings.target_language.clone());
    let mut text = format!(
        "# Translating FINAL FANTASY XIV for this project\n\nSource language: {}. Target \
         language: {}. Game version: {}.\n\n",
        project.settings.source_language,
        target.as_deref().unwrap_or("not set yet"),
        project.source.version()
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
    text
}

/// `po/README.md`: the layout of the files, how to work on them, and the
/// rules of a translation.
fn readme(project: &Project) -> String {
    let source = project.source.language().code();
    let target = &project.settings.target_language;
    let mut text = format!(
        "# The text of FINAL FANTASY XIV and its translation\n\n\
This folder is the project: every translatable string of the game, version {}, and its \
translation. It is kept in Git like source code.\n\n\
## Layout\n\n\
- A quest (`quest/…`) or cutscene (`cut_scene/…`) is one file, its strings in the order \
the player sees them.\n\
- Any other sheet is one file, or a folder of files by row ID: `BNpcName/3000.po` holds \
rows 3000 to 3999.\n\
- Each string is a gettext PO entry. `msgctxt` names it: `sheet:row:subrow:column`, or \
`sheet:key:column` in quest and cutscene dialogue, where rows are keyed. `msgid` is the \
{source} source, `msgstr` the {target} translation, empty while there is none.\n\
- The `#.` lines above an entry are the other client languages (`ja` is the original), \
the speaker or kind of line, the row's other cells, and what the macros do. `# ` lines \
are translators' notes.\n\
- `#, fuzzy` marks a translation whose source a game update changed; `#| msgid` is the \
source it was written for. Bring the translation in line with the new source; changing \
the translation removes the mark. A fuzzy translation does not reach players.\n\
- Obsolete entries (`#~`) at the end of a file are strings the game no longer has, kept \
for their translations.\n\
- Search the whole game with any tool, for example to see how a name is used everywhere \
or how the official localizations handled a macro.\n\n\
## Translating\n\n\
Write the translation into `msgstr` as a PO string: `\\\"` for a quote, `\\\\` for a \
backslash, one line (the game breaks lines with `<br>`). The macro text below writes a \
literal `<` as `\\<`; in a PO string that backslash is doubled, so `msgstr` holds \
`\\\\<`. Leave `msgctxt`, `msgid`, and the `#.` lines as they are. Notes go on `# ` lines \
above `msgctxt`.\n\n\
Then run `aeria check`. It checks the files changed since the last commit (`--all` for \
every file) and lists each problem as `file:line: what to fix`: the PO format, Git \
conflict markers, a `msgctxt` or `msgid` that is not the game's, and translations that \
break the macros, use a forbidden variant of a term, or write both genders at once. \
Advice, such as a term that seems missing, is listed separately. A translation with a \
problem does not reach players until it is fixed.\n\n\
Several agents can translate at once, each in its own files. Keep temporary files \
(lists, scripts, batches) in the system's temporary folder, never in the project: \
everything in the project ends up in its repository.\n\n\
## Git\n\n\
The files are the project, and Git is its history. `git diff` shows your work, and \
`git log` and `git blame` show who changed a string and when.\n\n\
- Never run `git restore`, `git checkout`, `git reset`, `git clean`, or `git stash` on \
project files: they discard translations that are not committed, yours and others'.\n\
- Do not commit or push unless asked. The user reviews the changes and commits them in \
Aeria.\n\
- A game update (`aeria update`) is the user's step: it makes every file again for the \
new game version as one commit.\n\n\
## The project knowledge\n\n\
`../aeria-knowledge/` is the documentation every translation follows. Read it, follow \
it, and keep it current by editing its files; `aeria check` reports every problem in them \
with its line.\n\n\
- `style.md`: how each kind of text reads, one `## <kind>` section per kind: general, \
journal, objective, system, dialogue, names, items, actions, interface, lore.\n\
- `terms.csv`: terms every translation renders the same way; columns term, translation, \
note, forbidden (variants separated by `;`), settled. `aeria check` reports a forbidden \
variant of a term in a translation of a string that contains the term.\n\
- `characters.md`: how characters speak, one `## LABEL` section per character, named by \
the speaker labels of the files (`#. speaker:`), several labels separated by commas.\n\
- `story.md`: what happened so far, one `## <sheet>` section per quest or cutscene \
sheet, so later scenes stay consistent with earlier ones.\n\
- `lessons.md`: recurring problems and what to do instead, one `## <id>` section per \
lesson; `<!-- aeria: domain=<kind> -->` under the heading limits one to a kind of text.\n\n\
An entry a person decided is settled: `yes` in the settled column of a term, or \
`<!-- aeria: settled=yes -->` as the first line under a heading. Follow settled entries \
and do not change them without asking the user; entries you write are not settled. \
Record decisions as you make them: a term, a character's voice, what a scene \
established, a correction that will recur. Settle what many strings share, names and \
terms first, before translating them in parallel, and translate quests and cutscenes in \
the game's order, so later scenes build on earlier ones. Ask the user about matters of \
taste, such as how formal the translation is or how a well-known name is rendered, and \
record the answer as a settled entry.\n\n",
        project.source.version()
    );
    text.push_str(&rules(project));
    text.push_str(
        "The project knowledge in `../aeria-knowledge/` takes precedence over the style \
         defaults above. Entries marked settled were decided by a person; ask before \
         changing them.\n",
    );
    text
}

/// Writes `po/README.md` when its text changed.
pub(crate) fn write_readme(project: &Project) -> Result<(), String> {
    let path = project.root.join(aeria_po::PO_DIR).join("README.md");
    let text = readme(project);
    if std::fs::read(&path).ok().as_deref() != Some(text.as_bytes()) {
        std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

const BLOCK_BEGIN: &str = "<!-- aeria:begin -->";
const BLOCK_END: &str = "<!-- aeria:end -->";

/// Aeria's part of `AGENTS.md`: what any agent harness needs to know first.
const AGENTS_BLOCK: &str = "\
## Aeria localization project

This directory is an [Aeria](https://angelicaproject.github.io/Aeria/) project: a fan
localization of FINAL FANTASY XIV, kept in Git.

- `po/` is the project: the whole text of the game as gettext PO files, a file per quest,
  cutscene, or sheet, each string with its Japanese, English, German, and French texts
  and its translation in `msgstr`. Read `po/README.md` first: the layout, the rules of a
  translation, how the game's macros work, and how to work with Git here.
- Translate by writing `msgstr` in those files with any tools, then run `aeria check`
  and fix every `file:line` it reports.
- `aeria-knowledge/` holds the project's terms, style, character voices, and story:
  follow it and keep it current. Entries marked settled are a person's decisions; ask
  before changing them.
- Never run `git restore`, `git checkout`, `git reset`, `git clean`, or `git stash` on
  project files, and do not commit or push unless asked: `git diff` shows your work,
  and the user reviews and commits it.";

/// Aeria's part of `CLAUDE.md`: Claude Code reads `CLAUDE.md` and imports
/// `AGENTS.md` from it.
const CLAUDE_BLOCK: &str = "@AGENTS.md";

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
        "Agent harnesses that start in this directory find the project and its rules. Commit these files with the project so collaborators get them too.\n",
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
        assert!(agents.contains("po/README.md"));
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
    }
}
