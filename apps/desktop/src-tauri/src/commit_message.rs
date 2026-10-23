//! Commit messages of a project by its convention: `kind(area): summary`,
//! then the facts of the change, in the project's language (see
//! `docs/architecture/git.md#commit-messages`). Everything but the summary
//! comes from the changes themselves; the summary is a template, or a line
//! a model writes from a short digest of them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::io::BufRead as _;
use std::path::Path;

use aeria_git::{EntryChange, EntryChangeKind};
use serde_json::{Value, json};

use crate::project_changes::{ChangeKindDto, ProjectAreaDto, ProjectChangeDto};

/// Areas listed in a message's body at most.
const SHOWN_AREAS: usize = 5;
/// Terms named in a message at most.
const SHOWN_TERMS: usize = 8;
/// Changed strings a model sees at most.
const SAMPLES: usize = 10;
/// Characters of a sample's texts a model sees at most.
const SAMPLE_CHARS: usize = 80;
/// Characters of a summary a model writes at most.
const SUMMARY_CHARS: usize = 72;

/// A run of machine translation that wrote some of the changed strings, as
/// its report names it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RunFact {
    /// `translating` or `fixing`.
    pub kind: String,
    pub skill: Option<String>,
    pub model: String,
}

/// A changed string a model may see, its texts shortened.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Sample {
    source: String,
    before: String,
    after: String,
}

/// What a commit takes, in the facts its message is made of.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Digest {
    translated: usize,
    fixed: usize,
    edited: usize,
    cleared: usize,
    marked: usize,
    /// Strings by area, most first.
    areas: Vec<(String, usize)>,
    /// Strings each run wrote, most first.
    runs: Vec<(RunFact, usize)>,
    terms: Vec<(ChangeKindDto, String)>,
    knowledge: Vec<String>,
    settings: Vec<&'static str>,
    workflows: bool,
    samples: Vec<Sample>,
}

/// The area of a string: the first part of its `msgctxt`, a sheet
/// (`DefaultTalk`) or a folder of sheets (`quest`, `cut_scene`).
fn area_of(context: &str) -> &str {
    let sheet = context.split(':').next().unwrap_or(context);
    sheet.split('/').next().unwrap_or(sheet)
}

fn shorten(text: &str) -> String {
    let mut short: String = text.chars().take(SAMPLE_CHARS).collect();
    if text.chars().count() > SAMPLE_CHARS {
        short.push('…');
    }
    short
}

/// The digest of string and project file changes, with the runs that wrote
/// strings by `msgctxt`.
#[must_use]
pub fn digest(
    changes: &[EntryChange],
    project: &[ProjectChangeDto],
    written_by: &HashMap<String, RunFact>,
) -> Digest {
    let mut digest = Digest::default();
    let mut areas: BTreeMap<String, usize> = BTreeMap::new();
    let mut runs: HashMap<RunFact, usize> = HashMap::new();
    for change in changes {
        let run = written_by.get(&change.context);
        match change.kind {
            EntryChangeKind::Translated => digest.translated += 1,
            EntryChangeKind::Changed if run.is_some_and(|run| run.kind == "fixing") => {
                digest.fixed += 1;
            }
            EntryChangeKind::Changed => digest.edited += 1,
            EntryChangeKind::Cleared => digest.cleared += 1,
            EntryChangeKind::Marked => digest.marked += 1,
        }
        if let Some(run) = run {
            *runs.entry(run.clone()).or_default() += 1;
        }
        *areas
            .entry(area_of(&change.context).to_owned())
            .or_default() += 1;
    }
    digest.areas = areas.into_iter().collect();
    digest
        .areas
        .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    digest.runs = runs.into_iter().collect();
    digest
        .runs
        .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.kind.cmp(&b.0.kind)));
    digest.samples = samples(changes);
    for change in project {
        match change.area {
            ProjectAreaDto::Terms => digest.terms.extend(
                change
                    .details
                    .iter()
                    .filter(|detail| !detail.label.is_empty())
                    .map(|detail| (detail.kind, detail.label.clone())),
            ),
            ProjectAreaDto::Knowledge => digest.knowledge.push(
                change
                    .path
                    .strip_prefix("aeria-knowledge/")
                    .unwrap_or(&change.path)
                    .to_owned(),
            ),
            ProjectAreaDto::FeedWorkflow | ProjectAreaDto::GuardWorkflow => {
                digest.workflows = true;
            }
            ProjectAreaDto::ProjectSettings => digest.settings.push("project"),
            ProjectAreaDto::PackSettings => digest.settings.push("pack"),
            ProjectAreaDto::FontSettings | ProjectAreaDto::FontFile => {
                digest.settings.push("fonts");
            }
            ProjectAreaDto::GitAttributes => digest.settings.push("git"),
        }
    }
    digest.settings.dedup();
    digest
}

/// Changed strings a model may see: translated or changed ones, a few of
/// each area in turn.
fn samples(changes: &[EntryChange]) -> Vec<Sample> {
    let mut by_area: BTreeMap<&str, Vec<&EntryChange>> = BTreeMap::new();
    for change in changes {
        if matches!(
            change.kind,
            EntryChangeKind::Translated | EntryChangeKind::Changed
        ) {
            by_area
                .entry(area_of(&change.context))
                .or_default()
                .push(change);
        }
    }
    let mut picked = Vec::new();
    let mut round = 0;
    while picked.len() < SAMPLES {
        let mut any = false;
        for changes in by_area.values() {
            if let Some(change) = changes.get(round) {
                any = true;
                if picked.len() < SAMPLES {
                    picked.push(Sample {
                        source: shorten(&change.source),
                        before: shorten(&change.before.translation),
                        after: shorten(&change.after.translation),
                    });
                }
            }
        }
        if !any {
            break;
        }
        round += 1;
    }
    picked
}

impl Digest {
    /// The kind of the commit: what most of it does.
    fn kind(&self) -> &'static str {
        let strings = [
            (self.fixed, "fix"),
            (self.translated, "translate"),
            (self.edited, "edit"),
            (self.cleared, "remove"),
            (self.marked, "mark"),
        ];
        if let Some((_, kind)) = strings
            .iter()
            .filter(|(count, _)| *count > 0)
            .max_by_key(|(count, _)| *count)
        {
            return kind;
        }
        if !self.terms.is_empty() {
            "terms"
        } else if !self.knowledge.is_empty() {
            "knowledge"
        } else if !self.settings.is_empty() {
            "settings"
        } else if self.workflows {
            "ci"
        } else {
            "update"
        }
    }

    /// The area of the commit: the one its strings are in, or the one most
    /// of them are in when that is two in three or more.
    fn area(&self) -> Option<&str> {
        let total: usize = self.areas.iter().map(|(_, count)| count).sum();
        let (area, count) = self.areas.first()?;
        (*count * 3 >= total * 2).then_some(area.as_str())
    }

    fn strings(&self) -> usize {
        self.translated + self.fixed + self.edited + self.cleared + self.marked
    }
}

/// The words of a message in one language.
struct Words {
    russian: bool,
}

impl Words {
    fn of(language: &str) -> Self {
        Self {
            russian: language.split(['-', '_']).next() == Some("ru"),
        }
    }

    /// `count` with the noun of `forms`: Russian one, few, and many, or
    /// English one and many.
    fn count(&self, count: usize, forms: [&str; 3]) -> String {
        let noun = if self.russian {
            let (tens, ones) = (count % 100, count % 10);
            if ones == 1 && tens != 11 {
                forms[0]
            } else if (2..=4).contains(&ones) && !(12..=14).contains(&tens) {
                forms[1]
            } else {
                forms[2]
            }
        } else if count == 1 {
            forms[0]
        } else {
            forms[2]
        };
        format!("{count} {noun}")
    }

    fn pick<'a>(&self, russian: &'a str, english: &'a str) -> &'a str {
        if self.russian { russian } else { english }
    }

    fn strings(&self, count: usize) -> String {
        if self.russian {
            self.count(count, ["строка", "строки", "строк"])
        } else {
            self.count(count, ["string", "", "strings"])
        }
    }

    fn translations(&self, count: usize) -> String {
        if self.russian {
            self.count(count, ["перевод", "перевода", "переводов"])
        } else {
            self.count(count, ["translation", "", "translations"])
        }
    }
}

fn names(items: &[String], shown: usize) -> String {
    let mut text = items
        .iter()
        .take(shown)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > shown {
        let _ = write!(text, " +{}", items.len() - shown);
    }
    text
}

/// The summary a template gives: what most of the commit holds, as the
/// kind before it says what was done.
#[must_use]
pub fn template_summary(digest: &Digest, language: &str) -> String {
    let words = Words::of(language);
    let skill = digest
        .runs
        .iter()
        .find(|(run, _)| run.kind == "fixing")
        .and_then(|(run, _)| run.skill.clone());
    match digest.kind() {
        "fix" => match skill {
            Some(skill) => format!("{skill}, {}", words.strings(digest.fixed)),
            None => words.strings(digest.fixed),
        },
        "translate" => words.strings(digest.translated),
        "edit" => words.translations(digest.edited),
        "remove" => words.translations(digest.cleared),
        "mark" => words.strings(digest.marked),
        "terms" => {
            let terms: Vec<String> = digest.terms.iter().map(|(_, term)| term.clone()).collect();
            names(&terms, 3)
        }
        "knowledge" => names(&digest.knowledge, 3),
        "settings" => settings_names(digest, &words).join(", "),
        "ci" => words.pick("рабочие процессы", "workflows").to_owned(),
        _ => words.pick("проект", "project").to_owned(),
    }
}

fn settings_names(digest: &Digest, words: &Words) -> Vec<String> {
    digest
        .settings
        .iter()
        .map(|setting| {
            match *setting {
                "project" => words.pick("проект", "project"),
                "pack" => words.pick("пакет", "pack"),
                "fonts" => words.pick("шрифты", "fonts"),
                _ => "Git",
            }
            .to_owned()
        })
        .collect()
}

/// The facts of a commit, a line each: its strings, areas, runs, terms,
/// knowledge, and settings.
fn body(digest: &Digest, language: &str) -> Vec<String> {
    let words = Words::of(language);
    let mut lines = Vec::new();
    let counts: Vec<String> = [
        (digest.translated, words.pick("переведено", "translated")),
        (
            digest.fixed,
            words.pick("исправлено прогоном", "fixed by a run"),
        ),
        (digest.edited, words.pick("изменено", "changed")),
        (digest.cleared, words.pick("удалено", "removed")),
        (digest.marked, words.pick("отмечено", "marked")),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, label)| format!("{label} {count}"))
    .collect();
    if !counts.is_empty() {
        let mut line = counts.join(", ");
        if let Some(first) = line.chars().next() {
            line.replace_range(..first.len_utf8(), &first.to_uppercase().to_string());
        }
        lines.push(line);
    }
    if digest.areas.len() > 1 {
        let shown: Vec<String> = digest
            .areas
            .iter()
            .take(SHOWN_AREAS)
            .map(|(area, count)| format!("{area} {count}"))
            .collect();
        let mut line = format!("{}: {}", words.pick("Области", "Areas"), shown.join(", "));
        if digest.areas.len() > SHOWN_AREAS {
            let _ = write!(line, " +{}", digest.areas.len() - SHOWN_AREAS);
        }
        lines.push(line);
    }
    for (run, count) in &digest.runs {
        let kind = if run.kind == "fixing" {
            words.pick("Редактура", "Editing")
        } else {
            words.pick("Перевод", "Translation")
        };
        let skill = run
            .skill
            .as_ref()
            .map(|skill| format!(" «{skill}»"))
            .unwrap_or_default();
        lines.push(format!(
            "{kind}{skill} ({}): {}",
            run.model,
            words.strings(*count)
        ));
    }
    if !digest.terms.is_empty() {
        let terms: Vec<String> = digest
            .terms
            .iter()
            .map(|(kind, term)| {
                let mark = match kind {
                    ChangeKindDto::Added => "+",
                    ChangeKindDto::Removed => "−",
                    ChangeKindDto::Modified => "~",
                };
                format!("{mark}{term}")
            })
            .collect();
        lines.push(format!(
            "{}: {}",
            words.pick("Термины", "Terms"),
            names(&terms, SHOWN_TERMS)
        ));
    }
    if !digest.knowledge.is_empty() {
        lines.push(format!(
            "{}: {}",
            words.pick("Знания", "Knowledge"),
            names(&digest.knowledge, SHOWN_TERMS)
        ));
    }
    if !digest.settings.is_empty() {
        lines.push(format!(
            "{}: {}",
            words.pick("Настройки", "Settings"),
            settings_names(digest, &words).join(", ")
        ));
    }
    if digest.workflows {
        lines.push(words.pick("Рабочие процессы", "Workflows").to_owned());
    }
    lines
}

/// The message of a commit: its header with `summary`, or the template's,
/// and its facts. `None` when it takes nothing.
#[must_use]
pub fn message(digest: &Digest, language: &str, summary: Option<&str>) -> Option<String> {
    if digest.strings() == 0
        && digest.terms.is_empty()
        && digest.knowledge.is_empty()
        && digest.settings.is_empty()
        && !digest.workflows
    {
        return None;
    }
    let summary = summary.map_or_else(|| template_summary(digest, language), str::to_owned);
    let header = match digest.area() {
        Some(area) => format!("{}({area}): {summary}", digest.kind()),
        None => format!("{}: {summary}", digest.kind()),
    };
    let body = body(digest, language);
    Some(if body.is_empty() {
        header
    } else {
        format!("{header}\n\n{}", body.join("\n"))
    })
}

/// What a model is asked for a summary: the rules, then the digest as JSON.
#[must_use]
pub fn summary_request(digest: &Digest, language: &str) -> (String, String) {
    // The verb takes the form a commit summary takes in the language.
    let verb = if Words::of(language).russian {
        "a verb in the infinitive first (исправить, перевести, согласовать; never the \
         imperative исправь), in lowercase"
    } else {
        "an imperative verb first, in lowercase"
    };
    let instructions = format!(
        "Write the summary of a Git commit of a translation project of a game: one line \
         in the language `{language}`, at most 60 characters, {verb}, no period at the \
         end. Say what the change does to the text, by its samples and runs (a skill names \
         the correction), not how many strings it touches. No kind or area prefix: the \
         line follows `kind(area): `. Answer with the line only."
    );
    let input = json!({
        "kind": digest.kind(),
        "area": digest.area(),
        "strings": {
            "translated": digest.translated,
            "fixedByRun": digest.fixed,
            "changed": digest.edited,
            "removed": digest.cleared,
            "marked": digest.marked,
        },
        "areas": digest.areas.iter().take(SHOWN_AREAS).map(|(area, count)| json!([area, count])).collect::<Vec<_>>(),
        "runs": digest.runs.iter().map(|(run, count)| json!({"kind": run.kind, "skill": run.skill, "strings": count})).collect::<Vec<_>>(),
        "terms": digest.terms.iter().take(SHOWN_TERMS).map(|(_, term)| term).collect::<Vec<_>>(),
        "knowledge": digest.knowledge,
        "samples": digest.samples.iter().map(|sample| json!({"source": sample.source, "before": sample.before, "after": sample.after})).collect::<Vec<_>>(),
    });
    (instructions, input.to_string())
}

/// The summary in a model's answer: its first line, without quotes, a
/// prefix it was told not to write, or a final period; `None` when empty.
#[must_use]
pub fn read_summary(answer: &str) -> Option<String> {
    let line = answer
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    // Quotes around the whole line go; quotes of a name inside it stay.
    let quoted = |open: char, close: char| line.strip_prefix(open)?.strip_suffix(close);
    let line = quoted('"', '"')
        .or_else(|| quoted('«', '»'))
        .or_else(|| quoted('`', '`'))
        .or_else(|| quoted('\'', '\''))
        .unwrap_or(line);
    let line = line
        .split_once("): ")
        .filter(|(prefix, _)| !prefix.contains(' '))
        .map_or(line, |(_, rest)| rest);
    let line = line.trim().trim_end_matches('.').trim();
    if line.is_empty() {
        return None;
    }
    let mut summary: String = line.chars().take(SUMMARY_CHARS).collect();
    if let Some(first) = summary.chars().next() {
        summary.replace_range(..first.len_utf8(), &first.to_lowercase().to_string());
    }
    Some(summary)
}

/// The run that last wrote each of `contexts`, from the reports of `folder`
/// changed since `since` (Unix seconds): a run's strings written before
/// the last commit are committed already.
#[must_use]
pub fn written_by(folder: &Path, since: i64, contexts: &HashSet<&str>) -> HashMap<String, RunFact> {
    let mut found = HashMap::new();
    let Ok(entries) = std::fs::read_dir(folder) else {
        return found;
    };
    let since = std::time::UNIX_EPOCH + std::time::Duration::from_secs(since.max(0).unsigned_abs());
    let mut reports: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("translation-"))
        })
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            (modified >= since).then_some((modified, path))
        })
        .collect();
    reports.sort();
    for (_, path) in reports {
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let mut run = None;
        for line in std::io::BufReader::new(file).lines() {
            let Ok(line) = line else { break };
            if run.is_none() {
                let Ok(record) = serde_json::from_str::<Value>(&line) else {
                    break;
                };
                if record["record"] != "run" {
                    break;
                }
                run = Some(RunFact {
                    kind: record["kind"].as_str().unwrap_or_default().to_owned(),
                    skill: record["skill"].as_str().map(str::to_owned),
                    model: record["model"].as_str().unwrap_or_default().to_owned(),
                });
                continue;
            }
            // Only written strings name a run; the rest are skipped unread.
            if !line.contains("\"outcome\":\"written\"") {
                continue;
            }
            let Ok(record) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let (Some(context), Some(run)) = (record["context"].as_str(), &run)
                && contexts.contains(context)
            {
                found.insert(context.to_owned(), run.clone());
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeria_git::EntryState;

    fn change(context: &str, kind: EntryChangeKind, before: &str, after: &str) -> EntryChange {
        EntryChange {
            path: "po/x.po".to_owned(),
            context: context.to_owned(),
            source: "Hello there.".to_owned(),
            kind,
            before: EntryState {
                translation: before.to_owned(),
                ..EntryState::default()
            },
            after: EntryState {
                translation: after.to_owned(),
                ..EntryState::default()
            },
        }
    }

    #[test]
    fn a_skill_correction_is_a_fix_of_its_area() {
        let changes = vec![
            change(
                "cut_scene/055/VoiceMan:A:1",
                EntryChangeKind::Changed,
                "a",
                "b",
            ),
            change(
                "cut_scene/055/VoiceMan:B:1",
                EntryChangeKind::Changed,
                "c",
                "d",
            ),
            change("DefaultTalk:1:0:0", EntryChangeKind::Translated, "", "e"),
        ];
        let run = RunFact {
            kind: "fixing".to_owned(),
            skill: Some("Согласование по полу игрока".to_owned()),
            model: "m".to_owned(),
        };
        let written: HashMap<String, RunFact> = changes[..2]
            .iter()
            .map(|change| (change.context.clone(), run.clone()))
            .collect();
        let digest = digest(&changes, &[], &written);
        assert_eq!(
            message(&digest, "ru", None).as_deref(),
            Some(
                "fix(cut_scene): Согласование по полу игрока, 2 строки\n\n\
                 Переведено 1, исправлено прогоном 2\n\
                 Области: cut_scene 2, DefaultTalk 1\n\
                 Редактура «Согласование по полу игрока» (m): 2 строки"
            )
        );
        assert_eq!(
            message(&digest, "en", Some("agree lines with the player's gender"))
                .expect("message")
                .lines()
                .next(),
            Some("fix(cut_scene): agree lines with the player's gender")
        );
    }

    #[test]
    fn strings_spread_over_areas_have_no_area() {
        let changes = vec![
            change("quest/1:A:1", EntryChangeKind::Translated, "", "a"),
            change("Addon:1:0:0", EntryChangeKind::Translated, "", "b"),
        ];
        let digest = digest(&changes, &[], &HashMap::new());
        assert_eq!(
            message(&digest, "en", None)
                .expect("message")
                .lines()
                .next(),
            Some("translate: 2 strings")
        );
        assert_eq!(message(&Digest::default(), "ru", None), None);
    }

    #[test]
    fn a_model_answer_is_read_as_one_summary_line() {
        assert_eq!(
            read_summary("\n«fix(quest): Согласовать реплики с полом игрока.»\nextra"),
            Some("согласовать реплики с полом игрока".to_owned())
        );
        assert_eq!(
            read_summary("Перевести: квест «Сокровище»"),
            Some("перевести: квест «Сокровище»".to_owned())
        );
        assert_eq!(read_summary("  \n "), None);
    }

    #[test]
    fn a_russian_summary_asks_for_the_infinitive() {
        let digest = Digest::default();
        assert!(summary_request(&digest, "ru").0.contains("infinitive"));
        assert!(
            summary_request(&digest, "en")
                .0
                .contains("imperative verb first")
        );
    }

    #[test]
    fn russian_counts_agree() {
        let words = Words::of("ru");
        assert_eq!(words.strings(1), "1 строка");
        assert_eq!(words.strings(3), "3 строки");
        assert_eq!(words.strings(11), "11 строк");
        assert_eq!(words.strings(22), "22 строки");
        assert_eq!(words.translations(25), "25 переводов");
    }
}
