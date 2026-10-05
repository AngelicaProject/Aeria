//! The string guide of the editor: what a request tells the model about the
//! selected string (`aeria_model::hints`), and what the checks find in the
//! translation being typed, before it is saved.

use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::SystemTime;

use aeria_model::fit;
use aeria_model::hints::{HintTerm, hints};
use aeria_model::names::{Names, name_sheet_of};
use aeria_po::{Entry, PO_DIR, Session};
use serde::Serialize;
use tauri::Manager;

use crate::commands::run_blocking;
use crate::dto::{IssueDto, SourceBindingDto};
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// A file's modification time and length.
type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &std::path::Path) -> Stamp {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

/// The translated names of the open project, read again when a file of a
/// name sheet changed. Reading them takes a second or two on a project of
/// the whole game, so the guide of every string shares one reading.
pub struct NamesCache {
    loaded: Option<LoadedNames>,
}

struct LoadedNames {
    session: Weak<Session>,
    files: Vec<(PathBuf, Stamp)>,
    names: Arc<Names>,
}

impl NamesCache {
    pub const fn new() -> Self {
        Self { loaded: None }
    }

    pub(crate) fn names(&mut self, session: &Arc<Session>) -> Arc<Names> {
        if let Some(loaded) = &self.loaded
            && loaded
                .session
                .upgrade()
                .is_some_and(|seen| Arc::ptr_eq(&seen, session))
            && loaded.files.iter().all(|(path, seen)| stamp(path) == *seen)
        {
            return Arc::clone(&loaded.names);
        }
        let root = session.root();
        let files = aeria_po::list(root)
            .unwrap_or_default()
            .into_iter()
            .filter(|path| name_sheet_of(path).is_some())
            .map(|path| {
                let full = root.join(PO_DIR).join(path);
                let seen = stamp(&full);
                (full, seen)
            })
            .collect();
        let names = Arc::new(Names::load(root));
        self.loaded = Some(LoadedNames {
            session: Arc::downgrade(session),
            files,
            names: Arc::clone(&names),
        });
        names
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HintNameDto {
    pub name: String,
    pub translation: String,
}

/// A game name of a string's source with the string its translation comes
/// from.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HintGameNameDto {
    pub name: String,
    pub translation: String,
    /// Every name sheet with this name and translation, such as `Action`
    /// and `Item` for `Potion`.
    pub sheets: Vec<String>,
    /// The row's name when this is another form of it, such as `The Walk`
    /// for `Walk`.
    pub full: Option<String>,
    /// The string the translation comes from, to open it in the editor.
    pub binding: Option<SourceBindingDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HintTermDto {
    pub term: String,
    pub translation: String,
    pub note: Option<String>,
    /// Variants never to use.
    pub never: Vec<String>,
    /// A person decided the term does not apply to this string.
    pub excepted: bool,
}

impl From<HintTerm> for HintTermDto {
    fn from(term: HintTerm) -> Self {
        Self {
            term: term.term,
            translation: term.translation,
            note: term.note,
            never: term.never,
            excepted: term.excepted,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HintSpeakerDto {
    /// The speaker label of the line, such as `ALPHINAUD`.
    pub label: String,
    /// The name the label stands for and its translation, when the project
    /// translates that name.
    pub name: Option<HintNameDto>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StringHintsDto {
    pub names: Vec<HintGameNameDto>,
    pub terms: Vec<HintTermDto>,
    pub speaker: Option<HintSpeakerDto>,
    /// The kind of a quest's text: `journal`, `objective`, or `other`.
    pub kind: Option<String>,
    /// The most characters an interface label's translation may show.
    pub max_length: Option<usize>,
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
    /// The names of the guide whose translation the text uses.
    pub names_used: Vec<String>,
    /// The characters the text shows, macros not counted, as the length of
    /// an interface label is counted.
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
    let (path, Some(entry)) = entry_of(&session, binding)? else {
        return Ok(StringHintsDto::default());
    };
    let names = state.names_cache().names(&session);
    let found = hints(&names, &session.knowledge().terms, &path, &entry);
    Ok(StringHintsDto {
        names: found
            .names
            .into_iter()
            .map(|name| HintGameNameDto {
                binding: session.coordinate_of(&name.context).map(
                    |(sheet_name, row_id, subrow_id, column_index)| SourceBindingDto {
                        sheet_name,
                        row_id,
                        subrow_id,
                        column_index,
                    },
                ),
                sheets: if name.sheets.is_empty() {
                    vec![name.sheet().to_owned()]
                } else {
                    name.sheets
                },
                name: name.source,
                translation: name.translation,
                full: name.full,
            })
            .collect(),
        terms: found.terms.into_iter().map(HintTermDto::from).collect(),
        speaker: found.speaker.map(|(label, name)| HintSpeakerDto {
            label,
            name: name.map(|(name, translation)| HintNameDto { name, translation }),
        }),
        kind: found.kind,
        max_length: found.max_length,
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
            names_used: Vec::new(),
            length: fit::visible_length(text),
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
    let names = state.names_cache().names(&session);
    let names_used = if text.is_empty() {
        Vec::new()
    } else {
        names
            .in_texts([entry.source.as_str()], usize::MAX)
            .into_iter()
            .filter(|name| aeria_knowledge::uses_translation(text, &name.translation))
            .map(|name| name.source)
            .collect()
    };
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
        names_used,
        length: fit::visible_length(text),
    })
}

#[tauri::command(rename_all = "camelCase")]
/// The guide of a string: the game's names and the project's terms in its
/// source, its speaker or kind, the length of an interface label, and
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
/// the names of the guide it uses, and its length.
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
                forbidden: vec!["Salut".to_owned()],
                ..GlossaryEntry::default()
            }]),
        )
        .expect("terms");

        let hello = binding("Synthetic", 42, 0);
        let found = string_hints_with(&state, &hello).expect("hints");
        assert_eq!(found.terms.len(), 1);
        assert_eq!(found.terms[0].translation, "Bonjour");
        assert_eq!(found.terms[0].never, ["Salut"]);
        assert!(!found.terms[0].excepted);
        assert!(found.names.is_empty());
        assert_eq!(found.max_length, None);

        let draft = check_draft_with(&state, &hello, "Salut <i>là</i>").expect("check");
        assert!(
            draft
                .issues
                .iter()
                .any(|issue| issue.kind == "forbiddenTerm"),
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
                .is_some_and(|length| length >= "Confirm".len()),
            "the longest of the label and its German and French lines: {:?}",
            label.max_length
        );
    }
}
