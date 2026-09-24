//! Angelica's Aeria-specific tools.
//!
//! Tools are typed and bounded. `aeria-ai` owns their schemas, argument
//! validation, and result shaping; the desktop implements [`ProjectReader`]
//! over the active project session. Invalid calls return an error result to
//! the model instead of failing the conversation.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::chat::ToolDefinition;

/// Longest source, target, or note text returned for one cell.
pub const MAX_CELL_TEXT_CHARS: usize = 2000;
/// Longest serialized tool result.
pub const MAX_RESULT_CHARS: usize = 24_000;
/// Most source row groups one `read_rows` call scans.
pub const MAX_READ_ROWS: u32 = 50;

/// A failure reported back to the model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolError(pub String);

impl ToolError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Facts about the open project.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFacts {
    pub source_language: String,
    /// `None` while the project has no real target language yet.
    pub target_language: Option<String>,
    pub game_version: String,
    pub sheet_count: usize,
    pub translatable_strings: u64,
    pub translated: u64,
    pub reviewed: u64,
    pub needs_review: u64,
    pub detached_units: usize,
}

/// Coverage of one sheet.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetSummary {
    pub name: String,
    pub translatable: u64,
    pub translated: u64,
    pub reviewed: u64,
    pub needs_review: u64,
}

/// A review state as the model sees it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewLabel {
    Draft,
    NeedsReview,
    Reviewed,
}

/// One translatable string occurrence and its translation, if any.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellSnapshot {
    pub column: u32,
    pub source: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub formatting_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_state: Option<ReviewLabel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_id: Option<String>,
}

/// One source row with its translatable cells and read-only context.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowSnapshot {
    pub row: u32,
    pub subrow: u16,
    pub cells: Vec<CellSnapshot>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<ContextCell>,
}

/// A read-only source cell shown for context.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCell {
    pub column: u32,
    pub source: String,
}

/// One bounded page of rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowsPage {
    pub rows: Vec<RowSnapshot>,
    /// Cursor for the next page, when more source rows exist.
    pub next_after: Option<(u32, u16)>,
}

/// A string occurrence address.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitLocation {
    pub sheet: String,
    pub row: u32,
    pub subrow: u16,
    pub column: Option<u32>,
}

/// Read access to the open project, implemented by the desktop.
pub trait ProjectReader: Send + Sync {
    /// # Errors
    /// Returns an error when no project is open or it cannot be read.
    fn facts(&self) -> Result<ProjectFacts, ToolError>;

    /// # Errors
    /// Returns an error when no project is open or it cannot be read.
    fn sheets(&self) -> Result<Vec<SheetSummary>, ToolError>;

    /// Reads up to `limit` source row groups after the exclusive cursor.
    ///
    /// # Errors
    /// Returns an error for an unknown sheet or a read failure.
    fn rows(
        &self,
        sheet: &str,
        after: Option<(u32, u16)>,
        limit: u32,
    ) -> Result<RowsPage, ToolError>;

    /// Reads one source row, or `None` when it has no translatable string.
    ///
    /// # Errors
    /// Returns an error for an unknown sheet or a read failure.
    fn row(&self, sheet: &str, row: u32, subrow: u16) -> Result<Option<RowSnapshot>, ToolError>;

    /// Uncommitted translation changes as JSON.
    ///
    /// # Errors
    /// Returns an error when Git is unavailable or fails.
    fn pending_changes(&self) -> Result<Value, ToolError>;

    /// Committed history of one unit's translation as JSON.
    ///
    /// # Errors
    /// Returns an error for an unknown unit or a Git failure.
    fn unit_history(&self, unit_id: &str, limit: u32) -> Result<Value, ToolError>;

    /// Opens an occurrence in the editor. Presentation only.
    ///
    /// # Errors
    /// Returns an error when the editor cannot be reached.
    fn navigate(&self, location: &UnitLocation) -> Result<(), ToolError>;
}

/// Definitions of the read-only tools offered in Chat mode.
#[must_use]
pub fn read_tool_definitions() -> Vec<ToolDefinition> {
    let location = |description: &str| {
        json!({
            "type": "object",
            "properties": {
                "sheet": { "type": "string", "description": "Exact sheet name, as listed by list_sheets." },
                "row": { "type": "integer", "minimum": 0 },
                "subrow": { "type": "integer", "minimum": 0, "description": "Defaults to 0." },
                "column": { "type": "integer", "minimum": 0, "description": description },
            },
            "required": ["sheet", "row"],
            "additionalProperties": false,
        })
    };
    vec![
        ToolDefinition {
            name: "project_overview",
            description: "Languages, game version, sheet count, and overall translation progress of the open project.",
            parameters: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDefinition {
            name: "list_sheets",
            description: "Sheets that contain translatable strings, with per-sheet progress. Filter by a name substring; results are paged.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Case-insensitive substring of the sheet name." },
                    "untranslated_only": { "type": "boolean", "description": "Only sheets with untranslated strings." },
                    "offset": { "type": "integer", "minimum": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Defaults to 50." },
                },
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "read_rows",
            description: "Reads source rows of one sheet in order with their translations. `limit` counts scanned source rows, so fewer rows can come back; continue from `nextAfter`.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "sheet": { "type": "string" },
                    "after_row": { "type": "integer", "minimum": 0, "description": "Exclusive cursor row from nextAfter." },
                    "after_subrow": { "type": "integer", "minimum": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_READ_ROWS, "description": "Defaults to 20." },
                    "state": {
                        "type": "string",
                        "enum": ["all", "untranslated", "translated", "draft", "needsReview", "reviewed"],
                        "description": "Keep only cells in this state. Defaults to all.",
                    },
                },
                "required": ["sheet"],
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "get_unit",
            description: "One source row with every translatable string, its translation, review state, note, and read-only context cells.",
            parameters: location("Only this column; omit for all cells of the row."),
        },
        ToolDefinition {
            name: "pending_changes",
            description: "Uncommitted translation changes in the project's Git working tree.",
            parameters: json!({
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Defaults to 50." } },
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "unit_history",
            description: "Committed history of one translation unit, by the unitId from get_unit or read_rows.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "unit_id": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "description": "Defaults to 10." },
                },
                "required": ["unit_id"],
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "navigate_to",
            description: "Opens a string in the user's editor so they can see it. Changes nothing in the project.",
            parameters: location("The string's column; omit for the row's first string."),
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListSheetsArgs {
    query: Option<String>,
    #[serde(default)]
    untranslated_only: bool,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
enum StateFilter {
    All,
    Untranslated,
    Translated,
    Draft,
    NeedsReview,
    Reviewed,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRowsArgs {
    sheet: String,
    after_row: Option<u32>,
    after_subrow: Option<u16>,
    limit: Option<u32>,
    state: Option<StateFilter>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocationArgs {
    sheet: String,
    row: u32,
    subrow: Option<u16>,
    column: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingArgs {
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryArgs {
    unit_id: String,
    limit: Option<u32>,
}

/// The outcome of one tool call as it is sent back to the model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

/// Executes the read-only tools against a [`ProjectReader`].
pub struct ReadTools<'a> {
    reader: &'a dyn ProjectReader,
}

impl<'a> ReadTools<'a> {
    #[must_use]
    pub fn new(reader: &'a dyn ProjectReader) -> Self {
        Self { reader }
    }

    /// Runs one tool call. Unknown tools, invalid arguments, and reader
    /// failures become error outputs for the model.
    #[must_use]
    pub fn execute(&self, name: &str, arguments: &str) -> ToolOutput {
        match self.dispatch(name, arguments) {
            Ok(value) => ToolOutput {
                content: bounded_json(&value),
                is_error: false,
            },
            Err(error) => ToolOutput {
                content: json!({ "error": error.0 }).to_string(),
                is_error: true,
            },
        }
    }

    fn dispatch(&self, name: &str, arguments: &str) -> Result<Value, ToolError> {
        match name {
            "project_overview" => {
                parse::<Empty>(arguments)?;
                to_value(&self.reader.facts()?)
            }
            "list_sheets" => self.list_sheets(parse(arguments)?),
            "read_rows" => self.read_rows(&parse(arguments)?),
            "get_unit" => self.get_unit(&parse(arguments)?),
            "pending_changes" => {
                let args: PendingArgs = parse(arguments)?;
                let limit = args.limit.unwrap_or(50).clamp(1, 200);
                let changes = self.reader.pending_changes()?;
                Ok(match changes {
                    Value::Array(items) => {
                        let total = items.len();
                        json!({ "total": total, "changes": items.into_iter().take(limit).collect::<Vec<_>>() })
                    }
                    other => other,
                })
            }
            "unit_history" => {
                let args: HistoryArgs = parse(arguments)?;
                self.reader
                    .unit_history(args.unit_id.trim(), args.limit.unwrap_or(10).clamp(1, 50))
            }
            "navigate_to" => {
                let args: LocationArgs = parse(arguments)?;
                let location = UnitLocation {
                    sheet: args.sheet,
                    row: args.row,
                    subrow: args.subrow.unwrap_or(0),
                    column: args.column,
                };
                self.reader.navigate(&location)?;
                Ok(json!({ "opened": location }))
            }
            other => Err(ToolError::new(format!("unknown tool {other:?}"))),
        }
    }

    fn list_sheets(&self, args: ListSheetsArgs) -> Result<Value, ToolError> {
        let query = args.query.map(|query| query.trim().to_lowercase());
        let matching: Vec<SheetSummary> = self
            .reader
            .sheets()?
            .into_iter()
            .filter(|sheet| sheet.translatable > 0)
            .filter(|sheet| {
                query
                    .as_deref()
                    .is_none_or(|query| sheet.name.to_lowercase().contains(query))
            })
            .filter(|sheet| !args.untranslated_only || sheet.translated < sheet.translatable)
            .collect();
        let limit = args.limit.unwrap_or(50).clamp(1, 200);
        let total = matching.len();
        let page: Vec<_> = matching.into_iter().skip(args.offset).take(limit).collect();
        let next_offset = (args.offset + page.len() < total).then_some(args.offset + page.len());
        Ok(json!({ "total": total, "sheets": page, "nextOffset": next_offset }))
    }

    fn read_rows(&self, args: &ReadRowsArgs) -> Result<Value, ToolError> {
        let after = match (args.after_row, args.after_subrow) {
            (Some(row), subrow) => Some((row, subrow.unwrap_or(0))),
            (None, None) => None,
            (None, Some(_)) => {
                return Err(ToolError::new("after_subrow requires after_row"));
            }
        };
        let limit = args.limit.unwrap_or(20).clamp(1, MAX_READ_ROWS);
        let filter = args.state.unwrap_or(StateFilter::All);
        let page = self.reader.rows(&args.sheet, after, limit)?;
        let rows: Vec<RowSnapshot> = page
            .rows
            .into_iter()
            .filter_map(|mut row| {
                row.cells.retain(|cell| matches_filter(cell, filter));
                (!row.cells.is_empty()).then(|| bound_row(row))
            })
            .collect();
        Ok(json!({
            "sheet": args.sheet,
            "rows": rows,
            "nextAfter": page.next_after.map(|(row, subrow)| json!({ "row": row, "subrow": subrow })),
        }))
    }

    fn get_unit(&self, args: &LocationArgs) -> Result<Value, ToolError> {
        let subrow = args.subrow.unwrap_or(0);
        let Some(mut row) = self.reader.row(&args.sheet, args.row, subrow)? else {
            return Err(ToolError::new(format!(
                "{}:{}:{} has no translatable string",
                args.sheet, args.row, subrow
            )));
        };
        if let Some(column) = args.column {
            row.cells.retain(|cell| cell.column == column);
            if row.cells.is_empty() {
                return Err(ToolError::new(format!(
                    "column {column} of {}:{}:{subrow} is not a translatable string",
                    args.sheet, args.row
                )));
            }
        }
        Ok(json!({ "sheet": args.sheet, "row": bound_row(row) }))
    }
}

fn matches_filter(cell: &CellSnapshot, filter: StateFilter) -> bool {
    match filter {
        StateFilter::All => true,
        StateFilter::Untranslated => cell.target.is_none(),
        StateFilter::Translated => cell.target.is_some(),
        StateFilter::Draft => cell.review_state == Some(ReviewLabel::Draft),
        StateFilter::NeedsReview => cell.review_state == Some(ReviewLabel::NeedsReview),
        StateFilter::Reviewed => cell.review_state == Some(ReviewLabel::Reviewed),
    }
}

fn bound_row(mut row: RowSnapshot) -> RowSnapshot {
    for cell in &mut row.cells {
        bound_text(&mut cell.source);
        if let Some(target) = &mut cell.target {
            bound_text(target);
        }
        if let Some(note) = &mut cell.note {
            bound_text(note);
        }
    }
    for cell in &mut row.context {
        bound_text(&mut cell.source);
    }
    row
}

fn bound_text(text: &mut String) {
    if text.chars().count() > MAX_CELL_TEXT_CHARS {
        *text = text.chars().take(MAX_CELL_TEXT_CHARS).collect::<String>() + "…[truncated]";
    }
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: &str) -> Result<T, ToolError> {
    let arguments = if arguments.trim().is_empty() {
        "{}"
    } else {
        arguments
    };
    serde_json::from_str(arguments)
        .map_err(|error| ToolError::new(format!("invalid arguments: {error}")))
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, ToolError> {
    serde_json::to_value(value).map_err(|error| ToolError::new(error.to_string()))
}

/// Serializes a result, cutting it at [`MAX_RESULT_CHARS`] with a notice so
/// one call cannot flood the context.
fn bounded_json(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() <= MAX_RESULT_CHARS {
        return text;
    }
    let cut: String = text.chars().take(MAX_RESULT_CHARS).collect();
    format!("{cut}\n[result truncated at {MAX_RESULT_CHARS} characters; request a smaller page]")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct FakeReader {
        navigated: Mutex<Vec<UnitLocation>>,
    }

    fn cell(column: u32, target: Option<&str>, state: Option<ReviewLabel>) -> CellSnapshot {
        CellSnapshot {
            column,
            source: format!("Source {column}"),
            formatting_only: false,
            target: target.map(str::to_owned),
            review_state: state,
            note: None,
            unit_id: target.map(|_| format!("unit-{column}")),
        }
    }

    impl ProjectReader for FakeReader {
        fn facts(&self) -> Result<ProjectFacts, ToolError> {
            Ok(ProjectFacts {
                source_language: "en".to_owned(),
                target_language: Some("ru".to_owned()),
                game_version: "2026.09.01".to_owned(),
                sheet_count: 3,
                translatable_strings: 30,
                translated: 5,
                reviewed: 1,
                needs_review: 0,
                detached_units: 0,
            })
        }

        fn sheets(&self) -> Result<Vec<SheetSummary>, ToolError> {
            Ok(["Addon", "Item", "Quest/Main"]
                .iter()
                .enumerate()
                .map(|(index, name)| SheetSummary {
                    name: (*name).to_owned(),
                    translatable: if index == 0 { 0 } else { 10 },
                    translated: if index == 1 { 10 } else { 0 },
                    reviewed: 0,
                    needs_review: 0,
                })
                .collect())
        }

        fn rows(
            &self,
            sheet: &str,
            after: Option<(u32, u16)>,
            limit: u32,
        ) -> Result<RowsPage, ToolError> {
            if sheet != "Item" {
                return Err(ToolError::new(format!("unknown sheet {sheet:?}")));
            }
            assert_eq!(after, Some((4, 0)));
            assert_eq!(limit, MAX_READ_ROWS);
            Ok(RowsPage {
                rows: vec![RowSnapshot {
                    row: 5,
                    subrow: 0,
                    cells: vec![
                        cell(0, Some("Меч"), Some(ReviewLabel::Draft)),
                        cell(1, None, None),
                    ],
                    context: Vec::new(),
                }],
                next_after: Some((5, 0)),
            })
        }

        fn row(
            &self,
            _sheet: &str,
            row: u32,
            subrow: u16,
        ) -> Result<Option<RowSnapshot>, ToolError> {
            Ok((row == 5).then(|| RowSnapshot {
                row,
                subrow,
                cells: vec![
                    cell(0, Some(&"я".repeat(3000)), Some(ReviewLabel::Reviewed)),
                    cell(1, None, None),
                ],
                context: vec![ContextCell {
                    column: 2,
                    source: "context".to_owned(),
                }],
            }))
        }

        fn pending_changes(&self) -> Result<Value, ToolError> {
            Ok(json!([{ "id": 1 }, { "id": 2 }, { "id": 3 }]))
        }

        fn unit_history(&self, unit_id: &str, limit: u32) -> Result<Value, ToolError> {
            Ok(json!({ "unitId": unit_id, "limit": limit }))
        }

        fn navigate(&self, location: &UnitLocation) -> Result<(), ToolError> {
            self.navigated.lock().expect("lock").push(location.clone());
            Ok(())
        }
    }

    fn reader() -> FakeReader {
        FakeReader {
            navigated: Mutex::new(Vec::new()),
        }
    }

    fn run(reader: &FakeReader, name: &str, arguments: &str) -> (Value, bool) {
        let output = ReadTools::new(reader).execute(name, arguments);
        (
            serde_json::from_str(&output.content).expect("json output"),
            output.is_error,
        )
    }

    #[test]
    fn every_definition_has_an_object_schema_and_a_handler() {
        let reader = reader();
        for tool in read_tool_definitions() {
            assert_eq!(tool.parameters["type"], "object", "{}", tool.name);
            let output = ReadTools::new(&reader).execute(tool.name, "{}");
            assert!(
                !output.content.contains("unknown tool"),
                "{} has no handler",
                tool.name
            );
        }
    }

    #[test]
    fn list_sheets_hides_empty_sheets_and_filters_by_name_and_progress() {
        let reader = reader();
        let (value, _) = run(&reader, "list_sheets", "{}");
        assert_eq!(value["total"], 2);
        let (value, _) = run(&reader, "list_sheets", r#"{"query":"quest"}"#);
        assert_eq!(value["sheets"][0]["name"], "Quest/Main");
        let (value, _) = run(&reader, "list_sheets", r#"{"untranslated_only":true}"#);
        assert_eq!(value["total"], 1);
        let (value, _) = run(&reader, "list_sheets", r#"{"limit":1}"#);
        assert_eq!(value["nextOffset"], 1);
    }

    #[test]
    fn read_rows_clamps_the_limit_and_filters_cells() {
        let reader = reader();
        let (value, error) = run(
            &reader,
            "read_rows",
            r#"{"sheet":"Item","after_row":4,"limit":999,"state":"untranslated"}"#,
        );
        assert!(!error);
        let cells = value["rows"][0]["cells"].as_array().expect("cells");
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0]["column"], 1);
        assert!(cells[0].get("target").is_none());
        assert_eq!(value["nextAfter"], json!({ "row": 5, "subrow": 0 }));
    }

    #[test]
    fn get_unit_bounds_long_text_and_selects_a_column() {
        let reader = reader();
        let (value, _) = run(
            &reader,
            "get_unit",
            r#"{"sheet":"Item","row":5,"column":0}"#,
        );
        let target = value["row"]["cells"][0]["target"].as_str().expect("target");
        assert!(target.ends_with("…[truncated]"));
        assert_eq!(
            target.chars().count(),
            MAX_CELL_TEXT_CHARS + "…[truncated]".chars().count()
        );
        assert_eq!(value["row"]["cells"].as_array().expect("cells").len(), 1);
        assert_eq!(value["row"]["cells"][0]["reviewState"], "reviewed");

        let (value, error) = run(&reader, "get_unit", r#"{"sheet":"Item","row":6}"#);
        assert!(error);
        assert!(
            value["error"]
                .as_str()
                .expect("error")
                .contains("no translatable string")
        );
    }

    #[test]
    fn invalid_arguments_and_unknown_tools_become_error_results() {
        let reader = reader();
        for (name, arguments) in [
            ("get_unit", r#"{"sheet":"Item"}"#),
            ("get_unit", r#"{"sheet":"Item","row":-1}"#),
            ("list_sheets", r#"{"unexpected":1}"#),
            ("read_rows", r#"{"sheet":"Item","after_subrow":1}"#),
            ("read_rows", r#"{"sheet":"Missing"}"#),
            ("write_everything", "{}"),
            ("project_overview", "not json"),
        ] {
            let (value, error) = run(&reader, name, arguments);
            assert!(error, "{name} {arguments}");
            assert!(value["error"].is_string());
        }
    }

    #[test]
    fn pending_changes_are_limited_and_navigation_is_forwarded() {
        let reader = reader();
        let (value, _) = run(&reader, "pending_changes", r#"{"limit":2}"#);
        assert_eq!(value["total"], 3);
        assert_eq!(value["changes"].as_array().expect("changes").len(), 2);

        let (value, error) = run(&reader, "navigate_to", r#"{"sheet":"Item","row":5}"#);
        assert!(!error);
        assert_eq!(value["opened"]["subrow"], 0);
        assert_eq!(reader.navigated.lock().expect("lock").len(), 1);
    }

    #[test]
    fn oversized_results_are_cut_with_a_notice() {
        let text = bounded_json(&json!({ "text": "x".repeat(MAX_RESULT_CHARS * 2) }));
        assert!(text.ends_with("request a smaller page]"));
        assert!(text.chars().count() < MAX_RESULT_CHARS + 100);
    }
}
