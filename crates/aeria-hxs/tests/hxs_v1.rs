use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use aeria_hxs::{HxsError, HxsSnapshot, SheetVariant};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

const SYNTHETIC_SCHEMA: &str = include_str!("fixtures/synthetic_v1.sql");
const APPLICATION_ID: i64 = 0x4841_544c;

#[derive(Clone)]
struct ColumnSpec {
    index: u32,
    offset: u32,
    type_code: u32,
}

#[derive(Clone)]
struct RowSpec {
    row_id: u32,
    subrow_id: u16,
    technical: Vec<(u32, u32, Vec<u8>)>,
    macro_text: String,
    raw_value: Option<Vec<u8>>,
}

struct BuiltRow {
    spec: RowSpec,
    technical_payload: Vec<u8>,
    row_hash: [u8; 32],
    technical_hash: [u8; 32],
    string_hash: [u8; 32],
    macro_hash: [u8; 32],
    raw_hash: Option<[u8; 32]>,
}

struct BuiltSheet {
    id: i64,
    name: String,
    variant: u32,
    effective_language: String,
    columns: Vec<ColumnSpec>,
    rows: Vec<BuiltRow>,
    schema_hash: [u8; 32],
    technical_hash: [u8; 32],
    string_hash: [u8; 32],
    content_hash: [u8; 32],
}

struct TempFixture {
    path: PathBuf,
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn opens_and_exposes_verified_metadata_sheets_rows_and_cells() {
    let (fixture, expected) = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).expect("synthetic HXS should verify");

    let metadata = snapshot.metadata();
    assert_eq!(metadata.format_version, 1);
    assert_eq!(metadata.game_version, "2026.09.17");
    assert_eq!(metadata.source_language, "en");
    assert_eq!(metadata.content_id, expected.content_id);
    assert_eq!(metadata.snapshot_id, expected.snapshot_id);
    assert_eq!(metadata.producer.extractor_version, "0.1.0");
    assert_eq!(metadata.producer.lumina_version, "7.7.0");
    assert_eq!(metadata.counts.sheets, 2);
    assert_eq!(metadata.counts.rows, 4);
    assert_eq!(metadata.counts.string_cells, 4);

    let sheets = snapshot.sheets();
    assert_eq!(
        sheets
            .iter()
            .map(|sheet| sheet.name.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Beta"]
    );
    let alpha = snapshot.sheet("Alpha").expect("Alpha metadata");
    assert_eq!(alpha.variant, SheetVariant::DefaultRows);
    assert_eq!(alpha.effective_language, "en");
    assert_eq!(alpha.row_count, 2);
    assert_eq!(alpha.columns.len(), 3);
    assert_eq!(
        alpha.hashes.schema.to_hex(),
        hex(&expected.alpha.schema_hash)
    );
    assert_eq!(
        alpha.hashes.technical.to_hex(),
        hex(&expected.alpha.technical_hash)
    );
    assert_eq!(
        alpha.hashes.strings.to_hex(),
        hex(&expected.alpha.string_hash)
    );
    assert_eq!(
        alpha.hashes.content.to_hex(),
        hex(&expected.alpha.content_hash)
    );
    assert_eq!(snapshot.sheet("missing"), None);

    let first_page = snapshot.page_rows("Alpha", 0, 1).expect("first page");
    assert_eq!(first_page.rows.len(), 1);
    assert_eq!(first_page.rows[0].row_id, 2);
    assert_eq!(first_page.next_offset, Some(1));
    assert_eq!(first_page.total_rows, 2);
    assert_eq!(
        first_page.rows[0].technical_payload,
        expected.alpha.rows[0].technical_payload
    );
    assert_eq!(
        first_page.rows[0].hashes.technical.to_hex(),
        hex(&expected.alpha.rows[0].technical_hash)
    );
    let second_page = snapshot.page_rows("Alpha", 1, 1).expect("second page");
    assert_eq!(
        second_page
            .rows
            .iter()
            .map(|row| row.row_id)
            .collect::<Vec<_>>(),
        [3]
    );
    assert_eq!(second_page.next_offset, None);
    assert!(
        snapshot
            .page_rows("Alpha", 9, 2)
            .expect("empty page")
            .rows
            .is_empty()
    );

    let beta_page = snapshot.page_rows("Beta", 0, 10).expect("subrow page");
    assert_eq!(
        beta_page
            .rows
            .iter()
            .map(|row| (row.row_id, row.subrow_id))
            .collect::<Vec<_>>(),
        [(5, 0), (5, 1)]
    );
    assert_eq!(
        snapshot
            .row("Beta", 5, 1)
            .expect("row lookup")
            .expect("row exists")
            .row_id,
        5
    );

    let hello = snapshot
        .string_cell("Alpha", 2, 0, 0)
        .expect("cell lookup")
        .expect("cell exists");
    assert_eq!(hello.macro_text, "Hello");
    assert_eq!(hello.raw_value, Some(b"hello".to_vec()));
    assert_eq!(
        hello.hashes.macro_text.to_hex(),
        hex(&expected.alpha.rows[0].macro_hash)
    );
    assert_eq!(
        hello.hashes.raw_value.as_ref().map(ToString::to_string),
        Some(hex(&expected.alpha.rows[0].raw_hash.expect("raw hash")))
    );
    let world = snapshot
        .string_cell("Alpha", 3, 0, 0)
        .expect("cell lookup")
        .expect("cell exists");
    assert_eq!(world.macro_text, "World");
    assert_eq!(world.raw_value, None);
    let beta_subrow = snapshot
        .string_cell("Beta", 5, 1, 0)
        .expect("subrow cell lookup")
        .expect("subrow cell exists");
    assert_eq!(beta_subrow.raw_value, Some(vec![2]));
}

#[test]
fn rejects_unsupported_versions_and_required_schema_changes() {
    let (fixture, _) = write_fixture();
    let connection =
        Connection::open(&fixture.path).expect("fixture opens read-write for tampering");
    connection
        .execute_batch("PRAGMA application_id = 123;")
        .expect("tamper application id");
    drop(connection);
    assert!(matches!(
        HxsSnapshot::open(&fixture.path),
        Err(HxsError::UnsupportedApplicationId { .. })
    ));

    let (fixture, _) = write_fixture();
    let connection =
        Connection::open(&fixture.path).expect("fixture opens read-write for tampering");
    connection
        .execute_batch("PRAGMA user_version = 2;")
        .expect("tamper version");
    drop(connection);
    assert!(matches!(
        HxsSnapshot::open(&fixture.path),
        Err(HxsError::UnsupportedFormatVersion { .. })
    ));

    let (fixture, _) = write_fixture();
    let connection =
        Connection::open(&fixture.path).expect("fixture opens read-write for tampering");
    connection
        .execute_batch("ALTER TABLE columns RENAME TO columns_old;")
        .expect("tamper schema");
    drop(connection);
    assert!(matches!(
        HxsSnapshot::open(&fixture.path),
        Err(HxsError::InvalidSchema { .. })
    ));

    let (fixture, _) = write_fixture();
    let connection =
        Connection::open(&fixture.path).expect("fixture opens read-write for tampering");
    connection
        .execute(
            "UPDATE string_cells SET macro_hash = zeroblob(32) WHERE row_id = 2",
            [],
        )
        .expect("tamper hash");
    drop(connection);
    assert!(matches!(
        HxsSnapshot::open(&fixture.path),
        Err(HxsError::InvalidData { .. })
    ));
}

#[test]
fn rejects_invalid_requests_and_unknown_sheets() {
    let (fixture, _) = write_fixture();
    let snapshot = HxsSnapshot::open(&fixture.path).expect("synthetic HXS should verify");
    assert!(matches!(
        snapshot.page_rows("Alpha", 0, 0),
        Err(HxsError::InvalidRequest { .. })
    ));
    assert!(matches!(
        snapshot.page_rows("Missing", 0, 1),
        Err(HxsError::SheetNotFound { .. })
    ));
    assert!(matches!(
        snapshot.row("Missing", 0, 0),
        Err(HxsError::SheetNotFound { .. })
    ));
}

struct Expected {
    content_id: String,
    snapshot_id: String,
    alpha: ExpectedSheet,
}

struct ExpectedSheet {
    schema_hash: [u8; 32],
    technical_hash: [u8; 32],
    string_hash: [u8; 32],
    content_hash: [u8; 32],
    rows: Vec<ExpectedRow>,
}

struct ExpectedRow {
    technical_payload: Vec<u8>,
    technical_hash: [u8; 32],
    macro_hash: [u8; 32],
    raw_hash: Option<[u8; 32]>,
}

#[allow(clippy::too_many_lines)]
fn write_fixture() -> (TempFixture, Expected) {
    let path = std::env::temp_dir().join(format!(
        "aeria-hxs-{}-{}.hxs",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let connection = Connection::open(&path).expect("create fixture database");
    connection
        .execute_batch(SYNTHETIC_SCHEMA)
        .expect("create fixture schema");
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = 1; PRAGMA foreign_keys = ON;"
        ))
        .expect("set HXS identity");

    let alpha = build_sheet(
        1,
        "Alpha",
        0,
        "en",
        vec![
            ColumnSpec {
                index: 0,
                offset: 0,
                type_code: 1,
            },
            ColumnSpec {
                index: 1,
                offset: 16,
                type_code: 14,
            },
            ColumnSpec {
                index: 2,
                offset: 20,
                type_code: 2,
            },
        ],
        vec![
            RowSpec {
                row_id: 2,
                subrow_id: 0,
                technical: vec![(1, 14, vec![0x2a, 0, 0, 0]), (2, 2, vec![1])],
                macro_text: "Hello".into(),
                raw_value: Some(b"hello".to_vec()),
            },
            RowSpec {
                row_id: 3,
                subrow_id: 0,
                technical: vec![(1, 14, vec![0xf9, 0xff, 0xff, 0xff]), (2, 2, vec![0])],
                macro_text: "World".into(),
                raw_value: None,
            },
        ],
    );
    let beta = build_sheet(
        2,
        "Beta",
        1,
        "none",
        vec![
            ColumnSpec {
                index: 0,
                offset: 0,
                type_code: 1,
            },
            ColumnSpec {
                index: 1,
                offset: 4,
                type_code: 13,
            },
        ],
        vec![
            RowSpec {
                row_id: 5,
                subrow_id: 0,
                technical: vec![(1, 13, vec![7, 0])],
                macro_text: "Beta 0".into(),
                raw_value: Some(vec![0, 1]),
            },
            RowSpec {
                row_id: 5,
                subrow_id: 1,
                technical: vec![(1, 13, vec![8, 0])],
                macro_text: "Beta 1".into(),
                raw_value: Some(vec![2]),
            },
        ],
    );
    insert_sheet(&connection, &alpha);
    insert_sheet(&connection, &beta);
    let content_id = content_id("en", &[&alpha, &beta]);
    let snapshot_id = snapshot_id("2026.09.17", "en", &content_id);
    connection
        .execute(
            "INSERT INTO hxs_meta (id, format_version, game_version, language, scope, content_id, snapshot_id, extractor_version, lumina_version, sheet_count, row_count, string_cell_count) VALUES (1, 1, '2026.09.17', 'en', 'full', ?1, ?2, '0.1.0', '7.7.0', 2, 4, 4)",
            params![content_id, snapshot_id],
        )
        .expect("insert metadata");
    drop(connection);

    let expected = Expected {
        content_id,
        snapshot_id,
        alpha: ExpectedSheet {
            schema_hash: alpha.schema_hash,
            technical_hash: alpha.technical_hash,
            string_hash: alpha.string_hash,
            content_hash: alpha.content_hash,
            rows: alpha
                .rows
                .iter()
                .map(|row| ExpectedRow {
                    technical_payload: row.technical_payload.clone(),
                    technical_hash: row.technical_hash,
                    macro_hash: row.macro_hash,
                    raw_hash: row.raw_hash,
                })
                .collect(),
        },
    };
    (TempFixture { path }, expected)
}

fn build_sheet(
    id: i64,
    name: &str,
    variant: u32,
    effective_language: &str,
    columns: Vec<ColumnSpec>,
    rows: Vec<RowSpec>,
) -> BuiltSheet {
    let rows = rows
        .into_iter()
        .map(|spec| {
            let technical_payload = technical_payload(&spec.technical);
            let macro_hash = macro_hash(&spec.macro_text);
            let raw_digest = spec.raw_value.as_deref().map(raw_hash);
            let string_part = string_part(0, &macro_hash, raw_digest.as_ref());
            let technical_hash = row_technical(name, &spec, &spec.technical);
            let string_hash = row_strings(name, &spec, &string_part);
            let combined_hash = row_hash(name, &spec, &technical_hash, &string_hash);
            BuiltRow {
                spec,
                technical_payload,
                row_hash: combined_hash,
                technical_hash,
                string_hash,
                macro_hash,
                raw_hash: raw_digest,
            }
        })
        .collect::<Vec<_>>();
    let schema_hash = schema_hash(name, variant, &columns);
    let technical_hash = sheet_hash("HARMONIA-HXS-V1-SHEET-TECHNICAL", name, &rows, |row| {
        &row.technical_hash
    });
    let string_hash = sheet_hash("HARMONIA-HXS-V1-SHEET-STRINGS", name, &rows, |row| {
        &row.string_hash
    });
    let content_hash = hash_parts(&[
        b"HARMONIA-HXS-V1-SHEET".to_vec(),
        utf8(name),
        u32_bytes(variant),
        schema_hash.to_vec(),
        technical_hash.to_vec(),
        string_hash.to_vec(),
    ]);
    BuiltSheet {
        id,
        name: name.into(),
        variant,
        effective_language: effective_language.into(),
        columns,
        rows,
        schema_hash,
        technical_hash,
        string_hash,
        content_hash,
    }
}

fn insert_sheet(connection: &Connection, sheet: &BuiltSheet) {
    connection
        .execute(
            "INSERT INTO sheets (id, name, variant, effective_language, column_count, row_count, schema_hash, technical_hash, string_hash, content_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![sheet.id, sheet.name, sheet.variant, sheet.effective_language, i64::try_from(sheet.columns.len()).expect("fixture column count fits SQLite"), i64::try_from(sheet.rows.len()).expect("fixture row count fits SQLite"), sheet.schema_hash.as_slice(), sheet.technical_hash.as_slice(), sheet.string_hash.as_slice(), sheet.content_hash.as_slice()],
        )
        .expect("insert sheet");
    for column in &sheet.columns {
        connection
            .execute(
                "INSERT INTO columns (sheet_id, column_index, offset, type) VALUES (?1, ?2, ?3, ?4)",
                params![sheet.id, column.index, column.offset, column.type_code],
            )
            .expect("insert column");
    }
    for row in &sheet.rows {
        connection
            .execute(
                "INSERT INTO rows (sheet_id, row_id, subrow_id, technical_payload, row_hash, technical_hash, string_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![sheet.id, row.spec.row_id, row.spec.subrow_id, row.technical_payload, row.row_hash.as_slice(), row.technical_hash.as_slice(), row.string_hash.as_slice()],
            )
            .expect("insert row");
        connection
            .execute(
                "INSERT INTO string_cells (sheet_id, row_id, subrow_id, column_index, macro_text, raw_value, macro_hash, raw_hash) VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7)",
                params![sheet.id, row.spec.row_id, row.spec.subrow_id, row.spec.macro_text, row.spec.raw_value, row.macro_hash.as_slice(), row.raw_hash.as_ref().map(<[u8; 32]>::as_slice)],
            )
            .expect("insert String cell");
    }
}

fn technical_payload(cells: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (index, type_code, value) in cells {
        bytes.extend(u32_bytes(*index));
        bytes.extend(u32_bytes(*type_code));
        bytes.extend(u32_bytes(
            u32::try_from(value.len()).expect("fixture value fits HXS framing"),
        ));
        bytes.extend(value);
    }
    bytes
}

fn schema_hash(name: &str, variant: u32, columns: &[ColumnSpec]) -> [u8; 32] {
    let mut parts = vec![
        b"HARMONIA-HXS-V1-SCHEMA".to_vec(),
        utf8(name),
        u32_bytes(variant),
    ];
    for column in columns {
        parts.extend([
            u32_bytes(column.index),
            u32_bytes(column.offset),
            u32_bytes(column.type_code),
        ]);
    }
    hash_parts(&parts)
}

fn row_technical(name: &str, row: &RowSpec, cells: &[(u32, u32, Vec<u8>)]) -> [u8; 32] {
    let mut parts = vec![
        b"HARMONIA-HXS-V1-ROW-TECHNICAL".to_vec(),
        identity(name, row),
    ];
    for (index, type_code, value) in cells {
        parts.push(u32_bytes(*index));
        parts.push(u32_bytes(*type_code));
        parts.push(framed(value));
    }
    hash_parts(&parts)
}

fn row_strings(name: &str, row: &RowSpec, string_part: &[u8]) -> [u8; 32] {
    hash_parts(&[
        b"HARMONIA-HXS-V1-ROW-STRINGS".to_vec(),
        identity(name, row),
        string_part.to_vec(),
    ])
}

fn row_hash(name: &str, row: &RowSpec, technical: &[u8; 32], strings: &[u8; 32]) -> [u8; 32] {
    hash_parts(&[
        b"HARMONIA-HXS-V1-ROW".to_vec(),
        identity(name, row),
        technical.to_vec(),
        strings.to_vec(),
    ])
}

fn sheet_hash(
    domain: &str,
    name: &str,
    rows: &[BuiltRow],
    select: impl Fn(&BuiltRow) -> &[u8; 32],
) -> [u8; 32] {
    let mut parts = vec![domain.as_bytes().to_vec(), utf8(name)];
    for row in rows {
        parts.push(u32_bytes(row.spec.row_id));
        parts.push(u32_bytes(u32::from(row.spec.subrow_id)));
        parts.push(select(row).to_vec());
    }
    hash_parts(&parts)
}

fn content_id(language: &str, sheets: &[&BuiltSheet]) -> String {
    let mut parts = vec![b"HARMONIA-HXS-CONTENT-v1".to_vec(), utf8(language)];
    for sheet in sheets {
        parts.extend([
            utf8(&sheet.name),
            utf8(&sheet.effective_language),
            sheet.schema_hash.to_vec(),
            sheet.content_hash.to_vec(),
        ]);
    }
    format!("sha256:{}", hex(&hash_parts(&parts)))
}

fn snapshot_id(game_version: &str, language: &str, content_id: &str) -> String {
    format!(
        "sha256:{}",
        hex(&hash_parts(&[
            b"HARMONIA-HXS-SNAPSHOT-v1".to_vec(),
            utf8(game_version),
            utf8(language),
            utf8(content_id)
        ]))
    )
}

fn string_part(index: u32, macro_hash: &[u8; 32], raw_hash: Option<&[u8; 32]>) -> Vec<u8> {
    let mut bytes = u32_bytes(index);
    bytes.extend(macro_hash);
    bytes.push(u8::from(raw_hash.is_some()));
    if let Some(raw_hash) = raw_hash {
        bytes.extend(raw_hash);
    }
    bytes
}

fn macro_hash(value: &str) -> [u8; 32] {
    hash_parts(&[b"HARMONIA-HXS-V1-MACRO".to_vec(), utf8(value)])
}
fn raw_hash(value: &[u8]) -> [u8; 32] {
    hash_parts(&[b"HARMONIA-HXS-V1-RAW-STRING".to_vec(), framed(value)])
}
fn identity(name: &str, row: &RowSpec) -> Vec<u8> {
    [
        utf8(name),
        u32_bytes(row.row_id),
        u32_bytes(u32::from(row.subrow_id)),
    ]
    .concat()
}
fn utf8(value: &str) -> Vec<u8> {
    [
        u32_bytes(u32::try_from(value.len()).expect("fixture text fits HXS framing")),
        value.as_bytes().to_vec(),
    ]
    .concat()
}
fn framed(value: &[u8]) -> Vec<u8> {
    [
        u32_bytes(u32::try_from(value.len()).expect("fixture bytes fit HXS framing")),
        value.to_vec(),
    ]
    .concat()
}
fn u32_bytes(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}
fn hash_parts(parts: &[Vec<u8>]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}
fn hex(bytes: &[u8; 32]) -> String {
    let mut result = String::with_capacity(64);
    for byte in bytes {
        write!(&mut result, "{byte:02x}").expect("writing to a String cannot fail");
    }
    result
}
