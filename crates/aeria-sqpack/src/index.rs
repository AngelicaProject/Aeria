//! `SqPack` index files: hashed game paths to data-file offsets.

use std::collections::HashMap;

/// Where a file's data starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IndexEntry {
    /// The `.datN` file number.
    pub data_file: u8,
    /// Byte offset of the file header in that data file.
    pub offset: u64,
}

impl IndexEntry {
    fn from_data(data: u32) -> Self {
        Self {
            data_file: u8::try_from((data & 0b1110) >> 1).unwrap_or_default(),
            offset: u64::from(data & !0xF) * 8,
        }
    }
}

/// One loaded index. `.index` files key entries by the folder and file
/// hashes of a path; `.index2` files by the hash of the whole path.
pub(crate) struct SqPackIndex {
    entries: HashMap<u64, IndexEntry>,
    whole_path: bool,
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .map(|value| u32::from_le_bytes(value.try_into().expect("four bytes")))
        .ok_or_else(|| format!("the index ends before offset {offset}"))
}

impl SqPackIndex {
    /// Parses an index file of the Windows platform.
    pub(crate) fn parse(bytes: &[u8], whole_path: bool) -> Result<Self, String> {
        if !bytes.starts_with(b"SqPack\0\0") {
            return Err("not a SqPack file".to_owned());
        }
        if bytes.get(8) != Some(&0) {
            return Err("only Windows archives are supported".to_owned());
        }
        let header_size = usize::try_from(u32_at(bytes, 12)?).map_err(|error| error.to_string())?;
        // The index header: size, version, then the hash table's offset and size.
        let table_offset =
            usize::try_from(u32_at(bytes, header_size + 8)?).map_err(|error| error.to_string())?;
        let table_size =
            usize::try_from(u32_at(bytes, header_size + 12)?).map_err(|error| error.to_string())?;
        let entry_size = if whole_path { 8 } else { 16 };
        let table = bytes
            .get(table_offset..table_offset + table_size)
            .ok_or("the hash table is outside the index")?;
        let mut entries = HashMap::with_capacity(table_size / entry_size);
        for entry in table.chunks_exact(entry_size) {
            let (hash, data) = if whole_path {
                (u64::from(u32_at(entry, 0)?), u32_at(entry, 4)?)
            } else {
                (
                    u64::from_le_bytes(entry[..8].try_into().expect("eight bytes")),
                    u32_at(entry, 8)?,
                )
            };
            entries.insert(hash, IndexEntry::from_data(data));
        }
        Ok(Self {
            entries,
            whole_path,
        })
    }

    /// Looks up a path by its folder-and-file hash, or by its whole-path
    /// hash in an `.index2` file.
    pub(crate) fn get(
        &self,
        folder_and_file_hash: u64,
        whole_path_hash: u32,
    ) -> Option<IndexEntry> {
        let key = if self.whole_path {
            u64::from(whole_path_hash)
        } else {
            folder_and_file_hash
        };
        self.entries.get(&key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_encode_the_data_file_and_an_eight_byte_aligned_offset() {
        let entry = IndexEntry::from_data(0x0000_1234 | 0b0110);
        assert_eq!(entry.data_file, 3);
        assert_eq!(entry.offset, 0x1230 * 8);
    }

    #[test]
    fn an_index_maps_hashes_to_entries() {
        let mut bytes = vec![0; 0x400 + 0x400];
        bytes[..8].copy_from_slice(b"SqPack\0\0");
        bytes[12..16].copy_from_slice(&0x400_u32.to_le_bytes());
        let table_offset = 0x800_u32;
        bytes[0x408..0x40C].copy_from_slice(&table_offset.to_le_bytes());
        bytes[0x40C..0x410].copy_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&0xAABB_CCDD_0011_2233_u64.to_le_bytes());
        bytes.extend_from_slice(&0x20_u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        let index = SqPackIndex::parse(&bytes, false).expect("index");
        assert_eq!(
            index.get(0xAABB_CCDD_0011_2233, 0),
            Some(IndexEntry {
                data_file: 0,
                offset: 0x100
            })
        );
        assert!(SqPackIndex::parse(b"nope", false).is_err());
    }
}
