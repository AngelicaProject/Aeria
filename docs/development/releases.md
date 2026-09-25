# Releases and updates

Aeria is a public tool distributed through GitHub Releases of
`AngelicaProject/Aeria`. Windows is the only release platform for now; Linux
packaging follows separately.

## Versions

- Aeria follows [Semantic Versioning](https://semver.org/). Before 1.0, a
  minor version may change persisted formats or behavior incompatibly; such
  changes are called out in the release notes.
- The whole application has one version. It is declared once, as the
  `[workspace.package]` version in `Cargo.toml`, which every crate and the
  Tauri configuration inherit; the root and `apps/desktop` `package.json`
  files mirror it. `node tools/release/version.mjs check` fails when these
  disagree.
- A stable release tag is `v` followed by exactly that version
  (`v0.1.0`). The release workflow refuses a tag that does not match, and
  the declared version on `main` is always a plain `MAJOR.MINOR.PATCH`.

## Channels

| Channel | Source | Version | Feed |
| --- | --- | --- | --- |
| Stable | Tag `vX.Y.Z` on a green `main` commit | `X.Y.Z` | `releases/latest/download/latest.json` |
| Nightly | Every successful CI run of a push to `main` | `<next>-nightly.<build>` | `releases/download/nightly/latest.json` |

Both are public. Stable releases are ordinary GitHub releases marked
*latest*. Nightly is one rolling GitHub pre-release, tag `nightly`, replaced
by each build and never marked *latest*.

Nightly versions sort before the stable release they lead to. `<next>` is the
declared version while it has not been tagged yet, and the next patch version
once it has; `<build>` is the release workflow's run number. So after
`v0.1.0`, nightlies are `0.1.1-nightly.N`, and `0.1.1` is newer than all of
them.

The channel is chosen in **Settings → About → Update channel** and stored in
`<app-data>/update-settings.json` (`{"version":1,"channel":"stable"}`). Without
the file a copy follows the channel it was published on. A malformed file is
reported as `updateSettings` and is never replaced with defaults.

- The nightly channel checks both feeds and offers the newer version, so a
  stable release newer than the last nightly is offered at once.
- Switching from nightly to stable needs no manual steps: the copy keeps its
  nightly build and moves to stable with the first stable release newer than
  it. Aeria never installs an older version, because a nightly may already
  have migrated local or workspace data to a newer format.

## Distribution

Each release carries:

| Asset | Purpose |
| --- | --- |
| `Aeria_<version>_x64-setup.exe` | NSIS installer; installs for the current user without administrator rights |
| `Aeria_<version>_x64-setup.exe.sig` | Updater signature of the installer |
| `Aeria_<version>_x64-portable.zip` | Portable build: `aeria.exe`, `harmonia-atlas.exe`, and `git/` |
| `latest.json` | Updater feed of this release |
| `SHA256SUMS.txt` | SHA-256 of the installer and the portable archive |

Executables are not Authenticode-signed; Windows SmartScreen may warn on first
run, and that is accepted. The installer uses Tauri's per-user NSIS mode
(`bundle.windows.nsis.installMode = "currentUser"`), and Windows builds produce
only the NSIS bundle. The portable build keeps its data in the same per-user
folders as an installed copy.

## Application updates

Updates use the standard Tauri updater (`tauri-plugin-updater`), driven from
Rust in `apps/desktop/src-tauri/src/updates.rs`; the renderer has no updater
permissions and calls the `update_*` commands.

- **Signatures.** Installers are signed with the updater's minisign key; the
  public key is `plugins.updater.pubkey` in `tauri.conf.json`, and a download
  whose signature does not verify is rejected. The private key and its
  password are the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; the maintainers keep an offline backup.
  Losing the key means installed copies can no longer update themselves and
  must reinstall manually after the key is replaced.
- **Checks.** Aeria checks the channel's feed 20 seconds after startup and
  every 6 hours, and on demand from Settings. Failures are shown in Settings
  and never interrupt work.
- **Offer.** A newer version shows a card in the corner of the window and an
  *Update* button in the title bar. *Later* hides the card until the next
  launch or a newer version; the title-bar button brings it back. Nothing is
  installed without the user's request.
- **Never interrupting.** Installing waits while the editor holds an unsaved
  draft or a save is in flight, and while Angelica translates, a Git
  synchronization, commit, or branch change runs, a pack is exported or
  published, or a source package is built. The backend enforces the same
  rule for its own work (`updateBusy`) and keeps new synchronization and
  export from starting while the installer launches. When the user asked to
  install during such work, Aeria installs and restarts once it finishes.
- **Installation.** The verified installer runs in passive mode, replaces the
  installed copy, and restarts Aeria.
- **Portable and development copies** check and announce updates but never
  install them; they link to the release page instead. A copy updates itself
  only when its executable carries the NSIS bundle type, which Tauri writes
  into the executable it packs into the installer. The portable archive is
  packaged from the unbundled build, before the installer is created.

## Release workflow

`.github/workflows/release.yml` builds on `windows-latest`:

1. For a tag, it waits for CI on `main` to pass for the tagged commit and
   fails if the commit is not on `main` or CI failed. A nightly starts only
   after a successful CI run of a push to `main`.
2. It checks or computes the version with `tools/release/version.mjs`; a
   nightly writes its version into the declarations and the lockfile before
   building.
3. It stages the pinned Harmonia Atlas sidecar and MinGit, runs
   `tauri build --no-bundle`, packages the portable archive, and then runs
   `tauri bundle --bundles nsis` with the signing secrets.
4. It writes `latest.json` and `SHA256SUMS.txt` and publishes the stable
   release with generated notes, or replaces the `nightly` pre-release.

## Cutting a stable release

1. On a branch, set the new version in `Cargo.toml` and both `package.json`
   files, run `cargo update --workspace`, and merge it to `main` through a pull
   request.
2. After the merge, tag the merge commit and push the tag:

   ```text
   git tag -a vX.Y.Z -m "Aeria X.Y.Z" <merge-commit>
   git push origin vX.Y.Z
   ```

3. Watch the Release workflow, then review the generated notes on the GitHub
   release and add highlights and any compatibility notes.

A published version is never rebuilt or re-tagged; fix problems with a new
patch release.

## Bundled Git

Git is Aeria's collaboration layer and translators are not expected to
install it.

- **Windows**: run `tools/git/stage-mingit.ps1` before building. It downloads
  the MinGit release pinned in `tools/git/version.json`, verifies its
  SHA-256, extracts it to `apps/desktop/src-tauri/binaries/mingit/`
  (ignored by Git), and checks that it runs and configures Git Credential
  Manager. `tauri.windows.conf.json` bundles that directory as the `git/`
  resource; a Windows build fails if it has not been staged. To update Git,
  change the URL, SHA-256, and source link in `version.json` together.
- **Linux**: the `.deb` and `.rpm` bundles declare a dependency on `git`.
  Other formats rely on Git being installed.
- **Licensing**: MinGit is GPL-2.0 and ships its `LICENSE.txt` and component
  licenses inside the bundled directory. Releases must keep those files and
  make the corresponding source available; the exact Git for Windows source
  tag is recorded as `source` in `tools/git/version.json`. Git Credential
  Manager (MIT) ships its license in the same directory.
