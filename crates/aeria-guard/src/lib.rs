//! Guards an Aeria translation repository: what may reach its main branch.
//!
//! A pull request merged on a Git host combines the project's PO files as
//! plain text, and files can be edited on the host's website or in any
//! editor. This crate runs, without the game, the checks Aeria runs when it
//! saves a translation, and checks that a change keeps what only Aeria may
//! change, so a CI job can refuse a merge that would break the project.
//!
//! The checks run in stages:
//!
//! 1. **Integrity**: no merge conflict markers in project text files,
//!    readable `aeria.json`, and valid pack and font settings with their
//!    font files present.
//! 2. **Translations**: every PO file reads without a problem, every
//!    `msgctxt` is an identity once per file, every translation passes the
//!    checks of a translation against its `msgid`, and the knowledge files
//!    read.
//! 3. **Changes**: compared with the base revision. While the game version
//!    stays, the game's data in the PO files (the files, their headers,
//!    their strings, each string's `msgid` and `#.` notes) must stay too:
//!    only a game update in Aeria changes it. A change of the game version
//!    or the project languages, removed translations, and changed project
//!    settings or workflows are reported for review.
//!
//! [`review`] summarizes a change string by string for the person who
//! merges it.

mod review;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use aeria_export::PackSettings;
use aeria_fonts::{FontSettings, project_path};
use aeria_git::PROJECT_PATHS;
use aeria_knowledge::{Knowledge, KnowledgeFile};
use aeria_po::{Entry, PO_DIR, PoFile, SETTINGS_FILE, Settings, check_file, list, read_settings};

pub use review::review;

/// Strings or paths listed individually in one finding before the rest are
/// counted.
const LISTED: usize = 10;

/// One stage of the guard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Integrity,
    Translations,
    Changes,
}

impl Stage {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Integrity => "integrity",
            Self::Translations => "translations",
            Self::Changes => "changes",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Integrity => "Project integrity",
            Self::Translations => "Translations",
            Self::Changes => "Changes against the base",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    /// Worth knowing; nothing to do.
    Notice,
    /// Needs a reviewer's attention; does not fail the guard.
    Warning,
    /// The project would break, or changes what only Aeria may change;
    /// fails the guard.
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

/// Runs one stage on the project at `project_root`. The changes stage
/// compares with `base`, a Git revision; without one it reports that it was
/// skipped.
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
        Stage::Changes => match base {
            Some(base) => changes(&mut report, base),
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

/// The first [`LISTED`] items joined, and how many more there are.
fn listing(items: &[String]) -> String {
    let listed = items
        .iter()
        .take(LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    match items.len().saturating_sub(LISTED) {
        0 => listed,
        more => format!("{listed} and {more} more"),
    }
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
    files.extend(KnowledgeFile::ALL.iter().map(|file| file.path(root)));
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
pub(crate) fn git(directory: &Path, args: &[&str]) -> Option<Vec<u8>> {
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
pub(crate) fn file_at(root: &Path, revision: &str, path: &str) -> Option<Vec<u8>> {
    git(root, &["show", &format!("{revision}:./{path}")])
}

fn parse_po(bytes: Option<&[u8]>) -> PoFile {
    bytes
        .map(|bytes| PoFile::parse(&String::from_utf8_lossy(bytes)).0)
        .unwrap_or_default()
}

/// A file changed since the base, by its path in the repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChangedFile {
    /// `A`, `D`, or `M` (renames are listed as a removal and an addition).
    pub status: char,
    /// The path from the repository's top folder.
    pub path: String,
    /// The path inside the project, when the file is in it.
    pub project_path: Option<String>,
}

/// Every file changed between `base` and the working tree.
pub(crate) fn changed_files(root: &Path, base: &str) -> Vec<ChangedFile> {
    let prefix = git(root, &["rev-parse", "--show-prefix"])
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
        .unwrap_or_default();
    let listing = git(
        root,
        &[
            "diff",
            "--name-status",
            "--no-renames",
            "-z",
            base,
            "--",
            ":/",
        ],
    )
    .unwrap_or_default();
    let fields: Vec<String> = listing
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .collect();
    let mut files: Vec<ChangedFile> = fields
        .chunks(2)
        .filter_map(|pair| {
            let [status, path] = pair else { return None };
            Some(ChangedFile {
                status: status.chars().next().unwrap_or('M'),
                project_path: path.strip_prefix(&prefix).map(str::to_owned),
                path: path.clone(),
            })
        })
        .collect();
    // A run outside CI may have files Git does not track yet.
    let untracked = git(
        root,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "--full-name",
            "-z",
            ":/",
        ],
    )
    .unwrap_or_default();
    files.extend(
        untracked
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| {
                let path = String::from_utf8_lossy(path).into_owned();
                ChangedFile {
                    status: 'A',
                    project_path: path.strip_prefix(&prefix).map(str::to_owned),
                    path,
                }
            }),
    );
    files
}

/// What a changed file is to the project.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Area {
    Translations,
    Knowledge,
    /// Project settings and fonts, which change the pack every player gets.
    Settings,
    /// GitHub workflows and repository settings, which run with the
    /// repository's permissions once merged.
    GitHub,
    Other,
}

pub(crate) fn area(file: &ChangedFile) -> Area {
    if file.path.starts_with(".github/") {
        return Area::GitHub;
    }
    let Some(path) = file.project_path.as_deref() else {
        return Area::Other;
    };
    if aeria_git::is_po_path(path) && path.starts_with(&format!("{PO_DIR}/")) {
        Area::Translations
    } else if path.starts_with(&format!("{}/", aeria_git::KNOWLEDGE_DIR)) {
        Area::Knowledge
    } else if path.starts_with(&format!("{}/", aeria_git::FONTS_DIR))
        || [
            SETTINGS_FILE,
            aeria_git::ATTRIBUTES_FILE,
            aeria_git::PACK_SETTINGS_FILE,
            aeria_git::FONT_SETTINGS_FILE,
        ]
        .contains(&path)
    {
        Area::Settings
    } else {
        Area::Other
    }
}

/// How the game's data of one PO file changed: kinds of change, each with
/// the strings it concerns and the line of the first.
fn game_data_changes(before: &PoFile, after: &PoFile) -> Vec<(String, Vec<String>, usize)> {
    let mut kinds: BTreeMap<&str, (Vec<String>, usize)> = BTreeMap::new();
    let mut add = |kind: &'static str, context: &str, line: usize| {
        let (contexts, first) = kinds.entry(kind).or_insert_with(|| (Vec::new(), line));
        contexts.push(context.to_owned());
        if *first == 0 {
            *first = line;
        }
    };
    if before.header.fields != after.header.fields {
        add("changes the file header", "", 1);
    }
    let index = |entries: &[Entry]| -> BTreeMap<String, (String, Vec<String>, usize)> {
        entries
            .iter()
            .map(|entry| {
                (
                    entry.context.clone(),
                    (entry.source.clone(), entry.extracted.clone(), entry.line),
                )
            })
            .collect()
    };
    for (old, new, added, removed) in [
        (
            index(&before.entries),
            index(&after.entries),
            "adds strings",
            "removes strings",
        ),
        (
            index(&before.obsolete),
            index(&after.obsolete),
            "adds kept strings of removed game text",
            "",
        ),
    ] {
        for (context, (source, extracted, line)) in &new {
            match old.get(context) {
                None => add(added, context, *line),
                Some((old_source, old_extracted, _)) => {
                    if old_source != source {
                        add("changes the game text (msgid) of", context, *line);
                    }
                    if old_extracted != extracted {
                        add("changes the game notes (#.) of", context, *line);
                    }
                }
            }
        }
        if !removed.is_empty() {
            for context in old.keys().filter(|context| !new.contains_key(*context)) {
                add(removed, context, 0);
            }
        }
    }
    kinds
        .into_iter()
        .map(|(kind, (contexts, line))| {
            (
                kind.to_owned(),
                contexts
                    .into_iter()
                    .filter(|context| !context.is_empty())
                    .collect(),
                line,
            )
        })
        .collect()
}

/// The translations of `before` that `after` lost: strings the game has and
/// kept strings of removed game text alike.
fn removed_translations(before: &PoFile, after: &PoFile) -> Vec<String> {
    let kept: BTreeMap<&str, &str> = after
        .entries
        .iter()
        .chain(&after.obsolete)
        .map(|entry| (entry.context.as_str(), entry.translation.as_str()))
        .collect();
    before
        .entries
        .iter()
        .chain(&before.obsolete)
        .filter(|entry| !entry.translation.is_empty())
        .filter(|entry| {
            kept.get(entry.context.as_str())
                .is_none_or(|translation| translation.is_empty())
        })
        .map(|entry| entry.context.clone())
        .collect()
}

/// Compares the project settings with the base. Returns whether the game's
/// data must have stayed as the base has it: the game version and the
/// languages are the same; `None` when there is nothing to compare.
fn compare_settings(report: &mut Reporter<'_>, base: &str) -> Option<bool> {
    let root = report.root.to_owned();
    let settings_path = root.join(SETTINGS_FILE);
    let Ok(head) = read_settings(&root) else {
        report.add(
            Severity::Notice,
            Some(&settings_path),
            "the project did not load (see the earlier stages), so it was not compared with the base",
        );
        return None;
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
        return None;
    }
    let Some(old) = file_at(&root, base, SETTINGS_FILE)
        .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
    else {
        report.add(
            Severity::Notice,
            Some(&settings_path),
            "the base revision has no Aeria project; this change adds it",
        );
        return None;
    };
    let languages_kept =
        old.source_language == head.source_language && old.target_language == head.target_language;
    if !languages_kept {
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
    let game_update = old.game_version != head.game_version;
    if game_update {
        report.add(
            Severity::Warning,
            Some(&settings_path),
            format!(
                "updates the project from game version {} to {}, so the game's text is not compared; check that the update was made in Aeria, and after the merge every collaborator needs that game version",
                old.game_version, head.game_version
            ),
        );
    }
    Some(!game_update && languages_kept)
}

/// Checks one changed PO file: the game's data it must keep, when
/// `guard_game_data`, and the translations it lost.
fn compare_po_file(
    report: &mut Reporter<'_>,
    base: &str,
    file: &ChangedFile,
    guard_game_data: bool,
) -> Vec<String> {
    let root = report.root.to_owned();
    let path = file.project_path.clone().unwrap_or_default();
    let full = root.join(&path);
    let before = parse_po(file_at(&root, base, &path).as_deref());
    let after = parse_po(fs::read(&full).ok().as_deref());
    let removed = removed_translations(&before, &after);
    if !guard_game_data {
        return removed;
    }
    match file.status {
        'A' => report.add(
            Severity::Error,
            Some(&full),
            "adds a PO file; PO files come from the game and only a game update in Aeria adds them",
        ),
        'D' => report.add(
            Severity::Error,
            Some(&full),
            "removes a PO file; PO files come from the game and only a game update in Aeria removes them",
        ),
        _ => {
            for (kind, contexts, line) in game_data_changes(&before, &after) {
                let what = if contexts.is_empty() {
                    kind
                } else {
                    format!("{kind} {}", listing(&contexts))
                };
                report.add_at(
                    Severity::Error,
                    Some(&full),
                    (line > 0).then_some(line),
                    format!("{what}; the game's data changes only with a game update in Aeria"),
                );
            }
        }
    }
    removed
}

fn changes(report: &mut Reporter<'_>, base: &str) {
    let Some(guard_game_data) = compare_settings(report, base) else {
        return;
    };
    let files = changed_files(report.root, base);
    let mut removed = Vec::new();
    let mut po_changed = 0;
    let mut settings = Vec::new();
    let mut github = Vec::new();
    let mut other = Vec::new();
    for file in &files {
        match area(file) {
            Area::Translations => {
                po_changed += 1;
                removed.extend(compare_po_file(report, base, file, guard_game_data));
            }
            Area::Knowledge => {}
            Area::Settings => settings.push(file.path.clone()),
            Area::GitHub => github.push(file.path.clone()),
            Area::Other => other.push(file.path.clone()),
        }
    }

    let mut flag = |severity: Severity, items: &[String], what: &str| {
        if !items.is_empty() {
            report.add(severity, None, format!("{what}: {}", listing(items)));
        }
    };
    flag(
        Severity::Warning,
        &removed,
        &format!(
            "removes {} translation(s) that the base has; check that this was intended",
            removed.len()
        ),
    );
    flag(
        Severity::Warning,
        &settings,
        "changes project settings or fonts, which change the pack every player gets",
    );
    flag(
        Severity::Warning,
        &github,
        "changes GitHub workflows or repository files, which run with this repository's permissions once merged",
    );
    flag(
        Severity::Notice,
        &other,
        "also changes files outside the project's data",
    );
    let distinct: BTreeSet<&str> = files.iter().map(|file| file.path.as_str()).collect();
    report.add(
        Severity::Notice,
        None,
        format!(
            "compared with {base}: {} file(s) changed, {po_changed} of them PO files, {} translation(s) removed",
            distinct.len(),
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
    fn changes_without_a_base_are_skipped() {
        let folder = tempfile::tempdir().expect("folder");
        let report = run_stage(Stage::Changes, folder.path(), None);
        assert!(!report.failed());
        assert_eq!(report.findings[0].severity, Severity::Notice);
    }

    fn file(path: &str, project: Option<&str>) -> ChangedFile {
        ChangedFile {
            status: 'M',
            path: path.to_owned(),
            project_path: project.map(str::to_owned),
        }
    }

    #[test]
    fn changed_files_are_sorted_into_areas() {
        assert_eq!(
            area(&file("po/Addon.po", Some("po/Addon.po"))),
            Area::Translations
        );
        assert_eq!(
            area(&file(
                "aeria-knowledge/terms.csv",
                Some("aeria-knowledge/terms.csv")
            )),
            Area::Knowledge
        );
        for settings in [
            "aeria.json",
            "aeria-pack.json",
            "fonts/a.otf",
            ".gitattributes",
        ] {
            assert_eq!(
                area(&file(settings, Some(settings))),
                Area::Settings,
                "{settings}"
            );
        }
        assert_eq!(
            area(&file(
                ".github/workflows/x.yml",
                Some(".github/workflows/x.yml")
            )),
            Area::GitHub
        );
        assert_eq!(area(&file("README.md", Some("README.md"))), Area::Other);
        // A project in a subfolder: files beside it are not its own.
        assert_eq!(area(&file("tools/po/x.po", None)), Area::Other);
    }

    fn po(text: &str) -> PoFile {
        PoFile::parse(text).0
    }

    #[test]
    fn the_games_data_is_compared_string_by_string() {
        let before = po(
            "#. a note\nmsgctxt \"A\"\nmsgid \"One\"\nmsgstr \"Один\"\n\nmsgctxt \"B\"\nmsgid \"Two\"\nmsgstr \"\"\n",
        );
        // Translations, notes, and marks may change.
        let translated = po(
            "# translator\n#. a note\nmsgctxt \"A\"\nmsgid \"One\"\nmsgstr \"Раз\"\n\n#, fuzzy\nmsgctxt \"B\"\nmsgid \"Two\"\nmsgstr \"Два\"\n",
        );
        assert!(game_data_changes(&before, &translated).is_empty());

        let edited = po(
            "#. another note\nmsgctxt \"A\"\nmsgid \"One!\"\nmsgstr \"Один\"\n\nmsgctxt \"C\"\nmsgid \"Three\"\nmsgstr \"\"\n",
        );
        let kinds: Vec<_> = game_data_changes(&before, &edited)
            .into_iter()
            .map(|(kind, contexts, _)| format!("{kind}: {}", contexts.join(",")))
            .collect();
        assert_eq!(
            kinds,
            [
                "adds strings: C",
                "changes the game notes (#.) of: A",
                "changes the game text (msgid) of: A",
                "removes strings: B",
            ]
        );
    }

    #[test]
    fn removed_translations_include_kept_strings() {
        let before = po(
            "msgctxt \"A\"\nmsgid \"One\"\nmsgstr \"Один\"\n\n#~ msgctxt \"Z\"\n#~ msgid \"Gone\"\n#~ msgstr \"Ушло\"\n",
        );
        let after = po("msgctxt \"A\"\nmsgid \"One\"\nmsgstr \"\"\n");
        assert_eq!(removed_translations(&before, &after), ["A", "Z"]);
    }
}
