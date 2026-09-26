//! Synthetic games and repository helpers for workspace tests.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aeria_source::{GameSource, SourceLanguage};
use aeria_sqpack::testing::{FakeGame, TextSheet};
use tempfile::TempDir;

pub const V1: &str = "2026.09.15.0000.0000";
pub const V2: &str = "2026.10.01.0000.0000";
pub const V3: &str = "2026.11.01.0000.0000";

/// An installed synthetic game.
pub struct Game {
    _folder: TempDir,
    pub source: Arc<GameSource>,
}

impl Game {
    pub fn handle(&self) -> Arc<GameSource> {
        Arc::clone(&self.source)
    }
}

pub fn game(version: &str, sheets: &[(&str, &TextSheet)]) -> Game {
    game_in(version, sheets, SourceLanguage::English)
}

pub fn game_in(version: &str, sheets: &[(&str, &TextSheet)], language: SourceLanguage) -> Game {
    let folder = tempfile::tempdir().expect("game folder");
    let mut fake = FakeGame::new(version);
    for (name, sheet) in sheets {
        fake = fake.with_text(*name, sheet);
    }
    fake.write(folder.path()).expect("write game");
    let source = GameSource::open(folder.path(), language).expect("open game");
    Game {
        _folder: folder,
        source: Arc::new(source),
    }
}

/// A sheet with one text column.
pub fn texts(rows: &[(u32, &str)]) -> TextSheet {
    rows.iter()
        .fold(TextSheet::new(1, &[0]), |sheet, (row, text)| {
            sheet.row(*row, &[(0, text)])
        })
}

/// A dialogue sheet: a key column and a text column.
pub fn dialogue(rows: &[(u32, &str, &str)]) -> TextSheet {
    rows.iter().fold(
        TextSheet::new(2, &[0, 1]).keyed(0),
        |sheet, (row, key, text)| sheet.row(*row, &[(0, key), (1, text)]),
    )
}

/// The bytes of every file under `.aeria/`.
pub fn managed_files(repository_root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let root = repository_root.join(".aeria");
    let mut pending = vec![root];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path
                    .strip_prefix(repository_root)
                    .expect("inside the repository")
                    .to_owned();
                files.insert(relative, fs::read(&path).expect("file"));
            }
        }
    }
    files
}

/// Copies the `.aeria/` directory of one repository into a new one.
pub fn copy_project(from: &Path) -> TempDir {
    let to = tempfile::tempdir().expect("repository");
    for (path, bytes) in managed_files(from) {
        let target = to.path().join(path);
        fs::create_dir_all(target.parent().expect("parent")).expect("directories");
        fs::write(target, bytes).expect("file");
    }
    to
}
