//! What the command needs from Git: which files of `po/` changed since the
//! last commit, whether paths are committed, and making a commit.

use std::path::Path;
use std::process::{Command, Output};

fn git(root: &Path, args: &[&str]) -> Result<Output, String> {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(args)
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .output()
        .map_err(|error| format!("git cannot be run: {error}"))
}

fn succeeded(output: &Output, what: &str) -> Result<String, String> {
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{what}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Whether `root` is inside a Git working tree.
pub(crate) fn is_repository(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|output| output.status.success())
}

/// Makes `root` a Git repository unless it is inside one.
pub(crate) fn init(root: &Path) -> Result<(), String> {
    if is_repository(root) {
        return Ok(());
    }
    succeeded(&git(root, &["init", "--quiet"])?, "git init")?;
    Ok(())
}

/// The status lines of `paths`: every change not committed, untracked files
/// included, as `(code, path)`.
fn status(root: &Path, paths: &[&str]) -> Result<Vec<(String, String)>, String> {
    let mut args = vec![
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--",
    ];
    args.extend(paths);
    let text = succeeded(&git(root, &args)?, "git status")?;
    let mut records = text.split('\0').filter(|record| !record.is_empty());
    let mut changes = Vec::new();
    while let Some(record) = records.next() {
        let Some((code, path)) = record.split_at_checked(3) else {
            continue;
        };
        // A rename names the new path, then the old one.
        if code.starts_with('R') || code.starts_with('C') {
            records.next();
        }
        changes.push((code.trim().to_owned(), path.to_owned()));
    }
    Ok(changes)
}

/// The files of `po/` changed since the last commit, relative to `po/`;
/// `None` when `root` is not a Git repository.
pub(crate) fn changed_po(root: &Path) -> Option<Vec<String>> {
    if !is_repository(root) {
        return None;
    }
    let changes = status(root, &[aeria_po::PO_DIR]).ok()?;
    let prefix = format!("{}/", aeria_po::PO_DIR);
    let mut paths: Vec<String> = changes
        .into_iter()
        .filter(|(code, _)| code != "D")
        .filter_map(|(_, path)| {
            path.strip_prefix(&prefix)
                .filter(|path| {
                    std::path::Path::new(path)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("po"))
                })
                .map(str::to_owned)
        })
        .collect();
    paths.sort();
    paths.dedup();
    Some(paths)
}

/// Whether `paths` have no changes that are not committed.
pub(crate) fn committed(root: &Path, paths: &[&str]) -> Result<bool, String> {
    Ok(status(root, paths)?.is_empty())
}

/// Commits `paths` as they are with `message`. Returns the new commit's
/// short hash, or `None` when nothing changed.
pub(crate) fn commit(root: &Path, paths: &[&str], message: &str) -> Result<Option<String>, String> {
    let mut add = vec!["add", "--all", "--"];
    add.extend(paths);
    succeeded(&git(root, &add)?, "git add")?;
    let mut staged = vec!["diff", "--cached", "--quiet", "--"];
    staged.extend(paths);
    if git(root, &staged)?.status.success() {
        return Ok(None);
    }
    let mut args = vec!["commit", "--quiet", "-m", message, "--"];
    args.extend(paths);
    succeeded(&git(root, &args)?, "git commit")?;
    let hash = succeeded(
        &git(root, &["rev-parse", "--short", "HEAD"])?,
        "git rev-parse",
    )?;
    Ok(Some(hash.trim().to_owned()))
}
