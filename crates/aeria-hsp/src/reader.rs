use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use aeria_hxs::HxsSnapshot;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::HspError;
use crate::guidance::{parse_and_validate, validate_relationships};
use crate::hash::{compute_package_id, is_sha256, sha256_reader, to_hash_string};
use crate::model::{HspComponentDescriptor, HspManifest, SourceGuidance, SourcePackage};
use crate::{MAX_HSP_GUIDANCE_BYTES, MAX_HSP_MANIFEST_BYTES};

const MANIFEST_PATH: &str = "manifest.json";
const SOURCE_PATH: &str = "source/source.hxs";
const GUIDANCE_PATH: &str = "guidance/source-guidance.json";

#[derive(Clone)]
struct VerifiedPackageCache {
    package_size: u64,
    package_hash: String,
    source_cache_path: PathBuf,
    source_cache_size: u64,
    source_cache_hash: String,
    manifest: HspManifest,
    guidance: SourceGuidance,
}

static VERIFIED_PACKAGE_CACHE: OnceLock<Mutex<HashMap<PathBuf, VerifiedPackageCache>>> =
    OnceLock::new();

/// Opens, validates, and materializes an HSP v1 source package.
///
/// # Errors
///
/// Returns [`HspError`] when archive, manifest, component, source, guidance,
/// relationship, or cache validation fails.
#[allow(clippy::too_many_lines)]
pub fn open(
    package_path: impl AsRef<Path>,
    cache_root: impl AsRef<Path>,
) -> Result<SourcePackage, HspError> {
    let trace = PerfTrace::new();
    let package_path = package_path.as_ref().to_path_buf();
    let cache_root = cache_root.as_ref().to_path_buf();
    if let Some(source_package) = try_open_verified_cache(&package_path, &cache_root) {
        trace.mark("hsp.verified-cache-fast-path");
        return Ok(source_package);
    }
    let file = File::open(&package_path).map_err(|source| HspError::Io {
        path: package_path.clone(),
        message: source.to_string(),
    })?;
    let mut archive = ZipArchive::new(file).map_err(|source| HspError::Archive {
        message: source.to_string(),
    })?;
    trace.mark("hsp.archive-open");
    validate_archive_names(&mut archive)?;
    let manifest_bytes = read_bounded_entry(&mut archive, MANIFEST_PATH, MAX_HSP_MANIFEST_BYTES)?;
    let manifest: HspManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|source| HspError::Manifest {
            message: format!("manifest JSON is invalid: {source}"),
        })?;
    validate_manifest(&manifest)?;

    let expected_entries = manifest
        .components
        .iter()
        .map(|component| component.path.clone())
        .chain(std::iter::once(MANIFEST_PATH.to_owned()))
        .collect::<HashSet<_>>();
    let archive_entries = (0..archive.len())
        .map(|index| {
            archive
                .by_index(index)
                .map(|entry| entry.name().to_owned())
                .map_err(|source| HspError::Archive {
                    message: source.to_string(),
                })
        })
        .collect::<Result<HashSet<_>, _>>()?;
    if archive_entries != expected_entries {
        return Err(HspError::Archive {
            message: "archive membership does not exactly match the manifest".to_owned(),
        });
    }
    trace.mark("hsp.manifest-and-membership");

    let source_component = required_component(&manifest, "sourceHxs", SOURCE_PATH)?;
    let guidance_component = required_component(&manifest, "sourceGuidance", GUIDANCE_PATH)?;
    let source_cache_path = cache_path(&cache_root, &manifest.source.snapshot_id)?;
    let (materialized_source, materialized) =
        materialize_source(&mut archive, source_component, &source_cache_path)?;
    let guidance_bytes = match read_guidance_component(&mut archive, guidance_component) {
        Ok(bytes) => bytes,
        Err(error) => {
            cleanup_materialized_source(materialized, &materialized_source);
            return Err(error);
        }
    };
    for component in &manifest.components {
        if component.id != source_component.id
            && component.id != guidance_component.id
            && let Err(error) = validate_component_entry(&mut archive, component, None, None)
        {
            cleanup_materialized_source(materialized, &materialized_source);
            return Err(error);
        }
    }
    trace.mark("hsp.component-verification");
    let source_snapshot = match HxsSnapshot::open(&materialized_source) {
        Ok(source) => source,
        Err(source) => {
            cleanup_materialized_source(materialized, &materialized_source);
            return Err(HspError::Hxs { source });
        }
    };
    trace.mark("hsp.hxs-snapshot-open");
    let guidance = match parse_and_validate(&guidance_bytes) {
        Ok(guidance) => guidance,
        Err(message) => {
            cleanup_materialized_source(materialized, &materialized_source);
            return Err(HspError::Guidance { message });
        }
    };
    let metadata = source_snapshot.metadata();
    if manifest.game_version != metadata.game_version
        || manifest.scope != metadata.scope
        || manifest.source.language != metadata.source_language
        || manifest.source.content_id != metadata.content_id
        || manifest.source.snapshot_id != metadata.snapshot_id
    {
        cleanup_materialized_source(materialized, &materialized_source);
        return Err(HspError::Relationship {
            message: "HSP manifest source identity does not match the embedded HXS".to_owned(),
        });
    }
    if let Err(source) = validate_relationships(&guidance, &source_snapshot) {
        cleanup_materialized_source(materialized, &materialized_source);
        return Err(HspError::Relationship {
            message: source.to_string(),
        });
    }
    trace.mark("hsp.guidance-relationships");

    let (package_size, package_hash) = digest_file(&package_path)?;
    let (source_cache_size, source_cache_hash) = digest_file(&materialized_source)?;
    remember_verified_cache(
        &package_path,
        VerifiedPackageCache {
            package_size,
            package_hash,
            source_cache_path: materialized_source.clone(),
            source_cache_size,
            source_cache_hash,
            manifest: manifest.clone(),
            guidance: guidance.clone(),
        },
    );
    trace.mark("hsp.verified-cache-recorded");

    Ok(SourcePackage::new(
        package_path,
        manifest,
        materialized_source,
        source_snapshot,
        guidance,
    ))
}

fn try_open_verified_cache(package_path: &Path, cache_root: &Path) -> Option<SourcePackage> {
    let key = package_cache_key(package_path);
    let cached = VERIFIED_PACKAGE_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?
        .get(&key)
        .cloned()?;
    let package_digest = digest_file(package_path).ok()?;
    if package_digest != (cached.package_size, cached.package_hash.clone()) {
        forget_verified_cache(&key);
        return None;
    }
    let source_digest = digest_file(&cached.source_cache_path).ok()?;
    if source_digest != (cached.source_cache_size, cached.source_cache_hash.clone()) {
        forget_verified_cache(&key);
        return None;
    }

    let file = File::open(package_path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;
    let manifest_bytes =
        read_bounded_entry(&mut archive, MANIFEST_PATH, MAX_HSP_MANIFEST_BYTES).ok()?;
    let manifest: HspManifest = serde_json::from_slice(&manifest_bytes).ok()?;
    if validate_manifest(&manifest).is_err() || manifest != cached.manifest {
        forget_verified_cache(&key);
        return None;
    }
    let guidance_component = required_component(&manifest, "sourceGuidance", GUIDANCE_PATH).ok()?;
    let guidance_bytes = read_guidance_component(&mut archive, guidance_component).ok()?;
    let guidance = parse_and_validate(&guidance_bytes).ok()?;
    if guidance != cached.guidance {
        forget_verified_cache(&key);
        return None;
    }
    let source_cache_path = cache_path(cache_root, &manifest.source.snapshot_id).ok()?;
    if source_cache_path != cached.source_cache_path {
        forget_verified_cache(&key);
        return None;
    }
    let Ok(source_snapshot) = HxsSnapshot::open_cached_verified(&source_cache_path) else {
        forget_verified_cache(&key);
        return None;
    };
    let metadata = source_snapshot.metadata();
    if manifest.game_version != metadata.game_version
        || manifest.scope != metadata.scope
        || manifest.source.language != metadata.source_language
        || manifest.source.content_id != metadata.content_id
        || manifest.source.snapshot_id != metadata.snapshot_id
    {
        forget_verified_cache(&key);
        return None;
    }

    Some(SourcePackage::new(
        package_path.to_owned(),
        manifest,
        source_cache_path,
        source_snapshot,
        guidance,
    ))
}

fn remember_verified_cache(package_path: &Path, cache: VerifiedPackageCache) {
    if let Ok(mut entries) = VERIFIED_PACKAGE_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        entries.insert(package_cache_key(package_path), cache);
    }
}

fn forget_verified_cache(package_key: &Path) {
    if let Ok(mut entries) = VERIFIED_PACKAGE_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        entries.remove(package_key);
    }
}

pub(crate) fn relocate_verified_cache(from: &Path, to: &Path) {
    if let Ok(mut entries) = VERIFIED_PACKAGE_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        && let Some(cache) = entries.remove(&package_cache_key(from))
    {
        entries.insert(package_cache_key(to), cache);
    }
}

fn package_cache_key(package_path: &Path) -> PathBuf {
    fs::canonicalize(package_path).unwrap_or_else(|_| package_path.to_owned())
}

fn digest_file(path: &Path) -> Result<(u64, String), HspError> {
    let mut file = File::open(path).map_err(|source| HspError::Io {
        path: path.to_owned(),
        message: source.to_string(),
    })?;
    sha256_reader(&mut file).map_err(|source| HspError::Io {
        path: path.to_owned(),
        message: source.to_string(),
    })
}

struct PerfTrace {
    enabled: bool,
    started: Instant,
}

impl PerfTrace {
    fn new() -> Self {
        Self {
            enabled: std::env::var("AERIA_PERF_TRACE").as_deref() == Ok("1"),
            started: Instant::now(),
        }
    }

    fn mark(&self, phase: &str) {
        if self.enabled {
            eprintln!(
                "[aeria-perf] {phase}: {} ms",
                self.started.elapsed().as_secs_f64() * 1_000.0
            );
        }
    }
}

fn cleanup_materialized_source(materialized: bool, path: &Path) {
    if materialized {
        let _ = fs::remove_file(path);
    }
}

fn validate_archive_names<R: Read + io::Seek>(archive: &mut ZipArchive<R>) -> Result<(), HspError> {
    let mut names = HashSet::new();
    let mut manifest_count = 0;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|source| HspError::Archive {
                message: source.to_string(),
            })?;
        let name = entry.name();
        if !names.insert(name.to_owned()) {
            return Err(HspError::Archive {
                message: "archive contains duplicate entry names".to_owned(),
            });
        }
        if name == MANIFEST_PATH {
            manifest_count += 1;
        }
        if name.ends_with('/') {
            return Err(HspError::Archive {
                message: "archive contains a directory entry".to_owned(),
            });
        }
    }
    if manifest_count != 1 {
        return Err(HspError::Archive {
            message: "archive must contain exactly one manifest.json".to_owned(),
        });
    }
    Ok(())
}

fn validate_manifest(manifest: &HspManifest) -> Result<(), HspError> {
    if manifest.format_version != 1
        || manifest.game_version.trim().is_empty()
        || manifest.scope.trim().is_empty()
        || !is_canonical_language(&manifest.source.language)
        || !is_sha256(&manifest.source.content_id)
        || !is_sha256(&manifest.source.snapshot_id)
        || !is_sha256(&manifest.package_id)
    {
        return Err(HspError::Manifest {
            message: "manifest metadata is invalid".to_owned(),
        });
    }

    let mut ids = HashSet::new();
    let mut paths = HashSet::new();
    let mut source_count = 0;
    let mut guidance_count = 0;
    for component in &manifest.components {
        validate_component_descriptor(component, &mut ids, &mut paths)?;
        if component.required && component.kind != "sourceHxs" && component.kind != "sourceGuidance"
        {
            return Err(HspError::Manifest {
                message: format!(
                    "required component kind {:?} is unsupported",
                    component.kind
                ),
            });
        }
        if component.required && component.kind == "sourceHxs" {
            source_count += 1;
            if component.format_version != 1 || component.path != SOURCE_PATH {
                return Err(HspError::Manifest {
                    message: "required sourceHxs descriptor is not the HSP v1 source".to_owned(),
                });
            }
        }
        if component.required && component.kind == "sourceGuidance" {
            guidance_count += 1;
            if component.format_version != 1 || component.path != GUIDANCE_PATH {
                return Err(HspError::Manifest {
                    message: "required sourceGuidance descriptor is not the HSP v1 guidance"
                        .to_owned(),
                });
            }
        }
    }
    if source_count != 1 || guidance_count != 1 {
        return Err(HspError::Manifest {
            message: "HSP v1 requires one required sourceHxs and one required sourceGuidance"
                .to_owned(),
        });
    }
    let expected =
        compute_package_id(manifest).map_err(|message| HspError::Manifest { message })?;
    if expected != manifest.package_id {
        return Err(HspError::Manifest {
            message: "packageId does not match the canonical logical manifest".to_owned(),
        });
    }
    Ok(())
}

fn validate_component_descriptor(
    component: &HspComponentDescriptor,
    ids: &mut HashSet<String>,
    paths: &mut HashSet<String>,
) -> Result<(), HspError> {
    if component.id.trim().is_empty()
        || component.kind.trim().is_empty()
        || component.format_version == 0
        || component.size < 0
        || !is_sha256(&component.sha256)
        || !ids.insert(component.id.clone())
        || !paths.insert(component.path.clone())
        || !is_safe_relative_path(&component.path)
    {
        return Err(HspError::Manifest {
            message: "component descriptors are invalid".to_owned(),
        });
    }
    Ok(())
}

fn required_component<'a>(
    manifest: &'a HspManifest,
    kind: &str,
    path: &str,
) -> Result<&'a HspComponentDescriptor, HspError> {
    manifest
        .components
        .iter()
        .find(|component| component.required && component.kind == kind && component.path == path)
        .ok_or_else(|| HspError::Manifest {
            message: format!("required {kind} component {path:?} is missing"),
        })
}

fn read_bounded_entry<R: Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
    maximum_size: u64,
) -> Result<Vec<u8>, HspError> {
    let mut entry = archive.by_name(path).map_err(|source| HspError::Archive {
        message: source.to_string(),
    })?;
    let declared_size = entry.size();
    if declared_size > maximum_size {
        return Err(HspError::Manifest {
            message: format!("{path} exceeds its {maximum_size}-byte hard limit"),
        });
    }
    let capacity = usize::try_from(declared_size).map_err(|_| HspError::Manifest {
        message: format!("{path} declared size cannot fit in memory"),
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    stream_bounded(&mut entry, declared_size, maximum_size, Some(&mut bytes)).map_err(|error| {
        HspError::Manifest {
            message: format!(
                "failed to read bounded {path}: {}",
                stream_error_message(&error)
            ),
        }
    })?;
    Ok(bytes)
}

fn read_guidance_component<R: Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    component: &HspComponentDescriptor,
) -> Result<Vec<u8>, HspError> {
    let mut entry = archive
        .by_name(&component.path)
        .map_err(|source| HspError::Archive {
            message: source.to_string(),
        })?;
    let expected_size = validate_declared_component_size(entry.size(), component)?;
    if expected_size > MAX_HSP_GUIDANCE_BYTES {
        return Err(HspError::Component {
            id: component.id.clone(),
            message: format!(
                "source guidance exceeds its {MAX_HSP_GUIDANCE_BYTES}-byte hard limit"
            ),
        });
    }
    let capacity = usize::try_from(expected_size).map_err(|_| HspError::Component {
        id: component.id.clone(),
        message: "source guidance declared size cannot fit in memory".to_owned(),
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    let digest = stream_bounded(
        &mut entry,
        expected_size,
        MAX_HSP_GUIDANCE_BYTES,
        Some(&mut bytes),
    )
    .map_err(|error| component_stream_error(component, error, None))?;
    verify_component_digest(component, digest.hash)?;
    Ok(bytes)
}

fn validate_component_entry<R: Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    component: &HspComponentDescriptor,
    output: Option<&mut dyn Write>,
    output_path: Option<&Path>,
) -> Result<(), HspError> {
    let mut entry = archive
        .by_name(&component.path)
        .map_err(|source| HspError::Archive {
            message: source.to_string(),
        })?;
    let expected_size = validate_declared_component_size(entry.size(), component)?;
    let digest =
        validate_component_stream(&mut entry, component, expected_size, output, output_path)?;
    verify_component_digest(component, digest.hash)
}

fn validate_declared_component_size(
    declared_size: u64,
    component: &HspComponentDescriptor,
) -> Result<u64, HspError> {
    let expected_size = u64::try_from(component.size).map_err(|_| HspError::Component {
        id: component.id.clone(),
        message: "component size is out of range".to_owned(),
    })?;
    if declared_size != expected_size {
        return Err(HspError::Component {
            id: component.id.clone(),
            message: format!(
                "ZIP entry declares {declared_size} bytes but manifest requires {expected_size}"
            ),
        });
    }
    Ok(expected_size)
}

fn verify_component_digest(
    component: &HspComponentDescriptor,
    actual_hash: [u8; 32],
) -> Result<(), HspError> {
    if to_hash_string(actual_hash) != component.sha256 {
        return Err(HspError::Component {
            id: component.id.clone(),
            message: "component SHA-256 does not match its manifest".to_owned(),
        });
    }
    Ok(())
}

fn materialize_source<R: Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    component: &HspComponentDescriptor,
    destination: &Path,
) -> Result<(PathBuf, bool), HspError> {
    let parent = destination.parent().ok_or_else(|| HspError::Cache {
        path: destination.to_path_buf(),
        message: "cache path has no parent directory".to_owned(),
    })?;
    fs::create_dir_all(parent).map_err(|source| HspError::Cache {
        path: parent.to_path_buf(),
        message: source.to_string(),
    })?;

    let reusable = destination.is_file() && file_matches(destination, component)?;
    let mut entry = archive
        .by_name(&component.path)
        .map_err(|source| HspError::Archive {
            message: source.to_string(),
        })?;
    let expected_size = validate_declared_component_size(entry.size(), component)?;
    if reusable {
        let digest = validate_component_stream(&mut entry, component, expected_size, None, None)?;
        verify_component_digest(component, digest.hash)?;
        return Ok((destination.to_path_buf(), false));
    }

    let temporary = temporary_path(parent);
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| HspError::Cache {
            path: temporary.clone(),
            message: source.to_string(),
        })?;
    let validation = validate_component_stream(
        &mut entry,
        component,
        expected_size,
        Some(&mut output),
        Some(&temporary),
    )
    .and_then(|digest| verify_component_digest(component, digest.hash).map(|()| digest));
    let close_result = output.flush();
    drop(output);
    if let Err(error) = validation {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(source) = close_result {
        let _ = fs::remove_file(&temporary);
        return Err(HspError::Cache {
            path: temporary.clone(),
            message: source.to_string(),
        });
    }
    if let Err(error) = publish_atomically(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok((destination.to_path_buf(), true))
}

fn validate_component_stream(
    input: &mut impl Read,
    component: &HspComponentDescriptor,
    expected_size: u64,
    output: Option<&mut dyn Write>,
    output_path: Option<&Path>,
) -> Result<StreamDigest, HspError> {
    stream_bounded(input, expected_size, expected_size, output)
        .map_err(|error| component_stream_error(component, error, output_path))
}

#[derive(Debug)]
struct StreamDigest {
    hash: [u8; 32],
}

#[derive(Debug)]
enum StreamError {
    Read(io::Error),
    Write(io::Error),
    Overflow { expected: u64, actual: u64 },
    Short { expected: u64, actual: u64 },
}

fn stream_bounded(
    input: &mut impl Read,
    expected_size: u64,
    maximum_size: u64,
    mut output: Option<&mut dyn Write>,
) -> Result<StreamDigest, StreamError> {
    if expected_size > maximum_size {
        return Err(StreamError::Overflow {
            expected: maximum_size,
            actual: expected_size,
        });
    }

    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let remaining = expected_size.saturating_sub(size);
        let read_size = usize::try_from(remaining.saturating_add(1))
            .unwrap_or(buffer.len())
            .clamp(1, buffer.len());
        let read = input
            .read(&mut buffer[..read_size])
            .map_err(StreamError::Read)?;
        if read == 0 {
            break;
        }
        let next_size = size
            .checked_add(u64::try_from(read).expect("buffer length fits u64"))
            .ok_or(StreamError::Overflow {
                expected: expected_size,
                actual: u64::MAX,
            })?;
        if next_size > expected_size {
            return Err(StreamError::Overflow {
                expected: expected_size,
                actual: next_size,
            });
        }
        hasher.update(&buffer[..read]);
        if let Some(output) = output.as_deref_mut() {
            output
                .write_all(&buffer[..read])
                .map_err(StreamError::Write)?;
        }
        size = next_size;
    }

    if size != expected_size {
        return Err(StreamError::Short {
            expected: expected_size,
            actual: size,
        });
    }
    Ok(StreamDigest {
        hash: hasher.finalize().into(),
    })
}

fn stream_error_message(error: &StreamError) -> String {
    match error {
        StreamError::Read(source) | StreamError::Write(source) => source.to_string(),
        StreamError::Overflow { expected, actual } => {
            format!("stream produced {actual} bytes, exceeding {expected}")
        }
        StreamError::Short { expected, actual } => {
            format!("stream ended at {actual} bytes, expected {expected}")
        }
    }
}

fn component_stream_error(
    component: &HspComponentDescriptor,
    error: StreamError,
    output_path: Option<&Path>,
) -> HspError {
    match error {
        StreamError::Write(source) => HspError::Cache {
            path: output_path.map_or_else(|| PathBuf::from(&component.path), Path::to_path_buf),
            message: source.to_string(),
        },
        error => HspError::Component {
            id: component.id.clone(),
            message: stream_error_message(&error),
        },
    }
}

fn file_matches(path: &Path, component: &HspComponentDescriptor) -> Result<bool, HspError> {
    let metadata = fs::metadata(path).map_err(|source| HspError::Cache {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    if metadata.len() != u64::try_from(component.size).unwrap_or(u64::MAX) {
        return Ok(false);
    }
    let mut file = File::open(path).map_err(|source| HspError::Cache {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    let (_, hash) = sha256_reader(&mut file).map_err(|source| HspError::Cache {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    Ok(hash == component.sha256)
}

fn publish_atomically(temporary: &Path, destination: &Path) -> Result<(), HspError> {
    let backup = destination.with_extension("hxs.previous");
    let had_destination = destination.exists();
    if had_destination {
        let _ = fs::remove_file(&backup);
        fs::rename(destination, &backup).map_err(|source| HspError::Cache {
            path: destination.to_path_buf(),
            message: source.to_string(),
        })?;
    }
    if let Err(source) = fs::rename(temporary, destination) {
        if had_destination {
            let _ = fs::rename(&backup, destination);
        }
        return Err(HspError::Cache {
            path: destination.to_path_buf(),
            message: source.to_string(),
        });
    }
    if had_destination {
        let _ = fs::remove_file(backup);
    }
    Ok(())
}

fn cache_path(cache_root: &Path, snapshot_id: &str) -> Result<PathBuf, HspError> {
    let digest = snapshot_id
        .strip_prefix("sha256:")
        .ok_or_else(|| HspError::Manifest {
            message: "snapshotId is not a canonical SHA-256 identifier".to_owned(),
        })?;
    Ok(cache_root.join("hxs").join(digest).join("source.hxs"))
}

fn temporary_path(parent: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    parent.join(format!(
        "source.hxs.{:x}.{}.partial",
        nonce,
        std::process::id()
    ))
}

fn is_safe_relative_path(path: &str) -> bool {
    if path.trim().is_empty()
        || path.contains('\\')
        || path.contains("//")
        || path.starts_with('/')
        || path.len() >= 2 && path.as_bytes()[1] == b':'
        || Path::new(path).is_absolute()
    {
        return false;
    }
    path.split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn is_canonical_language(language: &str) -> bool {
    matches!(
        language,
        "en" | "ja" | "de" | "fr" | "zh-cn" | "zh-tw" | "ko"
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn bounded_component_stream_aborts_on_the_first_byte_over_expected_size() {
        let component = HspComponentDescriptor {
            id: "test".to_owned(),
            kind: "future".to_owned(),
            format_version: 1,
            required: false,
            path: "optional/test.bin".to_owned(),
            size: 3,
            sha256: "sha256:".to_owned() + &"0".repeat(64),
        };
        let mut input = Cursor::new([1_u8, 2, 3, 4]);
        let error = validate_component_stream(&mut input, &component, 3, None, None)
            .expect_err("oversized stream");

        assert!(error.to_string().contains("exceeding 3"));
    }
}
