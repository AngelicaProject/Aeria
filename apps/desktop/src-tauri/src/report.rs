//! Run reports in the desktop: written beside each run's journal in `logs/`
//! of the data folder, listed, and read a page at a time with what to list
//! and a search (see `docs/architecture/translate.md#report`).

use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, Write as _};
use std::path::{Path, PathBuf};

use aeria_model::report::{Record, Report, VERSION};
use serde::Serialize;
use serde_json::Value;
use tauri::Manager;

use crate::commands::run_blocking;
use crate::dto::IssueDto;
use crate::error::CommandError;
use crate::paths::AeriaPaths;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// The file name prefix of journals and reports.
const PREFIX: &str = "translation-";
/// The extension of a report.
const EXTENSION: &str = "jsonl";
/// Items a page lists at most.
const PAGE: usize = 2000;
/// The kinds of run this build reads the reports of; a report of another
/// kind, written by another build, is not listed or read.
const KINDS: [&str; 2] = ["translating", "fixing"];

/// Whether `run`, a report's first record, is of a run this build reads.
fn readable(run: &Value) -> bool {
    run["record"] == "run"
        && run["version"] == VERSION
        && run["kind"]
            .as_str()
            .is_some_and(|kind| KINDS.contains(&kind))
}

/// The folder of the journals and reports.
pub(crate) fn folder(app: &tauri::AppHandle) -> Option<PathBuf> {
    Some(app.aeria_data_dir().ok()?.join("logs"))
}

/// The problems of an answer as the editor's issues: those the checks
/// found as data, the others by their message.
pub(crate) fn problem_dtos(problems: &[String], issues: &[aeria_po::Issue]) -> Vec<IssueDto> {
    let mut dtos: Vec<IssueDto> = problems
        .iter()
        .filter(|message| {
            !issues
                .iter()
                .any(|issue| aeria_model::run::names(message, issue))
        })
        .map(|message| IssueDto {
            kind: "other".to_owned(),
            group: "other".to_owned(),
            message: message.clone(),
            ..IssueDto::default()
        })
        .collect();
    dtos.extend(issues.iter().map(IssueDto::from));
    dtos
}

/// A report written as JSON Lines, a line per record, each flushed so a
/// report reads while its run goes.
pub(crate) struct ReportFile(std::fs::File);

impl ReportFile {
    pub(crate) fn new(file: std::fs::File) -> Self {
        Self(file)
    }
}

impl Report for ReportFile {
    fn write(&mut self, record: &Record) -> std::io::Result<()> {
        let mut value = serde_json::to_value(record).map_err(std::io::Error::other)?;
        // The issues of each answer are worded by the interface as the
        // editor's.
        if let (Record::String(string), Some(attempts)) = (
            record,
            value.get_mut("attempts").and_then(Value::as_array_mut),
        ) {
            for (attempt, value) in string.attempts.iter().zip(attempts) {
                value["problems"] =
                    serde_json::to_value(problem_dtos(&attempt.problems, &attempt.issues))
                        .map_err(std::io::Error::other)?;
            }
        }
        let mut line = serde_json::to_string(&value).map_err(std::io::Error::other)?;
        line.push('\n');
        self.0.write_all(line.as_bytes())?;
        self.0.flush()
    }
}

/// A report of the data folder.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportInfoDto {
    pub id: String,
    /// Its `run` record.
    pub run: Value,
}

/// The path of the report `id`, a run's start time in Unix milliseconds.
fn path_of(app: &tauri::AppHandle, id: &str) -> CommandResult<PathBuf> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(not_found());
    }
    Ok(folder(app)
        .ok_or_else(not_found)?
        .join(format!("{PREFIX}{id}.{EXTENSION}")))
}

fn not_found() -> CommandError {
    CommandError::new(
        "reportNotFound",
        "the run's report is not in the data folder",
    )
}

/// The reports of `folder`, newest first.
fn reports_of(folder: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|path| {
            if path
                .extension()
                .is_none_or(|extension| extension != EXTENSION)
            {
                return None;
            }
            let id = path.file_stem()?.to_str()?.strip_prefix(PREFIX)?.to_owned();
            id.bytes()
                .all(|byte| byte.is_ascii_digit())
                .then_some((id, path))
        })
        .collect();
    found.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| b.0.cmp(&a.0)));
    found
}

#[tauri::command(rename_all = "camelCase")]
/// The reports of the data folder, newest first.
///
/// # Errors
///
/// Returns an internal error when the desktop worker fails.
pub async fn run_reports(app: tauri::AppHandle) -> CommandResult<Vec<ReportInfoDto>> {
    run_blocking(move || {
        let Some(folder) = folder(&app) else {
            return Ok(Vec::new());
        };
        Ok(reports_of(&folder)
            .into_iter()
            .filter_map(|(id, path)| {
                let file = std::fs::File::open(path).ok()?;
                let mut first = String::new();
                std::io::BufReader::new(file).read_line(&mut first).ok()?;
                let run: Value = serde_json::from_str(&first).ok()?;
                readable(&run).then_some(ReportInfoDto { id, run })
            })
            .collect())
    })
    .await
}

/// What a record is listed as: `string:<outcome>`, with a written string
/// as `translated`, `changed`, or `kept` (an adaptation that kept its text
/// and cleared the fuzzy mark), or `left`; `None` for the run's first and
/// last records.
fn view_of(record: &Value) -> Option<String> {
    let outcome = record["outcome"].as_str().unwrap_or_default();
    match record["record"].as_str()? {
        "string" if outcome == "written" => {
            let before = record["before"].as_str().unwrap_or_default();
            Some(if before.is_empty() {
                "string:translated".to_owned()
            } else if record["after"].as_str() == Some(before) {
                "string:kept".to_owned()
            } else {
                "string:changed".to_owned()
            })
        }
        "string" => Some(format!("string:{outcome}")),
        "left" => Some("left".to_owned()),
        _ => None,
    }
}

/// Whether `record` has `query`, lowercase, in its terms, context,
/// paths, source, or translations.
fn has(record: &Value, query: &str) -> bool {
    fn walk(value: &Value, query: &str) -> bool {
        match value {
            Value::String(text) => text.to_lowercase().contains(query),
            Value::Array(items) => items.iter().any(|item| walk(item, query)),
            Value::Object(fields) => fields
                .iter()
                .filter(|(name, _)| !matches!(name.as_str(), "record" | "outcome" | "problems"))
                .any(|(_, value)| walk(value, query)),
            _ => false,
        }
    }
    query.is_empty() || walk(record, query)
}

/// A page of a report.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPageDto {
    /// The `run` record.
    pub run: Option<Value>,
    /// The `stop` record, once the run stopped.
    pub stop: Option<Value>,
    /// The items of each view, before the search.
    pub counts: BTreeMap<String, usize>,
    /// The items the view and search take.
    pub total: usize,
    /// Those items from `offset`, each with `view` and, for a string, where it
    /// is in the game as `binding`.
    pub items: Vec<Value>,
    /// Its run goes now.
    pub running: bool,
    /// Its run can be continued: it stopped before it settled every string
    /// it planned, or the service refused requests of it, and the report
    /// keeps what it was asked.
    pub continuable: bool,
}

#[tauri::command(rename_all = "camelCase")]
/// A page of the report `id`: the items of `view` (see [`view_of`]), or of
/// every view when empty, that have `query`, from `offset`, at most `limit`.
///
/// # Errors
///
/// Returns `reportNotFound` when there is no such report.
pub async fn run_report(
    app: tauri::AppHandle,
    id: String,
    view: String,
    query: String,
    offset: usize,
    limit: usize,
) -> CommandResult<ReportPageDto> {
    run_blocking(move || {
        let path = path_of(&app, &id)?;
        let file = std::fs::File::open(&path).map_err(|_| not_found())?;
        let query = query.trim().to_lowercase();
        let mut page = ReportPageDto {
            run: None,
            stop: None,
            counts: BTreeMap::new(),
            total: 0,
            items: Vec::new(),
            running: crate::translate::running_report(&app).as_deref() == Some(id.as_str()),
            continuable: false,
        };
        let mut options = false;
        let limit = limit.min(PAGE);
        for line in std::io::BufReader::new(file).lines() {
            let Ok(line) = line else { break };
            // A line the run is still writing, or one this build does not
            // read, is skipped.
            let Ok(mut record) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match record["record"].as_str() {
                Some("run") if !readable(&record) => return Err(not_found()),
                Some("run") => page.run = Some(record),
                Some("options") => options = true,
                Some("stop") => page.stop = Some(record),
                _ => {
                    let Some(of) = view_of(&record) else { continue };
                    *page.counts.entry(of.clone()).or_default() += 1;
                    if (!view.is_empty() && of != view) || !has(&record, &query) {
                        continue;
                    }
                    page.total += 1;
                    if page.total > offset && page.items.len() < limit {
                        record["view"] = Value::String(of);
                        page.items.push(record);
                    }
                }
            }
        }
        let finished = page
            .stop
            .as_ref()
            .is_some_and(|stop| stop["stop"]["reason"] == "finished");
        page.continuable = options
            && !page.running
            && (!finished || page.counts.get("left").is_some_and(|count| *count > 0));
        let session = app.state::<DesktopState>().session().ok();
        if let Some(session) = session {
            for item in &mut page.items {
                if let Some(context) = item["context"].as_str().map(str::to_owned) {
                    item["binding"] =
                        serde_json::to_value(crate::search::binding(&session, &context))
                            .unwrap_or(Value::Null);
                }
            }
        }
        Ok(page)
    })
    .await
}

/// What the run of the report `id` was asked, and every string it and the
/// runs it continues settled, by `msgctxt`.
fn continuation(
    app: &tauri::AppHandle,
    id: &str,
) -> CommandResult<(aeria_model::Options, HashSet<String>)> {
    let mut settled = HashSet::new();
    let mut options = None;
    let mut next = Some(id.to_owned());
    let mut seen = HashSet::new();
    while let Some(id) = next.take() {
        if !seen.insert(id.clone()) {
            break;
        }
        let Ok(file) = std::fs::File::open(path_of(app, &id)?) else {
            // A report deleted with the oldest journals: its strings are
            // asked again.
            break;
        };
        for line in std::io::BufReader::new(file).lines() {
            let Ok(line) = line else { break };
            let Ok(record) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match record["record"].as_str() {
                Some("options") => {
                    if options.is_none() {
                        options = Some(
                            serde_json::from_value::<aeria_model::Options>(
                                record["options"].clone(),
                            )
                            .map_err(|_| not_continuable())?,
                        );
                    }
                    next = record["continues"].as_str().map(str::to_owned);
                }
                Some("string") => {
                    if let Some(context) = record["context"].as_str() {
                        settled.insert(context.to_owned());
                    }
                }
                _ => {}
            }
        }
    }
    Ok((options.ok_or_else(not_continuable)?, settled))
}

fn not_continuable() -> CommandError {
    CommandError::new(
        "reportNotContinuable",
        "the run's report does not keep what it was asked",
    )
}

#[tauri::command(rename_all = "camelCase")]
/// Continues the run of the report `id`: the same options, for the strings
/// it and the runs it continues did not settle.
///
/// # Errors
///
/// Returns `translationRunning` while a run goes, `reportNotFound` or
/// `reportNotContinuable` for a report that cannot be continued,
/// `noProjectOpen`, or an error of the credential store.
pub async fn translation_continue(app: tauri::AppHandle, id: String) -> CommandResult<()> {
    if crate::translate::is_running(&app) {
        return Err(crate::translate::running());
    }
    let (options, settled) = run_blocking({
        let app = app.clone();
        let id = id.clone();
        move || continuation(&app, &id)
    })
    .await?;
    crate::translate::start_run_with(&app, options, Some((id, settled)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_are_listed_by_what_became_of_them() {
        let string = |before: &str, after: &str| serde_json::json!({"record": "string", "outcome": "written", "before": before, "after": after});
        assert_eq!(
            view_of(&string("", "а")).as_deref(),
            Some("string:translated")
        );
        assert_eq!(
            view_of(&string("а", "б")).as_deref(),
            Some("string:changed")
        );
        assert_eq!(view_of(&string("а", "а")).as_deref(), Some("string:kept"));
        let refused =
            serde_json::json!({"record": "string", "outcome": "refused", "source": "Moogle"});
        assert_eq!(view_of(&refused).as_deref(), Some("string:refused"));
        assert!(has(&refused, "moogle"));
        assert!(!has(&refused, "refused"));
        assert_eq!(view_of(&serde_json::json!({"record": "run"})), None);
    }

    #[test]
    fn a_report_of_another_build_is_not_read() {
        let run = |kind: &str| serde_json::json!({"record": "run", "version": 1, "kind": kind});
        assert!(readable(&run("translating")));
        assert!(readable(&run("fixing")));
        assert!(!readable(&run("reviewing")));
        assert!(!readable(&run("reconciling")));
        assert!(!readable(
            &serde_json::json!({"record": "run", "version": 2, "kind": "fixing"})
        ));
    }

    #[test]
    fn reports_are_listed_newest_first() {
        let folder = tempfile::tempdir().expect("folder");
        for name in [
            "translation-900.jsonl",
            "translation-1000.jsonl",
            "translation-1000.log",
            "other.jsonl",
        ] {
            std::fs::write(folder.path().join(name), "").expect("write");
        }
        let ids: Vec<String> = reports_of(folder.path())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, ["1000", "900"]);
    }
}
