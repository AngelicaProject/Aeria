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
publication an `Export` guard for their whole run. `update_install` runs through `DesktopState::while_idle`, which
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

`initialize_project_from_game` refuses a `targetLanguage` that is not a
target language as defined in [`workspace.md`](./workspace.md#project-scope).
`set_project_target_language(targetLanguage)` changes the open project's
target language through `ProjectSession::set_target_language` and updates
the Recent projects entry; a failed registry update is returned as the
result's warning.

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

`source_in_other_languages(sourceBinding)` returns the source cell's
macro text in each client language other than the project's source
language, from `GameSource::cell_in_other_languages` (see
[`source.md`](./source.md#additional-source-languages)); a language without
the cell has `null` text. It is display context and is never recorded.

`sheet_dialogue(sheetName)` returns the dialogue structure of a quest or
cutscene sheet for the scene view, from `GameSource::dialogue` and
`GameSource::quest_row` (see [`source.md`](./source.md#dialogue)): each row
with text, its role (journal, objective, speech, or other), its speaker label,
and its row key after the `TEXT_<ID>_` prefix, the quest's name with its
translation from the `Quest` row, the other versions of the quest from
`GameSource::quest_versions`, and the scenes traced from the quest's
script from `GameSource::quest_script` (see
[`source.md`](./source.md#quest-scripts)), with line keys shortened the same
way. Each traced function has its scene number, or `null` with the
`handler` name it is assigned to, and the `script` name of the battle script
it comes from, or `null` for the quest's own script. Each answer of a
choice says when it is `available`: `always`, `never` (grayed out), `when`
a guard holds, or `unknown`. `cutscenes` lists every
cutscene file that names the sheet's lines, from
`GameSource::cutscenes_naming`, with its `Cutscene` row, path, those
keys, and the quest scenes that play it from `GameSource::cutscene_plays`.
A cutscene in a traced scene has its file's `path` when it resolves. A script or cutscene index that cannot be read leaves `scenes` empty
and is reported in `scriptError` rather than failing the command. It is
`null` for any other sheet and is never recorded.

`translation_progress` returns per-sheet `SheetProgressDto` coverage for the
active project from `ProjectSession::translation_progress` (see
[`translation-read.md`](./translation-read.md#translation-progress)). The
renderer re-reads it after each committed translation mutation or workspace
reload and never derives sheet-wide progress from loaded row pages.
`app_info` returns the application name and version for display.

`macro_view(text)` describes macro text for the editor: its diagnostics and
its tags (opening, separator, closing, or inline, with their catalog
arguments; on an opening color tag, the color it sets; and on an opening
`<if>` or `<switch>`, what it tests part by part: operands as numbers,
parameters with the meaning of a known global, or time values, with a
number compared with a row global such as the class named by its row), with
UTF-16 offsets. `<ui-color>` colors and row names are read from the open
project's `GameSource` (`GameSource::ui_color` and `GameSource::cell_text`);
without a project they are unknown. It changes
nothing and needs no project.

`game_glyph_font()` returns, as raw bytes, a TrueType font of the private use
glyphs of the open project's game font (`GameSource::private_glyphs`, drawn
by `aeria_fonts::bitmap_font`), and `game_icon(id)` an inline icon of
`<icon>` macros (`GameSource::icon`): its width and height as little-endian
16-bit numbers, then RGBA pixels. Both are empty without a project or when
the game has nothing to show.

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

`agents_status` reports and `agents_connect` sets up what agent harnesses
need (see [`agents.md`](./agents.md#discovery)); connecting fails with
`agentsConnect` naming the step. They run in blocking workers; changing the
user's `PATH` runs PowerShell's
`[Environment]::SetEnvironmentVariable` with the value in an environment
variable.

`project_knowledge`, `save_knowledge_style`, `save_knowledge_terms`, and
`save_knowledge_characters` read and write the
[project knowledge](../formats/knowledge-v1.md) files for the editor dialog. A
save replaces the file through a temporary file and rename only if it still
has the content the dialog loaded, so a file changed since, by hand, by Git,
or by an agent, is reported as `projectKnowledgeConflict` and not
overwritten; an invalid entry or a file over the size limit is
`projectKnowledgeInvalid`.

`set_translation_target`, `set_translation_note`, and
`set_translation_review_state` write through `DesktopState::write_project`:
it takes the project's cross-process write lock before the project mutex,
reloads the workspace when an `aeria` command wrote it since the session last
took its writes in, and then runs the write. A lock that cannot be taken is
`projectLock`. A background thread started at setup compares the command
stamp every 1.5 seconds, reloads under the same lock order, and emits
`project://workspace-reloaded`. See
[`agents.md`](./agents.md#sharing-a-project-between-processes).

Commands that require an active project report `noProjectOpen` before
validating project-scoped payload such as translation-unit IDs.

The IPC boundary contains no source update planning and no background
server. React has no direct filesystem or SQLite access. Translation-unit IDs
cross IPC only in their canonical textual form, and review states use an
explicit camelCase protocol enum.

Opening, updating, and creating a project run no child process and cannot be
cancelled. After the new session is installed, the desktop attempts the
recent-project upsert under the registry lock only.
