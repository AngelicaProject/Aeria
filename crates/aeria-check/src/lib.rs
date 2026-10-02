//! Checks that an Aeria translation repository is safe to merge.
//!
//! A pull request merged on a Git host combines the project's PO files as
//! plain text, and files can be edited on the host's website. This crate
//! runs the checks Aeria runs when it saves a translation, without the game,
//! so a CI job can refuse a merge that would break the project.
//!
//! The checks run in stages:
//!
//! 1. **Integrity**: no merge conflict markers in project files, readable
//!    `aeria.json`, and valid pack and font settings with their font files
//!    present.
//! 2. **Translations**: every PO file reads without a problem, every
//!    `msgctxt` is an identity once per file, and every translation passes
//!    the checks of a translation against its `msgid`.
//! 3. **Merge**: compared with the base revision, a change of the game
//!    version or the project languages and removed translations are
//!    reported for review.
//!
//! Checks that need the game, such as whether a `msgid` is still the game's
//! text, remain with Aeria.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use aeria_export::PackSettings;
use aeria_fonts::{FontSettings, project_path};
use aeria_git::PROJECT_PATHS;
use aeria_knowledge::Knowledge;
use aeria_po::{PO_DIR, PoFile, SETTINGS_FILE, Settings, check_file, list, read_settings};

/// Removed translations listed individually before the rest are counted.
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
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The project's PO files, as full paths.
fn po_files(root: &Path) -> Vec<PathBuf> {
    list(root)
        .unwrap_or_default()
        .into_iter()
        .map(|path| root.join(PO_DIR).join(path))
        .collect()
}

/// Project text files where a conflict marker would break a reader.
fn text_files(root: &Path) -> Vec<PathBuf> {
    let mut files = po_files(root);
    files.extend(
        PROJECT_PATHS
            .iter()
            .filter(|path| **path != aeria_git::FONTS_DIR && **path != aeria_git::KNOWLEDGE_DIR)
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
                    "unresolved merge conflict markers on {} line(s); merge in Aeria (Pull), which joins translations per string",
                    lines.len()
                ),
            );
        }
    }

    if let Err(error) = read_settings(&root) {
        report.add(
            Severity::Error,
            Some(&root.join(SETTINGS_FILE)),
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
    let target = read_settings(&root)
        .map(|settings| settings.target_language)
        .unwrap_or_default();
    let knowledge = Knowledge::load(&root);
    for problem in &knowledge.problems {
        report.add(Severity::Error, None, problem.clone());
    }
    for path in po_files(&root) {
        let Ok(text) = fs::read_to_string(&path) else {
            report.add(Severity::Error, Some(&path), "the file is not UTF-8 text");
            continue;
        };
        let (file, problems) = PoFile::parse(&text);
        for problem in problems {
            report.add_at(
                Severity::Error,
                Some(&path),
                Some(problem.line),
                problem.message,
            );
        }
        for finding in check_file(&file, &knowledge, &target) {
            if !finding.advice {
                report.add_at(
                    Severity::Error,
                    Some(&path),
                    Some(finding.line),
                    finding.message,
                );
            }
        }
    }
}

/// Runs Git in `directory`, returning stdout when it succeeds.
fn git(directory: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new(
        std::env::var_os("AERIA_GIT_PATH")
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "git".into()),
    )
    .current_dir(directory)
    .args(args)
    .output()
    .ok()?;
    output.status.success().then_some(output.stdout)
}

/// A project file at `revision`; `None` when it does not exist there.
fn file_at(root: &Path, revision: &str, path: &str) -> Option<String> {
    git(root, &["show", &format!("{revision}:./{path}")])
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// The PO files changed since `base`, and the translations the base has
/// that the working tree lost.
fn removed_translations(root: &Path, base: &str) -> (usize, Vec<String>) {
    let changed = git(
        root,
        &["diff", "--name-only", "--relative", base, "--", PO_DIR],
    )
    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    .unwrap_or_default();
    let mut removed = Vec::new();
    let mut files = 0;
    for path in changed.lines().filter(|path| aeria_git::is_po_path(path)) {
        files += 1;
        let before = file_at(root, base, path)
            .map(|text| PoFile::parse(&text).0)
            .unwrap_or_default();
        let after = fs::read_to_string(root.join(path))
            .map(|text| PoFile::parse(&text).0)
            .unwrap_or_default();
        let kept: std::collections::HashMap<&str, &str> = after
            .entries
            .iter()
            .map(|entry| (entry.context.as_str(), entry.translation.as_str()))
            .collect();
        removed.extend(
            before
                .entries
                .iter()
                .filter(|entry| !entry.translation.is_empty())
                .filter(|entry| {
                    kept.get(entry.context.as_str())
                        .is_none_or(|translation| translation.is_empty())
                })
                .map(|entry| entry.context.clone()),
        );
    }
    (files, removed)
}

fn merge(report: &mut Reporter<'_>, base: &str) {
    let root = report.root.to_owned();
    let settings_path = root.join(SETTINGS_FILE);
    let Ok(head) = read_settings(&root) else {
        report.add(
            Severity::Notice,
            Some(&settings_path),
            "the merged project did not load (see the earlier stages), so it was not compared with the base",
        );
        return;
    };
    if git(
        &root,
        &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
    )
    .is_none()
    {
        report.add(
            Severity::Notice,
            None,
            format!(
                "cannot read revision {base:?}; fetch it first (for example with fetch-depth: 2)"
            ),
        );
        return;
    }
    let Some(old) = file_at(&root, base, SETTINGS_FILE)
        .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
    else {
        report.add(
            Severity::Notice,
            Some(&settings_path),
            "the base revision has no Aeria project; this merge adds it",
        );
        return;
    };
    if old.source_language != head.source_language || old.target_language != head.target_language {
        report.add(
            Severity::Warning,
            Some(&settings_path),
            format!(
                "changes the project languages from {:?} → {:?} to {:?} → {:?}",
                old.source_language,
                old.target_language,
                head.source_language,
                head.target_language
            ),
        );
    }
    if old.game_version != head.game_version {
        report.add(
            Severity::Warning,
            Some(&settings_path),
            format!(
                "updates the project from game version {} to {}; after the merge, every collaborator needs that game version to keep working",
                old.game_version, head.game_version
            ),
        );
    }

    let (files, removed) = removed_translations(&root, base);
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
                "removes {} translation(s) that the base has: {listed}{}; check that this was intended",
                removed.len(),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            ),
        );
    }
    report.add(
        Severity::Notice,
        None,
        format!(
            "compared with {base}: {files} PO file(s) changed, {} translation(s) removed",
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
        assert!(conflict_markers("msgstr \"=======\"\n").is_empty());
    }

    #[test]
    fn a_folder_without_a_project_fails_integrity() {
        let folder = tempfile::tempdir().expect("folder");
        let report = run_stage(Stage::Integrity, folder.path(), None);
        assert!(report.failed());
        assert_eq!(report.findings[0].path.as_deref(), Some("aeria.json"));
    }

    #[test]
    fn broken_settings_and_missing_fonts_are_errors() {
        let folder = tempfile::tempdir().expect("folder");
        fs::write(folder.path().join("aeria-pack.json"), "<<<<<<< HEAD\n{}\n").expect("write");
        let report = run_stage(Stage::Integrity, folder.path(), None);
        let paths: Vec<_> = report
            .findings
            .iter()
            .filter_map(|finding| finding.path.as_deref())
            .collect();
        assert!(paths.contains(&"aeria-pack.json"));
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
