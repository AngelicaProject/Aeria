//! The game installation Aeria builds source packages from.
//!
//! The installation is an application setting: the folder chosen in
//! Settings, or, while none is chosen, the first installation detected on
//! this computer. Detection only proposes paths: an installation is reported
//! when it has the layout Harmonia Atlas reads (`game/sqpack` and
//! `game/ffxivgame.ver`), and Atlas validates the game data itself.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::paths::AeriaPaths;

type CommandResult<T> = Result<T, CommandError>;

const STEAM_FOLDER: &str = "FINAL FANTASY XIV Online";
const MAX_VERSION_FILE_BYTES: u64 = 256;
const SETTINGS_FILE: &str = "game-settings.json";
const SETTINGS_VERSION: u32 = 1;
const MAX_SETTINGS_FILE_BYTES: u64 = 64 * 1024;

/// Where an installation comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GameOriginDto {
    /// The folder chosen in Settings.
    Settings,
    /// The Square Enix launcher's installation record.
    SquareEnix,
    /// A Steam library.
    Steam,
    /// The game path configured in `XIVLauncher`.
    XivLauncher,
    /// A conventional installation folder.
    DefaultLocation,
}

/// One detected game installation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInstallationDto {
    /// The installation root to pass to Atlas.
    pub path: String,
    /// Contents of `game/ffxivgame.ver`.
    pub game_version: String,
    pub origin: GameOriginDto,
}

/// The game installation setting and what it resolves to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSettingsDto {
    /// The folder chosen in Settings; `None` uses the first detected one.
    pub configured_path: Option<String>,
    /// The installation game operations use, or `None` when the chosen
    /// folder is no longer an installation or nothing was detected.
    pub active: Option<GameInstallationDto>,
    /// Installations found on this computer, most authoritative first.
    pub detected: Vec<GameInstallationDto>,
}

/// The persisted `game-settings.json` document in the app-data directory.
#[derive(Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GameSettings {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    game_path: Option<String>,
}

fn settings_path(app: &tauri::AppHandle) -> CommandResult<PathBuf> {
    app.aeria_data_dir()
        .map(|path| path.join(SETTINGS_FILE))
        .map_err(|error| {
            CommandError::new(
                "gameSettings",
                format!("could not resolve the Aeria app-data directory: {error}"),
            )
        })
}

/// Reads the configured game folder. A missing file means none is chosen; a
/// malformed file is an error and is never replaced with defaults.
pub(crate) fn load_game_path(path: &Path) -> CommandResult<Option<String>> {
    let settings_error = |message: String| CommandError::new("gameSettings", message);
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(settings_error(format!("{}: {error}", path.display()))),
    };
    if metadata.len() > MAX_SETTINGS_FILE_BYTES {
        return Err(settings_error(format!("{} is too large", path.display())));
    }
    let bytes =
        fs::read(path).map_err(|error| settings_error(format!("{}: {error}", path.display())))?;
    let settings: GameSettings = serde_json::from_slice(&bytes)
        .map_err(|error| settings_error(format!("{} is invalid: {error}", path.display())))?;
    if settings.version != SETTINGS_VERSION {
        return Err(settings_error(format!(
            "{} has unsupported version {}",
            path.display(),
            settings.version
        )));
    }
    Ok(settings.game_path)
}

/// Publishes the configured game folder by replacing the file atomically.
pub(crate) fn store_game_path(path: &Path, game_path: Option<String>) -> CommandResult<()> {
    let settings_error = |message: String| CommandError::new("gameSettings", message);
    let mut bytes = serde_json::to_vec_pretty(&GameSettings {
        version: SETTINGS_VERSION,
        game_path,
    })
    .map_err(|error| settings_error(error.to_string()))?;
    bytes.push(b'\n');
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| settings_error(format!("{}: {error}", parent.display())))?;
    }
    let partial = path.with_extension("json.partial");
    fs::File::create(&partial)
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&partial, path))
        .map_err(|error| {
            let _ = fs::remove_file(&partial);
            settings_error(format!("could not save {}: {error}", path.display()))
        })
}

/// Returns the installation root for a folder the user chose: the folder
/// itself, or its parent when the user picked the inner `game` folder.
pub(crate) fn installation_root(path: &Path) -> Option<(PathBuf, String)> {
    if let Some(version) = installation_version(path) {
        return Some((path.to_owned(), version));
    }
    let parent = path.parent().filter(|_| {
        path.file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("game"))
    })?;
    installation_version(parent).map(|version| (parent.to_owned(), version))
}

fn settings_dto(configured_path: Option<String>) -> GameSettingsDto {
    let detected = inspect_candidates(candidates());
    let active = match &configured_path {
        Some(path) => {
            installation_version(Path::new(path)).map(|game_version| GameInstallationDto {
                path: path.clone(),
                game_version,
                origin: GameOriginDto::Settings,
            })
        }
        None => detected.first().cloned(),
    };
    GameSettingsDto {
        configured_path,
        active,
        detected,
    }
}

/// Returns the installation game operations use: the folder chosen in
/// Settings, otherwise the first detected installation.
///
/// # Errors
///
/// Returns `gameInstallationInvalid` when the chosen folder is no longer an
/// installation, `gameInstallationRequired` when none is chosen or detected,
/// or `gameSettings` when the setting cannot be read.
pub(crate) fn resolve_game_path(app: &tauri::AppHandle) -> CommandResult<String> {
    match load_game_path(&settings_path(app)?)? {
        Some(path) if installation_version(Path::new(&path)).is_some() => Ok(path),
        Some(path) => Err(CommandError::new(
            "gameInstallationInvalid",
            format!("{path} is no longer a game installation; choose the game folder in Settings"),
        )),
        None => inspect_candidates(candidates())
            .into_iter()
            .next()
            .map(|installation| installation.path)
            .ok_or_else(|| {
                CommandError::new(
                    "gameInstallationRequired",
                    "no game installation was found; choose the game folder in Settings",
                )
            }),
    }
}

#[tauri::command(rename_all = "camelCase")]
/// Returns the game installation setting, what it resolves to, and the
/// installations detected on this computer.
///
/// # Errors
///
/// Returns a typed command error when the setting cannot be read.
pub async fn game_settings(app: tauri::AppHandle) -> CommandResult<GameSettingsDto> {
    run_blocking(move || Ok(settings_dto(load_game_path(&settings_path(&app)?)?))).await
}

#[tauri::command(rename_all = "camelCase")]
/// Chooses the game installation, or with `None` returns to automatic
/// detection. A chosen folder must have the installation layout; picking its
/// inner `game` folder selects the installation root.
///
/// # Errors
///
/// Returns `gameInstallationInvalid` for a folder that is not an
/// installation, or `gameSettings` when the setting cannot be saved.
pub async fn set_game_path(
    app: tauri::AppHandle,
    path: Option<String>,
) -> CommandResult<GameSettingsDto> {
    run_blocking(move || {
        let path = match path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            Some(path) => {
                let (root, _) = installation_root(Path::new(path)).ok_or_else(|| {
                    CommandError::new(
                        "gameInstallationInvalid",
                        format!("{path} has no game/sqpack and game/ffxivgame.ver"),
                    )
                })?;
                Some(root.to_string_lossy().into_owned())
            }
            None => None,
        };
        store_game_path(&settings_path(&app)?, path.clone())?;
        Ok(settings_dto(path))
    })
    .await
}

pub(crate) fn inspect_candidates(
    candidates: impl IntoIterator<Item = (PathBuf, GameOriginDto)>,
) -> Vec<GameInstallationDto> {
    let mut seen = HashSet::new();
    let mut installations = Vec::new();
    for (path, origin) in candidates {
        let Some(game_version) = installation_version(&path) else {
            continue;
        };
        let path = fs::canonicalize(&path).map_or(path, |canonical| plain_path(&canonical));
        if seen.insert(path.clone()) {
            installations.push(GameInstallationDto {
                path: path.to_string_lossy().into_owned(),
                game_version,
                origin,
            });
        }
    }
    installations
}

/// Drops the Windows verbatim prefix from a canonical drive path, so the
/// path reads as the user knows it. Other paths are returned unchanged.
fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_owned(),
    }
}

/// Returns the game version when `root` has the installation layout Atlas
/// reads.
pub(crate) fn installation_version(root: &Path) -> Option<String> {
    let game = root.join("game");
    if !game.join("sqpack").is_dir() {
        return None;
    }
    let version_path = game.join("ffxivgame.ver");
    let metadata = fs::metadata(&version_path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_VERSION_FILE_BYTES {
        return None;
    }
    let version = fs::read_to_string(version_path).ok()?;
    let version = version.trim();
    (!version.is_empty()).then(|| version.to_owned())
}

/// Returns every version file of an installation: `ffxivgame` for
/// `game/ffxivgame.ver` and `exN` for each `game/sqpack/exN/exN.ver`. A patch
/// changes at least one of them, so equal maps mean equal game data for
/// source-package reuse.
pub(crate) fn installation_versions(root: &Path) -> Option<BTreeMap<String, String>> {
    let mut versions = BTreeMap::from([("ffxivgame".to_owned(), installation_version(root)?)]);
    let sqpack = root.join("game").join("sqpack");
    for entry in fs::read_dir(&sqpack).ok()? {
        let name = entry.ok()?.file_name().to_string_lossy().into_owned();
        if !name.starts_with("ex") {
            continue;
        }
        let path = sqpack.join(&name).join(format!("{name}.ver"));
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_VERSION_FILE_BYTES {
            return None;
        }
        versions.insert(name, fs::read_to_string(path).ok()?.trim().to_owned());
    }
    Some(versions)
}

/// Reads the library paths from a Steam `libraryfolders.vdf`.
pub(crate) fn parse_steam_library_folders(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let mut tokens = line.split('"').skip(1).step_by(2);
            if tokens.next()? != "path" {
                return None;
            }
            Some(PathBuf::from(tokens.next()?.replace("\\\\", "\\")))
        })
        .collect()
}

fn steam_game_paths(steam_roots: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut libraries = Vec::new();
    for root in steam_roots {
        libraries.push(root.clone());
        if let Ok(text) = fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) {
            libraries.extend(parse_steam_library_folders(&text));
        }
    }
    libraries
        .into_iter()
        .map(|library| library.join("steamapps").join("common").join(STEAM_FOLDER))
        .collect()
}

#[cfg(windows)]
fn candidates() -> Vec<(PathBuf, GameOriginDto)> {
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

    const UNINSTALL: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
    let registry_path = |key: &windows_registry::Key, path: &str, value: &str| {
        key.open(path)
            .and_then(|key| key.get_string(value))
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| PathBuf::from(value.trim()))
    };

    let mut candidates = Vec::new();
    if let Some(path) = registry_path(
        LOCAL_MACHINE,
        &format!(r"{UNINSTALL}\{{2B41E132-07DF-4925-A3D3-F2D1765CCDFE}}"),
        "InstallLocation",
    ) {
        candidates.push((path, GameOriginDto::SquareEnix));
    }
    if let Some(path) = registry_path(
        LOCAL_MACHINE,
        &format!(r"{UNINSTALL}\Steam App 39210"),
        "InstallLocation",
    ) {
        candidates.push((path, GameOriginDto::Steam));
    }
    let steam_roots = [
        registry_path(CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
        registry_path(
            LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Valve\Steam",
            "InstallPath",
        ),
    ];
    candidates.extend(
        steam_game_paths(steam_roots.into_iter().flatten())
            .into_iter()
            .map(|path| (path, GameOriginDto::Steam)),
    );
    if let Some(app_data) = std::env::var_os("APPDATA") {
        let config = PathBuf::from(app_data)
            .join("XIVLauncher")
            .join("launcherConfigV3.json");
        if let Some(path) = fs::read_to_string(config)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|config| config.get("GamePath")?.as_str().map(PathBuf::from))
        {
            candidates.push((path, GameOriginDto::XivLauncher));
        }
    }
    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(program_files) = std::env::var_os(variable).map(PathBuf::from) {
            candidates.push((
                program_files
                    .join("SquareEnix")
                    .join("FINAL FANTASY XIV - A Realm Reborn"),
                GameOriginDto::DefaultLocation,
            ));
            candidates.extend(
                steam_game_paths([program_files.join("Steam")])
                    .into_iter()
                    .map(|path| (path, GameOriginDto::Steam)),
            );
        }
    }
    candidates
}

#[cfg(not(windows))]
fn candidates() -> Vec<(PathBuf, GameOriginDto)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    let xlcore = home.join(".xlcore");
    if let Some(path) = fs::read_to_string(xlcore.join("launcher.ini"))
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix("GamePath=").map(str::trim))
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
    {
        candidates.push((path, GameOriginDto::XivLauncher));
    }
    candidates.push((xlcore.join("ffxiv"), GameOriginDto::XivLauncher));
    let steam_roots = [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];
    candidates.extend(
        steam_game_paths(steam_roots)
            .into_iter()
            .map(|path| (path, GameOriginDto::Steam)),
    );
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_installation(root: &Path, version: &str) {
        fs::create_dir_all(root.join("game").join("sqpack")).expect("sqpack");
        fs::write(root.join("game").join("ffxivgame.ver"), version).expect("version");
    }

    #[test]
    fn only_the_atlas_layout_is_an_installation() {
        let directory = tempfile::tempdir().expect("temp dir");
        let game = directory.path().join("ffxiv");
        fake_installation(&game, "2026.09.01.0000.0000\r\n");
        assert_eq!(
            installation_version(&game).as_deref(),
            Some("2026.09.01.0000.0000")
        );
        assert_eq!(installation_version(&game.join("game")), None);
        assert_eq!(installation_version(directory.path()), None);

        let empty_version = directory.path().join("empty");
        fake_installation(&empty_version, "  ");
        assert_eq!(installation_version(&empty_version), None);
    }

    #[test]
    fn candidates_are_filtered_and_deduplicated_in_order() {
        let directory = tempfile::tempdir().expect("temp dir");
        let game = directory.path().join("ffxiv");
        fake_installation(&game, "1.0");
        let found = inspect_candidates([
            (directory.path().join("missing"), GameOriginDto::SquareEnix),
            (game.clone(), GameOriginDto::Steam),
            (game.join("game").join(".."), GameOriginDto::DefaultLocation),
        ]);
        let canonical = plain_path(&fs::canonicalize(&game).expect("canonical"));
        assert!(!canonical.to_string_lossy().starts_with(r"\\?\"));
        assert_eq!(
            found,
            vec![GameInstallationDto {
                path: canonical.to_string_lossy().into_owned(),
                game_version: "1.0".to_owned(),
                origin: GameOriginDto::Steam,
            }]
        );
    }

    #[test]
    fn installation_versions_include_every_expansion() {
        let directory = tempfile::tempdir().expect("temp dir");
        let game = directory.path().join("ffxiv");
        fake_installation(&game, "2026.09.15.0000.0000\r\n");
        for (name, version) in [
            ("ex1", "2026.09.01.0000.0000"),
            ("ex2", "2026.09.02.0000.0000"),
        ] {
            let folder = game.join("game").join("sqpack").join(name);
            fs::create_dir_all(&folder).expect("expansion");
            fs::write(folder.join(format!("{name}.ver")), version).expect("version");
        }
        fs::create_dir_all(game.join("game").join("sqpack").join("ffxiv")).expect("base");
        assert_eq!(
            installation_versions(&game),
            Some(BTreeMap::from([
                ("ex1".to_owned(), "2026.09.01.0000.0000".to_owned()),
                ("ex2".to_owned(), "2026.09.02.0000.0000".to_owned()),
                ("ffxivgame".to_owned(), "2026.09.15.0000.0000".to_owned()),
            ]))
        );
        assert_eq!(installation_versions(directory.path()), None);
    }

    #[test]
    fn the_inner_game_folder_selects_the_installation_root() {
        let directory = tempfile::tempdir().expect("temp dir");
        let game = directory.path().join("Игры с пробелом").join("ffxiv");
        fake_installation(&game, "1.0");
        assert_eq!(
            installation_root(&game.join("game")),
            Some((game.clone(), "1.0".to_owned()))
        );
        assert_eq!(
            installation_root(&game),
            Some((game.clone(), "1.0".to_owned()))
        );
        assert_eq!(installation_root(&game.join("game").join("sqpack")), None);
    }

    #[test]
    fn the_game_setting_round_trips_and_rejects_malformed_files() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("nested").join(SETTINGS_FILE);
        assert_eq!(load_game_path(&path).expect("missing"), None);

        let game = r"D:\Игры\Final Fantasy XIV — копия";
        store_game_path(&path, Some(game.to_owned())).expect("store");
        assert_eq!(load_game_path(&path).expect("load").as_deref(), Some(game));
        assert!(!path.with_extension("json.partial").exists());
        store_game_path(&path, None).expect("clear");
        assert_eq!(load_game_path(&path).expect("cleared"), None);

        for text in ["{", r#"{"version":2}"#, r#"{"version":1,"other":true}"#] {
            fs::write(&path, text).expect("write");
            assert_eq!(
                load_game_path(&path).expect_err(text).code,
                "gameSettings",
                "{text}"
            );
            assert_eq!(fs::read_to_string(&path).expect("kept"), text);
        }
    }

    #[test]
    fn steam_library_folders_are_parsed() {
        let text = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"39210"		"84551930164"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;
        assert_eq!(
            parse_steam_library_folders(text),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\SteamLibrary"),
            ]
        );
    }
}
