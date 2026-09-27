//! Workspace Format v3 persistence.

use std::fs;
use std::path::Path;
use std::str::FromStr;

use aeria_core::{
    DetachReason, LayoutHash, ReviewState, SourceBinding, SourceFacts, SourceStatus,
    TranslationUnit, TranslationUnitId, WorkspaceMetadata,
};
use aeria_workspace::{
    Workspace, WorkspaceError, WorkspaceStore, WorkspaceStoreError, decode_unit_record,
    decode_unit_shard, encode_unit_shard,
};
use tempfile::{TempDir, tempdir};

const FIXTURE_MANIFEST: &[u8] = include_bytes!("fixtures/workspace-v3/manifest.json");
const FIXTURE_00: &[u8] = include_bytes!("fixtures/workspace-v3/units/00.jsonl");
const FIXTURE_FF: &[u8] = include_bytes!("fixtures/workspace-v3/units/ff.jsonl");
const ID_00: &str = "00000000000000000000000000000000";
const ID_01: &str = "00000000000000000000000000000001";
const ID_FF: &str = "ffffffffffffffffffffffffffffffff";

fn metadata() -> WorkspaceMetadata {
    WorkspaceMetadata::new("en", "fr", "2026.09.15.0000.0000".parse().expect("version"))
        .expect("metadata")
}

fn id(text: &str) -> TranslationUnitId {
    TranslationUnitId::from_str(text).expect("ID")
}

fn fixture_repository() -> TempDir {
    let repository = tempdir().expect("repository");
    let units = repository.path().join(".aeria/units");
    fs::create_dir_all(&units).expect("directories");
    fs::write(
        repository.path().join(".aeria/manifest.json"),
        FIXTURE_MANIFEST,
    )
    .expect("manifest");
    fs::write(units.join("00.jsonl"), FIXTURE_00).expect("00");
    fs::write(units.join("ff.jsonl"), FIXTURE_FF).expect("ff");
    repository
}

/// A repository with the fixture manifest and one `00.jsonl` shard.
fn repository_with_shard(shard: &str) -> TempDir {
    let repository = tempdir().expect("repository");
    let units = repository.path().join(".aeria/units");
    fs::create_dir_all(&units).expect("directories");
    fs::write(
        repository.path().join(".aeria/manifest.json"),
        FIXTURE_MANIFEST,
    )
    .expect("manifest");
    fs::write(units.join("00.jsonl"), shard).expect("shard");
    repository
}

fn repository_with_manifest(manifest: &str) -> TempDir {
    let repository = tempdir().expect("repository");
    fs::create_dir_all(repository.path().join(".aeria")).expect("directory");
    fs::write(repository.path().join(".aeria/manifest.json"), manifest).expect("manifest");
    repository
}

fn first_record() -> String {
    String::from_utf8(FIXTURE_00.to_vec())
        .expect("UTF-8")
        .lines()
        .next()
        .expect("record")
        .to_owned()
}

fn invalid_data(repository: &TempDir) -> String {
    match WorkspaceStore::new(repository.path()).load() {
        Err(WorkspaceStoreError::InvalidData { message, .. }) => message,
        other => panic!("expected invalid data, got {other:?}"),
    }
}

#[test]
fn an_empty_workspace_has_a_manifest_and_no_units_directory() {
    let repository = tempdir().expect("repository");
    let workspace = Workspace::new(metadata());
    WorkspaceStore::new(repository.path())
        .initialize(&workspace)
        .expect("initialize");
    assert_eq!(
        fs::read(repository.path().join(".aeria/manifest.json")).expect("manifest"),
        FIXTURE_MANIFEST
    );
    assert!(!repository.path().join(".aeria/units").exists());
    assert_eq!(
        WorkspaceStore::new(repository.path()).load().expect("load"),
        workspace
    );
}

#[test]
fn the_golden_fixture_loads_and_writes_back_byte_for_byte() {
    let input = fixture_repository();
    let workspace = WorkspaceStore::new(input.path()).load().expect("load");

    let first = workspace.unit(id(ID_00)).expect("first unit");
    assert!(first.is_bound());
    assert_eq!(
        first.source_binding(),
        &SourceBinding::new("翻訳表", 0, 2, u32::MAX)
    );
    assert_eq!(
        first.source().layout(),
        LayoutHash::from_bytes([0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef])
    );
    assert_eq!(first.source().text(), "<i>Bonjour</i>\n<br>");
    assert_eq!(first.source().row_key(), Some("TEXT_KEY_000"));
    assert_eq!(first.target_macro(), "Salut\u{7} «monde»");
    assert_eq!(first.review_state(), ReviewState::Reviewed);
    assert_eq!(first.translator_note(), Some("контекст"));

    let removed = workspace.unit(id(ID_01)).expect("second unit");
    assert_eq!(
        removed.source_status(),
        SourceStatus::Detached(DetachReason::RowRemoved)
    );
    assert_eq!(removed.target_macro(), "");
    assert_eq!(removed.review_state(), ReviewState::NeedsReview);
    assert!(
        workspace
            .unit_by_source_binding(removed.source_binding())
            .is_none()
    );
    assert_eq!(workspace.detached_units().count(), 2);

    let output = tempdir().expect("output");
    WorkspaceStore::new(output.path())
        .initialize(&workspace)
        .expect("initialize");
    for (path, expected) in [
        (".aeria/manifest.json", FIXTURE_MANIFEST),
        (".aeria/units/00.jsonl", FIXTURE_00),
        (".aeria/units/ff.jsonl", FIXTURE_FF),
    ] {
        assert_eq!(
            fs::read(output.path().join(path)).expect(path),
            expected,
            "{path}"
        );
    }
    assert_eq!(
        WorkspaceStore::new(input.path())
            .read_metadata()
            .expect("metadata"),
        metadata()
    );
}

#[test]
fn accepted_but_non_canonical_files_are_reported() {
    let input = fixture_repository();
    let store = WorkspaceStore::new(input.path());
    assert!(store.non_canonical_files().expect("check").is_empty());
    let shard = input.path().join(".aeria/units/00.jsonl");
    let crlf = String::from_utf8(FIXTURE_00.to_vec())
        .expect("UTF-8")
        .replace('\n', "\r\n");
    fs::write(&shard, crlf).expect("CRLF shard");
    assert_eq!(store.non_canonical_files().expect("check"), [shard]);
}

#[test]
fn unrelated_repository_files_are_ignored() {
    let repository = fixture_repository();
    fs::write(repository.path().join("README.md"), b"project").expect("file");
    fs::create_dir(repository.path().join("docs")).expect("directory");
    WorkspaceStore::new(repository.path()).load().expect("load");
}

#[test]
fn persisting_a_unit_rewrites_only_its_shard() {
    let repository = fixture_repository();
    let store = WorkspaceStore::new(repository.path());
    let mut workspace = store.load().expect("load");
    workspace
        .update_target(id(ID_FF), "Changé")
        .expect_err("a detached unit is not edited");
    workspace
        .update_note(id(ID_00), Some("nouvelle note".to_owned()))
        .expect("note");
    store.persist_unit(&workspace, id(ID_00)).expect("persist");
    assert_eq!(
        fs::read(repository.path().join(".aeria/units/ff.jsonl")).expect("ff"),
        FIXTURE_FF
    );
    let reloaded = WorkspaceStore::new(repository.path())
        .load()
        .expect("reload");
    assert_eq!(reloaded, workspace);
    assert!(
        String::from_utf8(fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00"))
            .expect("UTF-8")
            .contains(r#""note":"nouvelle note""#)
    );
}

fn unit(sheet: &str, row: u32, text: &str) -> TranslationUnit {
    let binding = SourceBinding::new(sheet, row, 0, 0);
    let id = TranslationUnitId::derive(&binding, text).expect("ID");
    TranslationUnit::new(
        id,
        SourceFacts::new(binding, LayoutHash::from_bytes([7; 8]), text, None),
        "t",
    )
}

#[test]
fn persisting_rejects_absent_units_changed_source_facts_and_owned_bindings() {
    let repository = fixture_repository();
    let store = WorkspaceStore::new(repository.path());
    let workspace = store.load().expect("load");
    let absent = unit("Other", 1, "absent").id();
    assert!(matches!(
        store.persist_unit(&workspace, absent),
        Err(WorkspaceStoreError::Domain(
            WorkspaceError::UnitNotFound { .. }
        ))
    ));

    // A workspace whose unit claims the fixture's bound binding.
    let mut claiming = Workspace::new(metadata());
    let facts = SourceFacts::new(
        SourceBinding::new("翻訳表", 0, 2, u32::MAX),
        LayoutHash::from_bytes([1; 8]),
        "Other text",
        None,
    );
    let claimed = claiming.create_unit(facts, "x").expect("unit");
    let fresh = WorkspaceStore::new(repository.path());
    let before = fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00");
    assert!(fresh.persist_unit(&claiming, claimed).is_err());
    assert_eq!(
        fs::read(repository.path().join(".aeria/units/00.jsonl")).expect("00"),
        before
    );
}

#[test]
fn initialization_never_replaces_existing_state() {
    let repository = fixture_repository();
    assert!(matches!(
        WorkspaceStore::new(repository.path()).initialize(&Workspace::new(metadata())),
        Err(WorkspaceStoreError::AlreadyInitialized { .. })
    ));
    assert_eq!(
        fs::read(repository.path().join(".aeria/manifest.json")).expect("manifest"),
        FIXTURE_MANIFEST
    );
}

#[test]
fn the_managed_namespace_is_strict() {
    let extra = fixture_repository();
    fs::write(extra.path().join(".aeria/extra.json"), b"{}").expect("file");
    assert!(WorkspaceStore::new(extra.path()).load().is_err());

    let nested = fixture_repository();
    fs::create_dir(nested.path().join(".aeria/units/nested")).expect("directory");
    assert!(WorkspaceStore::new(nested.path()).load().is_err());

    let named = fixture_repository();
    fs::write(named.path().join(".aeria/units/0g.jsonl"), FIXTURE_00).expect("file");
    assert!(WorkspaceStore::new(named.path()).load().is_err());

    let empty = repository_with_shard("");
    assert!(WorkspaceStore::new(empty.path()).load().is_err());

    let missing = tempdir().expect("repository");
    assert!(matches!(
        WorkspaceStore::new(missing.path()).load(),
        Err(WorkspaceStoreError::MissingPath { .. })
    ));
}

#[test]
fn manifests_must_be_version_3_with_every_valid_field() {
    let valid = r#"{"formatVersion":3,"sourceLanguage":"en","targetLanguage":"fr","gameVersion":"2026.09.15.0000.0000"}"#;
    WorkspaceStore::new(repository_with_manifest(valid).path())
        .load()
        .expect("a compact manifest is accepted");
    for (manifest, unsupported) in [
        (
            valid.replace("\"formatVersion\":3", "\"formatVersion\":2"),
            true,
        ),
        (
            valid.replace("\"formatVersion\":3", "\"formatVersion\":4"),
            true,
        ),
        (
            valid.replace(",\"gameVersion\":\"2026.09.15.0000.0000\"", ""),
            false,
        ),
        (valid.replace('}', ",\"contentId\":\"x\"}"), false),
        (valid.replace("2026.09.15.0000.0000", "latest"), false),
        (valid.replace("\"en\"", "\"ru\""), false),
        (valid.replace("\"fr\"", "\" \""), false),
    ] {
        let repository = repository_with_manifest(&manifest);
        let result = WorkspaceStore::new(repository.path()).load();
        if unsupported {
            assert!(
                matches!(
                    result,
                    Err(WorkspaceStoreError::UnsupportedFormatVersion { .. })
                ),
                "{manifest}: {result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(WorkspaceStoreError::InvalidData { .. })),
                "{manifest}: {result:?}"
            );
        }
    }
}

#[test]
fn unit_records_are_validated_field_by_field() {
    let record = first_record();
    let second = String::from_utf8(FIXTURE_00.to_vec())
        .expect("UTF-8")
        .lines()
        .nth(1)
        .expect("second record")
        .to_owned();
    let cases = [
        record.replace(ID_00, "ff000000000000000000000000000000"),
        format!("{second}\n{record}\n"),
        format!("{record}\n{record}\n"),
        record.replace("0123456789abcdef", "0123456789ABCDEF"),
        record.replace("0123456789abcdef", "01234567"),
        record.replace(r#""source":"<i>Bonjour</i>\n<br>""#, r#""source":"""#),
        record.replace(r#""key":"TEXT_KEY_000""#, r#""key":"""#),
        record.replace(r#""sheet":"翻訳表""#, r#""sheet":"""#),
        record.replace(r#""review":"reviewed""#, r#""review":"approved""#),
        record.replace(r#""status":"bound""#, r#""status":"gone""#),
        record.replace(
            r#""target":"Salut\u0007 «monde»""#,
            r#""target":"<if $n1>""#,
        ),
        record.replace(r#","note":"контекст""#, ""),
        record.replace(r#""note":"контекст""#, r#""note":"контекст","extra":1"#),
        record.replace(r#""row":0"#, r#""row":-1"#),
        record.replace(r#""subrow":2"#, r#""subrow":65536"#),
    ];
    for shard in cases {
        let shard = if shard.ends_with('\n') {
            shard
        } else {
            format!("{shard}\n")
        };
        let repository = repository_with_shard(&shard);
        invalid_data(&repository);
    }
}

#[test]
fn duplicate_bound_bindings_are_readable_but_not_editable() {
    let record = first_record();
    let other = record.replace(ID_00, "00000000000000000000000000000002");
    let repository = repository_with_shard(&format!("{record}\n{other}\n"));
    assert!(invalid_data(&repository).contains("duplicate"));
}

#[test]
fn the_reader_accepts_bom_crlf_and_a_missing_final_lf_but_not_blank_lines_or_bare_cr() {
    let record = first_record();
    for accepted in [
        format!("\u{feff}{record}\n"),
        format!("{record}\r\n"),
        record.clone(),
    ] {
        WorkspaceStore::new(repository_with_shard(&accepted).path())
            .load()
            .expect("accepted");
    }
    for rejected in [
        format!("{record}\n\n"),
        format!("{record}\r"),
        format!(" \n{record}\n"),
        format!("{record}\n\u{feff}"),
    ] {
        assert!(
            WorkspaceStore::new(repository_with_shard(&rejected).path())
                .load()
                .is_err(),
            "{rejected:?}"
        );
    }
}

#[test]
fn shard_helpers_decode_and_encode_canonical_records() {
    let path = Path::new(".aeria/units/00.jsonl");
    let units = decode_unit_shard(FIXTURE_00, path).expect("decode");
    assert_eq!(units.len(), 2);
    assert_eq!(encode_unit_shard(&units, path).expect("encode"), FIXTURE_00);
    let record = decode_unit_record(&format!("{}\r\n", first_record()), path).expect("record");
    assert_eq!(record, units[0]);
    assert!(decode_unit_record(&first_record(), Path::new(".aeria/units/ff.jsonl")).is_err());
    assert!(encode_unit_shard(&[units[0].clone(), units[0].clone()], path).is_err());
    assert!(encode_unit_shard(&units, Path::new("units.jsonl")).is_err());
}
