//! Synthetic HXS/HSP fixtures shared by the workspace integration tests.

#![allow(dead_code)]

use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use aeria_hsp::{
    GuidanceEvidenceInput, GuidanceOccurrence, GuidanceSheet, GuidanceSheetStatus,
    HspComponentDescriptor, HspManifest, HspSourceIdentity, SourceGuidance,
    compute_guidance_bundle_id, compute_package_id, compute_source_evidence_id,
};
use aeria_hxs::HxsSnapshot;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use zip::ZipWriter;

pub const SYNTHETIC_SCHEMA: &str =
    include_str!("../../../aeria-hxs/tests/fixtures/synthetic_v1.sql");
pub const APPLICATION_ID: i64 = 0x4841_544c;

pub struct Fixture {
    pub _directory: TempDir,
    pub path: PathBuf,
    pub package_path: PathBuf,
    pub one_macro_hash: [u8; 32],
    pub one_row_technical_hash: [u8; 32],
    pub two_macro_hash: [u8; 32],
    pub two_raw_hash: [u8; 32],
}

pub struct ProjectionFixture {
    pub _directory: TempDir,
    pub path: PathBuf,
    pub package_path: PathBuf,
}

pub struct ProjectionString {
    pub column_index: u32,
    pub macro_text: String,
    pub macro_hash: [u8; 32],
}

pub struct ProjectionRow {
    pub row_id: u32,
    pub row_hash: [u8; 32],
    pub technical_hash: [u8; 32],
    pub string_hash: [u8; 32],
    pub strings: Vec<ProjectionString>,
}

pub type StringHashSpec = (u32, [u8; 32], Option<[u8; 32]>);

#[allow(clippy::too_many_lines)]
pub fn write_fixture() -> Fixture {
    write_fixture_with("en", "test-game")
}

#[allow(clippy::too_many_lines)]
pub fn write_fixture_with(source_language: &str, game_version: &str) -> Fixture {
    write_fixture_with_text(source_language, game_version, "one")
}

#[allow(clippy::too_many_lines)]
pub fn write_fixture_with_text(
    source_language: &str,
    game_version: &str,
    first_text: &str,
) -> Fixture {
    let directory = tempfile::tempdir().expect("create fixture directory");
    let path = directory.path().join("fixture.hxs");
    let connection = Connection::open(&path).expect("create fixture database");
    connection
        .execute_batch(SYNTHETIC_SCHEMA)
        .expect("create fixture schema");
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = 1; PRAGMA foreign_keys = ON;"
        ))
        .expect("set HXS identity");

    let one_macro_hash = macro_hash(first_text);
    let two_macro_hash = macro_hash("two");
    let two_raw_hash = raw_hash(b"raw");
    let one_row_technical_hash = row_technical_hash("Synthetic", 42, 0);
    let two_row_technical_hash = row_technical_hash("Synthetic", 7, 0);
    let one_row_string_hash = row_string_hash("Synthetic", 42, 0, 0, &one_macro_hash, None);
    let two_row_string_hash =
        row_string_hash("Synthetic", 7, 0, 0, &two_macro_hash, Some(&two_raw_hash));
    let one_row_hash = row_hash(
        "Synthetic",
        42,
        0,
        &one_row_technical_hash,
        &one_row_string_hash,
    );
    let second_row_hash = row_hash(
        "Synthetic",
        7,
        0,
        &two_row_technical_hash,
        &two_row_string_hash,
    );
    let schema_hash = schema_hash("Synthetic");
    let sheet_technical_hash = sheet_rows_hash(
        "HARMONIA-HXS-V1-SHEET-TECHNICAL",
        "Synthetic",
        &[
            (7, 0, two_row_technical_hash),
            (42, 0, one_row_technical_hash),
        ],
    );
    let sheet_string_hash = sheet_rows_hash(
        "HARMONIA-HXS-V1-SHEET-STRINGS",
        "Synthetic",
        &[(7, 0, two_row_string_hash), (42, 0, one_row_string_hash)],
    );
    let content_hash = digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-SHEET");
        framed_text(hasher, "Synthetic");
        hasher.update(0_u32.to_le_bytes());
        hasher.update(schema_hash);
        hasher.update(sheet_technical_hash);
        hasher.update(sheet_string_hash);
    });
    let content_id = format!(
        "sha256:{}",
        hex(&digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-CONTENT-v1");
            framed_text(hasher, source_language);
            framed_text(hasher, "Synthetic");
            framed_text(hasher, source_language);
            hasher.update(schema_hash);
            hasher.update(content_hash);
        }))
    );
    let snapshot_id = format!(
        "sha256:{}",
        hex(&digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-SNAPSHOT-v1");
            framed_text(hasher, game_version);
            framed_text(hasher, source_language);
            framed_text(hasher, &content_id);
        }))
    );

    connection
        .execute(
            "INSERT INTO sheets (id, name, variant, effective_language, column_count, row_count, schema_hash, technical_hash, string_hash, content_hash) VALUES (1, 'Synthetic', 0, ?1, 1, 2, ?2, ?3, ?4, ?5)",
            params![source_language, schema_hash.as_slice(), sheet_technical_hash.as_slice(), sheet_string_hash.as_slice(), content_hash.as_slice()],
        )
        .expect("insert sheet");
    connection
        .execute(
            "INSERT INTO columns (sheet_id, column_index, offset, type) VALUES (1, 0, 0, 1)",
            [],
        )
        .expect("insert String column");
    for (row_id, row_hash, row_technical_hash, row_string_hash) in [
        (
            42_u32,
            one_row_hash,
            one_row_technical_hash,
            one_row_string_hash,
        ),
        (
            7_u32,
            second_row_hash,
            two_row_technical_hash,
            two_row_string_hash,
        ),
    ] {
        connection
            .execute(
                "INSERT INTO rows (sheet_id, row_id, subrow_id, technical_payload, row_hash, technical_hash, string_hash) VALUES (1, ?1, 0, ?2, ?3, ?4, ?5)",
            params![row_id, Vec::<u8>::new(), row_hash.as_slice(), row_technical_hash.as_slice(), row_string_hash.as_slice()],
            )
            .expect("insert row");
    }
    connection
        .execute(
            "INSERT INTO string_cells (sheet_id, row_id, subrow_id, column_index, macro_text, raw_value, macro_hash, raw_hash) VALUES (1, 42, 0, 0, ?1, NULL, ?2, NULL)",
            params![first_text, one_macro_hash.as_slice()],
        )
        .expect("insert first String cell");
    connection
        .execute(
            "INSERT INTO string_cells (sheet_id, row_id, subrow_id, column_index, macro_text, raw_value, macro_hash, raw_hash) VALUES (1, 7, 0, 0, 'two', ?1, ?2, ?3)",
            params![b"raw".as_slice(), two_macro_hash.as_slice(), two_raw_hash.as_slice()],
        )
        .expect("insert second String cell");
    connection
        .execute(
            "INSERT INTO hxs_meta (id, format_version, game_version, language, scope, content_id, snapshot_id, extractor_version, lumina_version, sheet_count, row_count, string_cell_count) VALUES (1, 1, ?1, ?2, 'full', ?3, ?4, 'test', '7.7.0', 1, 2, 2)",
            params![game_version, source_language, content_id, snapshot_id],
        )
        .expect("insert metadata");

    let package_path = write_hsp_package(&path, |_, _, _, _, _| true);
    Fixture {
        _directory: directory,
        path,
        package_path,
        one_macro_hash,
        one_row_technical_hash,
        two_macro_hash,
        two_raw_hash,
    }
}

pub fn write_projection_fixture() -> ProjectionFixture {
    write_projection_fixture_with(
        "projection",
        &[
            (1, ["Context field", "Greetings and welcome", "", ""]),
            (2, ["Empty context", "", "", ""]),
            (3, ["", "", "", ""]),
            (
                4,
                [
                    "fire shard",
                    "fire shards",
                    "A tiny crystalline manifestation",
                    "Fire Shard",
                ],
            ),
            (5, ["Blocked only", "", "", ""]),
        ],
        |sheet, row_id, _, column_index, _| {
            sheet == "Projection" && matches!((row_id, column_index), (1 | 2, 1) | (4, 0..=3))
        },
    )
}

/// Writes a four-String-column `Projection` sheet and its HSP.
#[allow(clippy::too_many_lines)]
pub fn write_projection_fixture_with(
    game_version: &str,
    definitions: &[(u32, [&str; 4])],
    allow: impl Fn(&str, u32, u16, u32, &str) -> bool,
) -> ProjectionFixture {
    let directory = tempfile::tempdir().expect("create projection fixture directory");
    let path = directory.path().join("projection.hxs");
    let connection = Connection::open(&path).expect("create projection fixture database");
    connection
        .execute_batch(SYNTHETIC_SCHEMA)
        .expect("create projection fixture schema");
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = 1; PRAGMA foreign_keys = ON;"
        ))
        .expect("set HXS identity");

    let definitions: Vec<(u32, Vec<String>)> = definitions
        .iter()
        .map(|(row_id, texts)| {
            (
                *row_id,
                texts.iter().map(|text| (*text).to_owned()).collect(),
            )
        })
        .collect();
    let rows = definitions
        .into_iter()
        .map(|(row_id, texts)| {
            let string_hashes = texts
                .iter()
                .map(|text| macro_hash(text))
                .collect::<Vec<_>>();
            let string_hash = row_strings_hash(
                "Projection",
                row_id,
                0,
                &(0..4)
                    .map(|column| (column, string_hashes[column as usize], None))
                    .collect::<Vec<_>>(),
            );
            let technical_hash = row_technical_hash("Projection", row_id, 0);
            let row_hash = row_hash("Projection", row_id, 0, &technical_hash, &string_hash);
            ProjectionRow {
                row_id,
                row_hash,
                technical_hash,
                string_hash,
                strings: texts
                    .into_iter()
                    .enumerate()
                    .map(|(column_index, macro_text)| ProjectionString {
                        column_index: u32::try_from(column_index).expect("column fits"),
                        macro_hash: macro_hash(&macro_text),
                        macro_text,
                    })
                    .collect(),
            }
        })
        .collect::<Vec<_>>();

    let schema_hash =
        schema_hash_for_columns("Projection", &[(0, 0, 1), (1, 4, 1), (2, 8, 1), (3, 12, 1)]);
    let sheet_technical_hash = sheet_rows_hash(
        "HARMONIA-HXS-V1-SHEET-TECHNICAL",
        "Projection",
        &rows
            .iter()
            .map(|row| (row.row_id, 0, row.technical_hash))
            .collect::<Vec<_>>(),
    );
    let sheet_string_hash = sheet_rows_hash(
        "HARMONIA-HXS-V1-SHEET-STRINGS",
        "Projection",
        &rows
            .iter()
            .map(|row| (row.row_id, 0, row.string_hash))
            .collect::<Vec<_>>(),
    );
    let content_hash = digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-SHEET");
        framed_text(hasher, "Projection");
        hasher.update(0_u32.to_le_bytes());
        hasher.update(schema_hash);
        hasher.update(sheet_technical_hash);
        hasher.update(sheet_string_hash);
    });
    let content_id = format!(
        "sha256:{}",
        hex(&digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-CONTENT-v1");
            framed_text(hasher, "en");
            framed_text(hasher, "Projection");
            framed_text(hasher, "en");
            hasher.update(schema_hash);
            hasher.update(content_hash);
        }))
    );
    let snapshot_id = format!(
        "sha256:{}",
        hex(&digest(|hasher| {
            hasher.update(b"HARMONIA-HXS-SNAPSHOT-v1");
            framed_text(hasher, game_version);
            framed_text(hasher, "en");
            framed_text(hasher, &content_id);
        }))
    );

    connection
        .execute(
            "INSERT INTO sheets (id, name, variant, effective_language, column_count, row_count, schema_hash, technical_hash, string_hash, content_hash) VALUES (1, 'Projection', 0, 'en', 4, ?5, ?1, ?2, ?3, ?4)",
            params![schema_hash.as_slice(), sheet_technical_hash.as_slice(), sheet_string_hash.as_slice(), content_hash.as_slice(), i64::try_from(rows.len()).expect("row count")],
        )
        .expect("insert projection sheet");
    for (column_index, offset) in [(0_u32, 0_u32), (1, 4), (2, 8), (3, 12)] {
        connection
            .execute(
                "INSERT INTO columns (sheet_id, column_index, offset, type) VALUES (1, ?1, ?2, 1)",
                params![column_index, offset],
            )
            .expect("insert projection column");
    }
    for row in &rows {
        connection
            .execute(
                "INSERT INTO rows (sheet_id, row_id, subrow_id, technical_payload, row_hash, technical_hash, string_hash) VALUES (1, ?1, 0, ?2, ?3, ?4, ?5)",
                params![row.row_id, Vec::<u8>::new(), row.row_hash.as_slice(), row.technical_hash.as_slice(), row.string_hash.as_slice()],
            )
            .expect("insert projection row");
        for string in &row.strings {
            connection
                .execute(
                    "INSERT INTO string_cells (sheet_id, row_id, subrow_id, column_index, macro_text, raw_value, macro_hash, raw_hash) VALUES (1, ?1, 0, ?2, ?3, NULL, ?4, NULL)",
                    params![row.row_id, string.column_index, string.macro_text, string.macro_hash.as_slice()],
                )
                .expect("insert projection String cell");
        }
    }
    connection
        .execute(
            "INSERT INTO hxs_meta (id, format_version, game_version, language, scope, content_id, snapshot_id, extractor_version, lumina_version, sheet_count, row_count, string_cell_count) VALUES (1, 1, ?3, 'en', 'full', ?1, ?2, 'test', '7.7.0', 1, ?4, ?5)",
            params![content_id, snapshot_id, game_version, i64::try_from(rows.len()).expect("row count"), i64::try_from(rows.len() * 4).expect("cell count")],
        )
        .expect("insert projection metadata");

    let package_path = write_hsp_package(&path, allow);
    ProjectionFixture {
        _directory: directory,
        path,
        package_path,
    }
}

pub fn write_hsp_package(
    source_path: &Path,
    allow: impl Fn(&str, u32, u16, u32, &str) -> bool,
) -> PathBuf {
    let snapshot = HxsSnapshot::open(source_path).expect("source fixture verifies");
    let metadata = snapshot.metadata();
    let guidance = build_guidance(&snapshot, &allow);
    let mut guidance_bytes = serde_json::to_vec(&guidance).expect("guidance JSON");
    guidance_bytes.push(b'\n');
    let source_bytes = fs::read(source_path).expect("source bytes");
    let source_component = HspComponentDescriptor {
        id: "source".to_owned(),
        kind: "sourceHxs".to_owned(),
        format_version: 1,
        required: true,
        path: "source/source.hxs".to_owned(),
        size: i64::try_from(source_bytes.len()).expect("source size"),
        sha256: hash_bytes(&source_bytes),
    };
    let guidance_component = HspComponentDescriptor {
        id: "guidance".to_owned(),
        kind: "sourceGuidance".to_owned(),
        format_version: 1,
        required: true,
        path: "guidance/source-guidance.json".to_owned(),
        size: i64::try_from(guidance_bytes.len()).expect("guidance size"),
        sha256: hash_bytes(&guidance_bytes),
    };
    let manifest_without_id = HspManifest {
        format_version: 1,
        package_id: String::new(),
        game_version: metadata.game_version,
        scope: metadata.scope,
        source: HspSourceIdentity {
            language: metadata.source_language,
            content_id: metadata.content_id,
            snapshot_id: metadata.snapshot_id,
        },
        components: vec![guidance_component, source_component],
    };
    let manifest = HspManifest {
        package_id: compute_package_id(&manifest_without_id).expect("package hash"),
        ..manifest_without_id
    };
    let package_path = source_path.with_extension("hsp");
    let file = fs::File::create(&package_path).expect("package file");
    let mut archive = ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    archive
        .start_file("manifest.json", options)
        .expect("manifest entry");
    let mut manifest_bytes = serde_json::to_vec(&manifest).expect("manifest JSON");
    manifest_bytes.push(b'\n');
    archive.write_all(&manifest_bytes).expect("manifest bytes");
    archive
        .start_file("guidance/source-guidance.json", options)
        .expect("guidance entry");
    archive.write_all(&guidance_bytes).expect("guidance bytes");
    archive
        .start_file("source/source.hxs", options)
        .expect("source entry");
    archive.write_all(&source_bytes).expect("source bytes");
    archive.finish().expect("package archive");
    package_path
}

pub fn build_guidance(
    snapshot: &HxsSnapshot,
    allow: &impl Fn(&str, u32, u16, u32, &str) -> bool,
) -> SourceGuidance {
    let metadata = snapshot.metadata();
    let source_evidence_id = compute_source_evidence_id(snapshot).expect("source evidence");
    let comparison_language = if metadata.source_language == "en" {
        "ja"
    } else {
        "en"
    };
    let mut evidence_inputs = vec![
        GuidanceEvidenceInput {
            language: metadata.source_language.clone(),
            evidence_id: source_evidence_id,
        },
        GuidanceEvidenceInput {
            language: comparison_language.to_owned(),
            evidence_id: format!("sha256:{}", "1".repeat(64)),
        },
    ];
    evidence_inputs.sort_by(|left, right| left.language.cmp(&right.language));

    let mut sheets = snapshot.sheets();
    sheets.sort_by(|left, right| left.name.cmp(&right.name));
    let guidance_sheets = sheets
        .iter()
        .map(|sheet| {
            let mut occurrences = Vec::new();
            let mut after = None;
            loop {
                let page = snapshot
                    .page_string_rows(
                        &sheet.name,
                        after.as_ref(),
                        aeria_hxs::MAX_STRING_ROW_PAGE_SIZE,
                    )
                    .expect("String rows");
                for row in &page.rows {
                    for occurrence in &row.occurrences {
                        let coordinate = &occurrence.fingerprint.coordinate;
                        if allow(
                            &coordinate.sheet_name,
                            coordinate.row_id,
                            coordinate.subrow_id,
                            coordinate.column_index,
                            &occurrence.macro_text,
                        ) {
                            occurrences.push(GuidanceOccurrence {
                                row_id: coordinate.row_id,
                                subrow_id: coordinate.subrow_id,
                                column_index: coordinate.column_index,
                            });
                        }
                    }
                }
                let Some(next) = page.next_after else {
                    break;
                };
                after = Some(next);
            }
            GuidanceSheet {
                name: sheet.name.clone(),
                schema_hash: format!("sha256:{}", sheet.hashes.schema.to_hex()),
                status: GuidanceSheetStatus::Compatible,
                translatable: occurrences,
                incompatibility_reasons: Vec::new(),
            }
        })
        .collect::<Vec<_>>();

    let guidance_without_id = SourceGuidance {
        format_version: 1,
        game_version: metadata.game_version.clone(),
        scope: metadata.scope.clone(),
        bundle_id: String::new(),
        source: aeria_hsp::GuidanceSourceIdentity {
            language: metadata.source_language.clone(),
            content_id: metadata.content_id.clone(),
            snapshot_id: metadata.snapshot_id.clone(),
        },
        evidence_inputs,
        sheets: guidance_sheets,
    };
    SourceGuidance {
        bundle_id: compute_guidance_bundle_id(&guidance_without_id).expect("guidance hash"),
        ..guidance_without_id
    }
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    format!("sha256:{}", hex(&digest))
}

pub fn macro_hash(value: &str) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-MACRO");
        framed_text(hasher, value);
    })
}

pub fn raw_hash(value: &[u8]) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-RAW-STRING");
        framed_bytes(hasher, value);
    })
}

pub fn schema_hash(sheet_name: &str) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-SCHEMA");
        framed_text(hasher, sheet_name);
        hasher.update(0_u32.to_le_bytes());
        hasher.update(0_u32.to_le_bytes());
        hasher.update(0_u32.to_le_bytes());
        hasher.update(1_u32.to_le_bytes());
    })
}

pub fn schema_hash_for_columns(sheet_name: &str, columns: &[(u32, u32, u32)]) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-SCHEMA");
        framed_text(hasher, sheet_name);
        hasher.update(0_u32.to_le_bytes());
        for (index, offset, type_code) in columns {
            hasher.update(index.to_le_bytes());
            hasher.update(offset.to_le_bytes());
            hasher.update(type_code.to_le_bytes());
        }
    })
}

pub fn row_technical_hash(sheet_name: &str, row_id: u32, subrow_id: u16) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-ROW-TECHNICAL");
        row_identity(hasher, sheet_name, row_id, subrow_id);
    })
}

pub fn row_string_hash(
    sheet_name: &str,
    row_id: u32,
    subrow_id: u16,
    column_index: u32,
    macro_hash: &[u8; 32],
    raw_hash: Option<&[u8; 32]>,
) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-ROW-STRINGS");
        row_identity(hasher, sheet_name, row_id, subrow_id);
        hasher.update(column_index.to_le_bytes());
        hasher.update(macro_hash);
        hasher.update([u8::from(raw_hash.is_some())]);
        if let Some(raw_hash) = raw_hash {
            hasher.update(raw_hash);
        }
    })
}

pub fn row_strings_hash(
    sheet_name: &str,
    row_id: u32,
    subrow_id: u16,
    cells: &[StringHashSpec],
) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-ROW-STRINGS");
        row_identity(hasher, sheet_name, row_id, subrow_id);
        for (column_index, macro_hash, raw_hash) in cells {
            hasher.update(column_index.to_le_bytes());
            hasher.update(macro_hash);
            hasher.update([u8::from(raw_hash.is_some())]);
            if let Some(raw_hash) = raw_hash {
                hasher.update(raw_hash);
            }
        }
    })
}

pub fn row_hash(
    sheet_name: &str,
    row_id: u32,
    subrow_id: u16,
    technical_hash: &[u8; 32],
    string_hash: &[u8; 32],
) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(b"HARMONIA-HXS-V1-ROW");
        row_identity(hasher, sheet_name, row_id, subrow_id);
        hasher.update(technical_hash);
        hasher.update(string_hash);
    })
}

pub fn sheet_rows_hash(domain: &str, sheet_name: &str, rows: &[(u32, u16, [u8; 32])]) -> [u8; 32] {
    digest(|hasher| {
        hasher.update(domain.as_bytes());
        framed_text(hasher, sheet_name);
        for (row_id, subrow_id, row_hash) in rows {
            hasher.update(row_id.to_le_bytes());
            hasher.update(u32::from(*subrow_id).to_le_bytes());
            hasher.update(row_hash);
        }
    })
}

pub fn row_identity(hasher: &mut Sha256, sheet_name: &str, row_id: u32, subrow_id: u16) {
    framed_text(hasher, sheet_name);
    hasher.update(row_id.to_le_bytes());
    hasher.update(u32::from(subrow_id).to_le_bytes());
}

pub fn framed_text(hasher: &mut Sha256, value: &str) {
    hasher.update(
        u32::try_from(value.len())
            .expect("fixture text fits framing")
            .to_le_bytes(),
    );
    hasher.update(value.as_bytes());
}

pub fn framed_bytes(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(
        u32::try_from(value.len())
            .expect("fixture bytes fit framing")
            .to_le_bytes(),
    );
    hasher.update(value);
}

pub fn digest(update: impl FnOnce(&mut Sha256)) -> [u8; 32] {
    let mut hasher = Sha256::new();
    update(&mut hasher);
    hasher.finalize().into()
}

pub fn hex(bytes: &[u8; 32]) -> String {
    let mut result = String::with_capacity(64);
    for byte in bytes {
        write!(&mut result, "{byte:02x}").expect("writing to a String cannot fail");
    }
    result
}
