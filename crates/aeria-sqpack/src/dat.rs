//! `SqPack` data files: file headers and deflate blocks.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// A standard file: stored as blocks.
const STANDARD: u32 = 2;
/// A block whose data is stored without compression.
const UNCOMPRESSED_BLOCK: u32 = 32_000;
/// Largest file accepted, far above any Excel file.
const MAX_FILE_SIZE: u32 = 256 * 1024 * 1024;
/// Largest block accepted; the game writes blocks of at most 16,000 bytes.
const MAX_BLOCK_SIZE: u32 = 1024 * 1024;

fn read_exact(file: &mut File, offset: u64, length: usize) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    let mut buffer = vec![0; length];
    file.read_exact(&mut buffer)
        .map_err(|error| error.to_string())?;
    Ok(buffer)
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four bytes"))
}

/// Reads the standard file whose header starts at `offset`.
pub(crate) fn read_file(file: &mut File, offset: u64) -> Result<Vec<u8>, String> {
    // Header: size, type, raw size, two unknown words, block count.
    let header = read_exact(file, offset, 24)?;
    let header_size = u64::from(u32_at(&header, 0));
    let kind = u32_at(&header, 4);
    let raw_size = u32_at(&header, 8);
    let block_count = u32_at(&header, 20);
    if kind != STANDARD {
        return Err(format!("file type {kind} is not a standard file"));
    }
    if raw_size > MAX_FILE_SIZE {
        return Err(format!("the file is {raw_size} bytes"));
    }
    // Every block holds at least one byte, except in an empty file.
    if block_count > raw_size.max(1) {
        return Err(format!("{block_count} blocks for {raw_size} bytes"));
    }
    let block_count = block_count as usize;
    // Block table: offset from the end of the header, compressed and
    // uncompressed sizes.
    let table = read_exact(file, offset + 24, block_count * 8)?;
    let mut data = Vec::with_capacity(raw_size as usize);
    for block in table.as_chunks::<8>().0 {
        let block_offset = offset + header_size + u64::from(u32_at(block, 0));
        read_block(file, block_offset, &mut data)?;
        if data.len() > raw_size as usize {
            return Err(format!(
                "the blocks hold more than the stated {raw_size} bytes"
            ));
        }
    }
    if data.len() != raw_size as usize {
        return Err(format!(
            "the blocks hold {} bytes, the header states {raw_size}",
            data.len()
        ));
    }
    Ok(data)
}

/// Appends one block: a 16-byte header (size, unknown, compressed size or
/// the uncompressed marker, data size) and its data.
fn read_block(file: &mut File, offset: u64, output: &mut Vec<u8>) -> Result<(), String> {
    let header = read_exact(file, offset, 16)?;
    let stored = u32_at(&header, 8);
    let size = u32_at(&header, 12);
    if size > MAX_BLOCK_SIZE || (stored != UNCOMPRESSED_BLOCK && stored > MAX_BLOCK_SIZE) {
        return Err(format!(
            "a block of {size} bytes stored as {stored} is too large"
        ));
    }
    let size = size as usize;
    if stored == UNCOMPRESSED_BLOCK {
        output.extend_from_slice(&read_exact(file, offset + 16, size)?);
        return Ok(());
    }
    let compressed = read_exact(file, offset + 16, stored as usize)?;
    let start = output.len();
    output.resize(start + size, 0);
    let mut decoder = flate2::read::DeflateDecoder::new(compressed.as_slice());
    decoder
        .read_exact(&mut output[start..])
        .map_err(|error| format!("a block does not inflate to {size} bytes: {error}"))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn word(value: u32) -> [u8; 4] {
        value.to_le_bytes()
    }

    /// A standard file at offset 0 with one stored and one deflated block.
    fn two_block_file() -> Vec<u8> {
        let mut deflated =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        deflated.write_all(b"world").expect("deflate");
        let deflated = deflated.finish().expect("deflate");
        let header_size = 128_u32;
        let mut bytes = Vec::new();
        for value in [header_size, STANDARD, 10, 0, 0, 2] {
            bytes.extend_from_slice(&word(value));
        }
        // Block table: offsets relative to the end of the header.
        bytes.extend_from_slice(&word(0));
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&word(128));
        bytes.extend_from_slice(&[0; 4]);
        bytes.resize(header_size as usize, 0);
        for value in [16, 0, UNCOMPRESSED_BLOCK, 5] {
            bytes.extend_from_slice(&word(value));
        }
        bytes.extend_from_slice(b"hello");
        bytes.resize(header_size as usize + 128, 0);
        let stored = u32::try_from(deflated.len()).expect("small");
        for value in [16, 0, stored, 5] {
            bytes.extend_from_slice(&word(value));
        }
        bytes.extend_from_slice(&deflated);
        bytes
    }

    fn temporary_file(name: &str, bytes: &[u8]) -> (std::path::PathBuf, File) {
        let path = std::env::temp_dir().join(format!("aeria-sqpack-{name}-{}", std::process::id()));
        std::fs::write(&path, bytes).expect("write");
        let file = File::open(&path).expect("open");
        (path, file)
    }

    #[test]
    fn standard_files_join_stored_and_deflated_blocks() {
        let (path, mut file) = temporary_file("blocks", &two_block_file());
        let data = read_file(&mut file, 0);
        drop(file);
        std::fs::remove_file(path).expect("remove");
        assert_eq!(data.as_deref(), Ok(b"helloworld".as_slice()));
    }

    #[test]
    fn a_size_that_disagrees_with_the_blocks_is_rejected() {
        let mut bytes = two_block_file();
        bytes[8..12].copy_from_slice(&word(9));
        let (path, mut file) = temporary_file("size", &bytes);
        let data = read_file(&mut file, 0);
        drop(file);
        std::fs::remove_file(path).expect("remove");
        assert!(data.is_err());
    }
}
