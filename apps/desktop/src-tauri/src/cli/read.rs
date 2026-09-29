//! `overview`, `read`, `find`, and `knowledge`: what an agent reads.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use aeria_core::ReviewState;
use aeria_knowledge::{Domain, Knowledge, sheet_domain};
use aeria_search::{SimilarSearch, SourceIndex, SourceQuery, Tokenizer};
use serde::Serialize;
use serde_json::json;

use super::project::{
    Address, Author, Current, LineKind, Project, SheetLine, current, matches_pattern,
    other_languages, review_word, sheet_lines,
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
struct SheetCounts {
    name: String,
    strings: usize,
    translated: usize,
    reviewed: usize,
    needs_review: usize,
}

fn sheet_counts(project: &Project) -> Vec<SheetCounts> {
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
    let knowledge = Knowledge::load(project.root());
    if out.json {
        out.json_value(&json!({
            "root": project.root_display(),
            "sourceLanguage": project.source_language(),
            "targetLanguage": project.target_language(),
            "gameVersion": project.session.source().version().to_string(),
            "total": total,
            "areas": areas,
            "folders": if options.folders { Some(&folders) } else { None },
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

// ---------------------------------------------------------------- read

pub(crate) struct ReadOptions {
    pub sheet: String,
    /// Inclusive row range.
    pub rows: Option<(u32, u32)>,
    pub untranslated: bool,
    pub limit: usize,
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

/// Most similar translations shown per line.
const SIMILAR_PER_LINE: usize = 2;
const SIMILAR_CANDIDATES: usize = 60;

fn quest_title(project: &Project, sheet: &str) -> Option<String> {
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

/// The search index of the project's game data, when it was built.
pub(crate) fn search_index(project: &Project) -> Option<(String, SourceIndex)> {
    let key = search_key(project);
    let path = search_index_path(project, &key)?;
    SourceIndex::open(&path, &key)
        .ok()
        .flatten()
        .map(|index| (key, index))
}

fn search_key(project: &Project) -> String {
    let source = project.session.source();
    format!("{}/{}", source.language(), source.version())
}

fn search_index_path(project: &Project, key: &str) -> Option<std::path::PathBuf> {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(key.as_bytes());
    let name = digest[..16]
        .iter()
        .fold(String::with_capacity(32), |mut name, byte| {
            let _ = write!(name, "{byte:02x}");
            name
        });
    project
        .data_dir
        .as_ref()
        .map(|dir| dir.join("search").join(format!("{name}.sqlite3")))
}

/// The project's search index, built first when missing. Building reads
/// every sheet of the game once.
fn search_index_or_build(project: &Project) -> Result<SourceIndex, String> {
    if let Some((_, index)) = search_index(project) {
        return Ok(index);
    }
    let key = search_key(project);
    let path = search_index_path(project, &key)
        .ok_or_else(|| "Aeria's data folder is unknown, so there is no search index".to_owned())?;
    eprintln!(
        "aeria: building the search index of this game version once; this takes a few minutes…"
    );
    let source = project.session.source_handle();
    SourceIndex::build(
        path,
        &key,
        Tokenizer::for_language(source.language().code()),
        &source,
        &|| true,
    )
    .map_err(|error| format!("the search index could not be built: {error}"))
}

fn similar(project: &Project, index: &SimilarSearch<'_>, line: &SheetLine) -> Vec<Similar> {
    let address = &line.address;
    let Ok(candidates) = index.similar(
        &line.source,
        Some((&address.sheet, address.row, address.subrow, address.column)),
        SIMILAR_CANDIDATES,
    ) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
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
                source: candidate.hit.source,
                target: target.to_owned(),
                review: unit.review_state(),
            })
        })
        .take(SIMILAR_PER_LINE)
        .collect()
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
        .filter(|line| current(&project.session, ledger.as_ref(), &line.address).is_none())
        .count();
    let mut selected: Vec<&SheetLine> = all
        .iter()
        .filter(|line| {
            options
                .rows
                .is_none_or(|(first, last)| (first..=last).contains(&line.address.row))
        })
        .filter(|line| {
            !options.untranslated
                || current(&project.session, ledger.as_ref(), &line.address).is_none()
        })
        .collect();
    let next_row = (selected.len() > options.limit).then(|| selected[options.limit].address.row);
    selected.truncate(options.limit);

    let index = if options.memory {
        search_index(project).map(|(_, index)| index)
    } else {
        None
    };
    let searcher = index.as_ref().and_then(|index| index.similar_search().ok());
    let mut domains: Vec<Domain> = Vec::new();
    let mut speakers: Vec<&str> = Vec::new();
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
            similar: searcher
                .as_ref()
                .filter(|_| now.as_ref().is_none_or(Current::replaceable))
                .map(|index| similar(project, index, line))
                .unwrap_or_default(),
            current: now,
            gender_varies,
        });
    }
    let title = quest_title(project, &options.sheet);
    let knowledge = options.knowledge.then(|| {
        Knowledge::load(project.root()).scene_slice(
            &domains,
            selected.iter().map(|line| line.source.as_str()),
            speakers.iter().copied(),
            &[options.sheet.as_str()],
        )
    });
    if out.json {
        out.json_value(&json!({
            "sheet": options.sheet,
            "quest": title,
            "total": total,
            "untranslated": untranslated_count,
            "knowledge": knowledge,
            "lines": lines,
            "nextRow": next_row,
        }));
        return Ok(());
    }

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
    let _ = write!(out.text, "# {}", options.sheet);
    if let Some(title) = &title {
        let _ = write!(out.text, " — {kind} «{title}»");
    }
    let _ = writeln!(
        out.text,
        " · {} strings · {} untranslated{}",
        fmt_count(total),
        fmt_count(untranslated_count),
        if dialogue { " · in play order" } else { "" }
    );
    let _ = writeln!(
        out.text,
        "kinds of text: {}",
        domains
            .iter()
            .map(|domain| domain.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(knowledge) = knowledge.filter(|text| !text.is_empty()) {
        let _ = writeln!(
            out.text,
            "\n## Project knowledge for these lines\n{knowledge}"
        );
    }
    let _ = writeln!(out.text, "\n## Lines");
    for line in &lines {
        let _ = write!(out.text, "\n@{} · {}", line.address, line.kind.label());
        match &line.current {
            None => out.text.push_str(" · untranslated"),
            Some(current) => {
                let _ = write!(
                    out.text,
                    " · {} {}",
                    match current.author {
                        Author::Agent => "agent's",
                        Author::Person => "person's",
                    },
                    review_word(current.review)
                );
                if !current.replaceable() {
                    out.text.push_str(" (keep)");
                }
            }
        }
        if line.gender_varies {
            out.text.push_str(" · fr/de vary by player gender");
        }
        let _ = write!(out.text, "\n  {source_language}: {}", line.source);
        for (code, text) in &line.other_languages {
            let _ = write!(out.text, "\n  {code}: {text}");
        }
        for legend in &line.macros {
            let _ = write!(out.text, "\n  macro: {legend}");
        }
        for (column, text) in &line.context {
            let _ = write!(out.text, "\n  column {column}: {text}");
        }
        if let Some(current) = &line.current {
            let _ = write!(out.text, "\n  {target}: {}", current.target);
            if let Some(note) = &current.note {
                let _ = write!(out.text, "\n  note: {note}");
            }
        }
        for similar in &line.similar {
            let _ = write!(
                out.text,
                "\n  similar ({:.0} %, {}): {} → {}",
                similar.similarity * 100.0,
                review_word(similar.review),
                similar.source,
                similar.target
            );
        }
        out.text.push('\n');
    }
    if let Some(row) = next_row {
        let last = options.rows.map_or(u32::MAX, |(_, last)| last);
        let _ = writeln!(
            out.text,
            "\n… more lines follow: `aeria read {} --rows {row}-{}`",
            options.sheet,
            if last == u32::MAX {
                String::new()
            } else {
                last.to_string()
            }
        );
    }
    Ok(())
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
fn find_source(project: &Project, options: &FindOptions) -> Result<(Vec<Found>, bool), String> {
    let index = search_index_or_build(project)?;
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
        FindIn::Source => find_source(project, options)?,
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
    let knowledge = Knowledge::load(project.root());
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
