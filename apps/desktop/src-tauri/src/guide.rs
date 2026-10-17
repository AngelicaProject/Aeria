//! The project knowledge editor: the style, the terms, and the sheets declared part of the glossary in `aeria-knowledge/` (see `docs/formats/knowledge-v1.md`).
//!
//! A save replaces a file only if it still has the content the editor
//! loaded, so a change made meanwhile by hand, by Git, or by an agent is
//! never overwritten.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aeria_knowledge::knowledge::read_file;
use aeria_knowledge::{
    GlossaryDiagnostic, GlossaryEntry, KnowledgeFile, TermSheet, parse_glossary, write_glossary,
    write_term_sheets,
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
    /// `term-sheets.csv` exactly as read, sent back when saving.
    pub sheets_text: Option<String>,
    /// The sheets declared part of the glossary.
    pub sheets: Vec<TermSheet>,
    /// The terms of the declared sheets.
    pub sheet_terms: Vec<SheetTermDto>,
    /// The headwords of terms of `terms.csv` the sheets give alike: taking
    /// them out changes nothing.
    pub redundant: Vec<String>,
    /// The terms of the sheets the entries take the place of.
    pub replaced: Vec<ReplacedTermDto>,
}

/// A term of the sheets an entry of `terms.csv` takes the place of.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacedTermDto {
    /// The entry, by its place in `entries`.
    pub entry: usize,
    pub term: SheetTermDto,
}

/// A translation of a name among the strings of the sheets.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantDto {
    pub translation: String,
    pub strings: usize,
}

/// A term of the declared sheets.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetTermDto {
    /// The sheet of its first string.
    pub sheet: String,
    /// The `msgctxt` of its first string, to open it in the editor.
    pub context: String,
    pub term: String,
    pub translation: String,
    pub category: String,
    /// The other spellings of the name in the sheets.
    pub forms: Vec<String>,
    /// How many strings of the sheets have the name.
    pub strings: usize,
    /// Each translation of the name among those strings, the term's first.
    pub variants: Vec<VariantDto>,
}

impl From<aeria_po::SheetTerm> for SheetTermDto {
    fn from(term: aeria_po::SheetTerm) -> Self {
        Self {
            sheet: term.sheet,
            context: term.context,
            term: term.term,
            translation: term.translation,
            category: term.category,
            forms: term.forms,
            strings: term.strings,
            variants: term
                .variants
                .into_iter()
                .map(|variant| VariantDto {
                    translation: variant.translation,
                    strings: variant.strings,
                })
                .collect(),
        }
    }
}

/// One declared sheet.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TermSheetInput {
    pub sheet: String,
    #[serde(default)]
    pub column: u32,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub note: Option<String>,
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
    pub forms: Vec<String>,
    #[serde(default)]
    pub match_case: bool,
    /// What kind of thing the term names in this meaning.
    #[serde(default)]
    pub category: String,
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
        sheets_text: read_file(root, KnowledgeFile::Sheets).ok().flatten(),
        sheets: Vec::new(),
        sheet_terms: Vec::new(),
        redundant: Vec::new(),
        replaced: Vec::new(),
    };
    let project = aeria_po::project_knowledge(root);
    dto.sheets = project.knowledge.sheets;
    dto.redundant = project.redundant;
    dto.sheet_terms = project.sheet_terms.into_iter().map(Into::into).collect();
    dto.replaced = project
        .replaced
        .into_iter()
        .map(|replaced| ReplacedTermDto {
            entry: replaced.entry,
            term: replaced.term.into(),
        })
        .collect();
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

fn check_size(file: KnowledgeFile, text: &str) -> CommandResult<()> {
    if text.len() as u64 > file.max_bytes() {
        return Err(knowledge_error(format!(
            "the file would be larger than {} bytes",
            file.max_bytes()
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
            category_note: None,
            forms: entry
                .forms
                .iter()
                .map(|form| form.trim().to_owned())
                .filter(|form| !form.is_empty())
                .collect(),
            match_case: entry.match_case,
            category: entry.category.trim().to_owned(),
        })
        .collect();
    let text = write_glossary(&entries);
    check_size(KnowledgeFile::Terms, &text)?;
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
    check_size(KnowledgeFile::Style, &text)?;
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

#[tauri::command(rename_all = "camelCase")]
/// Writes `term-sheets.csv` in canonical form if it still has the
/// `expected` content: the sheets declared part of the glossary.
///
/// # Errors
///
/// Returns `projectKnowledgeInvalid` for a sheet without a name or declared
/// twice, `projectKnowledgeConflict` when the file changed, or
/// `projectKnowledgeWrite`.
pub async fn save_knowledge_sheets(
    app: tauri::AppHandle,
    expected: Option<String>,
    sheets: Vec<TermSheetInput>,
) -> CommandResult<ProjectKnowledgeDto> {
    run_blocking(move || {
        let text = sheets_file(sheets)?;
        save(
            &repository_root(&app)?,
            KnowledgeFile::Sheets,
            expected.as_deref(),
            &text,
        )
    })
    .await
}

fn sheets_file(sheets: Vec<TermSheetInput>) -> CommandResult<String> {
    let mut declared: Vec<TermSheet> = Vec::new();
    let text = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
    for sheet in sheets {
        let sheet = TermSheet {
            sheet: sheet.sheet.trim().to_owned(),
            column: sheet.column,
            category: text(&sheet.category),
            note: sheet
                .note
                .as_deref()
                .map(text)
                .filter(|note| !note.is_empty()),
        };
        if sheet.sheet.is_empty() {
            return Err(knowledge_error("a sheet has no name"));
        }
        if sheet.category.is_empty() {
            return Err(knowledge_error(format!(
                "the sheet {:?} has no category",
                sheet.sheet
            )));
        }
        if declared
            .iter()
            .any(|known| known.sheet == sheet.sheet && known.column == sheet.column)
        {
            return Err(knowledge_error(format!(
                "the sheet {:?} is declared twice",
                sheet.sheet
            )));
        }
        declared.push(sheet);
    }
    let text = write_term_sheets(&declared);
    check_size(KnowledgeFile::Sheets, &text)?;
    Ok(text)
}

/// How many strings of the game use one term.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermUsageDto {
    pub term: String,
    pub category: String,
    pub strings: usize,
}

#[tauri::command(rename_all = "camelCase")]
/// How many strings of the project use each term of the glossary as the
/// project has it saved, to order the terms by use.
///
/// # Errors
///
/// Returns a typed command error when no project is open or `po/` cannot be
/// read.
pub async fn term_usage(app: tauri::AppHandle) -> CommandResult<Vec<TermUsageDto>> {
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let knowledge = session.knowledge();
        let usage = session
            .corpus()
            .term_usage(
                session.root(),
                &knowledge.terms,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .map_err(|error| CommandError::new("projectKnowledgeRead", error.to_string()))?;
        Ok(knowledge
            .terms
            .entries
            .iter()
            .zip(usage)
            .map(|(entry, strings)| TermUsageDto {
                term: entry.term.clone(),
                category: entry.category.clone(),
                strings,
            })
            .collect())
    })
    .await
}

/// The strings that use one term.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermStringsDto {
    pub strings: Vec<TermStringDto>,
    /// How many strings use the term; `strings` holds the first of them.
    pub total: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermStringDto {
    pub context: String,
    pub source: String,
    pub translation: String,
    pub fuzzy: bool,
}

/// Most strings of a term sent at once.
const TERM_STRINGS: usize = 300;

/// The issue group of the strings whose translation does not use a term, by
/// the term's name (see [`aeria_po::Issue::group`]).
const TERM_NOT_USED: &str = "termNotUsed";

/// A meaning of a term some translations do not use, and how many.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnappliedTermDto {
    pub term: String,
    pub category: String,
    pub strings: usize,
}

/// A meaning of a term, by its headword and category, ignoring case.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TermKeyInput {
    pub term: String,
    pub category: String,
}

/// A translated string whose translation does not use a term it should.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnappliedStringDto {
    /// The file, relative to `po/`.
    pub path: String,
    pub context: String,
    pub source: String,
    pub translation: String,
    pub fuzzy: bool,
}

/// The knowledge the checks used, and every translated string with a term
/// not used with the meanings, by their place in the glossary, it leaves
/// unused.
type Unapplied = (
    Arc<aeria_knowledge::Knowledge>,
    Vec<(aeria_po::Hit, Vec<usize>)>,
);

/// Every translated string with a term not used, and the meanings, by their
/// place in the glossary, each leaves unused: a finding names its term by
/// the headword of the meaning found, so the meanings of that headword the
/// source uses, as the uses of terms are counted, are the ones not applied.
fn unapplied(session: &aeria_po::Session) -> CommandResult<Unapplied> {
    let knowledge = session.knowledge();
    let query = aeria_po::Query {
        check: aeria_po::CheckFilter::Advice,
        issue: Some(TERM_NOT_USED.to_owned()),
        ..aeria_po::Query::default()
    };
    let hits = session
        .corpus()
        .search_all(
            session.root(),
            &query,
            &knowledge,
            &session.settings().target_language,
        )
        .map_err(|error| CommandError::new("projectKnowledgeRead", error.to_string()))?;
    let glossary = &knowledge.terms;
    let mut by_headword: std::collections::HashMap<&str, Vec<usize>> =
        std::collections::HashMap::new();
    for (index, entry) in glossary.entries.iter().enumerate() {
        by_headword
            .entry(entry.term.as_str())
            .or_default()
            .push(index);
    }
    let found = hits
        .into_iter()
        .map(|hit| {
            let mut meanings: Vec<usize> = Vec::new();
            for finding in &hit.findings {
                let aeria_po::Issue::TermNotUsed { term, .. } = finding else {
                    continue;
                };
                for &index in by_headword.get(term.as_str()).into_iter().flatten() {
                    if !meanings.contains(&index) && glossary.uses(&hit.source, index) {
                        meanings.push(index);
                    }
                }
            }
            (hit, meanings)
        })
        .collect();
    Ok((knowledge, found))
}

#[tauri::command(rename_all = "camelCase")]
/// For each meaning of a term of the saved glossary, how many translated
/// strings do not use it, as the checks find: the strings a correction run
/// decides. Unlike a project search it never cancels one.
///
/// # Errors
///
/// Returns a typed command error when no project is open or `po/` cannot be
/// read.
pub async fn term_unapplied(app: tauri::AppHandle) -> CommandResult<Vec<UnappliedTermDto>> {
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let (knowledge, found) = unapplied(&session)?;
        let mut counts: std::collections::BTreeMap<usize, usize> =
            std::collections::BTreeMap::new();
        for (_, meanings) in &found {
            for index in meanings {
                *counts.entry(*index).or_default() += 1;
            }
        }
        Ok(counts
            .into_iter()
            .map(|(index, strings)| {
                let entry = &knowledge.terms.entries[index];
                UnappliedTermDto {
                    term: entry.term.clone(),
                    category: entry.category.clone(),
                    strings,
                }
            })
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Every translated string that does not use one of the meanings `terms`
/// of the saved glossary, or any term when `terms` is empty, in file order:
/// what a correction run of those terms takes.
///
/// # Errors
///
/// Returns a typed command error when no project is open or `po/` cannot be
/// read.
pub async fn term_unapplied_strings(
    app: tauri::AppHandle,
    terms: Vec<TermKeyInput>,
) -> CommandResult<Vec<UnappliedStringDto>> {
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let (knowledge, found) = unapplied(&session)?;
        let wanted: Vec<usize> = knowledge
            .terms
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                terms.iter().any(|key| {
                    key.term.to_lowercase() == entry.term.to_lowercase()
                        && key.category.to_lowercase() == entry.category.to_lowercase()
                })
            })
            .map(|(index, _)| index)
            .collect();
        Ok(found
            .into_iter()
            .filter(|(_, meanings)| {
                !meanings.is_empty()
                    && (terms.is_empty() || meanings.iter().any(|index| wanted.contains(index)))
            })
            .map(|(hit, _)| UnappliedStringDto {
                path: hit.path,
                context: hit.context,
                source: hit.source,
                translation: hit.translation,
                fuzzy: hit.fuzzy,
            })
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// The strings of the project whose source uses the term of `term` and
/// `category`, as the project has it saved: the first few hundred and how
/// many there are. `None` when the saved glossary has no such term.
///
/// # Errors
///
/// Returns a typed command error when no project is open or `po/` cannot be
/// read.
pub async fn term_strings(
    app: tauri::AppHandle,
    term: String,
    category: String,
) -> CommandResult<Option<TermStringsDto>> {
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let knowledge = session.knowledge();
        let (name, kind) = (term.to_lowercase(), category.to_lowercase());
        let Some(entry) = knowledge.terms.entries.iter().position(|entry| {
            entry.term.to_lowercase() == name && entry.category.to_lowercase() == kind
        }) else {
            return Ok(None);
        };
        let (strings, total) = session
            .corpus()
            .term_strings(
                session.root(),
                &knowledge.terms,
                entry,
                TERM_STRINGS,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .map_err(|error| CommandError::new("projectKnowledgeRead", error.to_string()))?;
        Ok(Some(TermStringsDto {
            strings: strings
                .into_iter()
                .map(|string| TermStringDto {
                    context: string.context,
                    source: string.source,
                    translation: string.translation,
                    fuzzy: string.fuzzy,
                })
                .collect(),
            total,
        }))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Where the string with `context` is in the game, to open it in the
/// editor; `None` when the game has no such string.
///
/// # Errors
///
/// Returns a typed command error when no project is open.
pub async fn string_binding(
    app: tauri::AppHandle,
    context: String,
) -> CommandResult<Option<crate::dto::SourceBindingDto>> {
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        Ok(crate::search::binding(&session, &context))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_sheets_are_written_once_each() {
        let sheet = |sheet: &str, column| TermSheetInput {
            sheet: sheet.to_owned(),
            column,
            category: " задание ".to_owned(),
            note: None,
        };
        let noted = TermSheetInput {
            category: " место ".to_owned(),
            note: Some(" Названия\n мест. ".to_owned()),
            ..sheet("PlaceName", 2)
        };
        assert_eq!(
            sheets_file(vec![sheet(" Quest ", 0), noted]).expect("valid"),
            "sheet,column,category,note\nQuest,0,задание,\nPlaceName,2,место,Названия мест.\n"
        );
        assert!(sheets_file(vec![sheet("Quest", 0), sheet("Quest", 0)]).is_err());
        assert!(sheets_file(vec![sheet(" ", 0)]).is_err());
        let uncategorized = TermSheetInput {
            category: " ".to_owned(),
            ..sheet("Quest", 0)
        };
        assert!(sheets_file(vec![uncategorized]).is_err());
    }

    fn entry(term: &str, translation: &str) -> TermInput {
        TermInput {
            term: term.to_owned(),
            translation: translation.to_owned(),
            note: Some("  ".to_owned()),
            forms: vec![format!(" {}s ", term.trim()), String::new()],
            match_case: false,
            category: " понятие ".to_owned(),
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
            "term,translation,forms,note,category\nAether,Эфир,Aethers,,понятие\nCrystal,Кристалл,Crystals,,понятие\n"
        );
        let duplicate = terms_file(vec![entry("Aether", "Эфир"), entry("aether", "Эфир")])
            .expect_err("duplicate");
        assert!(
            duplicate.message.starts_with("entry 2:"),
            "{}",
            duplicate.message
        );
        assert!(terms_file(vec![entry("Aether", " ")]).is_err());
        let uncategorized = TermInput {
            category: " ".to_owned(),
            ..entry("Aether", "Эфир")
        };
        assert!(terms_file(vec![uncategorized]).is_err());
        let form = terms_file(vec![entry("Aether", "Эфир"), entry("Aethers", "Эфиры")])
            .expect_err("a form of another term");
        assert!(form.message.starts_with("entry 2:"), "{}", form.message);
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
        assert_eq!(saved.entries[0].category, "понятие");
        assert_eq!(saved.terms_text.as_deref(), Some(text.as_str()));
        assert!(saved.diagnostics.is_empty());
    }
}
