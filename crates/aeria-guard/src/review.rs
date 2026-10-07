//! A change summarized for the person who merges it: the strings whose
//! translation, note, or marks changed, file by file, and the changed terms
//! and style, instead of the lines of the PO files.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use aeria_git::{EntryChange, EntryChangeKind, diff_file};
use aeria_knowledge::{GlossaryEntry, KnowledgeFile, parse_glossary};

use crate::{Area, area, changed_files, file_at, git};

/// Strings listed per file before the rest are only counted.
const ROWS_PER_FILE: usize = 100;
/// Terms listed before the rest are only counted.
const TERM_ROWS: usize = 200;
/// The text of one cell, in characters, before it is cut.
const CELL_CHARS: usize = 300;
/// The summary stops listing strings beyond this size; GitHub takes at most
/// 1 MiB of summary per step.
const MAX_BYTES: usize = 800_000;

/// Markdown that summarizes the change from `base` to the project at
/// `project_root`; `None` when `base` cannot be read.
#[must_use]
pub fn review(project_root: &Path, base: &str) -> Option<String> {
    git(
        project_root,
        &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
    )?;
    let files = changed_files(project_root, base);
    let mut strings: Vec<(String, Vec<EntryChange>)> = Vec::new();
    for file in files.iter().filter(|file| area(file) == Area::Translations) {
        let path = file.project_path.clone().unwrap_or_default();
        let before = file_at(project_root, base, &path);
        let after = fs::read(project_root.join(&path)).ok();
        let changes = diff_file(&path, before.as_deref(), after.as_deref());
        if !changes.is_empty() {
            strings.push((path, changes));
        }
    }
    let terms = KnowledgeFile::Terms.relative_path();
    let style = KnowledgeFile::Style.relative_path();
    let touched = |relative: &str| {
        files
            .iter()
            .any(|file| file.project_path.as_deref() == Some(relative))
    };
    let term_changes = touched(&terms).then(|| {
        term_changes(
            file_at(project_root, base, &terms).as_deref(),
            fs::read(project_root.join(&terms)).ok().as_deref(),
        )
    });
    let style_change = touched(&style).then(|| {
        line_counts(
            file_at(project_root, base, &style).as_deref(),
            fs::read(project_root.join(&style)).ok().as_deref(),
        )
    });
    let others: Vec<&str> = files
        .iter()
        .filter(|file| !matches!(area(file), Area::Translations | Area::Knowledge))
        .map(|file| file.path.as_str())
        .collect();

    let mut text = String::from("## Review\n\n");
    let total: usize = strings.iter().map(|(_, changes)| changes.len()).sum();
    if total == 0 && term_changes.is_none() && style_change.is_none() && others.is_empty() {
        text.push_str("Nothing changed against the base.\n");
        return Some(text);
    }
    if total > 0 {
        let _ = writeln!(
            text,
            "{total} string(s) changed in {} file(s): {}.\n",
            strings.len(),
            counts(strings.iter().flat_map(|(_, changes)| changes))
        );
    }
    write_strings(&mut text, &strings);
    write_terms(&mut text, &terms, term_changes);
    if let Some((added, removed)) = style_change {
        let _ = writeln!(
            text,
            "### Style\n\n`{style}` changed: {added} line(s) added, {removed} removed.\n"
        );
    }
    if !others.is_empty() {
        text.push_str("### Other files\n\n");
        for path in others {
            let _ = writeln!(text, "- `{}`", path.replace('`', "'"));
        }
        text.push('\n');
    }
    Some(text)
}

/// The changed strings, file by file, each file folded.
fn write_strings(text: &mut String, strings: &[(String, Vec<EntryChange>)]) {
    for (path, changes) in strings {
        let _ = writeln!(
            text,
            "<details><summary><code>{}</code>: {}</summary>\n\n| String | Change | Source | Before | After |\n|---|---|---|---|---|",
            html(path),
            counts(changes.iter())
        );
        for change in changes.iter().take(ROWS_PER_FILE) {
            if text.len() > MAX_BYTES {
                break;
            }
            let _ = writeln!(
                text,
                "| `{}` | {} | {} | {} | {} |",
                change.context.replace('`', "'"),
                change_label(change),
                cell(&change.source),
                cell(&state_text(&change.before)),
                cell(&state_text(&change.after)),
            );
        }
        if changes.len() > ROWS_PER_FILE {
            let _ = writeln!(text, "\n…and {} more.", changes.len() - ROWS_PER_FILE);
        }
        text.push_str("\n</details>\n\n");
    }
    if text.len() > MAX_BYTES {
        text.push_str("The list was cut to stay within GitHub's summary size.\n\n");
    }
}

/// The changed terms, or why the terms file does not read.
fn write_terms(text: &mut String, terms: &str, term_changes: Option<Result<TermChanges, String>>) {
    match term_changes {
        Some(Err(message)) => {
            let _ = writeln!(
                text,
                "### Terms\n\n`{terms}` does not read: {}\n",
                html(&message)
            );
        }
        Some(Ok((rows, base_unreadable))) if !rows.is_empty() => {
            let _ = writeln!(text, "### Terms\n\n{} term(s) changed.\n", rows.len());
            if base_unreadable {
                text.push_str(
                    "The base's terms file does not read in this format, so every term shows as added.\n\n",
                );
            }
            text.push_str("| Term | Change | Before | After |\n|---|---|---|---|\n");
            for (term, label, before, after) in rows.iter().take(TERM_ROWS) {
                let _ = writeln!(
                    text,
                    "| {} | {label} | {} | {} |",
                    cell(term),
                    cell(before),
                    cell(after)
                );
            }
            if rows.len() > TERM_ROWS {
                let _ = writeln!(text, "\n…and {} more.", rows.len() - TERM_ROWS);
            }
            text.push('\n');
        }
        _ => {}
    }
}

fn counts<'a>(changes: impl Iterator<Item = &'a EntryChange>) -> String {
    let mut by_kind: BTreeMap<EntryChangeKind, usize> = BTreeMap::new();
    for change in changes {
        *by_kind.entry(change.kind).or_default() += 1;
    }
    by_kind
        .into_iter()
        .map(|(kind, count)| format!("{count} {}", kind_label(kind)))
        .collect::<Vec<_>>()
        .join(", ")
}

const fn kind_label(kind: EntryChangeKind) -> &'static str {
    match kind {
        EntryChangeKind::Translated => "translated",
        EntryChangeKind::Changed => "changed",
        EntryChangeKind::Cleared => "removed",
        EntryChangeKind::Marked => "marked",
    }
}

/// What changed about one string, beyond its kind for a mark.
fn change_label(change: &EntryChange) -> String {
    if change.kind != EntryChangeKind::Marked {
        return kind_label(change.kind).to_owned();
    }
    let mut parts = Vec::new();
    if change.before.note != change.after.note {
        parts.push("note");
    }
    if change.before.fuzzy != change.after.fuzzy {
        parts.push(if change.after.fuzzy {
            "fuzzy"
        } else {
            "not fuzzy"
        });
    }
    if change.before.reviewed != change.after.reviewed {
        parts.push(if change.after.reviewed {
            "reviewed"
        } else {
            "not reviewed"
        });
    }
    parts.join(", ")
}

fn state_text(state: &aeria_git::EntryState) -> String {
    match &state.note {
        Some(note) if !note.is_empty() => format!("{}\n[note: {note}]", state.translation),
        _ => state.translation.clone(),
    }
}

/// Escapes text for HTML.
fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Text for a Markdown table cell: escaped, on one line, and cut.
fn cell(text: &str) -> String {
    if text.is_empty() {
        return "—".to_owned();
    }
    let mut cut: String = text.chars().take(CELL_CHARS).collect();
    if cut.len() < text.len() {
        cut.push('…');
    }
    html(&cut)
        .replace('|', "\\|")
        .replace("\r\n", "\n")
        .replace('\n', "<br>")
}

/// Terms added, removed, or changed, by headword: term, change, before,
/// after.
type TermRow = (String, &'static str, String, String);

fn term_text(entry: &GlossaryEntry) -> String {
    let mut text = entry.translation.clone();
    if !entry.forms.is_empty() {
        let _ = write!(text, "\nforms: {}", entry.forms.join(", "));
    }
    if let Some(note) = entry.note.as_deref().filter(|note| !note.is_empty()) {
        let _ = write!(text, "\nnote: {note}");
    }
    if !entry.folder.is_empty() {
        let _ = write!(text, "\nfolder: {}", entry.folder);
    }
    if entry.match_case {
        text.push_str("\nmatches case");
    }
    text
}

/// The changed terms, and whether the base's terms file did not read, so
/// that every term shows as added.
type TermChanges = (Vec<TermRow>, bool);

fn term_changes(before: Option<&[u8]>, after: Option<&[u8]>) -> Result<TermChanges, String> {
    let read = |bytes: Option<&[u8]>| -> Result<BTreeMap<String, GlossaryEntry>, String> {
        let Some(bytes) = bytes else {
            return Ok(BTreeMap::new());
        };
        let glossary = parse_glossary(bytes).map_err(|error| error.to_string())?;
        Ok(glossary
            .entries
            .into_iter()
            .map(|entry| (entry.term.to_lowercase(), entry))
            .collect())
    };
    let old = read(before);
    let base_unreadable = old.is_err();
    let old = old.unwrap_or_default();
    let new = read(after)?;
    let mut rows = Vec::new();
    for (key, entry) in &new {
        match old.get(key) {
            None => rows.push((entry.term.clone(), "added", String::new(), term_text(entry))),
            Some(previous) if previous != entry => rows.push((
                entry.term.clone(),
                "changed",
                term_text(previous),
                term_text(entry),
            )),
            Some(_) => {}
        }
    }
    for (key, entry) in &old {
        if !new.contains_key(key) {
            rows.push((
                entry.term.clone(),
                "removed",
                term_text(entry),
                String::new(),
            ));
        }
    }
    rows.sort_by_key(|row| row.0.to_lowercase());
    Ok((rows, base_unreadable))
}

/// Lines the later text has that the earlier lacks, and the other way.
fn line_counts(before: Option<&[u8]>, after: Option<&[u8]>) -> (usize, usize) {
    let lines = |bytes: Option<&[u8]>| -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for line in String::from_utf8_lossy(bytes.unwrap_or_default()).lines() {
            *counts.entry(line.to_owned()).or_default() += 1;
        }
        counts
    };
    let old = lines(before);
    let new = lines(after);
    let more = |a: &BTreeMap<String, usize>, b: &BTreeMap<String, usize>| {
        a.iter()
            .map(|(line, count)| count.saturating_sub(b.get(line).copied().unwrap_or(0)))
            .sum()
    };
    (more(&new, &old), more(&old, &new))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_are_escaped_and_cut() {
        assert_eq!(cell(""), "—");
        assert_eq!(cell("a|b\n<If>"), "a\\|b<br>&lt;If&gt;");
        let long = "я".repeat(CELL_CHARS + 5);
        assert!(cell(&long).ends_with('…'));
    }

    #[test]
    fn terms_are_compared_by_headword() {
        let before = b"term,translation\nLinkshell,linkshell\nEorzea,Eorzea\n";
        let after =
            b"term,translation,forms\nlinkshell,linkshell,linkshells\nAetheryte,aetheryte,\n";
        let (rows, _) = term_changes(Some(before), Some(after)).expect("terms");
        let summary: Vec<_> = rows
            .iter()
            .map(|(term, label, ..)| format!("{term} {label}"))
            .collect();
        assert_eq!(
            summary,
            ["Aetheryte added", "Eorzea removed", "linkshell changed"]
        );
    }

    #[test]
    fn style_lines_are_counted() {
        assert_eq!(line_counts(Some(b"a\nb\n"), Some(b"a\nc\nd\n")), (2, 1));
        assert_eq!(line_counts(None, Some(b"a\n")), (1, 0));
    }
}
