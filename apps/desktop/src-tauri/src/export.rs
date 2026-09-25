//! Harmonia pack export and publishing.
//!
//! Pack identity lives in the committed `aeria-pack.json`; the signing key in
//! the OS credential store; releases are created on GitHub with the Git
//! credential of the repository's `origin`. See
//! `docs/architecture/export.md`.

use std::path::{Path, PathBuf};

use aeria_atlas::{AtlasEncodeRunner, CancellationToken};
use aeria_export::{
    BuiltPack, Channel, ContentPolicy, ExportError, ExportReport, FeedDownload, PACK_SETTINGS_FILE,
    PackManifest, PackSettings, Publisher, StringEncoder, collect_project, compress_for_transport,
    feed_entry, pack_source, write_file_atomically, write_pack_with_fonts,
};
use aeria_fonts::{FontSection, FontSettings};
use aeria_git::{GitExecutable, GitRepository, HostCredential};
use aeria_publish::{
    GITHUB_HOST, GitHubClient, GitHubRepository, KeyError, KeyringSigningKeyStore, PublishError,
    RELEASE_TAG_PREFIX, ReleaseAsset, ReleaseRequest, SigningKeyStore, SigningSecret,
    WorkflowState, feed_workflow_state, install_feed_workflow, pack_asset_name,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::commands::{resolve_atlas_executable, run_blocking};
use crate::error::CommandError;
use crate::paths::AeriaPaths;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

const FEED_ENTRY_ASSET: &str = "feed-entry.json";
const ORIGIN: &str = "origin";

/// Everything the export screen shows before any network request.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOverviewDto {
    pub settings: Option<PackSettingsDto>,
    /// Why `aeria-pack.json` exists but cannot be used.
    pub settings_error: Option<String>,
    pub key: SigningKeyDto,
    pub project: ExportProjectDto,
    /// The GitHub repository of `origin`, when it is one.
    pub github: Option<GitHubTargetDto>,
    pub workflow: WorkflowStateDto,
    /// The main branch on GitHub (as of the last fetch) has the workflow
    /// this Aeria installs. Release events run the workflow from there, so
    /// without it published releases never reach the feed.
    pub workflow_on_github: bool,
    /// The main branch the workflow must reach.
    pub main_branch: Option<String>,
    /// Highest `harmonia/<n>` release tag in the local repository: the last
    /// published release Git knows of without asking GitHub.
    pub latest_release_tag: Option<u64>,
    /// `aeria-fonts.json` exists, so the pack will carry font glyphs.
    pub fonts_configured: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackSettingsDto {
    pub pack_id: String,
    pub title: String,
    pub publisher_name: String,
    pub publisher_url: Option<String>,
    pub license: Option<String>,
    pub min_harmonia: String,
    /// Only reported; changed through the key commands.
    #[serde(default, skip_deserializing)]
    pub signing_key_fingerprint: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyStateDto {
    Stored,
    Missing,
    /// The OS secret store could not be read.
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SigningKeyDto {
    pub state: KeyStateDto,
    /// Fingerprint of the key stored on this computer.
    pub fingerprint: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProjectDto {
    pub source_language: String,
    pub target_language: String,
    pub game_version: String,
    pub commit: Option<String>,
    /// Translation data or `aeria-pack.json` has uncommitted changes.
    pub uncommitted: bool,
    /// `aeria-pack.json` itself has uncommitted changes.
    pub settings_uncommitted: bool,
    /// What the export needs committed but is not: `translations`, `pack`,
    /// `fonts`.
    pub uncommitted_parts: Vec<&'static str>,
    pub upstream: Option<String>,
    /// Commits not pushed to the upstream branch.
    pub ahead: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubTargetDto {
    pub owner: String,
    pub name: String,
    pub homepage: String,
    pub feed_url: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowStateDto {
    Missing,
    Current,
    Different,
}

/// Parameters of one release.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseInputDto {
    pub sequence: u64,
    pub version: String,
    pub channel: ChannelDto,
    pub content_policy: ContentPolicyDto,
    #[serde(default)]
    pub changelog: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelDto {
    Stable,
    Testing,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentPolicyDto {
    Reviewed,
    All,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReportDto {
    pub exported: u64,
    pub skipped_detached: u64,
    pub skipped_untranslated: u64,
    pub skipped_unreviewed: u64,
    pub skipped_without_raw_hash: u64,
    pub sheets: u64,
    pub strings: u64,
    pub pack_hash: String,
    /// Game font sizes and glyphs in the `FONTS` section; zero without one.
    pub font_targets: u64,
    pub font_glyphs: u64,
    /// Fingerprint of the signing key, or `None` for an unsigned pack.
    pub signed_by: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalExportDto {
    pub path: String,
    pub report: ExportReportDto,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedReleaseDto {
    pub sequence: u64,
    pub release_url: String,
    pub feed_url: String,
    pub report: ExportReportDto,
}

fn export_error(code: &'static str, message: impl Into<String>) -> CommandError {
    CommandError::new(code, message)
}

impl From<ExportError> for CommandError {
    fn from(error: ExportError) -> Self {
        let code = match &error {
            ExportError::Settings(_) => "exportSettingsInvalid",
            ExportError::Fonts(_) => "exportFontsFailed",
            ExportError::Io(_) => "exportIo",
            _ => "exportFailed",
        };
        Self::new(code, error.to_string())
    }
}

impl From<KeyError> for CommandError {
    fn from(error: KeyError) -> Self {
        let code = match &error {
            KeyError::Unavailable { .. } => "exportKeyStoreUnavailable",
            KeyError::Failed { .. } | KeyError::Random { .. } => "exportKeyStore",
            KeyError::InvalidBackup { .. } => "exportKeyBackupInvalid",
        };
        Self::new(code, error.to_string())
    }
}

impl From<PublishError> for CommandError {
    fn from(error: PublishError) -> Self {
        let code = match &error {
            PublishError::Network { .. } => "githubNetwork",
            PublishError::Unauthorized { .. } => "githubUnauthorized",
            PublishError::Forbidden { .. } => "githubForbidden",
            PublishError::NotFound { .. } => "githubNotFound",
            PublishError::Rejected { .. } => "githubRejected",
            PublishError::InvalidResponse { .. } => "githubInvalidResponse",
        };
        Self::new(code, error.to_string())
    }
}

impl From<&PackSettings> for PackSettingsDto {
    fn from(settings: &PackSettings) -> Self {
        Self {
            pack_id: settings.pack_id.clone(),
            title: settings.title.clone(),
            publisher_name: settings.publisher.name.clone(),
            publisher_url: settings.publisher.url.clone(),
            license: settings.license.clone(),
            min_harmonia: settings.min_harmonia.clone(),
            signing_key_fingerprint: settings.signing_key_fingerprint.clone(),
        }
    }
}

fn optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn project_root(state: &DesktopState) -> CommandResult<PathBuf> {
    let project = state.lock_project()?;
    let session = project.as_ref().ok_or_else(CommandError::no_project)?;
    Ok(session.repository_root().to_owned())
}

fn load_settings(root: &Path) -> CommandResult<PackSettings> {
    PackSettings::load(root)?.ok_or_else(|| {
        export_error(
            "exportSettingsMissing",
            format!("{PACK_SETTINGS_FILE} does not exist; save the pack settings first"),
        )
    })
}

fn github_target(repository: &GitRepository) -> Option<GitHubRepository> {
    repository
        .remotes()
        .ok()?
        .into_iter()
        .find(|remote| remote.name == ORIGIN)
        .and_then(|remote| GitHubRepository::from_remote_url(&remote.url))
}

fn overview(state: &DesktopState, store: &dyn SigningKeyStore) -> CommandResult<ExportOverviewDto> {
    let (root, source, target_language) = {
        let project = state.lock_project()?;
        let session = project.as_ref().ok_or_else(CommandError::no_project)?;
        (
            session.repository_root().to_owned(),
            session.source().metadata(),
            session.workspace().metadata().target_language().to_owned(),
        )
    };
    let (settings, settings_error) = match PackSettings::load(&root) {
        Ok(settings) => (settings, None),
        Err(error) => (None, Some(error.to_string())),
    };
    let key = match settings
        .as_ref()
        .map(|settings| store.get(&settings.pack_id))
    {
        None | Some(Ok(None)) => SigningKeyDto {
            state: KeyStateDto::Missing,
            fingerprint: None,
        },
        Some(Ok(Some(secret))) => SigningKeyDto {
            state: KeyStateDto::Stored,
            fingerprint: Some(secret.fingerprint()),
        },
        Some(Err(_)) => SigningKeyDto {
            state: KeyStateDto::Unavailable,
            fingerprint: None,
        },
    };
    let repository = GitRepository::open(&root, state.git()).ok();
    let status = repository
        .as_ref()
        .and_then(|repository| repository.status().ok());
    let project = ExportProjectDto {
        source_language: source.source_language,
        target_language,
        game_version: source.game_version,
        commit: status.as_ref().and_then(|status| status.head.clone()),
        uncommitted: status.as_ref().is_some_and(has_export_changes),
        uncommitted_parts: status.as_ref().map_or_else(Vec::new, uncommitted_parts),
        settings_uncommitted: status.as_ref().is_some_and(|status| {
            status
                .files
                .iter()
                .any(|file| file.path == PACK_SETTINGS_FILE)
        }),
        upstream: status.as_ref().and_then(|status| status.upstream.clone()),
        ahead: status.as_ref().map_or(0, |status| status.ahead),
    };
    let github = repository
        .as_ref()
        .and_then(github_target)
        .map(|target| GitHubTargetDto {
            homepage: target.homepage(),
            feed_url: target.feed_url(),
            owner: target.owner,
            name: target.name,
        });
    let workflow = match feed_workflow_state(&root) {
        Ok(WorkflowState::Current) => WorkflowStateDto::Current,
        Ok(WorkflowState::Different) => WorkflowStateDto::Different,
        Ok(WorkflowState::Missing) | Err(_) => WorkflowStateDto::Missing,
    };
    let main_branch = repository
        .as_ref()
        .and_then(|repository| repository.main_branch().ok().flatten());
    let workflow_on_github = match (&repository, &main_branch) {
        (Some(repository), Some(main)) => repository
            .remote_file(&format!("{ORIGIN}/{main}"), aeria_git::FEED_WORKFLOW_FILE)
            .ok()
            .flatten()
            .is_some_and(|bytes| {
                String::from_utf8_lossy(&bytes).replace("\r\n", "\n")
                    == aeria_publish::FEED_WORKFLOW.replace("\r\n", "\n")
            }),
        _ => false,
    };
    let latest_release_tag = repository
        .as_ref()
        .and_then(|repository| repository.tags_with_prefix(RELEASE_TAG_PREFIX).ok())
        .and_then(|tags| {
            tags.iter()
                .filter_map(|tag| tag.strip_prefix(RELEASE_TAG_PREFIX)?.parse::<u64>().ok())
                .max()
        });
    Ok(ExportOverviewDto {
        latest_release_tag,
        fonts_configured: root.join(aeria_fonts::FONT_SETTINGS_FILE).exists(),
        settings: settings.as_ref().map(PackSettingsDto::from),
        settings_error,
        key,
        project,
        github,
        workflow,
        workflow_on_github,
        main_branch,
    })
}

fn uncommitted_parts(status: &aeria_git::RepositoryStatus) -> Vec<&'static str> {
    let mut parts = Vec::new();
    if status.has_translation_changes() {
        parts.push("translations");
    }
    if status
        .files
        .iter()
        .any(|file| file.path == PACK_SETTINGS_FILE)
    {
        parts.push("pack");
    }
    if status
        .files
        .iter()
        .any(|file| crate::fonts::is_font_path(&file.path))
    {
        parts.push("fonts");
    }
    parts
}

fn has_export_changes(status: &aeria_git::RepositoryStatus) -> bool {
    status.has_translation_changes()
        || status
            .files
            .iter()
            .any(|file| file.path == PACK_SETTINGS_FILE || crate::fonts::is_font_path(&file.path))
}

fn save_settings(root: &Path, input: PackSettingsDto) -> CommandResult<()> {
    let existing = PackSettings::load(root).ok().flatten();
    let settings = PackSettings {
        pack_id: input.pack_id.trim().to_owned(),
        title: input.title.trim().to_owned(),
        publisher: Publisher {
            name: input.publisher_name.trim().to_owned(),
            url: optional(input.publisher_url),
        },
        license: optional(input.license),
        min_harmonia: input.min_harmonia.trim().to_owned(),
        // The key belongs to the pack identity; a new packId starts without one.
        signing_key_fingerprint: existing
            .filter(|existing| existing.pack_id == input.pack_id.trim())
            .and_then(|existing| existing.signing_key_fingerprint),
    };
    settings.save(root)?;
    Ok(())
}

/// Stores `secret` as the pack's key and records its fingerprint in the
/// settings when the project has none yet.
fn adopt_key(
    root: &Path,
    store: &dyn SigningKeyStore,
    secret: &SigningSecret,
    replace_project_key: bool,
) -> CommandResult<()> {
    let mut settings = load_settings(root)?;
    let fingerprint = secret.fingerprint();
    match &settings.signing_key_fingerprint {
        Some(current) if *current != fingerprint && !replace_project_key => {
            return Err(export_error(
                "exportKeyMismatch",
                format!(
                    "the project is signed with key {current}; import that key's backup instead"
                ),
            ));
        }
        _ => {}
    }
    store.set(&settings.pack_id, secret)?;
    if settings.signing_key_fingerprint.as_deref() != Some(fingerprint.as_str()) {
        settings.signing_key_fingerprint = Some(fingerprint);
        settings.save(root)?;
    }
    Ok(())
}

/// The key that signs this project's packs. It must be the key recorded in
/// the settings.
fn project_signer(
    settings: &PackSettings,
    store: &dyn SigningKeyStore,
) -> CommandResult<SigningSecret> {
    let secret = store.get(&settings.pack_id)?.ok_or_else(|| {
        export_error(
            "exportKeyMissing",
            "no signing key for this pack is stored on this computer",
        )
    })?;
    if settings.signing_key_fingerprint.as_deref() != Some(secret.fingerprint().as_str()) {
        return Err(export_error(
            "exportKeyMismatch",
            "the signing key on this computer is not the key recorded in the pack settings",
        ));
    }
    Ok(secret)
}

struct AtlasEncoder {
    runner: AtlasEncodeRunner,
    cancellation: CancellationToken,
}

impl StringEncoder for AtlasEncoder {
    fn encode(&mut self, macros: &[&str]) -> Result<Vec<Result<Vec<u8>, String>>, String> {
        let results = self
            .runner
            .encode(macros, &self.cancellation)
            .map_err(|error| error.to_string())?;
        Ok(results
            .into_iter()
            .map(|result| {
                result.map_err(|rejection| format!("{}: {}", rejection.code, rejection.message))
            })
            .collect())
    }
}

struct BuiltRelease {
    manifest: PackManifest,
    fonts: Option<FontSection>,
    pack: BuiltPack,
    report: ExportReport,
    signed_by: Option<String>,
}

/// Builds a pack of the open project at its committed `HEAD`.
fn build(
    app: &tauri::AppHandle,
    release: &ReleaseInputDto,
    settings: &PackSettings,
    signer: Option<&SigningSecret>,
) -> CommandResult<BuiltRelease> {
    let version = release.version.trim();
    let atlas = AtlasEncodeRunner::new(
        resolve_atlas_executable(app)?,
        app.aeria_cache_dir()
            .map_err(|error| CommandError::internal_state(error.to_string()))?
            .join("atlas-encode"),
    );
    let exporter_atlas = atlas.version()?;
    let state = app.state::<DesktopState>();
    // The project stays locked so the workspace cannot change between the
    // commit check and the end of collection.
    let project = state.lock_project()?;
    let session = project.as_ref().ok_or_else(CommandError::no_project)?;
    let status = GitRepository::open(session.repository_root(), state.git())?.status()?;
    if has_export_changes(&status) {
        return Err(export_error(
            "exportUncommitted",
            "commit the translation changes, pack settings, and font settings before exporting",
        ));
    }
    let commit = status
        .head
        .ok_or_else(|| export_error("exportNoCommit", "the project has no commits yet"))?;
    let manifest = PackManifest {
        pack_id: settings.pack_id.clone(),
        title: settings.title.clone(),
        publisher: settings.publisher.clone(),
        license: settings.license.clone(),
        sequence: release.sequence,
        version: version.to_owned(),
        channel: match release.channel {
            ChannelDto::Stable => Channel::Stable,
            ChannelDto::Testing => Channel::Testing,
        },
        target_language: session.workspace().metadata().target_language().to_owned(),
        source: pack_source(session.source()),
        content_policy: match release.content_policy {
            ContentPolicyDto::Reviewed => ContentPolicy::Reviewed,
            ContentPolicyDto::All => ContentPolicy::All,
        },
        project_commit: commit,
        exporter_aeria: env!("CARGO_PKG_VERSION").to_owned(),
        exporter_atlas,
        min_harmonia: settings.min_harmonia.clone(),
    };
    let mut encoder = AtlasEncoder {
        runner: atlas,
        cancellation: CancellationToken::default(),
    };
    let export = collect_project(
        session.workspace(),
        session.source(),
        manifest.content_policy,
        &mut encoder,
    )?;
    let root = session.repository_root().to_owned();
    drop(project);

    // The font files are committed (checked above), so HEAD is what renders.
    let fonts = FontSettings::load(&root)
        .and_then(|settings| {
            settings
                .map(|settings| aeria_fonts::generate(&root, &settings))
                .transpose()
        })
        .map_err(ExportError::Fonts)?;
    let signer = signer.map(SigningSecret::signer);
    let pack = write_pack_with_fonts(
        &manifest,
        export.sheets,
        fonts.as_ref(),
        signer.as_ref(),
        None,
    )?;
    Ok(BuiltRelease {
        manifest,
        fonts,
        pack,
        report: export.report,
        signed_by: signer.map(|signer| signer.fingerprint()),
    })
}

fn report_dto(built: &BuiltRelease) -> ExportReportDto {
    let report = &built.report;
    ExportReportDto {
        exported: report.exported,
        skipped_detached: report.skipped_detached,
        skipped_untranslated: report.skipped_untranslated,
        skipped_unreviewed: report.skipped_unreviewed,
        skipped_without_raw_hash: report.skipped_without_raw_hash.len() as u64,
        sheets: built.pack.counts.sheets,
        strings: built.pack.counts.strings,
        pack_hash: built.pack.pack_hash_text(),
        font_targets: built
            .fonts
            .as_ref()
            .map_or(0, |fonts| fonts.targets.len() as u64),
        font_glyphs: built
            .fonts
            .as_ref()
            .map_or(0, |fonts| fonts.glyph_count() as u64),
        signed_by: built.signed_by.clone(),
    }
}

/// The number of a published release: the suggested one, raised above the
/// latest GitHub release when local tags were behind (another maintainer
/// published, or tags were not fetched). Release numbers never repeat.
fn next_sequence(suggested: u64, latest_on_github: Option<u64>) -> u64 {
    latest_on_github.map_or(suggested, |latest| suggested.max(latest + 1))
}

fn validate_release(release: &ReleaseInputDto) -> CommandResult<()> {
    if release.sequence == 0 {
        return Err(export_error(
            "exportInvalidRelease",
            "the release number must be positive",
        ));
    }
    if release.version.trim().is_empty() {
        return Err(export_error(
            "exportInvalidRelease",
            "the release version must not be empty",
        ));
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
/// Reads pack settings, key state, and project readiness.
///
/// # Errors
///
/// Returns `noProjectOpen`.
pub async fn export_overview(app: tauri::AppHandle) -> CommandResult<ExportOverviewDto> {
    run_blocking(move || overview(&app.state::<DesktopState>(), &KeyringSigningKeyStore)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Writes `aeria-pack.json`. Nothing is committed; the change is committed
/// with the next checkpoint.
///
/// # Errors
///
/// Returns `exportSettingsInvalid` for invalid settings.
pub async fn export_save_settings(
    app: tauri::AppHandle,
    settings: PackSettingsDto,
) -> CommandResult<ExportOverviewDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let root = project_root(&state)?;
        save_settings(&root, settings)?;
        overview(&state, &KeyringSigningKeyStore)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Generates a signing key for the pack. Replacing the key the project
/// already records requires `replaceProjectKey`.
///
/// # Errors
///
/// Returns `exportKeyMismatch` when the project records another key.
pub async fn export_generate_key(
    app: tauri::AppHandle,
    replace_project_key: bool,
) -> CommandResult<ExportOverviewDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let secret = SigningSecret::generate()?;
        let root = project_root(&state)?;
        adopt_key(&root, &KeyringSigningKeyStore, &secret, replace_project_key)?;
        overview(&state, &KeyringSigningKeyStore)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Imports a key backup. It must be the key the project records, if any.
///
/// # Errors
///
/// Returns `exportKeyBackupInvalid` or `exportKeyMismatch`.
pub async fn export_import_key(
    app: tauri::AppHandle,
    path: String,
) -> CommandResult<ExportOverviewDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let text = std::fs::read_to_string(&path)
            .map_err(|error| export_error("exportIo", format!("could not read {path}: {error}")))?;
        let secret = SigningSecret::from_backup(&text)?;
        let root = project_root(&state)?;
        adopt_key(&root, &KeyringSigningKeyStore, &secret, false)?;
        overview(&state, &KeyringSigningKeyStore)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Writes a backup of the stored key to a file the user chose.
///
/// # Errors
///
/// Returns `exportKeyMissing` when no key is stored.
pub async fn export_backup_key(app: tauri::AppHandle, path: String) -> CommandResult<()> {
    run_blocking(move || {
        let settings = load_settings(&project_root(&app.state::<DesktopState>())?)?;
        let secret = KeyringSigningKeyStore
            .get(&settings.pack_id)?
            .ok_or_else(|| export_error("exportKeyMissing", "no signing key is stored"))?;
        write_file_atomically(
            Path::new(&path),
            secret.to_backup(&settings.pack_id).as_bytes(),
        )?;
        Ok(())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Removes the pack's key from this computer. The settings keep its
/// fingerprint, so a backup can be imported again.
///
/// # Errors
///
/// Returns an error when the key store cannot be written.
pub async fn export_remove_key(app: tauri::AppHandle) -> CommandResult<ExportOverviewDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let settings = load_settings(&project_root(&state)?)?;
        KeyringSigningKeyStore.delete(&settings.pack_id)?;
        overview(&state, &KeyringSigningKeyStore)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Adds or updates `.github/workflows/harmonia-feed.yml`.
///
/// # Errors
///
/// Returns `exportIo` when the file cannot be written.
pub async fn export_install_workflow(app: tauri::AppHandle) -> CommandResult<ExportOverviewDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        install_feed_workflow(&project_root(&state)?)
            .map_err(|error| export_error("exportIo", error.to_string()))?;
        overview(&state, &KeyringSigningKeyStore)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Builds a pack and writes it as `<packId>-<sequence>.hpk` into
/// `directory`. Signing is optional for local exports.
///
/// # Errors
///
/// Returns the first failed precondition, validation, or encoding error.
pub async fn export_pack(
    app: tauri::AppHandle,
    release: ReleaseInputDto,
    directory: String,
    sign: bool,
) -> CommandResult<LocalExportDto> {
    validate_release(&release)?;
    run_blocking(move || {
        let settings = load_settings(&project_root(&app.state::<DesktopState>())?)?;
        let secret = if sign {
            Some(project_signer(&settings, &KeyringSigningKeyStore)?)
        } else {
            None
        };
        let built = build(&app, &release, &settings, secret.as_ref())?;
        let path =
            Path::new(&directory).join(format!("{}-{}.hpk", settings.pack_id, release.sequence));
        write_file_atomically(&path, &built.pack.bytes)?;
        Ok(LocalExportDto {
            path: path.to_string_lossy().into_owned(),
            report: report_dto(&built),
        })
    })
    .await
}

/// Reports whether GitHub accepted the credential to Git's helper, so a
/// refused token is not offered again and a working one is kept.
async fn settle_credential<T>(
    git: &GitExecutable,
    root: &Path,
    credential: &HostCredential,
    result: &Result<T, PublishError>,
) {
    let approve = match result {
        Ok(_) => true,
        Err(PublishError::Unauthorized { .. }) => false,
        Err(_) => return,
    };
    let (git, root, credential) = (git.clone(), root.to_owned(), credential.clone());
    let _ = run_blocking(move || {
        let _ = if approve {
            git.credential_approve(&root, GITHUB_HOST, &credential)
        } else {
            git.credential_reject(&root, GITHUB_HOST, &credential)
        };
        Ok(())
    })
    .await;
}

struct PublishTarget {
    root: PathBuf,
    git: GitExecutable,
    repository: GitHubRepository,
    credential: HostCredential,
}

/// The GitHub repository of `origin` and a Git credential for it. Getting
/// the credential may open the helper's sign-in window.
fn publish_target(state: &DesktopState) -> CommandResult<PublishTarget> {
    let root = project_root(state)?;
    let git = state.git();
    let repository = github_target(&GitRepository::open(&root, git.clone())?).ok_or_else(|| {
        export_error(
            "exportNoGitHub",
            "the origin remote is not a github.com repository",
        )
    })?;
    let credential = git.credential_fill(&root, GITHUB_HOST).map_err(|error| {
        export_error(
            "githubSignIn",
            format!("could not get a GitHub sign-in from Git: {error}"),
        )
    })?;
    Ok(PublishTarget {
        root,
        git,
        repository,
        credential,
    })
}

async fn latest_sequence(
    client: &GitHubClient,
    target: &PublishTarget,
) -> CommandResult<Option<u64>> {
    let releases = client
        .pack_releases(&target.repository, target.credential.expose())
        .await;
    settle_credential(&target.git, &target.root, &target.credential, &releases).await;
    Ok(releases?.first().map(|release| release.sequence))
}

#[tauri::command(rename_all = "camelCase")]
/// Builds, signs, and publishes a release on GitHub. The project must be
/// committed and pushed. The release number is raised above the latest
/// GitHub release when needed; the result reports the number used.
///
/// # Errors
///
/// Returns the first failed precondition, export error, or GitHub error.
pub async fn export_publish(
    app: tauri::AppHandle,
    release: ReleaseInputDto,
) -> CommandResult<PublishedReleaseDto> {
    validate_release(&release)?;
    let session = app.clone();
    let (target, settings, secret) = run_blocking(move || {
        let state = session.state::<DesktopState>();
        let root = project_root(&state)?;
        let status = GitRepository::open(&root, state.git())?.status()?;
        if status.upstream.is_none() || status.ahead > 0 {
            return Err(export_error(
                "exportNotPushed",
                "push the project to GitHub before publishing",
            ));
        }
        let settings = load_settings(&root)?;
        let secret = project_signer(&settings, &KeyringSigningKeyStore)?;
        Ok((publish_target(&state)?, settings, secret))
    })
    .await?;

    let client = GitHubClient::new()?;
    let mut release = release;
    release.sequence = next_sequence(release.sequence, latest_sequence(&client, &target).await?);

    let built = {
        let (release, settings) = (release.clone(), settings.clone());
        run_blocking(move || build(&app, &release, &settings, Some(&secret))).await?
    };
    let transport = compress_for_transport(&built.pack.bytes)?;
    let asset = pack_asset_name(&settings.pack_id, release.sequence);
    let changelog = optional(release.changelog.clone());
    let entry = feed_entry(
        &built.manifest,
        &built.pack,
        &FeedDownload {
            url: target
                .repository
                .asset_download_url(release.sequence, &asset),
            brotli: true,
            size: transport.len() as u64,
            sha256: Sha256::digest(&transport).into(),
            unpacked_size: built.pack.bytes.len() as u64,
        },
        changelog.as_deref(),
    );
    let request = ReleaseRequest {
        sequence: release.sequence,
        commit: built.manifest.project_commit.clone(),
        name: format!("{} {}", settings.title, built.manifest.version),
        body: changelog.unwrap_or_default(),
        prerelease: matches!(release.channel, ChannelDto::Testing),
        assets: vec![
            ReleaseAsset {
                name: asset,
                content_type: "application/octet-stream",
                bytes: transport,
            },
            ReleaseAsset {
                name: FEED_ENTRY_ASSET.to_owned(),
                content_type: "application/json",
                bytes: entry,
            },
        ],
    };
    let published = client
        .publish_release(&target.repository, target.credential.expose(), &request)
        .await;
    settle_credential(&target.git, &target.root, &target.credential, &published).await;
    Ok(PublishedReleaseDto {
        sequence: release.sequence,
        release_url: published?.html_url,
        feed_url: target.repository.feed_url(),
        report: report_dto(&built),
    })
}

#[cfg(test)]
mod tests {
    use aeria_publish::MemorySigningKeyStore;

    use super::*;

    fn settings(root: &Path) -> PackSettings {
        let settings = PackSettings {
            pack_id: "ru-main".to_owned(),
            title: "Russian".to_owned(),
            publisher: Publisher {
                name: "Team".to_owned(),
                url: None,
            },
            license: None,
            min_harmonia: "1.0.0".to_owned(),
            signing_key_fingerprint: None,
        };
        settings.save(root).expect("save");
        settings
    }

    #[test]
    fn published_numbers_never_repeat() {
        assert_eq!(next_sequence(1, None), 1);
        assert_eq!(next_sequence(5, Some(3)), 5);
        assert_eq!(next_sequence(2, Some(7)), 8);
        assert_eq!(next_sequence(8, Some(8)), 9);
    }

    #[test]
    fn git_manages_the_feed_workflow() {
        assert_eq!(
            aeria_git::FEED_WORKFLOW_FILE,
            aeria_publish::FEED_WORKFLOW_PATH
        );
    }

    #[test]
    fn git_manages_the_pack_settings_file() {
        assert_eq!(aeria_git::PACK_SETTINGS_FILE, PACK_SETTINGS_FILE);
    }

    #[test]
    fn the_first_key_is_recorded_and_others_are_refused() {
        let temp = tempfile::tempdir().expect("temp");
        settings(temp.path());
        let store = MemorySigningKeyStore::default();
        let first = SigningSecret::generate().expect("key");
        adopt_key(temp.path(), &store, &first, false).expect("first key");
        let recorded = load_settings(temp.path()).expect("settings");
        assert_eq!(recorded.signing_key_fingerprint, Some(first.fingerprint()));
        assert_eq!(project_signer(&recorded, &store).expect("signer"), first);

        let second = SigningSecret::generate().expect("key");
        let refused = adopt_key(temp.path(), &store, &second, false).expect_err("mismatch");
        assert_eq!(refused.code, "exportKeyMismatch");
        assert_eq!(store.get("ru-main").expect("get"), Some(first.clone()));

        adopt_key(temp.path(), &store, &second, true).expect("replace");
        assert_eq!(
            load_settings(temp.path())
                .expect("settings")
                .signing_key_fingerprint,
            Some(second.fingerprint())
        );
    }

    #[test]
    fn a_key_other_than_the_recorded_one_cannot_sign() {
        let temp = tempfile::tempdir().expect("temp");
        let mut recorded = settings(temp.path());
        let store = MemorySigningKeyStore::default();
        assert_eq!(
            project_signer(&recorded, &store).expect_err("missing").code,
            "exportKeyMissing"
        );
        let key = SigningSecret::generate().expect("key");
        store.set("ru-main", &key).expect("set");
        recorded.signing_key_fingerprint = Some("0".repeat(64));
        assert_eq!(
            project_signer(&recorded, &store)
                .expect_err("mismatch")
                .code,
            "exportKeyMismatch"
        );
    }

    #[test]
    fn saving_settings_keeps_the_key_of_the_same_pack_only() {
        let temp = tempfile::tempdir().expect("temp");
        let mut recorded = settings(temp.path());
        recorded.signing_key_fingerprint = Some("a".repeat(64));
        recorded.save(temp.path()).expect("save");
        let mut input = PackSettingsDto::from(&recorded);
        input.title = " Russian translation ".to_owned();
        input.license = Some("  ".to_owned());
        save_settings(temp.path(), input.clone()).expect("save");
        let saved = load_settings(temp.path()).expect("settings");
        assert_eq!(saved.title, "Russian translation");
        assert_eq!(saved.license, None);
        assert_eq!(saved.signing_key_fingerprint, Some("a".repeat(64)));

        input.pack_id = "ru-other".to_owned();
        save_settings(temp.path(), input).expect("save");
        assert_eq!(
            load_settings(temp.path())
                .expect("settings")
                .signing_key_fingerprint,
            None
        );
    }
}
