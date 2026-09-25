//! Live activity of a running job's workers.
//!
//! Each lane of a job runner reports what it is doing: which chunk it
//! translates, which model response it waits for, and when it last received
//! anything from the provider. The board lives only while the runner runs and
//! is never persisted; it exists so the user can see that workers are alive.

use std::sync::{Mutex, MutexGuard, PoisonError};

use aeria_ai::agent::AgentEvent;
use serde::Serialize;

use crate::angelica::now_unix_ms;

/// What one lane is doing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkerPhase {
    /// Claiming the next chunk.
    Idle,
    /// Loading the chunk's strings and context.
    Preparing,
    /// A request was sent; nothing has streamed back yet.
    Waiting,
    /// The model streams its reasoning.
    Reasoning,
    /// The model streams its reply or tool-call arguments.
    Writing,
    /// A tool call runs, such as writing the submitted translations.
    Tool,
    /// The chunk's outcomes are being recorded.
    Recording,
    /// Waiting before the next chunk after a provider failure.
    Backoff,
    /// The lane stopped; the job paused or ran out of chunks.
    Stopped,
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
    /// Strings in the current chunk.
    pub units: u32,
    /// Strings of the current chunk already written, skipped, or failed.
    pub finished_units: u32,
    /// The model response the worker is on, starting at 1.
    pub round: u32,
    pub max_rounds: u32,
    /// The tool that runs in the `tool` phase.
    pub tool: Option<String>,
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
    /// A response finished and the next request has not been counted yet.
    #[serde(skip)]
    response_finished: bool,
}

impl WorkerActivity {
    fn new(lane: u32, now: u64) -> Self {
        Self {
            lane,
            phase: WorkerPhase::Idle,
            chunk: None,
            sheet: None,
            units: 0,
            finished_units: 0,
            round: 0,
            max_rounds: 0,
            tool: None,
            chunk_tokens: 0,
            chunks_done: 0,
            phase_started_unix_ms: now,
            last_activity_unix_ms: now,
            retry_at_unix_ms: None,
            last_error: None,
            response_finished: false,
        }
    }

    fn set_phase(&mut self, phase: WorkerPhase, now: u64) {
        if self.phase != phase {
            self.phase = phase;
            self.phase_started_unix_ms = now;
        }
        if phase != WorkerPhase::Tool {
            self.tool = None;
        }
        if phase != WorkerPhase::Backoff {
            self.retry_at_unix_ms = None;
        }
    }

    /// Starts a claimed chunk.
    pub fn start_chunk(&mut self, chunk: u64, sheet: &str, units: u32, max_rounds: u32, now: u64) {
        self.set_phase(WorkerPhase::Preparing, now);
        self.chunk = Some(chunk);
        self.sheet = Some(sheet.to_owned());
        self.units = units;
        self.finished_units = 0;
        self.round = 0;
        self.max_rounds = max_rounds;
        self.chunk_tokens = 0;
        self.response_finished = false;
    }

    /// Marks the first request of a chunk as sent.
    pub fn first_request(&mut self, now: u64) {
        self.set_phase(WorkerPhase::Waiting, now);
        self.round = 1;
    }

    /// Waits before the next chunk after a provider failure.
    pub fn set_backoff(&mut self, retry_at: u64, error: String, now: u64) {
        self.set_phase(WorkerPhase::Backoff, now);
        self.retry_at_unix_ms = Some(retry_at);
        self.last_error = Some(error);
    }

    /// Follows one event of the worker's turn.
    pub fn apply(&mut self, event: &AgentEvent, now: u64) {
        match event {
            AgentEvent::ReasoningDelta { .. } => self.set_phase(WorkerPhase::Reasoning, now),
            AgentEvent::TextDelta { .. } | AgentEvent::ToolArgumentsDelta { .. } => {
                self.set_phase(WorkerPhase::Writing, now);
            }
            AgentEvent::ResponseFinished => self.response_finished = true,
            AgentEvent::Usage { usage } => {
                self.chunk_tokens += usage.prompt_tokens + usage.completion_tokens;
            }
            AgentEvent::ToolStarted { name, .. } => {
                self.set_phase(WorkerPhase::Tool, now);
                self.tool = Some(name.clone());
            }
            // The next request follows the tool results; a later tool of
            // the same response replaces this phase at once.
            AgentEvent::ToolFinished { .. } => {
                self.set_phase(WorkerPhase::Waiting, now);
                if std::mem::take(&mut self.response_finished) {
                    self.round = (self.round + 1).min(self.max_rounds);
                }
            }
        }
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
    use aeria_ai::chat::Usage;

    use super::*;

    #[test]
    fn a_lane_follows_its_chunk_through_the_turn() {
        let mut lane = WorkerActivity::new(1, 0);
        lane.start_chunk(7, "Addon", 30, 8, 10);
        assert_eq!(lane.phase, WorkerPhase::Preparing);
        lane.first_request(20);
        assert_eq!((lane.phase, lane.round), (WorkerPhase::Waiting, 1));

        lane.apply(
            &AgentEvent::ReasoningDelta {
                text: "…".to_owned(),
            },
            30,
        );
        assert_eq!(
            (lane.phase, lane.phase_started_unix_ms),
            (WorkerPhase::Reasoning, 30)
        );
        lane.apply(
            &AgentEvent::ReasoningDelta {
                text: "…".to_owned(),
            },
            40,
        );
        assert_eq!(
            lane.phase_started_unix_ms, 30,
            "the same phase keeps its start"
        );
        lane.apply(&AgentEvent::ToolArgumentsDelta { chars: 12 }, 50);
        assert_eq!(lane.phase, WorkerPhase::Writing);
        lane.apply(
            &AgentEvent::Usage {
                usage: Usage {
                    prompt_tokens: 900,
                    completion_tokens: 100,
                },
            },
            60,
        );
        assert_eq!(lane.chunk_tokens, 1000);
        lane.apply(&AgentEvent::ResponseFinished, 65);

        lane.apply(
            &AgentEvent::ToolStarted {
                id: "c1".to_owned(),
                name: "submit_translations".to_owned(),
                arguments: "{}".to_owned(),
            },
            70,
        );
        assert_eq!(lane.tool.as_deref(), Some("submit_translations"));
        lane.apply(
            &AgentEvent::ToolFinished {
                id: "c1".to_owned(),
                name: "submit_translations".to_owned(),
                content: "{}".to_owned(),
                is_error: false,
            },
            80,
        );
        assert_eq!(
            (lane.phase, lane.round, lane.tool.as_deref()),
            (WorkerPhase::Waiting, 2, None)
        );
        lane.apply(
            &AgentEvent::ToolFinished {
                id: "c2".to_owned(),
                name: "report_issue".to_owned(),
                content: "{}".to_owned(),
                is_error: false,
            },
            85,
        );
        assert_eq!(lane.round, 2, "one response is one round");

        lane.start_chunk(8, "Addon", 12, 8, 90);
        assert_eq!(
            (lane.chunk, lane.units, lane.round, lane.chunk_tokens),
            (Some(8), 12, 0, 0)
        );
    }

    #[test]
    fn the_board_marks_updates_as_activity() {
        let board = WorkerBoard::default();
        board.reset(2);
        board.update(1, |lane, now| {
            lane.set_backoff(5, "timeout".to_owned(), now);
        });
        board.update(9, |lane, _| lane.chunks_done = 99);
        let lanes = board.snapshot();
        assert_eq!(lanes.len(), 2);
        assert_eq!(lanes[1].lane, 2);
        assert_eq!(lanes[1].phase, WorkerPhase::Backoff);
        assert_eq!(lanes[1].retry_at_unix_ms, Some(5));
        board.set_phase(1, WorkerPhase::Idle);
        assert_eq!(board.snapshot()[1].retry_at_unix_ms, None);
    }
}
