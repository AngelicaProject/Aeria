//! Study for translation jobs.
//!
//! Before a job's first chunk, its runner studies the style of every text
//! domain the scope has and the agent knowledge lacks; a job does this once
//! and records a `study` event. Each chunk then studies, at the same time,
//! its own terms and the speakers of its scene who have no profile, before
//! its contract (see [`aeria_ai::study`]), so a job starts writing within
//! minutes whatever the number of its characters. Knowledge is written to
//! the agent layer of the project (see [`aeria_ai::knowledge`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use aeria_ai::dialogue::LineRole;
use aeria_ai::guidance::GlossaryEntry;
use aeria_ai::knowledge::{
    self as knowledge_files, Domain, Knowledge, KnowledgeFile, Section, sheet_domain,
};
use aeria_ai::localizer::{LineKind, UnitOfWork};
use aeria_ai::study::{
    CHARACTER_SAMPLES, KnowledgeHost, STYLE_SAMPLES, Sample, study_characters, study_style,
};
use aeria_ai::tools::ProjectReader;

use super::{JobCaller, JobRun, futures_join};
use crate::angelica::DesktopReader;
use crate::commands::run_blocking;
use crate::search::prepare_source_index;

/// Most speakers one chunk studies before its contract.
const MAX_UNIT_CHARACTERS: usize = 6;
/// Lines a speaker needs in a chunk's scene to be studied.
const MIN_SPEAKER_LINES: usize = 2;
/// Dialogue sheets read for the samples of the dialogue domains.
const STYLE_SHEETS: usize = 40;
/// Sheets of another domain read for its samples.
const DOMAIN_SHEETS: usize = 10;
/// The event that says a job studied its scope.
pub(super) const STUDY_EVENT: &str = "study";

/// Project knowledge as the job's researchers see it.
pub(super) struct DesktopKnowledge {
    pub(super) root: PathBuf,
}

/// Speakers a running job's chunks are studying or studied, so two chunks
/// never study the same speaker. Kept in memory while the runner runs.
pub(super) type StudiedSpeakers = Mutex<BTreeSet<String>>;

impl KnowledgeHost for DesktopKnowledge {
    fn knowledge(&self) -> Knowledge {
        Knowledge::load(&self.root)
    }

    fn set_terms(&self, entries: &[GlossaryEntry]) -> Result<Vec<String>, String> {
        knowledge_files::set_terms(&self.root, entries, false)
    }

    fn set_characters(&self, profiles: &[(Vec<String>, String)]) -> Result<usize, String> {
        knowledge_files::set_characters(&self.root, profiles)
    }

    fn set_section(&self, file: KnowledgeFile, section: Section) -> Result<(), String> {
        knowledge_files::set_section(&self.root, file, section)
    }
}

/// What a job studies before its first chunk.
struct StudyPlan {
    target: String,
    knowledge: String,
    styles: Vec<(Domain, Vec<Sample>)>,
}

fn evidence(
    reader: &DesktopReader,
    sheet: &str,
    row: u32,
    subrow: u16,
    column: u32,
) -> Vec<(String, String)> {
    reader
        .other_languages(sheet, row, subrow, column)
        .map(|languages| {
            languages
                .into_iter()
                .filter_map(|(code, text)| {
                    text.filter(|text| !text.trim().is_empty())
                        .map(|text| (code, text))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Up to `count` items spread evenly over `items`.
fn spread<T: Clone>(items: &[T], count: usize) -> Vec<T> {
    if items.len() <= count {
        return items.to_vec();
    }
    (0..count)
        .map(|index| items[index * items.len() / count].clone())
        .collect()
}

fn line_domain(role: &LineRole) -> Domain {
    match role {
        LineRole::Journal => Domain::Journal,
        LineRole::Objective => Domain::Objective,
        LineRole::Speech(speaker) if speaker.starts_with("SYSTEM") => Domain::System,
        LineRole::Speech(_) | LineRole::Other => Domain::Dialogue,
    }
}

/// Whether a speaker label names a character worth a profile: not system
/// text, not a player choice such as `Q1` or `A2`, and not only digits.
fn is_character(label: &str) -> bool {
    let choice = label.len() <= 3
        && label.starts_with(['Q', 'A'])
        && label[1..].chars().all(|c| c.is_ascii_digit());
    !label.starts_with("SYSTEM") && !choice && label.chars().any(|c| c.is_ascii_alphabetic())
}

/// Candidate samples of the dialogue domains, by address, from a spread of
/// the scope's dialogue sheets.
fn dialogue_candidates(
    reader: &DesktopReader,
    sheets: &[&String],
) -> BTreeMap<Domain, Vec<Sample>> {
    let mut by_domain: BTreeMap<Domain, Vec<Sample>> = BTreeMap::new();
    for sheet in spread(sheets, STYLE_SHEETS) {
        let Ok(Some(dialogue)) = reader.dialogue(sheet) else {
            continue;
        };
        for line in &dialogue.lines {
            by_domain
                .entry(line_domain(&line.role))
                .or_default()
                .push(Sample {
                    label: match &line.role {
                        LineRole::Speech(speaker) => speaker.clone(),
                        role => format!("[{}]", line_domain(role).as_str()),
                    },
                    source: format!("{}:{}:{}:{}", sheet, line.row, line.subrow, line.column),
                    evidence: Vec::new(),
                });
        }
    }
    by_domain
}

/// Candidate samples of other domains, by address: the first rows of a few
/// of their sheets.
fn domain_candidates(
    reader: &DesktopReader,
    sheets: &[&String],
    by_domain: &mut BTreeMap<Domain, Vec<Sample>>,
) {
    let mut grouped: BTreeMap<Domain, Vec<&String>> = BTreeMap::new();
    for sheet in sheets {
        grouped.entry(sheet_domain(sheet)).or_default().push(sheet);
    }
    for (domain, sheets) in grouped {
        for sheet in spread(&sheets, DOMAIN_SHEETS) {
            let Ok(page) = reader.rows(sheet, None, 20) else {
                continue;
            };
            for row in page.rows {
                for cell in row.cells {
                    by_domain.entry(domain).or_default().push(Sample {
                        label: format!("[{sheet}]"),
                        source: format!("{}:{}:{}:{}", sheet, row.row, row.subrow, cell.column),
                        evidence: Vec::new(),
                    });
                }
            }
        }
    }
}

/// A sample read from its address, `sheet:row:subrow:column`.
fn read_sample(reader: &DesktopReader, address: &str, label: &str) -> Option<Sample> {
    let mut parts = address.rsplitn(4, ':');
    let (column, subrow, row, sheet) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let (column, subrow, row): (u32, u16, u32) = (
        column.parse().ok()?,
        subrow.parse().ok()?,
        row.parse().ok()?,
    );
    let source = reader
        .row(sheet, row, subrow)
        .ok()
        .flatten()?
        .cells
        .into_iter()
        .find(|cell| cell.column == column)?
        .source;
    Some(Sample {
        label: label.to_owned(),
        source,
        evidence: evidence(reader, sheet, row, subrow, column),
    })
}

/// A speaker's lines sampled evenly across the game, with their total.
fn speaker_samples(reader: &DesktopReader, label: &str) -> Option<(usize, Vec<Sample>)> {
    let (total, _) = reader.speaker_lines(label, 0, 0).ok()?;
    let count = CHARACTER_SAMPLES.min(total);
    let lines = (0..count)
        .filter_map(|index| {
            let (_, found) = reader
                .speaker_lines(label, index * total / count.max(1), 1)
                .ok()?;
            let location = found.into_iter().next()?;
            let row = reader
                .row(&location.sheet, location.row, location.subrow)
                .ok()
                .flatten()?;
            let cell = row.cells.into_iter().next()?;
            Some(Sample {
                label: String::new(),
                evidence: evidence(
                    reader,
                    &location.sheet,
                    location.row,
                    location.subrow,
                    cell.column,
                ),
                source: cell.source,
            })
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then_some((total, lines))
}

fn plan(app: &tauri::AppHandle, root: &std::path::Path, sheets: &[String]) -> StudyPlan {
    let reader = DesktopReader { app: app.clone() };
    let knowledge = Knowledge::load(root);
    let target = reader
        .facts()
        .ok()
        .and_then(|facts| facts.target_language)
        .unwrap_or_else(|| "the target language".to_owned());
    let (dialogue_sheets, other_sheets): (Vec<&String>, Vec<&String>) = sheets
        .iter()
        .partition(|sheet| sheet_domain(sheet) == Domain::Dialogue);
    let mut by_domain = dialogue_candidates(&reader, &dialogue_sheets);
    domain_candidates(&reader, &other_sheets, &mut by_domain);

    let styles = std::iter::once(Domain::General)
        .chain(by_domain.keys().copied())
        .filter(|domain| knowledge.style(*domain).is_none())
        .map(|domain| {
            let pool = if domain == Domain::General {
                by_domain.values().flatten().cloned().collect::<Vec<_>>()
            } else {
                by_domain.get(&domain).cloned().unwrap_or_default()
            };
            let chosen = spread(&pool, STYLE_SAMPLES)
                .iter()
                .filter_map(|candidate| read_sample(&reader, &candidate.source, &candidate.label))
                .collect();
            (domain, chosen)
        })
        .collect();

    StudyPlan {
        knowledge: knowledge.prompt_for(&Domain::ALL, std::iter::empty(), std::iter::empty(), &[]),
        target,
        styles,
    }
}

/// Studies the speakers of a chunk's scene who have at least
/// [`MIN_SPEAKER_LINES`] lines in it, no profile, and no other chunk
/// studying them, the most lines first and at most [`MAX_UNIT_CHARACTERS`].
/// Returns how many profiles were written.
///
/// # Errors
///
/// Returns the provider failure of the study.
pub(super) async fn study_unit_characters(
    run: &JobRun,
    caller: &JobCaller,
    unit: &UnitOfWork,
) -> Result<usize, aeria_ai::client::ProviderError> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for line in &unit.lines {
        if let LineKind::Speech(speaker) = &line.kind {
            *counts.entry(speaker.clone()).or_default() += 1;
        }
    }
    let knowledge = {
        let root = run.root.clone();
        run_blocking(move || Ok(Knowledge::load(&root)))
            .await
            .unwrap_or_else(|_| Knowledge::load(&run.root))
    };
    let mut wanted: Vec<(String, usize)> = counts
        .into_iter()
        .filter(|(label, count)| {
            *count >= MIN_SPEAKER_LINES
                && is_character(label)
                && knowledge.character(label).is_none()
        })
        .collect();
    wanted.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let claimed: Vec<String> = {
        let mut studied = run.studied.lock().unwrap_or_else(PoisonError::into_inner);
        wanted
            .into_iter()
            .map(|(label, _)| label)
            .filter(|label| studied.insert(label.clone()))
            .take(MAX_UNIT_CHARACTERS)
            .collect()
    };
    if claimed.is_empty() {
        return Ok(0);
    }
    let app = run.app.clone();
    let speakers = run_blocking(move || {
        let reader = DesktopReader { app };
        Ok(claimed
            .into_iter()
            .filter_map(|label| {
                speaker_samples(&reader, &label).map(|(total, lines)| (label, total, lines))
            })
            .collect::<Vec<_>>())
    })
    .await
    .unwrap_or_default();
    let host = DesktopKnowledge {
        root: run.root.clone(),
    };
    let text = knowledge.prompt_for(&Domain::ALL, std::iter::empty(), std::iter::empty(), &[]);
    study_characters(caller, &host, &unit.target_language, &text, &speakers).await
}

/// Studies a job's scope once, before its first chunk. Returns why the job
/// must pause, if it must.
pub(super) async fn study_scope(run: &JobRun) -> Result<(), String> {
    let studied = run
        .with_store(|store, id| store.has_event(id, STUDY_EVENT))
        .await
        .map_err(|error| error.message)?;
    if studied {
        return Ok(());
    }
    let spec = run
        .with_store(|store, id| Ok(store.summary(id)?.spec))
        .await
        .map_err(|error| error.message)?;
    // The source index gives chunks the translations of similar strings;
    // it builds while the job runs.
    prepare_source_index(&run.app);
    let mut caller = JobCaller::for_job(run, &spec, 0, format!("{}-study", run.job_id))
        .await
        .map_err(|error| error.message)?;
    caller.background(aeria_ai::localizer::Step::Study);
    let sheets = run
        .with_store(aeria_ai::jobs::JobStore::sheets)
        .await
        .map_err(|error| error.message)?;
    let (app, root) = (run.app.clone(), run.root.clone());
    let study = run_blocking(move || Ok(plan(&app, &root, &sheets)))
        .await
        .map_err(|error| error.message)?;
    let host = DesktopKnowledge {
        root: run.root.clone(),
    };
    let styles = futures_join(
        study
            .styles
            .iter()
            .map(|(domain, samples)| {
                Box::pin(study_style(
                    &caller,
                    &host,
                    *domain,
                    &study.target,
                    &study.knowledge,
                    samples,
                ))
                    as std::pin::Pin<Box<dyn std::future::Future<Output = _> + Send + '_>>
            })
            .collect(),
    )
    .await;
    let mut domains = 0;
    for result in styles {
        match result {
            Ok(true) => domains += 1,
            Ok(false) => {}
            Err(error) => return Err(format!("the study of the job's scope failed: {error}")),
        }
    }
    let usage = caller.spent();
    let message = format!("studied the style of {domains} text domains");
    run.with_store(move |store, id| {
        store.add_usage(id, usage)?;
        store.add_event(id, STUDY_EVENT, &message, None)
    })
    .await
    .map_err(|error| error.message)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_and_system_text_are_not_characters() {
        assert!(is_character("URIANGER"));
        assert!(is_character("AMHGARANJY_GEVA"));
        assert!(!is_character("SYSTEM"));
        assert!(!is_character("SYSTEM_NONE_VOICE"));
        assert!(!is_character("Q1"));
        assert!(!is_character("A12"));
        assert!(is_character("A1B"));
    }

    #[test]
    fn samples_spread_over_the_whole_list() {
        let items: Vec<u32> = (0..100).collect();
        assert_eq!(spread(&items, 4), vec![0, 25, 50, 75]);
        assert_eq!(spread(&items[..3], 4), vec![0, 1, 2]);
    }
}
