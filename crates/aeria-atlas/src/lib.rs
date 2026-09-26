//! Harmonia Atlas process integration.
//!
//! This crate deliberately owns only the process boundary. It has no Tauri,
//! workspace, or HSP dependencies, which keeps the protocol and lifecycle
//! independently testable with a small fake child process.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

/// The only source languages currently supported by Atlas's global evidence
/// policy.
pub const SUPPORTED_SOURCE_LANGUAGES: &[&str] = &["en", "ja", "de", "fr"];

/// The exact request sent to the Atlas package command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AtlasPackageRequest {
    pub executable_path: PathBuf,
    pub game_path: PathBuf,
    pub source_language: String,
    pub output_path: PathBuf,
}

/// A protocol event emitted by Atlas.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AtlasEvent {
    Started {
        protocol_version: u32,
        language: Option<String>,
    },
    Phase {
        protocol_version: u32,
        phase: String,
    },
    Progress {
        protocol_version: u32,
        phase: String,
        sheet: Option<String>,
        language: Option<String>,
        sheet_index: Option<u64>,
        sheet_count: Option<u64>,
        rows_processed: Option<u64>,
        sheet_completed: Option<bool>,
    },
    Completed {
        protocol_version: u32,
        package_id: String,
        output_path: Option<String>,
        metadata: BTreeMap<String, Value>,
    },
    Failed {
        protocol_version: u32,
        code: Option<String>,
        message: Option<String>,
    },
}

/// The validated result of one successful Atlas package command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AtlasPackageResult {
    pub package_id: String,
    pub output_path: PathBuf,
    pub completed_metadata: BTreeMap<String, Value>,
}

/// A clonable cancellation token. The runner remains the owner of the child;
/// callers only retain this small signal object.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

/// The sender held by a desktop job registry.
#[derive(Clone, Debug)]
pub struct CancellationHandle {
    token: CancellationToken,
}

impl CancellationToken {
    /// Creates a token and the handle used to cancel it.
    #[must_use]
    pub fn new() -> (Self, CancellationHandle) {
        let token = Self::default();
        let handle = CancellationHandle {
            token: token.clone(),
        };
        (token, handle)
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl CancellationHandle {
    /// Requests cancellation. The runner will terminate and await the child.
    pub fn cancel(&self) {
        self.token.cancelled.store(true, Ordering::Release);
    }
}

/// Process-boundary failures. Diagnostics always contain at most the runner's
/// configured stderr tail.
#[derive(Debug, Error)]
pub enum AtlasError {
    #[error("could not spawn Atlas executable {path}: {message}")]
    Spawn { path: PathBuf, message: String },

    #[error("Atlas emitted invalid JSONL protocol: {message}")]
    Protocol {
        message: String,
        line: Option<String>,
        stderr_tail: String,
    },

    #[error("Atlas reported failure{code}: {message}")]
    Failed {
        code: String,
        message: String,
        stderr_tail: String,
    },

    #[error("Atlas exited unsuccessfully: {message}")]
    Exit {
        message: String,
        stderr_tail: String,
    },

    #[error("Atlas package output is invalid: {message}")]
    Package {
        message: String,
        stderr_tail: String,
    },

    #[error("Atlas package generation was cancelled")]
    Cancelled { stderr_tail: String },
}

impl AtlasError {
    #[must_use]
    pub fn stderr_tail(&self) -> &str {
        match self {
            Self::Spawn { .. } => "",
            Self::Protocol { stderr_tail, .. }
            | Self::Failed { stderr_tail, .. }
            | Self::Exit { stderr_tail, .. }
            | Self::Package { stderr_tail, .. }
            | Self::Cancelled { stderr_tail } => stderr_tail,
        }
    }
}

/// Runs one Atlas `package --events jsonl` invocation.
#[derive(Clone, Copy, Debug)]
pub struct AtlasPackageRunner {
    stderr_limit: usize,
}

impl Default for AtlasPackageRunner {
    fn default() -> Self {
        Self::new(64 * 1024)
    }
}

impl AtlasPackageRunner {
    #[must_use]
    pub const fn new(stderr_limit: usize) -> Self {
        Self { stderr_limit }
    }

    /// Runs Atlas while retaining only a bounded stderr tail.
    ///
    /// The child is owned by this call. Cancellation kills it, waits for it,
    /// joins both pipe readers, and then returns a cancelled error.
    ///
    /// # Errors
    ///
    /// Returns a typed error when Atlas cannot be spawned, emits invalid
    /// protocol, reports failure, exits unsuccessfully, or does not publish
    /// the requested package.
    #[allow(clippy::too_many_lines)]
    pub fn run<F>(
        &self,
        request: &AtlasPackageRequest,
        mut event_callback: F,
        cancellation: &CancellationToken,
    ) -> Result<AtlasPackageResult, AtlasError>
    where
        F: FnMut(AtlasEvent),
    {
        if cancellation.is_cancelled() {
            return Err(AtlasError::Cancelled {
                stderr_tail: String::new(),
            });
        }

        if !SUPPORTED_SOURCE_LANGUAGES.contains(&request.source_language.as_str()) {
            return Err(AtlasError::Package {
                message: format!(
                    "unsupported source language {:?}; supported languages are en, ja, de, fr",
                    request.source_language
                ),
                stderr_tail: String::new(),
            });
        }

        let mut command = Command::new(&request.executable_path);
        #[cfg(windows)]
        {
            // Atlas is a console program; without this flag Windows opens a
            // console window for it beside the desktop application.
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .arg("package")
            .arg("--game-path")
            .arg(&request.game_path)
            .arg("--language")
            .arg(&request.source_language)
            .arg("--output")
            .arg(&request.output_path)
            .arg("--events")
            .arg("jsonl")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| AtlasError::Spawn {
                path: request.executable_path.clone(),
                message: source.to_string(),
            })?;

        let stdout = child.stdout.take().ok_or_else(|| AtlasError::Spawn {
            path: request.executable_path.clone(),
            message: "Atlas stdout pipe was not available".to_owned(),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| AtlasError::Spawn {
            path: request.executable_path.clone(),
            message: "Atlas stderr pipe was not available".to_owned(),
        })?;

        let (message_tx, message_rx) = mpsc::channel();
        let stdout_thread = spawn_stdout_reader(stdout, message_tx.clone());
        let stderr_thread = spawn_stderr_reader(stderr, self.stderr_limit);

        let mut state = ProtocolState::default();
        let mut cancelled = false;

        loop {
            if cancellation.is_cancelled() {
                cancelled = true;
                break;
            }

            match message_rx.recv_timeout(Duration::from_millis(25)) {
                Ok(OutputMessage::Line(line)) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    match state.accept(&line) {
                        Ok(event) => event_callback(event),
                        Err(error) => {
                            terminate_and_wait(&mut child);
                            let stderr_tail = join_stderr(stderr_thread);
                            join_reader(stdout_thread);
                            return Err(AtlasError::Protocol {
                                message: error,
                                line: Some(line),
                                stderr_tail,
                            });
                        }
                    }
                }
                Ok(OutputMessage::Io(message)) => {
                    terminate_and_wait(&mut child);
                    let stderr_tail = join_stderr(stderr_thread);
                    join_reader(stdout_thread);
                    return Err(AtlasError::Protocol {
                        message: format!("failed to read Atlas stdout: {message}"),
                        line: None,
                        stderr_tail,
                    });
                }
                Ok(OutputMessage::Done) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }

        let status = if cancelled {
            terminate_and_wait(&mut child)
        } else {
            child.wait().ok()
        };
        join_reader(stdout_thread);
        let stderr_tail = join_stderr(stderr_thread);

        if cancelled {
            return Err(AtlasError::Cancelled { stderr_tail });
        }

        let Some(status) = status else {
            return Err(AtlasError::Exit {
                message: "could not observe Atlas exit status".to_owned(),
                stderr_tail,
            });
        };

        let Some(terminal) = state.terminal else {
            return Err(AtlasError::Protocol {
                message: if state.started {
                    "Atlas exited without a terminal completed or failed event".to_owned()
                } else {
                    "Atlas exited without a started event".to_owned()
                },
                line: None,
                stderr_tail,
            });
        };

        match terminal {
            TerminalEvent::Failed { code, message } => Err(AtlasError::Failed {
                code: code.unwrap_or_else(|| "unknown".to_owned()),
                message: message
                    .unwrap_or_else(|| "Atlas reported an unspecified failure".to_owned()),
                stderr_tail,
            }),
            TerminalEvent::Completed {
                package_id,
                output_path,
                metadata,
            } => {
                if !status.success() {
                    return Err(AtlasError::Exit {
                        message: format_exit_status(status),
                        stderr_tail,
                    });
                }
                if !is_canonical_package_id(&package_id) {
                    return Err(AtlasError::Package {
                        message: format!("completed.packageId is not canonical: {package_id:?}"),
                        stderr_tail,
                    });
                }
                if !request.output_path.is_file() {
                    return Err(AtlasError::Package {
                        message: format!(
                            "completed event referenced a package, but requested output does not exist: {}",
                            request.output_path.display()
                        ),
                        stderr_tail,
                    });
                }
                if let Some(reported_path) = output_path {
                    let expected = normalize_path(&request.output_path);
                    let reported = normalize_path(Path::new(&reported_path));
                    if expected != reported {
                        return Err(AtlasError::Package {
                            message: format!(
                                "completed.outputPath {:?} does not match requested output {}",
                                reported_path,
                                request.output_path.display()
                            ),
                            stderr_tail,
                        });
                    }
                }
                Ok(AtlasPackageResult {
                    package_id,
                    output_path: request.output_path.clone(),
                    completed_metadata: metadata,
                })
            }
        }
    }
}

#[derive(Default)]
struct ProtocolState {
    started: bool,
    terminal: Option<TerminalEvent>,
}

enum TerminalEvent {
    Completed {
        package_id: String,
        output_path: Option<String>,
        metadata: BTreeMap<String, Value>,
    },
    Failed {
        code: Option<String>,
        message: Option<String>,
    },
}

impl ProtocolState {
    fn accept(&mut self, line: &str) -> Result<AtlasEvent, String> {
        let value: Value = serde_json::from_str(line)
            .map_err(|error| format!("stdout line is not valid JSON: {error}"))?;
        let object = value
            .as_object()
            .ok_or_else(|| "stdout event must be a JSON object".to_owned())?;
        let protocol_version = required_u32(object, "protocolVersion")?;
        if protocol_version != 1 {
            return Err(format!(
                "unsupported protocolVersion {protocol_version}; expected 1"
            ));
        }
        let event_type = required_string(object, "type")?;
        if self.terminal.is_some() {
            return Err("event appeared after the terminal event".to_owned());
        }

        match event_type.as_str() {
            "started" => {
                if self.started {
                    return Err("duplicate started event".to_owned());
                }
                self.started = true;
                Ok(AtlasEvent::Started {
                    protocol_version,
                    language: optional_string(object, "language")?,
                })
            }
            "phase" => {
                self.require_started()?;
                Ok(AtlasEvent::Phase {
                    protocol_version,
                    phase: required_string(object, "phase")?,
                })
            }
            "progress" => {
                self.require_started()?;
                Ok(AtlasEvent::Progress {
                    protocol_version,
                    phase: required_string(object, "phase")?,
                    sheet: optional_string(object, "sheet")?,
                    language: optional_string(object, "language")?,
                    sheet_index: optional_u64(object, "sheetIndex")?,
                    sheet_count: optional_u64(object, "sheetCount")?,
                    rows_processed: optional_u64(object, "rowsProcessed")?,
                    sheet_completed: optional_bool(object, "sheetCompleted")?,
                })
            }
            "completed" => {
                self.require_started()?;
                let package_id = required_string(object, "packageId")?;
                let output_path = optional_string(object, "outputPath")?;
                let metadata: BTreeMap<String, Value> = object
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(
                            key.as_str(),
                            "type" | "protocolVersion" | "packageId" | "outputPath"
                        )
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                self.terminal = Some(TerminalEvent::Completed {
                    package_id: package_id.clone(),
                    output_path: output_path.clone(),
                    metadata: metadata.clone(),
                });
                Ok(AtlasEvent::Completed {
                    protocol_version,
                    package_id,
                    output_path,
                    metadata,
                })
            }
            "failed" => {
                self.require_started()?;
                let code = optional_string(object, "code")?;
                let message = optional_string(object, "message")?;
                self.terminal = Some(TerminalEvent::Failed {
                    code: code.clone(),
                    message: message.clone(),
                });
                Ok(AtlasEvent::Failed {
                    protocol_version,
                    code,
                    message,
                })
            }
            other => Err(format!("unknown protocol v1 event type {other:?}")),
        }
    }

    fn require_started(&self) -> Result<(), String> {
        self.started
            .then_some(())
            .ok_or_else(|| "event appeared before started".to_owned())
    }
}

fn required_string(object: &Map<String, Value>, key: &str) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("event field {key:?} must be a string"))
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(value.to_owned()))
            .ok_or_else(|| format!("event field {key:?} must be a string or null")),
    }
}

fn required_u32(object: &Map<String, Value>, key: &str) -> Result<u32, String> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("event field {key:?} must be an unsigned integer"))
}

fn optional_u64(object: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("event field {key:?} must be an unsigned integer or null")),
    }
}

fn optional_bool(object: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| format!("event field {key:?} must be a boolean or null")),
    }
}

fn is_canonical_package_id(value: &str) -> bool {
    value.len() == "sha256:".len() + 64
        && value.starts_with("sha256:")
        && value["sha256:".len()..]
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
}

fn normalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn format_exit_status(status: std::process::ExitStatus) -> String {
    status.code().map_or_else(
        || "Atlas was terminated by a signal".to_owned(),
        |code| format!("exit code {code}"),
    )
}

enum OutputMessage {
    Line(String),
    Io(String),
    Done,
}

fn spawn_stdout_reader<R>(reader: R, sender: Sender<OutputMessage>) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if sender.send(OutputMessage::Line(line)).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = sender.send(OutputMessage::Io(error.to_string()));
                    return;
                }
            }
        }
        let _ = sender.send(OutputMessage::Done);
    })
}

fn spawn_stderr_reader<R>(mut reader: R, limit: usize) -> thread::JoinHandle<String>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut tail: VecDeque<u8> = VecDeque::with_capacity(limit.min(8192));
        let mut chunk = [0_u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    tail.extend(&chunk[..read]);
                    while tail.len() > limit {
                        let _ = tail.pop_front();
                    }
                }
            }
        }
        String::from_utf8_lossy(&tail.into_iter().collect::<Vec<_>>()).into_owned()
    })
}

fn terminate_and_wait(child: &mut Child) -> Option<std::process::ExitStatus> {
    let _ = child.kill();
    child.wait().ok()
}

fn join_reader(thread: thread::JoinHandle<()>) {
    let _ = thread.join();
}

fn join_stderr(thread: thread::JoinHandle<String>) -> String {
    thread.join().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_with_the_renderer_field_names() {
        let event = AtlasEvent::Progress {
            protocol_version: 1,
            phase: "extractSource".to_owned(),
            sheet: Some("Item".to_owned()),
            language: None,
            sheet_index: Some(2),
            sheet_count: Some(10),
            rows_processed: Some(42),
            sheet_completed: Some(false),
        };
        assert_eq!(
            serde_json::to_value(event).expect("event"),
            serde_json::json!({
                "type": "progress",
                "protocolVersion": 1,
                "phase": "extractSource",
                "sheet": "Item",
                "language": null,
                "sheetIndex": 2,
                "sheetCount": 10,
                "rowsProcessed": 42,
                "sheetCompleted": false,
            })
        );
    }

    #[test]
    fn package_command_arguments_are_not_changed_by_the_parser() {
        assert!(is_canonical_package_id(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(!is_canonical_package_id(
            "sha256:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
    }

    #[test]
    fn protocol_requires_started_before_other_events() {
        let mut state = ProtocolState::default();
        let error = state
            .accept(r#"{"type":"phase","protocolVersion":1,"phase":"inspectInstallation"}"#)
            .expect_err("phase before started");
        assert!(error.contains("before started"));
    }

    #[test]
    fn completed_captures_unknown_metadata_without_accepting_unknown_event_types() {
        let mut state = ProtocolState::default();
        state
            .accept(r#"{"type":"started","protocolVersion":1}"#)
            .expect("started");
        let event = state
            .accept(r#"{"type":"completed","protocolVersion":1,"packageId":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","elapsedMs":4}"#)
            .expect("completed");
        assert!(
            matches!(event, AtlasEvent::Completed { metadata, .. } if metadata["elapsedMs"] == 4)
        );
        let error = state
            .accept(r#"{"type":"phase","protocolVersion":1,"phase":"late"}"#)
            .expect_err("event after terminal");
        assert!(error.contains("after the terminal"));
    }

    #[test]
    fn unknown_event_type_is_a_protocol_error() {
        let mut state = ProtocolState::default();
        state
            .accept(r#"{"type":"started","protocolVersion":1}"#)
            .expect("started");
        let error = state
            .accept(r#"{"type":"humanProse","protocolVersion":1}"#)
            .expect_err("unknown event");
        assert!(error.contains("unknown protocol"));
    }
}
