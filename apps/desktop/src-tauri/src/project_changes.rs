//! Readable changes of the project files a checkpoint commits besides the
//! translations: glossary terms, guidance lines, settings fields, and font
//! files. Used for uncommitted changes and for commits in history, so both
//! read the same way. See `docs/architecture/git.md`.

use std::collections::BTreeMap;
use std::path::Path;

use aeria_ai::guidance::{GlossaryEntry, parse_glossary};
use aeria_git::{
    ATTRIBUTES_FILE, COLLABORATION_FILE, FEED_WORKFLOW_FILE, FONT_SETTINGS_FILE, FONTS_DIR,
    GLOSSARY_FILE, GUIDANCE_FILE, GitError, GitRepository, PACK_SETTINGS_FILE, PROJECT_PATHS,
};
use serde::Serialize;

/// Details beyond this many are summarized as truncated.
const MAX_DETAILS: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectAreaDto {
    Glossary,
    Guidance,
    PackSettings,
    FontSettings,
    FontFile,
    Collaboration,
    GitAttributes,
    FeedWorkflow,
    CheckWorkflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKindDto {
    Added,
    Modified,
    Removed,
}

/// One changed term, line, or setting.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeDetailDto {
    pub kind: ChangeKindDto,
    /// The term, the setting path (`fonts › MiedingerMid › source`), or
    /// empty for a guidance line.
    pub label: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectChangeDto {
    pub path: String,
    pub area: ProjectAreaDto,
    pub kind: ChangeKindDto,
    pub details: Vec<ChangeDetailDto>,
    /// More details existed than are listed.
    pub truncated: bool,
    /// The content could not be compared (for example invalid JSON); only
    /// the file-level change is known.
    pub unreadable: bool,
    /// Size in bytes after the change (before it for a removed file).
    pub size: u64,
}

/// Whether a project-relative path is a project file (not a translation shard).
#[must_use]
pub fn is_project_path(path: &str) -> bool {
    PROJECT_PATHS
        .iter()
        .any(|project| path == *project || (*project == FONTS_DIR && path.starts_with("fonts/")))
}

fn area_of(path: &str) -> Option<ProjectAreaDto> {
    Some(match path {
        GLOSSARY_FILE => ProjectAreaDto::Glossary,
        GUIDANCE_FILE => ProjectAreaDto::Guidance,
        PACK_SETTINGS_FILE => ProjectAreaDto::PackSettings,
        FONT_SETTINGS_FILE => ProjectAreaDto::FontSettings,
        COLLABORATION_FILE => ProjectAreaDto::Collaboration,
        ATTRIBUTES_FILE => ProjectAreaDto::GitAttributes,
        FEED_WORKFLOW_FILE => ProjectAreaDto::FeedWorkflow,
        aeria_git::CHECK_WORKFLOW_FILE => ProjectAreaDto::CheckWorkflow,
        _ if path.starts_with("fonts/") => ProjectAreaDto::FontFile,
        _ => return None,
    })
}

/// Compares one project file. `None` when the path is not a project file or
/// nothing changed.
#[must_use]
pub fn compare(
    path: &str,
    before: Option<&[u8]>,
    after: Option<&[u8]>,
) -> Option<ProjectChangeDto> {
    let area = area_of(path)?;
    let kind = match (before, after) {
        (None, None) => return None,
        (None, Some(_)) => ChangeKindDto::Added,
        (Some(_), None) => ChangeKindDto::Removed,
        (Some(old), Some(new)) if old == new => return None,
        (Some(_), Some(_)) => ChangeKindDto::Modified,
    };
    let size = after.or(before).map_or(0, |bytes| bytes.len() as u64);
    let details = match area {
        ProjectAreaDto::FontFile | ProjectAreaDto::FeedWorkflow | ProjectAreaDto::CheckWorkflow => {
            Some(Vec::new())
        }
        ProjectAreaDto::Glossary => glossary_details(before, after),
        ProjectAreaDto::Guidance | ProjectAreaDto::GitAttributes => text_details(before, after),
        ProjectAreaDto::PackSettings
        | ProjectAreaDto::FontSettings
        | ProjectAreaDto::Collaboration => json_details(before, after),
    };
    let unreadable = details.is_none();
    let mut details = details.unwrap_or_default();
    let truncated = details.len() > MAX_DETAILS;
    details.truncate(MAX_DETAILS);
    Some(ProjectChangeDto {
        path: path.to_owned(),
        area,
        kind,
        details,
        truncated,
        unreadable,
        size,
    })
}

fn detail(label: String, before: Option<String>, after: Option<String>) -> Option<ChangeDetailDto> {
    let kind = match (&before, &after) {
        (None, Some(_)) => ChangeKindDto::Added,
        (Some(_), None) => ChangeKindDto::Removed,
        (Some(old), Some(new)) if old != new => ChangeKindDto::Modified,
        _ => return None,
    };
    Some(ChangeDetailDto {
        kind,
        label,
        before,
        after,
    })
}

fn glossary_entries(bytes: Option<&[u8]>) -> Option<BTreeMap<String, String>> {
    let Some(bytes) = bytes else {
        return Some(BTreeMap::new());
    };
    let glossary = parse_glossary(bytes).ok()?;
    Some(
        glossary
            .entries
            .into_iter()
            .map(|entry| (entry.term.clone(), describe_entry(&entry)))
            .collect(),
    )
}

fn describe_entry(entry: &GlossaryEntry) -> String {
    let mut text = entry.translation.clone();
    if let Some(note) = entry.note.as_deref().filter(|note| !note.is_empty()) {
        text.push_str(" — ");
        text.push_str(note);
    }
    if !entry.forbidden.is_empty() {
        text.push_str(" (≠ ");
        text.push_str(&entry.forbidden.join(", "));
        text.push(')');
    }
    text
}

fn glossary_details(before: Option<&[u8]>, after: Option<&[u8]>) -> Option<Vec<ChangeDetailDto>> {
    let (old, new) = (glossary_entries(before)?, glossary_entries(after)?);
    let mut terms: Vec<&String> = old.keys().chain(new.keys()).collect();
    terms.sort();
    terms.dedup();
    Some(
        terms
            .into_iter()
            .filter_map(|term| detail(term.clone(), old.get(term).cloned(), new.get(term).cloned()))
            .collect(),
    )
}

fn text_lines(bytes: Option<&[u8]>) -> Option<Vec<String>> {
    let Some(bytes) = bytes else {
        return Some(Vec::new());
    };
    let text = std::str::from_utf8(bytes).ok()?;
    Some(
        text.lines()
            .map(|line| line.trim_end_matches('\r').to_owned())
            .collect(),
    )
}

/// Added and removed lines from a longest-common-subsequence diff.
fn text_details(before: Option<&[u8]>, after: Option<&[u8]>) -> Option<Vec<ChangeDetailDto>> {
    let (old, new) = (text_lines(before)?, text_lines(after)?);
    // Guidance is at most 64 KiB, so the quadratic table stays small; beyond
    // that only the file-level change is reported.
    if old.len().saturating_mul(new.len()) > 4_000_000 {
        return None;
    }
    let mut lengths = vec![vec![0u32; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lengths[i][j] = if old[i] == new[j] {
                lengths[i + 1][j + 1] + 1
            } else {
                lengths[i + 1][j].max(lengths[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut details = Vec::new();
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            i += 1;
            j += 1;
        } else if j < new.len() && (i == old.len() || lengths[i][j + 1] >= lengths[i + 1][j]) {
            details.push(ChangeDetailDto {
                kind: ChangeKindDto::Added,
                label: String::new(),
                before: None,
                after: Some(new[j].clone()),
            });
            j += 1;
        } else {
            details.push(ChangeDetailDto {
                kind: ChangeKindDto::Removed,
                label: String::new(),
                before: Some(old[i].clone()),
                after: None,
            });
            i += 1;
        }
    }
    Some(details)
}

fn flatten(value: &serde_json::Value, path: &str, out: &mut BTreeMap<String, String>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                out.insert(path.to_owned(), "{}".to_owned());
            }
            for (key, child) in map {
                if path.is_empty() && key == "formatVersion" {
                    continue;
                }
                flatten(child, &join(path, key), out);
            }
        }
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                out.insert(path.to_owned(), "[]".to_owned());
            }
            for (index, item) in items.iter().enumerate() {
                // Objects that name themselves are keyed by that name, so
                // reordering or inserting entries does not show as a change
                // of every later entry.
                let key = item
                    .get("id")
                    .or_else(|| item.get("font"))
                    .and_then(serde_json::Value::as_str)
                    .map_or_else(|| (index + 1).to_string(), str::to_owned);
                flatten(item, &join(path, &key), out);
            }
        }
        serde_json::Value::String(text) => {
            out.insert(path.to_owned(), text.clone());
        }
        other => {
            out.insert(path.to_owned(), other.to_string());
        }
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path} › {key}")
    }
}

fn json_fields(bytes: Option<&[u8]>) -> Option<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    if let Some(bytes) = bytes {
        let text = std::str::from_utf8(bytes).ok()?;
        let value: serde_json::Value =
            serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
        flatten(&value, "", &mut out);
    }
    Some(out)
}

fn json_details(before: Option<&[u8]>, after: Option<&[u8]>) -> Option<Vec<ChangeDetailDto>> {
    let (old, new) = (json_fields(before)?, json_fields(after)?);
    let mut keys: Vec<&String> = old.keys().chain(new.keys()).collect();
    keys.sort();
    keys.dedup();
    Some(
        keys.into_iter()
            .filter_map(|key| detail(key.clone(), old.get(key).cloned(), new.get(key).cloned()))
            .collect(),
    )
}

/// Uncommitted changes of project files: `HEAD` against the working tree.
///
/// # Errors
///
/// Returns an error when Git fails.
pub fn pending(
    repository: &GitRepository,
    project_root: &Path,
) -> Result<Vec<ProjectChangeDto>, GitError> {
    let status = repository.status()?;
    let mut changes = Vec::new();
    for file in &status.files {
        if !is_project_path(&file.path) {
            continue;
        }
        let before = repository.file_at("HEAD", &file.path)?;
        let after = std::fs::read(aeria_fonts::project_path(project_root, &file.path)).ok();
        changes.extend(compare(&file.path, before.as_deref(), after.as_deref()));
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

/// Project file changes of one commit against its first parent.
///
/// # Errors
///
/// Returns an error for an invalid revision or when Git fails.
pub fn of_commit(
    repository: &GitRepository,
    revision: &str,
) -> Result<Vec<ProjectChangeDto>, GitError> {
    let (parent, paths) = repository.commit_project_paths(revision)?;
    let mut changes = Vec::new();
    for path in paths {
        let before = match &parent {
            Some(parent) => repository.file_at(parent, &path)?,
            None => None,
        };
        let after = repository.file_at(revision, &path)?;
        changes.extend(compare(&path, before.as_deref(), after.as_deref()));
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

/// A checkpoint message for changes without a user message: the translation
/// summary Git computed, followed by the changed project areas.
#[must_use]
pub fn checkpoint_message(
    translations: Option<&str>,
    changes: &[ProjectChangeDto],
) -> Option<String> {
    let mut areas: Vec<&str> = Vec::new();
    for change in changes {
        let name = match change.area {
            ProjectAreaDto::Glossary => "glossary",
            ProjectAreaDto::Guidance => "guidance",
            ProjectAreaDto::PackSettings => "pack settings",
            ProjectAreaDto::FontSettings | ProjectAreaDto::FontFile => "game fonts",
            ProjectAreaDto::Collaboration => "collaboration policy",
            ProjectAreaDto::GitAttributes => "Git attributes",
            ProjectAreaDto::FeedWorkflow => "feed workflow",
            ProjectAreaDto::CheckWorkflow => "merge check workflow",
        };
        if !areas.contains(&name) {
            areas.push(name);
        }
    }
    let project = (!areas.is_empty()).then(|| format!("update {}", areas.join(", ")));
    match (translations, project) {
        (Some(translations), Some(project)) => Some(format!("{translations}; {project}")),
        (Some(translations), None) => Some(translations.to_owned()),
        (None, Some(project)) => {
            let mut text = project;
            text[..1].make_ascii_uppercase();
            Some(text)
        }
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_paths_are_recognized() {
        assert!(is_project_path("aeria-glossary.csv"));
        assert!(is_project_path("fonts/Unbounded-Variable.ttf"));
        assert!(!is_project_path(".aeria/units/10.jsonl"));
        assert!(!is_project_path("fontsx"));
        assert!(is_project_path(".github/workflows/harmonia-feed.yml"));
        assert!(is_project_path(".github/workflows/aeria-check.yml"));
        assert!(!is_project_path(".github/workflows/other.yml"));
    }

    #[test]
    fn settings_are_compared_field_by_field() {
        let before = br#"{"formatVersion":1,"fonts":[{"font":"MiedingerMid","source":"tektur"},{"font":"Jupiter","source":"cormorant"}],"title":"A"}"#;
        let after = br#"{"formatVersion":1,"fonts":[{"font":"Jupiter","source":"cormorant"},{"font":"MiedingerMid","source":"unbounded"}],"title":"A","license":null}"#;
        let change = compare(FONT_SETTINGS_FILE, Some(before), Some(after)).expect("change");
        assert_eq!(change.kind, ChangeKindDto::Modified);
        assert_eq!(
            change.details,
            vec![
                ChangeDetailDto {
                    kind: ChangeKindDto::Modified,
                    label: "fonts › MiedingerMid › source".to_owned(),
                    before: Some("tektur".to_owned()),
                    after: Some("unbounded".to_owned())
                },
                ChangeDetailDto {
                    kind: ChangeKindDto::Added,
                    label: "license".to_owned(),
                    before: None,
                    after: Some("null".to_owned())
                },
            ]
        );
        assert!(compare(FONT_SETTINGS_FILE, Some(before), Some(before)).is_none());
        let broken = compare(PACK_SETTINGS_FILE, Some(b"{"), Some(before)).expect("change");
        assert!(broken.unreadable);
    }

    #[test]
    fn glossary_is_compared_by_term() {
        let before = "term,translation,note\nGil,гил,\nChocobo,чокобо,\n".as_bytes();
        let after = "term,translation,note\nChocobo,чокобо,птица\nAether,эфир,\n".as_bytes();
        let change = compare(GLOSSARY_FILE, Some(before), Some(after)).expect("change");
        let summary: Vec<(ChangeKindDto, &str)> = change
            .details
            .iter()
            .map(|d| (d.kind, d.label.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                (ChangeKindDto::Added, "Aether"),
                (ChangeKindDto::Modified, "Chocobo"),
                (ChangeKindDto::Removed, "Gil")
            ]
        );
        assert_eq!(change.details[1].after.as_deref(), Some("чокобо — птица"));
    }

    #[test]
    fn guidance_is_compared_by_line() {
        let change =
            compare(GUIDANCE_FILE, Some(b"a\nb\nc\n"), Some(b"a\nB\nc\nd\n")).expect("change");
        let lines: Vec<(ChangeKindDto, Option<&str>, Option<&str>)> = change
            .details
            .iter()
            .map(|d| (d.kind, d.before.as_deref(), d.after.as_deref()))
            .collect();
        assert_eq!(
            lines,
            [
                (ChangeKindDto::Added, None, Some("B")),
                (ChangeKindDto::Removed, Some("b"), None),
                (ChangeKindDto::Added, None, Some("d")),
            ]
        );
        let added = compare("fonts/a.ttf", None, Some(&[1, 2, 3])).expect("font");
        assert_eq!(
            (added.area, added.kind, added.size),
            (ProjectAreaDto::FontFile, ChangeKindDto::Added, 3)
        );
    }

    #[test]
    fn default_messages_name_the_changed_areas() {
        let change = |path: &str| compare(path, None, Some(b"x")).expect("change");
        let changes = [
            change(GLOSSARY_FILE),
            change("fonts/a.ttf"),
            change(FONT_SETTINGS_FILE),
        ];
        assert_eq!(
            checkpoint_message(None, &changes).as_deref(),
            Some("Update glossary, game fonts")
        );
        assert_eq!(
            checkpoint_message(Some("Translate 3 strings"), &changes[..1]).as_deref(),
            Some("Translate 3 strings; update glossary")
        );
        assert_eq!(checkpoint_message(None, &[]), None);
    }
}
