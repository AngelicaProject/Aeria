//! The changed files of the working tree or of a commit, each with how it
//! changed and what a commit does with it: translations and project files
//! are what Aeria commits, every other file is listed as left out. A PO file
//! carries how many of its strings changed, and a project file its readable
//! change. The working tree's files are split as Git splits them: the
//! changes the index holds (staged) and those of the working tree against
//! the index. See `docs/architecture/git.md`.

use std::collections::BTreeMap;

use aeria_git::{
    EntryChange, FileChangeKind, FileStatus, FileVersions, GitError, GitRepository, PendingCache,
    diff_file, is_po_path,
};
use serde::Serialize;

use crate::git::GitFileKindDto;
use crate::project_changes::{self, ProjectChangeDto, is_project_path};

/// What a commit made in Aeria does with a file.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileGroupDto {
    /// A PO file of `po/`.
    Translations,
    /// A project file Aeria commits with the translations.
    Project,
    /// Any other file: Aeria commits it only when it is staged.
    Other,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeDto {
    pub path: String,
    /// The path before a rename or copy.
    pub original_path: Option<String>,
    pub kind: GitFileKindDto,
    pub group: FileGroupDto,
    /// For a PO file, how many of its strings changed.
    pub strings: Option<usize>,
    /// For a project file whose content changed, the readable change.
    pub project: Option<ProjectChangeDto>,
}

/// The uncommitted files: what the index holds, and the working tree's
/// changes against the index.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingChangesDto {
    pub staged: Vec<FileChangeDto>,
    pub changes: Vec<FileChangeDto>,
}

/// The group a project-relative path belongs to.
#[must_use]
pub fn group_of(path: &str) -> FileGroupDto {
    if is_po_path(path) {
        FileGroupDto::Translations
    } else if is_project_path(path) {
        FileGroupDto::Project
    } else {
        FileGroupDto::Other
    }
}

fn change(
    path: &str,
    original_path: Option<&String>,
    kind: FileChangeKind,
    before: Option<&[u8]>,
    after: Option<&[u8]>,
    strings: Option<usize>,
) -> FileChangeDto {
    let group = group_of(path);
    FileChangeDto {
        path: path.to_owned(),
        original_path: original_path.cloned(),
        kind: kind.into(),
        group,
        strings: (group == FileGroupDto::Translations)
            .then(|| strings.unwrap_or_else(|| diff_file(path, before, after).len())),
        project: (group == FileGroupDto::Project)
            .then(|| project_changes::compare(path, before, after))
            .flatten(),
    }
}

/// The changed files of a commit in path order, with the string counts of
/// PO files (keyed by path) and the readable changes of project files.
#[must_use]
pub fn changed_files(
    files: Vec<FileStatus>,
    strings: &BTreeMap<String, usize>,
    project: Vec<ProjectChangeDto>,
) -> Vec<FileChangeDto> {
    let mut project: BTreeMap<String, ProjectChangeDto> = project
        .into_iter()
        .map(|change| (change.path.clone(), change))
        .collect();
    let mut changed: Vec<FileChangeDto> = files
        .into_iter()
        .map(|file| {
            let group = group_of(&file.path);
            FileChangeDto {
                strings: (group == FileGroupDto::Translations)
                    .then(|| strings.get(&file.path).copied().unwrap_or_default()),
                project: project.remove(&file.path),
                group,
                kind: file.kind.into(),
                original_path: file.original_path,
                path: file.path,
            }
        })
        .collect();
    changed.sort_by(|a, b| a.path.cmp(&b.path));
    changed
}

/// The working tree's uncommitted files. A file's staged side compares the
/// index with `HEAD`, and its other side the working tree with the index.
///
/// # Errors
///
/// Returns an error when Git fails.
pub fn working_changes(
    repository: &GitRepository,
    cache: &mut PendingCache,
) -> Result<WorkingChangesDto, GitError> {
    let status = repository.status()?;
    // Strings of PO files changed only in the working tree, against `HEAD`.
    let pending: BTreeMap<String, usize> = repository
        .pending_files(cache)?
        .into_iter()
        .map(|(path, changes)| (path, changes.len()))
        .collect();
    let compared: Vec<String> = status
        .files
        .iter()
        .filter(|file| file.index.is_some() || is_project_path(&file.path))
        .map(|file| file.path.clone())
        .collect();
    let versions: BTreeMap<&str, FileVersions> = compared
        .iter()
        .map(String::as_str)
        .zip(repository.committed_and_staged(&compared)?)
        .collect();
    let mut working = WorkingChangesDto::default();
    for file in &status.files {
        let (committed, staged) = versions
            .get(file.path.as_str())
            .map_or((None, None), |(committed, staged)| {
                (committed.as_deref(), staged.as_deref())
            });
        if file.worktree == Some(FileChangeKind::Conflicted) {
            working.changes.push(change(
                &file.path,
                None,
                FileChangeKind::Conflicted,
                None,
                None,
                Some(0),
            ));
            continue;
        }
        if let Some(kind) = file.index {
            working.staged.push(change(
                &file.path,
                file.original_path.as_ref(),
                kind,
                committed,
                staged,
                None,
            ));
        }
        if let Some(kind) = file.worktree {
            let read = repository.working_file(&file.path)?;
            let before = if file.index.is_some() {
                staged
            } else {
                committed
            };
            let strings = if file.index.is_some() {
                None
            } else {
                Some(pending.get(&file.path).copied().unwrap_or_default())
            };
            working.changes.push(change(
                &file.path,
                None,
                kind,
                before,
                read.as_deref(),
                strings,
            ));
        }
    }
    working.staged.sort_by(|a, b| a.path.cmp(&b.path));
    working.changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(working)
}

/// The string changes of one PO file on one side: what the index holds
/// against `HEAD`, or the working tree against the index.
///
/// # Errors
///
/// Returns an error when Git fails or the path is invalid.
pub fn file_strings(
    repository: &GitRepository,
    path: &str,
    staged: bool,
) -> Result<Vec<EntryChange>, GitError> {
    let paths = [path.to_owned()];
    let (committed, index) = repository
        .committed_and_staged(&paths)?
        .into_iter()
        .next()
        .unwrap_or_default();
    let status = repository.status()?;
    let has_staged = status
        .files
        .iter()
        .any(|file| file.path == path && file.index.is_some());
    Ok(if staged {
        diff_file(path, committed.as_deref(), index.as_deref())
    } else {
        let before = if has_staged { index } else { committed };
        let after = repository.working_file(path)?;
        diff_file(path, before.as_deref(), after.as_deref())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, kind: FileChangeKind) -> FileStatus {
        FileStatus {
            path: path.to_owned(),
            original_path: None,
            kind,
            staged: false,
            index: None,
            worktree: Some(kind),
        }
    }

    #[test]
    fn files_are_grouped_by_what_a_commit_does_with_them() {
        let terms = project_changes::compare(
            "aeria-knowledge/terms.csv",
            None,
            Some(b"term,translation\nGil,gil\n"),
        )
        .expect("terms");
        let strings = BTreeMap::from([("po/Addon.po".to_owned(), 3)]);
        let changed = changed_files(
            vec![
                file("notes.txt", FileChangeKind::Untracked),
                file("po/Addon.po", FileChangeKind::Modified),
                file("aeria-knowledge/terms.csv", FileChangeKind::Added),
                file("po/Item.po", FileChangeKind::Modified),
            ],
            &strings,
            vec![terms],
        );
        let summary: Vec<(&str, FileGroupDto, Option<usize>, bool)> = changed
            .iter()
            .map(|file| {
                (
                    file.path.as_str(),
                    file.group,
                    file.strings,
                    file.project.is_some(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "aeria-knowledge/terms.csv",
                    FileGroupDto::Project,
                    None,
                    true
                ),
                ("notes.txt", FileGroupDto::Other, None, false),
                ("po/Addon.po", FileGroupDto::Translations, Some(3), false),
                ("po/Item.po", FileGroupDto::Translations, Some(0), false),
            ]
        );
    }
}
