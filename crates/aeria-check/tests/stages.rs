use std::fs;
use std::path::Path;
use std::process::Command;

use aeria_check::{Severity, Stage, run_stage};

const SETTINGS: &str = "{\n  \"format\": \"aeria-po/1\",\n  \"sourceLanguage\": \"en\",\n  \"targetLanguage\": \"ru\",\n  \"gameVersion\": \"2026.09.15.0000.0000\"\n}\n";

const ADDON: &str = "# Addon
msgid \"\"
msgstr \"\"
\"Language: ru\\n\"
\"X-Game-Version: 2026.09.15.0000.0000\\n\"

#. de: Ok
msgctxt \"Addon:1:0:0\"
msgid \"OK\"
msgstr \"ОК\"

msgctxt \"Addon:2:0:0\"
msgid \"Cancel\"
msgstr \"Отмена\"

msgctxt \"Addon:3:0:0\"
msgid \"Quit <If(PlayerParameter(4))>now<Else/>later</If>\"
msgstr \"\"
";

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

/// A repository whose project is in `project/`, committed.
fn repository() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("folder");
    let project = folder.path().join("project");
    fs::create_dir_all(project.join("po")).expect("po");
    fs::write(project.join("aeria.json"), SETTINGS).expect("settings");
    fs::write(project.join("po/Addon.po"), ADDON).expect("addon");
    fs::write(project.join(".gitattributes"), "*.po text eol=lf\n").expect("attributes");
    git(folder.path(), &["init", "--quiet"]);
    git(folder.path(), &["add", "."]);
    git(folder.path(), &["commit", "--quiet", "-m", "base"]);
    folder
}

#[test]
fn a_valid_project_passes_every_stage() {
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
            .all(|finding| finding.severity == Severity::Notice),
        "{:?}",
        merge.findings
    );
}

#[test]
fn broken_translations_fail_and_removed_ones_are_reported() {
    let repository = repository();
    let project = repository.path().join("project");
    let path = project.join("po/Addon.po");

    // Removing a translation is noticed by the merge stage.
    fs::write(&path, ADDON.replace("msgstr \"Отмена\"", "msgstr \"\"")).expect("remove");
    git(repository.path(), &["commit", "--quiet", "-am", "remove"]);
    let merge = run_stage(Stage::Merge, &project, Some("HEAD^1"));
    let warning = merge
        .findings
        .iter()
        .find(|finding| finding.severity == Severity::Warning)
        .expect("removal warning");
    assert!(
        warning.message.contains("removes 1 translation(s)")
            && warning.message.contains("Addon:2:0:0"),
        "{}",
        warning.message
    );

    // A translation that breaks the source's macros fails.
    fs::write(
        &path,
        ADDON.replace(
            "msgstr \"\"\n",
            "msgstr \"Выйти <If(PlayerParameter(4))>сейчас\"\n",
        ),
    )
    .expect("broken");
    let translations = run_stage(Stage::Translations, &project, None);
    assert!(translations.failed(), "{:?}", translations.findings);
    assert_eq!(
        translations.findings[0].path.as_deref(),
        Some("po/Addon.po")
    );

    // A conflict marker breaks the file.
    fs::write(
        &path,
        format!("<<<<<<< HEAD\n{ADDON}=======\n>>>>>>> branch\n"),
    )
    .expect("conflict");
    let integrity = run_stage(Stage::Integrity, &project, None);
    assert!(integrity.failed());
    assert_eq!(integrity.findings[0].line, Some(1));
}
