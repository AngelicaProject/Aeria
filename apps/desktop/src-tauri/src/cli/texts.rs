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
    text
}

const BLOCK_BEGIN: &str = "<!-- aeria:begin -->";
const BLOCK_END: &str = "<!-- aeria:end -->";

/// Aeria's part of `AGENTS.md`: what any agent harness needs to find the
/// command.
const AGENTS_BLOCK: &str = "\
## Aeria localization project

This directory is an [Aeria](https://angelicaproject.github.io/Aeria/) project: a fan
localization of FINAL FANTASY XIV.

- `game/` holds the whole text of the game as gettext PO files: a file per quest,
  cutscene, or sheet, with the Japanese, English, German, and French texts. Read
  `game/README.md` first: the layout, the rules of a translation, and how the game's
  macros work. If `game/` is missing, make it with `aeria corpus`; otherwise start with
  `aeria check`, which brings it up to date with the project.
- Translate by writing `msgstr` in those files, with any tools. Then run `aeria check`:
  it saves what changed and lists each problem as `file:line`.
- `aeria-knowledge/` holds the project's terms, style, character voices, and story:
  follow it and keep it current. Entries marked settled are a person's decisions; ask
  before changing them.
- Never edit `.aeria/`, where the project keeps its translations: `aeria check` saves
  them there.";

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
        assert!(agents.contains("game/README.md"));
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
