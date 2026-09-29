//! The localization panel: how a project's localization goes, area by
//! area, and the decisions that wait for a person.
//!
//! A localization is a translation job presented as one continuous process
//! (see `docs/architecture/localization-system.md`). This module reads its
//! job store and the project knowledge and never changes a job; the one
//! write is a person's choice of a made-up name, which goes to the agent
//! knowledge like any other decision.

use aeria_ai::guidance::GlossaryEntry;
use aeria_ai::jobs::{JobStatus, JobSummary, UnitStatus};
use aeria_ai::knowledge::{self as knowledge_files, Domain, Knowledge, sheet_domain};
use serde::Serialize;

use crate::angelica::{now_unix_ms, repository_root};
use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::jobs::job_store;

type CommandResult<T> = Result<T, CommandError>;

/// Minutes over which a localization's speed is measured.
const SPEED_MINUTES: u64 = 10;
/// Findings against the knowledge listed as decisions, at most.
const KNOWLEDGE_FINDINGS: usize = 5;
/// The event kind of a finding against the knowledge.
const KNOWLEDGE_EVENT: &str = "knowledge";
/// The marker of a term's alternatives in its note (see
/// `docs/formats/knowledge-v1.md`).
const ALTERNATIVES: &str = "or: ";

/// One area of a localization: the strings of one kind of text.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Area {
    pub domain: Option<Domain>,
    pub total: u64,
    /// Written as final or drafts.
    pub done: u64,
    /// Written as needing review.
    pub flagged: u64,
    /// Rejected, failed, or skipped because they changed.
    pub problems: u64,
}

/// How a localization goes.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalizationOverview {
    /// Areas in the order the localization takes them.
    pub areas: Vec<Area>,
    /// Translations written per minute over the last minutes.
    pub per_minute: f64,
}

/// A decision that waits for a person.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Decision {
    /// A kind of text the localization covers whose style no person chose.
    Calibrate { domain: Domain },
    /// A name the localizations each made up anew, with the rendering the
    /// study chose first.
    Name {
        term: String,
        rendering: String,
        options: Vec<String>,
    },
    /// Strings the critics left open.
    Review { job_id: String, count: u64 },
    /// A finding that the knowledge itself may be wrong.
    Knowledge { job_id: String, message: String },
}

/// The area of a sheet: dialogue sheets are one area, other sheets their
/// domain; interface and general text are one area.
fn area_of(sheet: &str) -> Domain {
    match sheet_domain(sheet) {
        Domain::General => Domain::Interface,
        domain => domain,
    }
}

fn overview(counts: &[(String, UnitStatus, u64)], written: u64) -> LocalizationOverview {
    let mut areas: Vec<Area> = Vec::new();
    for (sheet, status, count) in counts {
        let domain = area_of(sheet);
        let index = if let Some(index) = areas.iter().position(|area| area.domain == Some(domain)) {
            index
        } else {
            areas.push(Area {
                domain: Some(domain),
                ..Area::default()
            });
            areas.len() - 1
        };
        let area = &mut areas[index];
        area.total += count;
        match status {
            UnitStatus::Finished | UnitStatus::Drafted => area.done += count,
            UnitStatus::Flagged => area.flagged += count,
            UnitStatus::Rejected | UnitStatus::Failed | UnitStatus::Conflict => {
                area.problems += count;
            }
            UnitStatus::Pending | UnitStatus::Running => {}
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let per_minute = written as f64 / SPEED_MINUTES as f64;
    LocalizationOverview { areas, per_minute }
}

/// The alternatives in a term's note, if a person may still choose.
fn alternatives(note: Option<&str>) -> Option<Vec<String>> {
    let note = note?;
    let start = note.find(ALTERNATIVES)? + ALTERNATIVES.len();
    let list = note[start..].split(';').next().unwrap_or_default();
    let options: Vec<String> = list
        .split('/')
        .map(str::trim)
        .filter(|option| !option.is_empty())
        .map(str::to_owned)
        .collect();
    (!options.is_empty()).then_some(options)
}

/// A term's note without its alternatives.
fn settled_note(note: Option<&str>) -> Option<String> {
    let note = note?;
    let kept: Vec<&str> = note
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty() && !part.starts_with(ALTERNATIVES))
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

fn name_decisions(knowledge: &Knowledge) -> Vec<Decision> {
    knowledge
        .terms
        .iter()
        .filter_map(|entry| {
            let others = alternatives(entry.note.as_deref())?;
            let mut options = vec![entry.translation.clone()];
            options.extend(
                others
                    .into_iter()
                    .filter(|option| *option != entry.translation),
            );
            Some(Decision::Name {
                term: entry.term.clone(),
                rendering: entry.translation.clone(),
                options,
            })
        })
        .collect()
}

/// Kinds of text a localization covers with strings still to do.
fn open_domains(counts: &[(String, UnitStatus, u64)]) -> Vec<Domain> {
    let mut domains = Vec::new();
    for (sheet, status, _) in counts {
        if !matches!(status, UnitStatus::Pending | UnitStatus::Running) {
            continue;
        }
        let found: &[Domain] = match sheet_domain(sheet) {
            Domain::Dialogue => &[Domain::Journal, Domain::Objective, Domain::Dialogue],
            Domain::General => &[Domain::Interface],
            domain => &[domain][..],
        };
        for domain in found {
            if !domains.contains(domain) {
                domains.push(*domain);
            }
        }
    }
    domains
}

fn live(job: &JobSummary) -> bool {
    matches!(job.status, JobStatus::Running | JobStatus::Paused)
}

fn decisions(app: &tauri::AppHandle) -> CommandResult<Vec<Decision>> {
    let root = repository_root(app)?;
    let store = job_store(app)?;
    let knowledge = Knowledge::load(&root);
    let jobs = store.list()?;
    let mut found = Vec::new();
    let mut domains: Vec<Domain> = Vec::new();
    for job in jobs.iter().filter(|job| live(job)) {
        for domain in open_domains(&store.sheet_counts(&job.id)?) {
            if !domains.contains(&domain) {
                domains.push(domain);
            }
        }
    }
    found.extend(
        knowledge
            .uncalibrated(&domains)
            .into_iter()
            .map(|domain| Decision::Calibrate { domain }),
    );
    found.extend(name_decisions(&knowledge));
    // Strings left open by the live localizations and the last finished one.
    let reviewed: Vec<&JobSummary> = jobs
        .iter()
        .filter(|job| live(job))
        .chain(jobs.iter().find(|job| job.status == JobStatus::Completed))
        .collect();
    for job in &reviewed {
        if job.counts.flagged > 0 {
            found.push(Decision::Review {
                job_id: job.id.clone(),
                count: job.counts.flagged,
            });
        }
    }
    for job in jobs.iter().filter(|job| live(job)) {
        let mut messages: Vec<String> = store
            .events(&job.id, 0)?
            .into_iter()
            .filter(|event| event.kind == KNOWLEDGE_EVENT)
            .map(|event| event.message)
            .collect();
        messages.dedup();
        found.extend(
            messages
                .into_iter()
                .rev()
                .take(KNOWLEDGE_FINDINGS)
                .map(|message| Decision::Knowledge {
                    job_id: job.id.clone(),
                    message,
                }),
        );
    }
    Ok(found)
}

#[tauri::command(rename_all = "camelCase")]
/// How a localization goes: its areas and its speed.
///
/// # Errors
///
/// Returns `noProjectOpen` or a job storage error.
pub async fn localization_overview(
    app: tauri::AppHandle,
    job_id: String,
) -> CommandResult<LocalizationOverview> {
    run_blocking(move || {
        let store = job_store(&app)?;
        let since = now_unix_ms().saturating_sub(SPEED_MINUTES * 60_000);
        Ok(overview(
            &store.sheet_counts(&job_id)?,
            store.written_since(&job_id, since)?,
        ))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// The decisions that wait for a person across the project's localizations.
///
/// # Errors
///
/// Returns `noProjectOpen` or a job storage error.
pub async fn localization_decisions(app: tauri::AppHandle) -> CommandResult<Vec<Decision>> {
    run_blocking(move || decisions(&app)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Settles a made-up name with the rendering a person chose: the term takes
/// it, the other options become forbidden, and its alternatives are gone.
/// Returns whether the rendering changed, so translations written with the
/// old one can be revised.
///
/// # Errors
///
/// Returns `noProjectOpen`, `localizationTermNotFound`, or
/// `localizationKnowledge` when the knowledge cannot be written.
pub async fn localization_choose_name(
    app: tauri::AppHandle,
    term: String,
    rendering: String,
) -> CommandResult<bool> {
    run_blocking(move || {
        let root = repository_root(&app)?;
        let knowledge = Knowledge::load(&root);
        let Some(entry) = knowledge
            .terms
            .iter()
            .find(|entry| entry.term == term)
            .cloned()
        else {
            return Err(CommandError::new(
                "localizationTermNotFound",
                format!("the knowledge has no agent term {term}"),
            ));
        };
        let rendering = rendering.trim().to_owned();
        let mut options = vec![entry.translation.clone()];
        options.extend(alternatives(entry.note.as_deref()).unwrap_or_default());
        let mut forbidden: Vec<String> = entry
            .forbidden
            .iter()
            .chain(options.iter())
            .filter(|option| **option != rendering)
            .cloned()
            .collect();
        forbidden.dedup();
        let changed = rendering != entry.translation;
        let settled = GlossaryEntry {
            term: entry.term,
            translation: rendering,
            note: settled_note(entry.note.as_deref()),
            forbidden,
        };
        knowledge_files::set_terms(&root, &[settled], true)
            .map_err(|message| CommandError::new("localizationKnowledge", message))?;
        Ok(changed)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_follow_the_localization_order_and_count_outcomes() {
        let counts = vec![
            ("PlaceName".to_owned(), UnitStatus::Finished, 8),
            ("PlaceName".to_owned(), UnitStatus::Pending, 2),
            ("Addon".to_owned(), UnitStatus::Flagged, 1),
            (
                "quest/000/SubFst000_00020".to_owned(),
                UnitStatus::Rejected,
                3,
            ),
        ];
        let overview = overview(&counts, 25);
        assert_eq!(
            overview
                .areas
                .iter()
                .map(|area| area.domain)
                .collect::<Vec<_>>(),
            vec![
                Some(Domain::Names),
                Some(Domain::Interface),
                Some(Domain::Dialogue)
            ]
        );
        assert_eq!((overview.areas[0].total, overview.areas[0].done), (10, 8));
        assert_eq!(overview.areas[1].flagged, 1);
        assert_eq!(overview.areas[2].problems, 3);
        assert!((overview.per_minute - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn a_term_with_alternatives_waits_and_its_note_settles() {
        assert_eq!(
            alternatives(Some("таверна; or: Утопленная чайка / Залитое горе; study")),
            Some(vec![
                "Утопленная чайка".to_owned(),
                "Залитое горе".to_owned()
            ])
        );
        assert_eq!(alternatives(Some("таверна; study")), None);
        assert_eq!(
            settled_note(Some("таверна; or: Утопленная чайка / Залитое горе; study")).as_deref(),
            Some("таверна; study")
        );
    }

    #[test]
    fn open_kinds_of_text_come_from_strings_still_to_do() {
        let counts = vec![
            ("Addon".to_owned(), UnitStatus::Pending, 1),
            ("Item".to_owned(), UnitStatus::Finished, 4),
            (
                "quest/000/SubFst000_00020".to_owned(),
                UnitStatus::Running,
                3,
            ),
        ];
        assert_eq!(
            open_domains(&counts),
            vec![
                Domain::Interface,
                Domain::Journal,
                Domain::Objective,
                Domain::Dialogue
            ]
        );
    }
}
