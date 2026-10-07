use std::fs;
use std::path::Path;
use std::process::Command;

use aeria_guard::{Severity, Stage, run_stage};

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
    let merge = run_stage(Stage::Changes, &project, Some("HEAD"));
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

    // Removing a translation is noticed by the changes stage.
    fs::write(&path, ADDON.replace("msgstr \"Отмена\"", "msgstr \"\"")).expect("remove");
    git(repository.path(), &["commit", "--quiet", "-am", "remove"]);
    let merge = run_stage(Stage::Changes, &project, Some("HEAD^1"));
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

#[test]
fn the_games_text_changes_only_with_the_game_version() {
    let repository = repository();
    let project = repository.path().join("project");
    let path = project.join("po/Addon.po");

    // Editing a msgid while the game version stays fails.
    fs::write(&path, ADDON.replace("msgid \"Cancel\"", "msgid \"Abort\"")).expect("edit");
    let changes = run_stage(Stage::Changes, &project, Some("HEAD"));
    assert!(changes.failed(), "{:?}", changes.findings);
    let error = changes
        .findings
        .iter()
        .find(|finding| finding.severity == Severity::Error)
        .expect("error");
    assert_eq!(error.path.as_deref(), Some("po/Addon.po"));
    assert!(error.message.contains("Addon:2:0:0"), "{}", error.message);

    // A new PO file fails too.
    git(repository.path(), &["checkout", "--quiet", "--", "."]);
    fs::write(project.join("po/Extra.po"), ADDON).expect("extra");
    assert!(run_stage(Stage::Changes, &project, Some("HEAD")).failed());
    fs::remove_file(project.join("po/Extra.po")).expect("remove extra");

    // With a new game version the text is not compared, and the update is
    // left to the reviewer.
    fs::write(&path, ADDON.replace("msgid \"Cancel\"", "msgid \"Abort\"")).expect("edit");
    fs::write(
        project.join("aeria.json"),
        SETTINGS.replace("2026.09.15.0000.0000", "2026.10.01.0000.0000"),
    )
    .expect("settings");
    let update = run_stage(Stage::Changes, &project, Some("HEAD"));
    assert!(!update.failed(), "{:?}", update.findings);
    assert!(
        update
            .findings
            .iter()
            .any(|finding| finding.severity == Severity::Warning
                && finding.message.contains("2026.10.01.0000.0000"))
    );
}

#[test]
fn settings_and_workflows_are_flagged_and_the_review_lists_strings() {
    let repository = repository();
    let project = repository.path().join("project");
    fs::write(
        project.join("po/Addon.po"),
        ADDON.replace("msgstr \"Отмена\"", "msgstr \"Отменить\""),
    )
    .expect("translate");
    fs::write(project.join("aeria-pack.json"), "{}\n").expect("pack");
    fs::create_dir_all(repository.path().join(".github/workflows")).expect("workflows");
    fs::write(
        repository.path().join(".github/workflows/x.yml"),
        "on: push\n",
    )
    .expect("workflow");
    git(repository.path(), &["add", "."]);

    let changes = run_stage(Stage::Changes, &project, Some("HEAD"));
    let warnings: Vec<_> = changes
        .findings
        .iter()
        .filter(|finding| finding.severity == Severity::Warning)
        .map(|finding| finding.message.as_str())
        .collect();
    assert!(
        warnings
            .iter()
            .any(|message| message.contains("project/aeria-pack.json")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|message| message.contains(".github/workflows/x.yml")),
        "{warnings:?}"
    );

    let review = aeria_guard::review(&project, "HEAD").expect("review");
    assert!(
        review.contains("1 string(s) changed in 1 file(s): 1 changed"),
        "{review}"
    );
    assert!(
        review.contains("| `Addon:2:0:0` | changed | Cancel | Отмена | Отменить |"),
        "{review}"
    );
    assert!(review.contains("`.github/workflows/x.yml`"), "{review}");
}
