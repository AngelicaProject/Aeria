//! Harmonia pack export and publishing.
//!
//! Pack identity lives in the committed `aeria-pack.json`; the signing key in
//! the OS credential store; releases are created on GitHub with the Git
//! credential of the repository's `origin`. See
//! `docs/architecture/export.md`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use aeria_export::{
    BuiltPack, Channel, ExportError, ExportReport, FeedDownload, PACK_SETTINGS_FILE, PackManifest,
    PackSettings, PackVersion, ReleaseDate, SeStringEncoder, Team, collect_project,
    compress_for_transport, feed_entry, pack_game, write_file_atomically, write_pack_with_fonts,
};
use aeria_fonts::{FontSection, FontSettings};
use aeria_git::{GitExecutable, GitRepository, HostCredential};
use aeria_publish::{
    GITHUB_HOST, GitHubClient, GitHubRepository, KeyError, KeyringSigningKeyStore, PublishError,
    RELEASE_TAG_PREFIX, ReleaseAsset, ReleaseRequest, SigningKeyStore, SigningSecret,
    WorkflowState, feed_workflow_state, generate_pack_id, install_feed_workflow, pack_asset_name,
    tag_version,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::state::{Activity, DesktopState};

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
    /// The version the next release gets from today's date and the
    /// `harmonia/<version>` tags Git knows of; publishing may raise it when
    /// GitHub has newer releases.
    pub next_version: String,
    /// `aeria-fonts.json` exists, so the pack will carry font glyphs.
    pub fonts_configured: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackSettingsDto {
    /// Only reported; made once when the settings are first saved.
    #[serde(default, skip_deserializing)]
    pub pack_id: String,
    pub title: String,
    pub team_name: String,
    pub team_url: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
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
    pub channel: ChannelDto,
    #[serde(default)]
    pub changelog: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelDto {
    Stable,
    Testing,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReportDto {
    pub exported: u64,
    pub skipped_untranslated: u64,
    /// Translations left out because their source changed since they were
    /// written.
    pub skipped_fuzzy: u64,
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
    pub version: String,
    pub path: String,
    pub report: ExportReportDto,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedReleaseDto {
    pub version: String,
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
            team_name: settings.team.name.clone(),
            team_url: settings.team.url.clone(),
            authors: settings.authors.clone(),
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
    Ok(state.session()?.root().to_owned())
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
    let (root, source_language, game_version, target_language) = {
        let session = state.session()?;
        let settings = session.settings();
        (
            session.root().to_owned(),
            settings.source_language,
            session.source().version().to_string(),
            settings.target_language,
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
        source_language,
        target_language,
        game_version,
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
    let next_version = next_version(
        today(),
        [repository.as_ref().and_then(latest_tagged_version)],
    )?;
    Ok(ExportOverviewDto {
        next_version: next_version.to_string(),
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

/// Writes the settings. The pack ID and key fingerprint are kept from the
/// existing file; the first save makes the ID.
fn save_settings(
    root: &Path,
    input: PackSettingsDto,
    new_pack_id: impl FnOnce() -> CommandResult<String>,
) -> CommandResult<()> {
    let existing = PackSettings::load(root)?;
    let (pack_id, signing_key_fingerprint) = match existing {
        Some(existing) => (existing.pack_id, existing.signing_key_fingerprint),
        None => (new_pack_id()?, None),
    };
    let mut authors: Vec<String> = Vec::new();
    for author in input.authors {
        let author = author.trim().to_owned();
        if !author.is_empty() && !authors.contains(&author) {
            authors.push(author);
        }
    }
    let settings = PackSettings {
        pack_id,
        title: input.title.trim().to_owned(),
        team: Team {
            name: input.team_name.trim().to_owned(),
            url: optional(input.team_url),
        },
        authors,
        license: optional(input.license),
        min_harmonia: input.min_harmonia.trim().to_owned(),
        signing_key_fingerprint,
    };
    settings.save(root)?;
    Ok(())
}

fn today() -> ReleaseDate {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    ReleaseDate::from_unix_seconds(seconds)
}

/// The newest `harmonia/<version>` tag of the local repository.
fn latest_tagged_version(repository: &GitRepository) -> Option<PackVersion> {
    repository
        .tags_with_prefix(RELEASE_TAG_PREFIX)
        .ok()?
        .iter()
        .filter_map(|tag| tag_version(tag))
        .max()
}

/// The version of the next release: after the newest of the known releases
/// (local tags, GitHub), so versions never repeat or go back.
fn next_version(
    today: ReleaseDate,
    known: impl IntoIterator<Item = Option<PackVersion>>,
) -> CommandResult<PackVersion> {
    let latest = known.into_iter().flatten().max();
    PackVersion::next(today, latest)
        .map_err(|error| export_error("exportInvalidRelease", error.to_string()))
}

/// `<title> <version>.hpk`, with characters file names cannot hold replaced.
fn pack_file_name(title: &str, version: PackVersion) -> String {
    let title: String = title
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let title = title.trim().trim_end_matches('.');
    format!("{title} {version}.hpk")
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
    version: PackVersion,
    settings: &PackSettings,
    signer: Option<&SigningSecret>,
) -> CommandResult<BuiltRelease> {
    let state = app.state::<DesktopState>();
    // Writes are held so the files cannot change between the commit check
    // and the end of collection.
    let session = state.session()?;
    let writes = session.hold_writes();
    let status = GitRepository::open(session.root(), state.git())?.status()?;
    if has_export_changes(&status) {
        return Err(export_error(
            "exportUncommitted",
            "commit the translation changes, pack settings, and font settings before exporting",
        ));
    }
    let commit = status
        .head
        .ok_or_else(|| export_error("exportNoCommit", "the project has no commits yet"))?;
    let game_version = session
        .source()
        .current_version()
        .map_err(|error| export_error("gameRead", error.to_string()))?;
    if &game_version != session.source().version() {
        return Err(export_error(
            "gameChanged",
            "the game was updated while the project was open; open the project again",
        ));
    }
    let manifest = PackManifest {
        pack_id: settings.pack_id.clone(),
        title: settings.title.clone(),
        team: settings.team.clone(),
        authors: settings.authors.clone(),
        license: settings.license.clone(),
        version,
        channel: match release.channel {
            ChannelDto::Stable => Channel::Stable,
            ChannelDto::Testing => Channel::Testing,
        },
        language: session.settings().target_language,
        game: pack_game(session.source()),
        aeria: env!("CARGO_PKG_VERSION").to_owned(),
        commit,
        min_harmonia: settings.min_harmonia.clone(),
    };
    let export = collect_project(session.root(), session.source(), &mut SeStringEncoder)?;
    let root = session.root().to_owned();
    drop(writes);

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
        skipped_untranslated: report.skipped_untranslated,
        skipped_fuzzy: report.skipped_fuzzy,
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
        save_settings(&root, settings, || Ok(generate_pack_id()?))?;
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
/// The names of the project's commit authors, most commits first, for the
/// maintainer to choose the pack's authors from.
///
/// # Errors
///
/// Returns `noProjectOpen` or the Git error.
pub async fn export_git_authors(app: tauri::AppHandle) -> CommandResult<Vec<String>> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        Ok(GitRepository::open(&project_root(&state)?, state.git())?.authors()?)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Builds a pack and writes it as `<title> <version>.hpk` into
/// `directory`. The version follows the local release tags; signing is
/// optional for local exports.
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
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let _export = state.begin_activity(Activity::Export);
        let root = project_root(&state)?;
        let settings = load_settings(&root)?;
        let secret = if sign {
            Some(project_signer(&settings, &KeyringSigningKeyStore)?)
        } else {
            None
        };
        let tagged = latest_tagged_version(&GitRepository::open(&root, state.git())?);
        let version = next_version(today(), [tagged])?;
        let built = build(&app, &release, version, &settings, secret.as_ref())?;
        let path = Path::new(&directory).join(pack_file_name(&settings.title, version));
        write_file_atomically(&path, &built.pack.bytes)?;
        Ok(LocalExportDto {
            version: version.to_string(),
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

async fn latest_published_version(
    client: &GitHubClient,
    target: &PublishTarget,
) -> CommandResult<Option<PackVersion>> {
    let releases = client
        .pack_releases(&target.repository, target.credential.expose())
        .await;
    settle_credential(&target.git, &target.root, &target.credential, &releases).await;
    Ok(releases?.first().map(|release| release.version))
}

#[tauri::command(rename_all = "camelCase")]
/// Builds, signs, and publishes a release on GitHub. The project must be
/// committed and pushed. The version follows the newest release on GitHub
/// and in the local tags; the result reports the version used.
///
/// # Errors
///
/// Returns the first failed precondition, export error, or GitHub error.
pub async fn export_publish(
    app: tauri::AppHandle,
    release: ReleaseInputDto,
) -> CommandResult<PublishedReleaseDto> {
    let export_app = app.clone();
    let export_state = export_app.state::<DesktopState>();
    let _export = export_state.begin_activity(Activity::Export);
    let session = app.clone();
    let (target, settings, secret, tagged) = run_blocking(move || {
        let state = session.state::<DesktopState>();
        let root = project_root(&state)?;
        let repository = GitRepository::open(&root, state.git())?;
        let status = repository.status()?;
        if status.upstream.is_none() || status.ahead > 0 {
            return Err(export_error(
                "exportNotPushed",
                "push the project to GitHub before publishing",
            ));
        }
        let settings = load_settings(&root)?;
        let secret = project_signer(&settings, &KeyringSigningKeyStore)?;
        let tagged = latest_tagged_version(&repository);
        Ok((publish_target(&state)?, settings, secret, tagged))
    })
    .await?;

    let client = GitHubClient::new()?;
    let published = latest_published_version(&client, &target).await?;
    let version = next_version(today(), [tagged, published])?;

    let built = {
        let (release, settings) = (release.clone(), settings.clone());
        run_blocking(move || build(&app, &release, version, &settings, Some(&secret))).await?
    };
    let transport = compress_for_transport(&built.pack.bytes)?;
    let asset = pack_asset_name(version);
    let changelog = optional(release.changelog.clone());
    let entry = feed_entry(
        &built.manifest,
        &built.pack,
        &FeedDownload {
            url: target.repository.asset_download_url(version, &asset),
            brotli: true,
            size: transport.len() as u64,
            sha256: Sha256::digest(&transport).into(),
            unpacked_size: built.pack.bytes.len() as u64,
        },
        changelog.as_deref(),
    );
    let request = ReleaseRequest {
        version,
        commit: built.manifest.commit.clone(),
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
        version: version.to_string(),
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
            team: Team {
                name: "Team".to_owned(),
                url: None,
            },
            authors: Vec::new(),
            license: None,
            min_harmonia: "1.0.0".to_owned(),
            signing_key_fingerprint: None,
        };
        settings.save(root).expect("save");
        settings
    }

    #[test]
    fn versions_follow_the_newest_known_release() {
        let today = ReleaseDate {
            year: 2026,
            month: 10,
            day: 1,
        };
        let version = |text: &str| Some(text.parse::<PackVersion>().expect("version"));
        let next =
            |known: [Option<PackVersion>; 2]| next_version(today, known).expect("next").to_string();
        assert_eq!(next([None, None]), "2026.10.01.0001");
        assert_eq!(next([version("2026.09.30.0002"), None]), "2026.10.01.0001");
        assert_eq!(
            next([version("2026.10.01.0001"), version("2026.10.01.0003")]),
            "2026.10.01.0004"
        );
        assert_eq!(
            next([version("2026.10.01.0002"), version("2026.10.01.0001")]),
            "2026.10.01.0003"
        );
    }

    #[test]
    fn pack_files_are_named_after_the_title() {
        let version: PackVersion = "2026.10.01.0002".parse().expect("version");
        assert_eq!(
            pack_file_name("Русский перевод", version),
            "Русский перевод 2026.10.01.0002.hpk"
        );
        assert_eq!(
            pack_file_name("A/B: \"C\"?.", version),
            "A-B- -C-- 2026.10.01.0002.hpk"
        );
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
    fn saving_settings_keeps_the_pack_id_and_key() {
        let temp = tempfile::tempdir().expect("temp");
        let mut recorded = settings(temp.path());
        recorded.signing_key_fingerprint = Some("a".repeat(64));
        recorded.save(temp.path()).expect("save");
        let mut input = PackSettingsDto::from(&recorded);
        input.title = " Russian translation ".to_owned();
        input.license = Some("  ".to_owned());
        input.authors = vec![" Анна ".to_owned(), String::new(), "Анна".to_owned()];
        save_settings(temp.path(), input, || unreachable!("the ID exists")).expect("save");
        let saved = load_settings(temp.path()).expect("settings");
        assert_eq!(saved.pack_id, "ru-main");
        assert_eq!(saved.title, "Russian translation");
        assert_eq!(saved.license, None);
        assert_eq!(saved.authors, ["Анна"]);
        assert_eq!(saved.signing_key_fingerprint, Some("a".repeat(64)));
    }

    #[test]
    fn the_first_save_makes_the_pack_id() {
        let temp = tempfile::tempdir().expect("temp");
        let input = PackSettingsDto {
            pack_id: String::new(),
            title: "Russian".to_owned(),
            team_name: "Team".to_owned(),
            team_url: None,
            authors: Vec::new(),
            license: None,
            min_harmonia: "0.1.2.0".to_owned(),
            signing_key_fingerprint: None,
        };
        save_settings(temp.path(), input, || Ok(generate_pack_id()?)).expect("save");
        let saved = load_settings(temp.path()).expect("settings");
        assert_eq!(saved.pack_id.len(), 36);
        assert_eq!(saved.signing_key_fingerprint, None);
    }
}
