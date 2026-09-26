//! Desktop adapter for project search and translation memory.
//!
//! The source index of the active project's game is built once per source
//! language and game version, in the background from the session's shared
//! game source, so building never holds the project lock. Searches read the
//! index without the lock and take it only to add translations from the
//! workspace.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use aeria_ai::search::{MemoryMatch, ProjectSearch, SearchMatch, SearchMatches, SearchQuery};
use aeria_ai::tools::{ToolError, UnitLocation};
use aeria_core::SourceBinding;
use aeria_search::{SearchError, SimilarSource, SourceHit, SourceIndex, SourceQuery, Tokenizer};
use aeria_source::GameSource;
use aeria_workspace::ProjectSession;
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::angelica::review_label;
use crate::paths::AeriaPaths;
use crate::state::DesktopState;

const SEARCH_DIRECTORY: &str = "search";
/// Index candidates read to find translated similar strings.
const MEMORY_CANDIDATES: usize = 100;

/// The index state of one source language and game version.
#[derive(Clone, Debug)]
pub(crate) enum IndexState {
    Building,
    Ready(SourceIndex),
    Failed(String),
}

/// The key of the game data an index is built from.
fn source_key(source: &GameSource) -> String {
    format!("{}/{}", source.language(), source.version())
}

fn index_path(app: &tauri::AppHandle, key: &str) -> Result<PathBuf, ToolError> {
    let data = app.aeria_data_dir().map_err(|error| {
        ToolError::new(format!(
            "could not resolve the Aeria app-data directory: {error}"
        ))
    })?;
    let digest = Sha256::digest(key.as_bytes());
    let key = digest[..16]
        .iter()
        .fold(String::with_capacity(32), |mut key, byte| {
            let _ = write!(key, "{byte:02x}");
            key
        });
    Ok(data.join(SEARCH_DIRECTORY).join(format!("{key}.sqlite3")))
}

/// What building an index needs, read under the project lock.
struct BuildInput {
    key: String,
    source: Arc<GameSource>,
    tokenizer: Tokenizer,
}

fn active_source_key(app: &tauri::AppHandle) -> Result<String, ToolError> {
    let state = app.state::<DesktopState>();
    let project = state
        .lock_project()
        .map_err(|error| ToolError::new(error.message))?;
    let session = project
        .as_ref()
        .ok_or_else(|| ToolError::new("no project is open"))?;
    Ok(source_key(session.source()))
}

fn build_input(app: &tauri::AppHandle) -> Result<BuildInput, ToolError> {
    let state = app.state::<DesktopState>();
    let project = state
        .lock_project()
        .map_err(|error| ToolError::new(error.message))?;
    let session = project
        .as_ref()
        .ok_or_else(|| ToolError::new("no project is open"))?;
    let source = session.source_handle();
    Ok(BuildInput {
        key: source_key(&source),
        tokenizer: Tokenizer::for_language(source.language().code()),
        source,
    })
}

fn build(app: &tauri::AppHandle, input: &BuildInput) -> Result<SourceIndex, SearchError> {
    let path = index_path(app, &input.key)
        .map_err(|error| SearchError::Io(std::io::Error::other(error.0)))?;
    SourceIndex::build(path, &input.key, input.tokenizer, &input.source, &|| true)
}

/// Returns the active game data's key and index, or starts building the
/// index and reports that it is not ready yet. A failed build is retried on
/// the next request.
pub(crate) fn source_index(app: &tauri::AppHandle) -> Result<(String, SourceIndex), ToolError> {
    let key = active_source_key(app)?;
    let state = app.state::<DesktopState>();
    match state.search_index(&key) {
        Some(IndexState::Ready(index)) => return Ok((key, index)),
        Some(IndexState::Building) => {
            return Err(ToolError::new(
                "the search index is still being built; try again in a minute",
            ));
        }
        Some(IndexState::Failed(message)) => {
            state.forget_search_index(&key);
            return Err(ToolError::new(format!(
                "the search index could not be built: {message}"
            )));
        }
        None => {}
    }
    let path = index_path(app, &key)?;
    if let Some(index) = SourceIndex::open(&path, &key).ok().flatten() {
        state.set_search_index(&key, IndexState::Ready(index.clone()));
        return Ok((key, index));
    }
    if state.claim_search_build(&key) {
        let input = match build_input(app) {
            Ok(input) if input.key == key => input,
            Ok(_) | Err(_) => {
                state.forget_search_index(&key);
                return Err(ToolError::new("the project changed; try again"));
            }
        };
        let task_app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = build(&task_app, &input);
            task_app.state::<DesktopState>().set_search_index(
                &input.key,
                match result {
                    Ok(index) => IndexState::Ready(index),
                    Err(error) => IndexState::Failed(error.to_string()),
                },
            );
        });
    }
    Err(ToolError::new(
        "the search index is being built for this source; try again in a minute",
    ))
}

/// Starts building the active project's index if it is missing.
pub(crate) fn prepare_source_index(app: &tauri::AppHandle) {
    let _ = source_index(app);
}

fn binding_of(hit: &SourceHit) -> SourceBinding {
    SourceBinding::new(hit.sheet.clone(), hit.row, hit.subrow, hit.column)
}

fn location_of(binding: &SourceBinding) -> UnitLocation {
    UnitLocation {
        sheet: binding.sheet_name().to_owned(),
        row: binding.row_id(),
        subrow: binding.subrow_id(),
        column: Some(binding.column_index()),
    }
}

/// Search over the active project for Angelica and job workers.
pub(crate) struct DesktopSearch {
    pub(crate) app: tauri::AppHandle,
}

impl DesktopSearch {
    /// Runs `read` on the session of the index's game source.
    fn with_session<T>(
        &self,
        key: Option<&str>,
        read: impl FnOnce(&ProjectSession) -> Result<T, ToolError>,
    ) -> Result<T, ToolError> {
        let state = self.app.state::<DesktopState>();
        let project = state
            .lock_project()
            .map_err(|error| ToolError::new(error.message))?;
        let session = project
            .as_ref()
            .ok_or_else(|| ToolError::new("no project is open"))?;
        if key.is_some_and(|id| id != source_key(session.source())) {
            return Err(ToolError::new("the project changed during the search"));
        }
        read(session)
    }
}

impl ProjectSearch for DesktopSearch {
    fn search_source(&self, query: &SearchQuery) -> Result<SearchMatches, ToolError> {
        let (key, index) = source_index(&self.app)?;
        let page = index
            .search(&SourceQuery {
                text: &query.text,
                sheet: query.sheet.as_deref(),
                offset: query.offset,
                limit: query.limit,
            })
            .map_err(|error| ToolError::new(error.to_string()))?;
        self.with_session(Some(&key), |session| {
            Ok(SearchMatches {
                matches: page
                    .hits
                    .into_iter()
                    .map(|hit| {
                        let binding = binding_of(&hit);
                        let unit = session.workspace().unit_by_source_binding(&binding);
                        SearchMatch {
                            location: location_of(&binding),
                            source: hit.source,
                            target: unit.map(|unit| unit.target_macro().to_owned()),
                            review_state: unit.map(|unit| review_label(unit.review_state())),
                        }
                    })
                    .collect(),
                more: page.more,
            })
        })
    }

    fn search_translations(&self, query: &SearchQuery) -> Result<SearchMatches, ToolError> {
        self.with_session(None, |session| Ok(translation_matches(session, query)))
    }

    fn similar_translations(
        &self,
        source: &str,
        exclude: Option<&UnitLocation>,
        limit: usize,
    ) -> Result<Vec<MemoryMatch>, ToolError> {
        let (key, index) = source_index(&self.app)?;
        let exclude = exclude.map(|location| {
            (
                location.sheet.as_str(),
                location.row,
                location.subrow,
                location.column.unwrap_or(0),
            )
        });
        let candidates = index
            .similar(source, exclude, MEMORY_CANDIDATES)
            .map_err(|error| ToolError::new(error.to_string()))?;
        self.with_session(Some(&key), |session| {
            Ok(memory_matches(session, candidates, limit))
        })
    }
}

/// Bound translations whose plain text contains the query, in binding order.
fn translation_matches(session: &ProjectSession, query: &SearchQuery) -> SearchMatches {
    let mut found: Vec<_> = session
        .workspace()
        .units()
        .filter(|unit| unit.is_bound())
        .filter(|unit| {
            query
                .sheet
                .as_deref()
                .is_none_or(|sheet| unit.source_binding().sheet_name() == sheet)
        })
        .filter(|unit| aeria_search::text_contains(unit.target_macro(), &query.text))
        .collect();
    found.sort_by(|left, right| left.source_binding().cmp(right.source_binding()));
    let start = usize::try_from(query.offset).unwrap_or(usize::MAX);
    let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);
    let more = found.len() > start.saturating_add(limit);
    SearchMatches {
        matches: found
            .into_iter()
            .skip(start)
            .take(limit)
            .map(|unit| SearchMatch {
                location: location_of(unit.source_binding()),
                source: session
                    .source_macro(unit.source_binding())
                    .unwrap_or_default(),
                target: Some(unit.target_macro().to_owned()),
                review_state: Some(review_label(unit.review_state())),
            })
            .collect(),
        more,
    }
}

/// The similar source strings that have a non-empty translation.
fn memory_matches(
    session: &ProjectSession,
    candidates: Vec<SimilarSource>,
    limit: usize,
) -> Vec<MemoryMatch> {
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let binding = binding_of(&candidate.hit);
            let unit = session.workspace().unit_by_source_binding(&binding)?;
            (!unit.target_macro().trim().is_empty()).then(|| MemoryMatch {
                location: location_of(&binding),
                source: candidate.hit.source,
                target: unit.target_macro().to_owned(),
                review_state: review_label(unit.review_state()),
                similarity: (candidate.score * 100.0).round() / 100.0,
            })
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {

    use aeria_ai::tools::ReviewLabel;

    use super::*;

    fn session() -> (
        (tempfile::TempDir, crate::test_support::TestGame),
        ProjectSession,
    ) {
        let directory = tempfile::tempdir().expect("directory");
        let game = crate::test_support::test_game();
        let session = crate::test_support::test_session(directory.path(), &game);
        ((directory, game), session)
    }

    #[test]
    fn translations_and_memory_come_from_bound_units() {
        let ((directory, _game), mut session) = session();
        let source = session.source_handle();
        let index = SourceIndex::build(
            directory.path().join("index.sqlite3"),
            "en/test",
            Tokenizer::Words,
            &source,
            &|| true,
        )
        .expect("index");
        let hit = SourceHit {
            sheet: "Synthetic".to_owned(),
            row: 42,
            subrow: 0,
            column: 0,
            source: "Hello there".to_owned(),
        };
        let binding = binding_of(&hit);
        let query = |text: &str| SearchQuery {
            text: text.to_owned(),
            sheet: None,
            offset: 0,
            limit: 10,
        };

        assert!(
            memory_matches(
                &session,
                index.similar(&hit.source, None, 10).expect("similar"),
                5
            )
            .is_empty()
        );
        assert!(
            translation_matches(&session, &query("эфирный"))
                .matches
                .is_empty()
        );

        let target = format!("{} эфирный", hit.source);
        session.set_target(&binding, &target).expect("target");
        let found = translation_matches(&session, &query("ЭФИРНЫЙ"));
        assert_eq!(found.matches.len(), 1);
        assert_eq!(found.matches[0].source, hit.source);
        assert_eq!(found.matches[0].review_state, Some(ReviewLabel::Draft));
        assert!(!found.more);

        let memory = memory_matches(
            &session,
            index.similar(&hit.source, None, 10).expect("similar"),
            5,
        );
        assert_eq!(memory[0].target, target);
        assert!((memory[0].similarity - 1.0).abs() < f64::EPSILON);
    }
}
