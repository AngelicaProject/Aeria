use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use aeria_hxs::HxsSnapshot;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::HspError;
use crate::guidance::{parse_and_validate, validate_relationships};
use crate::hash::{compute_package_id, is_sha256, sha256_reader, to_hash_string};
use crate::model::{HspComponentDescriptor, HspManifest, SourcePackage};

const MANIFEST_PATH: &str = "manifest.json";
const SOURCE_PATH: &str = "source/source.hxs";
const GUIDANCE_PATH: &str = "guidance/source-guidance.json";

/// Opens, validates, and materializes an HSP v1 source package.
///
/// # Errors
///
/// Returns [`HspError`] when archive, manifest, component, source, guidance,
/// relationship, or cache validation fails.
pub fn open(
    package_path: impl AsRef<Path>,
    cache_root: impl AsRef<Path>,
) -> Result<SourcePackage, HspError> {
    let package_path = package_path.as_ref().to_path_buf();
    let cache_root = cache_root.as_ref().to_path_buf();
    let file = File::open(&package_path).map_err(|source| HspError::Io {
        path: package_path.clone(),
        message: source.to_string(),
    })?;
    let mut archive = ZipArchive::new(file).map_err(|source| HspError::Archive {
        message: source.to_string(),
    })?;
    validate_archive_names(&mut archive)?;
    let manifest_bytes = read_entry(&mut archive, MANIFEST_PATH)?;
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

    let source_component = required_component(&manifest, "sourceHxs", SOURCE_PATH)?;
    let guidance_component = required_component(&manifest, "sourceGuidance", GUIDANCE_PATH)?;
    let source_cache_path = cache_path(&cache_root, &manifest.source.snapshot_id)?;
    let (materialized_source, materialized) =
        materialize_source(&mut archive, source_component, &source_cache_path)?;
    let guidance_bytes = match read_and_validate_component(&mut archive, guidance_component) {
        Ok(bytes) => bytes,
        Err(error) => {
            cleanup_materialized_source(materialized, &materialized_source);
            return Err(error);
        }
    };
    for component in &manifest.components {
        if component.id != source_component.id && component.id != guidance_component.id {
            read_and_validate_component(&mut archive, component)?;
        }
    }
    let source_snapshot = match HxsSnapshot::open(&materialized_source) {
        Ok(source) => source,
        Err(source) => {
            cleanup_materialized_source(materialized, &materialized_source);
            return Err(HspError::Hxs { source });
        }
    };
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

    Ok(SourcePackage::new(
        package_path,
        manifest,
        materialized_source,
        source_snapshot,
        guidance,
    ))
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

fn read_entry<R: Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
) -> Result<Vec<u8>, HspError> {
    let mut entry = archive.by_name(path).map_err(|source| HspError::Archive {
        message: source.to_string(),
    })?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(|source| HspError::Io {
            path: PathBuf::from(path),
            message: source.to_string(),
        })?;
    Ok(bytes)
}

fn read_and_validate_component<R: Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    component: &HspComponentDescriptor,
) -> Result<Vec<u8>, HspError> {
    let bytes = read_entry(archive, &component.path)?;
    let actual_size = i64::try_from(bytes.len()).map_err(|_| HspError::Component {
        id: component.id.clone(),
        message: "component is too large".to_owned(),
    })?;
    let actual_hash = to_hash_string(Sha256::digest(&bytes).into());
    if actual_size != component.size || actual_hash != component.sha256 {
        return Err(HspError::Component {
            id: component.id.clone(),
            message: "component size or SHA-256 does not match its manifest".to_owned(),
        });
    }
    Ok(bytes)
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
    if reusable {
        validate_stream(&mut entry, component, None)?;
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
    let validation = validate_stream(&mut entry, component, Some(&mut output));
    let close_result = output.flush();
    drop(output);
    if let Err(error) = validation {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    close_result.map_err(|source| HspError::Cache {
        path: temporary.clone(),
        message: source.to_string(),
    })?;
    publish_atomically(&temporary, destination)?;
    Ok((destination.to_path_buf(), true))
}

fn validate_stream(
    input: &mut impl Read,
    component: &HspComponentDescriptor,
    mut output: Option<&mut File>,
) -> Result<(), HspError> {
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|source| HspError::Component {
                id: component.id.clone(),
                message: source.to_string(),
            })?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(u64::try_from(read).expect("buffer length fits u64"))
            .ok_or_else(|| HspError::Component {
                id: component.id.clone(),
                message: "component size overflow".to_owned(),
            })?;
        hasher.update(&buffer[..read]);
        if let Some(output) = output.as_deref_mut() {
            output
                .write_all(&buffer[..read])
                .map_err(|source| HspError::Cache {
                    path: PathBuf::from(SOURCE_PATH),
                    message: source.to_string(),
                })?;
        }
    }
    let actual_hash: [u8; 32] = hasher.finalize().into();
    if i64::try_from(size).ok() != Some(component.size)
        || to_hash_string(actual_hash) != component.sha256
    {
        return Err(HspError::Component {
            id: component.id.clone(),
            message: "component size or SHA-256 does not match its manifest".to_owned(),
        });
    }
    Ok(())
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
