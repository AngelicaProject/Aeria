//! Live activity of a running job's workers.
//!
//! Each lane of a job runner reports what it is doing: which chunk it
//! translates, which model response it waits for, which string it is writing
//! or checking, when it last received anything from the provider, and the
//! tail of its latest reasoning. The board lives only while the runner runs
//! and is never persisted; it exists so the user can see that workers are
//! alive and what they are working on.

use std::sync::{Mutex, MutexGuard, PoisonError};

use aeria_ai::agent::AgentEvent;
use aeria_ai::worker::UnitPreview;
use serde::Serialize;
use serde_json::Value;

use crate::angelica::now_unix_ms;

/// Most characters of a lane's latest reasoning kept for display.
const THOUGHT_CHARS: usize = 600;
/// The tool that writes a chunk's translations.
const SUBMIT_TOOL: &str = "submit_translations";

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

/// The string a lane is writing, checking, or reading about.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerTarget {
    /// The string's number in the chunk, from 1; `None` for a row the
    /// worker reads for context.
    pub unit: Option<u32>,
    /// `sheet:row:subrow:column`, or `sheet:row` for a context read.
    pub address: String,
    /// The start of the source as plain text; empty when unknown.
    pub source: String,
}

/// Finds the `"unit": N` fields of streaming tool-call arguments. Only the
/// unread end of the arguments is kept.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct UnitScan {
    pending: String,
    /// Distinct units in the order they appeared.
    units: Vec<u32>,
}

impl UnitScan {
    fn push(&mut self, piece: &str) {
        self.pending.push_str(piece);
        let mut from = 0;
        let keep = loop {
            let Some(offset) = self.pending[from..].find("\"unit\"") else {
                // A key split across pieces starts at most five bytes back.
                let mut tail = self.pending.len().saturating_sub(5).max(from);
                while !self.pending.is_char_boundary(tail) {
                    tail -= 1;
                }
                break tail;
            };
            let start = from + offset;
            let after_key = &self.pending[start + 6..];
            let trimmed = after_key.trim_start();
            let Some(value) = trimmed.strip_prefix(':') else {
                if trimmed.is_empty() {
                    break start;
                }
                // The word inside a string, not a field name.
                from = start + 6;
                continue;
            };
            let value = value.trim_start();
            let digits = value.bytes().take_while(u8::is_ascii_digit).count();
            if digits == value.len() {
                // The number may continue in the next piece.
                break start;
            }
            if let Ok(unit) = value[..digits].parse::<u32>()
                && !self.units.contains(&unit)
            {
                self.units.push(unit);
            }
            from = self.pending.len() - value.len() + digits;
        };
        self.pending.drain(..keep);
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
    /// Workers submit a chunk's translations together, so this grows when a
    /// submission has been checked.
    pub finished_units: u32,
    /// Strings whose translation the current response has streamed so far.
    pub streamed_units: u32,
    /// Translations in the submission the `tool` phase checks and writes.
    pub tool_units: u32,
    /// The string the lane is writing, checking, or reading about.
    pub target: Option<WorkerTarget>,
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
    /// The end of the reasoning the provider streamed for the current
    /// response, or for the last one that had any, at most
    /// [`THOUGHT_CHARS`] characters.
    pub thought: Option<String>,
    /// The chunk's strings, numbered from 1 in the worker's messages.
    #[serde(skip)]
    previews: Vec<UnitPreview>,
    #[serde(skip)]
    scan: UnitScan,
    /// The next tool-call arguments start a new scan.
    #[serde(skip)]
    scan_ended: bool,
    /// The next reasoning delta starts a new thought.
    #[serde(skip)]
    thought_ended: bool,
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
            first_row: None,
            last_row: None,
            units: 0,
            finished_units: 0,
            streamed_units: 0,
            tool_units: 0,
            target: None,
            round: 0,
            max_rounds: 0,
            tool: None,
            chunk_tokens: 0,
            chunks_done: 0,
            phase_started_unix_ms: now,
            last_activity_unix_ms: now,
            retry_at_unix_ms: None,
            last_error: None,
            thought: None,
            previews: Vec::new(),
            scan: UnitScan::default(),
            scan_ended: false,
            thought_ended: false,
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
            self.tool_units = 0;
        }
        if phase != WorkerPhase::Backoff {
            self.retry_at_unix_ms = None;
        }
    }

    /// Starts a claimed chunk covering `rows`, its first and last row.
    pub fn start_chunk(
        &mut self,
        chunk: u64,
        sheet: &str,
        rows: (u32, u32),
        units: u32,
        max_rounds: u32,
        now: u64,
    ) {
        self.set_phase(WorkerPhase::Preparing, now);
        self.chunk = Some(chunk);
        self.sheet = Some(sheet.to_owned());
        self.first_row = Some(rows.0);
        self.last_row = Some(rows.1);
        self.units = units;
        self.finished_units = 0;
        self.streamed_units = 0;
        self.target = None;
        self.round = 0;
        self.max_rounds = max_rounds;
        self.chunk_tokens = 0;
        self.response_finished = false;
        self.thought = None;
        self.thought_ended = false;
        self.previews.clear();
        self.scan = UnitScan::default();
        self.scan_ended = false;
    }

    /// Sets the prepared chunk's strings, numbered from 1.
    pub fn set_previews(&mut self, previews: Vec<UnitPreview>) {
        self.previews = previews;
    }

    /// The target for a chunk string numbered from 1.
    fn unit_target(&self, unit: u32) -> WorkerTarget {
        let preview = usize::try_from(unit)
            .ok()
            .and_then(|unit| unit.checked_sub(1))
            .and_then(|index| self.previews.get(index))
            .cloned()
            .unwrap_or_default();
        WorkerTarget {
            unit: Some(unit),
            address: preview.address,
            source: preview.source,
        }
    }

    /// Appends streamed reasoning, keeping only its end.
    fn add_thought(&mut self, text: &str) {
        let thought = self.thought.get_or_insert_with(String::new);
        if std::mem::take(&mut self.thought_ended) {
            thought.clear();
        }
        thought.push_str(text);
        let chars = thought.chars().count();
        if chars > THOUGHT_CHARS {
            let cut = thought
                .char_indices()
                .nth(chars - THOUGHT_CHARS)
                .map_or(0, |(index, _)| index);
            thought.replace_range(..cut, "");
        }
    }

    /// Follows streamed tool-call arguments to the string being written.
    fn add_arguments(&mut self, text: &str) {
        if std::mem::take(&mut self.scan_ended) {
            self.scan = UnitScan::default();
        }
        self.scan.push(text);
        self.streamed_units = u32::try_from(self.scan.units.len()).unwrap_or(u32::MAX);
        if let Some(&unit) = self.scan.units.last() {
            self.target = Some(self.unit_target(unit));
        }
    }

    /// What a starting tool call works on, from its arguments.
    fn tool_target(&mut self, name: &str, arguments: &str) {
        let arguments: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
        if name == SUBMIT_TOOL {
            self.tool_units = arguments
                .get("translations")
                .and_then(Value::as_array)
                .map_or(0, |items| u32::try_from(items.len()).unwrap_or(u32::MAX));
            self.target = None;
            return;
        }
        let unit = arguments
            .get("unit")
            .and_then(Value::as_u64)
            .and_then(|unit| u32::try_from(unit).ok());
        let sheet = arguments.get("sheet").and_then(Value::as_str);
        let row = arguments
            .get("row")
            .or_else(|| arguments.get("after_row"))
            .and_then(Value::as_u64);
        self.target = match (unit, sheet, row) {
            (Some(unit), _, _) => Some(self.unit_target(unit)),
            (None, Some(sheet), Some(row)) => Some(WorkerTarget {
                unit: None,
                address: format!("{sheet}:{row}"),
                source: String::new(),
            }),
            (None, Some(sheet), None) => Some(WorkerTarget {
                unit: None,
                address: sheet.to_owned(),
                source: String::new(),
            }),
            _ => None,
        };
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
        self.target = None;
    }

    /// Follows one event of the worker's turn.
    pub fn apply(&mut self, event: &AgentEvent, now: u64) {
        match event {
            AgentEvent::ReasoningDelta { text } => {
                self.set_phase(WorkerPhase::Reasoning, now);
                self.add_thought(text);
            }
            AgentEvent::TextDelta { .. } => self.set_phase(WorkerPhase::Writing, now),
            AgentEvent::ToolArgumentsDelta { text, .. } => {
                self.set_phase(WorkerPhase::Writing, now);
                self.add_arguments(text);
            }
            AgentEvent::ResponseFinished => {
                self.response_finished = true;
                // The thought and the streamed count stay visible until the
                // next response replaces them.
                self.thought_ended = true;
                self.scan_ended = true;
            }
            AgentEvent::Usage { usage } => {
                self.chunk_tokens += usage.prompt_tokens + usage.completion_tokens;
            }
            AgentEvent::ToolStarted {
                name, arguments, ..
            } => {
                self.set_phase(WorkerPhase::Tool, now);
                self.tool = Some(name.clone());
                self.tool_target(name, arguments);
            }
            // The next request follows the tool results; a later tool of
            // the same response replaces this phase at once.
            AgentEvent::ToolFinished { .. } => {
                self.set_phase(WorkerPhase::Waiting, now);
                self.target = None;
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
    use aeria_ai::chat::Usage;

    use super::*;

    #[test]
    fn a_lane_follows_its_chunk_through_the_turn() {
        let mut lane = WorkerActivity::new(1, 0);
        lane.start_chunk(7, "Addon", (10, 39), 30, 8, 10);
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
        assert_eq!(lane.thought.as_deref(), Some("……"));
        lane.apply(
            &AgentEvent::ToolArgumentsDelta {
                chars: 12,
                text: "{\"translations\"".to_owned(),
            },
            50,
        );
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
        assert_eq!(
            lane.thought.as_deref(),
            Some("……"),
            "a finished thought stays visible"
        );

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

        lane.apply(
            &AgentEvent::ReasoningDelta {
                text: "next".to_owned(),
            },
            87,
        );
        assert_eq!(
            lane.thought.as_deref(),
            Some("next"),
            "a new response starts a new thought"
        );

        lane.start_chunk(8, "Addon", (40, 51), 12, 8, 90);
        assert_eq!(
            (lane.chunk, lane.units, lane.round, lane.chunk_tokens),
            (Some(8), 12, 0, 0)
        );
        assert_eq!(lane.thought, None);
    }

    fn arguments(lane: &mut WorkerActivity, text: &str, now: u64) {
        lane.apply(
            &AgentEvent::ToolArgumentsDelta {
                chars: text.chars().count(),
                text: text.to_owned(),
            },
            now,
        );
    }

    #[test]
    fn streamed_arguments_name_the_string_being_written() {
        let mut lane = WorkerActivity::new(1, 0);
        lane.start_chunk(3, "Item", (100, 102), 3, 8, 0);
        lane.set_previews(
            (0..3)
                .map(|index| UnitPreview {
                    address: format!("Item:{}:0:9", 100 + index),
                    source: format!("Potion {index}"),
                })
                .collect(),
        );
        // Pieces split keys and numbers anywhere.
        for piece in [
            "{\"translations\":[{\"un",
            "it\": 1",
            "0",
            ", \"target\":\"a \\\"unit\\\" word\"},{\"unit\":",
            "2,",
        ] {
            arguments(&mut lane, piece, 1);
        }
        assert_eq!(lane.phase, WorkerPhase::Writing);
        assert_eq!(lane.streamed_units, 2, "a quoted word is not a field");
        let target = lane.target.clone().expect("target");
        assert_eq!(
            (target.unit, target.address.as_str(), target.source.as_str()),
            (Some(2), "Item:101:0:9", "Potion 1")
        );
        assert!(lane.scan.pending.len() < 16, "read arguments are dropped");

        lane.apply(&AgentEvent::ResponseFinished, 2);
        lane.apply(
            &AgentEvent::ToolStarted {
                id: "c1".to_owned(),
                name: SUBMIT_TOOL.to_owned(),
                arguments: "{\"translations\":[{\"unit\":1,\"target\":\"a\"},{\"unit\":2,\"target\":\"b\"}]}".to_owned(),
            },
            3,
        );
        assert_eq!((lane.tool_units, lane.target.is_none()), (2, true));
        lane.apply(
            &AgentEvent::ToolFinished {
                id: "c1".to_owned(),
                name: SUBMIT_TOOL.to_owned(),
                content: "{}".to_owned(),
                is_error: false,
            },
            4,
        );
        assert_eq!(
            lane.streamed_units, 2,
            "the count stays until the next response"
        );
        arguments(&mut lane, "{\"unit\":3,", 5);
        assert_eq!(lane.streamed_units, 1, "a new response starts a new count");

        lane.apply(
            &AgentEvent::ToolStarted {
                id: "c2".to_owned(),
                name: "get_unit".to_owned(),
                arguments: "{\"sheet\":\"Item\",\"row\":7}".to_owned(),
            },
            6,
        );
        assert_eq!(
            lane.target.as_ref().map(|target| target.address.as_str()),
            Some("Item:7")
        );
    }

    #[test]
    fn a_long_thought_keeps_its_end() {
        let mut lane = WorkerActivity::new(1, 0);
        let long = format!("{}конец", "я".repeat(THOUGHT_CHARS));
        lane.apply(&AgentEvent::ReasoningDelta { text: long }, 1);
        let thought = lane.thought.expect("thought");
        assert_eq!(thought.chars().count(), THOUGHT_CHARS);
        assert!(thought.ends_with("конец"));
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
        board.ensure(4);
        let lanes = board.snapshot();
        assert_eq!(
            lanes.iter().map(|lane| lane.lane).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        assert_eq!(lanes[1].phase, WorkerPhase::Idle, "existing lanes stay");
        board.ensure(2);
        assert_eq!(board.snapshot().len(), 4, "lanes never shrink");
    }
}
