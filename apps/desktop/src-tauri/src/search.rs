//! Project search and bulk edits in the desktop: searching the open
//! project's files, replacing in translations after a preview, undoing the
//! last bulk edit, and translating found strings again (see
//! `docs/architecture/search.md`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use aeria_model::Options;
use aeria_po::{
    CheckFilter, EditKind, EditsApplied, EntryEdit, Field, Fields, Issue, MatchKind, Pattern,
    Query, Replacement, Session, SkipReason, State,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::commands::run_blocking;
use crate::dto::{IssueDto, SourceBindingDto};
use crate::error::CommandError;
use crate::state::DesktopState;
use crate::translate::{is_running, running, start_run};

type CommandResult<T> = Result<T, CommandError>;

/// The search in progress and the last bulk edit, which one undo reverts.
#[derive(Default)]
pub struct SearchState {
    cancel: Mutex<Arc<AtomicBool>>,
    undo: Mutex<Option<Vec<EntryEdit>>>,
}

impl SearchState {
    /// Cancels the search in progress and returns the flag of a new one.
    fn begin(&self) -> Arc<AtomicBool> {
        let mut cancel = self.cancel.lock().unwrap_or_else(PoisonError::into_inner);
        cancel.store(true, Ordering::Relaxed);
        *cancel = Arc::new(AtomicBool::new(false));
        Arc::clone(&cancel)
    }

    fn cancel(&self) {
        self.cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .store(true, Ordering::Relaxed);
    }

    fn set_undo(&self, applied: &EditsApplied) {
        *self.undo.lock().unwrap_or_else(PoisonError::into_inner) = (!applied.done.is_empty())
            .then(|| applied.done.iter().map(aeria_po::EditDone::undo).collect());
    }

    fn take_undo(&self) -> Option<Vec<EntryEdit>> {
        self.undo
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn has_undo(&self) -> bool {
        self.undo
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchKindDto {
    Text,
    Word,
    Regex,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldDto {
    Translation,
    Source,
    Note,
    Context,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum StateDto {
    Untranslated,
    Translated,
    Fuzzy,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CheckDto {
    #[default]
    Any,
    Problems,
    Advice,
}

/// A search from the renderer.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQueryDto {
    /// Empty to keep every string the filters keep.
    pub text: String,
    pub kind: MatchKindDto,
    pub case_sensitive: bool,
    /// The fields the text is matched in.
    pub fields: Vec<FieldDto>,
    /// Files and folders relative to `po/`; empty for the whole project.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Only these strings, by `msgctxt`.
    #[serde(default)]
    pub contexts: Vec<String>,
    #[serde(default)]
    pub states: Vec<StateDto>,
    #[serde(default)]
    pub check: CheckDto,
    /// With a check filter, only strings with an issue of this group.
    #[serde(default)]
    pub issue: Option<String>,
}

impl SearchQueryDto {
    fn query(&self) -> Query {
        let has = |field| self.fields.contains(&field);
        Query {
            pattern: (!self.text.is_empty()).then(|| Pattern {
                text: self.text.clone(),
                kind: match self.kind {
                    MatchKindDto::Text => MatchKind::Text,
                    MatchKindDto::Word => MatchKind::Word,
                    MatchKindDto::Regex => MatchKind::Regex,
                },
                case_sensitive: self.case_sensitive,
            }),
            fields: Fields {
                translation: has(FieldDto::Translation),
                source: has(FieldDto::Source),
                note: has(FieldDto::Note),
                context: has(FieldDto::Context),
            },
            paths: self.paths.clone(),
            contexts: self.contexts.clone(),
            states: self
                .states
                .iter()
                .map(|state| match state {
                    StateDto::Untranslated => State::Untranslated,
                    StateDto::Translated => State::Translated,
                    StateDto::Fuzzy => State::Fuzzy,
                })
                .collect(),
            check: match self.check {
                CheckDto::Any => CheckFilter::Any,
                CheckDto::Problems => CheckFilter::Problems,
                CheckDto::Advice => CheckFilter::Advice,
            },
            issue: self.issue.clone(),
        }
    }
}

fn issues_dto(issues: &[Issue]) -> Vec<IssueDto> {
    issues.iter().map(IssueDto::from).collect()
}

/// A message without a kind, such as a broken file.
fn other_issue(message: &str) -> IssueDto {
    IssueDto {
        kind: "other".to_owned(),
        group: "other".to_owned(),
        message: message.to_owned(),
        ..IssueDto::default()
    }
}

/// The matches in one field, as UTF-16 ranges of its text for the renderer.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldMatchDto {
    pub field: FieldDto,
    pub ranges: Vec<[usize; 2]>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHitDto {
    pub path: String,
    pub context: String,
    /// The string in the game; `None` when the game no longer has it.
    pub binding: Option<SourceBindingDto>,
    pub source: String,
    pub translation: String,
    pub fuzzy: bool,
    pub note: Option<String>,
    pub matches: Vec<FieldMatchDto>,
    /// Problems, or terms not used, when the search filters by them.
    pub findings: Vec<IssueDto>,
}

/// A string a search found, with what a bulk action on it needs: the
/// whole result of a search, which is not cut.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchEntryDto {
    pub path: String,
    pub context: String,
    pub translation: String,
    pub fuzzy: bool,
    /// Problems, or terms not used, when the search filters by them.
    pub findings: Vec<IssueDto>,
}

/// The strings found in one file.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHitsDto {
    pub path: String,
    pub sheet: String,
    pub count: usize,
}

/// How many strings have issues of one group.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCountDto {
    pub issue: IssueDto,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultDto {
    pub hits: Vec<SearchHitDto>,
    /// Every string found; more than `hits` when the result was cut.
    pub total: usize,
    pub matches: usize,
    /// Every file with a string found, with counts, in file order.
    pub files: Vec<FileHitsDto>,
    /// With a check filter, the issues of every string found by group.
    pub issues: Vec<IssueCountDto>,
    pub cancelled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementDto {
    pub text: String,
    pub preserve_case: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceChangeDto {
    pub path: String,
    pub context: String,
    pub binding: Option<SourceBindingDto>,
    pub source: String,
    pub before: String,
    pub after: String,
    pub fuzzy: bool,
    /// A change with problems is not written.
    pub problems: Vec<IssueDto>,
}

/// A string to change, as the renderer last saw it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryRefDto {
    pub path: String,
    pub context: String,
    pub expected_text: String,
    pub expected_fuzzy: bool,
}

/// A replacement to write.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceEditDto {
    #[serde(flatten)]
    pub entry: EntryRefDto,
    pub after: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReasonDto {
    Changed,
    Missing,
    Invalid,
    Broken,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedDto {
    pub path: String,
    pub context: String,
    pub binding: Option<SourceBindingDto>,
    pub reason: SkipReasonDto,
    pub problems: Vec<IssueDto>,
}

/// What a bulk edit did.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkEditDto {
    pub done: usize,
    pub skipped: Vec<SkippedDto>,
    /// An undo of the last bulk edit is available.
    pub undo_available: bool,
}

pub(crate) fn binding(session: &Session, context: &str) -> Option<SourceBindingDto> {
    session
        .coordinate_of(context)
        .map(
            |(sheet_name, row_id, subrow_id, column_index)| SourceBindingDto {
                sheet_name,
                row_id,
                subrow_id,
                column_index,
            },
        )
}

/// UTF-16 offsets of byte ranges of `text`.
fn utf16_ranges(text: &str, ranges: &[std::ops::Range<usize>]) -> Vec<[usize; 2]> {
    let offset = |byte: usize| text[..byte].encode_utf16().count();
    ranges
        .iter()
        .map(|range| [offset(range.start), offset(range.end)])
        .collect()
}

fn hit_dto(session: &Session, hit: aeria_po::Hit) -> SearchHitDto {
    let matches = hit
        .matches
        .iter()
        .map(|found| {
            let (field, text) = match found.field {
                Field::Translation => (FieldDto::Translation, hit.translation.as_str()),
                Field::Source => (FieldDto::Source, hit.source.as_str()),
                Field::Note => (FieldDto::Note, hit.note.as_deref().unwrap_or_default()),
                Field::Context => (FieldDto::Context, hit.context.as_str()),
            };
            FieldMatchDto {
                field,
                ranges: utf16_ranges(text, &found.ranges),
            }
        })
        .collect();
    SearchHitDto {
        binding: binding(session, &hit.context),
        path: hit.path,
        context: hit.context,
        source: hit.source,
        translation: hit.translation,
        fuzzy: hit.fuzzy,
        note: hit.note,
        matches,
        findings: issues_dto(&hit.findings),
    }
}

fn search_error(error: &aeria_po::SearchError) -> CommandError {
    let code = match error {
        aeria_po::SearchError::Pattern(_) | aeria_po::SearchError::EmptyPattern => "searchPattern",
        aeria_po::SearchError::Read { .. } => "translationRead",
    };
    CommandError::new(code, error.to_string())
}

fn bulk_dto(session: &Session, applied: &EditsApplied, state: &SearchState) -> BulkEditDto {
    BulkEditDto {
        done: applied.done.len(),
        skipped: applied
            .skipped
            .iter()
            .map(|skipped| {
                let (reason, problems) = match &skipped.reason {
                    SkipReason::Changed => (SkipReasonDto::Changed, Vec::new()),
                    SkipReason::Missing => (SkipReasonDto::Missing, Vec::new()),
                    SkipReason::Invalid(problems) => (SkipReasonDto::Invalid, issues_dto(problems)),
                    SkipReason::Broken(message) => {
                        (SkipReasonDto::Broken, vec![other_issue(message)])
                    }
                };
                SkippedDto {
                    binding: binding(session, &skipped.context),
                    path: skipped.path.clone(),
                    context: skipped.context.clone(),
                    reason,
                    problems,
                }
            })
            .collect(),
        undo_available: state.has_undo(),
    }
}

#[tauri::command(rename_all = "camelCase")]
/// Searches the open project's files. A new search cancels the one in
/// progress.
///
/// # Errors
///
/// Returns `searchPattern` for an invalid pattern, `noProjectOpen`, or
/// `translationRead` when a file cannot be read.
pub async fn project_search(
    app: tauri::AppHandle,
    query: SearchQueryDto,
) -> CommandResult<SearchResultDto> {
    let cancel = app.state::<SearchState>().begin();
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let knowledge = session.knowledge();
        let target = session.settings().target_language;
        let found = aeria_po::search(session.root(), &query.query(), &knowledge, &target, &cancel)
            .map_err(|error| search_error(&error))?;
        Ok(SearchResultDto {
            total: found.total,
            matches: found.matches,
            files: found
                .files
                .into_iter()
                .map(|file| FileHitsDto {
                    path: file.path,
                    sheet: file.sheet,
                    count: file.count,
                })
                .collect(),
            issues: found
                .issues
                .iter()
                .map(|count| IssueCountDto {
                    issue: IssueDto::from(&count.issue),
                    count: count.count,
                })
                .collect(),
            cancelled: found.cancelled,
            hits: found
                .hits
                .into_iter()
                .map(|hit| hit_dto(&session, hit))
                .collect(),
        })
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Cancels the search in progress.
#[allow(clippy::needless_pass_by_value)]
pub fn project_search_cancel(app: tauri::AppHandle) {
    app.state::<SearchState>().cancel();
}

/// A string where a glossary candidate is rendered one way.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermExampleDto {
    pub path: String,
    pub context: String,
    pub binding: Option<SourceBindingDto>,
    pub source: String,
    pub translation: String,
}

/// One way the project renders a glossary candidate.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermRenderingDto {
    pub words: Vec<String>,
    pub strings: usize,
    pub examples: Vec<TermExampleDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetCountDto {
    pub sheet: String,
    pub count: usize,
}

/// A name the project renders in several ways.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermCandidateDto {
    pub phrase: String,
    pub strings: usize,
    pub translated: usize,
    /// The most used rendering first.
    pub renderings: Vec<TermRenderingDto>,
    /// Every rendering sounds like the name: spellings of one name.
    pub spellings: bool,
    pub sheets: Vec<SheetCountDto>,
}

#[tauri::command(rename_all = "camelCase")]
/// The names of the open project's sources that its translations render in
/// several ways and that the glossary does not have (see
/// `aeria_po::term_candidates`), those affecting most strings first.
///
/// # Errors
///
/// Returns `noProjectOpen`, or `translationRead` when a file cannot be read.
pub async fn project_term_candidates(
    app: tauri::AppHandle,
) -> CommandResult<Vec<TermCandidateDto>> {
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let knowledge = session.knowledge();
        let found = aeria_po::term_candidates(session.root(), &knowledge)
            .map_err(|error| search_error(&error))?;
        Ok(found
            .into_iter()
            .map(|candidate| TermCandidateDto {
                phrase: candidate.phrase,
                strings: candidate.strings,
                translated: candidate.translated,
                renderings: candidate
                    .renderings
                    .into_iter()
                    .map(|rendering| TermRenderingDto {
                        words: rendering.words,
                        strings: rendering.strings,
                        examples: rendering
                            .examples
                            .into_iter()
                            .map(|example| TermExampleDto {
                                binding: binding(&session, &example.context),
                                path: example.path,
                                context: example.context,
                                source: example.source,
                                translation: example.translation,
                            })
                            .collect(),
                    })
                    .collect(),
                spellings: candidate.spellings,
                sheets: candidate
                    .sheets
                    .into_iter()
                    .map(|(sheet, count)| SheetCountDto { sheet, count })
                    .collect(),
            })
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Every string the query finds, without the limit of `project_search`, for
/// choosing a whole result or sheet. It is not cancelled by a new search, so
/// it never returns part of the result.
///
/// # Errors
///
/// As `project_search`.
pub async fn project_search_entries(
    app: tauri::AppHandle,
    query: SearchQueryDto,
) -> CommandResult<Vec<SearchEntryDto>> {
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let knowledge = session.knowledge();
        let target = session.settings().target_language;
        let hits = aeria_po::search_all(session.root(), &query.query(), &knowledge, &target)
            .map_err(|error| search_error(&error))?;
        Ok(hits
            .into_iter()
            .map(|hit| SearchEntryDto {
                findings: issues_dto(&hit.findings),
                path: hit.path,
                context: hit.context,
                translation: hit.translation,
                fuzzy: hit.fuzzy,
            })
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// The changes a replacement would make to the translations the query
/// finds, each with the problems that would keep it from being written.
///
/// # Errors
///
/// Returns `searchPattern` for an invalid or empty pattern, `noProjectOpen`,
/// or `translationRead`.
pub async fn project_replace_preview(
    app: tauri::AppHandle,
    query: SearchQueryDto,
    replacement: ReplacementDto,
) -> CommandResult<Vec<ReplaceChangeDto>> {
    let cancel = app.state::<SearchState>().begin();
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let knowledge = session.knowledge();
        let target = session.settings().target_language;
        let changes = aeria_po::preview_replace(
            session.root(),
            &query.query(),
            &Replacement {
                text: replacement.text,
                preserve_case: replacement.preserve_case,
            },
            &knowledge,
            &target,
            &cancel,
        )
        .map_err(|error| search_error(&error))?;
        Ok(changes
            .into_iter()
            .map(|change| ReplaceChangeDto {
                binding: binding(&session, &change.context),
                path: change.path,
                context: change.context,
                source: change.source,
                before: change.before,
                after: change.after,
                fuzzy: change.fuzzy,
                problems: issues_dto(&change.problems),
            })
            .collect())
    })
    .await
}

fn edit(entry: EntryRefDto, kind: EditKind) -> EntryEdit {
    EntryEdit {
        path: entry.path,
        context: entry.context,
        expected_text: entry.expected_text,
        expected_fuzzy: entry.expected_fuzzy,
        kind,
    }
}

#[tauri::command(rename_all = "camelCase")]
/// Writes replacements of translations. A string changed since the preview,
/// or whose new translation has problems, is skipped and reported. The edit
/// becomes the one an undo reverts.
///
/// # Errors
///
/// Returns `noProjectOpen` or `translationPersistence` when a file cannot be
/// written.
pub async fn project_replace_apply(
    app: tauri::AppHandle,
    edits: Vec<ReplaceEditDto>,
) -> CommandResult<BulkEditDto> {
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let edits: Vec<EntryEdit> = edits
            .into_iter()
            .map(|edit_dto| edit(edit_dto.entry, EditKind::Replace(edit_dto.after)))
            .collect();
        let applied = session.apply_edits(&edits)?;
        let state = app.state::<SearchState>();
        state.set_undo(&applied);
        Ok(bulk_dto(&session, &applied, &state))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Reverts the last bulk edit for the strings that did not change since;
/// the others are reported.
///
/// # Errors
///
/// Returns `nothingToUndo`, `noProjectOpen`, or `translationPersistence`.
pub async fn project_edit_undo(app: tauri::AppHandle) -> CommandResult<BulkEditDto> {
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let state = app.state::<SearchState>();
        let Some(undo) = state.take_undo() else {
            return Err(CommandError::new(
                "nothingToUndo",
                "there is no bulk edit to undo",
            ));
        };
        let applied = session.apply_edits(&undo)?;
        Ok(bulk_dto(&session, &applied, &state))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Adds or removes a term exception of the given strings. A string changed
/// since it was found is skipped and reported. The edit becomes the one an
/// undo reverts.
///
/// # Errors
///
/// Returns `termExceptionInvalid`, `noProjectOpen`, or
/// `translationPersistence` when a file cannot be written.
pub async fn project_term_exception(
    app: tauri::AppHandle,
    entries: Vec<EntryRefDto>,
    term: String,
    add: bool,
) -> CommandResult<BulkEditDto> {
    if add && !aeria_po::can_be_exception(&term) {
        return Err(crate::commands::term_exception_invalid(&term));
    }
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let edits: Vec<EntryEdit> = entries
            .into_iter()
            .map(|entry| {
                edit(
                    entry,
                    EditKind::TermException {
                        term: term.clone(),
                        add,
                    },
                )
            })
            .collect();
        let applied = session.apply_edits(&edits)?;
        let state = app.state::<SearchState>();
        state.set_undo(&applied);
        Ok(bulk_dto(&session, &applied, &state))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Clears the translations of the given strings and starts machine
/// translation of exactly those strings. The clear is the edit an undo
/// reverts. A stopped run leaves the rest untranslated, so any later run
/// takes them.
///
/// # Errors
///
/// Returns `translationRunning` while a run goes, `noProjectOpen`,
/// `translationPersistence`, or an error of the credential store.
pub async fn project_retranslate(
    app: tauri::AppHandle,
    entries: Vec<EntryRefDto>,
    model: String,
    effort: Option<String>,
) -> CommandResult<BulkEditDto> {
    if is_running(&app) {
        return Err(running());
    }
    let (applied_dto, contexts, paths) = {
        let app = app.clone();
        run_blocking(move || {
            let session = app.state::<DesktopState>().session()?;
            let edits: Vec<EntryEdit> = entries
                .into_iter()
                .map(|entry| edit(entry, EditKind::Clear))
                .collect();
            let applied = session.apply_edits(&edits)?;
            let state = app.state::<SearchState>();
            state.set_undo(&applied);
            let contexts: Vec<String> = applied
                .done
                .iter()
                .map(|done| done.context.clone())
                .collect();
            let mut paths: Vec<String> =
                applied.done.iter().map(|done| done.path.clone()).collect();
            paths.sort();
            paths.dedup();
            Ok((bulk_dto(&session, &applied, &state), contexts, paths))
        })
        .await?
    };
    if !contexts.is_empty() {
        start_run(
            &app,
            Options {
                paths,
                fuzzy: false,
                contexts,
                model,
                effort: effort.filter(|effort| !effort.is_empty()),
            },
        )?;
    }
    Ok(applied_dto)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_utf16_offsets() {
        let text = "Ёж 🦔 повар";
        let start = text.find("повар").expect("found");
        assert_eq!(
            utf16_ranges(text, std::slice::from_ref(&(start..text.len()))),
            vec![[6, 11]],
            "the hedgehog is two UTF-16 units"
        );
    }

    #[test]
    fn a_query_from_the_renderer_maps_to_the_core_query() {
        let dto: SearchQueryDto = serde_json::from_value(serde_json::json!({
            "text": "повар",
            "kind": "word",
            "caseSensitive": false,
            "fields": ["translation"],
            "states": ["fuzzy"],
            "check": "problems"
        }))
        .expect("query");
        let query = dto.query();
        assert_eq!(
            query.pattern.map(|pattern| pattern.kind),
            Some(MatchKind::Word)
        );
        assert!(query.fields.translation && !query.fields.source);
        assert_eq!(query.states, vec![State::Fuzzy]);
        assert_eq!(query.check, CheckFilter::Problems);
    }
}
