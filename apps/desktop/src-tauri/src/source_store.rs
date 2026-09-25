//! Aeria's local store of published source packages.
//!
//! The store is a cache in the app-data directory. Beside each package that
//! Aeria built, a build record (`<package>.build.json`) names the inputs:
//! the Atlas executable's SHA-256, the source language, and every game
//! version file. A later build with exactly these inputs reuses the package
//! instead of running Atlas again. Packages without a record, for example
//! ones built before records existed, are never reused for a build, because
//! nothing proves which Atlas and game data produced them.
//!
//! A package is kept while Aeria knows it needs it: a recent project or the
//! open project uses it, or it is the current build for the installed game
//! and Atlas. Any other package can be removed from the Settings.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use aeria_hsp::SourcePackage;
use aeria_projects::{ProjectRegistry, REGISTRY_FILE_NAME};
use aeria_workspace::WorkspaceStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

use crate::commands::{SOURCE_PACKAGES_DIRECTORY, resolve_atlas_executable, run_blocking};
use crate::error::CommandError;
use crate::games::{installation_versions, resolve_game_path};
use crate::paths::AeriaPaths;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

const BUILD_RECORD_EXTENSION: &str = "build.json";
const BUILD_RECORD_VERSION: u32 = 1;
const MAX_BUILD_RECORD_BYTES: u64 = 64 * 1024;

/// The inputs a package was built from.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BuildRecord {
    version: u32,
    atlas_sha256: String,
    source_language: String,
    game_versions: BTreeMap<String, String>,
}

impl BuildRecord {
    pub(crate) fn new(
        atlas_sha256: String,
        source_language: String,
        game_versions: BTreeMap<String, String>,
    ) -> Self {
        Self {
            version: BUILD_RECORD_VERSION,
            atlas_sha256,
            source_language,
            game_versions,
        }
    }
}

/// The build inputs available now. A missing part means no build record can
/// be formed, so nothing is reused or reported as current.
#[derive(Clone, Debug, Default)]
pub(crate) struct CurrentInputs {
    atlas_sha256: Option<String>,
    game_versions: Option<BTreeMap<String, String>>,
}

impl CurrentInputs {
    pub(crate) fn new(atlas: Option<&Path>, game: Option<&Path>) -> Self {
        Self {
            atlas_sha256: atlas.and_then(|path| cached_file_sha256(path).ok()),
            game_versions: game.and_then(installation_versions),
        }
    }

    /// Resolves the configured Atlas and game installation.
    pub(crate) fn resolve(app: &tauri::AppHandle) -> Self {
        let atlas = resolve_atlas_executable(app).ok();
        let game = resolve_game_path(app).ok().map(PathBuf::from);
        Self::new(atlas.as_deref(), game.as_deref())
    }

    pub(crate) fn record(&self, source_language: &str) -> Option<BuildRecord> {
        Some(BuildRecord::new(
            self.atlas_sha256.clone()?,
            source_language.to_owned(),
            self.game_versions.clone()?,
        ))
    }
}

/// Returns the SHA-256 of a file as lowercase hex.
pub(crate) fn file_sha256(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
            hex
        }))
}

/// The last hashed executable, keyed by path, size, and modification time,
/// so the launcher can ask about availability without rehashing Atlas.
type HashCacheEntry = (PathBuf, u64, SystemTime, String);
static HASH_CACHE: Mutex<Option<HashCacheEntry>> = Mutex::new(None);

pub(crate) fn cached_file_sha256(path: &Path) -> std::io::Result<String> {
    let metadata = fs::metadata(path)?;
    let modified = metadata.modified()?;
    let key = (path.to_owned(), metadata.len(), modified);
    if let Ok(cache) = HASH_CACHE.lock()
        && let Some((cached_path, size, time, hash)) = cache.as_ref()
        && (cached_path, *size, *time) == (&key.0, key.1, key.2)
    {
        return Ok(hash.clone());
    }
    let hash = file_sha256(path)?;
    if let Ok(mut cache) = HASH_CACHE.lock() {
        *cache = Some((key.0, key.1, key.2, hash.clone()));
    }
    Ok(hash)
}

fn build_record_path(package_path: &Path) -> PathBuf {
    package_path.with_extension(BUILD_RECORD_EXTENSION)
}

fn read_build_record(path: &Path) -> Option<BuildRecord> {
    if fs::metadata(path).ok()?.len() > MAX_BUILD_RECORD_BYTES {
        return None;
    }
    let record: BuildRecord = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (record.version == BUILD_RECORD_VERSION).then_some(record)
}

/// Records the inputs of a published package, replacing the file atomically.
pub(crate) fn write_build_record(package_path: &Path, record: &BuildRecord) -> std::io::Result<()> {
    let path = build_record_path(package_path);
    let partial = path.with_extension("json.partial");
    let mut bytes = serde_json::to_vec_pretty(record).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    let result = fs::File::create(&partial)
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&partial, &path));
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

fn store_packages(packages_root: &Path) -> Vec<PathBuf> {
    fs::read_dir(packages_root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension().is_some_and(|extension| extension == "hsp") && path.is_file()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Returns the store package whose build record equals `record`, without
/// verifying it.
fn built_package_path(packages_root: &Path, record: &BuildRecord) -> Option<PathBuf> {
    store_packages(packages_root)
        .into_iter()
        .find(|path| read_build_record(&build_record_path(path)).as_ref() == Some(record))
}

/// Returns a verified package in the store built from exactly `record`.
///
/// Records that are unreadable or name a package that no longer verifies
/// are skipped: the caller then runs Atlas and publishes a fresh package.
pub(crate) fn find_built_package(
    packages_root: &Path,
    cache_root: &Path,
    record: &BuildRecord,
) -> Option<SourcePackage> {
    store_packages(packages_root)
        .into_iter()
        .filter(|path| read_build_record(&build_record_path(path)).as_ref() == Some(record))
        .find_map(|path| {
            SourcePackage::open(&path, cache_root)
                .ok()
                .filter(|package| package.source_language() == record.source_language)
        })
}

/// One package in the store.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePackageEntryDto {
    pub path: String,
    pub package_id: String,
    pub source_language: String,
    pub game_version: String,
    pub size_bytes: u64,
    pub built_at_unix_ms: Option<u64>,
    /// Whether a build record allows reusing it for new builds.
    pub reusable: bool,
    /// Built from the installed game with the current Atlas: a new build in
    /// its language would reuse it.
    pub current: bool,
    /// Repository roots of recent projects, and of the open project, that
    /// open with it.
    pub used_by: Vec<String>,
    /// Aeria does not need it: nothing uses it and it is not current.
    pub removable: bool,
}

/// Package IDs Aeria knows it needs, with the projects that use them.
#[derive(Clone, Debug, Default)]
pub(crate) struct PackageUsers {
    pub(crate) projects: Vec<(String, String)>,
}

impl PackageUsers {
    fn of(&self, package_id: &str) -> Vec<String> {
        let mut roots = self
            .projects
            .iter()
            .filter(|(id, _)| id == package_id)
            .map(|(_, root)| root.clone())
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        roots
    }
}

pub(crate) fn list_packages(
    packages_root: &Path,
    users: &PackageUsers,
    inputs: &CurrentInputs,
) -> Vec<SourcePackageEntryDto> {
    let mut packages = store_packages(packages_root)
        .into_iter()
        .filter_map(|path| {
            let metadata = fs::metadata(&path).ok()?;
            let manifest = aeria_hsp::read_manifest(&path).ok()?;
            let record = read_build_record(&build_record_path(&path));
            let current = record.as_ref().is_some_and(|record| {
                inputs.record(&record.source_language).as_ref() == Some(record)
            });
            let used_by = users.of(&manifest.package_id);
            Some(SourcePackageEntryDto {
                removable: !current && used_by.is_empty(),
                used_by,
                current,
                reusable: record.is_some(),
                package_id: manifest.package_id,
                source_language: manifest.source.language,
                game_version: manifest.game_version,
                size_bytes: metadata.len(),
                built_at_unix_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .and_then(|duration| u64::try_from(duration.as_millis()).ok()),
                path: path.to_string_lossy().into_owned(),
            })
        })
        .collect::<Vec<_>>();
    packages.sort_by(|left, right| {
        right
            .built_at_unix_ms
            .cmp(&left.built_at_unix_ms)
            .then_with(|| left.path.cmp(&right.path))
    });
    packages
}

/// Deletes one removable package, its build record, and its materialized
/// HXS when no remaining package shares the snapshot.
pub(crate) fn remove_package(
    packages_root: &Path,
    cache_root: &Path,
    users: &PackageUsers,
    inputs: &CurrentInputs,
    package_id: &str,
) -> CommandResult<()> {
    let entry = list_packages(packages_root, users, inputs)
        .into_iter()
        .find(|entry| entry.package_id == package_id)
        .ok_or_else(|| {
            CommandError::new(
                "sourcePackageNotFound",
                format!("source package {package_id} is not in the store"),
            )
        })?;
    if !entry.removable {
        return Err(CommandError::new(
            "sourcePackageInUse",
            format!("source package {package_id} is still used by Aeria"),
        ));
    }
    let path = PathBuf::from(&entry.path);
    let snapshot_id = aeria_hsp::read_manifest(&path)
        .map(|manifest| manifest.source.snapshot_id)
        .ok();
    let storage = |error: std::io::Error| CommandError::new("atlasStorage", error.to_string());
    fs::remove_file(&path).map_err(storage)?;
    match fs::remove_file(build_record_path(&path)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(storage(error)),
        _ => {}
    }
    if let Some(snapshot_id) = snapshot_id {
        let shared = store_packages(packages_root).iter().any(|other| {
            aeria_hsp::read_manifest(other)
                .is_ok_and(|manifest| manifest.source.snapshot_id == snapshot_id)
        });
        if !shared {
            aeria_hsp::remove_materialized_source(cache_root, &snapshot_id)
                .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
        }
    }
    Ok(())
}

/// Whether a job can use a package Aeria already has.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceAvailabilityDto {
    /// A matching package exists; Atlas will not run.
    Ready,
    /// Atlas will build a package from the game.
    Build,
    /// It cannot be told yet, for example without a game or project.
    Unknown,
}

/// Previews whether a job finds a package without running Atlas: for
/// opening, a package with the project's content; otherwise, or when none
/// has it, a package whose build record matches the current inputs. The
/// package is not verified here; the job still verifies it.
pub(crate) fn availability(
    packages_root: &Path,
    inputs: &CurrentInputs,
    source_language: &str,
    content_id: Option<&str>,
) -> SourceAvailabilityDto {
    if let Some(content_id) = content_id {
        let found = store_packages(packages_root).iter().any(|path| {
            aeria_hsp::read_manifest(path).is_ok_and(|manifest| {
                manifest.source.language == source_language
                    && manifest.source.content_id == content_id
            })
        });
        if found {
            return SourceAvailabilityDto::Ready;
        }
    }
    match inputs.record(source_language) {
        Some(record) if built_package_path(packages_root, &record).is_some() => {
            SourceAvailabilityDto::Ready
        }
        Some(_) => SourceAvailabilityDto::Build,
        None => SourceAvailabilityDto::Unknown,
    }
}

fn packages_root(app: &tauri::AppHandle) -> CommandResult<PathBuf> {
    app.aeria_data_dir()
        .map(|path| path.join(SOURCE_PACKAGES_DIRECTORY))
        .map_err(|error| CommandError::new("atlasStorage", error.to_string()))
}

/// Recent projects and the open project, with their package IDs.
fn package_users(app: &tauri::AppHandle, state: &DesktopState) -> CommandResult<PackageUsers> {
    let mut projects = Vec::new();
    let registry = app
        .aeria_data_dir()
        .map_err(|error| CommandError::new("atlasStorage", error.to_string()))?
        .join(REGISTRY_FILE_NAME);
    {
        let _lock = state.lock_registry()?;
        // An unreadable registry could hide a project that needs a package,
        // so nothing may be removed until it is readable again.
        let entries = ProjectRegistry::new(registry)
            .load()
            .map_err(|error| CommandError::registry_read(&error))?;
        projects.extend(
            entries
                .into_iter()
                .map(|entry| (entry.source_package_id, entry.repository_root)),
        );
    }
    if let Some(session) = state.lock_project()?.as_ref() {
        projects.push((
            session.source_package().package_id().to_owned(),
            session.repository_root().to_string_lossy().into_owned(),
        ));
    }
    Ok(PackageUsers { projects })
}

#[tauri::command(rename_all = "camelCase")]
/// Lists the packages in Aeria's source-package store, newest first, with
/// what uses them. Files whose manifest cannot be read are left out.
///
/// # Errors
///
/// Returns a typed command error when the app-data directory or the
/// recent-project registry cannot be read.
pub async fn list_source_packages(
    app: tauri::AppHandle,
) -> CommandResult<Vec<SourcePackageEntryDto>> {
    run_blocking(move || {
        let state = tauri::Manager::state::<DesktopState>(&app);
        let users = package_users(&app, &state)?;
        Ok(list_packages(
            &packages_root(&app)?,
            &users,
            &CurrentInputs::resolve(&app),
        ))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Deletes a source package Aeria no longer needs and returns the updated
/// list. Runs as an Atlas job so no build can reuse the package meanwhile.
///
/// # Errors
///
/// Returns `sourcePackageInUse` for a package a project or the current build
/// uses, `sourcePackageNotFound`, or a typed storage error.
pub async fn delete_source_package(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    package_id: String,
) -> CommandResult<Vec<SourcePackageEntryDto>> {
    let job = state.start_atlas_job()?;
    let worker_app = app.clone();
    let result = run_blocking(move || {
        let state = tauri::Manager::state::<DesktopState>(&worker_app);
        let users = package_users(&worker_app, &state)?;
        let inputs = CurrentInputs::resolve(&worker_app);
        let root = packages_root(&worker_app)?;
        let cache_root = worker_app
            .aeria_cache_dir()
            .map_err(|error| CommandError::new("cachePath", error.to_string()))?;
        remove_package(&root, &cache_root, &users, &inputs, &package_id)?;
        Ok(list_packages(&root, &users, &inputs))
    })
    .await;
    state.finish_atlas_job(&job.id)?;
    result
}

#[tauri::command(rename_all = "camelCase")]
/// Tells whether a job would find a package without running Atlas: opening
/// or updating the project at `repositoryRoot`, or creating a project in
/// `sourceLanguage`.
///
/// # Errors
///
/// Returns a typed command error only when the worker fails; anything that
/// cannot be determined is `unknown`.
pub async fn source_availability(
    app: tauri::AppHandle,
    repository_root: Option<String>,
    source_language: Option<String>,
    opening: bool,
) -> CommandResult<SourceAvailabilityDto> {
    run_blocking(move || {
        let (language, content_id) = match repository_root.filter(|root| !root.trim().is_empty()) {
            Some(root) => match WorkspaceStore::new(root.trim()).read_metadata() {
                Ok(metadata) => (
                    metadata.source_language().to_owned(),
                    opening.then(|| metadata.source_content_id().to_owned()),
                ),
                Err(_) => return Ok(SourceAvailabilityDto::Unknown),
            },
            None => match source_language {
                Some(language) => (language, None),
                None => return Ok(SourceAvailabilityDto::Unknown),
            },
        };
        Ok(availability(
            &packages_root(&app)?,
            &CurrentInputs::resolve(&app),
            &language,
            content_id.as_deref(),
        ))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)]
/// Opens the source-package store folder in the system file manager.
///
/// # Errors
///
/// Returns a typed command error when the folder cannot be created or opened.
pub fn reveal_source_packages(app: tauri::AppHandle) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let root = packages_root(&app)?;
    fs::create_dir_all(&root)
        .map_err(|error| CommandError::new("atlasStorage", error.to_string()))?;
    app.opener()
        .open_path(root.to_string_lossy(), None::<&str>)
        .map_err(|error| CommandError::new("openPath", error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn ids(entries: &[SourcePackageEntryDto]) -> HashSet<String> {
        entries
            .iter()
            .map(|entry| entry.package_id.clone())
            .collect()
    }

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../crates/aeria-hsp/tests/fixtures")
            .join(name)
    }

    fn game_versions(version: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("ffxivgame".to_owned(), version.to_owned())])
    }

    fn inputs(atlas: &str, version: &str) -> CurrentInputs {
        CurrentInputs {
            atlas_sha256: Some(atlas.to_owned()),
            game_versions: Some(game_versions(version)),
        }
    }

    struct Store {
        _directory: tempfile::TempDir,
        root: PathBuf,
        cache: PathBuf,
    }

    fn store(packages: &[&str]) -> Store {
        let directory = tempfile::tempdir().expect("temp dir");
        let root = directory.path().join("store");
        let cache = directory.path().join("cache");
        fs::create_dir_all(&root).expect("store");
        for name in packages {
            fs::copy(fixture(name), root.join(name)).expect("package");
        }
        Store {
            _directory: directory,
            root,
            cache,
        }
    }

    fn language_of(path: &Path) -> String {
        aeria_hsp::read_manifest(path)
            .expect("manifest")
            .source
            .language
    }

    #[test]
    fn only_a_package_with_an_identical_build_record_is_reused() {
        let store = store(&["synthetic.hsp"]);
        let package_path = store.root.join("synthetic.hsp");
        let language = language_of(&package_path);
        let record = BuildRecord::new("a".repeat(64), language.clone(), game_versions("1"));
        let none = CurrentInputs::default();

        assert!(find_built_package(&store.root, &store.cache, &record).is_none());
        assert!(!list_packages(&store.root, &PackageUsers::default(), &none)[0].reusable);

        write_build_record(&package_path, &record).expect("record");
        assert!(find_built_package(&store.root, &store.cache, &record).is_some());
        assert!(list_packages(&store.root, &PackageUsers::default(), &none)[0].reusable);
        assert!(!store.root.join("synthetic.build.json.partial").exists());

        let other_atlas = BuildRecord::new("b".repeat(64), language.clone(), game_versions("1"));
        let other_game = BuildRecord::new("a".repeat(64), language, game_versions("2"));
        assert!(find_built_package(&store.root, &store.cache, &other_atlas).is_none());
        assert!(find_built_package(&store.root, &store.cache, &other_game).is_none());

        fs::write(build_record_path(&package_path), b"{").expect("broken record");
        assert!(find_built_package(&store.root, &store.cache, &record).is_none());
    }

    #[test]
    fn packages_in_use_or_current_are_kept() {
        let store = store(&["synthetic.hsp", "synthetic-v2.hsp"]);
        let current_path = store.root.join("synthetic-v2.hsp");
        let language = language_of(&current_path);
        write_build_record(
            &current_path,
            &BuildRecord::new("a".repeat(64), language, game_versions("2")),
        )
        .expect("record");
        let old_id = aeria_hsp::read_manifest(store.root.join("synthetic.hsp"))
            .expect("manifest")
            .package_id;

        let now = inputs(&"a".repeat(64), "2");
        let used = PackageUsers {
            projects: vec![(old_id.clone(), "C:/projects/one".to_owned())],
        };
        let listed = list_packages(&store.root, &used, &now);
        assert_eq!(ids(&listed).len(), 2);
        for entry in &listed {
            assert!(!entry.removable, "{}", entry.path);
        }
        let old = listed
            .iter()
            .find(|entry| entry.package_id == old_id)
            .expect("old");
        assert_eq!(old.used_by, vec!["C:/projects/one".to_owned()]);
        assert!(listed.iter().any(|entry| entry.current));
        assert_eq!(
            remove_package(&store.root, &store.cache, &used, &now, &old_id)
                .expect_err("in use")
                .code,
            "sourcePackageInUse"
        );

        // After a game update the old build is no longer current.
        let patched = inputs(&"a".repeat(64), "3");
        let unused = PackageUsers::default();
        assert!(
            list_packages(&store.root, &unused, &patched)
                .iter()
                .all(|entry| entry.removable)
        );
        SourcePackage::open(&current_path, &store.cache).expect("materialize");
        let current_id = aeria_hsp::read_manifest(&current_path)
            .expect("manifest")
            .package_id;
        remove_package(&store.root, &store.cache, &unused, &patched, &current_id).expect("remove");
        assert!(!current_path.exists());
        assert!(!build_record_path(&current_path).exists());
        assert_eq!(
            ids(&list_packages(&store.root, &unused, &patched)),
            HashSet::from([old_id])
        );
        assert_eq!(
            remove_package(
                &store.root,
                &store.cache,
                &unused,
                &patched,
                "sha256:missing"
            )
            .expect_err("missing")
            .code,
            "sourcePackageNotFound"
        );
    }

    #[test]
    fn availability_prefers_content_then_build_records() {
        let store = store(&["synthetic.hsp"]);
        let path = store.root.join("synthetic.hsp");
        let manifest = aeria_hsp::read_manifest(&path).expect("manifest");
        let language = manifest.source.language.clone();
        let now = inputs(&"a".repeat(64), "1");

        assert_eq!(
            availability(
                &store.root,
                &now,
                &language,
                Some(&manifest.source.content_id)
            ),
            SourceAvailabilityDto::Ready
        );
        assert_eq!(
            availability(&store.root, &now, &language, None),
            SourceAvailabilityDto::Build
        );
        assert_eq!(
            availability(&store.root, &CurrentInputs::default(), &language, None),
            SourceAvailabilityDto::Unknown
        );
        write_build_record(&path, &now.record(&language).expect("record")).expect("write");
        assert_eq!(
            availability(&store.root, &now, &language, None),
            SourceAvailabilityDto::Ready
        );
        assert_eq!(
            availability(&store.root, &inputs(&"b".repeat(64), "1"), &language, None),
            SourceAvailabilityDto::Build
        );
    }

    #[test]
    fn file_hashes_are_lowercase_sha256_hex() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("atlas");
        fs::write(&path, b"abc").expect("file");
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(file_sha256(&path).expect("hash"), expected);
        assert_eq!(cached_file_sha256(&path).expect("cached"), expected);
    }
}
