//! `overview`, `read`, `find`, and `knowledge`: what an agent reads.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use aeria_core::ReviewState;
use aeria_knowledge::{Domain, sheet_domain};
use aeria_search::SourceQuery;
use serde::Serialize;
use serde_json::json;

use super::project::{
    Address, Author, Current, LineKind, Project, SheetLine, current, matches_pattern,
    other_languages, review_word, sheet_lines, translated,
};
use super::{Output, fmt_count};

// ---------------------------------------------------------------- overview

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Counts {
    sheets: usize,
    strings: usize,
    translated: usize,
    reviewed: usize,
    needs_review: usize,
}

impl Counts {
    fn add(&mut self, other: &SheetCounts) {
        self.sheets += 1;
        self.strings += other.strings;
        self.translated += other.translated;
        self.reviewed += other.reviewed;
        self.needs_review += other.needs_review;
    }

    fn line(&self) -> String {
        format!(
            "{} sheets · {} strings · {} translated · {} reviewed · {} need review",
            fmt_count(self.sheets),
            fmt_count(self.strings),
            fmt_count(self.translated),
            fmt_count(self.reviewed),
            fmt_count(self.needs_review)
        )
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SheetCounts {
    pub(crate) name: String,
    pub(crate) strings: usize,
    pub(crate) translated: usize,
    reviewed: usize,
    needs_review: usize,
}

pub(crate) fn sheet_counts(project: &Project) -> Vec<SheetCounts> {
    let progress: BTreeMap<String, _> = project
        .session
        .translation_progress()
        .into_iter()
        .map(|sheet| (sheet.sheet_name.clone(), sheet))
        .collect();
    project
        .session
        .source()
        .catalog()
        .map(|catalog| {
            catalog
                .iter()
                .filter(|sheet| sheet.translatable > 0)
                .map(|sheet| {
                    let counts = progress.get(&sheet.name);
                    SheetCounts {
                        name: sheet.name.clone(),
                        strings: sheet.translatable,
                        translated: counts.map_or(0, |counts| counts.translated),
                        reviewed: counts.map_or(0, |counts| counts.reviewed),
                        needs_review: counts.map_or(0, |counts| counts.needs_review),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The area of a sheet in the overview.
fn area(sheet: &str) -> &'static str {
    if sheet.starts_with("quest/") {
        "quests"
    } else if sheet.starts_with("cut_scene/") {
        "cutscenes"
    } else {
        match sheet_domain(sheet) {
            Domain::Names => "names",
            Domain::Items => "items",
            Domain::Actions => "actions",
            Domain::Lore => "lore",
            _ => "interface",
        }
    }
}

/// The folder of a sheet: its first two path segments.
fn folder(sheet: &str) -> Option<String> {
    let mut parts = sheet.split('/');
    let first = parts.next()?;
    let second = parts.next()?;
    parts.next()?;
    Some(format!("{first}/{second}"))
}

pub(crate) struct OverviewOptions {
    pub pattern: Option<String>,
    pub folders: bool,
    pub untranslated: bool,
    pub limit: usize,
}

pub(crate) fn overview(project: &Project, options: &OverviewOptions, out: &mut Output) {
    let sheets = sheet_counts(project);
    let mut total = Counts::default();
    for sheet in &sheets {
        total.add(sheet);
    }
    let target = project
        .target_language()
        .unwrap_or_else(|| "(no target language yet)".to_owned());
    if let Some(pattern) = &options.pattern {
        let mut matching: Vec<&SheetCounts> = sheets
            .iter()
            .filter(|sheet| matches_pattern(&sheet.name, pattern))
            .filter(|sheet| !options.untranslated || sheet.translated < sheet.strings)
            .collect();
        let mut sum = Counts::default();
        for sheet in &matching {
            sum.add(sheet);
        }
        let more = matching.len().saturating_sub(options.limit);
        matching.truncate(options.limit);
        if out.json {
            out.json_value(&json!({ "total": sum, "sheets": matching, "more": more }));
            return;
        }
        let _ = writeln!(out.text, "{pattern}: {}", sum.line());
        for sheet in &matching {
            let _ = writeln!(
                out.text,
                "{}  {}/{} translated · {} reviewed{}",
                sheet.name,
                fmt_count(sheet.translated),
                fmt_count(sheet.strings),
                fmt_count(sheet.reviewed),
                if sheet.needs_review > 0 {
                    format!(" · {} need review", fmt_count(sheet.needs_review))
                } else {
                    String::new()
                }
            );
        }
        if more > 0 {
            let _ = writeln!(
                out.text,
                "… {more} more; narrow the pattern or pass --limit"
            );
        }
        return;
    }
    let mut areas: BTreeMap<&str, Counts> = BTreeMap::new();
    let mut folders: BTreeMap<String, Counts> = BTreeMap::new();
    for sheet in &sheets {
        areas.entry(area(&sheet.name)).or_default().add(sheet);
        if let Some(folder) = folder(&sheet.name) {
            folders.entry(folder).or_default().add(sheet);
        }
    }
    let knowledge = project.knowledge();
    if out.json {
        out.json_value(&json!({
            "root": project.root_display(),
            "sourceLanguage": project.source_language(),
            "targetLanguage": project.target_language(),
            "gameVersion": project.session.source().version().to_string(),
            "total": total,
            "areas": areas,
            "folders": if options.folders { Some(&folders) } else { None },
            "strayFiles": stray_files(project.root()),
            "knowledgeProblems": knowledge.problems,
        }));
        return;
    }
    let _ = writeln!(
        out.text,
        "{} · {} → {target} · game {}",
        project.root_display(),
        project.source_language(),
        project.session.source().version()
    );
    let _ = writeln!(out.text, "{}\n", total.line());
    let _ = writeln!(out.text, "Areas (sheet patterns in brackets):");
    for (name, counts) in &areas {
        let hint = match *name {
            "quests" => " [quest/*]",
            "cutscenes" => " [cut_scene/*]",
            _ => "",
        };
        let _ = writeln!(out.text, "  {name}{hint}: {}", counts.line());
    }
    let _ = writeln!(
        out.text,
        "\nKnowledge in aeria-knowledge/: {} style sections, {} terms, {} character voices, {} stories, {} lessons",
        knowledge.style.len(),
        knowledge.terms.entries.len(),
        knowledge.characters.profiles.len(),
        knowledge.story.len(),
        knowledge.lessons.len()
    );
    for problem in &knowledge.problems {
        let _ = writeln!(out.text, "  problem: {problem}");
    }
    let strays = stray_files(project.root());
    if !strays.is_empty() {
        let _ = writeln!(
            out.text,
            "\nFiles in the project root that look temporary: {}. Temporary files belong in the system's temporary folder; delete these if nothing needs them.",
            strays.join(", ")
        );
    }
    if options.folders {
        let _ = writeln!(out.text, "\nFolders:");
        for (name, counts) in &folders {
            let _ = writeln!(out.text, "  {name}/*: {}", counts.line());
        }
    } else {
        let _ = writeln!(
            out.text,
            "\n`aeria overview --folders` lists folders such as quest/000; `aeria overview <pattern>` lists sheets."
        );
    }
}

/// Files at the project root that look like an agent's temporary files:
/// batches, lists, and JSON Lines that are not part of the project.
fn stray_files(root: &std::path::Path) -> Vec<String> {
    const TEMPORARY: [&str; 6] = ["batch", "jsonl", "txt", "tmp", "csv", "tsv"];
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut strays: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            std::path::Path::new(name)
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    TEMPORARY.contains(&extension.to_ascii_lowercase().as_str())
                })
        })
        .collect();
    strays.sort();
    strays
}

// ---------------------------------------------------------------- read

pub(crate) struct ReadOptions {
    pub sheet: String,
    /// Inclusive row range.
    pub rows: Option<(u32, u32)>,
    pub untranslated: bool,
    /// Zero-based position in the whole listing to start from.
    pub from: usize,
    /// `--limit`; without it at most [`READ_LIMIT`] strings, and a
    /// continuation keeps what is left of a given one.
    pub limit: Option<usize>,
    /// Bytes of text output at most, 0 for no bound; whole strings are
    /// left out past it, and the end says how to go on.
    pub max_bytes: usize,
    pub knowledge: bool,
    pub memory: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadLine {
    address: String,
    #[serde(flatten)]
    kind: LineKind,
    source: String,
    other_languages: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    macros: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    context: Vec<(u32, String)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current: Option<Current>,
    /// The French or German line varies with the player character's gender
    /// where the source does not.
    gender_varies: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    similar: Vec<Similar>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Similar {
    address: String,
    similarity: f64,
    source: String,
    target: String,
    #[serde(serialize_with = "super::project::serialize_review")]
    review: ReviewState,
}

/// Strings `read` lists without `--limit`.
pub(crate) const READ_LIMIT: usize = 400;

/// Most similar translations shown per line.
const SIMILAR_PER_LINE: usize = 2;

pub(crate) fn quest_title(project: &Project, sheet: &str) -> Option<String> {
    let (row, subrow) = project.session.source().quest_row(sheet).ok().flatten()?;
    let quest = project.sheet("Quest").ok()?;
    let row = quest.row(row, subrow)?;
    quest
        .cells(row)
        .find(|cell| cell.translatable && !cell.text().trim().is_empty())
        .map(|cell| cell.text())
}

fn line_domain(sheet: &str, kind: &LineKind) -> Domain {
    match kind {
        LineKind::Journal => Domain::Journal,
        LineKind::Objective => Domain::Objective,
        LineKind::Speech(speaker) if speaker.starts_with("SYSTEM") => Domain::System,
        LineKind::Speech(_) | LineKind::Other => Domain::Dialogue,
        LineKind::Text => sheet_domain(sheet),
    }
}

fn similar(project: &Project, line: &SheetLine) -> Vec<Similar> {
    let address = &line.address;
    let candidates = project.similar_sources(&line.source);
    let mut seen = std::collections::HashSet::new();
    candidates
        .iter()
        .filter(|candidate| {
            (
                candidate.hit.sheet.as_str(),
                candidate.hit.row,
                candidate.hit.subrow,
                candidate.hit.column,
            ) != (
                address.sheet.as_str(),
                address.row,
                address.subrow,
                address.column,
            )
        })
        .filter_map(|candidate| {
            let found = Address {
                sheet: candidate.hit.sheet.clone(),
                row: candidate.hit.row,
                subrow: candidate.hit.subrow,
                column: candidate.hit.column,
            };
            let unit = project
                .session
                .workspace()
                .unit_by_source_binding(&found.binding())?;
            let target = unit.target_macro();
            if target.trim().is_empty()
                || !seen.insert((candidate.hit.source.clone(), target.to_owned()))
            {
                return None;
            }
            Some(Similar {
                address: found.to_string(),
                similarity: (candidate.score * 100.0).round() / 100.0,
                source: candidate.hit.source.clone(),
                target: target.to_owned(),
                review: unit.review_state(),
            })
        })
        .take(SIMILAR_PER_LINE)
        .collect()
}

/// Threads that look up similar strings for one read.
const SIMILAR_THREADS: usize = 4;

/// Looks up the similar strings of the lines an agent may write, on several
/// threads, so the listing finds them cached.
fn prefetch_similar(project: &Project, lines: &[&SheetLine], ledger: Option<&crate::sync::Ledger>) {
    let sources: Vec<&str> = lines
        .iter()
        .filter(|line| {
            current(&project.session, ledger, &line.address).is_none_or(|now| now.replaceable())
        })
        .map(|line| line.source.as_str())
        .collect();
    if sources.len() < 8 {
        return;
    }
    let chunk = sources.len().div_ceil(SIMILAR_THREADS);
    std::thread::scope(|scope| {
        for part in sources.chunks(chunk) {
            scope.spawn(move || {
                for source in part {
                    let _ = project.similar_sources(source);
                }
            });
        }
    });
}

#[allow(clippy::too_many_lines)] // one listing
pub(crate) fn read(
    project: &Project,
    options: &ReadOptions,
    out: &mut Output,
) -> Result<(), String> {
    let all = sheet_lines(project, &options.sheet)?;
    let ledger = project.ledger();
    let dialogue = all.iter().any(|line| line.kind != LineKind::Text);
    let total = all.len();
    let untranslated_count = all
        .iter()
        .filter(|line| !translated(&project.session, &line.address))
        .count();
    // Each selected string with its position in the whole listing.
    let (positions, mut selected): (Vec<usize>, Vec<&SheetLine>) = all
        .iter()
        .enumerate()
        .skip(options.from)
        .filter(|(_, line)| {
            options
                .rows
                .is_none_or(|(first, last)| (first..=last).contains(&line.address.row))
        })
        .filter(|(_, line)| !options.untranslated || !translated(&project.session, &line.address))
        .unzip();
    let limit = options.limit.unwrap_or(READ_LIMIT);
    let mut next = positions.get(limit).copied();
    selected.truncate(limit);

    let mut domains: Vec<Domain> = Vec::new();
    let mut speakers: Vec<&str> = Vec::new();
    if options.memory {
        prefetch_similar(project, &selected, ledger.as_ref());
    }
    let mut lines = Vec::with_capacity(selected.len());
    for line in &selected {
        let domain = line_domain(&options.sheet, &line.kind);
        if !domains.contains(&domain) {
            domains.push(domain);
        }
        if let LineKind::Speech(speaker) = &line.kind
            && !speakers.contains(&speaker.as_str())
        {
            speakers.push(speaker);
        }
        let now = current(&project.session, ledger.as_ref(), &line.address);
        let evidence = other_languages(project, &line.address);
        let gender_varies = !line.source.contains("$gn4")
            && evidence
                .iter()
                .any(|(code, text)| matches!(code.as_str(), "fr" | "de") && text.contains("$gn4"));
        lines.push(ReadLine {
            address: line.address.to_string(),
            kind: line.kind.clone(),
            source: line.source.clone(),
            other_languages: evidence.into_iter().collect(),
            macros: aeria_se::constructs(&line.source)
                .map(|constructs| constructs.iter().map(aeria_se::Construct::legend).collect())
                .unwrap_or_default(),
            context: line.context.clone(),
            // Similar translations help only where the line may be written.
            similar: if options.memory && now.as_ref().is_none_or(Current::replaceable) {
                similar(project, line)
            } else {
                Vec::new()
            },
            current: now,
            gender_varies,
        });
    }
    let title = quest_title(project, &options.sheet);
    let knowledge = options.knowledge.then(|| {
        project.knowledge().scene_slice(
            &domains,
            selected.iter().map(|line| line.source.as_str()),
            speakers.iter().copied(),
            &[options.sheet.as_str()],
        )
    });

    let source_language = project.source_language();
    let target = project
        .target_language()
        .unwrap_or_else(|| "target".to_owned());
    let kind = if options.sheet.starts_with("quest/") {
        "quest"
    } else if options.sheet.starts_with("cut_scene/") {
        "cutscene"
    } else {
        "sheet"
    };
    let mut head = format!("# {}", options.sheet);
    if let Some(title) = &title {
        let _ = write!(head, " — {kind} «{title}»");
    }
    let _ = writeln!(
        head,
        " · {} strings · {} untranslated{}",
        fmt_count(total),
        fmt_count(untranslated_count),
        if dialogue { " · in play order" } else { "" }
    );
    let _ = writeln!(
        head,
        "kinds of text: {}",
        domains
            .iter()
            .map(|domain| domain.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(knowledge) = knowledge.as_ref().filter(|text| !text.is_empty()) {
        let _ = writeln!(head, "\n## Project knowledge for these lines\n{knowledge}");
    }
    head.push_str("\n## Lines\n");
    let blocks: Vec<String> = lines
        .iter()
        .map(|line| line_block(line, &source_language, &target))
        .collect();
    // Harnesses cut long command output; whole strings past the budget are
    // left for the next read. The first string is always shown.
    let mut kept = blocks.len();
    if options.max_bytes > 0 {
        // JSON is measured as JSON: escaping makes it larger than the text.
        let size = |index: usize| {
            if out.json {
                serde_json::to_string_pretty(&lines[index]).map_or(0, |line| line.len())
            } else {
                blocks[index].len()
            }
        };
        let mut used = if out.json {
            serde_json::to_string_pretty(&knowledge).map_or(0, |knowledge| knowledge.len())
        } else {
            head.len()
        };
        for (index, position) in positions.iter().take(blocks.len()).enumerate() {
            used += size(index);
            if index > 0 && used > options.max_bytes {
                kept = index;
                next = Some(*position);
                break;
            }
        }
    }
    lines.truncate(kept);
    // Position shown to agents is one-based.
    let next_from = next.map(|position| position + 1);

    if out.json {
        out.json_value(&json!({
            "sheet": options.sheet,
            "quest": title,
            "total": total,
            "untranslated": untranslated_count,
            "knowledge": knowledge,
            "lines": lines,
            "nextFrom": next_from,
        }));
        return Ok(());
    }
    out.text.push_str(&head);
    for block in &blocks[..kept] {
        out.text.push_str(block);
    }
    if let Some(from) = next_from {
        // Past its --limit the listing may belong to another agent's task.
        let left = limit.saturating_sub(kept);
        if options.limit.is_some() && left == 0 {
            let _ = writeln!(
                out.text,
                "\n--limit reached; the {} strings after it are for other tasks.",
                positions.len() - kept.min(positions.len())
            );
            return Ok(());
        }
        let mut again = format!("aeria read {} --from {from}", options.sheet);
        if options.limit.is_some() {
            let _ = write!(again, " --limit {left}");
        }
        if let Some((first, last)) = options.rows {
            let _ = write!(again, " --rows {first}-");
            if last != u32::MAX {
                let _ = write!(again, "{last}");
            }
        }
        if options.untranslated {
            again.push_str(" --untranslated");
        }
        let _ = writeln!(
            out.text,
            "\n… {} more strings of this listing were left out to keep the output short: `{again}`",
            positions.len().min(limit) - kept.min(positions.len())
        );
    }
    Ok(())
}

/// One string of a read listing as text.
fn line_block(line: &ReadLine, source_language: &str, target: &str) -> String {
    let mut out = String::new();
    let _ = write!(out, "\n@{} · {}", line.address, line.kind.label());
    match &line.current {
        None => out.push_str(" · untranslated"),
        Some(current) => {
            let _ = write!(
                out,
                " · {} {}",
                match current.author {
                    Author::Agent => "agent's",
                    Author::Person => "person's",
                },
                review_word(current.review)
            );
            if !current.replaceable() {
                out.push_str(" (keep)");
            }
        }
    }
    if line.gender_varies {
        out.push_str(" · fr/de vary by player gender");
    }
    let _ = write!(out, "\n  {source_language}: {}", line.source);
    for (code, text) in &line.other_languages {
        let _ = write!(out, "\n  {code}: {text}");
    }
    for legend in &line.macros {
        let _ = write!(out, "\n  macro: {legend}");
    }
    for (column, text) in &line.context {
        let _ = write!(out, "\n  column {column}: {text}");
    }
    if let Some(current) = &line.current {
        let _ = write!(out, "\n  {target}: {}", current.target);
        if let Some(note) = &current.note {
            let _ = write!(out, "\n  note: {note}");
        }
    }
    for similar in &line.similar {
        let _ = write!(
            out,
            "\n  similar ({:.0} %, {}): {} → {}",
            similar.similarity * 100.0,
            review_word(similar.review),
            similar.source,
            similar.target
        );
    }
    out.push('\n');
    out
}

// ---------------------------------------------------------------- find

pub(crate) enum FindIn {
    Source,
    Translation,
}

pub(crate) struct FindOptions {
    pub text: String,
    pub within: FindIn,
    pub sheet: Option<String>,
    pub limit: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Found {
    address: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
}

/// Translations containing the text, in binding order, and whether more
/// follow.
fn find_translations(project: &Project, options: &FindOptions) -> (Vec<Found>, bool) {
    let mut units: Vec<_> = project
        .session
        .workspace()
        .units()
        .filter(|unit| unit.is_bound())
        .filter(|unit| {
            options
                .sheet
                .as_deref()
                .is_none_or(|pattern| matches_pattern(unit.source_binding().sheet_name(), pattern))
        })
        .filter(|unit| aeria_search::text_contains(unit.target_macro(), &options.text))
        .collect();
    units.sort_by(|left, right| left.source_binding().cmp(right.source_binding()));
    let more = units.len() > options.limit;
    let found = units
        .into_iter()
        .take(options.limit)
        .map(|unit| {
            let binding = unit.source_binding();
            Found {
                address: format!(
                    "{}:{}:{}:{}",
                    binding.sheet_name(),
                    binding.row_id(),
                    binding.subrow_id(),
                    binding.column_index()
                ),
                source: project.session.source_macro(binding).unwrap_or_default(),
                target: Some(unit.target_macro().to_owned()),
            }
        })
        .collect();
    (found, more)
}

/// Source strings containing the text, from the search index, and whether
/// more follow.
fn find_source(
    project: &Project,
    options: &FindOptions,
    out: &mut Output,
) -> Result<(Vec<Found>, bool), String> {
    let mut notices = Vec::new();
    let index = project.search_index_or_build(&mut |notice| notices.push(notice.to_owned()))?;
    for notice in notices {
        out.warn(&notice);
    }
    // A sheet pattern with wildcards filters the hits; a plain name narrows
    // the search itself.
    let exact_sheet = options
        .sheet
        .as_deref()
        .filter(|sheet| !sheet.contains(['*', '?']));
    let mut found = Vec::new();
    let mut offset = 0;
    loop {
        let page = index
            .search(&SourceQuery {
                text: &options.text,
                sheet: exact_sheet,
                offset,
                limit: 200,
            })
            .map_err(|error| error.to_string())?;
        offset += u32::try_from(page.hits.len()).unwrap_or(u32::MAX);
        for hit in page.hits {
            if options
                .sheet
                .as_deref()
                .is_some_and(|pattern| !matches_pattern(&hit.sheet, pattern))
            {
                continue;
            }
            if found.len() == options.limit {
                return Ok((found, true));
            }
            let address = Address {
                sheet: hit.sheet,
                row: hit.row,
                subrow: hit.subrow,
                column: hit.column,
            };
            let target = project
                .session
                .workspace()
                .unit_by_source_binding(&address.binding())
                .map(|unit| unit.target_macro().to_owned());
            found.push(Found {
                address: address.to_string(),
                source: hit.source,
                target,
            });
        }
        if !page.more {
            return Ok((found, false));
        }
    }
}

pub(crate) fn find(
    project: &Project,
    options: &FindOptions,
    out: &mut Output,
) -> Result<(), String> {
    let (found, more) = match options.within {
        FindIn::Translation => find_translations(project, options),
        FindIn::Source => find_source(project, options, out)?,
    };
    if out.json {
        out.json_value(&json!({ "matches": found, "more": more }));
        return Ok(());
    }
    if found.is_empty() {
        out.text.push_str("nothing found\n");
    }
    let target = project
        .target_language()
        .unwrap_or_else(|| "target".to_owned());
    for item in &found {
        let _ = writeln!(
            out.text,
            "@{}\n  {}: {}",
            item.address,
            project.source_language(),
            item.source
        );
        if let Some(text) = &item.target {
            let _ = writeln!(out.text, "  {target}: {text}");
        }
    }
    if more {
        let _ = writeln!(out.text, "… more matches; pass --limit or --sheet");
    }
    Ok(())
}

// ---------------------------------------------------------------- knowledge

pub(crate) fn knowledge(project: &Project, out: &mut Output) -> bool {
    let knowledge = project.knowledge();
    let settled_terms = knowledge
        .terms
        .entries
        .iter()
        .filter(|term| term.settled)
        .count();
    if out.json {
        out.json_value(&json!({
            "style": knowledge.style.iter().map(|section| &section.key).collect::<Vec<_>>(),
            "terms": knowledge.terms.entries.len(),
            "settledTerms": settled_terms,
            "characters": knowledge.characters.profiles.len(),
            "stories": knowledge.story.len(),
            "lessons": knowledge.lessons.len(),
            "problems": knowledge.problems,
        }));
        return knowledge.problems.is_empty();
    }
    let _ = writeln!(
        out.text,
        "aeria-knowledge/ in {}:\n  style.md: {}\n  terms.csv: {} terms, {} settled\n  characters.md: {} characters\n  story.md: {} stories\n  lessons.md: {} lessons",
        project.root_display(),
        if knowledge.style.is_empty() {
            "no sections".to_owned()
        } else {
            knowledge
                .style
                .iter()
                .map(|section| section.key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        },
        knowledge.terms.entries.len(),
        settled_terms,
        knowledge.characters.profiles.len(),
        knowledge.story.len(),
        knowledge.lessons.len()
    );
    if knowledge.problems.is_empty() {
        out.text.push_str("no problems\n");
    } else {
        for problem in &knowledge.problems {
            let _ = writeln!(out.text, "problem: {problem}");
        }
    }
    knowledge.problems.is_empty()
}
