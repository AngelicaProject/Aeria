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
idempotent and drops the active session without changing the workspace.

## Local project registry

The desktop keeps a bounded convenience registry at
`<app-data>/projects-v1.json`. It is application-local state, not workspace
data, and is never written to `.aeria/`, a translation repository, an
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
   (or open_with_source_update when the caller accepted a source update)
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

## Source updates

`open_project` and `open_recent_project` take an optional
`acceptSourceUpdate` flag. Without it, a workspace that is not current for
the package fails with the stable code `sourceUpdateRequired` and nothing is
written. `preview_source_update(repositoryRoot, sourcePackagePath)` returns
the `SourceUpdateReportDto` of the plan without writing or changing the
active project, so the renderer can ask for confirmation. With the flag set,
the command opens through `ProjectSession::open_with_source_update` and
returns the applied report in `ProjectOpenResultDto.sourceUpdate`.

`update_project_from_game(jobId, repositoryRoot, gamePath)` reads the source
language from the existing workspace manifest, builds and publishes a source
package with Harmonia Atlas exactly like project creation, and opens the
project with the update applied. It is the path for an installed game after a
patch and for a collaborator who has only a cloned repository.

`ProjectSummaryDto.detachedUnitCount` reports detached units, and
`list_detached_units` returns each one's last binding, reason, target,
review state, and note. A plan that cannot be built maps to `sourceUpdate`.
Planning and applying remain in `aeria-rebase` and `aeria-workspace`; the
IPC layer only chooses whether to call the preview or the applying
constructor.

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

AI provider commands (`ai_settings`, `ai_save_provider`, `ai_remove_provider`,
`ai_set_api_key`, `ai_clear_api_key`, `ai_set_agent_model`,
`ai_list_remote_models`, and `ai_test_connection`) manage the local provider
settings and OS-stored keys described in [`ai.md`](./ai.md#provider-boundary).
They do not require an open project. Settings and secret-store access run in
blocking workers; provider requests are async, use one shared HTTP client in
`DesktopState`, and hold no desktop lock. A key is accepted from the renderer
but never returned: provider DTOs report only `apiKey` as `stored`, `missing`,
or `unavailable`. `ai_test_connection` sends one minimal Chat Completions
request for any model ID and effort, so a model can be probed before it is
saved or an effort enabled. Failures map to stable `ai*` codes such as
`aiApiKeyMissing`, `aiUnauthorized`, `aiEndpointNotFound`, `aiRateLimited`,
`aiInvalidSettings`, and `aiSecretStoreUnavailable`.

`ai_chatgpt_login_start` starts the ChatGPT device sign-in for a ChatGPT
provider, opens the sign-in page in the default browser through the opener
plugin, and returns the code to show. Polling and the token exchange run in a
registered async task; the result arrives as an `ai://chatgpt-login` event
with the provider ID and either success or a typed error.
`ai_chatgpt_login_cancel` stops a waiting sign-in. Provider commands resolve a
ChatGPT provider's endpoint through the in-memory access-token cache in
`DesktopState`, whose async lock also serializes token refreshes.

Angelica commands (`angelica_conversations`, `angelica_conversation`,
`angelica_send`, `angelica_cancel`, and `angelica_delete_conversation`) work on
the active project's conversations described in [`ai.md`](./ai.md#angelica).
`angelica_send` validates the model selection against the AI settings,
appends the user message, stores the conversation, and returns it before the
turn runs; at most one turn runs per conversation (`angelicaBusy`). The turn
is an async task registered in `DesktopState` before it can start, so it can
always be found and stopped. It holds no desktop lock; each tool runs in a
blocking worker that locks the project only for its own read. Progress
reaches the renderer as `angelica://event` events carrying the conversation
ID and one of `textDelta`, `reasoningDelta`, `responseFinished`,
`toolStarted`, `toolFinished`, `usage`, `turnFinished`, `turnFailed`, or
`turnCancelled`. `angelica_cancel` aborts the task and emits `turnCancelled`.
The `navigate_to` tool resolves its location to one translatable occurrence
and emits `angelica://navigate` with that `SourceBinding`, which the editor
reveals.

`angelica_send` takes the conversation's mode. In Ask and Auto-draft modes
the write tools run in the same blocking workers; immediate writes hold the
project lock, and new proposals are appended under a separate proposal lock
and announced with `angelica://proposals`. `angelica_proposals`,
`angelica_apply_proposal`, and `angelica_reject_proposal` list and settle a
conversation's proposals; applying writes through
`ProjectSession::set_assisted_target` with the user's approval to replace a
reviewed string. Every write emits `angelica://translation-applied` with the
binding and its new `TranslationOverlayDto`. `angelica_draft` produces one
draft with the default model and returns it without saving
(`aiNoAgentModel`, `angelicaUntaggable`, and `angelicaDraftRejected` are its
own errors).

Commands that require an active project report `noProjectOpen` before
validating project-scoped payload such as translation-unit IDs.

The IPC boundary contains no source update planning and no background
server. React has no direct filesystem or SQLite access. Translation-unit IDs
cross IPC only in their canonical textual form, and review states use an
explicit camelCase protocol enum.

Source-package creation is the one desktop process workflow that owns an Atlas
child job. The renderer first starts the desktop job and receives its opaque
job ID before the Atlas worker starts; the long-running initialization command
uses that ID to claim the active cancellation token. Progress events report
Atlas state only and are not the source of job identity, so cancellation is
available even before the first external-process event arrives. The build
stages the v0.3.0 sidecar with Tauri's target-triple
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
cancellation acceptance with workspace creation or source update and
active-project replacement, leaving no newly initialized or updated project
successful when cancellation has been accepted.

After that final Atlas publication boundary and after the Atlas job finishes,
the desktop attempts the recent-project upsert using the final immutable HSP
path from the active `ProjectSession`; registry I/O is not performed inside
the cancellation/publication critical section.
