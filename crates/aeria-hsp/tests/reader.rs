use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use aeria_hsp::{
    GuidanceOccurrence, GuidanceSheetStatus, HspManifest, SourceGuidance, SourcePackage,
    compute_guidance_bundle_id, compute_package_id, compute_source_evidence_id,
};
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

fn fixture_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic.hsp")
}

#[test]
fn atlas_fixture_opens_and_materializes_verified_source() {
    let cache = tempdir().expect("cache directory");
    let package = SourcePackage::open(fixture_path(), cache.path()).expect("valid Atlas package");

    assert_eq!(package.game_version(), "test-game");
    assert_eq!(package.scope(), "full");
    assert_eq!(package.source_language(), "en");
    assert!(package.package_id().starts_with("sha256:"));
    assert!(package.materialized_hxs_path().is_file());
    assert_eq!(package.guidance().sheets.len(), 1);
    assert_eq!(
        package.guidance().sheets[0].status,
        GuidanceSheetStatus::Compatible
    );
    assert!(
        package
            .guidance_index()
            .is_translatable("Synthetic", 42, 0, 0)
    );
    assert!(
        !package
            .guidance_index()
            .is_translatable("Synthetic", 42, 0, 1)
    );
    assert!(
        !package
            .guidance_index()
            .is_translatable("Synthetic", 7, 0, 0)
    );
    assert!(
        fs::metadata(package.materialized_hxs_path())
            .expect("cached HXS")
            .len()
            > 0
    );
}

#[test]
fn atlas_fixture_evidence_id_remains_byte_compatible() {
    let cache = tempdir().expect("cache directory");
    let package = SourcePackage::open(fixture_path(), cache.path()).expect("valid Atlas package");
    let expected = package
        .guidance()
        .evidence_inputs
        .iter()
        .find(|input| input.language == package.source_language())
        .expect("source evidence input")
        .evidence_id
        .clone();

    assert_eq!(
        compute_source_evidence_id(package.source()).expect("source evidence"),
        expected
    );
}

#[test]
fn valid_cache_is_reused_only_after_component_verification() {
    let cache = tempdir().expect("cache directory");
    let first = SourcePackage::open(fixture_path(), cache.path()).expect("first open");
    let path = first.materialized_hxs_path().to_owned();
    drop(first);
    let original = fs::read(&path).expect("cached bytes");

    let second = SourcePackage::open(fixture_path(), cache.path()).expect("second open");
    assert_eq!(second.materialized_hxs_path(), path);
    assert_eq!(fs::read(path).expect("reused bytes"), original);
}

#[test]
fn verified_reopen_does_not_trust_a_tampered_package() {
    let directory = tempdir().expect("test directory");
    let package_path = directory.path().join("source.hsp");
    fs::copy(fixture_path(), &package_path).expect("package copy");
    let cache = directory.path().join("cache");

    SourcePackage::open(&package_path, &cache).expect("first open");
    fs::write(&package_path, b"tampered package").expect("tamper package");

    assert!(SourcePackage::open(&package_path, &cache).is_err());
}

#[test]
fn validated_package_can_relocate_its_runtime_path_without_reopening() {
    let cache = tempdir().expect("cache directory");
    let package = SourcePackage::open(fixture_path(), cache.path()).expect("valid package");
    let package_id = package.package_id().to_owned();
    let source_snapshot_id = package.source_snapshot_id().to_owned();
    let relocated = package.relocate_package_path("published.hsp");

    assert_eq!(relocated.package_path(), Path::new("published.hsp"));
    assert_eq!(relocated.package_id(), package_id);
    assert_eq!(relocated.source_snapshot_id(), source_snapshot_id);
}

#[test]
fn invalid_zip_duplicate_and_unlisted_entries_are_rejected() {
    let directory = tempdir().expect("test directory");
    let entries = fixture_entries();

    let invalid_zip = directory.path().join("invalid.hsp");
    fs::write(&invalid_zip, b"not a zip").expect("invalid archive");
    assert!(matches!(
        SourcePackage::open(&invalid_zip, directory.path()),
        Err(aeria_hsp::HspError::Archive { .. })
    ));

    let unlisted = directory.path().join("unlisted.hsp");
    let mut unlisted_entries = entries;
    unlisted_entries.push(("unexpected.txt".to_owned(), b"unexpected".to_vec()));
    write_archive(&unlisted, unlisted_entries);
    assert!(matches!(
        SourcePackage::open(&unlisted, directory.path()),
        Err(aeria_hsp::HspError::Archive { .. })
    ));
}

#[test]
fn manifest_component_rules_are_fail_closed() {
    let directory = tempdir().expect("test directory");
    let (entries, mut manifest) = load_fixture();

    let package_id_mismatch = directory.path().join("package-id.hsp");
    manifest.package_id = format!("sha256:{}", "0".repeat(64));
    write_manifest_archive(&package_id_mismatch, entries.clone(), &manifest);
    assert!(matches!(
        SourcePackage::open(&package_id_mismatch, directory.path()),
        Err(aeria_hsp::HspError::Manifest { .. })
    ));

    let unsafe_path = directory.path().join("unsafe-path.hsp");
    let mut unsafe_manifest = load_manifest();
    unsafe_manifest.components[0].path = "../guidance.json".to_owned();
    unsafe_manifest.package_id = compute_package_id(&unsafe_manifest).expect("package hash");
    write_manifest_archive(&unsafe_path, entries.clone(), &unsafe_manifest);
    assert!(matches!(
        SourcePackage::open(&unsafe_path, directory.path()),
        Err(aeria_hsp::HspError::Manifest { .. })
    ));

    let size_mismatch = directory.path().join("size.hsp");
    let mut size_manifest = load_manifest();
    size_manifest.components[0].size += 1;
    size_manifest.package_id = compute_package_id(&size_manifest).expect("package hash");
    write_manifest_archive(&size_mismatch, entries.clone(), &size_manifest);
    assert!(matches!(
        SourcePackage::open(&size_mismatch, directory.path()),
        Err(aeria_hsp::HspError::Component { .. })
    ));

    let hash_mismatch = directory.path().join("hash.hsp");
    let mut hash_manifest = load_manifest();
    hash_manifest.components[0].sha256 = format!("sha256:{}", "0".repeat(64));
    hash_manifest.package_id = compute_package_id(&hash_manifest).expect("package hash");
    write_manifest_archive(&hash_mismatch, entries.clone(), &hash_manifest);
    assert!(matches!(
        SourcePackage::open(&hash_mismatch, directory.path()),
        Err(aeria_hsp::HspError::Component { .. })
    ));

    let mut missing_manifest = load_manifest();
    missing_manifest
        .components
        .retain(|component| component.kind != "sourceHxs");
    missing_manifest.package_id = compute_package_id(&missing_manifest).expect("package hash");
    let missing = directory.path().join("missing-source.hsp");
    write_manifest_archive(&missing, entries, &missing_manifest);
    assert!(matches!(
        SourcePackage::open(&missing, directory.path()),
        Err(aeria_hsp::HspError::Manifest { .. })
    ));
}

#[test]
fn unknown_optional_component_is_checked_and_unknown_required_is_rejected() {
    let directory = tempdir().expect("test directory");
    let (mut entries, mut manifest) = load_fixture();
    let optional_bytes = b"future component".to_vec();
    entries.push(("optional/future.bin".to_owned(), optional_bytes.clone()));
    manifest.components.push(aeria_hsp::HspComponentDescriptor {
        id: "future".to_owned(),
        kind: "future".to_owned(),
        format_version: 1,
        required: false,
        path: "optional/future.bin".to_owned(),
        size: i64::try_from(optional_bytes.len()).expect("size"),
        sha256: hash_bytes(&optional_bytes),
    });
    manifest.package_id = compute_package_id(&manifest).expect("package hash");
    let optional = directory.path().join("optional.hsp");
    write_manifest_archive(&optional, entries.clone(), &manifest);
    assert!(SourcePackage::open(&optional, directory.path()).is_ok());

    manifest
        .components
        .last_mut()
        .expect("future component")
        .required = true;
    manifest.package_id = compute_package_id(&manifest).expect("package hash");
    let required = directory.path().join("required.hsp");
    write_manifest_archive(&required, entries, &manifest);
    assert!(matches!(
        SourcePackage::open(&required, directory.path()),
        Err(aeria_hsp::HspError::Manifest { .. })
    ));
}

#[test]
fn oversized_manifest_is_rejected_before_json_allocation() {
    let directory = tempdir().expect("test directory");
    let (mut entries, mut manifest) = load_fixture();
    let large_id = "x".repeat(1_100_000);
    let optional_bytes = Vec::new();
    entries.push(("optional/large.bin".to_owned(), optional_bytes.clone()));
    manifest.components.push(aeria_hsp::HspComponentDescriptor {
        id: large_id,
        kind: "future".to_owned(),
        format_version: 1,
        required: false,
        path: "optional/large.bin".to_owned(),
        size: 0,
        sha256: hash_bytes(&optional_bytes),
    });
    manifest.package_id = compute_package_id(&manifest).expect("package hash");
    let package = directory.path().join("oversized-manifest.hsp");
    write_manifest_archive(&package, entries, &manifest);

    assert!(matches!(
        SourcePackage::open(&package, directory.path()),
        Err(aeria_hsp::HspError::Manifest { .. })
    ));
}

#[test]
fn component_declared_size_mismatches_are_rejected_before_decompression() {
    let directory = tempdir().expect("test directory");

    let (entries, mut guidance_manifest) = load_fixture();
    let guidance = guidance_manifest
        .components
        .iter_mut()
        .find(|component| component.kind == "sourceGuidance")
        .expect("guidance component");
    guidance.size += 1;
    guidance_manifest.package_id = compute_package_id(&guidance_manifest).expect("package hash");
    let guidance_package = directory.path().join("guidance-size.hsp");
    write_manifest_archive(&guidance_package, entries.clone(), &guidance_manifest);
    assert!(matches!(
        SourcePackage::open(&guidance_package, directory.path()),
        Err(aeria_hsp::HspError::Component { .. })
    ));

    let (mut optional_entries, mut optional_manifest) = load_fixture();
    let optional_bytes = vec![0x5a; 2 * 1024 * 1024];
    optional_entries.push(("optional/large.bin".to_owned(), optional_bytes.clone()));
    optional_manifest
        .components
        .push(aeria_hsp::HspComponentDescriptor {
            id: "future-large".to_owned(),
            kind: "future".to_owned(),
            format_version: 1,
            required: false,
            path: "optional/large.bin".to_owned(),
            size: i64::try_from(optional_bytes.len()).expect("size"),
            sha256: hash_bytes(&optional_bytes),
        });
    optional_manifest.package_id = compute_package_id(&optional_manifest).expect("package hash");
    let optional_package = directory.path().join("large-optional.hsp");
    write_manifest_archive(&optional_package, optional_entries, &optional_manifest);
    assert!(SourcePackage::open(&optional_package, directory.path()).is_ok());

    let (mut mismatched_entries, mut mismatched_manifest) = load_fixture();
    let optional_bytes = b"optional".to_vec();
    mismatched_entries.push(("optional/mismatch.bin".to_owned(), optional_bytes.clone()));
    mismatched_manifest
        .components
        .push(aeria_hsp::HspComponentDescriptor {
            id: "future-mismatch".to_owned(),
            kind: "future".to_owned(),
            format_version: 1,
            required: false,
            path: "optional/mismatch.bin".to_owned(),
            size: i64::try_from(optional_bytes.len()).expect("size") + 1,
            sha256: hash_bytes(&optional_bytes),
        });
    mismatched_manifest.package_id =
        compute_package_id(&mismatched_manifest).expect("package hash");
    let mismatch_package = directory.path().join("optional-size.hsp");
    write_manifest_archive(&mismatch_package, mismatched_entries, &mismatched_manifest);
    assert!(matches!(
        SourcePackage::open(&mismatch_package, directory.path()),
        Err(aeria_hsp::HspError::Component { .. })
    ));
}

#[test]
fn oversized_source_entry_does_not_leave_a_partial_cache_file() {
    let directory = tempdir().expect("test directory");
    let (entries, mut manifest) = load_fixture();
    let source = manifest
        .components
        .iter_mut()
        .find(|component| component.kind == "sourceHxs")
        .expect("source component");
    source.size -= 1;
    manifest.package_id = compute_package_id(&manifest).expect("package hash");
    let cache_path = directory
        .path()
        .join("cache")
        .join("hxs")
        .join(
            manifest
                .source
                .snapshot_id
                .strip_prefix("sha256:")
                .expect("snapshot hash"),
        )
        .join("source.hxs");
    let package = directory.path().join("source-size.hsp");
    write_manifest_archive(&package, entries, &manifest);

    assert!(matches!(
        SourcePackage::open(&package, directory.path().join("cache")),
        Err(aeria_hsp::HspError::Component { .. })
    ));
    assert!(!cache_path.exists());
}

#[test]
fn source_and_guidance_relationships_are_validated() {
    let directory = tempdir().expect("test directory");
    let entries = fixture_entries();

    let mut manifest = load_manifest();
    manifest.source.content_id = format!("sha256:{}", "0".repeat(64));
    manifest.package_id = compute_package_id(&manifest).expect("package hash");
    let manifest_mismatch = directory.path().join("manifest-mismatch.hsp");
    write_manifest_archive(&manifest_mismatch, entries.clone(), &manifest);
    assert!(matches!(
        SourcePackage::open(&manifest_mismatch, directory.path()),
        Err(aeria_hsp::HspError::Relationship { .. })
    ));

    let (mut guidance_entries, mut guidance_manifest) = load_fixture();
    let mut guidance = load_guidance();
    guidance.source.content_id = format!("sha256:{}", "0".repeat(64));
    refresh_guidance(&mut guidance_entries, &mut guidance_manifest, &mut guidance);
    guidance_manifest.package_id = compute_package_id(&guidance_manifest).expect("package hash");
    let guidance_mismatch = directory.path().join("guidance-mismatch.hsp");
    write_manifest_archive(&guidance_mismatch, guidance_entries, &guidance_manifest);
    assert!(matches!(
        SourcePackage::open(&guidance_mismatch, directory.path()),
        Err(aeria_hsp::HspError::Relationship { .. })
    ));

    let (mut schema_entries, mut schema_manifest) = load_fixture();
    let mut schema_guidance = load_guidance();
    schema_guidance.sheets[0].schema_hash = format!("sha256:{}", "f".repeat(64));
    refresh_guidance(
        &mut schema_entries,
        &mut schema_manifest,
        &mut schema_guidance,
    );
    schema_manifest.package_id = compute_package_id(&schema_manifest).expect("package hash");
    let schema_mismatch = directory.path().join("schema-mismatch.hsp");
    write_manifest_archive(&schema_mismatch, schema_entries, &schema_manifest);
    assert!(matches!(
        SourcePackage::open(&schema_mismatch, directory.path()),
        Err(aeria_hsp::HspError::Relationship { .. })
    ));

    let (mut evidence_entries, mut evidence_manifest) = load_fixture();
    let mut evidence_guidance = load_guidance();
    evidence_guidance
        .evidence_inputs
        .iter_mut()
        .find(|input| input.language == evidence_guidance.source.language)
        .expect("source evidence")
        .evidence_id = format!("sha256:{}", "e".repeat(64));
    refresh_guidance(
        &mut evidence_entries,
        &mut evidence_manifest,
        &mut evidence_guidance,
    );
    evidence_manifest.package_id = compute_package_id(&evidence_manifest).expect("package hash");
    let evidence_mismatch = directory.path().join("evidence-mismatch.hsp");
    write_manifest_archive(&evidence_mismatch, evidence_entries, &evidence_manifest);
    assert!(matches!(
        SourcePackage::open(&evidence_mismatch, directory.path()),
        Err(aeria_hsp::HspError::Relationship { .. })
    ));
}

#[test]
fn guidance_bundle_and_occurrence_validation_are_fail_closed() {
    let directory = tempdir().expect("test directory");
    let (mut entries, mut manifest) = load_fixture();
    let mut guidance = load_guidance();
    guidance.sheets[0].translatable.push(GuidanceOccurrence {
        row_id: 42,
        subrow_id: 0,
        column_index: 0,
    });
    refresh_guidance(&mut entries, &mut manifest, &mut guidance);
    manifest.package_id = compute_package_id(&manifest).expect("package hash");
    let duplicate = directory.path().join("duplicate-occurrence.hsp");
    write_manifest_archive(&duplicate, entries.clone(), &manifest);
    assert!(matches!(
        SourcePackage::open(&duplicate, directory.path()),
        Err(aeria_hsp::HspError::Guidance { .. })
    ));

    let mut tampered_entries = entries;
    let mut tampered_manifest = manifest;
    let guidance_entry = tampered_entries
        .iter_mut()
        .find(|(path, _)| path == "guidance/source-guidance.json")
        .expect("guidance entry");
    guidance_entry.1 = String::from_utf8(guidance_entry.1.clone())
        .expect("guidance UTF-8")
        .replace("\"formatVersion\":1", "\"formatVersion\":2")
        .into_bytes();
    let guidance_component = tampered_manifest
        .components
        .iter_mut()
        .find(|component| component.kind == "sourceGuidance")
        .expect("guidance component");
    guidance_component.sha256 = hash_bytes(&guidance_entry.1);
    guidance_component.size = i64::try_from(guidance_entry.1.len()).expect("size");
    tampered_manifest.package_id = compute_package_id(&tampered_manifest).expect("package hash");
    let tampered = directory.path().join("tampered-bundle.hsp");
    write_manifest_archive(&tampered, tampered_entries, &tampered_manifest);
    assert!(matches!(
        SourcePackage::open(&tampered, directory.path()),
        Err(aeria_hsp::HspError::Guidance { .. })
    ));
}

fn fixture_entries() -> Vec<(String, Vec<u8>)> {
    let file = fs::File::open(fixture_path()).expect("fixture archive");
    let mut archive = ZipArchive::new(file).expect("fixture zip");
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).expect("fixture entry");
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).expect("fixture bytes");
            (entry.name().to_owned(), bytes)
        })
        .collect()
}

fn load_fixture() -> (Vec<(String, Vec<u8>)>, HspManifest) {
    (fixture_entries(), load_manifest())
}

fn load_manifest() -> HspManifest {
    let entries = fixture_entries();
    serde_json::from_slice(
        &entries
            .iter()
            .find(|(path, _)| path == "manifest.json")
            .expect("manifest entry")
            .1,
    )
    .expect("manifest JSON")
}

fn load_guidance() -> SourceGuidance {
    let entries = fixture_entries();
    serde_json::from_slice(
        &entries
            .iter()
            .find(|(path, _)| path == "guidance/source-guidance.json")
            .expect("guidance entry")
            .1,
    )
    .expect("guidance JSON")
}

fn refresh_guidance(
    entries: &mut [(String, Vec<u8>)],
    manifest: &mut HspManifest,
    guidance: &mut SourceGuidance,
) {
    guidance.bundle_id = compute_guidance_bundle_id(guidance).expect("bundle hash");
    let mut guidance_bytes = serde_json::to_vec(guidance).expect("guidance JSON");
    guidance_bytes.push(b'\n');
    let entry = entries
        .iter_mut()
        .find(|(path, _)| path == "guidance/source-guidance.json")
        .expect("guidance entry");
    entry.1 = guidance_bytes;
    let component = manifest
        .components
        .iter_mut()
        .find(|component| component.kind == "sourceGuidance")
        .expect("guidance component");
    component.size = i64::try_from(entry.1.len()).expect("guidance size");
    component.sha256 = hash_bytes(&entry.1);
}

fn write_manifest_archive(
    path: &Path,
    mut entries: Vec<(String, Vec<u8>)>,
    manifest: &HspManifest,
) {
    let manifest_json = manifest_bytes(manifest);
    let manifest_entry = entries
        .iter_mut()
        .find(|(entry_path, _)| entry_path == "manifest.json")
        .expect("manifest entry");
    manifest_entry.1 = manifest_json;
    write_archive(path, entries);
}

fn manifest_bytes(manifest: &HspManifest) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(manifest).expect("manifest JSON");
    bytes.push(b'\n');
    bytes
}

fn write_archive(path: &Path, entries: Vec<(String, Vec<u8>)>) {
    let file = fs::File::create(path).expect("archive file");
    let mut archive = ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    for (entry_path, bytes) in entries {
        archive
            .start_file(entry_path, options)
            .expect("archive entry");
        archive.write_all(&bytes).expect("archive bytes");
    }
    archive.finish().expect("archive finish");
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("hex string");
    }
    format!("sha256:{hex}")
}
