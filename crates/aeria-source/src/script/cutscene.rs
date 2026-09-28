//! The text keys a cutscene file names.
//!
//! A quest scene plays a cutscene with `PlayCutScene(<variable>)`. The
//! variable is one of the quest's script variables in its `Quest` row, whose
//! value is a row of the `Cutscene` sheet; that row's path names the file
//! `cut/<path>.cutb`. The file's timelines name the text keys of the lines the
//! cutscene shows. Aeria reads which keys the file names, not the timelines
//! themselves, so it does not know the order or timing of the lines.

use std::fmt;

/// The first bytes of a cutscene file.
const MAGIC: &[u8; 4] = b"CUTB";

/// Why a cutscene file could not be read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CutsceneError(pub String);

impl fmt::Display for CutsceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CutsceneError {}

/// The distinct `TEXT_…` keys a cutscene file names, in the order they first
/// appear. A key is a string of ASCII letters, digits, and `_` that starts
/// with `TEXT_` and is terminated by a NUL byte and preceded by one or by a
/// byte that is not part of a key.
///
/// # Errors
///
/// Returns an error when the data is not a cutscene file or its recorded size
/// is not its length.
pub fn cutscene_keys(data: &[u8]) -> Result<Vec<String>, CutsceneError> {
    if data.len() < 8 || &data[..4] != MAGIC {
        return Err(CutsceneError("not a cutscene file".to_owned()));
    }
    let size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    if usize::try_from(size).ok() != Some(data.len()) {
        return Err(CutsceneError(format!(
            "the cutscene file records {size} bytes but has {}",
            data.len()
        )));
    }
    let is_key_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut keys: Vec<String> = Vec::new();
    let mut start = 0;
    while let Some(found) = find(&data[start..], b"TEXT_") {
        let begin = start + found;
        let end = data[begin..]
            .iter()
            .position(|byte| !is_key_byte(*byte))
            .map_or(data.len(), |length| begin + length);
        let preceded = begin == 0 || !is_key_byte(data[begin - 1]);
        let terminated = data.get(end) == Some(&0);
        if preceded && terminated && end > begin + 5 {
            let key = String::from_utf8_lossy(&data[begin..end]).into_owned();
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        start = end.max(begin + 1);
    }
    Ok(keys)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(body: &[u8]) -> Vec<u8> {
        let mut data = MAGIC.to_vec();
        data.extend_from_slice(&u32::try_from(body.len() + 8).expect("small").to_le_bytes());
        data.extend_from_slice(body);
        data
    }

    #[test]
    fn keys_are_the_terminated_text_strings_in_first_order() {
        let data = file(
            b"\x01\x00cbfm_talk02\x00TEXT_Q_LUCIANE_000_0001\x00c01\x00\
              \x02TEXT_Q_LEIH_000_0006\x00TEXT_Q_LUCIANE_000_0001\x00\
              xTEXT_Q_NOT_A_KEY\x00TEXT_Q_UNTERMINATED\x01TEXT_\x00",
        );
        assert_eq!(
            cutscene_keys(&data).expect("cutscene"),
            ["TEXT_Q_LUCIANE_000_0001", "TEXT_Q_LEIH_000_0006"]
        );
    }

    #[test]
    fn other_files_and_wrong_sizes_are_refused() {
        assert!(cutscene_keys(b"TMLB\x08\x00\x00\x00").is_err());
        let mut data = file(b"TEXT_Q_A_000_1\x00");
        data.push(0);
        assert!(cutscene_keys(&data).is_err());
    }
}
