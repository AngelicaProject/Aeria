//! Atlas `encode` command: macro text to `SeString` bytes, using the Lumina
//! version that printed the HXS macro text.

use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::{AtlasError, CancellationToken, spawn_stderr_reader, terminate_and_wait};

/// Why Atlas refused to encode one string. `code` is one of the codes Atlas
/// documents (`empty`, `invalidMacro`, `containsNul`, `tooLong`, `notRoundTrip`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodeRejection {
    pub code: String,
    pub message: String,
}

/// Runs `encode --input <file> --output <file>` for one batch of strings.
#[derive(Clone, Debug)]
pub struct AtlasEncodeRunner {
    executable_path: PathBuf,
    work_dir: PathBuf,
    stderr_limit: usize,
}

impl AtlasEncodeRunner {
    /// `work_dir` holds the request and result files of each call; they are
    /// removed when the call returns.
    #[must_use]
    pub fn new(executable_path: impl Into<PathBuf>, work_dir: impl Into<PathBuf>) -> Self {
        Self {
            executable_path: executable_path.into(),
            work_dir: work_dir.into(),
            stderr_limit: 64 * 1024,
        }
    }

    /// Returns the version Atlas reports with `--version`, recorded in the
    /// pack manifest as the encoder that produced its strings.
    ///
    /// # Errors
    /// Returns an [`AtlasError`] when Atlas cannot be started, fails, or
    /// prints no version.
    pub fn version(&self) -> Result<String, AtlasError> {
        let mut command = Command::new(&self.executable_path);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let output = command
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| AtlasError::Spawn {
                path: self.executable_path.clone(),
                message: error.to_string(),
            })?;
        let stderr_tail = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            return Err(AtlasError::Exit {
                message: format!("--version exited with {}", output.status),
                stderr_tail,
            });
        }
        let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if version.is_empty() || version.contains(char::is_whitespace) {
            return Err(AtlasError::Protocol {
                message: "--version printed no single version".to_owned(),
                line: Some(version),
                stderr_tail,
            });
        }
        Ok(version)
    }

    /// Encodes `macros` in order; one result per input string.
    ///
    /// # Errors
    /// Returns an [`AtlasError`] when Atlas cannot be started, fails, is
    /// cancelled, or writes results that do not match the request.
    pub fn encode(
        &self,
        macros: &[&str],
        cancellation: &CancellationToken,
    ) -> Result<Vec<Result<Vec<u8>, EncodeRejection>>, AtlasError> {
        fs::create_dir_all(&self.work_dir).map_err(|error| self.io_error(&error))?;
        let stem = format!("encode-{}", std::process::id());
        let input = self.work_dir.join(format!("{stem}.requests.jsonl"));
        let output = self.work_dir.join(format!("{stem}.results.jsonl"));
        let result = self.run(macros, &input, &output, cancellation);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_file(&output);
        result
    }

    fn run(
        &self,
        macros: &[&str],
        input: &Path,
        output: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Vec<Result<Vec<u8>, EncodeRejection>>, AtlasError> {
        let mut requests = String::new();
        for text in macros {
            let line = serde_json::json!({ "macro": text });
            let _ = writeln!(requests, "{line}");
        }
        fs::write(input, requests).map_err(|error| self.io_error(&error))?;
        let _ = fs::remove_file(output);

        let mut command = Command::new(&self.executable_path);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .arg("encode")
            .arg("--input")
            .arg(input)
            .arg("--output")
            .arg(output)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| AtlasError::Spawn {
                path: self.executable_path.clone(),
                message: error.to_string(),
            })?;

        let (Some(mut stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            terminate_and_wait(&mut child);
            return Err(AtlasError::Spawn {
                path: self.executable_path.clone(),
                message: "Atlas output pipes were not available".to_owned(),
            });
        };
        let stdout_thread = thread::spawn(move || {
            let mut text = String::new();
            let _ = stdout.read_to_string(&mut text);
            text
        });
        let stderr_thread = spawn_stderr_reader(stderr, self.stderr_limit);

        let status = loop {
            if cancellation.is_cancelled() {
                terminate_and_wait(&mut child);
                let _ = stdout_thread.join();
                let stderr_tail = stderr_thread.join().unwrap_or_default();
                return Err(AtlasError::Cancelled { stderr_tail });
            }
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(_) => break terminate_and_wait(&mut child),
            }
        };
        let _ = stdout_thread.join();
        let stderr_tail = stderr_thread.join().unwrap_or_default();

        match status {
            Some(status) if status.success() => {}
            Some(status) => {
                return Err(AtlasError::Exit {
                    message: format!("encode exited with {status}"),
                    stderr_tail,
                });
            }
            None => {
                return Err(AtlasError::Exit {
                    message: "could not observe Atlas exit status".to_owned(),
                    stderr_tail,
                });
            }
        }

        let text = fs::read_to_string(output).map_err(|error| AtlasError::Package {
            message: format!("encode wrote no readable results: {error}"),
            stderr_tail: stderr_tail.clone(),
        })?;
        parse_results(&text, macros.len()).map_err(|message| AtlasError::Protocol {
            message,
            line: None,
            stderr_tail,
        })
    }

    fn io_error(&self, error: &std::io::Error) -> AtlasError {
        AtlasError::Spawn {
            path: self.work_dir.clone(),
            message: error.to_string(),
        }
    }
}

type EncodeResults = Vec<Result<Vec<u8>, EncodeRejection>>;

fn parse_results(text: &str, expected: usize) -> Result<EncodeResults, String> {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.len() != expected {
        return Err(format!(
            "encode returned {} results for {expected} requests",
            lines.len()
        ));
    }
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            parse_result(line).map_err(|error| format!("result {}: {error}", index + 1))
        })
        .collect()
}

fn parse_result(line: &str) -> Result<Result<Vec<u8>, EncodeRejection>, String> {
    let value: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
    let object = value.as_object().ok_or("result is not an object")?;
    if let Some(hex) = object.get("hex") {
        let hex = hex.as_str().ok_or("hex is not a string")?;
        return decode_hex(hex).map(Ok);
    }
    let code = object
        .get("error")
        .and_then(Value::as_str)
        .ok_or("result has neither hex nor error")?;
    let message = object
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(Err(EncodeRejection {
        code: code.to_owned(),
        message: message.to_owned(),
    }))
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    if hex.is_empty() || !hex.len().is_multiple_of(2) {
        return Err("hex has an invalid length".to_owned());
    }
    hex.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digits = std::str::from_utf8(pair).map_err(|_| "hex is not ASCII".to_owned())?;
            u8::from_str_radix(digits, 16).map_err(|_| format!("invalid hex digits {digits:?}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_parse_in_order() {
        let results = parse_results(
            "{\"hex\":\"d09f\"}\n{\"error\":\"invalidMacro\",\"message\":\"bad\"}\n",
            2,
        )
        .unwrap();
        assert_eq!(results[0], Ok(vec![0xd0, 0x9f]));
        assert_eq!(
            results[1],
            Err(EncodeRejection {
                code: "invalidMacro".to_owned(),
                message: "bad".to_owned()
            })
        );
    }

    #[test]
    fn mismatched_counts_and_bad_hex_are_protocol_errors() {
        assert!(parse_results("{\"hex\":\"00\"}\n", 2).is_err());
        assert!(parse_results("{\"hex\":\"0\"}\n", 1).is_err());
        assert!(parse_results("{\"hex\":\"zz\"}\n", 1).is_err());
        assert!(parse_results("{\"other\":1}\n", 1).is_err());
    }
}
