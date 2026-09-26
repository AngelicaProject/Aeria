//! Checks that an Aeria translation repository is safe to merge.
//!
//! A pull request merged on a Git host combines unit shards as plain text,
//! and files can be edited on the host's website. Aeria itself merges per
//! translation unit and validates the project, but a host does neither. This
//! crate runs the same strict readers Aeria uses, without the game source, so
//! a CI job can refuse a merge that would leave the project unreadable.
//!
//! The checks run in stages:
//!
//! 1. **Integrity**: no merge conflict markers in project files, a readable
//!    manifest, and valid collaboration, pack, and font settings with their
//!    font files present.
//! 2. **Translations**: every unit shard loads strictly (identity, bindings,
//!    and every target as a valid structured string), and every managed file
//!    is byte for byte what Aeria writes.
//! 3. **Merge**: compared with the base revision, a change of the game
//!    source or the project languages and removed translation units are
//!    reported for review.
//!
//! Checks that need the game source, such as tag compatibility with the
//! original text, remain with Aeria, which runs them when it opens or syncs
//! the project.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use aeria_export::PackSettings;
use aeria_fonts::{FontSettings, project_path};
use aeria_git::{CollaborationSettings, PROJECT_PATHS};
use aeria_workspace::{Workspace, WorkspaceStore, WorkspaceStoreError};

/// The managed workspace directory.
const AERIA_DIRECTORY: &str = ".aeria";
/// Removed units listed individually before the rest are only counted.
const LISTED_REMOVALS: usize = 10;

/// One stage of the check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Integrity,
    Translations,
    Merge,
}

impl Stage {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Integrity => "integrity",
            Self::Translations => "translations",
            Self::Merge => "merge",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Integrity => "Project integrity",
            Self::Translations => "Translations",
            Self::Merge => "Merge into the base branch",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    /// Worth knowing; nothing to do.
    Notice,
    /// Needs a reviewer's attention; does not fail the check.
    Warning,
    /// The project would break; fails the check.
    Error,
}

/// One result of a stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    /// Project-relative path of the file concerned, with `/` separators.
    pub path: Option<String>,
    /// One-based line in `path`.
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.path, self.line) {
            (Some(path), Some(line)) => write!(formatter, "{path}:{line}: {}", self.message),
            (Some(path), None) => write!(formatter, "{path}: {}", self.message),
            _ => formatter.write_str(&self.message),
        }
    }
}

/// The findings of one stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageReport {
    pub stage: Stage,
    pub findings: Vec<Finding>,
}

impl StageReport {
    #[must_use]
    pub fn failed(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
    }
}

/// Runs one stage on the project at `project_root`. The merge stage compares
/// with `base`, a Git revision; without one it reports that it was skipped.
#[must_use]
pub fn run_stage(stage: Stage, project_root: &Path, base: Option<&str>) -> StageReport {
    let mut findings = Vec::new();
    let mut report = Reporter {
        root: project_root,
        findings: &mut findings,
    };
    match stage {
        Stage::Integrity => integrity(&mut report),
        Stage::Translations => translations(&mut report),
        Stage::Merge => match base {
            Some(base) => merge(&mut report, base),
            None => report.add(
                Severity::Notice,
                None,
                "no base revision was given, so nothing was compared",
            ),
        },
    }
    StageReport { stage, findings }
}

struct Reporter<'a> {
    root: &'a Path,
    findings: &'a mut Vec<Finding>,
}

impl Reporter<'_> {
    fn add(&mut self, severity: Severity, path: Option<&Path>, message: impl Into<String>) {
        self.add_at(severity, path, None, message);
    }

    fn add_at(
        &mut self,
        severity: Severity,
        path: Option<&Path>,
        line: Option<usize>,
        message: impl Into<String>,
    ) {
        let message = message.into();
        // Messages from the readers name absolute paths; the path is shown
        // separately, relative to the project.
        let absolute = self.root.to_string_lossy();
        let message = message
            .replace(&format!("{absolute}{}", std::path::MAIN_SEPARATOR), "")
            .replace(absolute.as_ref(), ".");
        self.findings.push(Finding {
            severity,
            path: path.map(|path| relative(self.root, path)),
            line,
            message,
        });
    }

    fn workspace_error(&mut self, error: &WorkspaceStoreError) {
        let message = match error {
            WorkspaceStoreError::MigrationRequired { .. } => format!(
                "{error}; open the project in Aeria, which updates it, and commit the result"
            ),
            _ => error.to_string(),
        };
        self.add(Severity::Error, error.path(), message);
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Project text files where a conflict marker would break a reader.
fn text_files(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![root.join(AERIA_DIRECTORY).join("manifest.json")];
    if let Ok(entries) = fs::read_dir(root.join(AERIA_DIRECTORY).join("units")) {
        let mut shards: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "jsonl")
            })
            .collect();
        shards.sort();
        files.extend(shards);
    }
    files.extend(
        PROJECT_PATHS
            .iter()
            .filter(|path| **path != aeria_git::FONTS_DIR)
            .map(|path| project_path(root, path)),
    );
    files.retain(|path| path.is_file());
    files
}

/// One-based lines of Git conflict markers in `text`.
fn conflict_markers(text: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            line.starts_with("<<<<<<< ")
                || line.starts_with(">>>>>>> ")
                || line.starts_with("||||||| ")
                || *line == "======="
        })
        .map(|(index, _)| index + 1)
        .collect()
}

fn integrity(report: &mut Reporter<'_>) {
    let root = report.root.to_owned();
    for path in text_files(&root) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let lines = conflict_markers(&text);
        if let Some(first) = lines.first() {
            report.add_at(
                Severity::Error,
                Some(&path),
                Some(*first),
                format!(
                    "unresolved merge conflict markers on {} line(s); merge in Aeria (Pull or Sync), which merges translations per string",
                    lines.len()
                ),
            );
        }
    }

    if let Err(error) = WorkspaceStore::new(&root).read_metadata() {
        report.workspace_error(&error);
    }

    if let Err(error) = CollaborationSettings::load(&root) {
        report.add(
            Severity::Error,
            Some(&root.join(aeria_git::COLLABORATION_FILE)),
            error.to_string(),
        );
    }
    if let Err(error) = PackSettings::load(&root) {
        report.add(
            Severity::Error,
            Some(&root.join(aeria_export::PACK_SETTINGS_FILE)),
            error.to_string(),
        );
    }
    match FontSettings::load(&root) {
        Ok(Some(settings)) => {
            let file = root.join(aeria_fonts::FONT_SETTINGS_FILE);
            for source in &settings.sources {
                for relative in [&source.file, &source.license_file] {
                    if !project_path(&root, relative).is_file() {
                        report.add(
                            Severity::Error,
                            Some(&file),
                            format!(
                                "font source {:?} names {relative}, which is missing",
                                source.id
                            ),
                        );
                    }
                }
            }
        }
        Ok(None) => {}
        Err(error) => report.add(
            Severity::Error,
            Some(&root.join(aeria_fonts::FONT_SETTINGS_FILE)),
            error.to_string(),
        ),
    }
}

fn translations(report: &mut Reporter<'_>) {
    let root = report.root.to_owned();
    match WorkspaceStore::new(&root).non_canonical_files() {
        Ok(files) => {
            for path in files {
                report.add(
                    Severity::Error,
                    Some(&path),
                    "the file is valid but not written the way Aeria writes it, so it was edited or merged outside Aeria; open the project in Aeria, save any string, and commit, or merge in Aeria instead of on the website",
                );
            }
        }
        Err(error) => report.workspace_error(&error),
    }
}

/// Runs Git in `directory`, returning stdout when it succeeds.
fn git(directory: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// The project's `.aeria/` at `revision`, extracted to a temporary folder.
fn workspace_at(
    root: &Path,
    revision: &str,
) -> Result<Option<(tempfile::TempDir, Workspace)>, String> {
    let prefix = git(root, &["rev-parse", "--show-prefix"])
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
        .ok_or("the project is not in a Git repository")?;
    let tree = format!("{prefix}{AERIA_DIRECTORY}/");
    let listing = git(
        root,
        &[
            "ls-tree",
            "-r",
            "--name-only",
            "--full-tree",
            revision,
            &tree,
        ],
    )
    .ok_or_else(|| {
        format!(
            "cannot read revision {revision:?}; fetch it first (for example with fetch-depth: 2)"
        )
    })?;
    let names: Vec<String> = String::from_utf8_lossy(&listing)
        .lines()
        .map(str::to_owned)
        .collect();
    if names.is_empty() {
        return Ok(None);
    }
    let folder = tempfile::tempdir().map_err(|error| error.to_string())?;
    for name in &names {
        let bytes = git(root, &["show", &format!("{revision}:{name}")])
            .ok_or_else(|| format!("cannot read {name} at {revision}"))?;
        let local = folder
            .path()
            .join(name.strip_prefix(&prefix).unwrap_or(name));
        if let Some(parent) = local.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&local, bytes).map_err(|error| error.to_string())?;
    }
    let workspace = WorkspaceStore::new(folder.path())
        .load()
        .map_err(|error| format!("the base project cannot be loaded for comparison: {error}"))?;
    Ok(Some((folder, workspace)))
}

fn merge(report: &mut Reporter<'_>, base: &str) {
    let root = report.root.to_owned();
    let manifest = root.join(AERIA_DIRECTORY).join("manifest.json");
    let head = match WorkspaceStore::new(&root).load() {
        Ok(workspace) => workspace,
        Err(error) => {
            report.add(
                Severity::Notice,
                error.path(),
                "the merged project did not load (see the earlier stages), so it was not compared with the base",
            );
            return;
        }
    };
    let base_workspace = match workspace_at(&root, base) {
        Ok(Some((_folder, workspace))) => workspace,
        Ok(None) => {
            report.add(
                Severity::Notice,
                Some(&manifest),
                "the base revision has no Aeria project; this merge adds it",
            );
            return;
        }
        Err(message) => {
            report.add(Severity::Notice, None, message);
            return;
        }
    };

    let (old, new) = (base_workspace.metadata(), head.metadata());
    if old.source_language() != new.source_language()
        || old.target_language() != new.target_language()
    {
        report.add(
            Severity::Warning,
            Some(&manifest),
            format!(
                "changes the project languages from {:?} → {:?} to {:?} → {:?}",
                old.source_language(),
                old.target_language(),
                new.source_language(),
                new.target_language()
            ),
        );
    }
    if old.source_content_id() != new.source_content_id() {
        report.add(
            Severity::Warning,
            Some(&manifest),
            "updates the project to other game data (a source update); after the merge, every collaborator needs the same game version to keep working",
        );
    }

    let removed: Vec<String> = base_workspace
        .units()
        .filter(|unit| head.unit(unit.id()).is_none())
        .map(|unit| {
            let binding = unit.source_binding();
            format!(
                "{}:{}:{}:{}",
                binding.sheet_name(),
                binding.row_id(),
                binding.subrow_id(),
                binding.column_index()
            )
        })
        .collect();
    if !removed.is_empty() {
        let listed = removed
            .iter()
            .take(LISTED_REMOVALS)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let more = removed.len().saturating_sub(LISTED_REMOVALS);
        report.add(
            Severity::Warning,
            None,
            format!(
                "removes {} translation unit(s) that the base has: {listed}{}; Aeria never removes units, so check where this came from",
                removed.len(),
                if more > 0 { format!(" and {more} more") } else { String::new() }
            ),
        );
    }
    let added = head
        .units()
        .filter(|unit| base_workspace.unit(unit.id()).is_none())
        .count();
    report.add(
        Severity::Notice,
        None,
        format!(
            "compared with {base}: {added} unit(s) added, {} removed",
            removed.len()
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_markers_are_whole_marker_lines() {
        let text = "a\n<<<<<<< HEAD\nb\n=======\nc\n>>>>>>> branch\n== not a marker\n";
        assert_eq!(conflict_markers(text), [2, 4, 6]);
        assert!(conflict_markers("{\"target\":\"=======\"}\n").is_empty());
    }

    #[test]
    fn a_folder_without_a_project_fails_integrity() {
        let folder = tempfile::tempdir().expect("folder");
        let report = run_stage(Stage::Integrity, folder.path(), None);
        assert!(report.failed());
        assert_eq!(report.findings[0].path.as_deref(), Some(".aeria"));
    }

    #[test]
    fn broken_settings_and_missing_fonts_are_errors() {
        let folder = tempfile::tempdir().expect("folder");
        fs::write(
            folder.path().join("aeria-collaboration.json"),
            "<<<<<<< HEAD\n{}\n",
        )
        .expect("write");
        let report = run_stage(Stage::Integrity, folder.path(), None);
        let paths: Vec<_> = report
            .findings
            .iter()
            .filter_map(|finding| finding.path.as_deref())
            .collect();
        assert!(paths.contains(&"aeria-collaboration.json"));
        let marker = report
            .findings
            .iter()
            .find(|finding| finding.message.contains("conflict markers"))
            .expect("marker finding");
        assert_eq!(marker.line, Some(1));
    }

    #[test]
    fn merge_without_a_base_is_skipped() {
        let folder = tempfile::tempdir().expect("folder");
        let report = run_stage(Stage::Merge, folder.path(), None);
        assert!(!report.failed());
        assert_eq!(report.findings[0].severity, Severity::Notice);
    }
}
