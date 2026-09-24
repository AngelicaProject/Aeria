# Dependency policy and major dependencies

Aeria prefers a small set of mature dependencies over internal reinvention of generic infrastructure.

A dependency should have a concrete current use, a maintenance story, and compatible licensing. Remove unused dependencies rather than keeping speculative framework pieces.

## Runtime tools

- Git: collaboration features invoke the Git command-line client (2.28 or
  newer). Windows builds bundle a pinned MinGit (Git for Windows, GPL-2.0,
  including Git Credential Manager, MIT); the pin and SHA-256 live in
  `tools/git/version.json` and are staged by `tools/git/stage-mingit.ps1`.
  Linux packages depend on the distribution `git` package. Aeria remains
  usable for local translation without Git; Git commands report
  `gitUnavailable`.

## Approved architectural foundations

- Tauri 2: desktop shell and typed IPC boundary
- React: renderer UI
- TypeScript: strict frontend typing
- Vite: frontend build/dev tooling
- SQLite: local disposable indexes/cache/job state

Likely UI-level dependencies such as Tailwind, Radix primitives, CodeMirror, docking/layout libraries, and Git/SQLite Rust crates should be added when the first feature that needs them is implemented, after checking their current stable versions and licenses.

## AI provider dependencies

`aeria-ai` uses:

- `reqwest` (MIT OR Apache-2.0) with only `http2`, `json`, `system-proxy`, and `rustls-no-provider`: the provider HTTP client. It is already in the desktop dependency graph through Tauri.
- `rustls` (Apache-2.0 OR ISC OR MIT) with the `ring` crypto provider, installed once by the client, so builds need neither OpenSSL nor the `aws-lc` toolchain. Certificates are checked with the platform verifier.
- `keyring` (MIT OR Apache-2.0) with its default platform stores: Windows Credential Manager, the Secret Service on Linux, and the macOS Keychain.
- `base64` (MIT OR Apache-2.0): reads the claims of ChatGPT access tokens.
- `csv` (Unlicense OR MIT): reads and writes the project glossary.
- `rusqlite` (MIT) with `bundled`: the local translation-job store. The bundled SQLite needs only a C compiler, which the Windows and Linux toolchains already provide.
- `fs2`, `uuid`, `serde_json`, and `thiserror`, matching `aeria-projects`.

`aeria-search` uses `rusqlite` (MIT) with `bundled`, whose SQLite includes
FTS5 with the `unicode61` and `trigram` tokenizers.

The desktop adds `tauri-plugin-opener` (Apache-2.0 OR MIT) to open the ChatGPT
sign-in page from Rust, and `tokio` with only `rt` and `time`, already part of
the Tauri runtime, to pace sign-in polling and to run a translation job's
lanes as one abortable task set.

## Renderer UI dependencies

- `radix-ui`: accessible menus, dialogs, tooltips, and selects.
- `@codemirror/state`, `@codemirror/view`, `@codemirror/commands`: source and target text editors with macro highlighting.
- `@tanstack/react-virtual`: virtualized strings list.
- `lucide-react`: renderer icons through `UiIcon`.

All are MIT licensed except `lucide-react` (ISC), and all are pinned to exact versions in `apps/desktop/package.json`. Tailwind CSS remains an approved direction but is not used; styling is plain CSS over semantic tokens.
