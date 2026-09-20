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

`DesktopState` contains one `Mutex<Option<ProjectSession>>` and a separate
narrow mutex for local recent-project registry file operations. Rust owns the
authoritative project state; React receives owned DTO snapshots only. There is
one active project per desktop process for now. Opening or initializing a
replacement constructs and verifies the new `ProjectSession` before acquiring
the state lock, so failure preserves the previous active session. Closing is
idempotent and drops the active session without changing Workspace Format v1.

## Local project registry

The desktop keeps a bounded convenience registry at
`<app-data>/projects-v1.json`. It is application-local state, not Workspace
Format v1, and is never written to `.aeria/`, a translation repository, an
HSP, or the disposable HXS cache. The existing `<app-data>/source-packages/`
and `<app-cache>/` locations retain their existing roles.

The registry stores an opaque UUID-like local ID, canonical repository and HSP
paths, the exact validated HSP `sourcePackageId`, cached source/target language
and game-version display metadata, and the last-opened Unix timestamp. A
successful open or create canonicalizes both paths and upserts by canonical
repository path, preserving the local ID for that repository. The registry is
ordered newest first with the local ID as a deterministic tie-breaker and is
bounded to 50 entries. Missing paths remain visible until explicitly removed.

Registry v1 loading validates the version, IDs, paths, package identity, and
metadata without canonicalizing stale paths. Malformed or newer registries
produce typed errors and are not replaced with an empty file. Writes use a
same-directory flushed and synced temporary file followed by atomic rename
where supported; Windows uses an owned previous-file recovery path. A valid
final file wins over recovery state, and an owned previous file is recovered
only when the final file is missing.

The launcher can list recents using filesystem presence only, without opening
HSP/HXS data or creating a `ProjectSession`. Ready entries can be opened by
opaque ID through this sequence:

```text
filesystem existence checks
→ SourcePackage::open on the remembered HSP path
→ exact sourcePackageId comparison
→ ProjectSession::open_from_source_package with that validated package
→ active-project replacement
→ best-effort registry refresh
```

The exact remembered HSP association is therefore checked before workspace
compatibility can reject a replacement package. `Remove from recents`
removes only registry state. Manual Open project and Create project remain
fully usable when the registry is corrupt, stale, or unavailable, and closing
an active project does not remove its entry.

Successful open/create commands return `ProjectOpenResultDto`. A registry
write failure is a non-fatal `projectRegistryWrite` warning after the active
session is installed, so local convenience-state failure never rolls back a
valid project. The desktop does not auto-open the last project at startup.

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
child job. The build stages the v0.2.0 sidecar with Tauri's target-triple
filename convention, while packaged runtime lookup resolves
`harmonia-atlas[.exe]` beside the Aeria executable. Rust first honors the
explicit `AERIA_ATLAS_PATH` development/test override, then the packaged
executable sibling, and finally resource-directory compatibility fallbacks.
It supplies the stable app-data staging path, forwards typed
`source-package-event` payloads containing an opaque job ID, validates the
staging HSP before immutable publication, and publishes a project only after
the completed package has been validated.
Desktop state permits one active package job. Cancellation signals that job,
terminates and awaits Atlas, and remains authoritative through validation and
workspace initialization: the final publication boundary serializes
cancellation acceptance with Workspace Format v1 creation and active-project
replacement, leaving no newly initialized project successful when cancellation
has been accepted.

After that final Atlas publication boundary and after the Atlas job finishes,
the desktop attempts the recent-project upsert using the final immutable HSP
path from the active `ProjectSession`; registry I/O is not performed inside
the cancellation/publication critical section.
