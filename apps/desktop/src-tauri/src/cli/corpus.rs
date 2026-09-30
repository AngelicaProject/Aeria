//! `corpus` and file checks: the whole game text as gettext PO files in the
//! project's `game/` folder, which agents read and translate like source
//! code, and `check` without input, which saves what they changed.
//!
//! Every translatable string is one PO entry: `msgctxt` is its address,
//! `msgid` its source, `msgstr` its translation, and extracted comments
//! (`#.`) the other client languages, the speaker, the row's other cells,
//! and what its macros do. A quest or cutscene is one file in play order;
//! another sheet is one file, or a folder of files of [`FILE_STRINGS`]
//! strings. `game/` is made from the installed game and the project and is
//! never committed.
//!
//! `game/` always shows the project: `check` and the project's server bring
//! files up to date when translations change elsewhere, and after a game
//! update every file is made again. A translation whose source changed shows
//! `#, fuzzy` and the previous source as `#| msgid`, taken from the file
//! before it was made again. A file with changes not saved yet is never
//! replaced.
//!
//! `game/.aeria-state.json` keeps the game version the files were made for,
//! the translation each translated string's file shows, a digest of how it
//! shows it, and the size and time of each file when Aeria last read or
//! wrote it. A check reads the files that changed since and those with
//! problems left; a `msgstr` that differs from what its file shows is saved
//! through the checks of `write`, unless its source or its translation in
//! the project changed meanwhile.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use aeria_core::ReviewState;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::project::{
    Address, Author, Current, LineKind, Project, SheetLine, current, matches_pattern,
    other_languages, read_sheet_lines, sheet_lines, source_of,
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
    /// The game version the files were made for.
    #[serde(default)]
    game_version: String,
    /// Per file, relative to `game/` with `/`: how it was when last read or
    /// written.
    files: BTreeMap<String, FileStamp>,
    /// The translation each translated string's file shows.
    baseline: BTreeMap<String, String>,
    /// A digest of how each translated string's file shows it: translation,
    /// author, review state, and note.
    #[serde(default)]
    shown: BTreeMap<String, u64>,
}

#[derive(Clone, Deserialize, PartialEq, Serialize)]
struct FileStamp {
    len: u64,
    modified: u128,
    /// The file has changes that were not saved: rejected, skipped, or in
    /// conflict.
    #[serde(default)]
    pending: bool,
    /// The game version the file was made for; a file kept through a game
    /// update because of unsaved changes is made again once they are gone.
    #[serde(default)]
    game_version: String,
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
        game_version: String::new(),
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

/// Whether a file has changes a check has not saved: it has problems left,
/// or it changed since Aeria last read or wrote it.
fn unsaved(corpus: &Path, state: &State, relative: &str) -> bool {
    state.files.get(relative).is_some_and(|recorded| {
        recorded.pending
            || stamp(&corpus.join(relative), false)
                .is_none_or(|now| now.len != recorded.len || now.modified != recorded.modified)
    })
}

fn is_scene(sheet: &str) -> bool {
    sheet.starts_with("quest/") || sheet.starts_with("cut_scene/")
}

/// A stable digest (FNV-1a), so the state means the same to every build.
fn digest(parts: &[&str]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.bytes().chain([0xff]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

fn shown_digest(target: &str, author: Author, review: ReviewState, note: Option<&str>) -> u64 {
    let author = match author {
        Author::Agent => "agent",
        Author::Person => "person",
    };
    digest(&[target, author, &format!("{review:?}"), note.unwrap_or("")])
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
#[derive(Debug, Default, PartialEq)]
struct ParsedEntry {
    context: String,
    source: String,
    translation: String,
    /// The line of `msgstr`, from 1.
    line: usize,
    fuzzy: bool,
    /// The previous source, `#| msgid`.
    previous: Option<String>,
}

/// Reads the entries of a PO file. Entries without `msgctxt` (the header)
/// are left out. A line that breaks the format is a problem with its line,
/// and the rest of its entry is skipped; the other entries are read.
#[allow(clippy::too_many_lines)] // one pass over the lines
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
    fn close(open: &mut Option<ParsedEntry>, entries: &mut Vec<ParsedEntry>) {
        if let Some(entry) = open.take()
            && entry.line > 0
        {
            entries.push(entry);
        }
    }
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    let mut open: Option<ParsedEntry> = None;
    let mut field = Field::None;
    // Flags and the previous source come before the entry they belong to.
    let mut fuzzy = false;
    let mut previous: Option<String> = None;
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            if field == Field::Str {
                close(&mut open, &mut entries);
                field = Field::None;
            }
            if let Some(rest) = line.strip_prefix("#|") {
                let rest = rest.trim();
                if let Some(literal) = rest.strip_prefix("msgid ") {
                    previous = unquote(literal);
                } else if let (Some(text), Some(previous)) = (unquote(rest), &mut previous) {
                    previous.push_str(&text);
                }
            } else if let Some(flags) = line.strip_prefix("#,") {
                fuzzy |= flags.split(',').any(|flag| flag.trim() == "fuzzy");
            }
            continue;
        }
        let result: Result<(), &str> = if let Some(rest) = line.strip_prefix("msgctxt ") {
            close(&mut open, &mut entries);
            unquote(rest)
                .map(|context| {
                    open = Some(ParsedEntry {
                        context,
                        fuzzy: std::mem::take(&mut fuzzy),
                        previous: previous.take(),
                        ..ParsedEntry::default()
                    });
                    field = Field::Context;
                })
                .ok_or("msgctxt is not a quoted string")
        } else if field == Field::Broken {
            Ok(())
        } else if let Some(rest) = line.strip_prefix("msgid ") {
            if field == Field::Str {
                close(&mut open, &mut entries);
            }
            field = Field::Id;
            unquote(rest)
                .map(|source| {
                    if let Some(entry) = &mut open {
                        entry.source = source;
                    }
                })
                .ok_or("msgid is not a quoted string")
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            unquote(rest)
                .map(|translation| {
                    if let Some(entry) = &mut open {
                        entry.translation = translation;
                        entry.line = number;
                    }
                    field = Field::Str;
                })
                .ok_or("msgstr is not a quoted string: write it as msgstr \"…\", with \\\" for a quote and \\\\ for a backslash")
        } else if line.starts_with('"') {
            match (unquote(line), field, &mut open) {
                (None, _, _) => Err("not a quoted string"),
                (Some(text), Field::Context, Some(entry)) => {
                    entry.context.push_str(&text);
                    Ok(())
                }
                (Some(text), Field::Id, Some(entry)) => {
                    entry.source.push_str(&text);
                    Ok(())
                }
                (Some(text), Field::Str, Some(entry)) => {
                    entry.translation.push_str(&text);
                    Ok(())
                }
                (Some(_), Field::Id | Field::Str, None) => Ok(()),
                (Some(_), _, _) => Err("a string outside an entry"),
            }
        } else {
            Err("not a PO line: comments start with #, fields are msgctxt, msgid, msgstr")
        };
        if let Err(problem) = result {
            problems.push((number, problem.to_owned()));
            open = None;
            field = Field::Broken;
            fuzzy = false;
            previous = None;
        }
    }
    close(&mut open, &mut entries);
    (entries, problems)
}

// ---------------------------------------------------------------- making files

/// One file of the corpus: its path relative to `game/` and its lines.
struct CorpusFile<'a> {
    relative: String,
    sheet: &'a str,
    part: Option<(usize, usize)>,
    lines: &'a [SheetLine],
}

/// The files of a sheet, in order.
fn sheet_files<'a>(sheet: &'a str, lines: &'a [SheetLine]) -> Vec<CorpusFile<'a>> {
    if is_scene(sheet) || lines.len() <= FILE_STRINGS {
        return vec![CorpusFile {
            relative: format!("{sheet}.po"),
            sheet,
            part: None,
            lines,
        }];
    }
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
}

/// How the previous file showed a string.
struct Before {
    source: String,
    fuzzy: bool,
    /// Its `#| msgid`.
    earlier: Option<String>,
}

fn previous_entries(path: &Path) -> HashMap<String, Before> {
    std::fs::read_to_string(path)
        .map(|text| {
            parse_po(&text)
                .0
                .into_iter()
                .map(|entry| {
                    (
                        entry.context,
                        Before {
                            source: entry.source,
                            fuzzy: entry.fuzzy,
                            earlier: entry.previous,
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn file_text(
    project: &Project,
    file: &CorpusFile<'_>,
    title: Option<&str>,
    previous: &HashMap<String, Before>,
    state: &mut State,
) -> String {
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
        let now = current(&project.session, ledger.as_ref(), &line.address);
        if let Some(now) = &now {
            if let Some(note) = &now.note {
                let _ = writeln!(text, "# note: {}", comment(note));
            }
            // The source changed since the file was made, or it had changed
            // before and the translation still waits for review.
            let before = previous.get(&address).and_then(|before| {
                if before.source != line.source {
                    Some(before.source.clone())
                } else if before.fuzzy {
                    before.earlier.clone()
                } else {
                    None
                }
            });
            let fuzzy = before.filter(|_| now.review == ReviewState::NeedsReview);
            let mut flags = Vec::new();
            if !now.replaceable() {
                flags.push("keep");
            }
            if fuzzy.is_some() {
                flags.push("fuzzy");
            }
            if !flags.is_empty() {
                let _ = writeln!(text, "#, {}", flags.join(", "));
            }
            if let Some(before) = &fuzzy {
                let _ = writeln!(text, "#| msgid {}", quote(before));
            }
            state.baseline.insert(address.clone(), now.target.clone());
            state.shown.insert(
                address.clone(),
                shown_digest(&now.target, now.author, now.review, now.note.as_deref()),
            );
        } else {
            state.baseline.remove(&address);
            state.shown.remove(&address);
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

/// Makes one file again; returns whether its text changed.
fn write_file(
    project: &Project,
    corpus: &Path,
    file: &CorpusFile<'_>,
    title: Option<&str>,
    state: &mut State,
) -> Result<bool, String> {
    let path = corpus.join(&file.relative);
    let previous = previous_entries(&path);
    let text = file_text(project, file, title, &previous, state);
    let changed = std::fs::read(&path).ok().as_deref() != Some(text.as_bytes());
    if changed {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        std::fs::write(&path, &text).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    if let Some(mut now) = stamp(&path, false) {
        now.game_version = project.session.source().version().to_string();
        state.files.insert(file.relative.clone(), now);
    }
    Ok(changed)
}

fn title_of(project: &Project, sheet: &str) -> Option<String> {
    sheet
        .starts_with("quest/")
        .then(|| quest_title(project, sheet))
        .flatten()
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

pub(crate) struct CorpusOptions {
    pub pattern: Option<String>,
    /// Replace files with changes a check has not saved.
    pub force: bool,
}

/// What making the corpus did.
struct Made {
    sheets: usize,
    strings: usize,
    written: usize,
    removed: usize,
    kept: Vec<String>,
}

/// Makes the corpus, or the files of the matching sheets. Making all of it
/// also removes files no sheet has any more.
fn make(project: &Project, options: &CorpusOptions) -> Result<Made, String> {
    let root = project.root().to_owned();
    let corpus = root.join(CORPUS_DIR);
    std::fs::create_dir_all(&corpus).map_err(|error| format!("{}: {error}", corpus.display()))?;
    ignore_corpus(&root)?;
    let mut state = load_state(&corpus);
    let sheets: Vec<String> = sheet_counts(project)
        .into_iter()
        .map(|sheet| sheet.name)
        .filter(|name| {
            options
                .pattern
                .as_deref()
                .is_none_or(|pattern| matches_pattern(name, pattern))
        })
        .collect();
    let mut made = Made {
        sheets: sheets.len(),
        strings: 0,
        written: 0,
        removed: 0,
        kept: Vec::new(),
    };
    let mut current_files: BTreeSet<String> = BTreeSet::new();
    for sheet in &sheets {
        let lines = read_sheet_lines(project, sheet)?;
        made.strings += lines.len();
        let title = title_of(project, sheet);
        for file in sheet_files(sheet, &lines) {
            current_files.insert(file.relative.clone());
            if !options.force && unsaved(&corpus, &state, &file.relative) {
                made.kept.push(file.relative.clone());
                continue;
            }
            if write_file(project, &corpus, &file, title.as_deref(), &mut state)? {
                made.written += 1;
            }
        }
    }
    if options.pattern.is_none() {
        let gone: Vec<String> = state
            .files
            .keys()
            .filter(|relative| !current_files.contains(*relative))
            .cloned()
            .collect();
        for relative in gone {
            if !options.force && unsaved(&corpus, &state, &relative) {
                made.kept.push(relative);
                continue;
            }
            let path = corpus.join(&relative);
            for address in previous_entries(&path).into_keys() {
                state.baseline.remove(&address);
                state.shown.remove(&address);
            }
            let _ = std::fs::remove_file(&path);
            if let Some(parent) = path.parent()
                && parent != corpus
            {
                let _ = std::fs::remove_dir(parent);
            }
            state.files.remove(&relative);
            made.removed += 1;
        }
        state.game_version = project.session.source().version().to_string();
    } else if state.game_version.is_empty() {
        state.game_version = project.session.source().version().to_string();
    }
    std::fs::write(corpus.join(README), readme(project))
        .map_err(|error| format!("{}: {error}", corpus.join(README).display()))?;
    save_state(&corpus, &state)?;
    Ok(made)
}

/// Writes the corpus, or the files of the matching sheets.
pub(crate) fn corpus(
    project: &Project,
    options: &CorpusOptions,
    out: &mut Output,
) -> Result<(), String> {
    let made = make(project, options)?;
    if out.json {
        out.json_value(&json!({
            "folder": format!("{}/{CORPUS_DIR}", project.root_display()),
            "sheets": made.sheets,
            "strings": made.strings,
            "filesWritten": made.written,
            "filesRemoved": made.removed,
            "keptUnsaved": made.kept,
        }));
        return Ok(());
    }
    let _ = writeln!(
        out.text,
        "{}/{CORPUS_DIR}: {} strings of {} sheets · {} files written{}",
        project.root_display(),
        fmt_count(made.strings),
        fmt_count(made.sheets),
        fmt_count(made.written),
        if made.removed > 0 {
            format!(
                " · {} files of sheets the game no longer has removed",
                made.removed
            )
        } else {
            String::new()
        }
    );
    if !made.kept.is_empty() {
        let _ = writeln!(
            out.text,
            "{} files kept because they have changes `aeria check` has not saved; check them, or pass --force to replace them: {}",
            made.kept.len(),
            made.kept.join(", ")
        );
    }
    Ok(())
}

/// The sheets whose files no longer show the project, with the strings
/// that changed: a translation, its author, review state, or note changed,
/// or the file was made for another game version.
fn outdated_sheets(
    project: &Project,
    state: &State,
    version: &str,
) -> BTreeMap<String, BTreeSet<String>> {
    let agents = project
        .ledger()
        .and_then(|ledger| ledger.all().ok())
        .unwrap_or_default();
    let mut outdated: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for unit in project.session.workspace().units() {
        if !unit.is_bound() {
            continue;
        }
        let binding = unit.source_binding();
        let address = format!(
            "{}:{}:{}:{}",
            binding.sheet_name(),
            binding.row_id(),
            binding.subrow_id(),
            binding.column_index()
        );
        let author = if agents.get(&address).map(String::as_str) == Some(unit.target_macro()) {
            Author::Agent
        } else {
            Author::Person
        };
        let shown = shown_digest(
            unit.target_macro(),
            author,
            unit.review_state(),
            unit.translator_note(),
        );
        if state.shown.get(&address) != Some(&shown) {
            outdated
                .entry(binding.sheet_name().to_owned())
                .or_default()
                .insert(address.clone());
        }
        seen.insert(address);
    }
    // Translations the project no longer has.
    for address in state
        .shown
        .keys()
        .filter(|address| !seen.contains(*address))
    {
        if let Ok(parsed) = Address::parse(address) {
            outdated
                .entry(parsed.sheet)
                .or_default()
                .insert(address.clone());
        }
    }
    // Files kept through a game update because of unsaved changes.
    for (relative, recorded) in &state.files {
        if recorded.game_version != version
            && let Some(sheet) = relative.strip_suffix(".po").map(|name| {
                name.rsplit_once('/')
                    .filter(|(_, part)| part.chars().all(|c| c.is_ascii_digit()))
                    .map_or(name, |(sheet, _)| sheet)
            })
        {
            outdated.entry(sheet.to_owned()).or_default();
        }
    }
    outdated
}

/// Brings `game/` up to date with the project and the game: after a game
/// update every file is made again; otherwise the files of the strings whose
/// translation, author, review state, or note changed since their file was
/// made. Files with changes not saved yet are left alone. Returns how many
/// files were written.
pub(crate) fn refresh(project: &Project) -> Result<usize, String> {
    let corpus = project.root().join(CORPUS_DIR);
    if !corpus.join(STATE_FILE).is_file() {
        return Ok(0);
    }
    let mut state = load_state(&corpus);
    if state.game_version != project.session.source().version().to_string() {
        let made = make(
            project,
            &CorpusOptions {
                pattern: None,
                force: false,
            },
        )?;
        return Ok(made.written + made.removed);
    }
    let version = project.session.source().version().to_string();
    let outdated = outdated_sheets(project, &state, &version);
    let mut written = 0;
    for (sheet, addresses) in outdated {
        let Ok(lines) = sheet_lines(project, &sheet) else {
            continue;
        };
        let files = sheet_files(&sheet, &lines);
        let wanted: BTreeSet<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| addresses.contains(&line.address.to_string()))
            .map(|(position, _)| {
                if files.len() == 1 {
                    0
                } else {
                    position / FILE_STRINGS
                }
            })
            .chain(files.iter().enumerate().filter_map(|(index, file)| {
                state
                    .files
                    .get(&file.relative)
                    .is_some_and(|recorded| recorded.game_version != version)
                    .then_some(index)
            }))
            .collect();
        let title = title_of(project, &sheet);
        for index in wanted {
            let file = &files[index];
            // Only files the corpus has: `aeria corpus` decides what it holds.
            if !state.files.contains_key(&file.relative) || unsaved(&corpus, &state, &file.relative)
            {
                continue;
            }
            if write_file(project, &corpus, file, title.as_deref(), &mut state)? {
                written += 1;
            }
        }
    }
    save_state(&corpus, &state)?;
    Ok(written)
}

fn readme(project: &Project) -> String {
    let mut text = format!(
        "# The text of FINAL FANTASY XIV\n\n\
Every translatable string of the game, version {}, made by `aeria corpus` from the \
installed game and this project. It is never committed. It always shows the project: \
translations made in Aeria or elsewhere appear in the files by the next `aeria check` (run one before you start), \
and after a game update every file is made again.\n\n\
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
- `#, fuzzy` marks a translation whose source a game update changed; `#| msgid` shows the \
source it was written for. Bring the translation in line with the new source. The mark \
goes once the translation changes or a person reviews it.\n\
- Search the whole game with any tool, for example to see how a name is used \
everywhere or how the official localizations handled a macro.\n\n\
## Translating\n\n\
Write the translation into `msgstr` as a PO string: `\\\"` for a quote, `\\\\` for a \
backslash, one line (the game breaks lines with `<br>`). Then run `aeria check`: it saves \
every changed `msgstr` that passes the checks, lists each problem as \
`file:line: what to fix`, and brings the files up to date with the project. A translation \
that was not saved stays in its file until it is fixed. Leave `msgctxt` and `msgid` as \
they are. Terms, style, character voices, and the story so far are in \
`../aeria-knowledge/`.\n\n",
        project.session.source().version(),
        project.source_language(),
        project
            .target_language()
            .unwrap_or_else(|| "target".to_owned()),
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

/// Finds the translations changed in the files that changed since Aeria
/// last read them, or that had problems left.
fn find_changes(
    project: &Project,
    corpus: &Path,
    state: &mut State,
    problems: &mut Vec<Problem>,
) -> Result<(Vec<Change>, BTreeSet<String>), String> {
    let ledger = project.ledger();
    let mut changes = Vec::new();
    let mut read_files = BTreeSet::new();
    for (relative, path) in corpus_files(corpus) {
        let recorded = state.files.get(&relative);
        let to_read = match (recorded, stamp(&path, false)) {
            (Some(recorded), Some(now)) => {
                recorded.pending || recorded.len != now.len || recorded.modified != now.modified
            }
            _ => true,
        };
        if !to_read {
            continue;
        }
        read_files.insert(relative.clone());
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let (entries, broken) = parse_po(&text);
        let mut problem = |line: usize, message: String| {
            problems.push(Problem {
                file: relative.clone(),
                line,
                message,
            });
        };
        for (line, message) in broken {
            problem(line, message);
        }
        for entry in entries {
            let Ok(address) = Address::parse(&entry.context) else {
                problem(
                    entry.line,
                    format!(
                        "msgctxt {:?} is not an address; leave msgctxt as it is",
                        entry.context
                    ),
                );
                continue;
            };
            let key = address.to_string();
            let made_with = state.baseline.get(&key).map_or("", String::as_str);
            if entry.translation == made_with {
                continue;
            }
            let project_now: Option<Current> = current(&project.session, ledger.as_ref(), &address);
            let target_now = project_now.as_ref().map(|now| now.target.as_str());
            if target_now == Some(entry.translation.as_str()) {
                state.baseline.insert(key, entry.translation);
                continue;
            }
            match source_of(project, &address) {
                Ok(source) if source == entry.source => {}
                Ok(_) => {
                    problem(
                        entry.line,
                        "the game changed this string after this file was made: undo your change to this entry and run `aeria check`; the file is then made again for the new text".to_owned(),
                    );
                    continue;
                }
                Err(reason) => {
                    problem(entry.line, reason);
                    continue;
                }
            }
            if target_now.unwrap_or("") != made_with {
                problem(
                    entry.line,
                    format!(
                        "the translation changed in the project after this file was made (now {}); undo your change here, run `aeria check` to see the new translation, then change it again",
                        target_now.map_or_else(|| "none".to_owned(), quote)
                    ),
                );
                continue;
            }
            if entry.translation.trim().is_empty() {
                problem(
                    entry.line,
                    "a translation cannot be removed here; put it back or remove it in Aeria"
                        .to_owned(),
                );
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
    Ok((changes, read_files))
}

/// Saves the translations changed in the corpus and brings the files up to
/// date; returns whether nothing is left to fix.
#[allow(clippy::too_many_lines)] // one check
pub(crate) fn check_files(project: &mut Project, out: &mut Output) -> Result<bool, String> {
    let corpus = project.root().join(CORPUS_DIR);
    if !corpus.is_dir() {
        return Err(format!(
            "give translations with `-` on standard input, in a file, or with --at; or make the game's text with `aeria corpus` and check {CORPUS_DIR}/"
        ));
    }
    let mut state = load_state(&corpus);
    let mut problems: Vec<Problem> = Vec::new();
    let (changes, read_files) = find_changes(project, &corpus, &mut state, &mut problems)?;
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
        write::apply(
            project,
            &entries,
            &WriteOptions {
                needs_review: false,
            },
            out,
        )?
    };
    let outcomes: BTreeMap<String, Outcome> = outcomes.into_iter().collect();
    let mut advice: Vec<String> = Vec::new();
    for change in &changes {
        let key = change.address.to_string();
        let message = match outcomes.get(&key) {
            Some(Outcome::Ok { advice: notes }) => {
                state.baseline.insert(key, change.text.clone());
                if !notes.is_empty() {
                    advice.push(format!(
                        "{CORPUS_DIR}/{}:{}: saved; advice: {}",
                        change.relative,
                        change.line,
                        notes.join("; ")
                    ));
                }
                continue;
            }
            Some(Outcome::Unchanged) => {
                state.baseline.insert(key, change.text.clone());
                continue;
            }
            Some(Outcome::Rejected { reasons }) => format!("rejected: {}", reasons.join("; ")),
            Some(Outcome::Skipped { reason }) => format!("not saved: {reason}"),
            Some(Outcome::Failed { reason }) => {
                format!("not saved ({reason}); run `aeria check` again")
            }
            None => "not saved".to_owned(),
        };
        problems.push(Problem {
            file: change.relative.clone(),
            line: change.line,
            message,
        });
    }
    problems.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    let pending: BTreeSet<&str> = problems
        .iter()
        .map(|problem| problem.file.as_str())
        .collect();
    for relative in &read_files {
        if let Some(mut now) = stamp(&corpus.join(relative), pending.contains(relative.as_str())) {
            now.game_version = state
                .files
                .get(relative)
                .map(|recorded| recorded.game_version.clone())
                .unwrap_or_default();
            state.files.insert(relative.clone(), now);
        }
    }
    save_state(&corpus, &state)?;
    let updated = refresh(project)?;
    if out.json {
        out.json_value(&json!({
            "saved": written,
            "problems": problems,
            "advice": advice,
            "filesUpdated": updated,
        }));
        return Ok(problems.is_empty());
    }
    for line in &advice {
        let _ = writeln!(out.text, "{line}");
    }
    for problem in &problems {
        let _ = writeln!(
            out.text,
            "{CORPUS_DIR}/{}:{}: {}",
            problem.file, problem.line, problem.message
        );
    }
    let _ = write!(
        out.text,
        "saved: {} · problems: {}",
        fmt_count(written),
        fmt_count(problems.len())
    );
    if updated > 0 {
        let _ = write!(
            out.text,
            " · files brought up to date with the project: {}",
            fmt_count(updated)
        );
    }
    out.text.push('\n');
    if !problems.is_empty() {
        out.text
            .push_str("A translation that was not saved stays in its file until it is fixed.\n");
    }
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
    fn entries_are_read_with_continued_strings_flags_and_their_lines() {
        let text = "msgid \"\"\nmsgstr \"\"\n\"Language: ru\\n\"\n\n#. ja: 名前\n#, keep, fuzzy\n#| msgid \"Old \"\n#| \"name\"\nmsgctxt \"BNpcName:1:0:0\"\nmsgid \"Name\"\nmsgstr \"Имя\"\n\nmsgctxt \"BNpcName:2:0:0\"\nmsgid \"Long\"\n\"er\"\nmsgstr \"\"\n\"Длин\"\n\"ное\"\n";
        assert_eq!(
            parse_po(text).0,
            [
                ParsedEntry {
                    context: "BNpcName:1:0:0".to_owned(),
                    source: "Name".to_owned(),
                    translation: "Имя".to_owned(),
                    line: 11,
                    fuzzy: true,
                    previous: Some("Old name".to_owned()),
                },
                ParsedEntry {
                    context: "BNpcName:2:0:0".to_owned(),
                    source: "Longer".to_owned(),
                    translation: "Длинное".to_owned(),
                    line: 16,
                    fuzzy: false,
                    previous: None,
                },
            ]
        );
    }

    #[test]
    fn a_broken_entry_leaves_the_others_readable() {
        let (entries, problems) = parse_po(
            "msgctxt \"a:1:0:0\"\nmsgid \"x\"\nmsgstr Имя\n\nmsgctxt \"a:2:0:0\"\nmsgid \"y\"\nmsgstr \"Да\"\n",
        );
        assert_eq!(
            problems.iter().map(|(line, _)| *line).collect::<Vec<_>>(),
            [3]
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].translation, "Да");
    }

    #[test]
    fn the_digest_is_stable() {
        assert_eq!(digest(&["a", "b"]), digest(&["a", "b"]));
        assert_ne!(digest(&["ab", ""]), digest(&["a", "b"]));
        assert_eq!(digest(&[]), 0xcbf2_9ce4_8422_2325);
    }
}
