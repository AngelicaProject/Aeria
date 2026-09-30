//! `check`: the linter over `po/` and `aeria-knowledge/`. It changes no
//! translation; it keeps `po/README.md` current.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use aeria_knowledge::Knowledge;
use aeria_knowledge::rules::machine_phrasing;
use aeria_po::{GAME_VERSION_FIELD, Identity, PO_DIR, RowName};
use aeria_source::{GameSource, SheetLookup, SourceSheet};
use serde::Serialize;
use serde_json::json;

use super::project::Project;
use super::{Output, fmt_count};

pub(crate) struct CheckOptions {
    /// Every file, not only those changed since the last commit.
    pub all: bool,
}

/// One problem or piece of advice, with its place.
#[derive(Serialize)]
struct Finding {
    file: String,
    line: usize,
    message: String,
}

/// Russian forms that write both genders at once, such as `готов(а)`.
const BOTH_GENDERS: [&str; 6] = ["(а)", "(ла)", "(ая)", "(на)", "(ен)", "(ой)"];

/// Checks one translation against its source and the knowledge: what must be
/// fixed, and advice.
fn check_text(
    knowledge: &Knowledge,
    target_language: &str,
    source: &str,
    text: &str,
    extracted: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut reasons = Vec::new();
    let mut advice = Vec::new();
    if text.contains('\n') && !source.contains('\n') {
        reasons.push(
            "the translation has a line break the source does not; the game breaks lines with <br>"
                .to_owned(),
        );
    }
    if let Err(errors) = aeria_se::check_assisted_structure(source, text) {
        reasons.extend(errors.into_iter().map(|error| error.message));
    }
    reasons.extend(knowledge.terms.forbidden_in(source, text));
    let russian = target_language.eq_ignore_ascii_case("ru");
    if russian
        && let Some(form) = BOTH_GENDERS
            .iter()
            .find(|form| text.contains(*form) && !source.contains(*form))
    {
        reasons.push(format!(
            "{form} writes both genders at once; use a condition on $gn4 with the feminine form first, or a phrasing that shows no gender"
        ));
    }
    advice.extend(knowledge.terms.missing_in(source, text));
    if source.contains("$gn4") && !text.contains("$gn4") {
        advice.push("the source varies with the player character's gender and the translation does not; make sure nothing in it agrees with the player character's gender".to_owned());
    } else if russian
        && !source.contains("$gn4")
        && !text.contains("$gn4")
        && extracted.iter().any(|line| {
            (line.starts_with("fr: ") || line.starts_with("de: ")) && line.contains("$gn4")
        })
    {
        advice.push("the French or German line varies with the player character's gender; check whether a word about the player character needs a condition on $gn4".to_owned());
    }
    let phrasing = machine_phrasing(target_language, text);
    if !phrasing.is_empty() {
        advice.push(format!("reads machine-written: {}", phrasing.join(", ")));
    }
    (reasons, advice)
}

/// The installed game's strings, looked up by identity.
struct Game<'a> {
    source: &'a GameSource,
    sheets: HashMap<String, Option<std::sync::Arc<SourceSheet>>>,
    keys: HashMap<String, HashMap<String, (u32, u16)>>,
}

impl<'a> Game<'a> {
    fn new(source: &'a GameSource) -> Self {
        Self {
            source,
            sheets: HashMap::new(),
            keys: HashMap::new(),
        }
    }

    fn sheet(&mut self, name: &str) -> Option<std::sync::Arc<SourceSheet>> {
        self.sheets
            .entry(name.to_owned())
            .or_insert_with(|| match self.source.sheet(name) {
                Ok(SheetLookup::Present(sheet)) => Some(sheet),
                _ => None,
            })
            .clone()
    }

    /// The current text of a translatable string, or why there is none.
    fn text(&mut self, identity: &Identity) -> Result<String, &'static str> {
        let sheet = self
            .sheet(&identity.sheet)
            .ok_or("the installed game has no such sheet")?;
        let (row, subrow) = match &identity.row {
            RowName::Id { row, subrow } => (*row, *subrow),
            RowName::Key(key) => {
                let keys = self.keys.entry(identity.sheet.clone()).or_insert_with(|| {
                    sheet
                        .row_keys()
                        .map(|keys| {
                            sheet
                                .rows()
                                .iter()
                                .filter_map(|row| {
                                    keys.key_of(row.row_id, row.subrow_id)
                                        .map(|key| (key.to_owned(), (row.row_id, row.subrow_id)))
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                });
                *keys
                    .get(key)
                    .ok_or("the installed game has no row with this key")?
            }
        };
        let cell = sheet
            .cell(row, subrow, identity.column)
            .ok_or("the installed game has no such string")?;
        if !cell.translatable || cell.bytes.is_empty() {
            return Err("the installed game does not translate this string");
        }
        Ok(cell.text())
    }
}

/// What the checks found.
#[derive(Default)]
struct Findings {
    problems: Vec<Finding>,
    advice: Vec<Finding>,
    translated: usize,
    fuzzy: usize,
}

impl Findings {
    fn problem(&mut self, path: &str, line: usize, message: String) {
        self.problems.push(at(path, line, message));
    }
}

fn at(path: &str, line: usize, message: String) -> Finding {
    Finding {
        file: format!("{PO_DIR}/{path}"),
        line,
        message,
    }
}

/// What the file and the translation a check reads with.
struct Context<'a> {
    version: &'a str,
    knowledge: &'a Knowledge,
    target: &'a str,
}

/// Checks one file.
fn check_file(
    path: &str,
    file: &aeria_po::PoFile,
    context: &Context<'_>,
    game: &mut Game<'_>,
    found: &mut Findings,
) {
    if file.field(GAME_VERSION_FIELD) != Some(context.version) {
        found.problem(
            path,
            1,
            format!(
                "made for game version {}, and the installed game is {}: the files must be updated to it (`aeria update`, the user's step)",
                file.field(GAME_VERSION_FIELD).unwrap_or("unknown"),
                context.version
            ),
        );
        return;
    }
    let mut seen = BTreeSet::new();
    for entry in &file.entries {
        let identity = match Identity::parse(&entry.context) {
            Ok(identity) => identity,
            Err(message) => {
                found.problem(
                    path,
                    entry.line,
                    format!("{message}; leave msgctxt as Aeria wrote it"),
                );
                continue;
            }
        };
        if !seen.insert(entry.context.as_str()) {
            found.problem(
                path,
                entry.line,
                format!("{} appears twice in this file", entry.context),
            );
            continue;
        }
        match game.text(&identity) {
            Ok(text) if text == entry.source => {}
            Ok(_) => {
                found.problem(
                    path,
                    entry.line,
                    "msgid is not the game's text of this string; leave msgid as Aeria wrote it"
                        .to_owned(),
                );
                continue;
            }
            Err(message) => {
                found.problem(
                    path,
                    entry.line,
                    format!("{message}; leave msgctxt as Aeria wrote it"),
                );
                continue;
            }
        }
        if entry.fuzzy {
            found.fuzzy += 1;
        }
        if entry.translation.is_empty() {
            continue;
        }
        found.translated += 1;
        let (reasons, notes) = check_text(
            context.knowledge,
            context.target,
            &entry.source,
            &entry.translation,
            &entry.extracted,
        );
        for reason in reasons {
            found.problem(path, entry.line, reason);
        }
        found
            .advice
            .extend(notes.into_iter().map(|note| at(path, entry.line, note)));
    }
}

/// Runs the checks; returns whether there are no problems.
pub(crate) fn check(
    project: &Project,
    options: &CheckOptions,
    out: &mut Output,
) -> Result<bool, String> {
    super::texts::write_readme(project)?;
    let root = &project.root;
    let (paths, scope) = match (options.all, super::git::changed_po(root)) {
        (false, Some(changed)) => (changed, "changed since the last commit"),
        _ => (
            aeria_po::list(root).map_err(|error| error.to_string())?,
            "all",
        ),
    };
    let (files, broken) = aeria_po::read(root, &paths).map_err(|error| error.to_string())?;
    let mut found = Findings::default();
    for (path, problem) in broken {
        found.problem(&path, problem.line, problem.message);
    }
    let version = project.source.version().to_string();
    let knowledge = project.knowledge();
    let context = Context {
        version: &version,
        knowledge: &knowledge,
        target: &project.settings.target_language,
    };
    let mut game = Game::new(&project.source);
    for (path, file) in &files {
        check_file(path, file, &context, &mut game, &mut found);
    }
    let knowledge_problems = knowledge.problems.clone();
    let clean = found.problems.is_empty() && knowledge_problems.is_empty();
    if out.json {
        out.json_value(&json!({
            "scope": scope,
            "files": files.len(),
            "translated": found.translated,
            "fuzzy": found.fuzzy,
            "problems": found.problems,
            "knowledgeProblems": knowledge_problems,
            "advice": found.advice,
        }));
        return Ok(clean);
    }
    for finding in &found.problems {
        let _ = writeln!(
            out.text,
            "{}:{}: {}",
            finding.file, finding.line, finding.message
        );
    }
    for problem in &knowledge_problems {
        let _ = writeln!(out.text, "{problem}");
    }
    for finding in &found.advice {
        let _ = writeln!(
            out.text,
            "{}:{}: advice: {}",
            finding.file, finding.line, finding.message
        );
    }
    let _ = writeln!(
        out.text,
        "checked {} files ({scope}) · translated {} · fuzzy {} · problems {} · advice {}",
        fmt_count(files.len()),
        fmt_count(found.translated),
        fmt_count(found.fuzzy),
        fmt_count(found.problems.len() + knowledge_problems.len()),
        fmt_count(found.advice.len())
    );
    Ok(clean)
}
