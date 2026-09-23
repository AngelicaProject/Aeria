# Release channels

Aeria intends to support two channels from early development:

- **Stable**: signed tagged releases intended for ordinary users.
- **Nightly**: latest green build from the main development branch for users who opt into fast updates.

The desktop updater should use the standard signed Tauri update mechanism rather than a custom update engine.

Windows is the first packaging target. A portable Windows artifact is useful alongside the installer. Linux support should follow quickly, initially with a simple broadly usable distribution format before expanding package coverage.

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
