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
metadata without canonicalizing stale paths. It rejects documents with more
than 50 projects and files larger than 256 KiB before full JSON
deserialization. Malformed, oversized, or newer registries produce
typed errors and are not replaced with an empty file. Every registry
transaction acquires an exclusive OS file lock in app-data before cleanup or
recovery, load, read-modify-write mutation, and atomic publication; the lock
is released only after the transaction completes. This lock is required for
multiple Aeria processes to share the fixed final, partial, and previous
paths safely. Writes use a same-directory flushed and synced temporary file
followed by atomic rename where supported; Windows uses an owned previous-file
recovery path. A valid final file wins over recovery state, and an owned
previous file is recovered only when the final file is missing.

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

`translation_progress` returns per-sheet `SheetProgressDto` coverage for the
active project from `ProjectSession::translation_progress` (see
[`translation-read.md`](./translation-read.md#translation-progress)). The
renderer re-reads it after each committed translation mutation or workspace
reload and never derives sheet-wide progress from loaded row pages.
`app_info` returns the application name and version for display.

Filesystem, HSP/HXS, SQLite, workspace loading, row paging, and ordinary
translation mutations run inside Tauri blocking workers. The async command
handlers do not hold `DesktopState` or the project mutex across an await;
worker-side access still goes through the single `ProjectSession` mutex, so
mutations remain serialized. Target, note, and review commands return the
compact committed `TranslationOverlayDto` for the changed cell; the renderer
patches that cell instead of reloading the current sheet.

Git collaboration commands (`git_overview`, `git_initialize`,
`git_set_identity`, `git_set_remote`, `git_pending_changes`,
`git_checkpoint`, `git_log`, `git_commit_changes`, `git_unit_history`,
`git_unit_attribution`, `git_contributors`, `git_sync`, `git_branches`,
`git_create_branch`, `git_switch_branch`, `git_set_collaboration`,
`git_finish_contribution`, and `git_clone_repository`) delegate to
`aeria-git` for the active project's repository root; see
[`git.md`](./git.md). The Git executable is selected once at application
setup (override, bundled runtime, then `PATH`) and kept in `DesktopState`.
Checkpoint, the integration step of sync, branch switches, and finishing a
contribution hold the project mutex so they cannot interleave with
translation mutations; fetch and push run without it. Operations that change
the working tree reload the active `ProjectSession` and are rolled back if
the reload fails. Same-unit sync conflicts are returned in the sync result,
not as an error, so the renderer can collect per-unit resolutions and sync
again. Project-wide attribution is cached in memory per repository root and
`HEAD`; it is derived data and never persisted. Git failures map to stable
`git*` error codes such as `gitUnavailable`, `gitIdentityMissing`,
`gitMergeConflict`, `gitIncomingRejected`, and `gitInvalidSettings`.

Commands that require an active project report `noProjectOpen` before
validating project-scoped payload such as translation-unit IDs.

The IPC boundary contains no source update or rebase logic and no background
server. React has no direct filesystem or SQLite access. Translation-unit IDs
cross IPC only in their canonical textual form, and review states use an
explicit camelCase protocol enum.

Source-package creation is the one desktop process workflow that owns an Atlas
child job. The renderer first starts the desktop job and receives its opaque
job ID before the Atlas worker starts; the long-running initialization command
uses that ID to claim the active cancellation token. Progress events report
Atlas state only and are not the source of job identity, so cancellation is
available even before the first external-process event arrives. The build
stages the v0.2.0 sidecar with Tauri's target-triple
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
