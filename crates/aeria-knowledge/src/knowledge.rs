//! Loading the knowledge files.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::glossary::{Glossary, GlossaryEntry, parse_glossary};

/// The knowledge directory at the project root.
pub const KNOWLEDGE_DIR: &str = "aeria-knowledge";
/// Largest knowledge file read.
pub const MAX_KNOWLEDGE_BYTES: u64 = 8 * 1024 * 1024;

/// A knowledge file.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KnowledgeFile {
    Style,
    Terms,
}

impl KnowledgeFile {
    pub const ALL: [Self; 2] = [Self::Style, Self::Terms];

    /// The file name inside [`KNOWLEDGE_DIR`].
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Style => "style.md",
            Self::Terms => "terms.csv",
        }
    }

    /// The path relative to the project root, with `/`.
    #[must_use]
    pub fn relative_path(self) -> String {
        format!("{KNOWLEDGE_DIR}/{}", self.file_name())
    }

    #[must_use]
    pub fn path(self, root: &Path) -> PathBuf {
        root.join(KNOWLEDGE_DIR).join(self.file_name())
    }

    /// The text of the file in a new project: a starting style of the usual
    /// decisions for the translators to change, and terms with no entries.
    #[must_use]
    pub fn empty(self) -> String {
        match self {
            Self::Style => include_str!("default_style.md").to_owned(),
            Self::Terms => crate::glossary::write_glossary(&[]),
        }
    }
}

/// Writes every knowledge file the project does not have yet, with no
/// entries. Returns the files written, relative to the root.
///
/// # Errors
///
/// Returns a description when a file cannot be written.
pub fn create_empty(root: &Path) -> Result<Vec<String>, String> {
    let directory = root.join(KNOWLEDGE_DIR);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let mut written = Vec::new();
    for file in KnowledgeFile::ALL {
        let path = file.path(root);
        if path.exists() {
            continue;
        }
        std::fs::write(&path, file.empty())
            .map_err(|error| format!("{}: {error}", path.display()))?;
        written.push(file.relative_path());
    }
    Ok(written)
}

/// The project's knowledge.
#[derive(Clone, Debug, Default)]
pub struct Knowledge {
    /// `style.md` as written.
    pub style: Option<String>,
    pub terms: Glossary,
    /// Why a file or an entry could not be used, with the file and line.
    pub problems: Vec<String>,
}

/// The files' texts, for building knowledge without a project.
#[derive(Clone, Debug, Default)]
pub struct KnowledgeTexts {
    pub style: Option<String>,
    pub terms: Option<String>,
}

/// Reads one knowledge file as text. A missing file is `None`.
///
/// # Errors
///
/// Returns a description when the file is too large, unreadable, or not
/// UTF-8.
pub fn read_file(root: &Path, file: KnowledgeFile) -> Result<Option<String>, String> {
    let path = file.path(root);
    let name = file.relative_path();
    match fs::metadata(&path) {
        Ok(metadata) if metadata.len() > MAX_KNOWLEDGE_BYTES => {
            return Err(format!("{name} is larger than {MAX_KNOWLEDGE_BYTES} bytes"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{name}: {error}")),
    }
    let bytes = fs::read(&path).map_err(|error| format!("{name}: {error}"))?;
    String::from_utf8(bytes)
        .map(|text| Some(text.trim_start_matches('\u{feff}').to_owned()))
        .map_err(|_| format!("{name} is not UTF-8"))
}

impl Knowledge {
    /// Reads the knowledge of a project. Missing files are normal; unusable
    /// files and entries are reported in `problems` and left out.
    #[must_use]
    pub fn load(root: &Path) -> Self {
        let mut problems = Vec::new();
        let mut read = |file: KnowledgeFile| match read_file(root, file) {
            Ok(text) => text,
            Err(problem) => {
                problems.push(problem);
                None
            }
        };
        let texts = KnowledgeTexts {
            style: read(KnowledgeFile::Style),
            terms: read(KnowledgeFile::Terms),
        };
        let mut knowledge = Self::from_texts(&texts);
        knowledge.problems.splice(0..0, problems);
        knowledge
    }

    /// Builds knowledge from the files' texts.
    #[must_use]
    pub fn from_texts(texts: &KnowledgeTexts) -> Self {
        let mut problems = Vec::new();
        let terms_name = KnowledgeFile::Terms.relative_path();
        let terms = match texts
            .terms
            .as_deref()
            .map(|text| parse_glossary(text.as_bytes()))
        {
            Some(Ok(glossary)) => {
                for diagnostic in &glossary.diagnostics {
                    problems.push(format!(
                        "{terms_name} line {}: {}",
                        diagnostic.line, diagnostic.message
                    ));
                }
                glossary
            }
            Some(Err(error)) => {
                problems.push(error.to_string());
                Glossary::default()
            }
            None => Glossary::default(),
        };
        Self {
            style: texts.style.clone(),
            terms,
            problems,
        }
    }

    /// Terms that occur in `text`.
    #[must_use]
    pub fn terms_in(&self, text: &str) -> Vec<&GlossaryEntry> {
        self.terms.matches(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_files_read_without_problems_and_are_never_replaced() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path();
        assert_eq!(create_empty(root).expect("create").len(), 2);
        let knowledge = Knowledge::load(root);
        assert!(knowledge.problems.is_empty(), "{:?}", knowledge.problems);
        assert!(knowledge.terms.entries.is_empty());
        fs::write(KnowledgeFile::Style.path(root), "Use вы.\n").expect("write");
        assert!(create_empty(root).expect("again").is_empty());
        assert_eq!(Knowledge::load(root).style.as_deref(), Some("Use вы.\n"));
    }

    #[test]
    fn terms_are_found_in_a_text() {
        let knowledge = Knowledge::from_texts(&KnowledgeTexts {
            style: None,
            terms: Some(
                "term,translation,forms\nAether,Эфир,aethers\nMoogle,Моогл,,\n,broken,,\n"
                    .to_owned(),
            ),
        });
        assert_eq!(knowledge.problems.len(), 1, "{:?}", knowledge.problems);
        let found = knowledge.terms_in("Some aether, kupo.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].translation, "Эфир");
    }

    #[test]
    fn files_are_read_from_the_knowledge_directory() {
        let directory = tempfile::tempdir().expect("temp");
        let root = directory.path();
        assert!(Knowledge::load(root).problems.is_empty());
        fs::create_dir(root.join(KNOWLEDGE_DIR)).expect("dir");
        fs::write(
            KnowledgeFile::Terms.path(root),
            "\u{feff}term,translation\nAether,Эфир\n",
        )
        .expect("terms");
        fs::write(KnowledgeFile::Style.path(root), [0xff, 0xfe]).expect("style");
        let knowledge = Knowledge::load(root);
        assert_eq!(knowledge.terms.entries.len(), 1);
        assert_eq!(
            knowledge.problems,
            ["aeria-knowledge/style.md is not UTF-8"]
        );
    }
}
