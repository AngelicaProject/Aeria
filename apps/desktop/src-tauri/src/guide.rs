//! The project knowledge editor: the style and terms in `aeria-knowledge/` (see `docs/formats/knowledge-v1.md`).
//!
//! A save replaces a file only if it still has the content the editor
//! loaded, so a change made meanwhile by hand, by Git, or by an agent is
//! never overwritten.

use std::path::{Path, PathBuf};

use aeria_knowledge::knowledge::read_file;
use aeria_knowledge::{
    GlossaryDiagnostic, GlossaryEntry, KnowledgeFile, MAX_KNOWLEDGE_BYTES, parse_glossary,
    write_glossary,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// The files as the editor shows them.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectKnowledgeDto {
    /// `style.md`; `None` when the file does not exist.
    pub style: Option<String>,
    /// `terms.csv` exactly as read, sent back when saving.
    pub terms_text: Option<String>,
    pub entries: Vec<GlossaryEntry>,
    /// Rows excluded from the terms, with the reason.
    pub diagnostics: Vec<GlossaryDiagnostic>,
    /// Why a file cannot be used at all.
    pub style_error: Option<String>,
    pub terms_error: Option<String>,
}

/// One edited term.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TermInput {
    pub term: String,
    pub translation: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub forbidden: Vec<String>,
    #[serde(default)]
    pub settled: bool,
    #[serde(default)]
    pub match_case: bool,
}

fn knowledge_error(message: impl Into<String>) -> CommandError {
    CommandError::new("projectKnowledgeInvalid", message)
}

fn repository_root(app: &tauri::AppHandle) -> CommandResult<PathBuf> {
    Ok(app.state::<DesktopState>().session()?.root().to_owned())
}

fn load(root: &Path) -> ProjectKnowledgeDto {
    let mut dto = ProjectKnowledgeDto {
        style: None,
        terms_text: None,
        entries: Vec::new(),
        diagnostics: Vec::new(),
        style_error: None,
        terms_error: None,
    };
    match read_file(root, KnowledgeFile::Style) {
        Ok(text) => dto.style = text,
        Err(message) => dto.style_error = Some(message),
    }
    match read_file(root, KnowledgeFile::Terms) {
        Ok(Some(text)) => {
            match parse_glossary(text.as_bytes()) {
                Ok(glossary) => {
                    dto.entries = glossary.entries;
                    dto.diagnostics = glossary.diagnostics;
                }
                Err(error) => dto.terms_error = Some(error.to_string()),
            }
            dto.terms_text = Some(text);
        }
        Ok(None) => {}
        Err(message) => dto.terms_error = Some(message),
    }
    dto
}

fn check_size(text: &str) -> CommandResult<()> {
    if text.len() as u64 > MAX_KNOWLEDGE_BYTES {
        return Err(knowledge_error(format!(
            "the file would be larger than {MAX_KNOWLEDGE_BYTES} bytes"
        )));
    }
    Ok(())
}

/// The canonical terms file for edited entries. Every entry must be valid:
/// the written file is parsed again and must exclude nothing.
fn terms_file(entries: Vec<TermInput>) -> CommandResult<String> {
    let entries: Vec<GlossaryEntry> = entries
        .into_iter()
        .map(|entry| GlossaryEntry {
            term: entry.term.trim().to_owned(),
            translation: entry.translation.trim().to_owned(),
            note: entry
                .note
                .map(|note| note.trim().to_owned())
                .filter(|note| !note.is_empty()),
            forbidden: entry
                .forbidden
                .iter()
                .map(|variant| variant.trim().to_owned())
                .filter(|variant| !variant.is_empty())
                .collect(),
            settled: entry.settled,
            match_case: entry.match_case,
        })
        .collect();
    let text = write_glossary(&entries);
    check_size(&text)?;
    let parsed = parse_glossary(text.as_bytes()).map_err(|error| knowledge_error(error.message))?;
    if let Some(problem) = parsed.diagnostics.first() {
        // Line 1 is the header, so entry N is on line N + 1.
        return Err(knowledge_error(format!(
            "entry {}: {}",
            problem.line.saturating_sub(1),
            problem.message
        )));
    }
    Ok(text)
}

/// Replaces a knowledge file if it still has the content the change was
/// made against, through a temporary file and rename.
fn save(
    root: &Path,
    file: KnowledgeFile,
    expected: Option<&str>,
    content: &str,
) -> CommandResult<ProjectKnowledgeDto> {
    let current = read_file(root, file)
        .map_err(|message| CommandError::new("projectKnowledgeWrite", message))?;
    if current.as_deref() != expected {
        return Err(CommandError::new(
            "projectKnowledgeConflict",
            format!("{} changed after it was loaded", file.relative_path()),
        ));
    }
    let path = file.path(root);
    let directory = path.parent().unwrap_or(root);
    let partial = directory.join(format!(".{}.partial", file.file_name()));
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(directory)?;
        let mut handle = std::fs::File::create(&partial)?;
        std::io::Write::write_all(&mut handle, content.as_bytes())?;
        handle.sync_all()?;
        drop(handle);
        std::fs::rename(&partial, &path)
    };
    write().map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        CommandError::new(
            "projectKnowledgeWrite",
            format!("{}: {error}", file.relative_path()),
        )
    })?;
    Ok(load(root))
}

#[tauri::command(rename_all = "camelCase")]
/// Reads the project's style and terms.
///
/// # Errors
///
/// Returns `noProjectOpen`.
pub async fn project_knowledge(app: tauri::AppHandle) -> CommandResult<ProjectKnowledgeDto> {
    run_blocking(move || Ok(load(&repository_root(&app)?))).await
}

#[tauri::command(rename_all = "camelCase")]
/// Replaces `style.md` if it still has the `expected` content (`None` when
/// it did not exist).
///
/// # Errors
///
/// Returns `projectKnowledgeInvalid` for a file that is too large,
/// `projectKnowledgeConflict` when the file changed, or
/// `projectKnowledgeWrite`.
pub async fn save_knowledge_style(
    app: tauri::AppHandle,
    expected: Option<String>,
    text: String,
) -> CommandResult<ProjectKnowledgeDto> {
    check_size(&text)?;
    run_blocking(move || {
        save(
            &repository_root(&app)?,
            KnowledgeFile::Style,
            expected.as_deref(),
            &text,
        )
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Writes `terms.csv` in canonical form if it still has the `expected`
/// content. Rows the file excluded are not kept; the editor confirms that
/// with the user first.
///
/// # Errors
///
/// Returns `projectKnowledgeInvalid` for an invalid entry,
/// `projectKnowledgeConflict` when the file changed, or
/// `projectKnowledgeWrite`.
pub async fn save_knowledge_terms(
    app: tauri::AppHandle,
    expected: Option<String>,
    entries: Vec<TermInput>,
) -> CommandResult<ProjectKnowledgeDto> {
    run_blocking(move || {
        let text = terms_file(entries)?;
        save(
            &repository_root(&app)?,
            KnowledgeFile::Terms,
            expected.as_deref(),
            &text,
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(term: &str, translation: &str) -> TermInput {
        TermInput {
            term: term.to_owned(),
            translation: translation.to_owned(),
            note: Some("  ".to_owned()),
            forbidden: vec![" Хай ".to_owned(), String::new()],
            settled: true,
            match_case: false,
        }
    }

    #[test]
    fn edited_terms_are_canonical_and_complete() {
        let text = terms_file(vec![
            entry(" Aether ", "Эфир"),
            entry("Crystal", "Кристалл"),
        ])
        .expect("valid");
        assert_eq!(
            text,
            "term,translation,note,forbidden,settled\nAether,Эфир,,Хай,yes\nCrystal,Кристалл,,Хай,yes\n"
        );
        let duplicate = terms_file(vec![entry("Aether", "Эфир"), entry("aether", "Эфир")])
            .expect_err("duplicate");
        assert!(
            duplicate.message.starts_with("entry 2:"),
            "{}",
            duplicate.message
        );
        assert!(terms_file(vec![entry("Aether", " ")]).is_err());
    }

    #[test]
    fn saves_refuse_to_overwrite_changed_files() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path();
        let saved =
            save(root, KnowledgeFile::Style, None, "## general\nBe brief.\n").expect("create");
        assert_eq!(saved.style.as_deref(), Some("## general\nBe brief.\n"));
        let conflict = save(root, KnowledgeFile::Style, None, "Other.\n").expect_err("changed");
        assert_eq!(conflict.code, "projectKnowledgeConflict");

        let text = terms_file(vec![entry("Aether", "Эфир")]).expect("valid");
        let saved = save(root, KnowledgeFile::Terms, None, &text).expect("terms");
        assert_eq!(saved.entries.len(), 1);
        assert!(saved.entries[0].settled);
        assert_eq!(saved.terms_text.as_deref(), Some(text.as_str()));
        assert!(saved.diagnostics.is_empty());
    }
}
