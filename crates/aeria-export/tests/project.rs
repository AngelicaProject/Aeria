use aeria_core::{ReviewState, SourceBinding};
use aeria_export::{
    Channel, ContentPolicy, ExportError, PackManifest, Publisher, StringEncoder, collect_project,
    pack_source, write_pack,
};
use aeria_hxs::HxsSnapshot;
use aeria_workspace::Workspace;

#[path = "../../aeria-workspace/tests/support/fixture.rs"]
mod fixture;

use fixture::{raw_hash, write_fixture};

// Stands in for Atlas: plain text encodes to its UTF-8 bytes.
struct FakeEncoder {
    calls: usize,
    reject: Option<&'static str>,
}

impl StringEncoder for FakeEncoder {
    fn encode(&mut self, macros: &[&str]) -> Result<Vec<Result<Vec<u8>, String>>, String> {
        self.calls += 1;
        Ok(macros
            .iter()
            .map(|text| {
                if Some(*text) == self.reject {
                    Err("not round-trip".to_owned())
                } else {
                    Ok(text.as_bytes().to_vec())
                }
            })
            .collect())
    }
}

fn encoder() -> FakeEncoder {
    FakeEncoder {
        calls: 0,
        reject: None,
    }
}

// Row 42 has no raw-value hash in the fixture; row 7 has one.
fn workspace(snapshot: &HxsSnapshot, review_seven: bool) -> Workspace {
    let mut workspace = Workspace::from_verified_snapshot(snapshot, "ru").unwrap();
    workspace
        .create_unit_from_hxs(snapshot, "Synthetic", 42, 0, 0, "Раз")
        .unwrap();
    let seven = workspace
        .create_unit_from_hxs(snapshot, "Synthetic", 7, 0, 0, "Два")
        .unwrap();
    if review_seven {
        workspace
            .update_review_state(seven, ReviewState::Reviewed)
            .unwrap();
    }
    workspace
}

#[test]
fn reviewed_policy_exports_only_reviewed_units_with_a_source_guard() {
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).unwrap();
    let workspace = workspace(&snapshot, true);

    let export = collect_project(
        &workspace,
        &snapshot,
        ContentPolicy::Reviewed,
        &mut encoder(),
    )
    .unwrap();

    assert_eq!(export.report.exported, 1);
    assert_eq!(export.report.skipped_unreviewed, 1);
    assert!(export.report.skipped_without_raw_hash.is_empty());
    let sheet = &export.sheets[0];
    assert_eq!(sheet.name, "Synthetic");
    assert_eq!(sheet.layout.len(), 1);
    assert_eq!(sheet.cells.len(), 1);
    assert_eq!(sheet.cells[0].row_id, 7);
    assert_eq!(sheet.cells[0].text, "Два".as_bytes());
    assert_eq!(sheet.cells[0].source_guard, raw_hash(b"raw")[..8]);
}

#[test]
fn units_without_a_raw_hash_are_reported_not_exported() {
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).unwrap();
    let workspace = workspace(&snapshot, false);

    let export =
        collect_project(&workspace, &snapshot, ContentPolicy::All, &mut encoder()).unwrap();

    assert_eq!(export.report.exported, 1);
    assert_eq!(
        export.report.skipped_without_raw_hash,
        vec![SourceBinding::new("Synthetic", 42, 0, 0)]
    );
}

#[test]
fn collected_project_writes_a_pack_with_the_snapshot_identity() {
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).unwrap();
    let workspace = workspace(&snapshot, false);
    let export =
        collect_project(&workspace, &snapshot, ContentPolicy::All, &mut encoder()).unwrap();

    let source = pack_source(&snapshot);
    assert_eq!(source.language, "en");
    assert_eq!(source.game_version, "test-game");
    let manifest = PackManifest {
        pack_id: "synthetic".to_owned(),
        title: "Synthetic".to_owned(),
        publisher: Publisher {
            name: "Tests".to_owned(),
            url: None,
        },
        license: None,
        sequence: 1,
        version: "1".to_owned(),
        channel: Channel::Testing,
        target_language: "ru".to_owned(),
        source,
        content_policy: ContentPolicy::All,
        project_commit: "0".repeat(40),
        exporter_aeria: "0.1.0".to_owned(),
        exporter_atlas: "0.4.0".to_owned(),
        min_harmonia: "1.0.0".to_owned(),
    };
    let pack = write_pack(&manifest, export.sheets, None, None).unwrap();
    assert_eq!(pack.counts.cells, 1);
    assert_eq!(pack.counts.reviewed_cells, 0);
}

#[test]
fn a_failed_encoding_fails_the_export() {
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).unwrap();
    let workspace = workspace(&snapshot, true);
    let mut encoder = FakeEncoder {
        calls: 0,
        reject: Some("Два"),
    };

    let result = collect_project(&workspace, &snapshot, ContentPolicy::All, &mut encoder);

    assert!(matches!(result, Err(ExportError::Cell { row_id: 7, .. })));
    assert_eq!(encoder.calls, 1);
}

#[test]
fn an_encoder_returning_the_wrong_count_is_rejected() {
    struct Short;
    impl StringEncoder for Short {
        fn encode(&mut self, _: &[&str]) -> Result<Vec<Result<Vec<u8>, String>>, String> {
            Ok(Vec::new())
        }
    }
    let fixture = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).unwrap();
    let workspace = workspace(&snapshot, true);

    let result = collect_project(&workspace, &snapshot, ContentPolicy::All, &mut Short);
    assert!(matches!(result, Err(ExportError::Encoder(_))));
}
