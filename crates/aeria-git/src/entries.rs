//! Git over the project's PO files, per entry: what changed in the working
//! tree or in a commit, the history of one string, and a merge that joins
//! two branches' translations of one file entry by entry.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::sync::Arc;

use aeria_po::{Entry, PoFile};

use crate::GitError;
use crate::repository::{
    CommitSummary, FileChangeKind, FileStatus, GitRepository, validate_revision,
};

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
/// fuzzy, its note, whether they reviewed it, and the terms that do not
/// apply to it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EntryState {
    pub translation: String,
    pub fuzzy: bool,
    pub note: Option<String>,
    /// A person reviewed the translation as it is (see
    /// [`Entry::is_reviewed`]); a mark of another text counts as none.
    pub reviewed: bool,
    /// The terms of the string's term exceptions, a person's decisions.
    pub term_exceptions: Vec<String>,
}

impl EntryState {
    fn of(entry: &Entry) -> Self {
        Self {
            term_exceptions: entry.term_exceptions.clone(),
            ..Self::joined(entry)
        }
    }

    /// The state the join of two branches compares: without the
    /// exceptions, which join apart (see [`merge_exceptions`]).
    fn joined(entry: &Entry) -> Self {
        Self {
            translation: entry.translation.clone(),
            fuzzy: entry.fuzzy,
            note: (!entry.notes.is_empty()).then(|| entry.notes.join("\n")),
            reviewed: entry.is_reviewed(),
            term_exceptions: Vec::new(),
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
    /// Only its note, its fuzzy mark, its review, or its term exceptions
    /// changed.
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

/// Files of one call of Git and one thread's turn in
/// [`GitRepository::pending_files`].
const PENDING_PACK: usize = 32;

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

/// The strings whose translation, fuzzy mark, note, review, or exceptions
/// differ between two
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

    pub(crate) fn read_working(&self, path: &str) -> Result<Option<Vec<u8>>, GitError> {
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
        // A project can have thousands of changed files after a bulk edit or
        // a machine translation run: they are read and compared in packs on
        // several threads, each pack's committed versions in one call of Git.
        let packs: Vec<&[&str]> = stale.chunks(PENDING_PACK).collect();
        let committed = status.head.is_some();
        let compare = |pack: &[&str]| -> Result<Vec<_>, GitError> {
            let before = if committed {
                let specs: Vec<String> = pack
                    .iter()
                    .map(|path| format!("HEAD:{}", self.top_level_path(path)))
                    .collect();
                self.read_blobs(&specs)?
            } else {
                vec![None; pack.len()]
            };
            pack.iter()
                .zip(before)
                .map(|(path, before)| {
                    let stamp = file_stamp(&self.root().join(path));
                    let after = self.read_working(path)?;
                    let changes = diff_file(path, before.as_deref(), after.as_deref());
                    Ok(((*path).to_owned(), stamp, changes))
                })
                .collect()
        };
        let next = std::sync::atomic::AtomicUsize::new(0);
        let compared = std::sync::Mutex::new(Vec::with_capacity(packs.len()));
        let workers = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        std::thread::scope(|scope| {
            for _ in 0..workers.min(packs.len()) {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(pack) = packs.get(index) else {
                            return;
                        };
                        let done = compare(pack);
                        compared
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(done);
                    }
                });
            }
        });
        for pack in compared
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            for (path, stamp, changes) in pack? {
                cache.files.insert(path, (stamp, Arc::new(changes)));
            }
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

    /// Every file a commit changed against its first parent (against the
    /// empty tree for a root commit), as project-relative paths in path
    /// order, with how each changed.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid revision, or an
    /// error when Git fails or prints a status it does not document.
    pub fn commit_files(&self, revision: &str) -> Result<Vec<FileStatus>, GitError> {
        validate_revision(revision)?;
        let commit = self.commit(revision)?;
        let mut args = vec![
            "diff-tree",
            "-r",
            "-z",
            "--name-status",
            "--no-renames",
            "--relative",
        ];
        match commit.parents.first() {
            Some(parent) => args.push(parent),
            None => args.push("--root"),
        }
        args.push(commit.id.as_str());
        let text = self.run_text(&args)?;
        // A root commit's output starts with the commit ID.
        let mut fields = text
            .split('\0')
            .filter(|field| !field.is_empty() && *field != commit.id);
        let mut files = Vec::new();
        while let Some(status) = fields.next() {
            let kind = match status {
                "A" => FileChangeKind::Added,
                "M" => FileChangeKind::Modified,
                "D" => FileChangeKind::Deleted,
                "T" => FileChangeKind::TypeChanged,
                other => {
                    return Err(GitError::Parse {
                        message: format!("git diff-tree printed an unknown status {other:?}"),
                    });
                }
            };
            let Some(path) = fields.next() else {
                return Err(GitError::Parse {
                    message: "git diff-tree printed a status without a path".to_owned(),
                });
            };
            files.push(FileStatus {
                path: path.to_owned(),
                original_path: None,
                kind,
                staged: false,
                index: None,
                worktree: None,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
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

/// Takes the other branch's term exceptions of a string when only that
/// branch changed them; when both did, ours stay. They are decisions about
/// the source, apart from the translation, so a branch that only decided
/// terms and one that only translated both keep their work.
fn merge_exceptions(entry: &mut Entry, other: &Entry, base: Option<&Entry>) {
    let base = base
        .map(|base| base.term_exceptions.clone())
        .unwrap_or_default();
    if entry.term_exceptions == base {
        entry.term_exceptions.clone_from(&other.term_exceptions);
    }
}

/// Which version of a file a join takes its strings and their source texts
/// from.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Side {
    Ours,
    Theirs,
}

/// An entry of the version that kept the base's strings, as a game update
/// carries it to `game`, the same string in the other version (see
/// [`aeria_po::merge::carry`]): a translation whose source changed becomes
/// fuzzy.
fn carried(entry: &Entry, game: &Entry) -> Entry {
    let mut fresh = Entry {
        extracted: game.extracted.clone(),
        context: game.context.clone(),
        source: game.source.clone(),
        ..Entry::default()
    };
    aeria_po::merge::carry(entry, &mut fresh);
    fresh
}

/// A string the updated version no longer has: what the other branch
/// changed of it is kept with the obsolete entries, as the update keeps it.
/// `joined` are the strings both versions have.
fn keep_left_strings(
    merged: &mut PoFile,
    other: &PoFile,
    base: &HashMap<&str, &Entry>,
    joined: &BTreeSet<String>,
) {
    let mut kept = false;
    for other in &other.entries {
        let unchanged = base
            .get(other.context.as_str())
            .is_some_and(|base| EntryState::of(base) == EntryState::of(other));
        if unchanged || joined.contains(&other.context) {
            continue;
        }
        let Some(obsolete) = aeria_po::merge::obsolete(other) else {
            continue;
        };
        merged
            .obsolete
            .retain(|entry| entry.context != obsolete.context);
        merged.obsolete.push(obsolete);
        kept = true;
    }
    if kept {
        merged.obsolete.sort_by(|a, b| a.context.cmp(&b.context));
    }
}

/// Joins two versions of a file entry by entry: a string one branch changed
/// takes that branch's change, and a string both changed differently takes
/// the side `resolutions` names or is a conflict.
///
/// The versions hold the same strings with the same source texts, or one of
/// them holds those of `base` and the other was made again for the game: a
/// game update on one branch. The result then has the updated version's
/// strings, and each string the other branch changed is carried to it as
/// the update carries a translation, so one whose source changed is fuzzy.
/// `None` when the versions differ in any other way, which the caller
/// merges as text.
#[must_use]
pub fn merge_file(
    path: &str,
    base: Option<&[u8]>,
    ours: &[u8],
    theirs: &[u8],
    resolutions: &BTreeMap<String, ConflictResolution>,
) -> Option<(String, Vec<EntryConflict>)> {
    let (ours, ours_problems) = PoFile::parse(&String::from_utf8_lossy(ours));
    let (theirs, theirs_problems) = PoFile::parse(&String::from_utf8_lossy(theirs));
    if !ours_problems.is_empty() || !theirs_problems.is_empty() {
        return None;
    }
    let base_file = parse(base);
    let updated = if same_strings(&ours, &theirs) {
        Side::Ours
    } else if base.is_some() && same_strings(&base_file, &ours) {
        Side::Theirs
    } else if base.is_some() && same_strings(&base_file, &theirs) {
        Side::Ours
    } else {
        return None;
    };
    let (mut merged, other) = match updated {
        Side::Ours => (ours, theirs),
        Side::Theirs => (theirs, ours),
    };
    let base: HashMap<&str, &Entry> = base_file
        .entries
        .iter()
        .map(|entry| (entry.context.as_str(), entry))
        .collect();
    let others: HashMap<&str, &Entry> = other
        .entries
        .iter()
        .map(|entry| (entry.context.as_str(), entry))
        .collect();
    let mut conflicts = Vec::new();
    let mut joined = BTreeSet::new();
    for entry in &mut merged.entries {
        let Some(other) = others.get(entry.context.as_str()) else {
            continue;
        };
        joined.insert(entry.context.clone());
        let other = carried(other, entry);
        let base = base
            .get(entry.context.as_str())
            .map(|base| carried(base, entry));
        // Ours stay when both branches decided terms.
        match updated {
            Side::Ours => merge_exceptions(entry, &other, base.as_ref()),
            Side::Theirs => {
                let mut kept = other.clone();
                merge_exceptions(&mut kept, entry, base.as_ref());
                entry.term_exceptions = kept.term_exceptions;
            }
        }
        let state = EntryState::joined(entry);
        let other_state = EntryState::joined(&other);
        if state == other_state {
            // The record of the terms goes with the translation: the same
            // translation keeps the record a branch made anew.
            let base_record = base.as_ref().map(|base| &base.written_with);
            if base_record.is_none_or(|record| *record == entry.written_with) {
                entry.written_with = other.written_with;
            }
            continue;
        }
        let base_state = base.as_ref().map(EntryState::joined).unwrap_or_default();
        let (ours_state, theirs_state) = match updated {
            Side::Ours => (state, other_state),
            Side::Theirs => (other_state, state),
        };
        let take = if base_state == ours_state {
            Side::Theirs
        } else if base_state == theirs_state {
            Side::Ours
        } else {
            match resolutions.get(&entry.context) {
                Some(ConflictResolution::Ours) => Side::Ours,
                Some(ConflictResolution::Theirs) => Side::Theirs,
                None => {
                    conflicts.push(EntryConflict {
                        path: path.to_owned(),
                        context: entry.context.clone(),
                        source: entry.source.clone(),
                        base: base_state,
                        ours: ours_state,
                        theirs: theirs_state,
                    });
                    Side::Ours
                }
            }
        };
        if take != updated {
            entry.translation = other.translation;
            entry.fuzzy = other.fuzzy;
            entry.previous = other.previous;
            entry.notes = other.notes;
            entry.reviewed = other.reviewed;
            entry.written_with = other.written_with;
        }
    }
    keep_left_strings(&mut merged, &other, &base, &joined);
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

    /// A file of strings as (`msgctxt`, `msgid`, `msgstr`), with lines put
    /// before a string's `msgctxt` after `|` in its `msgctxt`.
    fn game_file(version: &str, entries: &[(&str, &str, &str)], obsolete: &str) -> String {
        let mut text = format!("msgid \"\"\nmsgstr \"\"\n\"X-Game-Version: {version}\\n\"\n\n");
        for (context, source, translation) in entries {
            let (context, marks) = context.split_once('|').unwrap_or((context, ""));
            let _ = write!(
                text,
                "{marks}msgctxt \"{context}\"\nmsgid \"{source}\"\nmsgstr \"{translation}\"\n\n"
            );
        }
        text + obsolete
    }

    #[test]
    fn a_branch_joins_with_a_game_update_as_the_update_carries_it() {
        let base = game_file(
            "1",
            &[
                ("A:1:0:0", "one", "один"),
                ("A:2:0:0", "two", ""),
                ("A:3:0:0", "three", "три"),
                ("A:4:0:0", "four", "четыре"),
            ],
            "",
        );
        // The update: the first source changed, the fourth string left the
        // game, and a fifth came.
        let update = game_file(
            "2",
            &[
                ("A:1:0:0|#, fuzzy\n#| msgid \"one\"\n", "first", "один"),
                ("A:2:0:0", "two", ""),
                ("A:3:0:0", "three", "три"),
                ("A:5:0:0", "five", ""),
            ],
            "#~ msgctxt \"A:4:0:0\"\n#~ msgid \"four\"\n#~ msgstr \"четыре\"\n",
        );
        // The branch of the previous version translated the first string
        // again, the second, and the fourth.
        let branch = game_file(
            "1",
            &[
                ("A:1:0:0", "one", "раз"),
                ("A:2:0:0", "two", "два"),
                ("A:3:0:0", "three", "три"),
                ("A:4:0:0", "four", "4"),
            ],
            "",
        );
        let expected = game_file(
            "2",
            &[
                ("A:1:0:0|#, fuzzy\n#| msgid \"one\"\n", "first", "раз"),
                ("A:2:0:0", "two", "два"),
                ("A:3:0:0", "three", "три"),
                ("A:5:0:0", "five", ""),
            ],
            "#~ msgctxt \"A:4:0:0\"\n#~ msgid \"four\"\n#~ msgstr \"4\"\n",
        );
        // Whichever side the update is on, the result is the same.
        for (ours, theirs) in [(&branch, &update), (&update, &branch)] {
            let (merged, conflicts) = merge_file(
                "po/A.po",
                Some(base.as_bytes()),
                ours.as_bytes(),
                theirs.as_bytes(),
                &BTreeMap::new(),
            )
            .expect("mergeable");
            assert!(conflicts.is_empty(), "{conflicts:?}");
            assert_eq!(merged, PoFile::parse(&expected).0.write());
        }

        // A translation the updated branch accepted, changed on the other
        // branch too, is a conflict of the string.
        let accepted = update.replace(
            "#, fuzzy\n#| msgid \"one\"\nmsgctxt \"A:1:0:0\"\nmsgid \"first\"\nmsgstr \"один\"",
            "msgctxt \"A:1:0:0\"\nmsgid \"first\"\nmsgstr \"первый\"",
        );
        assert_ne!(accepted, update);
        let (_, conflicts) = merge_file(
            "po/A.po",
            Some(base.as_bytes()),
            branch.as_bytes(),
            accepted.as_bytes(),
            &BTreeMap::new(),
        )
        .expect("mergeable");
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].source, "first");
        assert!(conflicts[0].ours.fuzzy && conflicts[0].ours.translation == "раз");
        assert!(!conflicts[0].theirs.fuzzy && conflicts[0].theirs.translation == "первый");
        let resolutions = BTreeMap::from([("A:1:0:0".to_owned(), ConflictResolution::Ours)]);
        let (merged, conflicts) = merge_file(
            "po/A.po",
            Some(base.as_bytes()),
            branch.as_bytes(),
            accepted.as_bytes(),
            &resolutions,
        )
        .expect("mergeable");
        assert!(conflicts.is_empty());
        assert!(merged.contains(
            "#, fuzzy\n#| msgid \"one\"\nmsgctxt \"A:1:0:0\"\nmsgid \"first\"\nmsgstr \"раз\""
        ));

        // Two branches that both left the base's strings do not join.
        let other = game_file("3", &[("A:1:0:0", "uno", "")], "");
        assert!(
            merge_file(
                "po/A.po",
                Some(base.as_bytes()),
                update.as_bytes(),
                other.as_bytes(),
                &BTreeMap::new()
            )
            .is_none()
        );
    }

    #[test]
    fn a_review_never_passes_to_a_text_nobody_reviewed() {
        let reviewed = |translation: &str| {
            format!(
                "#, aeria-reviewed: {}\nmsgctxt \"A:1:0:0\"\nmsgid \"x\"\nmsgstr \"{translation}\"\n",
                aeria_po::fingerprint(translation)
            )
        };
        let base = file(&[("A:1:0:0", "первый")]);
        // One branch reviews the first text; the other has the model write
        // a second one. Git joins the two lines without a conflict.
        let ours = reviewed("первый");
        let theirs = file(&[("A:1:0:0", "второй")]);
        let joined = format!(
            "#, aeria-reviewed: {}\nmsgctxt \"A:1:0:0\"\nmsgid \"x\"\nmsgstr \"второй\"\n",
            aeria_po::fingerprint("первый")
        );
        let entry = PoFile::parse(&joined).0.entries[0].clone();
        assert!(!entry.is_reviewed() && entry.review_is_stale());
        // Aeria's own join sees both sides changed the string.
        let (_, conflicts) = merge_file(
            "po/A.po",
            Some(base.as_bytes()),
            ours.as_bytes(),
            theirs.as_bytes(),
            &BTreeMap::new(),
        )
        .expect("mergeable");
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].ours.reviewed && !conflicts[0].theirs.reviewed);
        // A review on one side alone is taken with its text.
        let (merged, conflicts) = merge_file(
            "po/A.po",
            Some(base.as_bytes()),
            base.as_bytes(),
            ours.as_bytes(),
            &BTreeMap::new(),
        )
        .expect("mergeable");
        assert!(conflicts.is_empty());
        assert!(PoFile::parse(&merged).0.entries[0].is_reviewed());
        // A diff reports a review as a mark.
        let changes = diff_file("po/A.po", Some(base.as_bytes()), Some(ours.as_bytes()));
        assert_eq!(changes[0].kind, EntryChangeKind::Marked);
        assert!(changes[0].after.reviewed);
    }

    #[test]
    fn the_terms_a_translation_was_written_with_go_with_it() {
        let base = file(&[("A:1:0:0", "первый"), ("A:2:0:0", "второй")]);
        // One branch brings the first string to its terms without changing
        // it; the other translates the second again with the terms of now.
        let recorded = format!(
            "# aeria-terms: {{\"mount\":\"3f9a1c0b7d2e\"}}\n{}",
            file(&[("A:1:0:0", "первый"), ("A:2:0:0", "второй")])
        );
        let translated = format!(
            "{}# aeria-terms: {{\"aether\":\"1d4e0f7a9c2b\"}}\n{}",
            file(&[("A:1:0:0", "первый")]),
            file(&[("A:2:0:0", "другой")])
        );
        for (ours, theirs) in [(&recorded, &translated), (&translated, &recorded)] {
            let (merged, conflicts) = merge_file(
                "po/A.po",
                Some(base.as_bytes()),
                ours.as_bytes(),
                theirs.as_bytes(),
                &BTreeMap::new(),
            )
            .expect("mergeable");
            assert!(conflicts.is_empty());
            let merged = PoFile::parse(&merged).0;
            assert_eq!(
                merged.entries[0].written_with.keys().collect::<Vec<_>>(),
                ["mount"],
                "{merged:?}"
            );
            assert_eq!(merged.entries[1].translation, "другой");
            assert_eq!(
                merged.entries[1].written_with.keys().collect::<Vec<_>>(),
                ["aether"]
            );
        }
        // A record alone is no change a person made.
        assert!(diff_file("po/A.po", Some(base.as_bytes()), Some(recorded.as_bytes())).is_empty());
    }
}
