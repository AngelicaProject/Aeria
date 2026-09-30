//! `corpus` and file checks: the whole game text as gettext PO files in the
//! project's `game/` folder, which agents read and translate like source
//! code, and `check` without input, which saves what they changed.
//!
//! Every translatable string is one PO entry: `msgctxt` is its address,
//! `msgid` its source, `msgstr` its translation, and extracted comments
//! (`#.`) the other client languages, the speaker, the row's other cells,
//! and what its macros do. A quest or cutscene is one file in play order;
//! another sheet is one file, or a folder of files of [`FILE_STRINGS`]
//! strings. `game/` is generated from the installed game and the project and
//! is never committed.
//!
//! `game/.aeria-state.json` keeps, for every translated string, the
//! translation its file was made with, and the size and time of each file
//! when Aeria last read or wrote it. A check reads the files that changed
//! since and those with problems left; a `msgstr` that differs from what the
//! file was made with is saved through the checks of `write`, unless the
//! project's translation changed meanwhile.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::project::{
    Address, LineKind, Project, current, matches_pattern, other_languages, read_sheet_lines,
    translated,
};
use super::read::{quest_title, sheet_counts};
use super::write::{self, Entry, Outcome, WriteOptions};
use super::{Output, fmt_count};

/// The folder of the corpus in the project.
pub(crate) const CORPUS_DIR: &str = "game";
const STATE_FILE: &str = ".aeria-state.json";
const README: &str = "README.md";
/// Strings per file of a sheet that is not a scene.
const FILE_STRINGS: usize = 200;

#[derive(Default, Deserialize, Serialize)]
struct State {
    /// Per file, relative to `game/` with `/`: how it was when last read or
    /// written.
    files: BTreeMap<String, FileStamp>,
    /// The translation each translated string's file was made with.
    baseline: BTreeMap<String, String>,
}

#[derive(Clone, Deserialize, PartialEq, Serialize)]
struct FileStamp {
    len: u64,
    modified: u128,
    /// The file has changes that were not saved: rejected, skipped, or in
    /// conflict.
    #[serde(default)]
    pending: bool,
}

fn stamp(path: &Path, pending: bool) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(FileStamp {
        len: metadata.len(),
        modified,
        pending,
    })
}

fn load_state(corpus: &Path) -> State {
    std::fs::read(corpus.join(STATE_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_state(corpus: &Path, state: &State) -> Result<(), String> {
    let path = corpus.join(STATE_FILE);
    let bytes = serde_json::to_vec(state).map_err(|error| error.to_string())?;
    std::fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn is_scene(sheet: &str) -> bool {
    sheet.starts_with("quest/") || sheet.starts_with("cut_scene/")
}

// ---------------------------------------------------------------- PO text

/// A PO string literal.
fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// The text of a PO string literal, or `None` when it is not one.
fn unquote(literal: &str) -> Option<String> {
    let inner = literal.trim().strip_prefix('"')?.strip_suffix('"')?;
    let mut text = String::with_capacity(inner.len());
    let mut characters = inner.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            text.push(character);
            continue;
        }
        match characters.next()? {
            'n' => text.push('\n'),
            'r' => text.push('\r'),
            't' => text.push('\t'),
            other => text.push(other),
        }
    }
    Some(text)
}

/// A comment line's text: one line, whatever the text holds.
fn comment(text: &str) -> String {
    text.replace('\r', "").replace('\n', "\\n")
}

/// One entry read back from a file.
#[derive(Debug, PartialEq)]
struct ParsedEntry {
    context: String,
    translation: String,
    /// The line of `msgstr`, from 1.
    line: usize,
}

/// Reads the entries of a PO file. Entries without `msgctxt` (the header)
/// are left out. A line that breaks the format is a problem with its line,
/// and the rest of its entry is skipped; the other entries are read.
fn parse_po(text: &str) -> (Vec<ParsedEntry>, Vec<(usize, String)>) {
    #[derive(Clone, Copy, PartialEq)]
    enum Field {
        None,
        Context,
        Id,
        Str,
        /// The entry broke the format; its lines are skipped.
        Broken,
    }
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    let mut context: Option<String> = None;
    let mut translation: Option<(String, usize)> = None;
    let mut field = Field::None;
    let finish = |context: &mut Option<String>,
                  translation: &mut Option<(String, usize)>,
                  entries: &mut Vec<ParsedEntry>| {
        if let (Some(context), Some((translation, line))) = (context.take(), translation.take()) {
            entries.push(ParsedEntry {
                context,
                translation,
                line,
            });
        }
        *context = None;
        *translation = None;
    };
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            if field == Field::Str {
                finish(&mut context, &mut translation, &mut entries);
                field = Field::None;
            }
            continue;
        }
        let result: Result<(), &str> = if let Some(rest) = line.strip_prefix("msgctxt ") {
            finish(&mut context, &mut translation, &mut entries);
            unquote(rest)
                .map(|text| {
                    context = Some(text);
                    field = Field::Context;
                })
                .ok_or("msgctxt is not a quoted string")
        } else if field == Field::Broken {
            Ok(())
        } else if let Some(rest) = line.strip_prefix("msgid ") {
            if field == Field::Str {
                finish(&mut context, &mut translation, &mut entries);
            }
            field = Field::Id;
            unquote(rest).map(|_| ()).ok_or("msgid is not a quoted string")
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            unquote(rest)
                .map(|text| {
                    translation = Some((text, number));
                    field = Field::Str;
                })
                .ok_or("msgstr is not a quoted string: write it as msgstr \"…\", with \\\" for a quote and \\\\ for a backslash")
        } else if line.starts_with('"') {
            match (unquote(line), field) {
                (None, _) => Err("not a quoted string"),
                (Some(text), Field::Context) => {
                    if let Some(context) = &mut context {
                        context.push_str(&text);
                    }
                    Ok(())
                }
                (Some(text), Field::Str) => {
                    if let Some((translation, _)) = &mut translation {
                        translation.push_str(&text);
                    }
                    Ok(())
                }
                (Some(_), Field::Id) => Ok(()),
                (Some(_), _) => Err("a string outside an entry"),
            }
        } else {
            Err("not a PO line: comments start with #, fields are msgctxt, msgid, msgstr")
        };
        if let Err(problem) = result {
            problems.push((number, problem.to_owned()));
            context = None;
            translation = None;
            field = Field::Broken;
        }
    }
    finish(&mut context, &mut translation, &mut entries);
    (entries, problems)
}

// ---------------------------------------------------------------- generating

/// One file of the corpus: its path relative to `game/` and its lines.
struct CorpusFile<'a> {
    relative: String,
    sheet: &'a str,
    part: Option<(usize, usize)>,
    lines: &'a [super::project::SheetLine],
}

fn file_text(project: &Project, file: &CorpusFile<'_>, title: Option<&str>, baseline: &mut BTreeMap<String, String>) -> String {
    let ledger = project.ledger();
    let mut text = String::new();
    let _ = write!(text, "# {}", file.sheet);
    if let Some(title) = title {
        let _ = write!(text, " — «{title}»");
    }
    if let Some((part, parts)) = file.part {
        let _ = write!(text, " · part {part} of {parts}");
    }
    let _ = writeln!(
        text,
        " · {} strings{}",
        file.lines.len(),
        if is_scene(file.sheet) {
            " in play order"
        } else {
            ""
        }
    );
    text.push_str(
        "# Translate by filling msgstr; `aeria check` saves every change. See game/README.md.\n",
    );
    let target = project.target_language().unwrap_or_default();
    let _ = write!(
        text,
        "msgid \"\"\nmsgstr \"\"\n\"Language: {target}\\n\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n"
    );
    for line in file.lines {
        text.push('\n');
        let address = line.address.to_string();
        for (code, other) in other_languages(project, &line.address) {
            let _ = writeln!(text, "#. {code}: {}", comment(&other));
        }
        match &line.kind {
            LineKind::Speech(speaker) => {
                let _ = writeln!(text, "#. speaker: {}", comment(speaker));
            }
            LineKind::Journal | LineKind::Objective | LineKind::Other => {
                let _ = writeln!(text, "#. kind: {}", line.kind.label());
            }
            LineKind::Text => {}
        }
        for (column, other) in &line.context {
            let _ = writeln!(text, "#. column {column}: {}", comment(other));
        }
        if let Ok(constructs) = aeria_se::constructs(&line.source) {
            for construct in &constructs {
                let _ = writeln!(text, "#. macro: {}", comment(&construct.legend()));
            }
        }
        let now = translated(&project.session, &line.address)
            .then(|| current(&project.session, ledger.as_ref(), &line.address))
            .flatten();
        if let Some(now) = &now {
            if !now.replaceable() {
                text.push_str("#, keep\n");
            }
            if let Some(note) = &now.note {
                let _ = writeln!(text, "# note: {}", comment(note));
            }
            baseline.insert(address.clone(), now.target.clone());
        } else {
            baseline.remove(&address);
        }
        let _ = writeln!(text, "msgctxt {}", quote(&address));
        let _ = writeln!(text, "msgid {}", quote(&line.source));
        let _ = writeln!(
            text,
            "msgstr {}",
            quote(now.as_ref().map_or("", |now| now.target.as_str()))
        );
    }
    text
}

/// Adds `/game/` to the project's `.gitignore`.
fn ignore_corpus(root: &Path) -> Result<(), String> {
    let path = root.join(".gitignore");
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let rule = format!("/{CORPUS_DIR}/");
    if current.lines().any(|line| line.trim() == rule) {
        return Ok(());
    }
    let mut next = current.trim_end().to_owned();
    if !next.is_empty() {
        next.push('\n');
    }
    let _ = writeln!(
        next,
        "# The game's text for agents, made by `aeria corpus`; never committed.\n{rule}"
    );
    std::fs::write(&path, next).map_err(|error| format!("{}: {error}", path.display()))
}

/// Files with changes a check has not saved yet.
fn unsaved_files(corpus: &Path, state: &State) -> Vec<String> {
    state
        .files
        .iter()
        .filter(|(relative, recorded)| {
            recorded.pending
                || stamp(&corpus.join(relative.as_str()), false).is_none_or(|now| {
                    now.len != recorded.len || now.modified != recorded.modified
                })
        })
        .map(|(relative, _)| relative.clone())
        .collect()
}

pub(crate) struct CorpusOptions {
    pub pattern: Option<String>,
    /// Replace files with changes a check has not saved.
    pub force: bool,
}

/// Writes the corpus, or the files of the matching sheets.
pub(crate) fn corpus(project: &Project, options: &CorpusOptions, out: &mut Output) -> Result<(), String> {
    let root = project.root().to_owned();
    let corpus = root.join(CORPUS_DIR);
    std::fs::create_dir_all(&corpus).map_err(|error| format!("{}: {error}", corpus.display()))?;
    ignore_corpus(&root)?;
    let mut state = load_state(&corpus);
    let unsaved: BTreeSet<String> = unsaved_files(&corpus, &state).into_iter().collect();
    let sheets: Vec<String> = sheet_counts(project)
        .into_iter()
        .map(|sheet| sheet.name)
        .filter(|name| options.pattern.as_deref().is_none_or(|pattern| matches_pattern(name, pattern)))
        .collect();
    let mut written = 0usize;
    let mut strings = 0usize;
    let mut kept: Vec<String> = Vec::new();
    for sheet in &sheets {
        let lines = read_sheet_lines(project, sheet)?;
        if lines.is_empty() {
            continue;
        }
        strings += lines.len();
        let title = if sheet.starts_with("quest/") {
            quest_title(project, sheet)
        } else {
            None
        };
        let files: Vec<CorpusFile<'_>> = if is_scene(sheet) || lines.len() <= FILE_STRINGS {
            vec![CorpusFile {
                relative: format!("{sheet}.po"),
                sheet,
                part: None,
                lines: &lines,
            }]
        } else {
            let parts = lines.len().div_ceil(FILE_STRINGS);
            let width = parts.to_string().len().max(3);
            lines
                .chunks(FILE_STRINGS)
                .enumerate()
                .map(|(index, chunk)| CorpusFile {
                    relative: format!("{sheet}/{:0width$}.po", index + 1),
                    sheet,
                    part: Some((index + 1, parts)),
                    lines: chunk,
                })
                .collect()
        };
        for file in &files {
            if unsaved.contains(&file.relative) && !options.force {
                kept.push(file.relative.clone());
                continue;
            }
            let path = corpus.join(&file.relative);
            let text = file_text(project, file, title.as_deref(), &mut state.baseline);
            if std::fs::read(&path).ok().as_deref() != Some(text.as_bytes()) {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| format!("{}: {error}", parent.display()))?;
                }
                std::fs::write(&path, &text)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                written += 1;
            }
            if let Some(now) = stamp(&path, false) {
                state.files.insert(file.relative.clone(), now);
            }
        }
    }
    std::fs::write(corpus.join(README), readme(project))
        .map_err(|error| format!("{}: {error}", corpus.join(README).display()))?;
    save_state(&corpus, &state)?;
    if out.json {
        out.json_value(&json!({
            "folder": format!("{}/{CORPUS_DIR}", project.root_display()),
            "sheets": sheets.len(),
            "strings": strings,
            "filesWritten": written,
            "keptUnsaved": kept,
        }));
        return Ok(());
    }
    let _ = writeln!(
        out.text,
        "{}/{CORPUS_DIR}: {} strings of {} sheets · {} files written",
        project.root_display(),
        fmt_count(strings),
        fmt_count(sheets.len()),
        fmt_count(written)
    );
    if !kept.is_empty() {
        let _ = writeln!(
            out.text,
            "{} files kept because they have changes `aeria check` has not saved; check them, or pass --force to replace them: {}",
            kept.len(),
            kept.join(", ")
        );
    }
    Ok(())
}

fn readme(project: &Project) -> String {
    let mut text = format!(
        "# The text of FINAL FANTASY XIV\n\n\
Every translatable string of the game, version {}, made by `aeria corpus` from the \
installed game and this project. It is never committed; `aeria corpus` makes it again, for \
example after a game update.\n\n\
## Layout\n\n\
- A quest (`quest/…`) or cutscene (`cut_scene/…`) is one file, its strings in the order \
the player sees them.\n\
- Any other sheet is one file, or a folder of files of {FILE_STRINGS} strings in row \
order, such as `BNpcName/001.po`.\n\
- Each string is a gettext PO entry: `msgctxt` is its address, `msgid` the {} source, \
`msgstr` the {} translation, empty while there is none. The `#.` lines above it are the \
other client languages (`ja` is the original), the speaker, the row's other cells, and \
what the macros do.\n\
- `#, keep` marks a translation a person wrote or reviewed: leave it as it is.\n\
- Search the whole game with any tool, for example to see how a name is used \
everywhere or how the official localizations handled a macro.\n\n\
## Translating\n\n\
Write the translation into `msgstr` as a PO string: `\\\"` for a quote, `\\\\` for a \
backslash, one line (the game breaks lines with `<br>`). Then run `aeria check`: it saves \
every changed `msgstr` that passes the checks and lists each problem as \
`file:line: what to fix`. A translation that was not saved stays in its file until it is \
fixed. Terms, style, character voices, and the story so far are in `../aeria-knowledge/`.\n\n",
        project.session.source().version(),
        project.source_language(),
        project.target_language().unwrap_or_else(|| "target".to_owned()),
    );
    text.push_str(&super::texts::brief(project));
    text
}

// ---------------------------------------------------------------- checking

/// One changed translation found in a file.
struct Change {
    relative: String,
    line: usize,
    address: Address,
    text: String,
}

/// A problem with one string of a file.
#[derive(Serialize)]
struct Problem {
    file: String,
    line: usize,
    message: String,
}

fn corpus_files(corpus: &Path) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    let mut folders = vec![corpus.to_owned()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|extension| extension == "po")
                && let Ok(relative) = path.strip_prefix(corpus)
            {
                files.push((relative.to_string_lossy().replace('\\', "/"), path));
            }
        }
    }
    files.sort();
    files
}

/// Saves the translations changed in the corpus; returns whether nothing is
/// left to fix.
#[allow(clippy::too_many_lines)] // one pass over the files
pub(crate) fn check_files(project: &mut Project, out: &mut Output) -> Result<bool, String> {
    let corpus = project.root().join(CORPUS_DIR);
    if !corpus.is_dir() {
        return Err(format!(
            "give translations on standard input or in a file, or make the game's text with `aeria corpus` and check {CORPUS_DIR}/"
        ));
    }
    let mut state = load_state(&corpus);
    let ledger = project.ledger();
    let mut changes: Vec<Change> = Vec::new();
    let mut problems: Vec<Problem> = Vec::new();
    let mut read_files: BTreeSet<String> = BTreeSet::new();
    for (relative, path) in corpus_files(&corpus) {
        let now = stamp(&path, false);
        let recorded = state.files.get(&relative);
        let to_read = match (recorded, &now) {
            (Some(recorded), Some(now)) => {
                recorded.pending || recorded.len != now.len || recorded.modified != now.modified
            }
            _ => true,
        };
        if !to_read {
            continue;
        }
        read_files.insert(relative.clone());
        let text = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let (entries, broken) = parse_po(&text);
        problems.extend(broken.into_iter().map(|(line, message)| Problem {
            file: relative.clone(),
            line,
            message,
        }));
        for entry in entries {
            let Ok(address) = Address::parse(&entry.context) else {
                problems.push(Problem {
                    file: relative.clone(),
                    line: entry.line,
                    message: format!("msgctxt {:?} is not an address; leave msgctxt as it is", entry.context),
                });
                continue;
            };
            let key = address.to_string();
            let made_with = state.baseline.get(&key).map_or("", String::as_str);
            if entry.translation == made_with {
                continue;
            }
            let project_now = current(&project.session, ledger.as_ref(), &address).map(|now| now.target);
            if project_now.as_deref() == Some(entry.translation.as_str()) {
                state.baseline.insert(key, entry.translation);
                continue;
            }
            if project_now.as_deref().unwrap_or("") != made_with {
                problems.push(Problem {
                    file: relative.clone(),
                    line: entry.line,
                    message: format!(
                        "the translation changed in the project after this file was made (now {}); run `aeria corpus` to see it, then make your change again",
                        project_now.as_deref().map_or_else(|| "none".to_owned(), quote)
                    ),
                });
                continue;
            }
            if entry.translation.trim().is_empty() {
                problems.push(Problem {
                    file: relative.clone(),
                    line: entry.line,
                    message: "a translation cannot be removed here; put it back or remove it in Aeria".to_owned(),
                });
                continue;
            }
            changes.push(Change {
                relative: relative.clone(),
                line: entry.line,
                address,
                text: entry.translation,
            });
        }
    }
    let entries: Vec<Entry> = changes
        .iter()
        .map(|change| Entry {
            address: change.address.clone(),
            text: change.text.clone(),
        })
        .collect();
    let (outcomes, written) = if entries.is_empty() {
        (Vec::new(), 0)
    } else {
        write::apply(project, &entries, &WriteOptions { needs_review: false }, out)?
    };
    let outcomes: BTreeMap<String, Outcome> = outcomes.into_iter().collect();
    let mut advice = 0usize;
    for change in &changes {
        let key = change.address.to_string();
        let message = match outcomes.get(&key) {
            Some(Outcome::Ok { advice: notes }) => {
                state.baseline.insert(key, change.text.clone());
                if notes.is_empty() {
                    continue;
                }
                advice += 1;
                let _ = writeln!(
                    out.text,
                    "{CORPUS_DIR}/{}:{}: saved; advice: {}",
                    change.relative,
                    change.line,
                    notes.join("; ")
                );
                continue;
            }
            Some(Outcome::Unchanged) => {
                state.baseline.insert(key, change.text.clone());
                continue;
            }
            Some(Outcome::Rejected { reasons }) => format!("rejected: {}", reasons.join("; ")),
            Some(Outcome::Skipped { reason }) => format!("not saved: {reason}"),
            Some(Outcome::Failed { reason }) => format!("not saved ({reason}); run `aeria check` again"),
            None => "not saved".to_owned(),
        };
        problems.push(Problem {
            file: change.relative.clone(),
            line: change.line,
            message,
        });
    }
    problems.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    let pending: BTreeSet<&str> = problems.iter().map(|problem| problem.file.as_str()).collect();
    for relative in &read_files {
        if let Some(now) = stamp(&corpus.join(relative), pending.contains(relative.as_str())) {
            state.files.insert(relative.clone(), now);
        }
    }
    save_state(&corpus, &state)?;
    if out.json {
        out.json_value(&json!({
            "saved": written,
            "problems": problems,
        }));
        return Ok(problems.is_empty());
    }
    for problem in &problems {
        let _ = writeln!(
            out.text,
            "{CORPUS_DIR}/{}:{}: {}",
            problem.file, problem.line, problem.message
        );
    }
    let _ = writeln!(
        out.text,
        "{} translations saved{} · {} problems{}",
        fmt_count(written),
        if advice > 0 {
            format!(", {advice} with advice")
        } else {
            String::new()
        },
        fmt_count(problems.len()),
        if problems.is_empty() {
            String::new()
        } else {
            "; a translation that was not saved stays in its file until it is fixed".to_owned()
        }
    );
    Ok(problems.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn po_strings_round_trip() {
        for text in ["plain", "a \"quote\" and \\<br\\>", "tab\tand\nline", ""] {
            assert_eq!(unquote(&quote(text)).as_deref(), Some(text));
        }
    }

    #[test]
    fn entries_are_read_with_continued_strings_and_their_lines() {
        let text = "msgid \"\"\nmsgstr \"\"\n\"Language: ru\\n\"\n\n#. ja: 名前\n#, keep\nmsgctxt \"BNpcName:1:0:0\"\nmsgid \"Name\"\nmsgstr \"Имя\"\n\nmsgctxt \"BNpcName:2:0:0\"\nmsgid \"Long\"\nmsgstr \"\"\n\"Длин\"\n\"ное\"\n";
        assert_eq!(
            parse_po(text).0,
            [
                ParsedEntry {
                    context: "BNpcName:1:0:0".to_owned(),
                    translation: "Имя".to_owned(),
                    line: 9,
                },
                ParsedEntry {
                    context: "BNpcName:2:0:0".to_owned(),
                    translation: "Длинное".to_owned(),
                    line: 13,
                },
            ]
        );
        let (entries, problems) = parse_po(
            "msgctxt \"a:1:0:0\"\nmsgid \"x\"\nmsgstr Имя\n\nmsgctxt \"a:2:0:0\"\nmsgid \"y\"\nmsgstr \"Да\"\n",
        );
        assert_eq!(problems.iter().map(|(line, _)| *line).collect::<Vec<_>>(), [3]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].translation, "Да");
    }
}
