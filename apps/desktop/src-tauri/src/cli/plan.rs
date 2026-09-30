//! `plan`: the untranslated strings of a scope, split into tasks that
//! agents can translate at the same time without overlapping.
//!
//! A quest or cutscene is one scene and is never split. Other sheets are
//! cut into chunks of at most `size` untranslated strings, addressed by
//! position in the sheet's whole listing (`read --from`), which writes do
//! not shift. Small sheets and scenes are packed together up to `size`.
//! Everything here is deterministic.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;
use serde_json::json;

use super::project::{Project, matches_pattern, sheet_lines, translated};
use super::read::sheet_counts;
use super::{Output, fmt_count};

pub(crate) struct PlanOptions {
    pub pattern: String,
    /// Untranslated strings per task at most, except a scene larger than it.
    pub size: usize,
    /// Zero-based index of the first task shown.
    pub from: usize,
    /// Tasks shown at most.
    pub limit: usize,
    pub terms: bool,
}

/// One task: the reads that list its strings.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Task {
    strings: usize,
    commands: Vec<String>,
}

/// A frequent word of the scope that no term of the project covers.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Frequent {
    word: String,
    strings: usize,
    example: String,
}

fn is_scene(sheet: &str) -> bool {
    sheet.starts_with("quest/") || sheet.starts_with("cut_scene/")
}

/// Quotes a sheet name for a shell when it needs it.
fn shell_sheet(sheet: &str) -> String {
    if sheet
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
    {
        sheet.to_owned()
    } else {
        format!("\"{sheet}\"")
    }
}

/// Splits the scope into tasks. `positions` gives the listing positions of
/// a sheet's untranslated strings; it is asked only for sheets to cut.
fn split(
    sheets: &[(String, usize)],
    size: usize,
    mut positions: impl FnMut(&str) -> Result<Vec<usize>, String>,
) -> Result<Vec<Task>, String> {
    let mut tasks = Vec::new();
    let mut packed = Task {
        strings: 0,
        commands: Vec::new(),
    };
    for (sheet, untranslated) in sheets {
        let name = shell_sheet(sheet);
        let whole = format!("aeria read {name} --untranslated");
        if *untranslated <= size {
            if packed.strings + untranslated > size {
                tasks.push(std::mem::replace(
                    &mut packed,
                    Task {
                        strings: 0,
                        commands: Vec::new(),
                    },
                ));
            }
            packed.strings += untranslated;
            packed.commands.push(whole);
        } else if is_scene(sheet) {
            // A scene larger than a task is a task of its own, whole.
            tasks.push(Task {
                strings: *untranslated,
                commands: vec![whole],
            });
        } else {
            for chunk in positions(sheet)?.chunks(size) {
                tasks.push(Task {
                    strings: chunk.len(),
                    commands: vec![format!(
                        "{whole} --from {} --limit {}",
                        chunk[0] + 1,
                        chunk.len()
                    )],
                });
            }
        }
    }
    if packed.strings > 0 {
        tasks.push(packed);
    }
    Ok(tasks)
}

fn tasks(project: &Project, sheets: &[(String, usize)], size: usize) -> Result<Vec<Task>, String> {
    split(sheets, size, |sheet| {
        Ok(sheet_lines(project, sheet)?
            .iter()
            .enumerate()
            .filter(|(_, line)| !translated(&project.session, &line.address))
            .map(|(position, _)| position)
            .collect())
    })
}

/// Common English words that are never terms.
const COMMON: &[&str] = &[
    "the", "and", "for", "you", "your", "with", "that", "this", "from", "are", "was", "were",
    "have", "has", "had", "not", "but", "all", "any", "can", "will", "would", "shall", "should",
    "may", "might", "must", "into", "onto", "out", "off", "over", "under", "about", "after",
    "before", "been", "being", "who", "whom", "what", "when", "where", "which", "why", "how",
    "its", "his", "her", "him", "she", "they", "them", "their", "our", "ours", "one", "two",
    "more", "most", "some", "such", "than", "then", "there", "these", "those", "very", "just",
    "also", "only", "each", "other", "per", "via", "upon", "while", "yet",
];

/// Words that recur in the scope's untranslated strings and that no term of
/// the project covers: what to settle before agents translate in parallel.
fn frequent_words(project: &Project, sheets: &[(String, usize)]) -> Result<Vec<Frequent>, String> {
    let knowledge = project.knowledge();
    // word (lowercase) -> (strings, most frequent spelling counts, example)
    let mut words: BTreeMap<String, (usize, BTreeMap<String, usize>, String)> = BTreeMap::new();
    for (sheet, _) in sheets {
        for line in sheet_lines(project, sheet)?.iter() {
            if translated(&project.session, &line.address) {
                continue;
            }
            let plain = aeria_search::plain_text(&line.source);
            let mut seen = std::collections::HashSet::new();
            for token in plain.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’')) {
                let token = token.trim_matches(|c| c == '\'' || c == '’');
                if token.chars().count() < 3 || !token.chars().any(char::is_alphabetic) {
                    continue;
                }
                let key = token.to_lowercase();
                if COMMON.contains(&key.as_str()) || !seen.insert(key.clone()) {
                    continue;
                }
                let entry = words
                    .entry(key)
                    .or_insert_with(|| (0, BTreeMap::new(), plain.clone()));
                entry.0 += 1;
                *entry.1.entry(token.to_owned()).or_default() += 1;
            }
        }
    }
    let mut frequent: Vec<Frequent> = words
        .into_values()
        .filter(|(strings, _, _)| *strings >= 3)
        .filter_map(|(strings, spellings, example)| {
            let word = spellings
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                .map(|(spelling, _)| spelling)?;
            let covered = knowledge
                .terms
                .entries
                .iter()
                .any(|entry| aeria_knowledge::contains_term(&entry.term, &word));
            (!covered).then_some(Frequent {
                word,
                strings,
                example,
            })
        })
        .collect();
    frequent.sort_by(|a, b| b.strings.cmp(&a.strings).then_with(|| a.word.cmp(&b.word)));
    frequent.truncate(TERMS_SHOWN);
    Ok(frequent)
}

/// Frequent words shown at most.
const TERMS_SHOWN: usize = 60;

const WORKER: &str = "\
Give each task to one agent with these instructions: run `aeria brief` and follow it; \
run the task's commands and translate every string they list (when an output ends with \
a command that continues it, run that too); write with `aeria write`, about 50 strings \
per write, the blocks in a file or on standard input, and fix what comes back rejected. \
Translate only the task's strings: those past its --limit belong to other tasks. Reply \
with one line: written, rejected, skipped. Tasks do not overlap, so any number can run \
at once; `aeria overview <pattern>` shows progress, and `aeria plan` again lists what is \
left.";

pub(crate) fn plan(project: &Project, options: &PlanOptions, out: &mut Output) -> Result<(), String> {
    let sheets: Vec<(String, usize)> = sheet_counts(project)
        .into_iter()
        .filter(|sheet| matches_pattern(&sheet.name, &options.pattern))
        .map(|sheet| {
            let untranslated = sheet.strings.saturating_sub(sheet.translated);
            (sheet.name, untranslated)
        })
        .filter(|(_, untranslated)| *untranslated > 0)
        .collect();
    let all = tasks(project, &sheets, options.size)?;
    let words = if options.terms {
        Some(frequent_words(project, &sheets)?)
    } else {
        None
    };
    let untranslated: usize = all.iter().map(|task| task.strings).sum();
    let shown: Vec<(usize, &Task)> = all
        .iter()
        .enumerate()
        .skip(options.from)
        .take(options.limit)
        .collect();
    let next = (options.from + shown.len() < all.len()).then(|| options.from + shown.len() + 1);
    if out.json {
        out.json_value(&json!({
            "pattern": options.pattern,
            "sheets": sheets.len(),
            "untranslated": untranslated,
            "taskCount": all.len(),
            "tasks": shown.iter().map(|(index, task)| json!({
                "task": index + 1,
                "strings": task.strings,
                "commands": task.commands,
            })).collect::<Vec<_>>(),
            "nextTask": next,
            "words": words,
            "instructions": WORKER,
        }));
        return Ok(());
    }
    let _ = writeln!(
        out.text,
        "{}: {} untranslated strings in {} sheets · {} tasks of at most {} strings (a scene is never split)",
        options.pattern,
        fmt_count(untranslated),
        fmt_count(sheets.len()),
        fmt_count(all.len()),
        options.size
    );
    if let Some(words) = &words {
        if words.is_empty() {
            out.text
                .push_str("\nNo frequent word of these strings lacks a term.\n");
        } else {
            out.text.push_str(
                "\n## Frequent words without a term\nSettle these in aeria-knowledge/terms.csv before the tasks run, so parallel agents render them the same way.\n",
            );
            for word in words {
                let _ = writeln!(
                    out.text,
                    "{} · {} strings · e.g. {}",
                    word.word, word.strings, word.example
                );
            }
        }
    }
    let _ = writeln!(out.text, "\n## Tasks\n{WORKER}\n");
    for (index, task) in &shown {
        let _ = writeln!(out.text, "T{} · {} strings", index + 1, task.strings);
        for command in &task.commands {
            let _ = writeln!(out.text, "  {command}");
        }
    }
    if let Some(next) = next {
        let _ = writeln!(
            out.text,
            "\n… {} more tasks: `aeria plan {} --size {} --from {next}`",
            all.len() - next + 1,
            shell_sheet(&options.pattern),
            options.size
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_stay_whole_small_sheets_pack_and_large_sheets_cut() {
        let sheets = [
            ("Race".to_owned(), 3),
            ("quest/000/Big_00001".to_owned(), 120),
            ("Tribe".to_owned(), 3),
            ("BNpcName".to_owned(), 7),
            ("quest/000/Small_00002".to_owned(), 2),
        ];
        let tasks = split(&sheets, 6, |sheet| {
            assert_eq!(sheet, "BNpcName");
            Ok(vec![0, 4, 5, 9, 10, 11, 30])
        })
        .expect("split");
        let listed: Vec<(usize, Vec<&str>)> = tasks
            .iter()
            .map(|task| (task.strings, task.commands.iter().map(String::as_str).collect()))
            .collect();
        assert_eq!(
            listed,
            [
                (120, vec!["aeria read quest/000/Big_00001 --untranslated"]),
                (6, vec!["aeria read BNpcName --untranslated --from 1 --limit 6"]),
                (1, vec!["aeria read BNpcName --untranslated --from 31 --limit 1"]),
                (
                    6,
                    vec!["aeria read Race --untranslated", "aeria read Tribe --untranslated"]
                ),
                (2, vec!["aeria read quest/000/Small_00002 --untranslated"]),
            ]
        );
    }
}
