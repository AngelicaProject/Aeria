//! The string guide of the editor: what a request tells the model about the
//! selected string (`aeria_model::hints`), and what the checks find in the
//! translation being typed, before it is saved.

use aeria_model::hints::{HintTerm, hints};
use aeria_po::length::{self, Budget, Unit, length_budget};
use aeria_po::{Entry, Session};
use serde::Serialize;
use tauri::Manager;

use crate::commands::run_blocking;
use crate::dto::{IssueDto, SourceBindingDto};
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HintTermDto {
    pub term: String,
    pub translation: String,
    pub note: Option<String>,
    /// What the term names in this meaning.
    pub category: String,
    /// The words of its translation that are never declined.
    pub indeclinable: Vec<String>,
    /// A person, or a correction run's model, decided the term does not
    /// apply to this string.
    pub excepted: bool,
    /// The decision is the model's.
    pub by_model: bool,
    /// The words a correction run's model decided render the term in this
    /// string's translation.
    pub rendered: Vec<String>,
}

impl From<HintTerm> for HintTermDto {
    fn from(term: HintTerm) -> Self {
        Self {
            term: term.term,
            translation: term.translation,
            note: term.note,
            category: term.category,
            indeclinable: term.indeclinable,
            excepted: term.excepted,
            by_model: term.by_model,
            rendered: term.rendered,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HintSpeakerDto {
    /// The speaker label of the line, such as `ALPHINAUD`.
    pub label: String,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LengthBudgetDto {
    pub max: usize,
    /// `characters` shown, macros not counted, for an interface label;
    /// `bytes` of the encoded string for a world object's name.
    pub unit: &'static str,
}

impl From<Budget> for LengthBudgetDto {
    fn from(budget: Budget) -> Self {
        Self {
            max: budget.max,
            unit: match budget.unit {
                Unit::Characters => "characters",
                Unit::Bytes => "bytes",
            },
        }
    }
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StringHintsDto {
    pub terms: Vec<HintTermDto>,
    pub speaker: Option<HintSpeakerDto>,
    /// The kind of a quest's text: `journal`, `objective`, or `other`.
    pub kind: Option<String>,
    /// The longest an interface label's translation may show, or a world
    /// object's name may be.
    pub max_length: Option<LengthBudgetDto>,
    /// `source` and the client languages (`fr`, `de`) whose line varies
    /// with the player character's gender.
    pub gendered: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftCheckDto {
    /// What the checks find, as for a saved translation.
    pub issues: Vec<IssueDto>,
    /// The game data of the source the text lacks, each by its spelling in
    /// the source.
    pub missing: Vec<String>,
    /// The length of the text in the unit of the string's length budget:
    /// the bytes of a world object's name, otherwise the characters it
    /// shows, macros not counted.
    pub length: usize,
}

fn entry_of(
    session: &Session,
    binding: &SourceBindingDto,
) -> CommandResult<(String, Option<Entry>)> {
    Ok(session.entry(
        &binding.sheet_name,
        binding.row_id,
        binding.subrow_id,
        binding.column_index,
    )?)
}

fn string_hints_with(
    state: &DesktopState,
    binding: &SourceBindingDto,
) -> CommandResult<StringHintsDto> {
    let session = state.session()?;
    let (_, Some(entry)) = entry_of(&session, binding)? else {
        return Ok(StringHintsDto::default());
    };
    let found = hints(&session.knowledge().terms, &entry);
    Ok(StringHintsDto {
        terms: found.terms.into_iter().map(HintTermDto::from).collect(),
        speaker: found.speaker.map(|label| HintSpeakerDto { label }),
        kind: found.kind,
        max_length: found.max_length.map(LengthBudgetDto::from),
        gendered: found.gendered,
    })
}

fn check_draft_with(
    state: &DesktopState,
    binding: &SourceBindingDto,
    text: &str,
) -> CommandResult<DraftCheckDto> {
    let session = state.session()?;
    let (_, entry) = entry_of(&session, binding)?;
    let Some(entry) = entry else {
        return Ok(DraftCheckDto {
            issues: Vec::new(),
            missing: Vec::new(),
            length: length::visible_length(text),
        });
    };
    let localizations: Vec<&str> = entry
        .extracted
        .iter()
        .filter_map(|line| {
            ["ja: ", "de: ", "fr: "]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix))
        })
        .collect();
    Ok(DraftCheckDto {
        issues: session
            .check_text(&entry, text)
            .iter()
            .map(IssueDto::from)
            .collect(),
        missing: if text.is_empty() {
            Vec::new()
        } else {
            aeria_se::missing_game_data(&entry.source, text, &localizations)
        },
        length: length::length_in(
            length_budget(&entry).map_or(Unit::Characters, |budget| budget.unit),
            text,
        ),
    })
}

#[tauri::command(rename_all = "camelCase")]
/// The guide of a string: the project's terms in its source, its speaker or kind, the length of an interface label, and
/// whether its line varies with the player character's gender, as a
/// machine translation request reads them.
///
/// # Errors
///
/// Returns a typed command error when no project is open, the string is not
/// an entry of the project, or its file cannot be read.
pub async fn string_hints(
    app: tauri::AppHandle,
    source_binding: SourceBindingDto,
) -> CommandResult<StringHintsDto> {
    run_blocking(move || string_hints_with(&app.state::<DesktopState>(), &source_binding)).await
}

#[tauri::command(rename_all = "camelCase")]
/// What the checks find in `text` as the translation of a string, before it
/// is saved: the issues a save reports, the source's game data it lacks,
/// and its length.
///
/// # Errors
///
/// Returns a typed command error when no project is open, the string is not
/// an entry of the project, or its file cannot be read.
pub async fn check_draft(
    app: tauri::AppHandle,
    source_binding: SourceBindingDto,
    text: String,
) -> CommandResult<DraftCheckDto> {
    run_blocking(move || check_draft_with(&app.state::<DesktopState>(), &source_binding, &text))
        .await
}

#[cfg(test)]
mod tests {
    use aeria_knowledge::{GlossaryEntry, KnowledgeFile, write_glossary};

    use super::*;
    use crate::commands::initialize_with_game;
    use crate::test_support::{open, test_game};

    fn binding(sheet: &str, row: u32, column: u32) -> SourceBindingDto {
        SourceBindingDto {
            sheet_name: sheet.to_owned(),
            row_id: row,
            subrow_id: 0,
            column_index: column,
        }
    }

    #[test]
    fn the_guide_reads_a_string_and_checks_its_draft() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path().join("project");
        std::fs::create_dir_all(&root).expect("root");
        let game = test_game();
        let state = DesktopState::new();
        initialize_with_game(
            &state,
            &root,
            open(game.path()),
            &root.join("../cache"),
            "fr",
        )
        .expect("initialize");
        std::fs::write(
            KnowledgeFile::Terms.path(&root),
            write_glossary(&[GlossaryEntry {
                term: "Hello".to_owned(),
                translation: "Bonjour".to_owned(),
                forms: vec!["Hi".to_owned()],
                category: "приветствие".to_owned(),
                ..GlossaryEntry::default()
            }]),
        )
        .expect("terms");

        let hello = binding("Synthetic", 42, 0);
        let found = string_hints_with(&state, &hello).expect("hints");
        assert_eq!(found.terms.len(), 1);
        assert_eq!(found.terms[0].translation, "Bonjour");
        assert!(!found.terms[0].excepted);
        assert_eq!(found.max_length, None);

        let draft = check_draft_with(&state, &hello, "Salut <i>là</i>").expect("check");
        assert!(
            draft.issues.iter().any(|issue| issue.kind == "termNotUsed"),
            "{:?}",
            draft.issues
        );
        assert_eq!(draft.length, "Salut là".chars().count());
        assert!(draft.missing.is_empty());
        let empty = check_draft_with(&state, &hello, "").expect("empty");
        assert!(empty.issues.is_empty());

        let label = string_hints_with(&state, &binding("Addon", 1, 1)).expect("label");
        assert!(
            label
                .max_length
                .as_ref()
                .is_some_and(|length| length.unit == "characters" && length.max >= "Confirm".len()),
            "the longest of the label and its German and French lines: {:?}",
            label.max_length
        );

        let aide = binding("ENpcResident", 1_019_070, 0);
        let name = string_hints_with(&state, &aide).expect("name");
        assert_eq!(
            name.max_length,
            Some(LengthBudgetDto {
                max: 63,
                unit: "bytes"
            })
        );
        let draft = check_draft_with(
            &state,
            &aide,
            "служащий торговой компании «Восточный Альденард»",
        )
        .expect("check");
        assert_eq!(
            draft.length, 92,
            "a world object's name is counted in bytes"
        );
        assert_eq!(
            draft
                .issues
                .iter()
                .map(|issue| (issue.kind.as_str(), issue.advice, issue.length, issue.max))
                .collect::<Vec<_>>(),
            vec![("nameTooLong", true, Some(92), Some(63))]
        );
    }
}
