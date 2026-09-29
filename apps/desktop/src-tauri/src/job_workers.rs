//! Live activity of a running job's workers.
//!
//! Each lane of a job runner reports what it is doing: which chunk it
//! localizes, which step of the localizer it is on, how many requests it
//! waits for, and when it last received anything from the provider. The
//! board lives only while the runner runs and is never persisted; it exists
//! so the user can see that workers are alive and what they are working on.

use std::sync::{Mutex, MutexGuard, PoisonError};

use aeria_ai::localizer::Step;
use serde::Serialize;

use crate::angelica::now_unix_ms;

/// Steps of the localizer, for "step N of M".
pub const LOCALIZER_STEPS: u32 = 6;

/// What one lane is doing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkerPhase {
    /// Claiming the next chunk.
    Idle,
    /// Loading the chunk's strings and context.
    Preparing,
    /// Requests were sent; nothing has streamed back yet.
    Waiting,
    /// The model streams its reasoning.
    Reasoning,
    /// The model streams its reply.
    Writing,
    /// The chunk's translations and outcomes are being recorded.
    Recording,
    /// Waiting before the next chunk after a provider failure.
    Backoff,
    /// The lane stopped; the job paused or ran out of chunks.
    Stopped,
}

/// The localizer step a lane is on.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkerStep {
    /// The study of the job's scope, before its first chunk.
    Study,
    /// Learning from the finished job.
    Learning,
    Terms,
    Contract,
    Writing,
    Reviewing,
    Fixing,
    Rechecking,
}

impl From<Step> for WorkerStep {
    fn from(step: Step) -> Self {
        match step {
            Step::Study => Self::Study,
            Step::Learning => Self::Learning,
            Step::Terms => Self::Terms,
            Step::Contract => Self::Contract,
            Step::Writing => Self::Writing,
            Step::Reviewing => Self::Reviewing,
            Step::Fixing => Self::Fixing,
            Step::Rechecking => Self::Rechecking,
        }
    }
}

impl WorkerStep {
    const fn number(self) -> u32 {
        match self {
            Self::Study | Self::Learning => 0,
            Self::Terms => 1,
            Self::Contract => 2,
            Self::Writing => 3,
            Self::Reviewing => 4,
            Self::Fixing => 5,
            Self::Rechecking => 6,
        }
    }
}

/// One lane's current activity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerActivity {
    /// One-based lane number.
    pub lane: u32,
    pub phase: WorkerPhase,
    pub chunk: Option<u64>,
    pub sheet: Option<String>,
    /// The first and last source row of the current chunk.
    pub first_row: Option<u32>,
    pub last_row: Option<u32>,
    /// Strings in the current chunk.
    pub units: u32,
    /// Strings of the current chunk already written, skipped, or failed.
    /// The localizer writes a chunk's translations together at its end.
    pub finished_units: u32,
    /// The localizer step, while a chunk runs.
    pub step: Option<WorkerStep>,
    /// The step's number, from 1, and the number of steps.
    pub round: u32,
    pub max_rounds: u32,
    /// Requests of the current step that have not answered yet.
    pub requests: u32,
    /// Tokens the current chunk used so far.
    pub chunk_tokens: u64,
    /// Chunks this lane finished since the runner started.
    pub chunks_done: u32,
    pub phase_started_unix_ms: u64,
    /// When the lane last changed or received anything from the provider.
    pub last_activity_unix_ms: u64,
    /// When a lane in backoff tries again.
    pub retry_at_unix_ms: Option<u64>,
    /// The last provider failure of this lane.
    pub last_error: Option<String>,
}

impl WorkerActivity {
    fn new(lane: u32, now: u64) -> Self {
        Self {
            lane,
            phase: WorkerPhase::Idle,
            chunk: None,
            sheet: None,
            first_row: None,
            last_row: None,
            units: 0,
            finished_units: 0,
            step: None,
            round: 0,
            max_rounds: 0,
            requests: 0,
            chunk_tokens: 0,
            chunks_done: 0,
            phase_started_unix_ms: now,
            last_activity_unix_ms: now,
            retry_at_unix_ms: None,
            last_error: None,
        }
    }

    fn set_phase(&mut self, phase: WorkerPhase, now: u64) {
        if self.phase != phase {
            self.phase = phase;
            self.phase_started_unix_ms = now;
        }
        if phase != WorkerPhase::Backoff {
            self.retry_at_unix_ms = None;
        }
    }

    /// Starts a claimed chunk covering `rows`, its first and last row.
    pub fn start_chunk(&mut self, chunk: u64, sheet: &str, rows: (u32, u32), units: u32, now: u64) {
        self.set_phase(WorkerPhase::Preparing, now);
        self.chunk = Some(chunk);
        self.sheet = Some(sheet.to_owned());
        self.first_row = Some(rows.0);
        self.last_row = Some(rows.1);
        self.units = units;
        self.finished_units = 0;
        self.step = None;
        self.round = 0;
        self.max_rounds = LOCALIZER_STEPS;
        self.requests = 0;
        self.chunk_tokens = 0;
    }

    /// Moves to a localizer step.
    pub fn set_step(&mut self, step: WorkerStep, now: u64) {
        self.step = Some(step);
        self.round = step.number();
        // The step starts again, even when its phase does not change.
        self.phase_started_unix_ms = now;
    }

    /// Sends `count` requests of the current step.
    pub fn send(&mut self, count: u32, now: u64) {
        self.requests += count;
        self.set_phase(WorkerPhase::Waiting, now);
    }

    /// Follows a streamed delta of any of the step's requests.
    pub fn stream(&mut self, reasoning: bool, now: u64) {
        let phase = if reasoning {
            WorkerPhase::Reasoning
        } else {
            WorkerPhase::Writing
        };
        self.set_phase(phase, now);
    }

    /// One request answered, with the tokens it used.
    pub fn answered(&mut self, tokens: u64, now: u64) {
        self.requests = self.requests.saturating_sub(1);
        self.chunk_tokens += tokens;
        if self.requests == 0 {
            self.set_phase(WorkerPhase::Waiting, now);
        }
    }

    /// Waits before the next chunk after a provider failure.
    pub fn set_backoff(&mut self, retry_at: u64, error: String, now: u64) {
        self.set_phase(WorkerPhase::Backoff, now);
        self.retry_at_unix_ms = Some(retry_at);
        self.last_error = Some(error);
        self.step = None;
        self.requests = 0;
    }
}

/// The lanes of one running job.
#[derive(Debug, Default)]
pub struct WorkerBoard {
    lanes: Mutex<Vec<WorkerActivity>>,
}

impl WorkerBoard {
    fn lock(&self) -> MutexGuard<'_, Vec<WorkerActivity>> {
        self.lanes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Resets the board to `count` idle lanes.
    pub fn reset(&self, count: u32) {
        let now = now_unix_ms();
        *self.lock() = (1..=count)
            .map(|lane| WorkerActivity::new(lane, now))
            .collect();
    }

    /// Adds idle lanes up to `count`, keeping the existing ones.
    pub fn ensure(&self, count: u32) {
        let now = now_unix_ms();
        let mut lanes = self.lock();
        let existing = u32::try_from(lanes.len()).unwrap_or(u32::MAX);
        lanes.extend((existing + 1..=count).map(|lane| WorkerActivity::new(lane, now)));
    }

    /// Updates one lane (zero-based) and marks it active now.
    pub fn update(&self, lane: usize, change: impl FnOnce(&mut WorkerActivity, u64)) {
        let now = now_unix_ms();
        if let Some(activity) = self.lock().get_mut(lane) {
            change(activity, now);
            activity.last_activity_unix_ms = now;
        }
    }

    /// Sets one lane's phase.
    pub fn set_phase(&self, lane: usize, phase: WorkerPhase) {
        self.update(lane, |activity, now| activity.set_phase(phase, now));
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<WorkerActivity> {
        self.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lane_follows_its_chunk_through_the_steps() {
        let mut lane = WorkerActivity::new(1, 0);
        lane.start_chunk(4, "quest/000/Test", (3, 90), 42, 10);
        assert_eq!(lane.phase, WorkerPhase::Preparing);
        assert_eq!(
            (lane.units, lane.max_rounds, lane.step),
            (42, LOCALIZER_STEPS, None)
        );

        lane.set_step(WorkerStep::Writing, 20);
        lane.send(3, 20);
        assert_eq!(
            (lane.round, lane.requests, lane.phase),
            (3, 3, WorkerPhase::Waiting)
        );
        lane.stream(true, 25);
        assert_eq!(lane.phase, WorkerPhase::Reasoning);
        lane.stream(false, 30);
        assert_eq!(
            (lane.phase, lane.phase_started_unix_ms),
            (WorkerPhase::Writing, 30)
        );
        lane.answered(100, 40);
        lane.answered(50, 41);
        assert_eq!((lane.requests, lane.phase), (1, WorkerPhase::Writing));
        lane.answered(50, 42);
        assert_eq!(
            (lane.requests, lane.phase, lane.chunk_tokens),
            (0, WorkerPhase::Waiting, 200)
        );

        lane.set_backoff(1_000, "rate limited".to_owned(), 50);
        assert_eq!(
            (lane.phase, lane.step, lane.retry_at_unix_ms),
            (WorkerPhase::Backoff, None, Some(1_000))
        );
        lane.start_chunk(5, "Item", (1, 1), 1, 60);
        assert_eq!((lane.chunk_tokens, lane.retry_at_unix_ms), (0, None));
    }

    #[test]
    fn the_board_marks_updates_as_activity() {
        let board = WorkerBoard::default();
        board.reset(2);
        board.ensure(3);
        let before = board.snapshot()[1].last_activity_unix_ms;
        board.update(1, |lane, now| lane.set_step(WorkerStep::Contract, now));
        let lanes = board.snapshot();
        assert_eq!(lanes.len(), 3);
        assert_eq!(lanes[1].step, Some(WorkerStep::Contract));
        assert!(lanes[1].last_activity_unix_ms >= before);
        board.set_phase(0, WorkerPhase::Stopped);
        assert_eq!(board.snapshot()[0].phase, WorkerPhase::Stopped);
    }
}
