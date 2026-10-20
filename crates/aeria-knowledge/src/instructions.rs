//! Instructions: what a person asks machine translation to do to every
//! translation of a run, kept as Markdown files in
//! `aeria-knowledge/instructions/`, one per instruction, named by the file
//! (see `docs/formats/knowledge-v1.md#instructions`).

use std::fs;
use std::path::{Path, PathBuf};

use crate::KNOWLEDGE_DIR;

/// The directory of the instructions inside [`KNOWLEDGE_DIR`].
pub const INSTRUCTIONS_DIR: &str = "instructions";

/// The extension of an instruction's file.
const EXTENSION: &str = "md";

/// Largest instruction read or written.
pub const MAX_INSTRUCTION_BYTES: u64 = 256 * 1024;

/// Characters a name cannot hold: those a file name of Windows or Linux
/// cannot.
const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// One instruction: its name, which is its file's name without `.md`, and
/// its text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub name: String,
    pub text: String,
}

/// The directory of the instructions of the project at `root`.
#[must_use]
pub fn instructions_dir(root: &Path) -> PathBuf {
    root.join(KNOWLEDGE_DIR).join(INSTRUCTIONS_DIR)
}

/// The file of the instruction `name`.
#[must_use]
pub fn instruction_path(root: &Path, name: &str) -> PathBuf {
    instructions_dir(root).join(format!("{name}.{EXTENSION}"))
}

/// Whether `name` can name an instruction: not empty, without white space
/// at either end, a leading dot, a control character, or a character a file
/// name cannot hold.
///
/// # Errors
///
/// Returns why it cannot.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("an instruction needs a name".to_owned());
    }
    if name.trim() != name {
        return Err("a name cannot begin or end with a space".to_owned());
    }
    if name.starts_with('.') {
        return Err("a name cannot begin with a dot".to_owned());
    }
    if let Some(found) = name
        .chars()
        .find(|character| FORBIDDEN.contains(character) || character.is_control())
    {
        return Err(format!("a name cannot hold {found:?}"));
    }
    Ok(())
}

/// The text of the instruction `name`, `None` when there is none.
///
/// # Errors
///
/// Returns a description when the file is too large, not UTF-8, or cannot
/// be read.
pub fn read_instruction(root: &Path, name: &str) -> Result<Option<String>, String> {
    let path = instruction_path(root, name);
    let shown = format!("{KNOWLEDGE_DIR}/{INSTRUCTIONS_DIR}/{name}.{EXTENSION}");
    match fs::metadata(&path) {
        Ok(metadata) if metadata.len() > MAX_INSTRUCTION_BYTES => {
            return Err(format!(
                "{shown} is larger than {MAX_INSTRUCTION_BYTES} bytes"
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{shown}: {error}")),
    }
    let bytes = fs::read(&path).map_err(|error| format!("{shown}: {error}"))?;
    String::from_utf8(bytes)
        .map(|text| Some(text.trim_start_matches('\u{feff}').to_owned()))
        .map_err(|_| format!("{shown} is not UTF-8"))
}

/// The instructions of the project at `root`, by name ignoring case, and
/// the problems of the files that cannot be read, which are left out.
#[must_use]
pub fn list_instructions(root: &Path) -> (Vec<Instruction>, Vec<String>) {
    let mut instructions = Vec::new();
    let mut problems = Vec::new();
    let Ok(entries) = fs::read_dir(instructions_dir(root)) else {
        return (instructions, problems);
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file()
            || path
                .extension()
                .is_none_or(|extension| !extension.eq_ignore_ascii_case(EXTENSION))
        {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if check_name(name).is_err() {
            continue;
        }
        match read_instruction(root, name) {
            Ok(Some(text)) => instructions.push(Instruction {
                name: name.to_owned(),
                text,
            }),
            Ok(None) => {}
            Err(problem) => problems.push(problem),
        }
    }
    instructions.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    (instructions, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_are_the_markdown_files_of_their_directory() {
        let root = tempfile::tempdir().expect("root");
        let dir = instructions_dir(root.path());
        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("Род игрока.md"), "\u{feff}Исправь род.").expect("write");
        fs::write(dir.join("канцелярит.md"), "Убери канцелярит.").expect("write");
        fs::write(dir.join("notes.txt"), "not one").expect("write");
        fs::write(dir.join(".hidden.md"), "not one").expect("write");
        let (found, problems) = list_instructions(root.path());
        assert!(problems.is_empty(), "{problems:?}");
        let names: Vec<&str> = found.iter().map(|found| found.name.as_str()).collect();
        assert_eq!(names, ["канцелярит", "Род игрока"]);
        assert_eq!(found[1].text, "Исправь род.");
        assert_eq!(
            read_instruction(root.path(), "нет такой").expect("read"),
            None
        );
    }

    #[test]
    fn a_name_is_one_a_file_can_have() {
        assert!(check_name("Род игрока").is_ok());
        for name in ["", " ", " Род", "Род ", ".git", "a/b", "a:b", "a\u{7}"] {
            assert!(check_name(name).is_err(), "{name:?}");
        }
    }
}
