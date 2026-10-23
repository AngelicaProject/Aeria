//! Skills: what a person asks machine translation to do to the translations
//! of a run, and where, kept as Markdown files in `aeria-knowledge/skills/`,
//! one per skill, named by the file (see `docs/formats/knowledge-v1.md#skills`).
//!
//! A skill's file may begin with a front matter of YAML between two `---`
//! lines that says where it applies: the sheets and folders it takes, those
//! it leaves out, and regular expressions the strings it takes match. The
//! rest of the file is what the model is asked.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use yaml_rust2::{Yaml, YamlLoader};

use crate::KNOWLEDGE_DIR;

/// The directory of the skills inside [`KNOWLEDGE_DIR`].
pub const SKILLS_DIR: &str = "skills";

/// The extension of a skill's file.
const EXTENSION: &str = "md";

/// Largest skill read or written.
pub const MAX_SKILL_BYTES: u64 = 256 * 1024;

/// Characters a name cannot hold: those a file name of Windows or Linux
/// cannot.
const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// The line that opens and closes a front matter.
const FENCE: &str = "---";

/// Where a skill applies.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SkillScope {
    /// Sheets and folders of sheets (`quest/`) relative to `po/`; empty for
    /// every one.
    pub paths: Vec<String>,
    /// Sheets and folders left out of `paths`.
    pub exclude: Vec<String>,
    /// A regular expression the source of a string matches, as macro text.
    pub source: Option<String>,
    /// A regular expression the translation of a string matches.
    pub translation: Option<String>,
    /// A regular expression one of the string's lines in the other client
    /// languages (`ja`, `de`, `fr`) matches.
    pub other: Option<String>,
}

impl SkillScope {
    /// Whether the scope says nothing: every translation of the project.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One skill: its name, which is its file's name without `.md`, where it
/// applies, and what it asks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Skill {
    pub name: String,
    pub scope: SkillScope,
    pub body: String,
}

/// The directory of the skills of the project at `root`.
#[must_use]
pub fn skills_dir(root: &Path) -> PathBuf {
    root.join(KNOWLEDGE_DIR).join(SKILLS_DIR)
}

/// The file of the skill `name`.
#[must_use]
pub fn skill_path(root: &Path, name: &str) -> PathBuf {
    skills_dir(root).join(format!("{name}.{EXTENSION}"))
}

/// Whether `name` can name a skill: not empty, without white space at
/// either end, a leading dot, a control character, or a character a file
/// name cannot hold.
///
/// # Errors
///
/// Returns why it cannot.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("a skill needs a name".to_owned());
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

/// Reads the text of a skill's file: its scope and what it asks.
///
/// # Errors
///
/// Returns why the front matter cannot be read: it is not closed, is not
/// YAML, has a key or a value of a kind a skill does not have, or a regular
/// expression that does not compile.
pub fn parse_skill(text: &str) -> Result<(SkillScope, String), String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().unwrap_or_default();
    if first.trim_end() != FENCE {
        return Ok((SkillScope::default(), text.to_owned()));
    }
    let mut front = String::new();
    let mut closed = false;
    let mut consumed = first.len();
    for line in lines.by_ref() {
        consumed += line.len();
        if line.trim_end() == FENCE {
            closed = true;
            break;
        }
        front.push_str(line);
    }
    if !closed {
        return Err("the front matter has no closing --- line".to_owned());
    }
    let body = text[consumed..].trim_start_matches(['\r', '\n']).to_owned();
    Ok((scope_of(&front)?, body))
}

fn scope_of(front: &str) -> Result<SkillScope, String> {
    let documents =
        YamlLoader::load_from_str(front).map_err(|error| format!("the front matter: {error}"))?;
    let mut scope = SkillScope::default();
    let Some(document) = documents.into_iter().next() else {
        return Ok(scope);
    };
    let Yaml::Hash(fields) = document else {
        if matches!(document, Yaml::Null) {
            return Ok(scope);
        }
        return Err("the front matter is not a map of keys".to_owned());
    };
    for (key, value) in fields {
        let Yaml::String(key) = key else {
            return Err("a key of the front matter is not text".to_owned());
        };
        match key.as_str() {
            "scope" => scope.paths = paths_of(&key, value)?,
            "exclude" => scope.exclude = paths_of(&key, value)?,
            "match" => {
                let Yaml::Hash(matches) = value else {
                    return Err("`match` is a map of source, translation, and other".to_owned());
                };
                for (field, pattern) in matches {
                    let (Yaml::String(field), Yaml::String(pattern)) = (field, pattern) else {
                        return Err("each `match` is a field and a regular expression".to_owned());
                    };
                    regex::RegexBuilder::new(&pattern)
                        .case_insensitive(true)
                        .build()
                        .map_err(|error| format!("`match.{field}`: {error}"))?;
                    match field.as_str() {
                        "source" => scope.source = Some(pattern),
                        "translation" => scope.translation = Some(pattern),
                        "other" => scope.other = Some(pattern),
                        other => {
                            return Err(format!(
                                "`match.{other}` is not a field: source, translation, or other"
                            ));
                        }
                    }
                }
            }
            other => {
                return Err(format!(
                    "`{other}` is not a key of a skill: scope, exclude, or match"
                ));
            }
        }
    }
    Ok(scope)
}

fn paths_of(key: &str, value: Yaml) -> Result<Vec<String>, String> {
    let Yaml::Array(items) = value else {
        return Err(format!("`{key}` is a list of sheets and folders"));
    };
    items
        .into_iter()
        .map(|item| match item {
            Yaml::String(path) if !path.trim().is_empty() => Ok(path.trim().to_owned()),
            _ => Err(format!("an item of `{key}` is not a sheet or folder")),
        })
        .collect()
}

/// A YAML scalar in single quotes, where only a quote is doubled: a regular
/// expression keeps its backslashes.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The file of a skill in canonical form: the front matter of its scope,
/// when it has one, then what it asks.
#[must_use]
pub fn write_skill(scope: &SkillScope, body: &str) -> String {
    let mut text = String::new();
    if !scope.is_empty() {
        text.push_str(FENCE);
        text.push('\n');
        for (key, paths) in [("scope", &scope.paths), ("exclude", &scope.exclude)] {
            if paths.is_empty() {
                continue;
            }
            let _ = writeln!(text, "{key}:");
            for path in paths {
                let _ = writeln!(text, "  - {}", quoted(path));
            }
        }
        let matches = [
            ("source", &scope.source),
            ("translation", &scope.translation),
            ("other", &scope.other),
        ];
        if matches.iter().any(|(_, pattern)| pattern.is_some()) {
            text.push_str("match:\n");
            for (field, pattern) in matches {
                if let Some(pattern) = pattern {
                    let _ = writeln!(text, "  {field}: {}", quoted(pattern));
                }
            }
        }
        text.push_str(FENCE);
        text.push_str("\n\n");
    }
    text.push_str(body.trim_end());
    text.push('\n');
    text
}

/// The text of the skill `name`, `None` when there is none.
///
/// # Errors
///
/// Returns a description when the file is too large, not UTF-8, or cannot
/// be read.
pub fn read_skill(root: &Path, name: &str) -> Result<Option<String>, String> {
    let path = skill_path(root, name);
    let shown = format!("{KNOWLEDGE_DIR}/{SKILLS_DIR}/{name}.{EXTENSION}");
    match fs::metadata(&path) {
        Ok(metadata) if metadata.len() > MAX_SKILL_BYTES => {
            return Err(format!("{shown} is larger than {MAX_SKILL_BYTES} bytes"));
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

/// A skill as its file is: its name, its text, and the skill it reads as,
/// or why it does not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillFile {
    pub name: String,
    pub text: String,
    pub skill: Result<Skill, String>,
}

/// The skills of the project at `root`, by name ignoring case, and the
/// problems of the files that cannot be read, which are left out.
#[must_use]
pub fn list_skills(root: &Path) -> (Vec<SkillFile>, Vec<String>) {
    let mut skills = Vec::new();
    let mut problems = Vec::new();
    let Ok(entries) = fs::read_dir(skills_dir(root)) else {
        return (skills, problems);
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
        match read_skill(root, name) {
            Ok(Some(text)) => {
                let skill = parse_skill(&text).map(|(scope, body)| Skill {
                    name: name.to_owned(),
                    scope,
                    body,
                });
                skills.push(SkillFile {
                    name: name.to_owned(),
                    text,
                    skill,
                });
            }
            Ok(None) => {}
            Err(problem) => problems.push(problem),
        }
    }
    skills.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    (skills, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_are_the_markdown_files_of_their_directory() {
        let root = tempfile::tempdir().expect("root");
        let dir = skills_dir(root.path());
        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("Род игрока.md"), "\u{feff}Исправь род.").expect("write");
        fs::write(dir.join("канцелярит.md"), "Убери канцелярит.").expect("write");
        fs::write(dir.join("notes.txt"), "not one").expect("write");
        fs::write(dir.join(".hidden.md"), "not one").expect("write");
        let (found, problems) = list_skills(root.path());
        assert!(problems.is_empty(), "{problems:?}");
        let names: Vec<&str> = found.iter().map(|found| found.name.as_str()).collect();
        assert_eq!(names, ["канцелярит", "Род игрока"]);
        assert_eq!(found[1].text, "Исправь род.");
        assert_eq!(read_skill(root.path(), "нет такого").expect("read"), None);
    }

    #[test]
    fn a_skill_says_where_it_applies_in_its_front_matter() {
        let text = "---\nscope:\n  - quest/\n  - DefaultTalk\nexclude: ['quest/000/']\nmatch:\n  other: '\\$gn4'\n  translation: \"ты\"\n---\n\nИсправь род.\n";
        let (scope, body) = parse_skill(text).expect("parse");
        assert_eq!(scope.paths, ["quest/", "DefaultTalk"]);
        assert_eq!(scope.exclude, ["quest/000/"]);
        assert_eq!(scope.other.as_deref(), Some("\\$gn4"));
        assert_eq!(scope.translation.as_deref(), Some("ты"));
        assert_eq!(body, "Исправь род.\n");
        // The canonical form reads back the same.
        let written = write_skill(&scope, &body);
        assert_eq!(parse_skill(&written).expect("again"), (scope, body.clone()));
        assert_eq!(
            write_skill(&SkillScope::default(), "Текст.\n\n"),
            "Текст.\n"
        );
        // A file without a front matter is all body.
        assert_eq!(
            parse_skill("Просто текст.").expect("plain"),
            (SkillScope::default(), "Просто текст.".to_owned())
        );
    }

    #[test]
    fn a_front_matter_a_skill_cannot_have_is_refused() {
        for text in [
            "---\nscope: quest/\n---\nx",
            "---\nwhere: [quest/]\n---\nx",
            "---\nmatch:\n  speaker: a\n---\nx",
            "---\nmatch:\n  source: '('\n---\nx",
            "---\nscope: [quest/]\n",
            "---\n- a\n---\nx",
        ] {
            assert!(parse_skill(text).is_err(), "{text}");
        }
    }

    #[test]
    fn a_name_is_one_a_file_can_have() {
        assert!(check_name("Род игрока").is_ok());
        for name in ["", " ", " Род", "Род ", ".git", "a/b", "a:b", "a\u{7}"] {
            assert!(check_name(name).is_err(), "{name:?}");
        }
    }
}
