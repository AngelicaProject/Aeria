# Desktop application boundary

The desktop process currently owns one active project session and exposes the
existing Rust application API to the renderer through typed Tauri commands:

```text
React
  ↓ invoke
Tauri command DTOs
  ↓
DesktopState
  ↓
one ProjectSession
  ↓
aeria-workspace
```

`DesktopState` contains one `Mutex<Option<ProjectSession>>`. Rust owns the
authoritative project state; React receives owned DTO snapshots only. There is
one active project per desktop process for now. Opening or initializing a
replacement constructs and verifies the new `ProjectSession` before acquiring
the state lock, so failure preserves the previous active session. Closing is
idempotent and drops the active session without changing Workspace Format v1.

The HSP path remains local runtime state held by `ProjectSession`; its
materialized HXS cache path is also local runtime state. Neither is added to
Workspace Format. Public commands derive the application cache directory
through Tauri's path API; React cannot choose an arbitrary cache root.
Translation browsing delegates to the bounded
`ProjectSession::page_translation_rows` API, so page size remains governed by
the backend contract. The DTO is row-centric, while each contained cell keeps
its existing `SourceBinding` and overlay. Tauri performs DTO and error mapping,
not business logic, and does not access HXS or SQLite directly.

Commands that require an active project report `noProjectOpen` before
validating project-scoped payload such as translation-unit IDs.

The IPC boundary contains no source update or rebase logic, no background
server, and no async worker architecture. React has no direct filesystem or
SQLite access. Translation-unit IDs cross IPC only in their canonical textual
form, and review states use an explicit camelCase protocol enum.

Source-package creation is the one desktop process workflow that owns an Atlas
child job. Rust resolves the bundled v0.2.0 executable (or the explicit
`AERIA_ATLAS_PATH` development/test override), supplies the stable app-data
staging path, forwards typed `source-package-event` payloads containing an
opaque job ID, validates the staging HSP before immutable publication, and
publishes a project only after the completed package has been validated.
Desktop state permits one active package job. Cancellation signals that job,
terminates and awaits Atlas, and remains authoritative through validation and
workspace initialization: the final publication boundary serializes
cancellation acceptance with Workspace Format v1 creation and active-project
replacement, leaving no newly initialized project successful when cancellation
has been accepted.
