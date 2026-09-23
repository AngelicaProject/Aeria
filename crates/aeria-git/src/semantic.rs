//! Translation-unit-level interpretation of Git history and changes.
//!
//! Workspace Format v1 stores one unit per JSONL line in a shard selected by
//! its stable ID, so a unit's history is exactly the history of that line.
//! Everything here is derived deterministically from repository data.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use std::str::FromStr;

use aeria_core::{ReviewState, TranslationUnit, TranslationUnitId};
use aeria_workspace::{decode_unit_record, decode_unit_shard, unit_shard_path};

use crate::GitError;
use crate::repository::{CommitSummary, GitRepository, parse_commit, validate_revision};

const UNITS_PATH: &str = ".aeria/units";
const RECORD_SEPARATOR: char = '\x1e';
/// Sheet names listed in a generated checkpoint message.
const SHOWN_SHEETS: usize = 3;

/// The before/after record of every unit changed by one commit.
type CommitUnitChanges = BTreeMap<TranslationUnitId, (RecordVersion, RecordVersion)>;

/// How a translation unit changed between two states.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnitChangeKind {
    Added,
    Modified,
    Removed,
}

/// One translation-unit change between two workspace states.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitChange {
    pub id: TranslationUnitId,
    pub kind: UnitChangeKind,
    pub before: Option<TranslationUnit>,
    pub after: Option<TranslationUnit>,
}

impl UnitChange {
    fn new(before: Option<TranslationUnit>, after: Option<TranslationUnit>) -> Option<Self> {
        let (id, kind) = match (&before, &after) {
            (None, Some(after)) => (after.id(), UnitChangeKind::Added),
            (Some(before), None) => (before.id(), UnitChangeKind::Removed),
            (Some(before), Some(after)) if before != after => {
                (after.id(), UnitChangeKind::Modified)
            }
            _ => return None,
        };
        Some(Self {
            id,
            kind,
            before,
            after,
        })
    }

    /// Returns the most recent version of the unit.
    #[must_use]
    pub fn latest(&self) -> Option<&TranslationUnit> {
        self.after.as_ref().or(self.before.as_ref())
    }

    /// Returns whether the target text changed.
    #[must_use]
    pub fn target_changed(&self) -> bool {
        self.before.as_ref().map(TranslationUnit::target_macro)
            != self.after.as_ref().map(TranslationUnit::target_macro)
    }

    /// Returns whether the review state changed.
    #[must_use]
    pub fn review_changed(&self) -> bool {
        self.before.as_ref().map(TranslationUnit::review_state)
            != self.after.as_ref().map(TranslationUnit::review_state)
    }

    /// Returns whether the translator note changed.
    #[must_use]
    pub fn note_changed(&self) -> bool {
        self.before.as_ref().map(TranslationUnit::translator_note)
            != self.after.as_ref().map(TranslationUnit::translator_note)
    }
}

/// One historical version of a unit record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordVersion {
    /// The unit did not exist.
    Absent,
    /// A valid Workspace Format v1 record.
    Valid(TranslationUnit),
    /// The historical record is not valid Workspace Format v1 data, for
    /// example because it was edited by hand. It is reported, never repaired.
    Invalid { message: String },
}

/// One commit that changed a unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitRevision {
    pub commit: CommitSummary,
    pub kind: UnitChangeKind,
    pub before: RecordVersion,
    pub after: RecordVersion,
}

/// The author and commit behind one attributed change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attribution {
    pub commit: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: i64,
}

impl From<&CommitSummary> for Attribution {
    fn from(commit: &CommitSummary) -> Self {
        Self {
            commit: commit.id.clone(),
            author_name: commit.author_name.clone(),
            author_email: commit.author_email.clone(),
            authored_at: commit.authored_at,
        }
    }
}

/// The history of one translation unit, newest first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitHistory {
    pub id: TranslationUnitId,
    /// The uncommitted working-tree change, if any.
    pub pending: Option<UnitChange>,
    pub revisions: Vec<UnitRevision>,
    /// Whether older revisions exist beyond the requested limit.
    pub truncated: bool,
    /// Who introduced the committed target text.
    pub translated_by: Option<Attribution>,
    /// Who marked the committed target text reviewed, when it is reviewed.
    pub reviewed_by: Option<Attribution>,
}

/// Committed attribution of one current unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitAttribution {
    pub id: TranslationUnitId,
    /// Who introduced the current target text.
    pub translated_by: Option<Attribution>,
    /// Who marked the current target text reviewed, when it is reviewed.
    pub reviewed_by: Option<Attribution>,
    /// The newest commit that changed the unit record.
    pub last_changed_by: Option<Attribution>,
}

/// Current committed units credited to one author.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContributorSummary {
    pub name: String,
    pub email: String,
    /// Units whose current target text this author introduced.
    pub translated: usize,
    /// Units whose current review this author made.
    pub reviewed: usize,
    pub last_authored_at: i64,
}

impl GitRepository {
    /// Returns uncommitted translation-unit changes relative to `HEAD`.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails or a committed or working-tree shard
    /// is not valid Workspace Format v1 data.
    pub fn pending_changes(&self) -> Result<Vec<UnitChange>, GitError> {
        let status = self.status()?;
        let shards: BTreeSet<&str> = status
            .files
            .iter()
            .flat_map(|file| std::iter::once(&file.path).chain(file.original_path.as_ref()))
            .map(String::as_str)
            .filter(|path| is_shard_path(path))
            .collect();
        let head = status.head.is_some();
        let committed = if head {
            let specs: Vec<String> = shards
                .iter()
                .map(|path| format!("HEAD:{}", self.top_level_path(path)))
                .collect();
            self.read_blobs(&specs)?
        } else {
            vec![None; shards.len()]
        };

        let mut changes = Vec::new();
        for (path, before) in shards.into_iter().zip(committed) {
            let after = self.read_working_file(path)?;
            changes.extend(diff_shards(path, before.as_deref(), after.as_deref())?);
        }
        Ok(changes)
    }

    /// Returns the translation-unit changes introduced by one commit,
    /// relative to its first parent.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid revision, a Git failure, or invalid
    /// shard data in either commit.
    pub fn commit_changes(
        &self,
        revision: &str,
    ) -> Result<(CommitSummary, Vec<UnitChange>), GitError> {
        validate_revision(revision)?;
        let commit = self.commit(revision)?;
        let mut args = vec![
            "diff-tree",
            "-r",
            "-z",
            "--name-only",
            "--no-renames",
            "--relative",
        ];
        if let Some(parent) = commit.parents.first() {
            args.push(parent);
        } else {
            args.push("--root");
        }
        args.extend([commit.id.as_str(), "--", UNITS_PATH]);
        let text = self.run_text(&args)?;
        let shards: Vec<&str> = text
            .split('\0')
            .filter(|path| is_shard_path(path))
            .collect();

        let mut specs = Vec::with_capacity(shards.len() * 2);
        for path in &shards {
            let path = self.top_level_path(path);
            specs.push(
                commit
                    .parents
                    .first()
                    .map(|parent| format!("{parent}:{path}")),
            );
            specs.push(Some(format!("{}:{path}", commit.id)));
        }
        let requested: Vec<String> = specs.iter().flatten().cloned().collect();
        let mut blobs = self.read_blobs(&requested)?.into_iter();
        let mut changes = Vec::new();
        for (index, path) in shards.iter().enumerate() {
            let before = if specs[index * 2].is_some() {
                blobs.next().flatten()
            } else {
                None
            };
            let after = blobs.next().flatten();
            changes.extend(diff_shards(path, before.as_deref(), after.as_deref())?);
        }
        Ok((commit, changes))
    }

    /// Returns the history of one translation unit, newest first, with at
    /// most `limit` committed revisions, and who translated and reviewed its
    /// committed text.
    ///
    /// Merge commits are not listed: a unit change is attributed to the
    /// commit that authored it on its original branch.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails or the current shard is invalid.
    pub fn unit_history(
        &self,
        id: TranslationUnitId,
        limit: usize,
    ) -> Result<UnitHistory, GitError> {
        let path = unit_shard_path(id);
        let has_head = self.head()?.is_some();

        let committed = if has_head {
            self.read_blobs(&[format!("HEAD:{}", self.top_level_path(&path))])?
                .pop()
                .flatten()
        } else {
            None
        };
        let working = self.read_working_file(&path)?;
        let pending = diff_shards(&path, committed.as_deref(), working.as_deref())?
            .into_iter()
            .find(|change| change.id == id);
        let current = match committed.as_deref() {
            Some(bytes) => decode_unit_shard(bytes, Path::new(&path))?
                .into_iter()
                .find(|unit| unit.id() == id),
            None => None,
        };

        let mut revisions = Vec::new();
        if has_head {
            let pickaxe = format!("-G{id}");
            for (commit, mut changes) in self.scan_history(&path, Some(&pickaxe))? {
                if let Some((before, after)) = changes.remove(&id) {
                    revisions.push(UnitRevision {
                        kind: change_kind(&before, &after),
                        commit,
                        before,
                        after,
                    });
                }
            }
        }
        let (translated_by, reviewed_by) = credit(current.as_ref(), &revisions);
        let truncated = revisions.len() > limit;
        revisions.truncate(limit);
        Ok(UnitHistory {
            id,
            pending,
            revisions,
            truncated,
            translated_by,
            reviewed_by,
        })
    }

    /// Returns committed attribution for every unit in the `HEAD` workspace,
    /// ordered by ID. This reads the complete unit history once.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails or a committed shard is invalid.
    pub fn attribution(&self) -> Result<Vec<UnitAttribution>, GitError> {
        if self.head()?.is_none() {
            return Ok(Vec::new());
        }
        let listing = self.run_text(&[
            "ls-tree",
            "-z",
            "--name-only",
            "HEAD",
            "--",
            ".aeria/units/",
        ])?;
        let shards: Vec<&str> = listing
            .split('\0')
            .filter(|path| is_shard_path(path))
            .collect();
        let specs: Vec<String> = shards
            .iter()
            .map(|path| format!("HEAD:{}", self.top_level_path(path)))
            .collect();
        let mut current = BTreeMap::new();
        for (path, bytes) in shards.iter().zip(self.read_blobs(&specs)?) {
            if let Some(bytes) = bytes {
                for unit in decode_unit_shard(&bytes, Path::new(path))? {
                    current.insert(unit.id(), unit);
                }
            }
        }

        let mut revisions: BTreeMap<TranslationUnitId, Vec<UnitRevision>> = BTreeMap::new();
        for (commit, changes) in self.scan_history(UNITS_PATH, None)? {
            for (id, (before, after)) in changes {
                if current.contains_key(&id) {
                    revisions.entry(id).or_default().push(UnitRevision {
                        kind: change_kind(&before, &after),
                        commit: commit.clone(),
                        before,
                        after,
                    });
                }
            }
        }
        Ok(current
            .into_values()
            .map(|unit| {
                let history = revisions.remove(&unit.id()).unwrap_or_default();
                let (translated_by, reviewed_by) = credit(Some(&unit), &history);
                UnitAttribution {
                    id: unit.id(),
                    translated_by,
                    reviewed_by,
                    last_changed_by: history.first().map(|revision| (&revision.commit).into()),
                }
            })
            .collect())
    }

    /// Counts current committed units by the authors who translated and
    /// reviewed their current text.
    ///
    /// # Errors
    ///
    /// Returns an error when attribution fails.
    pub fn contributors(&self) -> Result<Vec<ContributorSummary>, GitError> {
        Ok(summarize_contributors(&self.attribution()?))
    }

    /// Reads `git log -p` for `pathspec` (newest first) and returns, per
    /// commit, the before/after record of every unit whose line changed.
    fn scan_history(
        &self,
        pathspec: &str,
        pickaxe: Option<&str>,
    ) -> Result<Vec<(CommitSummary, CommitUnitChanges)>, GitError> {
        let mut args = vec![
            "log",
            "--format=%x1e%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s",
            "--patch",
            "--unified=0",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
        ];
        args.extend(pickaxe);
        args.extend(["HEAD", "--", pathspec]);
        let text = self.run_text(&args)?;
        let mut commits = Vec::new();
        for chunk in text
            .split(RECORD_SEPARATOR)
            .filter(|chunk| !chunk.is_empty())
        {
            let (header, patch) = chunk.split_once('\n').unwrap_or((chunk, ""));
            let commit = parse_commit(header)?;
            let mut changes = CommitUnitChanges::new();
            for line in patch.lines() {
                let (removed, record) = if let Some(record) = line.strip_prefix('-') {
                    (true, record)
                } else if let Some(record) = line.strip_prefix('+') {
                    (false, record)
                } else {
                    continue;
                };
                if !record.starts_with('{') {
                    continue;
                }
                let Some(id) = record_id(record) else {
                    continue;
                };
                let version = match decode_unit_record(record, Path::new(&unit_shard_path(id))) {
                    Ok(unit) => RecordVersion::Valid(unit),
                    Err(error) => RecordVersion::Invalid {
                        message: error.to_string(),
                    },
                };
                let entry = changes
                    .entry(id)
                    .or_insert((RecordVersion::Absent, RecordVersion::Absent));
                if removed {
                    entry.0 = version;
                } else {
                    entry.1 = version;
                }
            }
            changes.retain(|_, (before, after)| before != after);
            if !changes.is_empty() {
                commits.push((commit, changes));
            }
        }
        Ok(commits)
    }

    fn read_working_file(&self, path: &str) -> Result<Option<Vec<u8>>, GitError> {
        let full = self.root().join(path);
        match fs::read(&full) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(source) if source.kind() == ErrorKind::NotFound => Ok(None),
            Err(source) => Err(GitError::Io {
                operation: "read unit shard",
                path: full,
                source,
            }),
        }
    }

    /// Reads `<revision>:<path>` blobs in one `git cat-file --batch` process.
    /// Missing objects are returned as `None`.
    pub(crate) fn read_blobs(&self, specs: &[String]) -> Result<Vec<Option<Vec<u8>>>, GitError> {
        if specs.is_empty() {
            return Ok(Vec::new());
        }
        let mut input = Vec::new();
        for spec in specs {
            input.extend_from_slice(spec.as_bytes());
            input.push(b'\n');
        }
        let args = ["cat-file", "--batch"];
        let output = self.git().output_with_input(self.root(), &args, input)?;
        crate::process::require_success(&args, &output)?;

        let mut blobs = Vec::with_capacity(specs.len());
        let mut rest = output.stdout.as_slice();
        for spec in specs {
            let header_end = rest
                .iter()
                .position(|byte| *byte == b'\n')
                .ok_or_else(|| batch_error(spec))?;
            let header = std::str::from_utf8(&rest[..header_end]).map_err(|_| batch_error(spec))?;
            rest = &rest[header_end + 1..];
            if header.ends_with(" missing") || header.ends_with(" ambiguous") {
                blobs.push(None);
                continue;
            }
            let mut fields = header.split(' ');
            let kind = fields.nth(1);
            let size: usize = fields
                .next()
                .and_then(|size| size.parse().ok())
                .ok_or_else(|| batch_error(spec))?;
            if rest.len() < size + 1 {
                return Err(batch_error(spec));
            }
            let content = rest[..size].to_vec();
            rest = &rest[size + 1..];
            blobs.push((kind == Some("blob")).then_some(content));
        }
        Ok(blobs)
    }
}

/// Counts units by the authors who translated and reviewed their current
/// text.
#[must_use]
pub fn summarize_contributors(attribution: &[UnitAttribution]) -> Vec<ContributorSummary> {
    let mut summaries: BTreeMap<(String, String), ContributorSummary> = BTreeMap::new();
    let mut credit_to = |attribution: &Attribution, reviewed: bool| {
        let summary = summaries
            .entry((
                attribution.author_name.clone(),
                attribution.author_email.clone(),
            ))
            .or_insert_with(|| ContributorSummary {
                name: attribution.author_name.clone(),
                email: attribution.author_email.clone(),
                translated: 0,
                reviewed: 0,
                last_authored_at: attribution.authored_at,
            });
        if reviewed {
            summary.reviewed += 1;
        } else {
            summary.translated += 1;
        }
        summary.last_authored_at = summary.last_authored_at.max(attribution.authored_at);
    };
    for entry in attribution {
        if let Some(translator) = &entry.translated_by {
            credit_to(translator, false);
        }
        if let Some(reviewer) = &entry.reviewed_by {
            credit_to(reviewer, true);
        }
    }
    let mut summaries: Vec<_> = summaries.into_values().collect();
    summaries.sort_by(|left, right| {
        (right.translated + right.reviewed)
            .cmp(&(left.translated + left.reviewed))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.email.cmp(&right.email))
    });
    summaries
}

/// Builds a deterministic checkpoint message from translation-unit changes.
#[must_use]
pub fn summarize_changes(changes: &[UnitChange]) -> String {
    let mut added = 0;
    let mut updated = 0;
    let mut reviewed = 0;
    let mut notes = 0;
    let mut removed = 0;
    let mut sheets = BTreeSet::new();
    for change in changes {
        if let Some(unit) = change.latest() {
            sheets.insert(unit.source_binding().sheet_name());
        }
        match change.kind {
            UnitChangeKind::Added => added += 1,
            UnitChangeKind::Removed => removed += 1,
            UnitChangeKind::Modified if change.target_changed() => updated += 1,
            UnitChangeKind::Modified if change.review_changed() => reviewed += 1,
            UnitChangeKind::Modified => notes += 1,
        }
    }
    let plural = |count: usize, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!("translate {}", plural(added, "string", "strings")));
    }
    if updated > 0 {
        parts.push(format!(
            "update {}",
            plural(updated, "translation", "translations")
        ));
    }
    if reviewed > 0 {
        parts.push(format!(
            "change review state of {}",
            plural(reviewed, "string", "strings")
        ));
    }
    if notes > 0 {
        parts.push(format!("edit {}", plural(notes, "note", "notes")));
    }
    if removed > 0 {
        parts.push(format!(
            "remove {}",
            plural(removed, "translation", "translations")
        ));
    }
    if parts.is_empty() {
        return "Update translations".to_owned();
    }
    let mut message = parts.join(", ");
    if let Some(first) = message.get(..1) {
        message = format!("{}{}", first.to_uppercase(), &message[1..]);
    }
    let shown: Vec<&str> = sheets.iter().take(SHOWN_SHEETS).copied().collect();
    if !shown.is_empty() {
        let more = sheets.len().saturating_sub(SHOWN_SHEETS);
        let suffix = if more > 0 {
            format!(" +{more} more")
        } else {
            String::new()
        };
        let _ = write!(message, " ({}{suffix})", shown.join(", "));
    }
    message
}

pub(crate) fn is_shard_path(path: &str) -> bool {
    path.strip_prefix(".aeria/units/").is_some_and(|name| {
        let bytes = name.as_bytes();
        bytes.len() == 8
            && &bytes[2..] == b".jsonl"
            && bytes[..2]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    })
}

fn batch_error(spec: &str) -> GitError {
    GitError::Parse {
        message: format!("unexpected git cat-file output for {spec}"),
    }
}

fn diff_shards(
    path: &str,
    before: Option<&[u8]>,
    after: Option<&[u8]>,
) -> Result<Vec<UnitChange>, GitError> {
    let decode =
        |bytes: Option<&[u8]>| -> Result<BTreeMap<TranslationUnitId, TranslationUnit>, GitError> {
            Ok(match bytes {
                Some(bytes) => decode_unit_shard(bytes, Path::new(path))?
                    .into_iter()
                    .map(|unit| (unit.id(), unit))
                    .collect(),
                None => BTreeMap::new(),
            })
        };
    let mut before = decode(before)?;
    let after = decode(after)?;
    let mut changes = Vec::new();
    for (id, unit) in after {
        changes.extend(UnitChange::new(before.remove(&id), Some(unit)));
    }
    changes.extend(
        before
            .into_values()
            .filter_map(|unit| UnitChange::new(Some(unit), None)),
    );
    changes.sort_by_key(|change| change.id);
    Ok(changes)
}

fn change_kind(before: &RecordVersion, after: &RecordVersion) -> UnitChangeKind {
    match (before, after) {
        (RecordVersion::Absent, _) => UnitChangeKind::Added,
        (_, RecordVersion::Absent) => UnitChangeKind::Removed,
        _ => UnitChangeKind::Modified,
    }
}

/// Extracts the `TranslationUnitId` of a record line without decoding it,
/// so invalid historical records can still be attributed to their unit.
fn record_id(record: &str) -> Option<TranslationUnitId> {
    const KEY: &str = "\"id\":\"";
    let start = record.find(KEY)? + KEY.len();
    let end = start + record[start..].find('"')?;
    TranslationUnitId::from_str(&record[start..end]).ok()
}

/// Finds who introduced `current`'s target text and, when it is reviewed,
/// who reviewed that text. `revisions` are newest first.
fn credit(
    current: Option<&TranslationUnit>,
    revisions: &[UnitRevision],
) -> (Option<Attribution>, Option<Attribution>) {
    let Some(current) = current else {
        return (None, None);
    };
    let valid = |version: &RecordVersion| match version {
        RecordVersion::Valid(unit) => Some(unit.clone()),
        _ => None,
    };
    let has_current_text = |unit: &Option<TranslationUnit>| {
        unit.as_ref()
            .is_some_and(|unit| unit.target_macro() == current.target_macro())
    };
    let translated_by = revisions
        .iter()
        .find(|revision| {
            has_current_text(&valid(&revision.after)) && !has_current_text(&valid(&revision.before))
        })
        .map(|revision| (&revision.commit).into());
    let reviewed = |unit: &Option<TranslationUnit>| {
        has_current_text(unit)
            && unit
                .as_ref()
                .is_some_and(|unit| unit.review_state() == ReviewState::Reviewed)
    };
    let reviewed_by = (current.review_state() == ReviewState::Reviewed)
        .then(|| {
            revisions
                .iter()
                .find(|revision| {
                    reviewed(&valid(&revision.after)) && !reviewed(&valid(&revision.before))
                })
                .map(|revision| (&revision.commit).into())
        })
        .flatten();
    (translated_by, reviewed_by)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_paths_are_recognized_exactly() {
        assert!(is_shard_path(".aeria/units/7a.jsonl"));
        assert!(!is_shard_path(".aeria/manifest.json"));
        assert!(!is_shard_path(".aeria/units/7a.json"));
        assert!(!is_shard_path("other/.aeria/units/7a.jsonl"));
    }
}
