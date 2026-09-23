//! Bounded invocation of the system Git executable.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;

use crate::GitError;

/// The Git executable and environment used for repository operations.
///
/// Aeria drives the ordinary Git command-line client so that repositories,
/// hooks, SSH configuration, and OS/Git credential helpers behave exactly as
/// they do for the user's other Git tools.
#[derive(Clone, Debug)]
pub struct GitExecutable {
    program: OsString,
    env: Vec<(OsString, OsString)>,
    origin: GitOrigin,
}

/// Where the Git executable comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitOrigin {
    /// `git` found on `PATH`.
    System,
    /// The Git runtime bundled with Aeria.
    Bundled,
    /// The `AERIA_GIT_PATH` override.
    Override,
}

impl GitExecutable {
    /// Uses `git` from `PATH`.
    #[must_use]
    pub fn system() -> Self {
        Self::at("git")
    }

    /// Uses an explicit Git executable.
    #[must_use]
    pub fn at(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            env: Vec::new(),
            origin: GitOrigin::System,
        }
    }

    /// Selects the Git executable Aeria should use, in this order:
    ///
    /// 1. `AERIA_GIT_PATH`, an explicit development/test override;
    /// 2. `bundled`, the Git runtime shipped with the application, when it
    ///    exists;
    /// 3. `git` from `PATH`.
    #[must_use]
    pub fn discover(bundled: Option<&Path>) -> Self {
        if let Some(path) = std::env::var_os("AERIA_GIT_PATH").filter(|path| !path.is_empty()) {
            return Self::at(path).with_origin(GitOrigin::Override);
        }
        if let Some(path) = bundled.filter(|path| path.is_file()) {
            return Self::at(path).with_origin(GitOrigin::Bundled);
        }
        Self::system()
    }

    fn with_origin(mut self, origin: GitOrigin) -> Self {
        self.origin = origin;
        self
    }

    /// Returns where this executable was found.
    #[must_use]
    pub const fn origin(&self) -> GitOrigin {
        self.origin
    }

    /// Returns the output of `git --version`, for example
    /// `git version 2.55.0.windows.5`.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::GitUnavailable`] when Git cannot be started.
    pub fn version(&self, cwd: &Path) -> Result<String, GitError> {
        Ok(self.run_text(cwd, &["--version"])?.trim().to_owned())
    }

    /// Adds an environment variable to every invocation. Tests use this to
    /// isolate Git from user and system configuration.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    fn command(&self, cwd: &Path) -> Command {
        let mut command = Command::new(&self.program);
        command
            .current_dir(cwd)
            // Stable, parseable diagnostics and no interactive terminal prompts.
            // Credential helpers with their own UI still work.
            .env("LC_ALL", "C")
            .env("LANGUAGE", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .args([
                "-c",
                "core.quotepath=false",
                "-c",
                "color.ui=never",
                "-c",
                "i18n.logOutputEncoding=UTF-8",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &self.env {
            command.env(key, value);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }

    /// Runs Git and returns its raw output without interpreting the exit code.
    pub(crate) fn output<S: AsRef<OsStr>>(
        &self,
        cwd: &Path,
        args: &[S],
    ) -> Result<Output, GitError> {
        self.command(cwd)
            .args(args)
            .output()
            .map_err(|source| GitError::GitUnavailable {
                program: self.program.to_string_lossy().into_owned(),
                message: source.to_string(),
            })
    }

    /// Runs Git with `input` on stdin and returns its raw output.
    pub(crate) fn output_with_input<S: AsRef<OsStr>>(
        &self,
        cwd: &Path,
        args: &[S],
        input: Vec<u8>,
    ) -> Result<Output, GitError> {
        let spawn_error = |source: std::io::Error| GitError::GitUnavailable {
            program: self.program.to_string_lossy().into_owned(),
            message: source.to_string(),
        };
        let mut child = self
            .command(cwd)
            .args(args)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(spawn_error)?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| spawn_error(std::io::Error::other("stdin pipe unavailable")))?;
        // Write from a separate thread so a large response cannot deadlock
        // against a full stdin pipe.
        let writer = thread::spawn(move || stdin.write_all(&input));
        let output = child.wait_with_output().map_err(spawn_error)?;
        writer
            .join()
            .map_err(|_| spawn_error(std::io::Error::other("stdin writer panicked")))?
            .map_err(spawn_error)?;
        Ok(output)
    }

    /// Runs Git and requires a successful exit.
    pub(crate) fn run<S: AsRef<OsStr>>(&self, cwd: &Path, args: &[S]) -> Result<Vec<u8>, GitError> {
        let output = self.output(cwd, args)?;
        require_success(args, &output)?;
        Ok(output.stdout)
    }

    /// Runs Git, requires success, and decodes stdout as UTF-8.
    pub(crate) fn run_text<S: AsRef<OsStr>>(
        &self,
        cwd: &Path,
        args: &[S],
    ) -> Result<String, GitError> {
        let stdout = self.run(cwd, args)?;
        String::from_utf8(stdout).map_err(|_| GitError::Parse {
            message: format!("git {} produced non-UTF-8 output", describe(args)),
        })
    }
}

impl Default for GitExecutable {
    fn default() -> Self {
        Self::system()
    }
}

pub(crate) fn require_success<S: AsRef<OsStr>>(
    args: &[S],
    output: &Output,
) -> Result<(), GitError> {
    if output.status.success() {
        return Ok(());
    }
    Err(GitError::CommandFailed {
        command: describe(args),
        status: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

/// Names the Git subcommand for diagnostics. Arguments are omitted because
/// they may contain commit messages or remote URLs with credentials.
fn describe<S: AsRef<OsStr>>(args: &[S]) -> String {
    let mut args = args.iter().map(|arg| arg.as_ref().to_string_lossy());
    let mut name = args.next().unwrap_or_default();
    while name == "-c" {
        args.next();
        name = args.next().unwrap_or_default();
    }
    name.into_owned()
}
