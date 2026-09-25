use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use brotli::enc::BrotliEncoderParams;

use crate::error::ExportError;

const BROTLI_QUALITY: i32 = 11;
const BROTLI_WINDOW: i32 = 22;

/// Wraps a pack as `.hpk.br`. Fixed encoder parameters keep the output stable
/// for one encoder version; pack identity never depends on these bytes.
///
/// # Errors
/// Returns the encoder's I/O error; in-memory compression does not fail in practice.
pub fn compress_for_transport(pack: &[u8]) -> Result<Vec<u8>, ExportError> {
    let params = BrotliEncoderParams {
        quality: BROTLI_QUALITY,
        lgwin: BROTLI_WINDOW,
        ..BrotliEncoderParams::default()
    };
    let mut output = Vec::new();
    brotli::BrotliCompress(&mut &pack[..], &mut output, &params)?;
    Ok(output)
}

/// Writes `bytes` to a sibling temporary file, flushes it to disk, and
/// renames it over `path`, so readers never see a partially written file.
///
/// # Errors
/// Returns the I/O error; the temporary file is removed on failure.
pub fn write_file_atomically(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let mut temp_name = file_name.to_os_string();
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp = path.with_file_name(temp_name);

    let result = (|| {
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    Ok(result?)
}
