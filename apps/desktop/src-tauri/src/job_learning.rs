//! Learning when a translation job finishes.
//!
//! The mentor reads the job's findings, the translations people changed
//! after agents wrote them, and the findings against the knowledge itself
//! (see [`aeria_ai::learning`]). New lessons go on trial in the project's
//! `aeria-knowledge/lessons.md` and are evaluated at once on up to two units
//! the job translated: localized again with the lessons, without writing,
//! and judged against what the job wrote. The job records a `lessons` event
//! and learns once.

use std::collections::BTreeMap;

use aeria_ai::jobs::{JobStore, JobUnit, UnitStatus};
use aeria_ai::knowledge::{self as knowledge_files, Knowledge, LessonStatus, sheet_domain};
use aeria_ai::learning::{Reaction, Winner, evaluate_unit, mentor_request, parse_mentor, verdict};
use aeria_ai::localizer::{Caller as _, Step, UnitOfWork};
use aeria_ai::tools::ProjectReader;
use aeria_ai::worker::prepare_unit;

use super::{DesktopJobHost, JobCaller, JobReader, JobRun, now_unix_ms};
use crate::angelica::DesktopReader;
use crate::commands::run_blocking;

/// The event that says a job learned.
pub(super) const LEARN_EVENT: &str = "lessons";
/// Major findings a job needs before its mentor runs, when people changed
/// nothing and the knowledge had no findings.
const MIN_MAJOR_FINDINGS: usize = 5;
/// Translations jobs wrote that are compared with the project for reactions.
const REACTION_SCAN: usize = 3_000;
/// Units a job's new lessons are evaluated on.
const EVALUATION_UNITS: usize = 2;
/// Strings of one evaluation unit, at most.
const EVALUATION_LINES: usize = 40;

/// What a job has to learn from.
struct Material {
    findings: Vec<aeria_ai::jobs::Finding>,
    reactions: Vec<Reaction>,
    knowledge_findings: Vec<String>,
}

fn material(run: &JobRun) -> Material {
    let store: &JobStore = &run.store;
    let findings = store.findings(&run.job_id).unwrap_or_default();
    let mut knowledge_findings = Vec::new();
    let mut after = 0;
    while let Ok(events) = store.events(&run.job_id, after) {
        let Some(last) = events.last() else {
            break;
        };
        after = last.seq;
        knowledge_findings.extend(
            events
                .into_iter()
                .filter(|event| event.kind == "knowledge")
                .map(|event| event.message),
        );
    }
    // A translation a person changed after an agent wrote it is a reaction.
    let reader = DesktopReader {
        app: run.app.clone(),
    };
    let reactions = store
        .written(REACTION_SCAN)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|written| {
            let row = reader
                .row(
                    &written.location.sheet,
                    written.location.row,
                    written.location.subrow,
                )
                .ok()
                .flatten()?;
            let current = row
                .cells
                .into_iter()
                .find(|cell| Some(cell.column) == written.location.column)?
                .target?;
            (current.trim() != written.target.trim()).then_some(Reaction {
                source: written.source,
                agent: written.target,
                person: current,
            })
        })
        .collect();
    Material {
        findings,
        reactions,
        knowledge_findings,
    }
}

/// Units to evaluate lessons on: the sheets with the most strings the job
/// finished, quests and cutscenes first, at most [`EVALUATION_LINES`] each.
fn evaluation_units(run: &JobRun) -> Vec<(UnitOfWork, BTreeMap<usize, String>)> {
    let finished = run
        .store
        .units(&run.job_id, &[UnitStatus::Finished], 0, 10_000)
        .unwrap_or_default();
    let mut by_sheet: BTreeMap<String, Vec<JobUnit>> = BTreeMap::new();
    for unit in finished {
        by_sheet
            .entry(unit.location.sheet.clone())
            .or_default()
            .push(unit);
    }
    let mut sheets: Vec<(String, Vec<JobUnit>)> = by_sheet.into_iter().collect();
    sheets.sort_by_key(|(sheet, units)| {
        (
            sheet_domain(sheet) != aeria_ai::knowledge::Domain::Dialogue,
            std::cmp::Reverse(units.len()),
        )
    });
    let reader = JobReader { run: run.clone() };
    let facts = reader.facts().ok();
    let knowledge = Knowledge::load(&run.root);
    let host = DesktopJobHost {
        run: run.clone(),
        replace_reviewed: false,
    };
    sheets
        .into_iter()
        .take(EVALUATION_UNITS)
        .filter_map(|(_, mut units)| {
            units.truncate(EVALUATION_LINES);
            let mut prepared =
                prepare_unit(&units, &host, &reader, &knowledge, facts.as_ref(), "").unit;
            // The job's translations are the baseline; the writers must not
            // see them.
            let mut baseline = BTreeMap::new();
            for (index, line) in prepared.lines.iter_mut().enumerate() {
                if line.task.is_some() {
                    if let Some(current) = line.current.take() {
                        baseline.insert(index, current);
                    }
                    line.memory.clear();
                }
            }
            (!baseline.is_empty()).then_some((prepared, baseline))
        })
        .collect()
}

/// Learns from a finished job: lessons on trial, term corrections, and an
/// evaluation of the new lessons. Returns a summary for the job's report.
pub(super) async fn learn(run: &JobRun) -> Option<String> {
    let learned = run
        .with_store(|store, id| store.has_event(id, LEARN_EVENT))
        .await
        .unwrap_or(true);
    if learned {
        return None;
    }
    let spec = run
        .with_store(|store, id| Ok(store.summary(id)?.spec))
        .await
        .ok()?;
    let read_run = run.clone();
    let material = run_blocking(move || Ok(material(&read_run))).await.ok()?;
    let majors = material
        .findings
        .iter()
        .filter(|finding| finding.major)
        .count();
    let summary = if majors < MIN_MAJOR_FINDINGS
        && material.reactions.is_empty()
        && material.knowledge_findings.is_empty()
    {
        "nothing recurring to learn from".to_owned()
    } else {
        match mentor(run, &spec, &material).await {
            Ok(summary) => summary,
            Err(error) => format!("learning failed: {error}"),
        }
    };
    let message = summary.clone();
    let _ = run
        .with_store(move |store, id| store.add_event(id, LEARN_EVENT, &message, None))
        .await;
    Some(summary)
}

async fn mentor(
    run: &JobRun,
    spec: &aeria_ai::jobs::JobSpec,
    material: &Material,
) -> Result<String, String> {
    let mut caller = JobCaller::for_job(run, spec, 0, format!("{}-learn", run.job_id))
        .await
        .map_err(|error| error.message)?;
    caller.background(Step::Learning);
    let knowledge = Knowledge::load(&run.root);
    let target = JobReader { run: run.clone() }
        .facts()
        .ok()
        .and_then(|facts| facts.target_language)
        .unwrap_or_else(|| "the target language".to_owned());
    let request = mentor_request(
        &target,
        &material.findings,
        &material.reactions,
        &material.knowledge_findings,
        &knowledge.lessons,
    );
    let replies = caller
        .call_all(vec![request])
        .await
        .map_err(|error| error.to_string())?;
    let reply = parse_mentor(&replies[0].0, &knowledge.lessons, &run.job_id);
    for lesson in &reply.lessons {
        knowledge_files::set_lesson(&run.root, lesson)?;
    }
    let corrected = knowledge_files::set_terms(&run.root, &reply.terms, true)?;

    // Lessons on trial that were never evaluated, new ones included, are
    // evaluated together on units the job translated.
    let pending: Vec<aeria_ai::knowledge::Lesson> = Knowledge::load(&run.root)
        .lessons
        .into_iter()
        .filter(|lesson| {
            lesson.status == LessonStatus::Trial && !lesson.meta.contains_key("evaluated")
        })
        .collect();
    let mut winners = Vec::new();
    if !pending.is_empty() {
        let units_run = run.clone();
        let units = run_blocking(move || Ok(evaluation_units(&units_run)))
            .await
            .unwrap_or_default();
        for (unit, baseline) in units {
            match evaluate_unit(&caller, unit, &baseline).await {
                Ok((winner, _)) => winners.push(winner),
                Err(error) => return Err(format!("the evaluation failed: {error}")),
            }
        }
    }
    let decided = verdict(&winners);
    let date = now_unix_ms().to_string();
    let wins = winners
        .iter()
        .filter(|winner| **winner == Winner::Candidate)
        .count();
    for lesson in winners
        .is_empty()
        .then(Vec::new)
        .unwrap_or_else(|| pending.clone())
    {
        let mut lesson = lesson;
        if let Some(status) = decided {
            lesson.status = status;
        }
        lesson.meta.insert("evaluated".to_owned(), date.clone());
        lesson.meta.insert(
            "effect".to_owned(),
            format!("won {wins} of {} comparisons", winners.len()),
        );
        knowledge_files::set_lesson(&run.root, &lesson)?;
    }
    let usage = caller.spent();
    let _ = run
        .with_store(move |store, id| store.add_usage(id, usage))
        .await;
    let outcome = match decided {
        Some(LessonStatus::Active) => "kept",
        Some(LessonStatus::Dropped) => "dropped",
        _ => "left on trial",
    };
    Ok(format!(
        "{} new lesson(s); {} lesson(s) on trial {outcome} after {} comparison(s); {} term(s) corrected; from {} finding(s) and {} change(s) by people",
        reply.lessons.len(),
        pending.len(),
        winners.len(),
        corrected.len(),
        material.findings.len(),
        material.reactions.len()
    ))
}
