use std::fs;
use std::path::Path;
use std::str::FromStr;

use aeria_core::{ReviewState, TranslationUnitId, WorkspaceMetadata};
use aeria_workspace::{Workspace, WorkspaceError, WorkspaceStore, WorkspaceStoreError};
use tempfile::{TempDir, tempdir};

const FIXTURE_MANIFEST: &[u8] = include_bytes!("fixtures/workspace-v1/manifest.json");
const FIXTURE_00: &[u8] = include_bytes!("fixtures/workspace-v1/units/00.jsonl");
const FIXTURE_FF: &[u8] = include_bytes!("fixtures/workspace-v1/units/ff.jsonl");
const ID_00: &str = "tu1:0000000000000000000000000000000000000000000000000000000000000000";
const ID_FF: &str = "tu1:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
const FIRST_BINDING: &str =
    r#""sourceBinding":{"sheetName":"翻訳表","rowId":0,"subrowId":2,"columnIndex":4294967295}"#;
const SECOND_BINDING: &str =
    r#""sourceBinding":{"sheetName":"Synthetic","rowId":42,"subrowId":0,"columnIndex":1}"#;
const OTHER_BINDING: &str =
    r#""sourceBinding":{"sheetName":"Other","rowId":4294967295,"subrowId":65535,"columnIndex":0}"#;

#[test]
fn initializes_empty_workspace_without_units_directory() {
    let repository = tempdir().expect("temporary repository");
    let workspace = Workspace::new(metadata());

    WorkspaceStore::new(repository.path())
        .initialize(&workspace)
        .expect("empty workspace initializes");

    assert_eq!(
        fs::read(repository.path().join(".aeria/manifest.json")).expect("manifest"),
        FIXTURE_MANIFEST
    );
    assert!(!repository.path().join(".aeria/units").exists());
    assert_eq!(
        WorkspaceStore::new(repository.path())
            .load()
            .expect("empty workspace loads"),
        workspace
    );
}

#[test]
fn golden_fixture_loads_and_writer_bytes_are_canonical() {
    let input = fixture_repository();
    let workspace = WorkspaceStore::new(input.path())
        .load()
        .expect("fixture loads");

    let first = workspace
        .unit(TranslationUnitId::from_str(ID_00).expect("ID"))
        .expect("first unit");
    assert_eq!(first.source_binding().sheet_name(), "翻訳表");
    assert_eq!(
        first.target_macro(),
        "Quote \" slash \\ line\n tab\t control\0"
    );
    assert_eq!(first.review_state(), ReviewState::Draft);
    assert_eq!(first.translator_note(), None);

    let reviewed = workspace
        .unit(
            TranslationUnitId::from_str(
                "tu1:0000000000000000000000000000000000000000000000000000000000000001",
            )
            .expect("ID"),
        )
        .expect("reviewed unit");
    assert_eq!(reviewed.review_state(), ReviewState::Reviewed);
    assert_eq!(reviewed.translator_note(), Some("note \"quoted\""));

    let needs_review = workspace
        .unit(TranslationUnitId::from_str(ID_FF).expect("ID"))
        .expect("needs-review unit");
    assert_eq!(needs_review.review_state(), ReviewState::NeedsReview);
    assert_eq!(needs_review.target_macro(), "");

    let output = tempdir().expect("output repository");
    WorkspaceStore::new(output.path())
        .initialize(&workspace)
        .expect("fixture initializes");

    assert_eq!(
        fs::read(output.path().join(".aeria/manifest.json")).expect("manifest"),
        FIXTURE_MANIFEST
    );
    assert_eq!(
        fs::read(output.path().join(".aeria/units/00.jsonl")).expect("00 shard"),
        FIXTURE_00
    );
    assert_eq!(
        fs::read(output.path().join(".aeria/units/ff.jsonl")).expect("ff shard"),
        FIXTURE_FF
    );
    assert_eq!(
        WorkspaceStore::new(output.path())
            .load()
            .expect("round trip loads"),
        workspace
    );
}

#[test]
fn unrelated_repository_files_are_ignored() {
    let repository = fixture_repository();
    fs::write(repository.path().join("README.md"), b"project-owned").expect("unrelated file");
    fs::create_dir(repository.path().join("docs")).expect("unrelated directory");

    WorkspaceStore::new(repository.path())
        .load()
        .expect("unrelated repository content is ignored");
}

#[test]
fn persist_rewrites_only_the_selected_shard_and_preserves_complete_canonical_bytes() {
    let repository = fixture_repository();
    let store = WorkspaceStore::new(repository.path());
    let mut workspace = store.load().expect("fixture loads");
    let ff_before = fs::read(repository.path().join(".aeria/units/ff.jsonl")).expect("ff shard");
    let first_id = TranslationUnitId::from_str(ID_00).expect("ID");
    workspace
        .update_target(first_id, "updated")
        .expect("target is valid");

    store
        .persist_unit(&workspace, first_id)
        .expect("selected shard persists");

    assert_eq!(
        fs::read(repository.path().join(".aeria/units/ff.jsonl")).expect("ff shard"),
        ff_before
    );
    let reloaded = store.load().expect("persisted workspace loads");
    assert_eq!(
        reloaded.unit(first_id).expect("first unit").target_macro(),
        "updated"
    );
    let managed_entries: Vec<_> = fs::read_dir(repository.path().join(".aeria"))
        .expect("managed directory")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert!(
        managed_entries
            .iter()
            .all(|name| name != "manifest.json.tmp")
    );
}

#[test]
fn persist_unit_merges_a_partial_workspace_without_deleting_persisted_units() {
    let repository = fixture_repository();
    let store = WorkspaceStore::new(repository.path());
    let first_id = TranslationUnitId::from_str(ID_00).expect("ID");
    let second_id = TranslationUnitId::from_str(
        "tu1:0000000000000000000000000000000000000000000000000000000000000001",
    )
    .expect("ID");
    let before = store.load().expect("fixture loads");
    let partial_workspace = load_partial_workspace("00.jsonl", &first_fixture_record());

    store
        .persist_unit(&partial_workspace, first_id)
        .expect("selected unit persists");

    assert_eq!(
        fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00 shard"),
        FIXTURE_00
    );
    let after = store.load().expect("persisted workspace loads");
    assert_eq!(after.unit(first_id), partial_workspace.unit(first_id));
    assert_eq!(after.unit(second_id), before.unit(second_id));
}

#[test]
fn persist_unit_rejects_an_absent_id_without_modifying_the_shard() {
    let repository = fixture_repository();
    let store = WorkspaceStore::new(repository.path());
    let first_id = TranslationUnitId::from_str(ID_00).expect("ID");
    let absent_id = TranslationUnitId::from_str(
        "tu1:0000000000000000000000000000000000000000000000000000000000000001",
    )
    .expect("ID");
    let partial_workspace = load_partial_workspace("00.jsonl", &first_fixture_record());
    let before = fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00 shard");

    let error = store
        .persist_unit(&partial_workspace, absent_id)
        .expect_err("absent unit must fail");
    assert!(matches!(
        error,
        WorkspaceStoreError::Domain(WorkspaceError::UnitNotFound { id }) if id == absent_id
    ));
    assert_eq!(
        fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00 shard"),
        before
    );
    assert!(partial_workspace.unit(first_id).is_some());
}

#[test]
fn persist_unit_rejects_source_binding_transition_without_modifying_the_shard() {
    let repository = fixture_repository();
    let store = WorkspaceStore::new(repository.path());
    let id = TranslationUnitId::from_str(ID_00).expect("ID");
    let before = fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00 shard");
    let replacement = first_fixture_record().replace(FIRST_BINDING, SECOND_BINDING);
    let partial_workspace = load_partial_workspace("00.jsonl", &replacement);

    let error = store
        .persist_unit(&partial_workspace, id)
        .expect_err("source binding transition must fail");
    assert!(error.to_string().contains("SourceBinding"));
    assert_eq!(
        fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00 shard"),
        before
    );
}

#[test]
fn persist_unit_rejects_source_fingerprint_transition_without_modifying_the_shard() {
    let repository = fixture_repository();
    let store = WorkspaceStore::new(repository.path());
    let id = TranslationUnitId::from_str(ID_00).expect("ID");
    let before = fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00 shard");
    let replacement = first_fixture_record().replace(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    );
    let partial_workspace = load_partial_workspace("00.jsonl", &replacement);

    let error = store
        .persist_unit(&partial_workspace, id)
        .expect_err("source fingerprint transition must fail");
    assert!(error.to_string().contains("SourceFingerprint"));
    assert_eq!(
        fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00 shard"),
        before
    );
}

#[test]
fn persist_unit_rejects_new_id_with_binding_owned_in_another_shard() {
    let repository = fixture_repository();
    let store = WorkspaceStore::new(repository.path());
    let before_00 = fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00 shard");
    let before_ff = fs::read(repository.path().join(".aeria/units/ff.jsonl")).expect("ff shard");
    let new_id_text = format!("tu1:fe{}", "00".repeat(31));
    let replacement = std::str::from_utf8(FIXTURE_FF)
        .expect("fixture UTF-8")
        .replace(ID_FF, &new_id_text)
        .replace(OTHER_BINDING, FIRST_BINDING);
    let partial_workspace = load_partial_workspace("fe.jsonl", &replacement);
    let new_id = TranslationUnitId::from_str(&new_id_text).expect("ID");

    let error = store
        .persist_unit(&partial_workspace, new_id)
        .expect_err("duplicate persisted binding must fail");
    assert!(error.to_string().contains("already owned"));
    assert_eq!(
        fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00 shard"),
        before_00
    );
    assert_eq!(
        fs::read(repository.path().join(".aeria/units/ff.jsonl")).expect("ff shard"),
        before_ff
    );
    assert!(!repository.path().join(".aeria/units/fe.jsonl").exists());
}

#[test]
fn initialization_refuses_existing_state() {
    let repository = fixture_repository();
    let error = WorkspaceStore::new(repository.path())
        .initialize(&Workspace::new(metadata()))
        .expect_err("existing state must not be replaced");
    assert!(error.to_string().contains("already exists"));
    assert_eq!(
        fs::read(repository.path().join(".aeria/units/ff.jsonl")).expect("ff"),
        FIXTURE_FF
    );
}

#[test]
fn initialization_validation_failure_does_not_publish_partial_state() {
    let repository = tempdir().expect("temporary repository");
    let workspace = Workspace::new(
        WorkspaceMetadata::new("en", "fr", "not-an-hxs-id", "not-an-hxs-id")
            .expect("domain metadata permits the persistence-invalid value"),
    );

    let error = WorkspaceStore::new(repository.path())
        .initialize(&workspace)
        .expect_err("invalid persistence metadata must fail");
    assert!(error.to_string().contains("canonical sha256"));
    assert!(!repository.path().join(".aeria").exists());
}

#[test]
fn strict_namespace_and_shard_validation_rejects_invalid_repositories() {
    let cases = [
        ("unexpected.json", "unexpected entry inside .aeria/"),
        ("units/not-a-shard.jsonl", "invalid unit shard filename"),
        ("units/00.jsonl", "empty shard files are not allowed"),
    ];
    for (relative_path, expected) in cases {
        let repository = minimal_repository();
        let path = repository.path().join(".aeria").join(relative_path);
        if path.extension().is_some() {
            fs::create_dir_all(path.parent().expect("parent")).expect("parent");
            fs::write(
                &path,
                if relative_path.ends_with("00.jsonl") {
                    b"".as_slice()
                } else {
                    b"{}\n".as_slice()
                },
            )
            .expect("invalid entry");
        } else {
            fs::create_dir_all(&path).expect("invalid directory");
        }
        let error = WorkspaceStore::new(repository.path())
            .load()
            .expect_err("invalid namespace must fail");
        assert!(error.to_string().contains(expected), "{error}");
    }

    let repository = minimal_repository();
    fs::create_dir_all(repository.path().join(".aeria/units/nested")).expect("nested directory");
    let error = WorkspaceStore::new(repository.path())
        .load()
        .expect_err("nested units directory must fail");
    assert!(error.to_string().contains("invalid unit shard filename"));
}

#[test]
fn manifest_validation_rejects_missing_unknown_duplicate_unsupported_and_noncanonical_fields() {
    let zero = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    let one = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    let cases = vec![
        (
            format!(
                r#"{{"formatVersion":1,"sourceLanguage":"en","targetLanguage":"fr","contentId":"{zero}"}}"#
            ),
            "missing field",
        ),
        (
            format!(
                r#"{{"formatVersion":1,"sourceLanguage":"en","targetLanguage":"fr","contentId":"{zero}","snapshotId":"{one}","extra":true}}"#
            ),
            "unknown field",
        ),
        (
            format!(
                r#"{{"formatVersion":1,"formatVersion":1,"sourceLanguage":"en","targetLanguage":"fr","contentId":"{zero}","snapshotId":"{one}"}}"#
            ),
            "duplicate field",
        ),
        (
            format!(
                r#"{{"formatVersion":2,"sourceLanguage":"en","targetLanguage":"fr","contentId":"{zero}","snapshotId":"{one}"}}"#
            ),
            "unsupported Workspace Format version",
        ),
        (
            format!(
                r#"{{"formatVersion":1,"sourceLanguage":"en","targetLanguage":"fr","contentId":"SHA256:{zero}","snapshotId":"{one}"}}"#
            ),
            "canonical sha256",
        ),
    ];
    for (manifest, expected) in cases {
        let repository = minimal_repository_with_manifest(&manifest);
        let error = WorkspaceStore::new(repository.path())
            .load()
            .expect_err("invalid manifest must fail");
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn unit_reader_rejects_wrong_shard_unsorted_duplicate_binding_bad_hash_review_and_target() {
    let valid_first = String::from_utf8(FIXTURE_00.to_vec()).expect("fixture UTF-8");
    let first_line = valid_first.lines().next().expect("first line");
    let second_line = valid_first.lines().nth(1).expect("second line");
    let cases = [
        ("01.jsonl", first_line.to_owned(), "wrong shard"),
        (
            "00.jsonl",
            format!("{second_line}\n{first_line}\n"),
            "strictly increasing",
        ),
        (
            "00.jsonl",
            format!("{first_line}\n{first_line}\n"),
            "duplicate TranslationUnitId",
        ),
        (
            "00.jsonl",
            first_line.replace("\"rowId\":0", "\"rowId\":4294967296"),
            "expected u32",
        ),
        (
            "00.jsonl",
            first_line.replace(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            ),
            "lowercase hexadecimal",
        ),
        (
            "00.jsonl",
            first_line.replace("\"reviewState\":\"draft\"", "\"reviewState\":\"unknown\""),
            "reviewState",
        ),
        (
            "00.jsonl",
            first_line.replace("control\\u0000", "<if(1,2>"),
            "targetMacro",
        ),
    ];
    for (filename, line, expected) in cases {
        let repository = minimal_repository_with_shard(filename, &(line + "\n"));
        let error = WorkspaceStore::new(repository.path())
            .load()
            .expect_err("invalid unit must fail");
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn duplicate_source_bindings_across_shards_and_missing_nullable_fields_are_rejected() {
    let repository = fixture_repository();
    let ff_path = repository.path().join(".aeria/units/ff.jsonl");
    let ff = String::from_utf8(fs::read(&ff_path).expect("ff shard")).expect("UTF-8");
    fs::write(
        &ff_path,
        ff.replace(
            r#"{"sheetName":"Other","rowId":4294967295,"subrowId":65535,"columnIndex":0}"#,
            r#"{"sheetName":"翻訳表","rowId":0,"subrowId":2,"columnIndex":4294967295}"#,
        ),
    )
    .expect("duplicate binding shard");
    let error = WorkspaceStore::new(repository.path())
        .load()
        .expect_err("duplicate binding must fail");
    assert!(
        error
            .to_string()
            .contains("duplicate current SourceBinding")
    );

    for missing in ["rawValueHash", "translatorNote"] {
        let repository = fixture_repository();
        let path = repository.path().join(".aeria/units/00.jsonl");
        let source = String::from_utf8(fs::read(&path).expect("00 shard")).expect("UTF-8");
        let needle = if missing == "rawValueHash" {
            ",\"rawValueHash\":null"
        } else {
            ",\"translatorNote\":null"
        };
        fs::write(&path, source.replace(needle, "")).expect("missing required nullable field");
        let error = WorkspaceStore::new(repository.path())
            .load()
            .expect_err("missing nullable field must fail");
        assert!(error.to_string().contains("missing field"));
    }
}

#[test]
fn jsonl_reader_accepts_bom_crlf_and_missing_final_lf_but_rejects_blank_and_bare_cr() {
    let first_line = String::from_utf8(FIXTURE_00.to_vec())
        .expect("UTF-8")
        .lines()
        .next()
        .expect("line")
        .to_owned();
    let cases = [
        (format!("\u{feff}{first_line}\n"), true),
        (
            String::from_utf8(FIXTURE_00.to_vec())
                .expect("UTF-8")
                .replace('\n', "\r\n"),
            true,
        ),
        (
            String::from_utf8(FIXTURE_00.to_vec())
                .expect("UTF-8")
                .trim_end_matches('\n')
                .to_owned(),
            true,
        ),
        (
            format!(
                "{first}\n\n",
                first = String::from_utf8(FIXTURE_00.to_vec())
                    .expect("UTF-8")
                    .lines()
                    .next()
                    .expect("line")
            ),
            false,
        ),
        (
            format!(
                "{}\r",
                String::from_utf8(FIXTURE_00.to_vec())
                    .expect("UTF-8")
                    .lines()
                    .next()
                    .expect("line")
            ),
            false,
        ),
    ];
    for (contents, valid) in cases {
        let repository = minimal_repository_with_shard("00.jsonl", &contents);
        let result = WorkspaceStore::new(repository.path()).load();
        assert_eq!(result.is_ok(), valid, "{contents:?}: {result:?}");
    }
}

#[test]
fn managed_manifest_symlinks_are_rejected_when_the_platform_supports_them() {
    let repository = fixture_repository();
    let outside = tempdir().expect("outside temporary directory");
    let target = outside.path().join("manifest.json");
    fs::write(&target, FIXTURE_MANIFEST).expect("outside manifest");
    let manifest = repository.path().join(".aeria/manifest.json");
    fs::remove_file(&manifest).expect("remove manifest");
    if let Err(error) = create_file_symlink(&target, &manifest) {
        if matches!(
            error.kind(),
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
        ) || error.raw_os_error() == Some(1314)
        {
            return;
        }
        panic!("symlink creation failed: {error}");
    }

    let error = WorkspaceStore::new(repository.path())
        .load()
        .expect_err("managed symlink must fail");
    assert!(error.to_string().contains("symlinks are not allowed"));
}

fn metadata() -> WorkspaceMetadata {
    WorkspaceMetadata::new(
        "en",
        "fr",
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    )
    .expect("canonical metadata")
}

fn fixture_repository() -> TempDir {
    let repository = minimal_repository();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace-v1/units/00.jsonl"),
        repository.path().join(".aeria/units/00.jsonl"),
    )
    .expect("00 fixture");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace-v1/units/ff.jsonl"),
        repository.path().join(".aeria/units/ff.jsonl"),
    )
    .expect("ff fixture");
    repository
}

fn minimal_repository() -> TempDir {
    minimal_repository_with_manifest(std::str::from_utf8(FIXTURE_MANIFEST).expect("manifest UTF-8"))
}

fn minimal_repository_with_manifest(manifest: &str) -> TempDir {
    let repository = tempdir().expect("temporary repository");
    fs::create_dir_all(repository.path().join(".aeria/units")).expect("workspace directories");
    fs::write(repository.path().join(".aeria/manifest.json"), manifest).expect("manifest");
    repository
}

fn minimal_repository_with_shard(filename: &str, contents: &str) -> TempDir {
    let repository = minimal_repository_with_manifest(
        std::str::from_utf8(FIXTURE_MANIFEST).expect("manifest UTF-8"),
    );
    fs::write(
        repository.path().join(".aeria/units").join(filename),
        contents,
    )
    .expect("shard");
    repository
}

fn first_fixture_record() -> String {
    std::str::from_utf8(FIXTURE_00)
        .expect("fixture UTF-8")
        .lines()
        .next()
        .expect("first fixture record")
        .to_owned()
        + "\n"
}

fn load_partial_workspace(filename: &str, contents: &str) -> Workspace {
    let repository = minimal_repository_with_shard(filename, contents);
    WorkspaceStore::new(repository.path())
        .load()
        .expect("partial workspace loads")
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}
