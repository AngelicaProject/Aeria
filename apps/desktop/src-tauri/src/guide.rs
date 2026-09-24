//! The project guidance and glossary editor.
//!
//! The user edits `aeria-guidance.md` and `aeria-glossary.csv` directly. A
//! save replaces a file only if it still has the content the editor loaded,
//! so a change made meanwhile by hand, by Git, or through an approved
//! Angelica proposal is never overwritten.

use aeria_ai::guidance::{
    GlossaryDiagnostic, GlossaryEntry, MAX_GUIDANCE_BYTES, ProjectFile, parse_glossary,
    read_project_file, write_glossary,
};
use serde::{Deserialize, Serialize};

use crate::angelica::{apply_file_change, repository_root};
use crate::commands::run_blocking;
use crate::error::CommandError;

type CommandResult<T> = Result<T, CommandError>;

/// Both files as the editor shows them.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGuideDto {
    /// The guidance text; `None` when the file does not exist.
    pub guidance: Option<String>,
    /// The glossary file's exact content, sent back when saving.
    pub glossary_text: Option<String>,
    pub entries: Vec<GlossaryEntry>,
    /// Rows excluded from the glossary, with the reason.
    pub diagnostics: Vec<GlossaryDiagnostic>,
    /// Why a file cannot be used at all.
    pub guidance_error: Option<String>,
    pub glossary_error: Option<String>,
}

/// One edited glossary entry.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlossaryEntryInput {
    pub term: String,
    pub translation: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub forbidden: Vec<String>,
}

fn guide_error(message: impl Into<String>) -> CommandError {
    CommandError::new("projectGuideInvalid", message)
}

fn load(root: &std::path::Path) -> ProjectGuideDto {
    let mut dto = ProjectGuideDto {
        guidance: None,
        glossary_text: None,
        entries: Vec::new(),
        diagnostics: Vec::new(),
        guidance_error: None,
        glossary_error: None,
    };
    match read_project_file(root, ProjectFile::Guidance) {
        Ok(text) => dto.guidance = text,
        Err(message) => dto.guidance_error = Some(message),
    }
    match read_project_file(root, ProjectFile::Glossary) {
        Ok(Some(text)) => {
            match parse_glossary(text.as_bytes()) {
                Ok(glossary) => {
                    dto.entries = glossary.entries;
                    dto.diagnostics = glossary.diagnostics;
                }
                Err(error) => dto.glossary_error = Some(error.to_string()),
            }
            dto.glossary_text = Some(text);
        }
        Ok(None) => {}
        Err(message) => dto.glossary_error = Some(message),
    }
    dto
}

/// The canonical glossary file for edited entries. Every entry must be
/// valid: the written file is parsed again and must exclude nothing.
fn glossary_file(entries: Vec<GlossaryEntryInput>) -> CommandResult<String> {
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
        })
        .collect();
    let text = write_glossary(&entries);
    let parsed = parse_glossary(text.as_bytes()).map_err(|error| guide_error(error.message))?;
    if let Some(problem) = parsed.diagnostics.first() {
        // Line 1 is the header, so entry N is on line N + 1.
        return Err(guide_error(format!(
            "entry {}: {}",
            problem.line.saturating_sub(1),
            problem.message
        )));
    }
    Ok(text)
}

fn save(
    root: &std::path::Path,
    file: ProjectFile,
    expected: Option<&str>,
    content: &str,
) -> CommandResult<ProjectGuideDto> {
    apply_file_change(root, file, expected, content).map_err(|(status, message)| {
        let code = if status == aeria_ai::conversation::ProposalStatus::Conflict {
            "projectGuideConflict"
        } else {
            "projectGuideWrite"
        };
        CommandError::new(code, message)
    })?;
    Ok(load(root))
}

#[tauri::command(rename_all = "camelCase")]
/// Reads the project guidance and glossary.
///
/// # Errors
///
/// Returns `noProjectOpen`.
pub async fn project_guide(app: tauri::AppHandle) -> CommandResult<ProjectGuideDto> {
    run_blocking(move || Ok(load(&repository_root(&app)?))).await
}

#[tauri::command(rename_all = "camelCase")]
/// Replaces the guidance if the file still has the `expected` content
/// (`None` when it did not exist).
///
/// # Errors
///
/// Returns `projectGuideInvalid` for text over 64 KiB,
/// `projectGuideConflict` when the file changed, or `projectGuideWrite`.
pub async fn save_project_guidance(
    app: tauri::AppHandle,
    expected: Option<String>,
    text: String,
) -> CommandResult<ProjectGuideDto> {
    if text.len() as u64 > MAX_GUIDANCE_BYTES {
        return Err(guide_error(format!(
            "the guidance is larger than {MAX_GUIDANCE_BYTES} bytes"
        )));
    }
    run_blocking(move || {
        save(
            &repository_root(&app)?,
            ProjectFile::Guidance,
            expected.as_deref(),
            &text,
        )
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Writes the glossary in canonical form if the file still has the
/// `expected` content. Rows the file excluded are not kept; the editor
/// confirms that with the user first.
///
/// # Errors
///
/// Returns `projectGuideInvalid` for an invalid entry,
/// `projectGuideConflict` when the file changed, or `projectGuideWrite`.
pub async fn save_project_glossary(
    app: tauri::AppHandle,
    expected: Option<String>,
    entries: Vec<GlossaryEntryInput>,
) -> CommandResult<ProjectGuideDto> {
    run_blocking(move || {
        let text = glossary_file(entries)?;
        save(
            &repository_root(&app)?,
            ProjectFile::Glossary,
            expected.as_deref(),
            &text,
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(term: &str, translation: &str) -> GlossaryEntryInput {
        GlossaryEntryInput {
            term: term.to_owned(),
            translation: translation.to_owned(),
            note: Some("  ".to_owned()),
            forbidden: vec![" Хай ".to_owned(), String::new()],
        }
    }

    #[test]
    fn edited_glossaries_are_canonical_and_complete() {
        let text = glossary_file(vec![
            entry(" Aether ", "Эфир"),
            entry("Crystal", "Кристалл"),
        ])
        .expect("valid");
        assert_eq!(
            text,
            "term,translation,note,forbidden\nAether,Эфир,,Хай\nCrystal,Кристалл,,Хай\n"
        );
        let duplicate = glossary_file(vec![entry("Aether", "Эфир"), entry("aether", "Эфир")])
            .expect_err("duplicate");
        assert!(
            duplicate.message.starts_with("entry 2:"),
            "{}",
            duplicate.message
        );
        assert!(glossary_file(vec![entry("Aether", " ")]).is_err());
    }

    #[test]
    fn saves_refuse_to_overwrite_changed_files() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path();
        let saved = save(root, ProjectFile::Guidance, None, "Be brief.\n").expect("create");
        assert_eq!(saved.guidance.as_deref(), Some("Be brief.\n"));
        let conflict = save(root, ProjectFile::Guidance, None, "Other.\n").expect_err("changed");
        assert_eq!(conflict.code, "projectGuideConflict");

        let text = glossary_file(vec![entry("Aether", "Эфир")]).expect("valid");
        let saved = save(root, ProjectFile::Glossary, None, &text).expect("glossary");
        assert_eq!(saved.entries.len(), 1);
        assert_eq!(saved.glossary_text.as_deref(), Some(text.as_str()));
        assert!(saved.diagnostics.is_empty());
    }
}
