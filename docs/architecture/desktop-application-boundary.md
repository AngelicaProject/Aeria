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

## Local data folders

`<app-data>` is the `Aeria` folder in the platform data directory
(`%APPDATA%\Aeria` on Windows, `~/.local/share/aeria` on Linux), and
`<app-cache>` is the same folder name in the platform cache directory
(`%LOCALAPPDATA%\Aeria`, `~/.cache/aeria`). All desktop code resolves them
through `paths::AeriaPaths`, never Tauri's identifier-named directories. The
bundle identifier `org.angelicaproject.aeria` still names the WebView profile
and installer registration.

Earlier versions stored data under the identifier-named folder. At startup
Aeria renames the legacy data folder to `<app-data>` when `<app-data>` does
not exist yet. A failed move is reported and leaves the legacy folder in use,
so no data is lost; when both folders exist, the legacy one is left
untouched.

`<app-cache>` holds the sheet catalog per source language and game version
(`sheet-catalog/`), and `<app-data>/search/` the source search indexes. Both
are disposable data derived from the game: a missing or unreadable file is
built again.

## Local project registry

The desktop keeps a bounded convenience registry at
`<app-data>/projects-v2.json`. It is application-local state, not workspace
data, and is never written to `.aeria/`, a translation repository, or the
cache. Registries of earlier versions (`projects-v1.json`) are not read.

The registry stores an opaque UUID-like local ID, the canonical repository
path, cached source/target language and game-version display metadata, and
the last-opened Unix timestamp. A successful open or create canonicalizes the
repository path and upserts by it, preserving the local ID for that repository. The registry is
ordered newest first with the local ID as a deterministic tie-breaker and is
bounded to 50 entries. Missing paths remain visible until explicitly removed.

Registry loading validates the version, IDs, paths, and
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

The launcher can list recents using filesystem presence only, without reading
the game or creating a `ProjectSession`. Ready entries can be opened by
opaque ID through this sequence:

```text
repository existence check
→ GameSource::open on the configured installation in the remembered
   source language
→ ProjectSession::open with that game
   (or open_with_source_update when the caller accepted a source update)
→ active-project replacement
→ best-effort registry refresh
```

The workspace manifest, not the registry, decides whether the game fits the
project; the registry's language and game version are display data, and the
language selects which language the game is opened in. `Remove from recents`
removes only registry state. Manual Open project and Create project remain
fully usable when the registry is corrupt, stale, or unavailable, and closing
an active project does not remove its entry.

Successful open/create commands return `ProjectOpenResultDto`. A registry
write failure is a non-fatal `projectRegistryWrite` warning after the active
session is installed, so local convenience-state failure never rolls back a
valid project. The desktop does not auto-open the last project at startup.

## Application updates

`updates.rs` owns application updates; the release model is described in
[`../development/releases.md`](../development/releases.md#application-updates).
The renderer has no updater permissions and uses `update_status`,
`update_check`, `update_set_channel(channel)`, `update_download`,
`update_install`, and `update_open_release`, and listens to
`update://status`, which carries the full `UpdateStatusDto` after every
change. The channel is local application state in `update-settings.json`; a
malformed file fails with `updateSettings` and is never replaced with
defaults.

`DesktopState` records the work an update must not interrupt. Mutating Git
commands (checkpoint, commit, sync, branch switch, finishing or merging a
contribution, clone) hold a `Sync` activity guard and pack export and
publication an `Export` guard for their whole run; Angelica turns,
and translation-job runners are read from their own registries. `update_install` runs through `DesktopState::while_idle`, which
fails with `updateBusy` while any of them runs and holds the activity lock
while the installer starts, so no guarded operation begins in between. The
renderer additionally reports unsaved editor drafts and saves in flight to
the update store and does not request installation while one exists.

## Source updates

Project commands take no game path. They resolve the installation in
the worker from the application setting: the folder chosen with
`set_game_path`, otherwise the first detected installation. A missing
installation fails with `gameInstallationRequired`; a chosen folder that is
no longer an installation fails with `gameInstallationInvalid`.
`game_settings` returns the chosen folder, the installation in use, and the
detected installations; `set_game_path(path)` validates and stores a folder,
or with `null` returns to detection. The setting is local application state
in `game-settings.json`; a malformed file fails with `gameSettings` and is
never replaced with defaults.

`open_project_from_game(repositoryRoot)` reads the source language from the
workspace manifest and opens the configured game in that language. It
returns `GameOpenResultDto`: `opened` with the `ProjectOpenResultDto`, or
`sourceUpdateRequired` with the `SourceUpdateReportDto` of the plan, written
nowhere. `open_recent_project(projectId, acceptSourceUpdate?)` returns the
same DTO for a registry entry. After confirmation the renderer calls
`update_project_from_game(repositoryRoot)`, or `open_recent_project` with
`acceptSourceUpdate`; both open through
`ProjectSession::open_with_source_update` and return the applied report in
`ProjectOpenResultDto.sourceUpdate`. `update_project_from_game` is also the
path for an installed game after a patch. `preview_source_update(repositoryRoot)`
returns the plan without writing or changing the active project. A game
older than the project's game version is refused; see
[`rebase.md`](./rebase.md).

`initialize_project_from_game(repositoryRoot, sourceLanguage,
targetLanguage)` creates the folder when needed, opens the game in the source
language, and initializes the workspace; when it fails, a folder it created
is removed again if it is still empty. Opening, updating, and creating read
the cached sheet catalog, building it when needed, before the session is
installed.

`default_projects_directory_path` returns
`Documents/Aeria`, the folder for new projects and for clones without a
parent.

`ProjectSummaryDto.detachedUnitCount` reports detached units, and
`list_detached_units` returns each one's last binding, reason, target,
review state, and note. A plan that cannot be built maps to `sourceUpdate`.
Planning and applying remain in `aeria-rebase` and `aeria-workspace`; the
IPC layer only chooses whether to call the preview or the applying
constructor.

The game installation path is local runtime state held by the session's
`GameSource`; it is never added to Workspace Format. Public commands derive
the application cache directory
through Tauri's path API; React cannot choose an arbitrary cache root.
Translation browsing delegates to the bounded
`ProjectSession::page_translation_rows` API, so page size remains governed by
the backend contract. The DTO is row-centric, while each contained cell keeps
its existing `SourceBinding` and overlay. Tauri performs DTO and error mapping,
not business logic, and does not read the game or SQLite directly.

`translation_progress` returns per-sheet `SheetProgressDto` coverage for the
active project from `ProjectSession::translation_progress` (see
[`translation-read.md`](./translation-read.md#translation-progress)). The
renderer re-reads it after each committed translation mutation or workspace
reload and never derives sheet-wide progress from loaded row pages.
`app_info` returns the application name and version for display.

`macro_view(text, values)` describes macro text for the editor: its
diagnostics, its tags (opening, separator, closing, or inline, with their
catalog arguments), a game preview from `aeria_se::preview` evaluated with
`values` (the renderer's chosen values of preview variables, numbers or
texts by key such as `gn68`), and the variables the preview read with their
defaults, meanings, and row names, all with UTF-16 offsets. It reads
`UIColor` rows, sheet rows, and row ids from the open project's
`GameSource` (`GameSource::ui_color`, `GameSource::cell_text`, and
`GameSource::row_ids`); without a project, references are shown as values.
It changes nothing and needs no project.

Filesystem, game reading, SQLite, workspace loading, row paging, and ordinary
translation mutations run inside Tauri blocking workers. The async command
handlers do not hold `DesktopState` or the project mutex across an await;
worker-side access still goes through the single `ProjectSession` mutex, so
mutations remain serialized. Target, note, and review commands return the
compact committed `TranslationOverlayDto` for the changed cell; the renderer
patches that cell instead of reloading the current sheet.

Git collaboration commands (`git_overview`, `git_initialize`,
`git_set_identity`, `git_set_remote`, `git_remove_remote`,
`git_remote_branches`, `git_fetch_main`, `git_set_upstream`, `git_pending_changes`,
`git_project_changes`, `git_checkpoint`, `git_log`, `git_commit_changes`,
`git_unit_history`, `git_unit_attribution`, `git_contributors`, `git_sync`,
`git_branches`, `git_create_branch`, `git_switch_branch`,
`git_set_main_branch`, `git_state_stamp`, `git_finish_contribution`,
`git_merge_contribution`, `git_delete_branch`, and
`git_clone_repository`) delegate to
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
`angelica_image`, `angelica_send`, `angelica_cancel`, and
`angelica_delete_conversation`) work on the active project's conversations
described in [`ai.md`](./ai.md#angelica).
`angelica_send` validates the model selection against the AI settings,
checks and stores the message's base64 images (see
[`ai.md`](./ai.md#images)), appends the user message, stores the
conversation, and returns it before the turn runs; at most one turn runs per
conversation (`angelicaBusy`). A turn for a model that accepts images loads
the conversation's image files once before its first request.
`angelica_image` returns one image of a conversation as a `data:` URL for
display (`angelicaImageNotFound` when the conversation has no such image or
its file is gone). The turn
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
binding and its new `TranslationOverlayDto`. Guidance and glossary proposals
are applied to the repository root by the same command. `angelica_draft` produces one
draft with the default model and returns it without saving
(`aiNoAgentModel`, `angelicaUntaggable`, and `angelicaDraftRejected` are its
own errors).

A job proposal is applied by starting the job: `angelica_apply_proposal`
enumerates the scope under the project lock, creates the job in the
project's job store, and starts its runner; the proposal's message holds the
job ID. A runner is an async task registered per job in `DesktopState`; its
lanes run in a Tokio join set, so aborting the runner aborts them. The runner
keeps the job store and repository root it started with, and workers read and
write only while that project is still open. Worker tools run in blocking
workers that lock the project only for their own reads and writes, and every
written draft emits `angelica://translation-applied`. Job changes emit
`angelica://job` with the job ID. `angelica_jobs`, `angelica_job_units`,
`angelica_job_events`, `angelica_job_control` (pause, resume, cancel),
`angelica_job_retry` (requeue strings with given statuses and resume), and
`angelica_job_workers` (each lane's live activity, kept with the runner's
registration in `DesktopState` and empty once the runner ends) serve
the renderer; `ai_set_worker_model` sets the jobs model. When a job
completes or pauses on its own, the runner starts an automatic Angelica turn
in the job's conversation unless one is running.

Angelica's search tools and job workers' translation memory use
`DesktopSearch`. The source index of the active game source is built by a
background blocking task registered in `DesktopState` (building, ready, or
failed per source language and game version), from the session's shared
`GameSource`; see
[`search.md`](./search.md#desktop-use).

`project_guide`, `save_project_guidance`, and `save_project_glossary` read
and write the repository's guidance and glossary for the editor dialog. A save
goes through the same compare-and-rename write as an approved file proposal,
so a file changed since it was loaded is reported as `projectGuideConflict`
and not overwritten; an invalid glossary entry is `projectGuideInvalid`.

`fetch_url` runs in the turn's async task rather than a blocking worker,
with a separate HTTP client (no automatic redirects) kept in `DesktopState`.
Applying a web-access proposal updates the AI settings under their file lock
and then wakes Angelica. `ai_set_web_domains` replaces the allowed domains;
entries may be domains or links and are normalized, sorted, and
deduplicated.

Applying a review proposal holds the project lock while it compares each
recorded target with the workspace and calls `set_review_state`, emitting
`angelica://translation-applied` for every approved string.

Commands that require an active project report `noProjectOpen` before
validating project-scoped payload such as translation-unit IDs.

The IPC boundary contains no source update planning and no background
server. React has no direct filesystem or SQLite access. Translation-unit IDs
cross IPC only in their canonical textual form, and review states use an
explicit camelCase protocol enum.

Opening, updating, and creating a project run no child process and cannot be
cancelled. After the new session is installed, the desktop attempts the
recent-project upsert under the registry lock only.
