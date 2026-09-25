use std::collections::HashMap;
use std::collections::hash_map::Entry;

use aeria_fonts::{FONTS_SECTION_KIND, FontSection};
use sha2::{Digest, Sha256};

use crate::error::ExportError;
use crate::manifest::{ContentPolicy, CountsJson, PackManifest};
use crate::signing::{KeyEndorsement, PackSigner, SIGNATURE_DOMAIN};

const MAGIC: &[u8; 8] = b"AERIAHPK";
const SIGNATURE_MAGIC: &[u8; 8] = b"HPKSIG01";
const FORMAT_MAJOR: u16 = 1;
const FORMAT_MINOR: u16 = 0;
/// Format minor that introduced the optional `FONTS` section.
const FORMAT_MINOR_FONTS: u16 = 1;
const HEADER_SIZE: usize = 64;
const SECTION_ENTRY_SIZE: usize = 24;
const DIGEST_SIZE: usize = 32;
const MAX_STRING_LENGTH: usize = 65_535;
const MAX_PACK_BYTES: usize = 1 << 30;
const SIGNATURE_ALGORITHM_ECDSA_P256: u16 = 1;

const KIND_MANIFEST: u32 = 1;
const KIND_NAMES: u32 = 2;
const KIND_SHEETS: u32 = 3;
const KIND_LAYOUT: u32 = 4;
const KIND_ROWS: u32 = 5;
const KIND_CELLS: u32 = 6;
const KIND_STRINGS: u32 = 7;

/// HXS sheet variant, with the same codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SheetVariant {
    DefaultRows,
    Subrows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellState {
    Reviewed,
    Unreviewed,
}

/// One String column of the source sheet (HXS `columns.index`, `columns.offset`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayoutColumn {
    pub column_index: u32,
    pub offset: u32,
}

/// One translation. `text` is the encoded `SeString` without a terminator;
/// `source_guard` comes from [`source_guard`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackCell {
    pub row_id: u32,
    pub subrow_id: u16,
    pub column_index: u32,
    pub state: CellState,
    pub text: Vec<u8>,
    pub source_guard: [u8; 8],
}

/// A sheet with every String column of the source sheet in `layout`,
/// whether or not it has translations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackSheet {
    pub name: String,
    pub variant: SheetVariant,
    pub layout: Vec<LayoutColumn>,
    pub cells: Vec<PackCell>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackCounts {
    pub sheets: u64,
    pub rows: u64,
    pub cells: u64,
    pub reviewed_cells: u64,
    pub strings: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltPack {
    pub bytes: Vec<u8>,
    pub pack_hash: [u8; 32],
    pub counts: PackCounts,
}

impl BuiltPack {
    /// `sha256:<hex>` form used by the feed and install records.
    #[must_use]
    pub fn pack_hash_text(&self) -> String {
        format!("sha256:{}", crate::hex(&self.pack_hash))
    }
}

/// The source guard stored with a cell: the first 8 bytes of the HXS
/// `raw_hash` of the source string the translation was made for.
#[must_use]
pub fn source_guard(raw_value_hash: &[u8; 32]) -> [u8; 8] {
    let mut guard = [0u8; 8];
    guard.copy_from_slice(&raw_value_hash[..8]);
    guard
}

/// Validates the input and writes a Pack Format v1 file.
///
/// Sheets and cells may come in any order; the output is canonical. Sheets
/// without cells are omitted. A signer appends the signature block; an
/// endorsement requires a signer and must endorse that signer's key.
///
/// # Errors
/// Returns an [`ExportError`] describing the first invalid manifest field,
/// sheet or cell, or a format limit that would be exceeded.
///
/// # Panics
/// Never for validated input; internal size conversions are checked first.
pub fn write_pack(
    manifest: &PackManifest,
    sheets: Vec<PackSheet>,
    signer: Option<&PackSigner>,
    endorsement: Option<&KeyEndorsement>,
) -> Result<BuiltPack, ExportError> {
    write_pack_with_fonts(manifest, sheets, None, signer, endorsement)
}

/// [`write_pack`] with an optional `FONTS` section. A pack with fonts is
/// written as format minor 1; one without is identical to [`write_pack`].
///
/// # Errors
/// As [`write_pack`], and [`ExportError::Fonts`] for an invalid section.
pub fn write_pack_with_fonts(
    manifest: &PackManifest,
    sheets: Vec<PackSheet>,
    fonts: Option<&FontSection>,
    signer: Option<&PackSigner>,
    endorsement: Option<&KeyEndorsement>,
) -> Result<BuiltPack, ExportError> {
    manifest.validate()?;
    let fonts = fonts.map(FontSection::encode).transpose()?;
    if let Some(endorsement) = endorsement {
        let Some(signer) = signer else {
            return Err(ExportError::Manifest(
                "a key endorsement requires a signing key".to_owned(),
            ));
        };
        if !endorsement.is_valid_for(&signer.public_key()) {
            return Err(ExportError::Manifest(
                "the key endorsement does not endorse the signing key".to_owned(),
            ));
        }
    }

    let mut sheets: Vec<PackSheet> = sheets.into_iter().filter(|s| !s.cells.is_empty()).collect();
    sheets.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));

    let mut sections = Sections::default();
    let mut counts = PackCounts {
        sheets: len_u64(sheets.len()),
        rows: 0,
        cells: 0,
        reviewed_cells: 0,
        strings: 0,
    };
    let mut strings: HashMap<Vec<u8>, u32> = HashMap::new();

    for pair in sheets.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(ExportError::Sheet {
                sheet: pair[1].name.clone(),
                reason: "duplicate sheet name".to_owned(),
            });
        }
    }
    for sheet in &sheets {
        write_sheet(manifest, sheet, &mut sections, &mut counts, &mut strings)?;
    }
    counts.strings = len_u64(strings.len());

    let manifest_bytes = manifest.to_json(CountsJson {
        sheets: counts.sheets,
        rows: counts.rows,
        cells: counts.cells,
        reviewed_cells: counts.reviewed_cells,
        strings: counts.strings,
    });

    let mut table = vec![
        (KIND_MANIFEST, manifest_bytes.as_slice()),
        (KIND_NAMES, sections.names.as_slice()),
        (KIND_SHEETS, sections.sheets.as_slice()),
        (KIND_LAYOUT, sections.layout.as_slice()),
        (KIND_ROWS, sections.rows.as_slice()),
        (KIND_CELLS, sections.cells.as_slice()),
        (KIND_STRINGS, sections.strings.as_slice()),
    ];
    let minor = match &fonts {
        Some(fonts) => {
            table.push((FONTS_SECTION_KIND, fonts.as_slice()));
            FORMAT_MINOR_FONTS
        }
        None => FORMAT_MINOR,
    };
    let bytes = assemble(&table, minor, signer, endorsement)?;
    let pack_hash = digest_of(&bytes);
    Ok(BuiltPack {
        bytes,
        pack_hash,
        counts,
    })
}

#[derive(Default)]
struct Sections {
    names: Vec<u8>,
    sheets: Vec<u8>,
    layout: Vec<u8>,
    rows: Vec<u8>,
    cells: Vec<u8>,
    strings: Vec<u8>,
    layout_count: u64,
}

type KeyedCell<'a> = ((u32, u16, u16), &'a PackCell);

fn write_sheet(
    manifest: &PackManifest,
    sheet: &PackSheet,
    sections: &mut Sections,
    counts: &mut PackCounts,
    strings: &mut HashMap<Vec<u8>, u32>,
) -> Result<(), ExportError> {
    let keyed = canonical_cells(manifest, sheet)?;
    let variant = match sheet.variant {
        SheetVariant::DefaultRows => 0u8,
        SheetVariant::Subrows => 1u8,
    };
    let row_start = counts.rows;
    let mut row_total = 0u64;
    let mut index = 0;
    while index < keyed.len() {
        let (row_id, subrow_id, _) = keyed[index].0;
        let end = keyed[index..]
            .iter()
            .position(|((r, s, _), _)| (*r, *s) != (row_id, subrow_id))
            .map_or(keyed.len(), |offset| index + offset);
        write_row(&keyed[index..end], sections, counts, strings)?;
        row_total += 1;
        index = end;
    }
    counts.rows += row_total;

    put_u32(
        &mut sections.sheets,
        to_u32(len_u64(sections.names.len()), "NAMES size")?,
    );
    put_u32(
        &mut sections.sheets,
        to_u32(len_u64(sheet.name.len()), "sheet name")?,
    );
    sections.sheets.push(variant);
    sections.sheets.extend_from_slice(&[0, 0, 0]);
    put_u32(
        &mut sections.sheets,
        to_u32(sections.layout_count, "layout count")?,
    );
    put_u32(
        &mut sections.sheets,
        to_u32(len_u64(sheet.layout.len()), "layout count")?,
    );
    put_u32(&mut sections.sheets, to_u32(row_start, "row count")?);
    put_u32(&mut sections.sheets, to_u32(row_total, "row count")?);
    put_u32(&mut sections.sheets, 0);
    sections.names.extend_from_slice(sheet.name.as_bytes());

    for column in &sheet.layout {
        put_u32(&mut sections.layout, column.column_index);
        put_u32(&mut sections.layout, column.offset);
    }
    sections.layout_count += len_u64(sheet.layout.len());
    Ok(())
}

fn write_row(
    cells: &[KeyedCell<'_>],
    sections: &mut Sections,
    counts: &mut PackCounts,
    strings: &mut HashMap<Vec<u8>, u32>,
) -> Result<(), ExportError> {
    let (row_id, subrow_id, _) = cells[0].0;
    let cell_count = u16::try_from(cells.len())
        .map_err(|_| ExportError::Limit("a row has more than 65535 cells".to_owned()))?;
    put_u32(&mut sections.rows, row_id);
    put_u16(&mut sections.rows, subrow_id);
    put_u16(&mut sections.rows, cell_count);
    put_u32(&mut sections.rows, to_u32(counts.cells, "cell count")?);
    put_u32(&mut sections.rows, 0);

    for ((_, _, ordinal), cell) in cells {
        let offset = match strings.entry(cell.text.clone()) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let offset = to_u32(len_u64(sections.strings.len()), "STRINGS size")?;
                sections.strings.extend_from_slice(&cell.text);
                sections.strings.push(0);
                *entry.insert(offset)
            }
        };
        put_u16(&mut sections.cells, *ordinal);
        sections.cells.push(match cell.state {
            CellState::Reviewed => 1,
            CellState::Unreviewed => 2,
        });
        sections.cells.push(0);
        put_u32(
            &mut sections.cells,
            to_u32(len_u64(cell.text.len()), "string length")?,
        );
        put_u32(&mut sections.cells, offset);
        put_u32(&mut sections.cells, 0);
        sections.cells.extend_from_slice(&cell.source_guard);
        counts.cells += 1;
        if cell.state == CellState::Reviewed {
            counts.reviewed_cells += 1;
        }
    }
    Ok(())
}

// Validates a sheet and returns its cells keyed and sorted by
// (row, subrow, ordinal).
fn canonical_cells<'a>(
    manifest: &PackManifest,
    sheet: &'a PackSheet,
) -> Result<Vec<KeyedCell<'a>>, ExportError> {
    let sheet_error = |reason: &str| ExportError::Sheet {
        sheet: sheet.name.clone(),
        reason: reason.to_owned(),
    };
    if sheet.name.is_empty() {
        return Err(sheet_error("empty sheet name"));
    }
    if sheet.layout.is_empty() {
        return Err(sheet_error("a sheet needs at least one String column"));
    }
    for pair in sheet.layout.windows(2) {
        if pair[0].column_index >= pair[1].column_index {
            return Err(sheet_error("layout columns must be strictly ascending"));
        }
    }
    if sheet
        .layout
        .iter()
        .any(|c| c.column_index > u32::from(u16::MAX) || c.offset > u32::from(u16::MAX))
    {
        return Err(sheet_error("layout column index or offset exceeds 65535"));
    }
    // Strictly ascending indices up to 65535 bound the layout to 65536 entries.
    let ordinals: HashMap<u32, u16> = sheet
        .layout
        .iter()
        .zip(0..=u16::MAX)
        .map(|(column, ordinal)| (column.column_index, ordinal))
        .collect();

    let mut keyed = Vec::with_capacity(sheet.cells.len());
    for cell in &sheet.cells {
        let cell_error = |reason: &str| ExportError::Cell {
            sheet: sheet.name.clone(),
            row_id: cell.row_id,
            subrow_id: cell.subrow_id,
            column_index: cell.column_index,
            reason: reason.to_owned(),
        };
        let Some(&ordinal) = ordinals.get(&cell.column_index) else {
            return Err(cell_error(
                "column is not a String column of the sheet layout",
            ));
        };
        if sheet.variant == SheetVariant::DefaultRows && cell.subrow_id != 0 {
            return Err(cell_error("a default-variant sheet has no subrows"));
        }
        if cell.text.is_empty() || cell.text.len() > MAX_STRING_LENGTH {
            return Err(cell_error("encoded string must be 1..=65535 bytes"));
        }
        if cell.text.contains(&0) {
            return Err(cell_error("encoded string contains a NUL byte"));
        }
        if manifest.content_policy == ContentPolicy::Reviewed && cell.state != CellState::Reviewed {
            return Err(cell_error("unreviewed cell in a reviewed-only pack"));
        }
        keyed.push(((cell.row_id, cell.subrow_id, ordinal), cell));
    }
    keyed.sort_by_key(|(key, _)| *key);
    for pair in keyed.windows(2) {
        if pair[0].0 == pair[1].0 {
            let cell = pair[1].1;
            return Err(ExportError::Cell {
                sheet: sheet.name.clone(),
                row_id: cell.row_id,
                subrow_id: cell.subrow_id,
                column_index: cell.column_index,
                reason: "duplicate cell".to_owned(),
            });
        }
    }
    Ok(keyed)
}

fn assemble(
    sections: &[(u32, &[u8])],
    minor: u16,
    signer: Option<&PackSigner>,
    endorsement: Option<&KeyEndorsement>,
) -> Result<Vec<u8>, ExportError> {
    let table_end = HEADER_SIZE + sections.len() * SECTION_ENTRY_SIZE;
    let mut bytes = vec![0u8; table_end];
    let mut table = Vec::with_capacity(sections.len());
    for (kind, data) in sections {
        pad8(&mut bytes);
        table.push((*kind, bytes.len(), data.len()));
        bytes.extend_from_slice(data);
    }
    pad8(&mut bytes);
    let body_length = bytes.len() + DIGEST_SIZE;
    if body_length > MAX_PACK_BYTES {
        return Err(ExportError::Limit("pack is larger than 1 GiB".to_owned()));
    }

    bytes[0..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&FORMAT_MAJOR.to_le_bytes());
    bytes[10..12].copy_from_slice(&minor.to_le_bytes());
    bytes[12..16].copy_from_slice(&u32::try_from(HEADER_SIZE).expect("64").to_le_bytes());
    bytes[16..24].copy_from_slice(&len_u64(body_length).to_le_bytes());
    bytes[24..28].copy_from_slice(&u32::try_from(sections.len()).expect("8").to_le_bytes());
    for (i, (kind, offset, length)) in table.into_iter().enumerate() {
        let at = HEADER_SIZE + i * SECTION_ENTRY_SIZE;
        bytes[at..at + 4].copy_from_slice(&kind.to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&len_u64(offset).to_le_bytes());
        bytes[at + 16..at + 24].copy_from_slice(&len_u64(length).to_le_bytes());
    }

    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    bytes.extend_from_slice(&digest);

    if let Some(signer) = signer {
        bytes.extend_from_slice(SIGNATURE_MAGIC);
        put_u16(&mut bytes, SIGNATURE_ALGORITHM_ECDSA_P256);
        put_u16(&mut bytes, 0);
        bytes.extend_from_slice(&signer.public_key());
        bytes.extend_from_slice(&signer.sign(SIGNATURE_DOMAIN, &digest));
        match endorsement {
            None => bytes.push(0),
            Some(endorsement) => {
                bytes.push(1);
                bytes.extend_from_slice(&endorsement.previous_public_key);
                bytes.extend_from_slice(&endorsement.signature);
            }
        }
    }
    Ok(bytes)
}

fn digest_of(bytes: &[u8]) -> [u8; 32] {
    let body_length = usize::try_from(u64::from_le_bytes(
        bytes[16..24].try_into().expect("8 bytes"),
    ))
    .expect("pack body fits in memory");
    bytes[body_length - DIGEST_SIZE..body_length]
        .try_into()
        .expect("32 bytes")
}

fn pad8(bytes: &mut Vec<u8>) {
    while !bytes.len().is_multiple_of(8) {
        bytes.push(0);
    }
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn len_u64(len: usize) -> u64 {
    u64::try_from(len).expect("usize fits in u64")
}

fn to_u32(value: u64, what: &str) -> Result<u32, ExportError> {
    u32::try_from(value).map_err(|_| ExportError::Limit(format!("{what} exceeds u32")))
}
