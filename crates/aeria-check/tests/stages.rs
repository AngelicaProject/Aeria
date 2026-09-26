use std::fs;
use std::path::Path;
use std::process::Command;

use aeria_check::{Severity, Stage, run_stage};

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../aeria-workspace/tests/fixtures/workspace-v2")
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new(
        std::env::var_os("AERIA_GIT_PATH")
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "git".into()),
    )
    .current_dir(root)
    .args([
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "-c",
        "commit.gpgsign=false",
    ])
    .args(args)
    .status()
    .expect("run git");
    assert!(status.success(), "git {args:?}");
}

/// A repository whose project is in `project/`, with the fixture committed.
fn repository() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("folder");
    let aeria = folder.path().join("project/.aeria/units");
    fs::create_dir_all(&aeria).expect("units");
    fs::copy(
        fixture().join("manifest.json"),
        folder.path().join("project/.aeria/manifest.json"),
    )
    .expect("manifest");
    for shard in ["00.jsonl", "ff.jsonl"] {
        fs::copy(fixture().join("units").join(shard), aeria.join(shard)).expect("shard");
    }
    git(folder.path(), &["init", "--quiet"]);
    git(folder.path(), &["add", "."]);
    git(folder.path(), &["commit", "--quiet", "-m", "base"]);
    folder
}

#[test]
fn a_canonical_project_passes_every_stage() {
    let repository = repository();
    let project = repository.path().join("project");
    for stage in [Stage::Integrity, Stage::Translations] {
        let report = run_stage(stage, &project, None);
        assert!(
            report.findings.is_empty(),
            "{stage:?}: {:?}",
            report.findings
        );
    }
    let merge = run_stage(Stage::Merge, &project, Some("HEAD"));
    assert!(!merge.failed());
    assert!(
        merge
            .findings
            .iter()
            .all(|finding| finding.severity == Severity::Notice)
    );
}

#[test]
fn text_edits_fail_and_removed_units_are_reported() {
    let repository = repository();
    let project = repository.path().join("project");
    let shard = project.join(".aeria/units/00.jsonl");
    let text = fs::read_to_string(&shard).expect("shard");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() > 1, "the fixture shard has several units");

    // Dropping a unit keeps the shard canonical, but the merge notices it.
    fs::write(&shard, format!("{}\n", lines[1..].join("\n"))).expect("remove a unit");
    git(repository.path(), &["commit", "--quiet", "-am", "remove"]);
    let merge = run_stage(Stage::Merge, &project, Some("HEAD^1"));
    let warning = merge
        .findings
        .iter()
        .find(|finding| finding.severity == Severity::Warning)
        .expect("removal warning");
    assert!(
        warning.message.contains("removes 1 translation unit"),
        "{}",
        warning.message
    );

    // CRLF is readable but not what Aeria writes.
    fs::write(&shard, text.replace('\n', "\r\n")).expect("CRLF");
    let translations = run_stage(Stage::Translations, &project, None);
    assert!(translations.failed());
    assert_eq!(
        translations.findings[0].path.as_deref(),
        Some(".aeria/units/00.jsonl")
    );

    // A conflict marker breaks the shard.
    fs::write(
        &shard,
        format!("<<<<<<< HEAD\n{text}=======\n>>>>>>> branch\n"),
    )
    .expect("conflict");
    let integrity = run_stage(Stage::Integrity, &project, None);
    assert!(integrity.failed());
    assert_eq!(integrity.findings[0].line, Some(1));
}
