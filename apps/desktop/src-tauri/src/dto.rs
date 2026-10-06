use std::path::Path;

use aeria_po::{CellView, Page, RowView, Session, SheetProgress, Translation, Updated};
use aeria_projects::RegistryEntry;
use serde::{Deserialize, Serialize};

use crate::error::CommandError;

/// The text of one source cell in another client language, for comparison.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherLanguageTextDto {
    /// The language code, such as `de`.
    pub language: String,
    /// The cell's macro text; `None` when that language has no such cell.
    pub text: Option<String>,
}

/// A string of the game by its coordinate, accepted and returned by desktop
/// commands.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBindingDto {
    pub sheet_name: String,
    pub row_id: u32,
    pub subrow_id: u16,
    pub column_index: u32,
}

/// One sheet of the game with its size.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSheetDto {
    pub name: String,
    pub row_count: usize,
    pub translatable_cell_count: usize,
    /// The sheet is listed by the game but cannot be read.
    pub unavailable: bool,
}

/// Owned metadata for the currently active project.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummaryDto {
    pub repository_root: String,
    pub source_language: String,
    pub target_language: String,
    /// The game version the project's files are for, which is the installed
    /// game's version while the project is open.
    pub game_version: String,
    /// The game installation the project reads.
    pub game_path: String,
    pub sheets: Vec<ProjectSheetDto>,
}

impl ProjectSummaryDto {
    pub(crate) fn from_session(session: &Session) -> Self {
        let settings = session.settings();
        let catalog = session.source().catalog().unwrap_or_default();
        Self {
            repository_root: session.root().to_string_lossy().into_owned(),
            source_language: settings.source_language.clone(),
            target_language: settings.target_language.clone(),
            game_version: session.source().version().to_string(),
            game_path: session.source().game_path().to_string_lossy().into_owned(),
            sheets: catalog
                .iter()
                .map(|sheet| ProjectSheetDto {
                    name: sheet.name.clone(),
                    row_count: sheet.rows,
                    translatable_cell_count: sheet.translatable,
                    unavailable: sheet.unavailable,
                })
                .collect(),
        }
    }
}

/// How much of one sheet is translated. Sheets without strings are omitted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetProgressDto {
    pub sheet_name: String,
    /// The sheet's strings in the project.
    pub strings: usize,
    pub translated: usize,
    /// Translations whose source changed since they were written.
    pub fuzzy: usize,
}

impl From<SheetProgress> for SheetProgressDto {
    fn from(progress: SheetProgress) -> Self {
        Self {
            sheet_name: progress.sheet,
            strings: progress.entries,
            translated: progress.translated,
            fuzzy: progress.fuzzy,
        }
    }
}

/// The result of a successful project launch. Local recent-project persistence
/// is convenience state, so its failure is returned as a warning while the
/// active project remains usable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOpenResultDto {
    pub project: ProjectSummaryDto,
    pub warning: Option<CommandError>,
    /// Present when opening updated the project to the installed game.
    pub source_update: Option<SourceUpdateReportDto>,
}

/// The outcome of opening a project from a game installation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum GameOpenResultDto {
    /// The project is open and describes the game.
    Opened { result: Box<ProjectOpenResultDto> },
    /// The project's files are for an older game version. Nothing was
    /// written; after confirmation the renderer updates the project.
    SourceUpdateRequired { update: SourceUpdateNeededDto },
}

/// The game versions of a project that needs an update.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceUpdateNeededDto {
    pub previous_game_version: String,
    pub game_version: String,
}

/// What updating a project to the installed game did.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceUpdateReportDto {
    pub previous_game_version: String,
    pub game_version: String,
    /// Files written or removed.
    pub files: usize,
    /// Translations marked fuzzy because their source changed.
    pub fuzzy: usize,
    /// Translations kept as obsolete because their string left the game.
    pub obsolete: usize,
    /// The commit that recorded the update, when one was made.
    pub commit: Option<String>,
}

impl SourceUpdateReportDto {
    pub(crate) fn new(
        previous_game_version: String,
        game_version: String,
        updated: Updated,
        commit: Option<String>,
    ) -> Self {
        Self {
            previous_game_version,
            game_version,
            files: updated.files,
            fuzzy: updated.fuzzy,
            obsolete: updated.obsolete,
            commit,
        }
    }
}

/// Cheap filesystem-only presentation state for one recent project.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecentProjectAvailability {
    Ready,
    RepositoryMissing,
}

/// Frontend-facing metadata for a local recent project.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProjectDto {
    pub id: String,
    pub repository_root: String,
    pub source_language: String,
    pub target_language: String,
    pub game_version: String,
    pub last_opened_at_unix_ms: u64,
    pub availability: RecentProjectAvailability,
}

impl RecentProjectDto {
    pub(crate) fn from_registry_entry(entry: RegistryEntry) -> Self {
        let availability = if Path::new(&entry.repository_root).is_dir() {
            RecentProjectAvailability::Ready
        } else {
            RecentProjectAvailability::RepositoryMissing
        };
        Self {
            id: entry.id,
            repository_root: entry.repository_root,
            source_language: entry.source_language,
            target_language: entry.target_language,
            game_version: entry.game_version,
            last_opened_at_unix_ms: entry.last_opened_at_unix_ms,
            availability,
        }
    }
}

/// A row/subrow cursor for the bounded desktop translation read.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRowCursorDto {
    pub sheet_name: String,
    pub row_id: u32,
    pub subrow_id: u16,
}

/// One read-only context cell in a row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationContextCellDto {
    pub column_index: u32,
    pub source_macro: String,
}

/// One string of a row and its translation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationCellDto {
    pub source_binding: SourceBindingDto,
    pub source_macro: String,
    /// The source has no letters outside protected structure.
    pub formatting_only: bool,
    pub translation: Option<TranslationOverlayDto>,
}

/// One row of a sheet for the desktop editor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRowDto {
    pub sheet_name: String,
    pub row_id: u32,
    pub subrow_id: u16,
    pub context: Vec<TranslationContextCellDto>,
    pub cells: Vec<TranslationCellDto>,
}

impl TranslationRowDto {
    pub(crate) fn new(sheet_name: &str, row: RowView) -> Self {
        Self {
            sheet_name: sheet_name.to_owned(),
            row_id: row.row,
            subrow_id: row.subrow,
            context: row
                .context
                .into_iter()
                .map(|(column_index, source_macro)| TranslationContextCellDto {
                    column_index,
                    source_macro,
                })
                .collect(),
            cells: row
                .cells
                .into_iter()
                .map(|cell: CellView| TranslationCellDto {
                    source_binding: SourceBindingDto {
                        sheet_name: sheet_name.to_owned(),
                        row_id: row.row,
                        subrow_id: row.subrow,
                        column_index: cell.column,
                    },
                    source_macro: cell.source,
                    formatting_only: cell.formatting_only,
                    translation: cell.translation.map(Into::into),
                })
                .collect(),
        }
    }
}

/// One bounded page of rows with their translations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRowPageDto {
    pub rows: Vec<TranslationRowDto>,
    pub next_after: Option<TranslationRowCursorDto>,
}

impl TranslationRowPageDto {
    pub(crate) fn new(sheet_name: &str, page: Page) -> Self {
        Self {
            rows: page
                .rows
                .into_iter()
                .map(|row| TranslationRowDto::new(sheet_name, row))
                .collect(),
            next_after: page
                .next_after
                .map(|(row_id, subrow_id)| TranslationRowCursorDto {
                    sheet_name: sheet_name.to_owned(),
                    row_id,
                    subrow_id,
                }),
        }
    }
}

/// The translation of one string as its entry holds it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationOverlayDto {
    /// The translation; empty when the string is not translated.
    pub target_macro: String,
    /// The source changed since the translation was written.
    pub fuzzy: bool,
    pub translator_note: Option<String>,
    /// The source the translation was written for, while it is fuzzy.
    pub previous_source: Option<String>,
    /// Terms a person decided do not apply to the string.
    #[serde(default)]
    pub term_exceptions: Vec<String>,
    /// A person reviewed the translation as it is; machine translation
    /// leaves it.
    #[serde(default)]
    pub reviewed: bool,
    /// The string has a review mark of another text: the translation
    /// changed after the review.
    #[serde(default)]
    pub review_stale: bool,
}

impl From<Translation> for TranslationOverlayDto {
    fn from(translation: Translation) -> Self {
        Self {
            target_macro: translation.text,
            fuzzy: translation.fuzzy,
            translator_note: translation.note,
            previous_source: translation.previous,
            term_exceptions: translation.term_exceptions,
            reviewed: translation.reviewed,
            review_stale: translation.review_stale,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use aeria_projects::RegistryEntry;

    #[test]
    fn a_page_maps_rows_cells_and_translations() {
        let page = Page {
            rows: vec![RowView {
                row: 42,
                subrow: 0,
                context: vec![(3, "Context field".to_owned())],
                cells: vec![
                    CellView {
                        column: 0,
                        source: "source".to_owned(),
                        formatting_only: false,
                        translation: Some(Translation {
                            text: String::new(),
                            fuzzy: true,
                            note: Some("check later".to_owned()),
                            previous: None,
                            term_exceptions: Vec::new(),
                            reviewed: false,
                            review_stale: false,
                        }),
                    },
                    CellView {
                        column: 1,
                        source: "...".to_owned(),
                        formatting_only: true,
                        translation: None,
                    },
                ],
            }],
            next_after: Some((7, 0)),
        };
        let dto = TranslationRowPageDto::new("Synthetic", page);
        assert_eq!(dto.rows[0].row_id, 42);
        assert_eq!(dto.rows[0].context[0].source_macro, "Context field");
        let cell = &dto.rows[0].cells[0];
        assert_eq!(cell.source_binding.sheet_name, "Synthetic");
        let overlay = cell.translation.as_ref().expect("overlay");
        assert!(overlay.fuzzy);
        assert_eq!(overlay.translator_note.as_deref(), Some("check later"));
        assert!(dto.rows[0].cells[1].translation.is_none());
        assert!(dto.rows[0].cells[1].formatting_only);
        assert_eq!(dto.next_after.expect("cursor").row_id, 7);
    }

    #[test]
    fn recent_availability_checks_only_the_repository_folder() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let base = RegistryEntry {
            id: "local-id".to_owned(),
            repository_root: repository.to_string_lossy().into_owned(),
            source_language: "en".to_owned(),
            target_language: "fr".to_owned(),
            game_version: "2026.09.15.0000.0000".to_owned(),
            last_opened_at_unix_ms: 1,
        };
        assert_eq!(
            RecentProjectDto::from_registry_entry(base.clone()).availability,
            RecentProjectAvailability::Ready
        );
        let mut missing = base;
        missing.repository_root = repository
            .join("missing-repository")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            RecentProjectDto::from_registry_entry(missing).availability,
            RecentProjectAvailability::RepositoryMissing
        );
    }
}

/// One finding of the checks, as data the renderer words in its language;
/// `message` is the English text for kinds it does not know.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDto {
    /// `lineBreak`, `structure`, `forbiddenTerm`, `mark`, `mixedAlphabets`,
    /// `bothGenders`, `termNotUsed`, `genderNotVaried`,
    /// `genderInOtherLanguages`, `machinePhrasing`, `staleTermException`,
    /// `labelTooLong`, `nameTooLong`, or `other`.
    pub kind: String,
    /// What may be wrong rather than a problem: it does not keep the
    /// translation from being saved or exported.
    #[serde(default)]
    pub advice: bool,
    /// What groups issues in the summary, and filters by them.
    pub group: String,
    pub message: String,
    pub term: Option<String>,
    pub translation: Option<String>,
    pub variant: Option<String>,
    /// The word with mixed alphabets, the form with both genders, or the
    /// mark as `U+0301`.
    pub text: Option<String>,
    pub phrases: Vec<String>,
    /// The length of a translation longer than its budget, and the budget:
    /// characters for a label, bytes for a name.
    pub length: Option<usize>,
    pub max: Option<usize>,
}

impl From<&aeria_po::Issue> for IssueDto {
    fn from(issue: &aeria_po::Issue) -> Self {
        let base = Self {
            group: issue.group(),
            message: issue.to_string(),
            advice: !issue.is_problem(),
            ..Self::default()
        };
        match issue {
            aeria_po::Issue::LineBreak => Self {
                kind: "lineBreak".to_owned(),
                ..base
            },
            aeria_po::Issue::Structure(_) => Self {
                kind: "structure".to_owned(),
                ..base
            },
            aeria_po::Issue::ForbiddenTerm {
                term,
                translation,
                variant,
            } => Self {
                kind: "forbiddenTerm".to_owned(),
                term: Some(term.clone()),
                translation: Some(translation.clone()),
                variant: Some(variant.clone()),
                ..base
            },
            aeria_po::Issue::Mark(mark) => Self {
                kind: "mark".to_owned(),
                text: Some(format!("U+{:04X}", u32::from(*mark))),
                ..base
            },
            aeria_po::Issue::MixedAlphabets(word) => Self {
                kind: "mixedAlphabets".to_owned(),
                text: Some(word.clone()),
                ..base
            },
            aeria_po::Issue::BothGenders(form) => Self {
                kind: "bothGenders".to_owned(),
                text: Some(form.clone()),
                ..base
            },
            aeria_po::Issue::TermNotUsed { term, translation } => Self {
                kind: "termNotUsed".to_owned(),
                term: Some(term.clone()),
                translation: Some(translation.clone()),
                ..base
            },
            aeria_po::Issue::GenderNotVaried => Self {
                kind: "genderNotVaried".to_owned(),
                ..base
            },
            aeria_po::Issue::GenderInOtherLanguages => Self {
                kind: "genderInOtherLanguages".to_owned(),
                ..base
            },
            aeria_po::Issue::MachinePhrasing(phrases) => Self {
                kind: "machinePhrasing".to_owned(),
                phrases: phrases.clone(),
                ..base
            },
            aeria_po::Issue::StaleTermException(term) => Self {
                kind: "staleTermException".to_owned(),
                term: Some(term.clone()),
                ..base
            },
            aeria_po::Issue::LabelTooLong { length, max } => Self {
                kind: "labelTooLong".to_owned(),
                length: Some(*length),
                max: Some(*max),
                ..base
            },
            aeria_po::Issue::NameTooLong { length, max } => Self {
                kind: "nameTooLong".to_owned(),
                length: Some(*length),
                max: Some(*max),
                ..base
            },
        }
    }
}
