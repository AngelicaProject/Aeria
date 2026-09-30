//! Git over the project's PO files, per entry: what changed in the working
//! tree or in a commit, the history of one string, and a merge that joins
//! two branches' translations of one file entry by entry.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::sync::Arc;

use aeria_po::{Entry, PoFile};

use crate::GitError;
use crate::repository::{CommitSummary, GitRepository, validate_revision};

/// The folder of the project's PO files.
pub const PO_DIR: &str = "po";

/// Whether a project-relative path is a PO file of the project.
#[must_use]
pub fn is_po_path(path: &str) -> bool {
    path.starts_with("po/")
        && std::path::Path::new(path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("po"))
}

/// What a person can change about a string: its translation, whether it is
/// fuzzy, and its note.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EntryState {
    pub translation: String,
    pub fuzzy: bool,
    pub note: Option<String>,
}

impl EntryState {
    fn of(entry: &Entry) -> Self {
        Self {
            translation: entry.translation.clone(),
            fuzzy: entry.fuzzy,
            note: (!entry.notes.is_empty()).then(|| entry.notes.join("\n")),
        }
    }
}

/// How a string changed.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EntryChangeKind {
    /// It was not translated and now is.
    Translated,
    /// Its translation changed.
    Changed,
    /// Its translation was removed.
    Cleared,
    /// Only its note or its fuzzy mark changed.
    Marked,
}

/// A change of one string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryChange {
    /// The file, relative to the project root: `po/...`.
    pub path: String,
    /// The entry's `msgctxt`.
    pub context: String,
    /// The entry's `msgid` after the change.
    pub source: String,
    pub kind: EntryChangeKind,
    pub before: EntryState,
    pub after: EntryState,
}

/// The size and modification time of a file; `None` when it is missing.
type FileStamp = (u64, Option<std::time::SystemTime>);

fn file_stamp(path: &std::path::Path) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()))
}

/// A changed PO file, relative to the project root, with its string changes.
pub type PendingFile = (String, Arc<Vec<EntryChange>>);

/// The string changes of files read by [`GitRepository::pending_files`],
/// kept while the file on disk, the repository, and `HEAD` stay the same.
#[derive(Debug, Default)]
pub struct PendingCache {
    root: Option<std::path::PathBuf>,
    head: Option<String>,
    files: BTreeMap<String, (Option<FileStamp>, Arc<Vec<EntryChange>>)>,
}

impl PendingCache {
    /// An empty cache.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            root: None,
            head: None,
            files: BTreeMap::new(),
        }
    }
}

fn parse(bytes: Option<&[u8]>) -> PoFile {
    bytes
        .map(|bytes| PoFile::parse(&String::from_utf8_lossy(bytes)).0)
        .unwrap_or_default()
}

/// The strings whose translation, fuzzy mark, or note differ between two
/// versions of a file, in the order of the later version.
#[must_use]
pub fn diff_file(path: &str, before: Option<&[u8]>, after: Option<&[u8]>) -> Vec<EntryChange> {
    let before = parse(before);
    let after = parse(after);
    let earlier: HashMap<&str, &Entry> = before
        .entries
        .iter()
        .map(|entry| (entry.context.as_str(), entry))
        .collect();
    let mut changes = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push = |context: &str, source: &str, old: EntryState, new: EntryState| {
        if old == new {
            return;
        }
        let kind = match (old.translation.is_empty(), new.translation.is_empty()) {
            (true, false) => EntryChangeKind::Translated,
            (false, true) => EntryChangeKind::Cleared,
            (false, false) if old.translation != new.translation => EntryChangeKind::Changed,
            _ => EntryChangeKind::Marked,
        };
        changes.push(EntryChange {
            path: path.to_owned(),
            context: context.to_owned(),
            source: source.to_owned(),
            kind,
            before: old,
            after: new,
        });
    };
    for entry in &after.entries {
        seen.insert(entry.context.as_str());
        let old = earlier
            .get(entry.context.as_str())
            .map(|entry| EntryState::of(entry))
            .unwrap_or_default();
        push(&entry.context, &entry.source, old, EntryState::of(entry));
    }
    for entry in &before.entries {
        if !seen.contains(entry.context.as_str()) {
            push(
                &entry.context,
                &entry.source,
                EntryState::of(entry),
                EntryState::default(),
            );
        }
    }
    changes
}

/// A revision of a string in history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryRevision {
    pub commit: CommitSummary,
    pub kind: EntryChangeKind,
    pub before: EntryState,
    pub after: EntryState,
}

/// The history of one string: its uncommitted change and its revisions,
/// newest first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryHistory {
    pub pending: Option<EntryChange>,
    pub revisions: Vec<EntryRevision>,
    /// More commits changed the file than were read.
    pub truncated: bool,
}

/// Builds a commit message from string changes: what was done and in which
/// sheets.
#[must_use]
pub fn summarize_changes(changes: &[EntryChange]) -> String {
    const SHOWN_SHEETS: usize = 3;
    let mut counts = BTreeMap::new();
    let mut sheets = BTreeSet::new();
    for change in changes {
        *counts.entry(change.kind).or_insert(0_usize) += 1;
        if let Some(sheet) = change.context.split(':').next() {
            sheets.insert(sheet.to_owned());
        }
    }
    let plural = |count: usize, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    let parts: Vec<String> = counts
        .into_iter()
        .map(|(kind, count)| match kind {
            EntryChangeKind::Translated => {
                format!("translate {}", plural(count, "string", "strings"))
            }
            EntryChangeKind::Changed => {
                format!("update {}", plural(count, "translation", "translations"))
            }
            EntryChangeKind::Cleared => {
                format!("remove {}", plural(count, "translation", "translations"))
            }
            EntryChangeKind::Marked => format!("mark {}", plural(count, "string", "strings")),
        })
        .collect();
    if parts.is_empty() {
        return "Update translations".to_owned();
    }
    let mut message = parts.join(", ");
    if let Some(first) = message.get(..1) {
        message = format!("{}{}", first.to_uppercase(), &message[1..]);
    }
    let shown: Vec<&str> = sheets
        .iter()
        .take(SHOWN_SHEETS)
        .map(String::as_str)
        .collect();
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

impl GitRepository {
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
        let batch_error = |spec: &str| GitError::Parse {
            message: format!("unexpected cat-file --batch output for {spec}"),
        };
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

    fn read_working(&self, path: &str) -> Result<Option<Vec<u8>>, GitError> {
        let full = self.root().join(path);
        match std::fs::read(&full) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(GitError::Io {
                operation: "read project file",
                path: full,
                source,
            }),
        }
    }

    /// The uncommitted string changes of the working tree against `HEAD`.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails or a file cannot be read.
    pub fn pending_changes(&self) -> Result<Vec<EntryChange>, GitError> {
        let mut cache = PendingCache::default();
        Ok(self
            .pending_files(&mut cache)?
            .iter()
            .flat_map(|(_, changes)| changes.iter().cloned())
            .collect())
    }

    /// The uncommitted string changes of the working tree against `HEAD`,
    /// per changed PO file in path order. Files that did not change on disk
    /// since `cache` read them, with the same `HEAD`, are not read again, so
    /// a project with many uncommitted files can be asked often.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails or a file cannot be read.
    pub fn pending_files(&self, cache: &mut PendingCache) -> Result<Vec<PendingFile>, GitError> {
        let status = self.status()?;
        let root = self.root().to_owned();
        if cache.root.as_ref() != Some(&root) || cache.head != status.head {
            *cache = PendingCache {
                root: Some(root),
                head: status.head.clone(),
                files: BTreeMap::new(),
            };
        }
        let paths: BTreeSet<&str> = status
            .files
            .iter()
            .flat_map(|file| std::iter::once(&file.path).chain(file.original_path.as_ref()))
            .map(String::as_str)
            .filter(|path| is_po_path(path))
            .collect();
        let stamps: Vec<Option<FileStamp>> = paths
            .iter()
            .map(|path| file_stamp(&self.root().join(path)))
            .collect();
        let stale: Vec<&str> = paths
            .iter()
            .zip(&stamps)
            .filter(|(path, stamp)| {
                cache
                    .files
                    .get(**path)
                    .is_none_or(|(seen, _)| seen != *stamp)
            })
            .map(|(path, _)| *path)
            .collect();
        let committed = if status.head.is_some() {
            let specs: Vec<String> = stale
                .iter()
                .map(|path| format!("HEAD:{}", self.top_level_path(path)))
                .collect();
            self.read_blobs(&specs)?
        } else {
            vec![None; stale.len()]
        };
        for (path, before) in stale.into_iter().zip(committed) {
            let stamp = file_stamp(&self.root().join(path));
            let after = self.read_working(path)?;
            let changes = diff_file(path, before.as_deref(), after.as_deref());
            cache
                .files
                .insert(path.to_owned(), (stamp, Arc::new(changes)));
        }
        cache.files.retain(|path, _| paths.contains(path.as_str()));
        Ok(paths
            .into_iter()
            .filter_map(|path| {
                let (_, changes) = cache.files.get(path)?;
                (!changes.is_empty()).then(|| (path.to_owned(), Arc::clone(changes)))
            })
            .collect())
    }

    /// Project files (see [`crate::PROJECT_PATHS`]) a commit changed
    /// against its first parent, as project-relative paths, with the first
    /// parent (`None` for a root commit).
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid revision, or an
    /// error when Git fails.
    pub fn commit_project_paths(
        &self,
        revision: &str,
    ) -> Result<(Option<String>, Vec<String>), GitError> {
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
        let parent = commit.parents.first().cloned();
        match &parent {
            Some(parent) => args.push(parent),
            None => args.push("--root"),
        }
        args.push(commit.id.as_str());
        args.push("--");
        args.extend(crate::PROJECT_PATHS);
        let text = self.run_text(&args)?;
        Ok((
            parent,
            text.split(' ')
                .filter(|path| !path.is_empty() && *path != commit.id)
                .map(str::to_owned)
                .collect(),
        ))
    }

    /// The string changes a commit made against its first parent.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid revision or when Git fails.
    pub fn commit_changes(
        &self,
        revision: &str,
    ) -> Result<(CommitSummary, Vec<EntryChange>), GitError> {
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
        args.extend([commit.id.as_str(), "--", PO_DIR]);
        let text = self.run_text(&args)?;
        let paths: Vec<&str> = text.split('\0').filter(|path| is_po_path(path)).collect();
        let parent = commit.parents.first();
        let mut specs = Vec::with_capacity(paths.len() * 2);
        for path in &paths {
            let top = self.top_level_path(path);
            if let Some(parent) = parent {
                specs.push(format!("{parent}:{top}"));
            }
            specs.push(format!("{}:{top}", commit.id));
        }
        let mut blobs = self.read_blobs(&specs)?.into_iter();
        let mut changes = Vec::new();
        for path in &paths {
            let before = if parent.is_some() {
                blobs.next().flatten()
            } else {
                None
            };
            let after = blobs.next().flatten();
            changes.extend(diff_file(path, before.as_deref(), after.as_deref()));
        }
        Ok((commit, changes))
    }

    /// The history of the string `context` in the file `path`: its
    /// uncommitted change, and the commits that changed it among the last
    /// `scan` commits of the file, at most `limit` of them, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails or a file cannot be read.
    pub fn entry_history(
        &self,
        path: &str,
        context: &str,
        limit: usize,
        scan: usize,
    ) -> Result<EntryHistory, GitError> {
        let has_head = self.head()?.is_some();
        let top = self.top_level_path(path);
        let committed = if has_head {
            self.read_blobs(&[format!("HEAD:{top}")])?.pop().flatten()
        } else {
            None
        };
        let working = self.read_working(path)?;
        let pending = diff_file(path, committed.as_deref(), working.as_deref())
            .into_iter()
            .find(|change| change.context == context);
        let mut revisions = Vec::new();
        let mut truncated = false;
        if has_head {
            let count = format!("--max-count={}", scan + 1);
            let text = self.run_text(&[
                "log",
                "--format=%H",
                "--no-merges",
                &count,
                "HEAD",
                "--",
                path,
            ])?;
            let mut commits: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
            if commits.len() > scan {
                commits.truncate(scan);
                truncated = true;
            }
            let mut specs = Vec::with_capacity(commits.len() * 2);
            for commit in &commits {
                specs.push(format!("{commit}^:{top}"));
                specs.push(format!("{commit}:{top}"));
            }
            let blobs = self.read_blobs(&specs)?;
            for (index, commit) in commits.iter().enumerate() {
                let found = diff_file(
                    path,
                    blobs[index * 2].as_deref(),
                    blobs[index * 2 + 1].as_deref(),
                )
                .into_iter()
                .find(|change| change.context == context);
                if let Some(change) = found {
                    revisions.push(EntryRevision {
                        commit: self.commit(commit)?,
                        kind: change.kind,
                        before: change.before,
                        after: change.after,
                    });
                    if revisions.len() == limit {
                        truncated = index + 1 < commits.len() || truncated;
                        break;
                    }
                }
            }
        }
        Ok(EntryHistory {
            pending,
            revisions,
            truncated,
        })
    }
}

/// Which side of a conflict to keep.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictResolution {
    Ours,
    Theirs,
}

/// A string both branches changed differently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryConflict {
    pub path: String,
    pub context: String,
    pub source: String,
    pub base: EntryState,
    pub ours: EntryState,
    pub theirs: EntryState,
}

/// The same strings in the same order, with the same source texts and
/// comments: only what a person changes may differ between the branches.
fn same_strings(ours: &PoFile, theirs: &PoFile) -> bool {
    let key = |entry: &Entry| {
        (
            entry.context.clone(),
            entry.source.clone(),
            entry.extracted.clone(),
        )
    };
    ours.header == theirs.header
        && ours.obsolete == theirs.obsolete
        && ours.entries.len() == theirs.entries.len()
        && ours
            .entries
            .iter()
            .zip(&theirs.entries)
            .all(|(ours, theirs)| key(ours) == key(theirs))
}

/// Joins two versions of a file entry by entry: a string one branch changed
/// takes that branch's change, and a string both changed differently takes
/// the side `resolutions` names or is a conflict. `None` when the versions
/// differ in more than translations, fuzzy marks, and notes (for example
/// one branch updated the game), which the caller merges as text.
#[must_use]
pub fn merge_file(
    path: &str,
    base: Option<&[u8]>,
    ours: &[u8],
    theirs: &[u8],
    resolutions: &BTreeMap<String, ConflictResolution>,
) -> Option<(String, Vec<EntryConflict>)> {
    let (mut merged, ours_problems) = PoFile::parse(&String::from_utf8_lossy(ours));
    let (theirs, theirs_problems) = PoFile::parse(&String::from_utf8_lossy(theirs));
    if !ours_problems.is_empty() || !theirs_problems.is_empty() || !same_strings(&merged, &theirs) {
        return None;
    }
    let base = parse(base);
    let base: HashMap<&str, &Entry> = base
        .entries
        .iter()
        .map(|entry| (entry.context.as_str(), entry))
        .collect();
    let mut conflicts = Vec::new();
    for (entry, other) in merged.entries.iter_mut().zip(&theirs.entries) {
        let ours_state = EntryState::of(entry);
        let theirs_state = EntryState::of(other);
        if ours_state == theirs_state {
            continue;
        }
        let base_state = base
            .get(entry.context.as_str())
            .map(|entry| EntryState::of(entry))
            .unwrap_or_default();
        let take_theirs = if base_state == ours_state {
            true
        } else if base_state == theirs_state {
            false
        } else {
            match resolutions.get(&entry.context) {
                Some(ConflictResolution::Ours) => false,
                Some(ConflictResolution::Theirs) => true,
                None => {
                    conflicts.push(EntryConflict {
                        path: path.to_owned(),
                        context: entry.context.clone(),
                        source: entry.source.clone(),
                        base: base_state,
                        ours: ours_state,
                        theirs: theirs_state,
                    });
                    false
                }
            }
        };
        if take_theirs {
            entry.translation.clone_from(&other.translation);
            entry.fuzzy = other.fuzzy;
            entry.previous.clone_from(&other.previous);
            entry.notes.clone_from(&other.notes);
        }
    }
    Some((merged.write(), conflicts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(entries: &[(&str, &str)]) -> String {
        let mut text = String::new();
        for (context, translation) in entries {
            let _ = write!(
                text,
                "msgctxt \"{context}\"\nmsgid \"x\"\nmsgstr \"{translation}\"\n\n"
            );
        }
        text
    }

    #[test]
    fn changes_are_found_per_string() {
        let before = file(&[("A:1:0:0", ""), ("A:2:0:0", "два"), ("A:3:0:0", "три")]);
        let after = file(&[("A:1:0:0", "один"), ("A:2:0:0", "2"), ("A:3:0:0", "")]);
        let changes = diff_file("po/A.po", Some(before.as_bytes()), Some(after.as_bytes()));
        let kinds: Vec<EntryChangeKind> = changes.iter().map(|change| change.kind).collect();
        assert_eq!(
            kinds,
            [
                EntryChangeKind::Translated,
                EntryChangeKind::Changed,
                EntryChangeKind::Cleared
            ]
        );
        assert_eq!(
            summarize_changes(&changes),
            "Translate 1 string, update 1 translation, remove 1 translation (A)"
        );
    }

    #[test]
    fn branches_join_per_string_and_report_real_conflicts() {
        let base = file(&[("A:1:0:0", ""), ("A:2:0:0", ""), ("A:3:0:0", "")]);
        let ours = file(&[("A:1:0:0", "наш"), ("A:2:0:0", ""), ("A:3:0:0", "мы")]);
        let theirs = file(&[("A:1:0:0", ""), ("A:2:0:0", "их"), ("A:3:0:0", "они")]);
        let (merged, conflicts) = merge_file(
            "po/A.po",
            Some(base.as_bytes()),
            ours.as_bytes(),
            theirs.as_bytes(),
            &BTreeMap::new(),
        )
        .expect("mergeable");
        assert!(merged.contains("msgstr \"наш\"") && merged.contains("msgstr \"их\""));
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].context, "A:3:0:0");
        let resolutions = BTreeMap::from([("A:3:0:0".to_owned(), ConflictResolution::Theirs)]);
        let (merged, conflicts) = merge_file(
            "po/A.po",
            Some(base.as_bytes()),
            ours.as_bytes(),
            theirs.as_bytes(),
            &resolutions,
        )
        .expect("mergeable");
        assert!(conflicts.is_empty());
        assert!(merged.contains("msgstr \"они\""));
        // A branch that changed more than translations is merged as text.
        let other = file(&[("A:1:0:0", ""), ("A:2:0:0", "")]);
        assert!(
            merge_file(
                "po/A.po",
                None,
                ours.as_bytes(),
                other.as_bytes(),
                &BTreeMap::new()
            )
            .is_none()
        );
    }
}
