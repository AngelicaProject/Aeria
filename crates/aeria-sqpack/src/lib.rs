//! Read-only access to an installed game's `SqPack` archives and Excel sheets.
//!
//! [`GameData`] finds files in `game/sqpack` by their game path, and
//! [`excel`] reads the sheet list, sheet headers, and row data from them.
//! Nothing here writes to the installation.

#![forbid(unsafe_code)]

mod dat;
pub mod excel;
mod index;

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use thiserror::Error;

use crate::index::{IndexEntry, SqPackIndex};

/// Errors from reading game data.
#[derive(Debug, Error)]
pub enum SqPackError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("the game path has no game/sqpack directory: {0}")]
    NotAnInstallation(PathBuf),

    #[error("{0}")]
    Invalid(String),
}

impl SqPackError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

/// `SqPack` category IDs by the first segment of a game path.
fn category_id(name: &str) -> Option<u8> {
    Some(match name {
        "common" => 0x00,
        "bgcommon" => 0x01,
        "bg" => 0x02,
        "cut" => 0x03,
        "chara" => 0x04,
        "shader" => 0x05,
        "ui" => 0x06,
        "sound" => 0x07,
        "vfx" => 0x08,
        "ui_script" => 0x09,
        "exd" => 0x0A,
        "game_script" => 0x0B,
        "music" => 0x0C,
        "sqpack_test" => 0x12,
        "debug" => 0x13,
        _ => return None,
    })
}

/// The index hash of a game path: the CRC-32 register (the standard CRC-32
/// with its final inversion undone) of the folder and of the file name.
fn path_hash(path: &str) -> Option<u64> {
    let (folder, file) = path.rsplit_once('/')?;
    let crc = |text: &str| !crc32fast::hash(text.as_bytes());
    Some((u64::from(crc(folder)) << 32) | u64::from(crc(file)))
}

/// One archive chunk of a category: its index and data files.
struct Chunk {
    /// File name prefix, such as `0a0000`.
    prefix: String,
    index: SqPackIndex,
}

/// Loaded chunks per (repository, category).
type ChunkCache = HashMap<(String, u8), std::sync::Arc<Vec<Chunk>>>;

/// An installed game's data files.
pub struct GameData {
    sqpack: PathBuf,
    chunks: Mutex<ChunkCache>,
    /// Open data files by name.
    files: Mutex<HashMap<String, File>>,
}

impl GameData {
    /// Opens the installation at `game_path`, the folder that contains
    /// `game/sqpack`.
    ///
    /// # Errors
    ///
    /// Returns an error when `game/sqpack` is missing.
    pub fn open(game_path: impl AsRef<Path>) -> Result<Self, SqPackError> {
        let sqpack = game_path.as_ref().join("game").join("sqpack");
        if !sqpack.is_dir() {
            return Err(SqPackError::NotAnInstallation(
                game_path.as_ref().to_owned(),
            ));
        }
        Ok(Self {
            sqpack,
            chunks: Mutex::new(HashMap::new()),
            files: Mutex::new(HashMap::new()),
        })
    }

    /// Reads a file by its game path, such as `exd/root.exl`. Paths are
    /// case-insensitive; a file the archives do not contain is `None`.
    ///
    /// # Errors
    ///
    /// Returns an error for an unreadable or malformed archive.
    pub fn file(&self, path: &str) -> Result<Option<Vec<u8>>, SqPackError> {
        let path = path.trim().to_lowercase();
        if path.is_empty() || path.ends_with('/') || path.len() >= 260 {
            return Ok(None);
        }
        let Some(hash) = path_hash(&path) else {
            return Ok(None);
        };
        let whole_hash = !crc32fast::hash(path.as_bytes());
        let mut parts = path.split('/');
        let Some(category) = parts.next().and_then(category_id) else {
            return Ok(None);
        };
        let repository = parts
            .next()
            .filter(|part| {
                let bytes = part.as_bytes();
                bytes.len() >= 3 && bytes.starts_with(b"ex") && bytes[2].is_ascii_digit()
            })
            .unwrap_or("ffxiv")
            .to_owned();
        let chunks = self.chunks(&repository, category)?;
        for chunk in chunks.iter() {
            if let Some(entry) = chunk.index.get(hash, whole_hash) {
                return self.read_entry(&repository, chunk, entry).map(Some);
            }
        }
        Ok(None)
    }

    fn chunks(
        &self,
        repository: &str,
        category: u8,
    ) -> Result<std::sync::Arc<Vec<Chunk>>, SqPackError> {
        let key = (repository.to_owned(), category);
        let mut cache = self
            .chunks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(chunks) = cache.get(&key) {
            return Ok(std::sync::Arc::clone(chunks));
        }
        let expansion = repository
            .strip_prefix("ex")
            .and_then(|number| number.parse::<u8>().ok())
            .unwrap_or(0);
        let folder = self.sqpack.join(repository);
        let mut chunks = Vec::new();
        for chunk in 0..255_u8 {
            let prefix = format!("{category:02x}{expansion:02x}{chunk:02x}");
            // The first index kind present is used, as the game does.
            for kind in ["index", "index2"] {
                let path = folder.join(format!("{prefix}.win32.{kind}"));
                if path.is_file() {
                    let bytes =
                        std::fs::read(&path).map_err(|error| SqPackError::io(&path, error))?;
                    let index =
                        SqPackIndex::parse(&bytes, kind == "index2").map_err(|message| {
                            SqPackError::invalid(format!("{}: {message}", path.display()))
                        })?;
                    chunks.push(Chunk {
                        prefix: prefix.clone(),
                        index,
                    });
                    break;
                }
            }
        }
        let chunks = std::sync::Arc::new(chunks);
        cache.insert(key, std::sync::Arc::clone(&chunks));
        Ok(chunks)
    }

    fn read_entry(
        &self,
        repository: &str,
        chunk: &Chunk,
        entry: IndexEntry,
    ) -> Result<Vec<u8>, SqPackError> {
        let name = format!("{}.win32.dat{}", chunk.prefix, entry.data_file);
        let path = self.sqpack.join(repository).join(&name);
        let mut files = self
            .files
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !files.contains_key(&name) {
            let file = File::open(&path).map_err(|error| SqPackError::io(&path, error))?;
            files.insert(name.clone(), file);
        }
        let file = files.get_mut(&name).expect("the file was just opened");
        dat::read_file(file, entry.offset).map_err(|message| {
            SqPackError::invalid(format!(
                "{} at {:#x}: {message}",
                path.display(),
                entry.offset
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_hashes_use_the_crc_register_of_folder_and_file() {
        // CRC-32("exd") = 0x1C648666 and CRC-32("root.exl") = 0xAE4A8143;
        // the index stores their inversions.
        assert_eq!(path_hash("exd/root.exl"), Some(0xE39B_7999_51B5_7EBC));
        assert_eq!(path_hash("noslash"), None);
    }

    #[test]
    fn game_paths_map_to_categories() {
        assert_eq!(category_id("exd"), Some(0x0A));
        assert_eq!(category_id("nope"), None);
    }
}
