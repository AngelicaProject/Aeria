//! Integration tests over synthetic project repositories.
//!
//! Tests require the `git` executable (`AERIA_GIT_PATH` or `PATH`). They
//! isolate Git from user and system configuration and use only local bare
//! remotes.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use aeria_git::{
    ConflictResolution, EntryChangeKind, FONT_SETTINGS_FILE, FONTS_DIR, GitError, GitExecutable,
    GitRepository, IntegrateOutcome, KNOWLEDGE_DIR, PACK_SETTINGS_FILE, PendingCache,
};
use tempfile::TempDir;

const SETTINGS: &str = "{\n  \"format\": \"aeria-po/1\",\n  \"sourceLanguage\": \"en\",\n  \"targetLanguage\": \"fr\",\n  \"gameVersion\": \"2026.09.15.0000.0000\"\n}\n";

fn git_program() -> std::ffi::OsString {
    std::env::var_os("AERIA_GIT_PATH")
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "git".into())
}

fn no_resolutions() -> BTreeMap<String, ConflictResolution> {
    BTreeMap::new()
}

struct Sandbox {
    dir: TempDir,
    git: GitExecutable,
}

impl Sandbox {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let global = dir.path().join("gitconfig");
        fs::write(&global, "").expect("global config");
        let git = GitExecutable::at(git_program())
            .with_env("GIT_CONFIG_GLOBAL", &global)
            .with_env("GIT_CONFIG_NOSYSTEM", "1");
        Self { dir, git }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn raw_git(&self, cwd: &Path, args: &[&str]) -> String {
        let output = Command::new(git_program())
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn project(&self, name: &str, translator: &str) -> GitRepository {
        let root = self.path(name);
        make_project(&root);
        let repository = GitRepository::init(&root, self.git.clone()).expect("init");
        set_translator(&repository, translator);
        repository
    }

    fn bare_remote(&self) -> String {
        let remote = self.path("remote.git");
        fs::create_dir_all(&remote).expect("remote dir");
        self.raw_git(
            &remote,
            &["init", "--quiet", "--bare", "--initial-branch=main"],
        );
        remote.to_string_lossy().into_owned()
    }

    fn clone(&self, url: &str, name: &str, translator: &str) -> GitRepository {
        let repository =
            GitRepository::clone_from(url, self.path(name), self.git.clone()).expect("clone");
        set_translator(&repository, translator);
        repository
    }
}

fn make_project(root: &Path) {
    fs::create_dir_all(root.join("po")).expect("project dir");
    fs::write(root.join("aeria.json"), SETTINGS).expect("settings");
}

fn set_translator(repository: &GitRepository, translator: &str) {
    let email = format!(
        "{}@example.invalid",
        translator.to_lowercase().replace(' ', ".")
    );
    repository
        .set_identity(translator, Some(&email), false)
        .expect("identity");
}

/// A string of the synthetic game, named by its `msgctxt`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Unit(u8, u8);

impl fmt::Display for Unit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Addon:{}:0:{}", self.0, self.1)
    }
}

fn id(first: u8, last: u8) -> Unit {
    Unit(first, last)
}

/// A string, its row (for its source text), its translation, and a mark
/// (`fuzzy` or anything else).
type Record<'a> = (Unit, u32, &'a str, &'a str);

fn record((unit, row, target, mark): Record<'_>) -> String {
    let fuzzy = if mark == "fuzzy" { "#, fuzzy\n" } else { "" };
    format!("{fuzzy}msgctxt \"{unit}\"\nmsgid \"Source {row}\"\nmsgstr \"{target}\"\n")
}

fn shard_path(shard: u8) -> String {
    format!("po/{shard:02x}.po")
}

/// Writes a complete PO file with the given strings, sorted by `msgctxt`.
fn write_shard(root: &Path, shard: u8, records: &[Record<'_>]) {
    fs::create_dir_all(root.join("po")).expect("po dir");
    let mut records = records.to_vec();
    records.sort_by_key(|(unit, ..)| *unit);
    let text: Vec<String> = records.into_iter().map(record).collect();
    fs::write(root.join(shard_path(shard)), text.join("\n")).expect("file");
}

fn shard_text(root: &Path, shard: u8) -> String {
    fs::read_to_string(root.join(shard_path(shard))).expect("file")
}

fn current_branch(repository: &GitRepository) -> String {
    repository
        .branches()
        .expect("branches")
        .into_iter()
        .find(|branch| branch.current)
        .expect("current branch")
        .name
}

#[test]
fn checkpoints_commit_strings_and_their_history_is_read_per_string() {
    let sandbox = Sandbox::new();
    let repository = sandbox.project("project", "Ada");
    let root = repository.root().to_owned();
    let unit = id(0x7a, 1);
    let path = shard_path(0x7a);

    let attributes = fs::read_to_string(root.join(".gitattributes")).expect("attributes");
    assert!(attributes.contains("*.po text eol=lf"));

    write_shard(&root, 0x7a, &[(unit, 1, "Bonjour", "")]);
    let pending = repository.pending_changes().expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, EntryChangeKind::Translated);

    let first = repository.checkpoint(None).expect("first checkpoint");
    assert_eq!(first.commit.author_name, "Ada");
    assert_eq!(first.commit.subject, "Translate 1 string (Addon)");
    assert!(repository.pending_changes().expect("clean").is_empty());
    assert!(matches!(
        repository.checkpoint(None),
        Err(GitError::NothingToCommit)
    ));

    set_translator(&repository, "Grace");
    write_shard(&root, 0x7a, &[(unit, 1, "Salut", "")]);
    let second = repository
        .checkpoint(Some("Reword greeting"))
        .expect("second checkpoint");
    assert_eq!(second.commit.subject, "Reword greeting");
    // One commit each: equal counts go by name.
    assert_eq!(repository.authors().expect("authors"), ["Ada", "Grace"]);

    write_shard(&root, 0x7a, &[(unit, 1, "Coucou", "")]);
    let history = repository
        .entry_history(&path, &unit.to_string(), 10, 100)
        .expect("history");
    let pending = history.pending.expect("pending change");
    assert_eq!(pending.after.translation, "Coucou");
    assert_eq!(history.revisions.len(), 2);
    assert!(!history.truncated);
    let newest = &history.revisions[0];
    assert_eq!(newest.commit.author_name, "Grace");
    assert_eq!(newest.kind, EntryChangeKind::Changed);
    assert_eq!(newest.after.translation, "Salut");
    assert_eq!(history.revisions[1].kind, EntryChangeKind::Translated);

    let limited = repository
        .entry_history(&path, &unit.to_string(), 1, 100)
        .expect("limited history");
    assert_eq!(limited.revisions.len(), 1);
    assert!(limited.truncated);

    let (commit, changes) = repository
        .commit_changes(&second.commit.id)
        .expect("commit changes");
    assert_eq!(commit.id, second.commit.id);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].before.translation, "Bonjour");

    let log = repository.log(0, 10).expect("log");
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].id, second.commit.id);
}

#[test]
fn cached_pending_changes_follow_the_files_and_head() {
    let sandbox = Sandbox::new();
    let repository = sandbox.project("project", "Ada");
    let root = repository.root().to_owned();
    let unit = id(0x7a, 1);
    let mut cache = PendingCache::new();
    let translations = |cache: &mut PendingCache| -> Vec<String> {
        repository
            .pending_files(cache)
            .expect("pending")
            .iter()
            .flat_map(|(_, changes)| {
                changes
                    .iter()
                    .map(|change| change.after.translation.clone())
            })
            .collect()
    };

    write_shard(&root, 0x7a, &[(unit, 1, "Bonjour", "")]);
    assert_eq!(translations(&mut cache), ["Bonjour"]);
    assert_eq!(translations(&mut cache), ["Bonjour"]);
    // A different length, so the file's stamp changes even within the
    // resolution of its modification time.
    write_shard(&root, 0x7a, &[(unit, 1, "Salut", "")]);
    assert_eq!(translations(&mut cache), ["Salut"]);

    repository.checkpoint(None).expect("checkpoint");
    assert!(translations(&mut cache).is_empty());
    write_shard(&root, 0x7a, &[(unit, 1, "Coucou !", "")]);
    let pending = repository.pending_files(&mut cache).expect("pending");
    assert_eq!(pending[0].1[0].before.translation, "Salut");
    assert_eq!(pending[0].1[0].after.translation, "Coucou !");
}

#[test]
fn integration_leaves_the_reconciliation_for_a_checkpoint() {
    let sandbox = Sandbox::new();
    let remote = sandbox.bare_remote();
    let (one, two) = (id(0x10, 1), id(0x20, 1));

    let ada = sandbox.project("ada", "Ada");
    ada.set_remote("origin", &remote).expect("remote");
    write_shard(ada.root(), 0x10, &[(one, 1, "Un", "")]);
    write_shard(ada.root(), 0x20, &[(two, 2, "Deux", "")]);
    ada.checkpoint(None).expect("initial");
    ada.push().expect("push");
    let grace = sandbox.clone(&remote, "grace", "Grace");

    write_shard(ada.root(), 0x10, &[(one, 1, "Une", "")]);
    ada.checkpoint(None).expect("ada edit");
    ada.push().expect("push");
    write_shard(grace.root(), 0x20, &[(two, 2, "Deux !", "")]);
    grace.checkpoint(None).expect("grace edit");

    grace.fetch().expect("fetch");
    let grace_root = grace.root().to_owned();
    let outcome = grace
        .integrate(&no_resolutions(), || {
            // Accepting may change files, such as a translation it marks.
            write_shard(&grace_root, 0x10, &[(one, 1, "Une", "fuzzy")]);
            Ok(())
        })
        .expect("integrate");
    assert_eq!(outcome, IntegrateOutcome::Merged);
    let parents = sandbox.raw_git(grace.root(), &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents.split_whitespace().count(), 3, "HEAD is the merge");
    // The reconciliation is an ordinary uncommitted change.
    assert!(grace.status().expect("status").has_translation_changes());
    assert!(shard_text(grace.root(), 0x10).contains("#, fuzzy"));
    grace
        .checkpoint(None)
        .expect("checkpoint the reconciliation");
    assert!(grace.status().expect("status").files.is_empty());
    assert!(shard_text(grace.root(), 0x20).contains("\"Deux !\""));
    assert!(grace.push().expect("push"));

    // An acceptance that writes nothing adds no commit.
    ada.fetch().expect("fetch");
    ada.integrate(&no_resolutions(), || Ok(()))
        .expect("ada integrates");
    write_shard(ada.root(), 0x20, &[(two, 2, "Deux !", "fuzzy")]);
    ada.checkpoint(None).expect("ada marks");
    ada.push().expect("push");
    let before = grace.head().expect("head");
    grace.fetch().expect("fetch");
    assert_eq!(
        grace.integrate(&no_resolutions(), || Ok(())).expect("ff"),
        IntegrateOutcome::FastForward
    );
    assert_ne!(grace.head().expect("head"), before);
    assert!(grace.status().expect("status").files.is_empty());
}

#[test]
fn the_email_is_optional_and_never_derived_from_the_host() {
    let sandbox = Sandbox::new();
    let root = sandbox.path("anonymous");
    make_project(&root);
    let repository = GitRepository::init(&root, sandbox.git.clone()).expect("init");

    assert!(!repository.identity().expect("identity").is_complete());
    assert!(matches!(
        repository.checkpoint(None),
        Err(GitError::IdentityMissing)
    ));

    repository
        .set_identity("Анна", None, false)
        .expect("name only");
    let identity = repository.identity().expect("identity");
    assert!(identity.is_complete());
    assert_eq!(identity.email, None);

    write_shard(&root, 0x10, &[(id(0x10, 1), 1, "Un", "")]);
    let outcome = repository.checkpoint(None).expect("checkpoint");
    assert_eq!(outcome.commit.author_name, "Анна");
    assert_eq!(outcome.commit.author_email, "");

    assert!(matches!(
        GitRepository::init(&root, sandbox.git.clone()),
        Err(GitError::AlreadyARepository { .. })
    ));
}

#[test]
fn sync_fast_forwards_and_merges_adjacent_units_semantically() {
    let sandbox = Sandbox::new();
    let remote = sandbox.bare_remote();

    let ada = sandbox.project("ada", "Ada");
    let ada_root = ada.root().to_owned();
    ada.set_remote("origin", &remote).expect("remote");
    let (one, two, three) = (id(0x10, 1), id(0x10, 2), id(0x10, 3));
    write_shard(
        &ada_root,
        0x10,
        &[
            (one, 1, "Un", ""),
            (two, 2, "Deux", ""),
            (three, 3, "Trois", ""),
        ],
    );
    ada.checkpoint(None).expect("initial");
    assert!(ada.push().expect("publish branch"));
    assert!(!ada.push().expect("nothing to push"));

    let grace = sandbox.clone(&remote, "grace", "Grace");
    let grace_root = grace.root().to_owned();

    // Fast-forward.
    write_shard(&ada_root, 0x20, &[(id(0x20, 1), 4, "Quatre", "")]);
    ada.checkpoint(None).expect("ada second");
    ada.push().expect("push");
    grace.fetch().expect("fetch");
    assert_eq!(
        grace
            .integrate(&no_resolutions(), || Ok(()))
            .expect("fast-forward"),
        IntegrateOutcome::FastForward
    );
    assert!(grace_root.join("po/20.po").exists());

    // Adjacent units in one shard: a textual conflict for Git, a clean
    // semantic merge for Aeria.
    write_shard(
        &ada_root,
        0x10,
        &[
            (one, 1, "Une", ""),
            (two, 2, "Deux", ""),
            (three, 3, "Trois", ""),
        ],
    );
    ada.checkpoint(None).expect("ada edits one");
    ada.push().expect("push");
    write_shard(
        &grace_root,
        0x10,
        &[
            (one, 1, "Un", ""),
            (two, 2, "Deux !", ""),
            (three, 3, "Trois", ""),
        ],
    );
    grace.checkpoint(None).expect("grace edits two");
    grace.fetch().expect("fetch");
    assert_eq!(
        grace
            .integrate(&no_resolutions(), || Ok(()))
            .expect("semantic merge"),
        IntegrateOutcome::Merged
    );
    let merged = shard_text(&grace_root, 0x10);
    assert!(merged.contains("\"Une\""));
    assert!(merged.contains("\"Deux !\""));
    assert!(grace.status().expect("status").files.is_empty());
    assert!(grace.push().expect("push merge"));

    // Uncommitted translations block integration.
    write_shard(&grace_root, 0x20, &[(id(0x20, 1), 4, "Quatre !", "")]);
    assert!(matches!(
        grace.integrate(&no_resolutions(), || Ok(())),
        Err(GitError::UncommittedTranslations)
    ));
}

#[test]
fn same_unit_conflicts_are_reported_and_resolved_only_explicitly() {
    let sandbox = Sandbox::new();
    let remote = sandbox.bare_remote();
    let unit = id(0x10, 1);

    let ada = sandbox.project("ada", "Ada");
    ada.set_remote("origin", &remote).expect("remote");
    write_shard(ada.root(), 0x10, &[(unit, 1, "Un", "")]);
    ada.checkpoint(None).expect("initial");
    ada.push().expect("push");
    let grace = sandbox.clone(&remote, "grace", "Grace");

    write_shard(ada.root(), 0x10, &[(unit, 1, "Uno", "")]);
    ada.checkpoint(None).expect("ada edit");
    ada.push().expect("push");
    write_shard(grace.root(), 0x10, &[(unit, 1, "Une", "")]);
    grace.checkpoint(None).expect("grace edit");
    let grace_head = grace.head().expect("head");

    grace.fetch().expect("fetch");
    match grace.integrate(&no_resolutions(), || Ok(())) {
        Err(GitError::TranslationConflicts { conflicts }) => {
            assert_eq!(conflicts.len(), 1);
            let conflict = &conflicts[0];
            assert_eq!(conflict.context, unit.to_string());
            assert_eq!(conflict.ours.translation, "Une");
            assert_eq!(conflict.theirs.translation, "Uno");
        }
        other => panic!("expected translation conflicts, got {other:?}"),
    }
    assert_eq!(grace.head().expect("head"), grace_head);
    let status = grace.status().expect("status");
    assert!(!status.merge_in_progress);
    assert!(status.files.is_empty());

    let resolutions = BTreeMap::from([(unit.to_string(), ConflictResolution::Theirs)]);
    assert_eq!(
        grace.integrate(&resolutions, || Ok(())).expect("resolved"),
        IntegrateOutcome::Merged
    );
    assert!(shard_text(grace.root(), 0x10).contains("\"Uno\""));
}

#[test]
fn rejected_incoming_changes_are_rolled_back() {
    let sandbox = Sandbox::new();
    let remote = sandbox.bare_remote();

    let ada = sandbox.project("ada", "Ada");
    ada.set_remote("origin", &remote).expect("remote");
    write_shard(ada.root(), 0x10, &[(id(0x10, 1), 1, "Un", "")]);
    ada.checkpoint(None).expect("initial");
    ada.push().expect("push");

    let grace = sandbox.clone(&remote, "grace", "Grace");
    let before = grace.head().expect("head");

    write_shard(ada.root(), 0x20, &[(id(0x20, 1), 2, "Deux", "")]);
    ada.checkpoint(None).expect("second");
    ada.push().expect("push");

    grace.fetch().expect("fetch");
    match grace.integrate(&no_resolutions(), || Err("source changed".to_owned())) {
        Err(GitError::IncomingRejected { reason }) => assert_eq!(reason, "source changed"),
        other => panic!("expected rejection, got {other:?}"),
    }
    assert_eq!(grace.head().expect("head"), before);
    assert!(!grace.root().join("po/20.po").exists());

    // A rejected branch switch restores the previous branch.
    grace.create_branch("experiment").expect("create");
    match grace.switch_branch("main", || Err("blocked".to_owned())) {
        Err(GitError::IncomingRejected { .. }) => {}
        other => panic!("expected rejection, got {other:?}"),
    }
    assert_eq!(current_branch(&grace), "experiment");
    grace.switch_branch("main", || Ok(())).expect("switch");
    assert_eq!(current_branch(&grace), "main");
}

#[test]
fn a_project_in_a_repository_subdirectory_uses_project_relative_paths() {
    let sandbox = Sandbox::new();
    let top = sandbox.path("monorepo");
    fs::create_dir_all(&top).expect("top");
    sandbox.raw_git(&top, &["init", "--quiet", "--initial-branch=main"]);
    let root = top.join("translations/fr");
    make_project(&root);
    fs::write(top.join("README.md"), "unrelated\n").expect("readme");

    let repository = GitRepository::open(&root, sandbox.git.clone()).expect("open");
    set_translator(&repository, "Ada");
    let unit = id(0x7a, 9);
    write_shard(&root, 0x7a, &[(unit, 1, "Bonjour", "")]);

    let status = repository.status().expect("status");
    assert!(
        status
            .files
            .iter()
            .all(|file| file.path.starts_with("po/") || file.path == "aeria.json")
    );
    repository.checkpoint(None).expect("checkpoint");

    write_shard(&root, 0x7a, &[(unit, 1, "Salut", "")]);
    let history = repository
        .entry_history(&shard_path(0x7a), &unit.to_string(), 10, 100)
        .expect("history");
    assert!(history.pending.is_some());
    assert_eq!(history.revisions.len(), 1);
    let log = repository.log(0, 10).expect("log");
    let (_, changes) = repository.commit_changes(&log[0].id).expect("changes");
    assert_eq!(changes.len(), 1);
    // Unrelated repository files are never committed by a checkpoint.
    assert!(
        GitRepository::open(&top, sandbox.git.clone())
            .expect("top")
            .status()
            .expect("top status")
            .files
            .iter()
            .any(|file| file.path == "README.md")
    );
}

#[test]
fn clone_into_names_the_folder_after_the_remote_and_never_overwrites() {
    let sandbox = Sandbox::new();
    let remote = sandbox.bare_remote();
    let ada = sandbox.project("ada", "Ada");
    ada.set_remote("origin", &remote).expect("remote");
    ada.checkpoint(None).expect("initial");
    ada.push().expect("push");

    let parent = sandbox.path("projects/nested");
    let clone = GitRepository::clone_into(&remote, &parent, sandbox.git.clone()).expect("clone");
    assert_eq!(clone.root(), parent.join("remote"));
    assert!(parent.join("remote/aeria.json").is_file());

    assert!(matches!(
        GitRepository::clone_into(&remote, &parent, sandbox.git.clone()),
        Err(GitError::InvalidInput {
            field: "clone destination",
            ..
        })
    ));
}

/// Every repository operation works when the remote, the projects, a nested
/// project folder, and the clone parent have Cyrillic names with spaces, as
/// they do under a Russian Windows user profile.
#[test]
fn cyrillic_paths_with_spaces_work_for_every_operation() {
    let sandbox = Sandbox::new();
    let remote = sandbox.path("общий репозиторий.git");
    fs::create_dir_all(&remote).expect("remote dir");
    sandbox.raw_git(
        &remote,
        &["init", "--quiet", "--bare", "--initial-branch=main"],
    );
    let remote = remote.to_string_lossy().into_owned();

    // A project in a Cyrillic subfolder of a Cyrillic repository.
    let top = sandbox.path("мои проекты");
    fs::create_dir_all(&top).expect("top");
    sandbox.raw_git(&top, &["init", "--quiet", "--initial-branch=main"]);
    let root = top.join("переводы/русский перевод");
    make_project(&root);
    let ada = GitRepository::open(&root, sandbox.git.clone()).expect("open");
    set_translator(&ada, "Ада");
    ada.set_remote("origin", &remote)
        .expect("remote with spaces");
    let unit = id(0x31, 1);
    write_shard(&root, 0x31, &[(unit, 1, "Привет", "")]);
    assert!(
        ada.status()
            .expect("status")
            .files
            .iter()
            .all(|file| file.path.starts_with("po/") || file.path == "aeria.json")
    );
    ada.checkpoint(None).expect("checkpoint");
    assert!(ada.push().expect("push"));

    // The clone lands in a folder named after the remote inside a nested
    // Cyrillic parent.
    let parent = sandbox.path("клоны/вложенная папка");
    let grace = GitRepository::clone_into(&remote, &parent, sandbox.git.clone()).expect("clone");
    assert_eq!(grace.root(), parent.join("общий репозиторий"));
    set_translator(&grace, "Грейс");
    let grace_project = grace.root().join("переводы/русский перевод");
    let grace = GitRepository::open(&grace_project, sandbox.git.clone()).expect("open clone");
    set_translator(&grace, "Грейс");

    write_shard(&root, 0x31, &[(unit, 1, "Здравствуйте", "")]);
    ada.checkpoint(None).expect("second checkpoint");
    ada.push().expect("second push");
    grace.fetch().expect("fetch");
    assert_eq!(
        grace
            .integrate(&no_resolutions(), || Ok(()))
            .expect("fast-forward"),
        IntegrateOutcome::FastForward
    );
    let history = grace
        .entry_history(&shard_path(0x31), &unit.to_string(), 10, 100)
        .expect("history");
    assert_eq!(history.revisions.len(), 2);
    let log = grace.log(0, 10).expect("log");
    let (_, changes) = grace.commit_changes(&log[0].id).expect("changes");
    assert_eq!(changes.len(), 1);
}

#[test]
fn project_files_are_committed_only_by_checkpoints() {
    let sandbox = Sandbox::new();
    let repository = sandbox.project("project", "Ada");
    let root = repository.root().to_owned();
    write_shard(&root, 0x7a, &[(id(0x7a, 1), 1, "Bonjour", "")]);
    repository.checkpoint(None).expect("first checkpoint");
    let first = repository.head().expect("head");

    fs::write(root.join(PACK_SETTINGS_FILE), "{}\n").expect("pack");
    fs::write(root.join(FONT_SETTINGS_FILE), "{}\n").expect("fonts");
    fs::create_dir_all(root.join(FONTS_DIR)).expect("dir");
    fs::write(root.join(FONTS_DIR).join("a.ttf"), [0u8, 1, 2]).expect("font");
    fs::create_dir_all(root.join(KNOWLEDGE_DIR)).expect("knowledge");
    fs::write(
        root.join(KNOWLEDGE_DIR).join("terms.csv"),
        "term,translation\n",
    )
    .expect("terms");
    fs::write(root.join(KNOWLEDGE_DIR).join("style.md"), "Use ты.\n").expect("style");

    // Nothing was committed by writing the files.
    assert_eq!(repository.head().expect("head"), first);
    assert_eq!(repository.status().expect("status").files.len(), 5);
    assert_eq!(
        repository
            .file_at("HEAD", PACK_SETTINGS_FILE)
            .expect("show"),
        None
    );

    let outcome = repository.checkpoint(None).expect("checkpoint");
    assert_eq!(current_branch(&repository), "main");
    assert_eq!(outcome.commit.subject, "Update project settings");
    assert!(repository.status().expect("status").files.is_empty());
    assert_eq!(
        repository
            .file_at("HEAD", "aeria-knowledge/style.md")
            .expect("show"),
        Some("Use ты.\n".as_bytes().to_vec())
    );

    // Removing a font file is committed too.
    fs::remove_file(root.join(FONTS_DIR).join("a.ttf")).expect("remove");
    fs::write(root.join(FONTS_DIR).join("b.ttf"), [3u8]).expect("font");
    repository
        .checkpoint(Some("Replace font"))
        .expect("checkpoint");
    assert!(repository.status().expect("status").files.is_empty());
    assert!(repository.file_at("HEAD", "../outside").is_err());
}
#[test]
fn tags_are_listed_by_prefix() {
    let sandbox = Sandbox::new();
    let repository = sandbox.project("project", "Ada");
    let root = repository.root().to_owned();
    write_shard(&root, 0x7a, &[(id(0x7a, 1), 1, "Bonjour", "")]);
    repository.checkpoint(None).expect("checkpoint");
    for tag in ["harmonia/3", "harmonia/12", "other"] {
        sandbox.raw_git(&root, &["tag", tag]);
    }
    let mut tags = repository.tags_with_prefix("harmonia/").expect("tags");
    tags.sort();
    assert_eq!(tags, ["harmonia/12", "harmonia/3"]);
}

#[test]
fn remotes_and_the_upstream_can_be_changed() {
    let sandbox = Sandbox::new();
    let first = sandbox.bare_remote();
    let repository = sandbox.project("project", "Ada");
    repository.set_remote("origin", &first).expect("remote");
    write_shard(repository.root(), 0x10, &[(id(0x10, 1), 1, "Un", "")]);
    repository.checkpoint(None).expect("checkpoint");
    repository.push().expect("push");

    let second = sandbox.bare_remote();
    repository
        .set_remote("backup", &second)
        .expect("second remote");
    sandbox.raw_git(repository.root(), &["push", "--quiet", "backup", "main"]);
    sandbox.raw_git(repository.root(), &["fetch", "--quiet", "backup"]);
    assert_eq!(
        repository.remote_branches().expect("branches"),
        ["backup/main", "origin/main"]
    );

    repository.set_upstream("backup/main").expect("upstream");
    assert_eq!(
        repository.status().expect("status").upstream.as_deref(),
        Some("backup/main")
    );
    assert!(repository.set_upstream("nowhere/main").is_err());

    // The log shows where branches point.
    let head = &repository.log(0, 1).expect("log")[0];
    assert!(
        head.refs.iter().any(|name| name == "HEAD -> main"),
        "{:?}",
        head.refs
    );
    assert!(
        head.refs.iter().any(|name| name == "backup/main"),
        "{:?}",
        head.refs
    );

    repository.remove_remote("origin").expect("remove");
    assert_eq!(repository.remotes().expect("remotes").len(), 1);
    assert!(repository.remove_remote("origin").is_err());
}

#[test]
fn checkpoints_and_pushes_stay_on_the_current_branch() {
    let sandbox = Sandbox::new();
    let remote = sandbox.bare_remote();
    let repository = sandbox.project("project", "Ada");
    repository.set_remote("origin", &remote).expect("remote");
    write_shard(repository.root(), 0x10, &[(id(0x10, 1), 1, "Un", "")]);
    repository.checkpoint(None).expect("initial");
    assert!(repository.push().expect("publish main"));

    // A checkpoint on the published main branch is made there and pushed.
    write_shard(repository.root(), 0x10, &[(id(0x10, 1), 1, "Une", "")]);
    repository.checkpoint(None).expect("checkpoint");
    assert_eq!(current_branch(&repository), "main");
    assert!(repository.push().expect("push main"));

    // A clone records the remote's default branch.
    let clone = sandbox.clone(&remote, "clone", "Grace");
    assert_eq!(
        clone.remote_default_branch().expect("default").as_deref(),
        Some("main")
    );
    assert_eq!(repository.remote_default_branch().expect("default"), None);
}

#[test]
fn branches_are_deleted_locally_and_on_the_remote_and_lost_work_needs_force() {
    let sandbox = Sandbox::new();
    let remote = sandbox.bare_remote();
    let repository = sandbox.project("project", "Ada");
    let root = repository.root().to_owned();
    repository.set_remote("origin", &remote).expect("remote");
    write_shard(&root, 0x10, &[(id(0x10, 1), 1, "Un", "")]);
    repository.checkpoint(None).expect("initial");
    repository.push().expect("publish main");

    // A published branch with work main does not have.
    repository.create_branch("feature").expect("branch");
    write_shard(&root, 0x10, &[(id(0x10, 1), 1, "Une", "")]);
    repository.checkpoint(None).expect("feature work");
    repository.push().expect("publish feature");
    // A local branch with work that exists nowhere else.
    sandbox.raw_git(&root, &["switch", "--quiet", "main"]);
    repository.create_branch("draft").expect("branch");
    write_shard(&root, 0x10, &[(id(0x10, 1), 1, "Unes", "")]);
    repository.checkpoint(None).expect("draft work");
    sandbox.raw_git(&root, &["switch", "--quiet", "main"]);

    assert!(
        repository.delete_branch("main", false, true).is_err(),
        "the current branch stays"
    );
    // Published work is kept by the remote branch; deleting both loses it.
    assert_eq!(repository.lost_commits("feature", false).expect("lost"), 0);
    assert_eq!(repository.lost_commits("feature", true).expect("lost"), 1);
    assert_eq!(repository.lost_commits("draft", false).expect("lost"), 1);
    assert!(repository.delete_branch("draft", false, false).is_err());
    repository
        .delete_branch("draft", false, true)
        .expect("delete draft with force");
    assert_eq!(repository.branch_head("draft").expect("head"), None);

    assert!(repository.delete_branch("feature", true, false).is_err());
    repository
        .delete_branch("feature", false, false)
        .expect("delete the local branch only");
    assert_eq!(repository.branch_head("feature").expect("head"), None);
    assert_eq!(
        repository
            .lost_remote_commits("origin/feature")
            .expect("lost"),
        1
    );
    assert!(
        repository
            .delete_remote_branch("origin/feature", false)
            .is_err()
    );
    repository
        .delete_remote_branch("origin/feature", true)
        .expect("delete on the remote");
    assert!(
        !repository
            .branches()
            .expect("branches")
            .iter()
            .any(|branch| branch.name == "origin/feature")
    );
    let remote_heads = sandbox.raw_git(&root, &["ls-remote", "--heads", "origin"]);
    assert!(
        !remote_heads.contains("refs/heads/feature"),
        "{remote_heads}"
    );
    assert!(remote_heads.contains("refs/heads/main"), "{remote_heads}");

    // A local branch and its upstream go together.
    repository.create_branch("merged").expect("branch");
    repository.push().expect("publish merged");
    sandbox.raw_git(&root, &["switch", "--quiet", "main"]);
    assert_eq!(repository.lost_commits("merged", true).expect("lost"), 0);
    repository
        .delete_branch("merged", true, false)
        .expect("delete both");
    assert_eq!(repository.branch_head("merged").expect("head"), None);
    let remote_heads = sandbox.raw_git(&root, &["ls-remote", "--heads", "origin"]);
    assert!(
        !remote_heads.contains("refs/heads/merged"),
        "{remote_heads}"
    );
}
