//! Application updates: release channels, background checks, and installation.
//!
//! Aeria checks the release feed of the chosen channel in the background and
//! offers a newer version. It never installs one without the user's request,
//! and it refuses to while an [`Activity`] runs. Only a copy installed by the
//! Windows installer updates itself; the portable build and development builds
//! report the new version and link to its release. The release model is
//! described in `docs/development/releases.md`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::utils::config::BundleType;
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::CommandError;
use crate::paths::AeriaPaths;
use crate::state::{Activity, DesktopState};

type CommandResult<T> = Result<T, CommandError>;

/// Emitted with an [`UpdateStatusDto`] whenever the update state changes.
pub const STATUS_EVENT: &str = "update://status";

const SETTINGS_FILE: &str = "update-settings.json";
const SETTINGS_VERSION: u32 = 1;
const MAX_SETTINGS_FILE_BYTES: u64 = 64 * 1024;

const RELEASES_URL: &str = "https://github.com/AngelicaProject/Aeria/releases";
/// The newest stable release; GitHub excludes pre-releases from `latest`.
const STABLE_FEED: &str =
    "https://github.com/AngelicaProject/Aeria/releases/latest/download/latest.json";
/// The rolling pre-release rebuilt from every green `main` commit.
const NIGHTLY_FEED: &str =
    "https://github.com/AngelicaProject/Aeria/releases/download/nightly/latest.json";
/// Nightly versions are `<next>-nightly.<build>`.
const NIGHTLY_PRERELEASE: &str = "nightly";

const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
const CHECK_INTERVAL: Duration = Duration::from_hours(6);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS_EVENT_INTERVAL: Duration = Duration::from_millis(250);

/// Which releases a copy of Aeria follows.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateChannel {
    /// Tagged releases.
    Stable,
    /// Every green build of `main`, and stable releases that are newer.
    Nightly,
}

impl UpdateChannel {
    /// The channel a version was published on.
    fn of_version(version: &str) -> Self {
        let nightly = semver::Version::parse(version)
            .is_ok_and(|version| version.pre.split('.').next() == Some(NIGHTLY_PRERELEASE));
        if nightly { Self::Nightly } else { Self::Stable }
    }

    /// Feeds to check. A nightly user also follows stable releases, so a
    /// stable release that is newer than the last nightly is offered at once.
    const fn feeds(self) -> &'static [&'static str] {
        match self {
            Self::Stable => &[STABLE_FEED],
            Self::Nightly => &[NIGHTLY_FEED, STABLE_FEED],
        }
    }
}

/// The GitHub release page of a published version.
fn release_url(version: &str) -> String {
    match UpdateChannel::of_version(version) {
        UpdateChannel::Nightly => format!("{RELEASES_URL}/tag/{NIGHTLY_PRERELEASE}"),
        UpdateChannel::Stable => format!("{RELEASES_URL}/tag/v{version}"),
    }
}

/// Whether `candidate` is a newer version than `current`. Unparseable
/// versions are never newer.
fn is_newer(candidate: &str, current: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(current),
    ) {
        (Ok(candidate), Ok(current)) => candidate > current,
        _ => false,
    }
}

/// The persisted `update-settings.json` document in the app-data directory.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateSettings {
    version: u32,
    channel: UpdateChannel,
}

fn settings_error(message: String) -> CommandError {
    CommandError::new("updateSettings", message)
}

/// Reads the chosen channel. Without a settings file a copy follows the
/// channel it was published on; a malformed file is an error and is never
/// replaced with defaults.
fn load_channel(path: &Path, current_version: &str) -> CommandResult<UpdateChannel> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UpdateChannel::of_version(current_version));
        }
        Err(error) => return Err(settings_error(format!("{}: {error}", path.display()))),
    };
    if metadata.len() > MAX_SETTINGS_FILE_BYTES {
        return Err(settings_error(format!("{} is too large", path.display())));
    }
    let bytes =
        fs::read(path).map_err(|error| settings_error(format!("{}: {error}", path.display())))?;
    let settings: UpdateSettings = serde_json::from_slice(&bytes)
        .map_err(|error| settings_error(format!("{} is invalid: {error}", path.display())))?;
    if settings.version != SETTINGS_VERSION {
        return Err(settings_error(format!(
            "{} has unsupported version {}",
            path.display(),
            settings.version
        )));
    }
    Ok(settings.channel)
}

/// Publishes the chosen channel by replacing the file atomically.
fn store_channel(path: &Path, channel: UpdateChannel) -> CommandResult<()> {
    let mut bytes = serde_json::to_vec_pretty(&UpdateSettings {
        version: SETTINGS_VERSION,
        channel,
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

fn settings_path(app: &tauri::AppHandle) -> CommandResult<PathBuf> {
    app.aeria_data_dir()
        .map(|path| path.join(SETTINGS_FILE))
        .map_err(|error| {
            settings_error(format!(
                "could not resolve the Aeria app-data directory: {error}"
            ))
        })
}

fn current_version(app: &tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Only a copy installed by the Windows installer replaces itself.
fn can_install() -> bool {
    matches!(
        tauri::utils::platform::bundle_type(),
        Some(BundleType::Nsis)
    )
}

/// The update state owned by the desktop process.
#[derive(Default)]
pub struct Updates {
    inner: Mutex<Inner>,
    /// Serializes feed checks.
    checks: tauri::async_runtime::Mutex<()>,
}

#[derive(Default)]
struct Inner {
    checking: bool,
    last_checked_at: Option<u64>,
    available: Option<Update>,
    download: Option<Download>,
    installing: bool,
    error: Option<CommandError>,
}

struct Download {
    version: String,
    received: u64,
    total: Option<u64>,
    /// The verified installer, once downloaded.
    bytes: Option<Vec<u8>>,
}

impl Updates {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatusDto {
    pub current_version: String,
    pub channel: UpdateChannel,
    /// Whether this copy installs updates itself. Portable and development
    /// copies only link to the release.
    pub can_install: bool,
    pub checking: bool,
    /// Milliseconds since the Unix epoch.
    pub last_checked_at: Option<u64>,
    pub available: Option<AvailableUpdateDto>,
    pub download: Option<UpdateDownloadDto>,
    pub installing: bool,
    pub error: Option<CommandError>,
    /// Work that installation waits for.
    pub running_activities: Vec<Activity>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdateDto {
    pub version: String,
    pub channel: UpdateChannel,
    pub notes: Option<String>,
    /// RFC 3339, as published in the feed.
    pub published_at: Option<String>,
    pub release_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDownloadDto {
    pub received: u64,
    pub total: Option<u64>,
    pub ready: bool,
}

fn status(app: &tauri::AppHandle) -> UpdateStatusDto {
    let version = current_version(app);
    let channel = settings_path(app).and_then(|path| load_channel(&path, &version));
    let running_activities = app.state::<DesktopState>().running_activities();
    let inner = app.state::<Updates>();
    let inner = inner.lock();
    let (channel, error) = match channel {
        Ok(channel) => (channel, inner.error.clone()),
        Err(error) => (UpdateChannel::of_version(&version), Some(error)),
    };
    UpdateStatusDto {
        channel,
        can_install: can_install(),
        checking: inner.checking,
        last_checked_at: inner.last_checked_at,
        available: inner.available.as_ref().map(|update| AvailableUpdateDto {
            channel: UpdateChannel::of_version(&update.version),
            notes: update.body.clone().filter(|notes| !notes.trim().is_empty()),
            published_at: update.raw_json["pub_date"].as_str().map(str::to_owned),
            release_url: release_url(&update.version),
            version: update.version.clone(),
        }),
        download: inner.download.as_ref().map(|download| UpdateDownloadDto {
            received: download.received,
            total: download.total,
            ready: download.bytes.is_some(),
        }),
        installing: inner.installing,
        error,
        running_activities,
        current_version: version,
    }
}

fn emit_status(app: &tauri::AppHandle) {
    let _ = app.emit(STATUS_EVENT, status(app));
}

fn now_millis() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
}

/// Checks every feed of the channel and returns the newest offered update.
/// Fails only when no feed answered.
async fn fetch(app: &tauri::AppHandle, channel: UpdateChannel) -> CommandResult<Option<Update>> {
    let check_error =
        |error: tauri_plugin_updater::Error| CommandError::new("updateCheck", error.to_string());
    let mut newest: Option<Update> = None;
    let mut answered = false;
    let mut failure = None;
    for feed in channel.feeds() {
        let endpoint = tauri::Url::parse(feed)
            .map_err(|error| CommandError::internal_state(format!("{feed}: {error}")))?;
        let updater = app
            .updater_builder()
            .endpoints(vec![endpoint])
            .and_then(|builder| builder.timeout(REQUEST_TIMEOUT).build())
            .map_err(check_error)?;
        match updater.check().await {
            Ok(found) => {
                answered = true;
                if let Some(update) = found
                    && newest
                        .as_ref()
                        .is_none_or(|newest| is_newer(&update.version, &newest.version))
                {
                    newest = Some(update);
                }
            }
            Err(error) => failure = Some(error),
        }
    }
    match failure {
        Some(error) if !answered => Err(check_error(error)),
        _ => Ok(newest),
    }
}

async fn check(app: &tauri::AppHandle) -> UpdateStatusDto {
    let updates = app.state::<Updates>();
    let _serial = updates.checks.lock().await;
    {
        let mut inner = updates.lock();
        inner.checking = true;
        inner.error = None;
    }
    emit_status(app);
    let version = current_version(app);
    let result = match settings_path(app).and_then(|path| load_channel(&path, &version)) {
        Ok(channel) => fetch(app, channel).await,
        Err(error) => Err(error),
    };
    {
        let mut inner = updates.lock();
        inner.checking = false;
        inner.last_checked_at = now_millis();
        match result {
            Ok(found) => {
                let found_version = found.as_ref().map(|update| update.version.clone());
                let downloading = inner
                    .download
                    .as_ref()
                    .is_some_and(|download| download.bytes.is_none());
                // A running download keeps the version it started with.
                if !downloading && !inner.installing {
                    if inner
                        .download
                        .as_ref()
                        .is_some_and(|download| Some(&download.version) != found_version.as_ref())
                    {
                        inner.download = None;
                    }
                    inner.available = found;
                }
            }
            Err(error) => inner.error = Some(error),
        }
    }
    emit_status(app);
    status(app)
}

/// Checks for updates shortly after startup and then periodically.
pub(crate) fn start_background_checks(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            check(&app).await;
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

#[tauri::command]
/// Returns the update state without contacting the feed.
pub async fn update_status(app: tauri::AppHandle) -> UpdateStatusDto {
    status(&app)
}

#[tauri::command]
/// Checks the channel's feeds now. A failed check is reported in the status.
pub async fn update_check(app: tauri::AppHandle) -> UpdateStatusDto {
    check(&app).await
}

#[tauri::command]
/// Chooses the release channel and checks it.
///
/// # Errors
///
/// Returns `updateSettings` when the choice cannot be saved, or `updateBusy`
/// while an update is being installed.
pub async fn update_set_channel(
    app: tauri::AppHandle,
    channel: UpdateChannel,
) -> CommandResult<UpdateStatusDto> {
    {
        let updates = app.state::<Updates>();
        let mut inner = updates.lock();
        if inner.installing {
            return Err(CommandError::new(
                "updateBusy",
                "an update is being installed",
            ));
        }
        store_channel(&settings_path(&app)?, channel)?;
        inner.available = None;
        inner.download = None;
        inner.error = None;
    }
    Ok(check(&app).await)
}

#[tauri::command]
/// Downloads and verifies the offered update so it can be installed.
///
/// # Errors
///
/// Returns `updateNotInstallable` for a copy that does not update itself,
/// `updateUnavailable` when no update is offered, `updateBusy` while another
/// download runs, or `updateDownload` when the download or its signature
/// check fails.
pub async fn update_download(app: tauri::AppHandle) -> CommandResult<UpdateStatusDto> {
    if !can_install() {
        return Err(not_installable());
    }
    let update = {
        let updates = app.state::<Updates>();
        let mut inner = updates.lock();
        let Some(update) = inner.available.clone() else {
            return Err(CommandError::new(
                "updateUnavailable",
                "no update is available",
            ));
        };
        let downloaded = inner
            .download
            .as_ref()
            .filter(|download| download.version == update.version)
            .map(|download| download.bytes.is_some());
        match downloaded {
            Some(true) => {
                drop(inner);
                return Ok(status(&app));
            }
            Some(false) => {
                return Err(CommandError::new(
                    "updateBusy",
                    "the update is already downloading",
                ));
            }
            None => {}
        }
        inner.error = None;
        inner.download = Some(Download {
            version: update.version.clone(),
            received: 0,
            total: None,
            bytes: None,
        });
        update
    };
    emit_status(&app);

    let progress_app = app.clone();
    let mut last_event = Instant::now();
    let result = update
        .download(
            |chunk, total| {
                {
                    let updates = progress_app.state::<Updates>();
                    let mut inner = updates.lock();
                    if let Some(download) = inner.download.as_mut() {
                        download.received += chunk as u64;
                        download.total = total;
                    }
                }
                if last_event.elapsed() >= PROGRESS_EVENT_INTERVAL {
                    last_event = Instant::now();
                    emit_status(&progress_app);
                }
            },
            || {},
        )
        .await;

    let outcome = {
        let updates = app.state::<Updates>();
        let mut inner = updates.lock();
        match result {
            Ok(bytes) => {
                if let Some(download) = inner.download.as_mut() {
                    download.bytes = Some(bytes);
                }
                Ok(())
            }
            Err(error) => {
                let error = CommandError::new("updateDownload", error.to_string());
                inner.download = None;
                inner.error = Some(error.clone());
                Err(error)
            }
        }
    };
    emit_status(&app);
    outcome.map(|()| status(&app))
}

#[tauri::command]
/// Installs the downloaded update and restarts Aeria. On Windows the
/// installer replaces the running process, so a successful call does not
/// return.
///
/// # Errors
///
/// Returns `updateBusy` while synchronization, export, translation, or a
/// source package build runs; `updateNotDownloaded` before the download
/// finished; or `updateInstall` when the installer cannot start.
pub async fn update_install(app: tauri::AppHandle) -> CommandResult<()> {
    if !can_install() {
        return Err(not_installable());
    }
    let desktop = app.state::<DesktopState>();
    let updates = app.state::<Updates>();
    // Status events read the activities, so none is emitted while they are
    // held; on Windows a successful install exits before returning.
    let result = desktop.while_idle(|| {
        let (update, bytes) = {
            let mut inner = updates.lock();
            let Some(update) = inner.available.clone() else {
                return Err(not_downloaded());
            };
            let Some(bytes) = inner
                .download
                .as_mut()
                .filter(|download| download.version == update.version)
                .and_then(|download| download.bytes.take())
            else {
                return Err(not_downloaded());
            };
            inner.installing = true;
            (update, bytes)
        };
        let result = update.install(&bytes);
        let mut inner = updates.lock();
        inner.installing = false;
        match result {
            Ok(()) => {
                drop(inner);
                app.restart();
            }
            Err(error) => {
                let error = CommandError::new("updateInstall", error.to_string());
                // The verified installer stays available for another attempt.
                if let Some(download) = inner.download.as_mut() {
                    download.bytes = Some(bytes);
                }
                inner.error = Some(error.clone());
                Err(error)
            }
        }
    });
    emit_status(&app);
    result?
}

#[tauri::command]
/// Opens the release page of the offered update, or of the current version.
///
/// # Errors
///
/// Returns `updateOpen` when the system cannot open the page.
pub async fn update_open_release(app: tauri::AppHandle) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;

    let version = app
        .state::<Updates>()
        .lock()
        .available
        .as_ref()
        .map_or_else(|| current_version(&app), |update| update.version.clone());
    app.opener()
        .open_url(release_url(&version), None::<&str>)
        .map_err(|error| CommandError::new("updateOpen", error.to_string()))
}

fn not_installable() -> CommandError {
    CommandError::new(
        "updateNotInstallable",
        "this copy of Aeria does not update itself; download the new version from its release page",
    )
}

fn not_downloaded() -> CommandError {
    CommandError::new(
        "updateNotDownloaded",
        "download the update before installing it",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_belong_to_the_channel_they_were_published_on() {
        assert_eq!(UpdateChannel::of_version("0.1.0"), UpdateChannel::Stable);
        assert_eq!(
            UpdateChannel::of_version("0.1.1-nightly.42"),
            UpdateChannel::Nightly
        );
        assert_eq!(
            UpdateChannel::of_version("0.2.0-rc.1"),
            UpdateChannel::Stable
        );
        assert_eq!(
            UpdateChannel::of_version("not a version"),
            UpdateChannel::Stable
        );
    }

    #[test]
    fn nightly_builds_precede_the_stable_release_they_lead_to() {
        assert!(is_newer("0.1.1-nightly.1", "0.1.0"));
        assert!(is_newer("0.1.1-nightly.10", "0.1.1-nightly.9"));
        assert!(is_newer("0.1.1", "0.1.1-nightly.10"));
        assert!(!is_newer("0.1.0", "0.1.1-nightly.1"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn release_pages_follow_the_tagging_scheme() {
        assert_eq!(
            release_url("0.1.0"),
            "https://github.com/AngelicaProject/Aeria/releases/tag/v0.1.0"
        );
        assert_eq!(
            release_url("0.1.1-nightly.3"),
            "https://github.com/AngelicaProject/Aeria/releases/tag/nightly"
        );
    }

    #[test]
    fn a_missing_settings_file_follows_the_running_build() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let path = folder.path().join(SETTINGS_FILE);
        assert_eq!(
            load_channel(&path, "0.1.0").expect("default"),
            UpdateChannel::Stable
        );
        assert_eq!(
            load_channel(&path, "0.1.1-nightly.2").expect("default"),
            UpdateChannel::Nightly
        );
    }

    #[test]
    fn the_chosen_channel_round_trips_and_overrides_the_build() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let path = folder.path().join("nested").join(SETTINGS_FILE);
        store_channel(&path, UpdateChannel::Stable).expect("store");
        assert_eq!(
            load_channel(&path, "0.1.1-nightly.2").expect("load"),
            UpdateChannel::Stable
        );
        store_channel(&path, UpdateChannel::Nightly).expect("store");
        assert_eq!(
            load_channel(&path, "0.1.0").expect("load"),
            UpdateChannel::Nightly
        );
        assert!(!path.with_extension("json.partial").exists());
    }

    #[test]
    fn malformed_settings_are_errors_not_defaults() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let path = folder.path().join(SETTINGS_FILE);
        for contents in [
            "not json",
            r#"{"version":2,"channel":"stable"}"#,
            r#"{"version":1,"channel":"beta"}"#,
            r#"{"version":1,"channel":"stable","extra":true}"#,
        ] {
            fs::write(&path, contents).expect("write settings");
            let error = load_channel(&path, "0.1.0").expect_err(contents);
            assert_eq!(error.code, "updateSettings", "{contents}");
        }
    }
}
