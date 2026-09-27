//! Search and translation-memory tools.
//!
//! The desktop implements [`ProjectSearch`] over the local source index
//! (`aeria-search`) and the workspace. Results are data for the model; they
//! never change the project.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::chat::ToolDefinition;
use crate::guidance::{ProjectFile, ProjectGuide};
use crate::tools::{
    ProjectReader, ReviewLabel, ToolError, UnitLocation, bound_text, parse, to_value,
};

/// Most matches one search returns to the model.
pub const MAX_MATCHES: u32 = 50;
/// Most translation-memory matches returned.
pub const MAX_MEMORY_MATCHES: usize = 10;
/// Most glossary candidates one call returns.
pub const MAX_GLOSSARY_CANDIDATES: usize = 100;
/// Other strings a glossary candidate must occur in by default.
const DEFAULT_CANDIDATE_STRINGS: usize = 3;

/// A name from the game's data sheets that recurs in other strings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlossaryCandidate {
    pub term: String,
    /// A few cells whose whole text is the term.
    pub locations: Vec<UnitLocation>,
    /// Cells whose whole text is the term.
    pub names: usize,
    /// Other strings that contain the term.
    pub strings: usize,
    /// Occurrences in those strings.
    pub occurrences: usize,
}

/// One string found by a search, with its translation state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub location: UnitLocation,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_state: Option<ReviewLabel>,
}

/// A page of search matches.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatches {
    pub matches: Vec<SearchMatch>,
    /// Whether more matches follow at `offset + matches.len()`.
    pub more: bool,
}

/// A translated string whose source is similar to another string's.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMatch {
    pub location: UnitLocation,
    pub source: String,
    pub target: String,
    pub review_state: ReviewLabel,
    /// Similarity of the plain source texts, from 0.5 to 1.
    pub similarity: f64,
}

/// A text search.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchQuery {
    pub text: String,
    pub sheet: Option<String>,
    pub offset: u32,
    pub limit: u32,
}

/// Search access to the open project, implemented by the desktop.
pub trait ProjectSearch: Send + Sync {
    /// Translatable strings whose source text matches the query.
    ///
    /// # Errors
    /// Returns an error when the index is not ready or cannot be read.
    fn search_source(&self, query: &SearchQuery) -> Result<SearchMatches, ToolError>;

    /// Translations whose text contains the query, ignoring case.
    ///
    /// # Errors
    /// Returns an error when no project is open.
    fn search_translations(&self, query: &SearchQuery) -> Result<SearchMatches, ToolError>;

    /// Translated strings whose source is similar to `source`, most similar
    /// first, without the string at `exclude`.
    ///
    /// # Errors
    /// Returns an error when the index is not ready or cannot be read.
    fn similar_translations(
        &self,
        source: &str,
        exclude: Option<&UnitLocation>,
        limit: usize,
    ) -> Result<Vec<MemoryMatch>, ToolError>;

    /// Terminology candidates of the project's source (see
    /// `aeria_search::terms`), those in the most strings first.
    ///
    /// # Errors
    /// Returns an error when the index is not ready or cannot be read.
    fn glossary_candidates(&self) -> Result<Arc<[GlossaryCandidate]>, ToolError>;
}

/// Definitions of the search tools, offered in every mode.
#[must_use]
pub fn search_tool_definitions() -> Vec<ToolDefinition> {
    let query = |description: &str| {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": description },
                "sheet": { "type": "string", "description": "Only this sheet." },
                "offset": { "type": "integer", "minimum": 0 },
                "limit": { "type": "integer", "minimum": 1, "maximum": MAX_MATCHES },
            },
            "required": ["query"],
            "additionalProperties": false,
        })
    };
    vec![
        ToolDefinition {
            name: "search_source",
            description: "Finds translatable strings by their source text across the whole project, best matches first, with their current translations. Macros are ignored; each word matches as a prefix, and short queries match anywhere in the text.",
            parameters: query("Words to find in the source text."),
        },
        ToolDefinition {
            name: "search_translations",
            description: "Finds existing translations containing a text, ignoring case and macros, with their sources. Use it to check how a term was translated before.",
            parameters: query("Text to find in translations."),
        },
        ToolDefinition {
            name: "similar_translations",
            description: "Translation memory: already translated strings whose source is similar to one string's source (or to a given text), most similar first, with a similarity from 0.5 to 1. Use them for consistent wording.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "sheet": { "type": "string" },
                    "row": { "type": "integer", "minimum": 0 },
                    "subrow": { "type": "integer", "minimum": 0 },
                    "column": { "type": "integer", "minimum": 0 },
                    "text": { "type": "string", "description": "A source text instead of a string's location." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_MEMORY_MATCHES },
                },
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "glossary_candidates",
            description: "Terminology candidates for the glossary: names from the game's data sheets (places, characters, items, actions, statuses, interface terms, and so on) that recur in other strings, those used in the most strings first, leaving out terms the glossary already has. Each lists where it is a name, with the project's current translation there. Which candidates are terminology is for you to judge: skip ordinary words and generic labels.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Only terms containing this text, ignoring case." },
                    "sheet": { "type": "string", "description": "Only names from this data sheet, such as PlaceName or ENpcResident." },
                    "min_strings": { "type": "integer", "minimum": 1, "description": "Least number of other strings a term occurs in (default 3)." },
                    "offset": { "type": "integer", "minimum": 0, "description": "The nextOffset of the previous page. The ranking does not change with the glossary, so continue from it after a glossary change." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_GLOSSARY_CANDIDATES },
                },
                "additionalProperties": false,
            }),
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryArgs {
    query: String,
    sheet: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SimilarArgs {
    sheet: Option<String>,
    row: Option<u32>,
    subrow: Option<u16>,
    column: Option<u32>,
    text: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateArgs {
    query: Option<String>,
    sheet: Option<String>,
    min_strings: Option<usize>,
    offset: Option<usize>,
    limit: Option<usize>,
}

impl QueryArgs {
    fn query(self) -> Result<SearchQuery, ToolError> {
        let text = self.query.trim().to_owned();
        if text.is_empty() {
            return Err(ToolError::new("the query is empty"));
        }
        Ok(SearchQuery {
            text,
            sheet: self.sheet.filter(|sheet| !sheet.trim().is_empty()),
            offset: self.offset.unwrap_or(0),
            limit: self.limit.unwrap_or(20).clamp(1, MAX_MATCHES),
        })
    }
}

fn bounded(mut matches: SearchMatches) -> SearchMatches {
    for found in &mut matches.matches {
        bound_text(&mut found.source);
        if let Some(target) = &mut found.target {
            bound_text(target);
        }
    }
    matches
}

/// The source text to find memory for: the given text, or the source of
/// the string at the given location.
fn memory_source(
    reader: &dyn ProjectReader,
    args: &SimilarArgs,
) -> Result<(String, Option<UnitLocation>), ToolError> {
    if let Some(text) = args
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Ok((text.to_owned(), None));
    }
    let (Some(sheet), Some(row), Some(column)) = (&args.sheet, args.row, args.column) else {
        return Err(ToolError::new(
            "give either text or a string's sheet, row, and column",
        ));
    };
    let subrow = args.subrow.unwrap_or(0);
    let source = reader
        .row(sheet, row, subrow)?
        .and_then(|row| row.cells.into_iter().find(|cell| cell.column == column))
        .map(|cell| cell.source)
        .ok_or_else(|| {
            ToolError::new(format!(
                "{sheet}:{row}:{subrow}:{column} is not a translatable string"
            ))
        })?;
    Ok((
        source,
        Some(UnitLocation {
            sheet: sheet.clone(),
            row,
            subrow,
            column: Some(column),
        }),
    ))
}

/// Runs one search tool.
///
/// # Errors
/// Returns invalid arguments, an unknown string, or a search failure.
pub fn run_search_tool(
    search: &dyn ProjectSearch,
    reader: &dyn ProjectReader,
    name: &str,
    arguments: &str,
) -> Result<Value, ToolError> {
    match name {
        "search_source" => {
            let query = parse::<QueryArgs>(arguments)?.query()?;
            to_value(&bounded(search.search_source(&query)?))
        }
        "search_translations" => {
            let query = parse::<QueryArgs>(arguments)?.query()?;
            to_value(&bounded(search.search_translations(&query)?))
        }
        "similar_translations" => {
            let args: SimilarArgs = parse(arguments)?;
            let (source, exclude) = memory_source(reader, &args)?;
            let limit = args.limit.unwrap_or(5).clamp(1, MAX_MEMORY_MATCHES);
            let mut matches = search.similar_translations(&source, exclude.as_ref(), limit)?;
            for found in &mut matches {
                bound_text(&mut found.source);
                bound_text(&mut found.target);
            }
            Ok(json!({ "matches": to_value(&matches)? }))
        }
        "glossary_candidates" => glossary_candidates(search, reader, &parse(arguments)?),
        _ => Err(ToolError::new(format!("unknown tool {name:?}"))),
    }
}

/// A page of glossary candidates that the glossary does not have yet, each
/// with the project's translations of the cells where it is a name.
fn glossary_candidates(
    search: &dyn ProjectSearch,
    reader: &dyn ProjectReader,
    args: &CandidateArgs,
) -> Result<Value, ToolError> {
    let guide = ProjectGuide::from_files(
        Ok(None),
        reader
            .project_file(ProjectFile::Glossary)
            .map_err(|error| error.0),
    );
    let known: std::collections::HashSet<String> = guide
        .glossary
        .iter()
        .flat_map(|glossary| &glossary.entries)
        .map(|entry| entry.term.to_lowercase())
        .collect();
    let query = args
        .query
        .as_deref()
        .map(|query| query.trim().to_lowercase())
        .filter(|query| !query.is_empty());
    let sheet = args
        .sheet
        .as_deref()
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty());
    let min_strings = args.min_strings.unwrap_or(DEFAULT_CANDIDATE_STRINGS).max(1);
    let offset = args.offset.unwrap_or(0);
    let limit = args.limit.unwrap_or(50).clamp(1, MAX_GLOSSARY_CANDIDATES);

    let all = search.glossary_candidates()?;
    // The ranking does not depend on the glossary, so a cursor into it
    // stays valid after glossary changes and skipped terms stay behind it.
    let ranked: Vec<&GlossaryCandidate> = all
        .iter()
        .filter(|candidate| candidate.strings >= min_strings)
        .filter(|candidate| {
            query
                .as_ref()
                .is_none_or(|query| candidate.term.to_lowercase().contains(query))
        })
        .filter(|candidate| {
            sheet.is_none_or(|sheet| {
                candidate
                    .locations
                    .iter()
                    .any(|location| location.sheet == sheet)
            })
        })
        .collect();
    let is_new = |candidate: &&&GlossaryCandidate| !known.contains(&candidate.term.to_lowercase());
    let mut candidates = Vec::new();
    let mut next = ranked.len();
    for (position, candidate) in ranked.iter().enumerate().skip(offset) {
        if !is_new(&candidate) {
            continue;
        }
        if candidates.len() == limit {
            next = position;
            break;
        }
        let mut sources = Vec::new();
        for location in &candidate.locations {
            let cell = reader
                .row(&location.sheet, location.row, location.subrow)?
                .and_then(|row| {
                    row.cells
                        .into_iter()
                        .find(|cell| Some(cell.column) == location.column)
                });
            let mut source = json!({ "location": location });
            if let Some(cell) = cell {
                if let Some(mut target) = cell.target.filter(|target| !target.trim().is_empty()) {
                    bound_text(&mut target);
                    source["target"] = json!(target);
                }
                if let Some(review_state) = cell.review_state {
                    source["reviewState"] = to_value(&review_state)?;
                }
            }
            sources.push(source);
        }
        candidates.push(json!({
            "term": candidate.term,
            "strings": candidate.strings,
            "occurrences": candidate.occurrences,
            "names": candidate.names,
            "sources": sources,
        }));
    }
    let mut result = json!({
        "notInGlossary": ranked.iter().filter(is_new).count(),
        "candidates": candidates,
    });
    if next < ranked.len() {
        result["nextOffset"] = json!(next);
        result["remainingAfterThisPage"] = json!(ranked[next..].iter().filter(is_new).count());
    }
    if !guide.problems.is_empty() {
        result["glossaryProblems"] = json!(guide.problems);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::guidance::ProjectFile;
    use crate::tools::{
        CellSnapshot, ProjectFacts, ReadTools, RowSnapshot, RowsPage, SheetSummary,
    };

    struct Reader;

    impl ProjectReader for Reader {
        fn facts(&self) -> Result<ProjectFacts, ToolError> {
            Err(ToolError::new("unused"))
        }

        fn other_languages(
            &self,
            _: &str,
            _: u32,
            _: u16,
            _: u32,
        ) -> Result<Vec<(String, Option<String>)>, ToolError> {
            Err(ToolError::new("unused"))
        }
        fn sheets(&self) -> Result<Vec<SheetSummary>, ToolError> {
            Ok(Vec::new())
        }
        fn rows(&self, _: &str, _: Option<(u32, u16)>, _: u32) -> Result<RowsPage, ToolError> {
            Err(ToolError::new("unused"))
        }
        fn row(
            &self,
            sheet: &str,
            row: u32,
            subrow: u16,
        ) -> Result<Option<RowSnapshot>, ToolError> {
            Ok((sheet == "Item" && row == 5).then(|| RowSnapshot {
                row,
                subrow,
                cells: vec![CellSnapshot {
                    column: 0,
                    source: "Fire Crystal".to_owned(),
                    formatting_only: false,
                    target: None,
                    review_state: None,
                    note: None,
                    unit_id: None,
                    tagged: None,
                    tags: Vec::new(),
                    untaggable: false,
                    glossary: Vec::new(),
                }],
                context: Vec::new(),
            }))
        }
        fn pending_changes(&self) -> Result<Value, ToolError> {
            Ok(Value::Null)
        }
        fn unit_history(&self, _: &str, _: u32) -> Result<Value, ToolError> {
            Ok(Value::Null)
        }
        fn navigate(&self, _: &UnitLocation) -> Result<(), ToolError> {
            Ok(())
        }
        fn project_file(&self, file: ProjectFile) -> Result<Option<String>, ToolError> {
            Ok((file == ProjectFile::Glossary)
                .then(|| "term,translation\nice crystal,Ледяной кристалл\n".to_owned()))
        }

        fn dialogue(&self, _: &str) -> Result<Option<crate::dialogue::SheetDialogue>, ToolError> {
            Ok(None)
        }

        fn speakers(&self, _: &str) -> Result<Vec<(String, usize)>, ToolError> {
            Ok(Vec::new())
        }

        fn speaker_lines(
            &self,
            _: &str,
            _: usize,
            _: usize,
        ) -> Result<(usize, Vec<UnitLocation>), ToolError> {
            Ok((0, Vec::new()))
        }
    }

    #[derive(Default)]
    struct Search {
        calls: Mutex<Vec<String>>,
    }

    impl ProjectSearch for Search {
        fn search_source(&self, query: &SearchQuery) -> Result<SearchMatches, ToolError> {
            self.calls.lock().expect("lock").push(format!(
                "source {} {:?} {} {}",
                query.text, query.sheet, query.offset, query.limit
            ));
            Ok(SearchMatches {
                matches: vec![SearchMatch {
                    location: UnitLocation {
                        sheet: "Item".to_owned(),
                        row: 5,
                        subrow: 0,
                        column: Some(0),
                    },
                    source: "x".repeat(3000),
                    target: None,
                    review_state: None,
                }],
                more: true,
            })
        }
        fn search_translations(&self, query: &SearchQuery) -> Result<SearchMatches, ToolError> {
            self.calls
                .lock()
                .expect("lock")
                .push(format!("translations {}", query.text));
            Ok(SearchMatches::default())
        }
        fn similar_translations(
            &self,
            source: &str,
            exclude: Option<&UnitLocation>,
            limit: usize,
        ) -> Result<Vec<MemoryMatch>, ToolError> {
            self.calls.lock().expect("lock").push(format!(
                "similar {source} {:?} {limit}",
                exclude.map(|location| (location.sheet.as_str(), location.row, location.column))
            ));
            Ok(Vec::new())
        }
        fn glossary_candidates(&self) -> Result<Arc<[GlossaryCandidate]>, ToolError> {
            let candidate = |term: &str, sheet: &str, row: u32, strings: usize| GlossaryCandidate {
                term: term.to_owned(),
                locations: vec![UnitLocation {
                    sheet: sheet.to_owned(),
                    row,
                    subrow: 0,
                    column: Some(0),
                }],
                names: 1,
                strings,
                occurrences: strings + 1,
            };
            Ok(Arc::from(vec![
                candidate("Fire Crystal", "Item", 5, 10),
                candidate("Ice Crystal", "Item", 6, 8),
                candidate("Limsa Lominsa", "PlaceName", 1, 4),
                candidate("Gridania", "PlaceName", 2, 2),
            ]))
        }
    }

    #[test]
    fn glossary_candidates_skip_known_terms_and_page() {
        let search = Search::default();
        let tools = ReadTools::new(&Reader).with_search(&search);
        let page = tools.execute("glossary_candidates", r#"{"limit":1}"#);
        assert!(!page.is_error, "{}", page.content);
        let value: Value = serde_json::from_str(&page.content).expect("json");
        assert_eq!(
            value["notInGlossary"], 2,
            "known and rare terms are left out"
        );
        assert_eq!(value["candidates"][0]["term"], "Fire Crystal");
        assert_eq!(value["candidates"][0]["strings"], 10);
        assert_eq!(
            value["candidates"][0]["sources"][0]["location"]["sheet"],
            "Item"
        );
        assert_eq!(
            value["nextOffset"], 2,
            "the cursor counts the known term it passed"
        );
        assert_eq!(value["remainingAfterThisPage"], 1);
        let rest = tools.execute("glossary_candidates", r#"{"offset":2}"#);
        let value: Value = serde_json::from_str(&rest.content).expect("json");
        assert_eq!(value["candidates"][0]["term"], "Limsa Lominsa");
        assert!(value.get("nextOffset").is_none());

        let places = tools.execute(
            "glossary_candidates",
            r#"{"sheet":"PlaceName","min_strings":1}"#,
        );
        let value: Value = serde_json::from_str(&places.content).expect("json");
        assert_eq!(value["notInGlossary"], 2);
        assert!(value.get("nextOffset").is_none());
        let found = tools.execute("glossary_candidates", r#"{"query":"LOMINSA"}"#);
        let value: Value = serde_json::from_str(&found.content).expect("json");
        assert_eq!(value["candidates"][0]["term"], "Limsa Lominsa");
    }

    #[test]
    fn search_tools_check_arguments_and_bound_results() {
        let search = Search::default();
        let tools = ReadTools::new(&Reader).with_search(&search);
        let found = tools.execute("search_source", r#"{"query":" crystal ","limit":500}"#);
        assert!(!found.is_error, "{}", found.content);
        assert!(found.content.contains("\"more\":true"));
        assert!(found.content.contains("…[truncated]"));
        assert!(tools.execute("search_source", r#"{"query":"  "}"#).is_error);
        assert!(
            !tools
                .execute("search_translations", r#"{"query":"Кристалл","sheet":""}"#)
                .is_error
        );

        let by_location = tools.execute(
            "similar_translations",
            r#"{"sheet":"Item","row":5,"column":0,"limit":99}"#,
        );
        assert!(!by_location.is_error, "{}", by_location.content);
        assert!(
            tools
                .execute(
                    "similar_translations",
                    r#"{"sheet":"Item","row":6,"column":0}"#
                )
                .is_error
        );
        assert!(tools.execute("similar_translations", "{}").is_error);
        assert!(
            !tools
                .execute("similar_translations", r#"{"text":"Ice Crystal"}"#)
                .is_error
        );
        assert_eq!(
            search.calls.lock().expect("lock").as_slice(),
            [
                "source crystal None 0 50",
                "translations Кристалл",
                "similar Fire Crystal Some((\"Item\", 5, Some(0))) 10",
                "similar Ice Crystal None 5",
            ]
        );
        assert!(
            ReadTools::new(&Reader)
                .execute("search_source", r#"{"query":"x"}"#)
                .is_error
        );
    }
}
