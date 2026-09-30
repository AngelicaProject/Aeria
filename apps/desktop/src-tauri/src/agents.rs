//! Connecting agent harnesses to Aeria: the `aeria` command on `PATH`, and
//! `AGENTS.md` and `CLAUDE.md` in the open project (see
//! `docs/architecture/agents.md`). What agents need to know comes from the
//! command itself (`aeria guide`, `aeria brief`), so it matches the
//! installed version; Aeria installs no skills.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::Manager;

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// The skill earlier versions installed into harnesses' skill folders.
const OLD_SKILL_NAME: &str = "aeria-localization";
/// How that skill began; a skill of the same name written by anyone else is
/// left alone.
const OLD_SKILL_START: &str = "---\nname: aeria-localization\ndescription: Localize FINAL FANTASY XIV in an Aeria project with the aeria command";

/// What is connected.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentsStatusDto {
    /// Where `aeria` is installed for agents, when it is.
    pub command_path: Option<String>,
    /// The installed command is this version's.
    pub command_current: bool,
    /// The command's folder is on the user's `PATH`.
    pub on_path: bool,
    /// The open project's `AGENTS.md` has Aeria's section; `None` without a
    /// project.
    pub project_files: Option<bool>,
    /// The command this Aeria ships was not found (a development build that
    /// did not build `aeria-cli`).
    pub command_missing: bool,
}

/// The command this Aeria ships: `bin/aeria` beside the application, or
/// `aeria-cli` in a development build.
fn bundled_command() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let directory = exe.parent()?;
    let suffix = std::env::consts::EXE_SUFFIX;
    [
        directory.join("bin").join(format!("aeria{suffix}")),
        directory.join(format!("aeria-cli{suffix}")),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

/// Where the command is installed for agents.
fn command_directory() -> Option<PathBuf> {
    if cfg!(windows) {
        Some(dirs::data_local_dir()?.join("Aeria").join("bin"))
    } else {
        Some(dirs::home_dir()?.join(".local").join("bin"))
    }
}

fn installed_command() -> Option<PathBuf> {
    Some(command_directory()?.join(format!("aeria{}", std::env::consts::EXE_SUFFIX)))
}

/// Whether two files have the same size and content.
fn same_file(left: &Path, right: &Path) -> bool {
    match (std::fs::read(left), std::fs::read(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// The skill folders earlier versions installed into.
fn old_skill_folders() -> Vec<PathBuf> {
    let mut targets = Vec::new();
    if let Some(home) = dirs::home_dir() {
        for folder in [".claude", ".codex"] {
            if home.join(folder).is_dir() {
                targets.push(home.join(folder).join("skills"));
            }
        }
    }
    let hermes = std::env::var_os("HERMES_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            if cfg!(windows) {
                dirs::data_local_dir().map(|dir| dir.join("hermes"))
            } else {
                dirs::home_dir().map(|dir| dir.join(".hermes"))
            }
        });
    if let Some(hermes) = hermes.filter(|dir| dir.is_dir()) {
        targets.push(hermes.join("skills"));
        targets.extend(
            std::fs::read_dir(hermes.join("profiles"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path().join("skills")),
        );
    }
    targets
}

/// The user's `PATH` as stored, without the process's own additions.
#[cfg(windows)]
fn user_path() -> Option<String> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Environment]::GetEnvironmentVariable('Path','User')",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(not(windows))]
fn user_path() -> Option<String> {
    std::env::var("PATH").ok()
}

fn path_contains(path: &str, directory: &Path) -> bool {
    std::env::split_paths(path).any(|entry| {
        entry
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(directory.to_string_lossy().trim_end_matches(['\\', '/']))
    })
}

/// Adds a folder to the user's `PATH`; new terminals and agents see it.
#[cfg(windows)]
fn add_to_user_path(directory: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let current = user_path().unwrap_or_default();
    if path_contains(&current, directory) {
        return Ok(());
    }
    let next = if current.trim().is_empty() {
        directory.to_string_lossy().into_owned()
    } else {
        format!(
            "{};{}",
            current.trim_end_matches(';'),
            directory.to_string_lossy()
        )
    };
    // The value travels in an environment variable, so it needs no quoting.
    let status = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Environment]::SetEnvironmentVariable('Path', $env:AERIA_USER_PATH, 'User')",
        ])
        .env("AERIA_USER_PATH", next)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| format!("PATH could not be changed: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("PATH could not be changed".to_owned())
    }
}

/// `~/.local/bin` is on `PATH` in most Linux distributions.
#[cfg(not(windows))]
fn add_to_user_path(_directory: &Path) -> Result<(), String> {
    Ok(())
}

fn project_root(app: &tauri::AppHandle) -> Option<PathBuf> {
    let state = app.state::<DesktopState>();
    let project = state.lock_project().ok()?;
    project
        .as_ref()
        .map(|session| session.repository_root().to_owned())
}

fn status(app: &tauri::AppHandle) -> AgentsStatusDto {
    let bundled = bundled_command();
    let installed = installed_command().filter(|path| path.is_file());
    let command_current = match (&bundled, &installed) {
        (Some(bundled), Some(installed)) => same_file(bundled, installed),
        _ => false,
    };
    let on_path = command_directory()
        .is_some_and(|directory| user_path().is_some_and(|path| path_contains(&path, &directory)));
    let project_files = project_root(app).map(|root| {
        std::fs::read_to_string(root.join("AGENTS.md"))
            .is_ok_and(|text| text.contains("<!-- aeria:begin -->"))
    });
    AgentsStatusDto {
        command_path: installed.map(|path| path.to_string_lossy().into_owned()),
        command_current,
        on_path,
        project_files,
        command_missing: bundled.is_none(),
    }
}

/// Copies the bundled command over the installed one when it differs.
fn install_command() -> Result<(), String> {
    let bundled = bundled_command().ok_or("this build of Aeria has no aeria command")?;
    let directory = command_directory().ok_or("no folder for the aeria command")?;
    let installed = installed_command().ok_or("no folder for the aeria command")?;
    if installed.is_file() && same_file(&bundled, &installed) {
        return Ok(());
    }
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let partial = installed.with_extension("partial");
    std::fs::copy(&bundled, &partial).map_err(|error| format!("{}: {error}", partial.display()))?;
    // A command that runs right now keeps the old file open; it is replaced
    // once the command ends.
    crate::sync::replace_file(&partial, &installed)
        .map_err(|error| format!("{}: {error}", installed.display()))
}

/// Removes the skill earlier versions installed, where it is still theirs.
fn remove_old_skills() {
    for folder in old_skill_folders() {
        let directory = folder.join(OLD_SKILL_NAME);
        let file = directory.join("SKILL.md");
        if std::fs::read_to_string(&file).is_ok_and(|text| text.starts_with(OLD_SKILL_START)) {
            let _ = std::fs::remove_file(&file);
            // Only an emptied folder goes.
            let _ = std::fs::remove_dir(&directory);
        }
    }
}

/// Keeps an installed command current after Aeria updates. Called at
/// startup; does nothing unless agents were connected.
pub(crate) fn refresh_installed_command() {
    if installed_command().is_some_and(|path| path.is_file()) {
        let _ = install_command();
    }
}

fn connect(app: &tauri::AppHandle) -> CommandResult<AgentsStatusDto> {
    let fail = |message: String| CommandError::new("agentsConnect", message);
    install_command().map_err(fail)?;
    if let Some(directory) = command_directory() {
        add_to_user_path(&directory).map_err(fail)?;
    }
    remove_old_skills();
    if let Some(root) = project_root(app) {
        crate::cli::write_agent_files(&root).map_err(fail)?;
    }
    Ok(status(app))
}

#[tauri::command(rename_all = "camelCase")]
/// Reports what is connected for agent harnesses.
///
/// # Errors
///
/// Returns `internalState` when the worker fails.
pub async fn agents_status(app: tauri::AppHandle) -> CommandResult<AgentsStatusDto> {
    run_blocking(move || Ok(status(&app))).await
}

#[tauri::command(rename_all = "camelCase")]
/// Installs the `aeria` command on the user's `PATH` and writes `AGENTS.md`
/// and `CLAUDE.md` in the open project.
///
/// # Errors
///
/// Returns `agentsConnect` naming the step that failed.
pub async fn agents_connect(app: tauri::AppHandle) -> CommandResult<AgentsStatusDto> {
    run_blocking(move || connect(&app)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_entries_match_ignoring_case_and_trailing_separators() {
        let separator = if cfg!(windows) { ";" } else { ":" };
        let path = format!("/usr/bin{separator}/Home/Me/.local/bin/{separator}/opt");
        assert!(path_contains(&path, Path::new("/home/me/.local/bin")));
        assert!(!path_contains(&path, Path::new("/home/me/bin")));
    }
}
