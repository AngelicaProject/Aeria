use std::path::Path;

use aeria_export::{
    CellState, Channel, ContentPolicy, ExportError, FeedDownload, LayoutColumn, PackCell,
    PackManifest, PackSheet, PackSigner, PackSource, Publisher, SheetVariant,
    compress_for_transport, feed_entry, fingerprint, source_guard, write_file_atomically,
    write_pack, write_pack_with_fonts,
};
use aeria_fonts::{FONTS_SECTION_KIND, FontSection, SectionGlyph, SectionSource, SectionTarget};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

type Edit = Box<dyn Fn(&mut PackManifest, &mut Vec<PackSheet>)>;

const FIXTURE: &str = "tests/fixtures/harmonia-interop.hpk";
const FONTS_FIXTURE: &str = "tests/fixtures/harmonia-interop-fonts.hpk";

fn manifest(policy: ContentPolicy) -> PackManifest {
    PackManifest {
        pack_id: "interop-test".to_owned(),
        title: "Interop test pack".to_owned(),
        publisher: Publisher {
            name: "Aeria tests".to_owned(),
            url: None,
        },
        license: Some("CC0-1.0".to_owned()),
        sequence: 7,
        version: "2026.09.25".to_owned(),
        channel: Channel::Stable,
        target_language: "ru".to_owned(),
        source: PackSource {
            language: "en".to_owned(),
            game_version: "2026.08.12.0000.0000".to_owned(),
            content_id: format!("sha256:{}", "1".repeat(64)),
            snapshot_id: format!("sha256:{}", "2".repeat(64)),
        },
        content_policy: policy,
        project_commit: "a".repeat(40),
        exporter_aeria: "0.1.0".to_owned(),
        exporter_atlas: "0.4.0".to_owned(),
        min_harmonia: "1.0.0".to_owned(),
    }
}

// The HXS raw-string hash, as Atlas computes it.
fn raw_hash(raw: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"HARMONIA-HXS-V1-RAW-STRING");
    hasher.update(u32::try_from(raw.len()).unwrap().to_le_bytes());
    hasher.update(raw);
    hasher.finalize().into()
}

fn cell(row_id: u32, subrow_id: u16, column_index: u32, text: &str, source: &str) -> PackCell {
    PackCell {
        row_id,
        subrow_id,
        column_index,
        state: CellState::Reviewed,
        text: text.as_bytes().to_vec(),
        source_guard: source_guard(&raw_hash(source.as_bytes())),
    }
}

// Deliberately unsorted input; the writer owns canonical order.
fn sheets() -> Vec<PackSheet> {
    vec![
        PackSheet {
            name: "quest/000/Test".to_owned(),
            variant: SheetVariant::Subrows,
            layout: vec![LayoutColumn {
                column_index: 1,
                offset: 0,
            }],
            cells: vec![
                cell(3, 1, 1, "Вторая", "Second"),
                cell(3, 0, 1, "Первая", "First"),
            ],
        },
        PackSheet {
            name: "Addon".to_owned(),
            variant: SheetVariant::DefaultRows,
            layout: vec![
                LayoutColumn {
                    column_index: 0,
                    offset: 4,
                },
                LayoutColumn {
                    column_index: 2,
                    offset: 8,
                },
            ],
            cells: vec![
                cell(7, 0, 2, "Привет", "Hi"),
                cell(1, 0, 2, "Мир", "World"),
                cell(1, 0, 0, "Привет", "Hello"),
            ],
        },
        PackSheet {
            name: "Empty".to_owned(),
            variant: SheetVariant::DefaultRows,
            layout: vec![LayoutColumn {
                column_index: 0,
                offset: 0,
            }],
            cells: vec![],
        },
    ]
}

fn signer() -> PackSigner {
    let mut secret = [0u8; 32];
    for (i, byte) in secret.iter_mut().enumerate() {
        *byte = u8::try_from(i + 1).unwrap();
    }
    PackSigner::from_secret_bytes(&secret).unwrap()
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], at: usize) -> usize {
    usize::try_from(u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())).unwrap()
}

fn section(bytes: &[u8], kind: u32) -> &[u8] {
    let count = usize::try_from(u32_at(bytes, 24)).unwrap();
    (0..count)
        .map(|i| 64 + i * 24)
        .find(|at| u32_at(bytes, *at) == kind)
        .map(|at| {
            let offset = u64_at(bytes, at + 8);
            &bytes[offset..offset + u64_at(bytes, at + 16)]
        })
        .expect("section present")
}

#[test]
fn output_is_canonical_and_deterministic() {
    let first = write_pack(&manifest(ContentPolicy::Reviewed), sheets(), None, None).unwrap();
    let mut shuffled = sheets();
    shuffled.reverse();
    for sheet in &mut shuffled {
        sheet.cells.reverse();
    }
    let second = write_pack(&manifest(ContentPolicy::Reviewed), shuffled, None, None).unwrap();
    assert_eq!(first, second);
}

#[test]
fn layout_follows_pack_format_v1() {
    let pack = write_pack(&manifest(ContentPolicy::Reviewed), sheets(), None, None).unwrap();
    let bytes = &pack.bytes;

    assert_eq!(&bytes[0..8], b"AERIAHPK");
    assert_eq!(u16_at(bytes, 8), 1);
    assert_eq!(u32_at(bytes, 12), 64);
    let body_length = u64_at(bytes, 16);
    assert_eq!(body_length, bytes.len(), "unsigned packs end at the digest");
    let digest: [u8; 32] = Sha256::digest(&bytes[..body_length - 32]).into();
    assert_eq!(digest, pack.pack_hash);
    assert_eq!(&bytes[body_length - 32..], &pack.pack_hash);

    // Empty sheets are omitted and names are sorted bytewise.
    assert_eq!(section(bytes, 2), b"Addonquest/000/Test");
    let sheets = section(bytes, 3);
    assert_eq!(sheets.len(), 64);
    assert_eq!(sheets[8], 0);
    assert_eq!(sheets[32 + 8], 1);

    // Addon rows 1 and 7; column index 2 is ordinal 1.
    let rows = section(bytes, 5);
    assert_eq!(
        (u32_at(rows, 0), u16_at(rows, 4), u16_at(rows, 6)),
        (1, 0, 2)
    );
    assert_eq!(
        (u32_at(rows, 16), u16_at(rows, 20), u16_at(rows, 22)),
        (7, 0, 1)
    );
    let cells = section(bytes, 6);
    assert_eq!(u16_at(cells, 0), 0);
    assert_eq!(u16_at(cells, 24), 1);
    assert_eq!(&cells[16..24], &raw_hash(b"Hello")[..8]);

    // "Привет" is stored once.
    let strings = section(bytes, 7);
    assert_eq!(u32_at(cells, 8), u32_at(cells, 48 + 8));
    assert_eq!(pack.counts.strings, 4);
    assert_eq!(strings.split(|b| *b == 0).count() - 1, 4);

    let manifest: serde_json::Value = serde_json::from_slice(section(bytes, 1)).unwrap();
    assert_eq!(manifest["counts"]["cells"], 5);
    assert_eq!(manifest["counts"]["rows"], 4);
    assert_eq!(manifest["release"]["channel"], "stable");
    assert_eq!(manifest["publisher"]["url"], serde_json::Value::Null);
    assert!(section(bytes, 1).ends_with(b"}\n"));
}

#[test]
fn signature_and_endorsement_verify() {
    let previous = PackSigner::from_secret_bytes(&[9u8; 32]).unwrap();
    let signer = signer();
    let endorsement = previous.endorse(&signer.public_key());
    let pack = write_pack(
        &manifest(ContentPolicy::Reviewed),
        sheets(),
        Some(&signer),
        Some(&endorsement),
    )
    .unwrap();

    let block = &pack.bytes[u64_at(&pack.bytes, 16)..];
    assert_eq!(&block[0..8], b"HPKSIG01");
    assert_eq!(u16_at(block, 8), 1);
    let public_key: [u8; 65] = block[12..77].try_into().unwrap();
    assert_eq!(public_key, signer.public_key());
    assert_eq!(fingerprint(&public_key), signer.fingerprint());

    let key = VerifyingKey::from_sec1_bytes(&public_key).unwrap();
    let signature = Signature::from_slice(&block[77..141]).unwrap();
    let message = [b"AERIA-HPK-V1-SIGNATURE".as_slice(), &pack.pack_hash].concat();
    key.verify(&message, &signature).unwrap();

    assert_eq!(block[141], 1);
    let previous_key = VerifyingKey::from_sec1_bytes(&block[142..207]).unwrap();
    let endorsement = Signature::from_slice(&block[207..271]).unwrap();
    let rotation = [b"AERIA-HPK-V1-KEY-ROTATION".as_slice(), &public_key].concat();
    previous_key.verify(&rotation, &endorsement).unwrap();
    assert_eq!(block.len(), 271);
}

#[test]
fn endorsement_must_match_the_signing_key() {
    let previous = PackSigner::from_secret_bytes(&[9u8; 32]).unwrap();
    let other = PackSigner::from_secret_bytes(&[8u8; 32]).unwrap();
    let endorsement = previous.endorse(&other.public_key());
    let result = write_pack(
        &manifest(ContentPolicy::Reviewed),
        sheets(),
        Some(&signer()),
        Some(&endorsement),
    );
    assert!(matches!(result, Err(ExportError::Manifest(_))));
}

#[test]
fn invalid_input_is_rejected() {
    let reviewed = manifest(ContentPolicy::Reviewed);
    let cases: Vec<(&str, Edit)> = vec![
        ("bad pack id", Box::new(|m, _| m.pack_id = "Bad".to_owned())),
        ("zero sequence", Box::new(|m, _| m.sequence = 0)),
        (
            "bad commit",
            Box::new(|m, _| m.project_commit = "xyz".to_owned()),
        ),
        (
            "bad version",
            Box::new(|m, _| m.min_harmonia = "1".to_owned()),
        ),
        (
            "unknown column",
            Box::new(|_, s| s[1].cells[0].column_index = 1),
        ),
        (
            "subrow in default sheet",
            Box::new(|_, s| s[1].cells[0].subrow_id = 1),
        ),
        (
            "nul in text",
            Box::new(|_, s| s[1].cells[0].text = b"a\0b".to_vec()),
        ),
        ("empty text", Box::new(|_, s| s[1].cells[0].text.clear())),
        (
            "too long",
            Box::new(|_, s| s[1].cells[0].text = vec![b'a'; 65_536]),
        ),
        (
            "duplicate cell",
            Box::new(|_, s| {
                let copy = s[1].cells[1].clone();
                s[1].cells.push(copy);
            }),
        ),
        ("unsorted layout", Box::new(|_, s| s[1].layout.reverse())),
        (
            "duplicate sheet",
            Box::new(|_, s| {
                let copy = s[1].clone();
                s.push(copy);
            }),
        ),
        (
            "unreviewed under reviewed policy",
            Box::new(|_, s| s[1].cells[0].state = CellState::Unreviewed),
        ),
    ];

    for (name, edit) in cases {
        let mut manifest = reviewed.clone();
        let mut input = sheets();
        edit(&mut manifest, &mut input);
        assert!(
            write_pack(&manifest, input, None, None).is_err(),
            "{name} was accepted"
        );
    }
}

#[test]
fn all_policy_marks_unreviewed_cells() {
    let mut input = sheets();
    input[1].cells[0].state = CellState::Unreviewed;
    let pack = write_pack(&manifest(ContentPolicy::All), input, None, None).unwrap();
    assert_eq!(pack.counts.cells, 5);
    assert_eq!(pack.counts.reviewed_cells, 4);
    let cells = section(&pack.bytes, 6);
    assert_eq!(cells.chunks(24).filter(|c| c[2] == 2).count(), 1);
}

#[test]
fn transport_compression_round_trips() {
    let pack = write_pack(
        &manifest(ContentPolicy::Reviewed),
        sheets(),
        Some(&signer()),
        None,
    )
    .unwrap();
    let compressed = compress_for_transport(&pack.bytes).unwrap();
    let mut decoded = Vec::new();
    brotli::BrotliDecompress(&mut compressed.as_slice(), &mut decoded).unwrap();
    assert_eq!(decoded, pack.bytes);
}

#[test]
fn feed_entry_describes_the_release() {
    let manifest = manifest(ContentPolicy::Reviewed);
    let pack = write_pack(&manifest, sheets(), None, None).unwrap();
    let entry = feed_entry(
        &manifest,
        &pack,
        &FeedDownload {
            url: "https://github.com/o/r/releases/download/harmonia/7/interop-test-7.hpk.br"
                .to_owned(),
            brotli: true,
            size: 10,
            sha256: [0xab; 32],
            unpacked_size: u64::try_from(pack.bytes.len()).unwrap(),
        },
        Some("Notes"),
    );
    let json: serde_json::Value = serde_json::from_slice(&entry).unwrap();
    assert_eq!(json["sequence"], 7);
    assert_eq!(json["packHash"], pack.pack_hash_text());
    assert_eq!(json["source"]["gameVersion"], "2026.08.12.0000.0000");
    assert_eq!(json["download"]["encoding"], "br");
    assert_eq!(json["download"]["sha256"], "ab".repeat(32));
    assert_eq!(json["contentPolicy"], "reviewed");
}

#[test]
fn atomic_write_replaces_the_file_under_a_non_ascii_path() {
    let dir = tempfile::Builder::new()
        .prefix("Анна Иванова ")
        .tempdir()
        .unwrap();
    let path = dir.path().join("пак перевода.hpk");
    write_file_atomically(&path, b"old").unwrap();
    write_file_atomically(&path, b"new").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

// The committed fixture is read by Harmonia's tests, so any byte change is a
// format change. Regenerate with AERIA_UPDATE_FIXTURES=1 only on purpose.
#[test]
fn interop_fixture_is_stable() {
    let mut input = sheets();
    input[1].cells[0].state = CellState::Unreviewed;
    let pack = write_pack(&manifest(ContentPolicy::All), input, Some(&signer()), None).unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    if std::env::var_os("AERIA_UPDATE_FIXTURES").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &pack.bytes).unwrap();
    }
    let committed = std::fs::read(&path)
        .expect("fixture exists; run with AERIA_UPDATE_FIXTURES=1 to create it");
    assert_eq!(committed, pack.bytes);
}

fn fonts() -> FontSection {
    let glyph = |character, width: u8, height: u8, offset_y, advance| SectionGlyph {
        character,
        width,
        height,
        offset_y,
        advance,
        bitmap: (0..u16::from(width) * u16::from(height))
            .map(|i| u8::try_from(i * 37 % 256).unwrap())
            .collect(),
    };
    FontSection {
        sources: vec![SectionSource {
            family: "Test Sans".to_owned(),
            copyright: "Copyright 2026 Test".to_owned(),
            license: "OFL-1.1".to_owned(),
            license_text: "Test license
"
            .to_owned(),
            sha256: [0x5a; 32],
        }],
        targets: vec![
            SectionTarget {
                font: "Jupiter".to_owned(),
                size: "16".to_owned(),
                line_height: 26,
                ascent: 19,
                source: 0,
                glyphs: vec![glyph('Б', 9, 13, 6, 10), glyph('Ж', 14, 13, 6, 13)],
            },
            SectionTarget {
                font: "TrumpGothic".to_owned(),
                size: "184".to_owned(),
                line_height: 24,
                ascent: 19,
                source: 0,
                glyphs: vec![glyph('Д', 6, 16, 5, 7)],
            },
        ],
    }
}

#[test]
fn fonts_are_an_optional_minor_1_section() {
    let manifest = manifest(ContentPolicy::Reviewed);
    let plain = write_pack(&manifest, sheets(), None, None).unwrap();
    assert_eq!(
        write_pack_with_fonts(&manifest, sheets(), None, None, None).unwrap(),
        plain
    );
    assert_eq!(u16_at(&plain.bytes, 10), 0);

    let pack = write_pack_with_fonts(&manifest, sheets(), Some(&fonts()), None, None).unwrap();
    assert_eq!(u16_at(&pack.bytes, 10), 1);
    assert_eq!(u32_at(&pack.bytes, 24), 8);
    assert_eq!(u32_at(&pack.bytes, 64 + 7 * 24), FONTS_SECTION_KIND);
    assert_eq!(
        FontSection::decode(section(&pack.bytes, FONTS_SECTION_KIND)).unwrap(),
        fonts()
    );
    assert_eq!(section(&pack.bytes, 7), section(&plain.bytes, 7));

    let mut invalid = fonts();
    invalid.targets[0].glyphs.reverse();
    assert!(matches!(
        write_pack_with_fonts(&manifest, sheets(), Some(&invalid), None, None),
        Err(ExportError::Fonts(_))
    ));
}

#[test]
fn fonts_interop_fixture_is_stable() {
    let pack = write_pack_with_fonts(
        &manifest(ContentPolicy::Reviewed),
        sheets(),
        Some(&fonts()),
        Some(&signer()),
        None,
    )
    .unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FONTS_FIXTURE);
    if std::env::var_os("AERIA_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &pack.bytes).unwrap();
    }
    let committed = std::fs::read(&path)
        .expect("fixture exists; run with AERIA_UPDATE_FIXTURES=1 to create it");
    assert_eq!(committed, pack.bytes);
}
