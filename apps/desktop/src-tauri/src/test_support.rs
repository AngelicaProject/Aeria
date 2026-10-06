//! Synthetic games and project sessions for desktop tests.

use std::path::Path;
use std::sync::Arc;

use aeria_source::{GameSource, SourceLanguage};
use aeria_sqpack::testing::{FakeGame, TextSheet};
use tempfile::TempDir;

/// The game version of [`test_game`].
pub(crate) const GAME_VERSION: &str = "2026.09.15.0000.0000";

/// A synthetic game installation with two sheets: `Synthetic`, whose rows 7
/// and 42 hold translatable text in column 0, and `Addon`, keyed by
/// column 0 with text in column 1.
pub(crate) struct TestGame {
    folder: TempDir,
}

impl TestGame {
    pub(crate) fn path(&self) -> &Path {
        self.folder.path()
    }
}

pub(crate) fn test_game() -> TestGame {
    let folder = tempfile::tempdir().expect("game folder");
    FakeGame::new(GAME_VERSION)
        .with_text(
            "Synthetic",
            &TextSheet::new(2, &[0])
                .row(7, &[(0, "Welcome back, adventurer")])
                .row(42, &[(0, "Hello there")]),
        )
        .with_text(
            "Addon",
            &TextSheet::new(2, &[0, 1])
                .keyed(0)
                .row(1, &[(0, "ADDON_OK"), (1, "Confirm")])
                .row(2, &[(0, "ADDON_CANCEL"), (1, "Cancel")]),
        )
        .with_text(
            "ENpcResident",
            &TextSheet::new(1, &[0]).row(1_019_070, &[(0, "East Aldenard Trading Company aide")]),
        )
        .write(folder.path())
        .expect("write game");
    TestGame { folder }
}

/// Opens a synthetic game in English with its sheet catalog.
pub(crate) fn open(game_path: &Path) -> Arc<GameSource> {
    let source = GameSource::open(game_path, SourceLanguage::English).expect("open game");
    let catalog = source
        .summarize(1, &|| true)
        .expect("summary")
        .expect("not cancelled");
    source.set_catalog(Arc::new(catalog));
    Arc::new(source)
}
