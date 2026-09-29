//! Learning during and after a translation job.
//!
//! Every [`LEARN_EVERY_CHUNKS`] finished chunks, and when the job
//! completes, the mentor reads what is new since the job last learned: the
//! job's findings, the translations people changed after agents wrote them,
//! and the findings against the knowledge itself (see
//! [`aeria_ai::learning`]). New lessons go on trial in the project's
//! `aeria-knowledge/lessons.md`, so the chunks that follow use them, and
//! every lesson still on trial is evaluated on up to two units the job
//! translated: localized again with the lessons, without writing, and judged
//! against what the job wrote. Each learning records a `lessons` event.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::PoisonError;

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
/// Finished chunks between two learnings of a running job.
const LEARN_EVERY_CHUNKS: u64 = 40;
/// Units a job's new lessons are evaluated on.
const EVALUATION_UNITS: usize = 2;
/// Strings of one evaluation unit, at most.
const EVALUATION_LINES: usize = 40;

/// What a running job has already learned from, so each learning reads
/// only what is new. Kept in memory while the runner runs.
#[derive(Debug, Default)]
pub(super) struct LearningState {
    /// Findings already read.
    findings_seen: usize,
    /// The last job event already read.
    events_seen: u64,
    /// Reactions already read, by string and the person's text.
    reactions_seen: BTreeSet<(String, String)>,
    /// Finished chunks when the job last learned.
    chunks_at: u64,
    /// A learning is running.
    running: bool,
}

/// What a job has to learn from.
struct Material {
    findings: Vec<aeria_ai::jobs::Finding>,
    reactions: Vec<Reaction>,
    knowledge_findings: Vec<String>,
    /// What to mark as read once the material was used.
    findings_total: usize,
    last_event: u64,
    reaction_keys: Vec<(String, String)>,
}

fn material(
    run: &JobRun,
    findings_seen: usize,
    events_seen: u64,
    reactions_seen: &BTreeSet<(String, String)>,
) -> Material {
    let store: &JobStore = &run.store;
    let all = store.findings(&run.job_id).unwrap_or_default();
    let findings_total = all.len();
    let findings = all.into_iter().skip(findings_seen).collect();
    let mut knowledge_findings = Vec::new();
    let mut after = events_seen;
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
    let mut reaction_keys = Vec::new();
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
            let key = (
                serde_json::to_string(&written.location).unwrap_or_default(),
                current.clone(),
            );
            if current.trim() == written.target.trim() || reactions_seen.contains(&key) {
                return None;
            }
            reaction_keys.push(key);
            Some(Reaction {
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
        findings_total,
        last_event: after,
        reaction_keys,
    }
}

/// Learns when a running job finished [`LEARN_EVERY_CHUNKS`] chunks since
/// it last learned; the lane that finds it due learns before its next chunk.
pub(super) async fn learn_if_due(run: &JobRun, lane: usize) {
    let finished = run
        .with_store(|store, id| Ok(store.summary(id)?.finished_chunks))
        .await
        .unwrap_or(0);
    let due = {
        let state = run.learning.lock().unwrap_or_else(PoisonError::into_inner);
        !state.running && finished >= state.chunks_at + LEARN_EVERY_CHUNKS
    };
    if due {
        let _ = learn(run, lane).await;
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

/// Learns from what is new in a job: lessons on trial, term corrections, and
/// an evaluation of the lessons on trial. Returns a summary for the job's
/// report; `None` when another learning of the job is running.
pub(super) async fn learn(run: &JobRun, lane: usize) -> Option<String> {
    let (findings_seen, events_seen, reactions_seen) = {
        let mut state = run.learning.lock().unwrap_or_else(PoisonError::into_inner);
        if state.running {
            return None;
        }
        state.running = true;
        (
            state.findings_seen,
            state.events_seen,
            state.reactions_seen.clone(),
        )
    };
    let summary = learn_new(run, lane, findings_seen, events_seen, reactions_seen).await;
    run.learning
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .running = false;
    summary
}

async fn learn_new(
    run: &JobRun,
    lane: usize,
    findings_seen: usize,
    events_seen: u64,
    reactions_seen: BTreeSet<(String, String)>,
) -> Option<String> {
    let (spec, finished) = run
        .with_store(|store, id| {
            let job = store.summary(id)?;
            Ok((job.spec, job.finished_chunks))
        })
        .await
        .ok()?;
    let read_run = run.clone();
    let material = run_blocking(move || {
        Ok(material(
            &read_run,
            findings_seen,
            events_seen,
            &reactions_seen,
        ))
    })
    .await
    .ok()?;
    let majors = material
        .findings
        .iter()
        .filter(|finding| finding.major)
        .count();
    let summary = if majors < MIN_MAJOR_FINDINGS
        && material.reactions.is_empty()
        && material.knowledge_findings.is_empty()
    {
        "nothing new and recurring to learn from".to_owned()
    } else {
        match mentor(run, &spec, &material, lane).await {
            Ok(summary) => summary,
            Err(error) => format!("learning failed: {error}"),
        }
    };
    {
        let mut state = run.learning.lock().unwrap_or_else(PoisonError::into_inner);
        state.findings_seen = material.findings_total;
        state.events_seen = material.last_event;
        state
            .reactions_seen
            .extend(material.reaction_keys.iter().cloned());
        state.chunks_at = finished;
    }
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
    lane: usize,
) -> Result<String, String> {
    let mut caller = JobCaller::for_job(run, spec, lane, format!("{}-learn", run.job_id))
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
