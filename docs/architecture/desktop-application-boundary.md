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

The HXS path remains local runtime state held by `ProjectSession`; it is not
added to Workspace Format. Translation browsing delegates to the bounded
`ProjectSession::page_translation_entries` API, so page size remains governed
by the backend contract. Tauri performs DTO and error mapping, not business
logic, and does not access HXS or SQLite directly.

Commands that require an active project report `noProjectOpen` before
validating project-scoped payload such as translation-unit IDs.

The IPC boundary contains no source update or rebase logic, no background
server, and no async worker architecture. React has no direct filesystem or
SQLite access. Translation-unit IDs cross IPC only in their canonical textual
form, and review states use an explicit camelCase protocol enum.
