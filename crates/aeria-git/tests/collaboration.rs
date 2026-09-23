//! Integration tests over synthetic Workspace Format v1 repositories.
//!
//! Tests require the `git` executable (`AERIA_GIT_PATH` or `PATH`). They
//! isolate Git from user and system configuration and use only local bare
//! remotes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use aeria_core::{ReviewState, TranslationUnitId};
use aeria_git::{
    CollaborationPolicy, CollaborationSettings, ConflictResolution, GitError, GitExecutable,
    GitRepository, IntegrateOutcome, RecordVersion, UnitChangeKind,
};
use tempfile::TempDir;

const HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const MANIFEST: &str = "{\n  \"formatVersion\": 1,\n  \"sourceLanguage\": \"en\",\n  \"targetLanguage\": \"fr\",\n  \"contentId\": \"sha256:1111111111111111111111111111111111111111111111111111111111111111\",\n  \"snapshotId\": \"sha256:2222222222222222222222222222222222222222222222222222222222222222\"\n}\n";

fn git_program() -> std::ffi::OsString {
    std::env::var_os("AERIA_GIT_PATH")
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "git".into())
}

fn no_resolutions() -> BTreeMap<TranslationUnitId, ConflictResolution> {
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
        fs::create_dir_all(root.join(".aeria")).expect("project dir");
        fs::write(root.join(".aeria/manifest.json"), MANIFEST).expect("manifest");
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

fn set_translator(repository: &GitRepository, translator: &str) {
    let email = format!(
        "{}@example.invalid",
        translator.to_lowercase().replace(' ', ".")
    );
    repository
        .set_identity(translator, Some(&email), false)
        .expect("identity");
}

fn id(first_byte: u8, last: u8) -> TranslationUnitId {
    let mut bytes = [0u8; 32];
    bytes[0] = first_byte;
    bytes[31] = last;
    TranslationUnitId::from_bytes(bytes)
}

type Record<'a> = (TranslationUnitId, u32, &'a str, &'a str);

fn record((id, row, target, review): Record<'_>) -> String {
    format!(
        "{{\"id\":\"{id}\",\"sourceBinding\":{{\"sheetName\":\"Addon\",\"rowId\":{row},\"subrowId\":0,\"columnIndex\":0}},\"sourceFingerprint\":{{\"macroTextHash\":\"{HASH}\",\"rawValueHash\":null,\"rowTechnicalHash\":\"{HASH}\"}},\"targetMacro\":\"{target}\",\"reviewState\":\"{review}\",\"translatorNote\":null}}\n"
    )
}

/// Writes a complete shard containing the given records, sorted by ID.
fn write_shard(root: &Path, shard: u8, records: &[Record<'_>]) {
    let dir = root.join(".aeria/units");
    fs::create_dir_all(&dir).expect("units dir");
    let mut records = records.to_vec();
    records.sort_by_key(|(id, ..)| *id);
    let text: String = records.into_iter().map(record).collect();
    fs::write(dir.join(format!("{shard:02x}.jsonl")), text).expect("shard");
}

fn shard_text(root: &Path, shard: u8) -> String {
    fs::read_to_string(root.join(format!(".aeria/units/{shard:02x}.jsonl"))).expect("shard")
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
fn checkpoint_attributes_translations_and_builds_unit_history() {
    let sandbox = Sandbox::new();
    let repository = sandbox.project("project", "Ada");
    let root = repository.root().to_owned();
    let unit = id(0x7a, 1);

    let attributes = fs::read_to_string(root.join(".gitattributes")).expect("attributes");
    assert!(attributes.contains("/.aeria/** text eol=lf"));

    write_shard(&root, 0x7a, &[(unit, 1, "Bonjour", "draft")]);
    let pending = repository.pending_changes().expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, UnitChangeKind::Added);

    let first = repository.checkpoint(None).expect("first checkpoint");
    assert_eq!(first.commit.author_name, "Ada");
    assert_eq!(first.commit.subject, "Translate 1 string (Addon)");
    assert!(repository.pending_changes().expect("clean").is_empty());
    assert!(matches!(
        repository.checkpoint(None),
        Err(GitError::NothingToCommit)
    ));

    set_translator(&repository, "Grace");
    write_shard(&root, 0x7a, &[(unit, 1, "Bonjour", "reviewed")]);
    let review = repository
        .checkpoint(Some("Review greeting"))
        .expect("review checkpoint");
    assert_eq!(review.commit.subject, "Review greeting");

    write_shard(&root, 0x7a, &[(unit, 1, "Coucou", "draft")]);
    let history = repository.unit_history(unit, 10).expect("history");
    let pending = history.pending.expect("pending change");
    assert_eq!(pending.after.expect("after").target_macro(), "Coucou");
    assert_eq!(history.revisions.len(), 2);
    assert!(!history.truncated);
    assert_eq!(
        history.translated_by.expect("translator").author_name,
        "Ada"
    );
    assert_eq!(history.reviewed_by.expect("reviewer").author_name, "Grace");

    let newest = &history.revisions[0];
    assert_eq!(newest.commit.author_name, "Grace");
    assert_eq!(newest.kind, UnitChangeKind::Modified);
    let RecordVersion::Valid(after) = &newest.after else {
        panic!("expected a valid record");
    };
    assert_eq!(after.review_state(), ReviewState::Reviewed);
    assert_eq!(history.revisions[1].kind, UnitChangeKind::Added);

    let limited = repository.unit_history(unit, 1).expect("limited history");
    assert_eq!(limited.revisions.len(), 1);
    assert!(limited.truncated);

    let (commit, changes) = repository
        .commit_changes(&review.commit.id)
        .expect("commit changes");
    assert_eq!(commit.id, review.commit.id);
    assert_eq!(changes.len(), 1);
    assert!(!changes[0].target_changed());
    assert!(changes[0].review_changed());

    let log = repository.log(0, 10).expect("log");
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].id, review.commit.id);
}

#[test]
fn contributors_credit_translators_and_reviewers() {
    let sandbox = Sandbox::new();
    let repository = sandbox.project("project", "Ada");
    let root = repository.root().to_owned();
    let (first, second, third) = (id(0x01, 1), id(0x02, 1), id(0x02, 2));

    write_shard(&root, 0x01, &[(first, 1, "Un", "draft")]);
    write_shard(&root, 0x02, &[(second, 2, "Deux", "draft")]);
    repository.checkpoint(None).expect("Ada checkpoint");

    set_translator(&repository, "Grace");
    write_shard(&root, 0x01, &[(first, 1, "Un", "reviewed")]);
    write_shard(
        &root,
        0x02,
        &[(second, 2, "Deux", "draft"), (third, 3, "Trois", "draft")],
    );
    repository.checkpoint(None).expect("Grace checkpoint");

    let attribution = repository.attribution().expect("attribution");
    assert_eq!(attribution.len(), 3);
    let first_entry = &attribution[0];
    assert_eq!(first_entry.id, first);
    let name = |attribution: Option<&aeria_git::Attribution>| {
        attribution.expect("attribution").author_name.clone()
    };
    assert_eq!(name(first_entry.translated_by.as_ref()), "Ada");
    assert_eq!(name(first_entry.reviewed_by.as_ref()), "Grace");
    assert_eq!(name(first_entry.last_changed_by.as_ref()), "Grace");

    let contributors = repository.contributors().expect("contributors");
    assert_eq!(contributors.len(), 2);
    let ada = contributors.iter().find(|c| c.name == "Ada").expect("Ada");
    assert_eq!((ada.translated, ada.reviewed), (2, 0));
    let grace = contributors
        .iter()
        .find(|c| c.name == "Grace")
        .expect("Grace");
    assert_eq!((grace.translated, grace.reviewed), (1, 1));
}

#[test]
fn the_email_is_optional_and_never_derived_from_the_host() {
    let sandbox = Sandbox::new();
    let root = sandbox.path("anonymous");
    fs::create_dir_all(root.join(".aeria")).expect("dir");
    fs::write(root.join(".aeria/manifest.json"), MANIFEST).expect("manifest");
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

    write_shard(&root, 0x10, &[(id(0x10, 1), 1, "Un", "draft")]);
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
            (one, 1, "Un", "draft"),
            (two, 2, "Deux", "draft"),
            (three, 3, "Trois", "draft"),
        ],
    );
    ada.checkpoint(None).expect("initial");
    assert!(ada.push().expect("publish branch"));
    assert!(!ada.push().expect("nothing to push"));

    let grace = sandbox.clone(&remote, "grace", "Grace");
    let grace_root = grace.root().to_owned();

    // Fast-forward.
    write_shard(&ada_root, 0x20, &[(id(0x20, 1), 4, "Quatre", "draft")]);
    ada.checkpoint(None).expect("ada second");
    ada.push().expect("push");
    grace.fetch().expect("fetch");
    assert_eq!(
        grace
            .integrate(&no_resolutions(), || Ok(()))
            .expect("fast-forward"),
        IntegrateOutcome::FastForward
    );
    assert!(grace_root.join(".aeria/units/20.jsonl").exists());

    // Adjacent units in one shard: a textual conflict for Git, a clean
    // semantic merge for Aeria.
    write_shard(
        &ada_root,
        0x10,
        &[
            (one, 1, "Une", "draft"),
            (two, 2, "Deux", "draft"),
            (three, 3, "Trois", "draft"),
        ],
    );
    ada.checkpoint(None).expect("ada edits one");
    ada.push().expect("push");
    write_shard(
        &grace_root,
        0x10,
        &[
            (one, 1, "Un", "draft"),
            (two, 2, "Deux !", "draft"),
            (three, 3, "Trois", "draft"),
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
    write_shard(&grace_root, 0x20, &[(id(0x20, 1), 4, "Quatre !", "draft")]);
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
    write_shard(ada.root(), 0x10, &[(unit, 1, "Un", "draft")]);
    ada.checkpoint(None).expect("initial");
    ada.push().expect("push");
    let grace = sandbox.clone(&remote, "grace", "Grace");

    write_shard(ada.root(), 0x10, &[(unit, 1, "Uno", "draft")]);
    ada.checkpoint(None).expect("ada edit");
    ada.push().expect("push");
    write_shard(grace.root(), 0x10, &[(unit, 1, "Une", "draft")]);
    grace.checkpoint(None).expect("grace edit");
    let grace_head = grace.head().expect("head");

    grace.fetch().expect("fetch");
    match grace.integrate(&no_resolutions(), || Ok(())) {
        Err(GitError::TranslationConflicts { conflicts }) => {
            assert_eq!(conflicts.len(), 1);
            let conflict = &conflicts[0];
            assert_eq!(conflict.id, unit);
            assert_eq!(conflict.ours.as_ref().expect("ours").target_macro(), "Une");
            assert_eq!(
                conflict.theirs.as_ref().expect("theirs").target_macro(),
                "Uno"
            );
        }
        other => panic!("expected translation conflicts, got {other:?}"),
    }
    assert_eq!(grace.head().expect("head"), grace_head);
    let status = grace.status().expect("status");
    assert!(!status.merge_in_progress);
    assert!(status.files.is_empty());

    let resolutions = BTreeMap::from([(unit, ConflictResolution::Theirs)]);
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
    write_shard(ada.root(), 0x10, &[(id(0x10, 1), 1, "Un", "draft")]);
    ada.checkpoint(None).expect("initial");
    ada.push().expect("push");

    let grace = sandbox.clone(&remote, "grace", "Grace");
    let before = grace.head().expect("head");

    write_shard(ada.root(), 0x20, &[(id(0x20, 1), 2, "Deux", "draft")]);
    ada.checkpoint(None).expect("second");
    ada.push().expect("push");

    grace.fetch().expect("fetch");
    match grace.integrate(&no_resolutions(), || Err("source changed".to_owned())) {
        Err(GitError::IncomingRejected { reason }) => assert_eq!(reason, "source changed"),
        other => panic!("expected rejection, got {other:?}"),
    }
    assert_eq!(grace.head().expect("head"), before);
    assert!(!grace.root().join(".aeria/units/20.jsonl").exists());

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
fn the_pull_request_policy_uses_contribution_branches() {
    let sandbox = Sandbox::new();
    let remote = sandbox.bare_remote();

    let maintainer = sandbox.project("maintainer", "Ada");
    maintainer.set_remote("origin", &remote).expect("remote");
    write_shard(maintainer.root(), 0x10, &[(id(0x10, 1), 1, "Un", "draft")]);
    maintainer.checkpoint(None).expect("initial");
    maintainer
        .set_collaboration(&CollaborationSettings {
            policy: CollaborationPolicy::PullRequest,
            main_branch: Some("main".to_owned()),
        })
        .expect("policy");
    maintainer.push().expect("push");

    let translator = sandbox.clone(&remote, "translator", "Grace Hopper");
    assert_eq!(
        translator.collaboration().expect("settings").policy,
        CollaborationPolicy::PullRequest
    );
    let status = translator
        .contribution_status()
        .expect("status")
        .expect("pull-request policy");
    assert_eq!(status.branch, None);

    write_shard(
        translator.root(),
        0x20,
        &[(id(0x20, 1), 2, "Deux", "draft")],
    );
    let outcome = translator.checkpoint(None).expect("checkpoint");
    let branch = outcome.branch_created.expect("contribution branch");
    assert!(branch.starts_with("translations/grace-hopper-"), "{branch}");

    translator.fetch().expect("fetch");
    translator
        .integrate(&no_resolutions(), || Ok(()))
        .expect("nothing new");
    assert!(translator.push().expect("publish contribution"));
    let status = translator
        .contribution_status()
        .expect("status")
        .expect("pull-request policy");
    assert_eq!(status.branch.as_deref(), Some(branch.as_str()));
    assert!(status.published);
    assert_eq!(status.unmerged_commits, 1);

    // Under the policy a checkpoint on main moves to a contribution branch,
    // for the maintainer too.
    write_shard(
        maintainer.root(),
        0x30,
        &[(id(0x30, 1), 3, "Trois", "draft")],
    );
    let maintainer_work = maintainer.checkpoint(None).expect("maintainer work");
    assert!(maintainer_work.branch_created.is_some());

    // Main moves on through a reviewed merge; syncing the contribution
    // brings it in.
    let identity = [
        "-c",
        "user.name=Ada",
        "-c",
        "user.email=ada@example.invalid",
    ];
    sandbox.raw_git(maintainer.root(), &["switch", "--quiet", "main"]);
    let mut merge = identity.to_vec();
    merge.extend(["merge", "--no-edit", "--quiet", "--no-ff", "-"]);
    sandbox.raw_git(maintainer.root(), &merge);
    sandbox.raw_git(maintainer.root(), &["push", "--quiet", "origin", "main"]);
    translator.fetch().expect("fetch");
    assert_eq!(
        translator
            .integrate(&no_resolutions(), || Ok(()))
            .expect("merge main"),
        IntegrateOutcome::Merged
    );
    assert!(translator.root().join(".aeria/units/30.jsonl").exists());
    translator.push().expect("push");

    // The maintainer reviews and merges the contribution on the remote.
    maintainer.fetch().expect("fetch");
    let contribution_ref = format!("origin/{branch}");
    sandbox.raw_git(
        maintainer.root(),
        &[
            "-c",
            "user.name=Ada",
            "-c",
            "user.email=ada@example.invalid",
            "merge",
            "--no-edit",
            "--quiet",
            &contribution_ref,
        ],
    );
    sandbox.raw_git(maintainer.root(), &["push", "--quiet", "origin", "main"]);

    translator.fetch().expect("fetch");
    let status = translator
        .contribution_status()
        .expect("status")
        .expect("pull-request policy");
    assert_eq!(status.unmerged_commits, 0);
    let finished = translator.finish_contribution(|| Ok(())).expect("finish");
    assert_eq!(finished.deleted_branch.as_deref(), Some(branch.as_str()));
    assert!(translator.root().join(".aeria/units/20.jsonl").exists());
    assert_eq!(current_branch(&translator), "main");
}

#[test]
fn a_project_in_a_repository_subdirectory_uses_project_relative_paths() {
    let sandbox = Sandbox::new();
    let top = sandbox.path("monorepo");
    fs::create_dir_all(&top).expect("top");
    sandbox.raw_git(&top, &["init", "--quiet", "--initial-branch=main"]);
    let root = top.join("translations/fr");
    fs::create_dir_all(root.join(".aeria")).expect("project");
    fs::write(root.join(".aeria/manifest.json"), MANIFEST).expect("manifest");
    fs::write(top.join("README.md"), "unrelated\n").expect("readme");

    let repository = GitRepository::open(&root, sandbox.git.clone()).expect("open");
    set_translator(&repository, "Ada");
    let unit = id(0x7a, 9);
    write_shard(&root, 0x7a, &[(unit, 1, "Bonjour", "draft")]);

    let status = repository.status().expect("status");
    assert!(
        status
            .files
            .iter()
            .all(|file| file.path.starts_with(".aeria/"))
    );
    repository.checkpoint(None).expect("checkpoint");

    write_shard(&root, 0x7a, &[(unit, 1, "Salut", "draft")]);
    let history = repository.unit_history(unit, 10).expect("history");
    assert!(history.pending.is_some());
    assert_eq!(history.revisions.len(), 1);
    assert_eq!(repository.attribution().expect("attribution").len(), 1);
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
