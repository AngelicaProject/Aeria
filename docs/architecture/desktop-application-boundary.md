# Desktop application boundary

The desktop process owns one active project session and exposes the Rust
application API to the renderer through typed Tauri commands:

```text
React
  ↓ invoke
Tauri command DTOs
  ↓
DesktopState
  ↓
one aeria_po::Session
  ↓
the project's files and the installed game
```

`DesktopState` holds the open session as `Mutex<Option<Arc<Session>>>` and a
separate narrow mutex for local recent-project registry file operations. Rust
owns the authoritative project state, which is the project's files; React
receives owned DTO snapshots only. There is one active project per desktop
process. Opening or creating a replacement constructs and verifies the new
session before it replaces the active one, so failure preserves the previous
session. Closing is idempotent and drops the active session without changing
any file.

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
`<app-data>/projects-v2.json`. It is application-local state, not project
data, and is never written to a project, a translation repository, or the
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
the game or opening a project. Ready entries can be opened by opaque ID
through this sequence:

```text
repository existence check
→ GameSource::open on the configured installation in the remembered
   source language
→ Session::open with that game
   (or a game update first when the caller accepted one)
→ active-project replacement
→ best-effort registry refresh
```

`aeria.json`, not the registry, decides whether the game fits the project; the registry's language and game version are display data, and the
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
commands (checkpoint, commit, sync, branch switch, clone) hold a `Sync` activity guard and pack export and
publication an `Export` guard for their whole run. A machine translation run
is not guarded; an update that restarts the application stops it, and
starting it again continues. `update_install` runs through `DesktopState::while_idle`, which
fails with `updateBusy` while any of them runs and holds the activity lock
while the installer starts, so no guarded operation begins in between. The
renderer additionally reports unsaved editor drafts and saves in flight to
the update store and does not request installation while one exists.

## Opening, creating, and updating projects

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

`open_project_from_game(repositoryRoot)` reads the source language from
`aeria.json` and opens the configured game in that language. It returns
`GameOpenResultDto`: `opened` with the `ProjectOpenResultDto`, or
`sourceUpdateRequired` with the project's and the game's versions when the
project's files are for an older game version, written nowhere. A game older
than the project is refused with `gameOutdated`, a game in another source
language with `projectCompatibility`, and a folder without `aeria.json` with
`projectMissing`. `open_recent_project(projectId, acceptSourceUpdate?)`
returns the same DTO for a registry entry. After confirmation the renderer
calls `update_project_from_game(repositoryRoot)`, or `open_recent_project`
with `acceptSourceUpdate`: the [game update](./po-project.md#game-updates)
runs, is committed when the project is a repository with an identity, and the
report (files, fuzzy and obsolete translations, the commit) comes back in
`ProjectOpenResultDto.sourceUpdate`. An update refuses with
`gitUncommittedTranslations` while `po/` or `aeria.json` has uncommitted
changes.

`initialize_project_from_game(repositoryRoot, sourceLanguage,
targetLanguage)` creates the folder when needed, opens the game in the source
language, and writes the project (see
[`po-project.md`](./po-project.md#editing)); when it fails, a folder it
created is removed again if it is still empty. A folder that already has
`aeria.json` or `po/` fails with `projectExists`. Opening, updating, and
creating read the cached sheet catalog, building it when needed, before the
session is installed.

`initialize_project_from_game` refuses a `targetLanguage` that is not a
BCP 47 language tag of a language other than `und`
(`invalidTargetLanguage`). `set_project_target_language(targetLanguage)`
writes the open project's target language to `aeria.json` and updates the
Recent projects entry; a failed registry update is returned as the result's
warning.

`default_projects_directory_path` returns
`Documents/Aeria`, the folder for new projects and for clones without a
parent.

The game installation path is local runtime state held by the session's
`GameSource`; it is never written to a project. Public commands derive the
application cache directory through Tauri's path API; React cannot choose an
arbitrary cache root. `page_translation_rows(sheetName, after, limit)` reads
a page of at most 256 source rows through `Session::page`; each cell carries
its coordinate (`SourceBinding`) and its translation (`TranslationOverlayDto`:
the text, whether it is fuzzy, the note, and the previous source of a fuzzy
one). Tauri performs DTO and error mapping, not business logic.

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

`translation_progress` returns per sheet the strings, translated strings, and
fuzzy strings of the project from `Session::progress`; the first call reads
every file, later calls only changed files. The renderer re-reads it after
each save and never derives sheet-wide progress from loaded row pages.
`app_info` returns the application name and version for display.

`macro_view(text)` describes macro text for the editor: its diagnostics and
its tags (opening, separator, closing, or inline, with their catalog
arguments; on an opening color tag, the color it sets; and on an opening
`<if>` or `<switch>`, what it tests part by part: operands as numbers,
parameters with the meaning of a known global, or time values, with a
number compared with a row global such as the class named by its row), with
UTF-16 offsets. Each tag also has the `rule` the structure policy gives its
macro (`aeria_se::construct_rule`: `keep`, `formatting`, `condition`,
`free`, or `letterCase`; raw bytes are kept). `<ui-color>` colors and row
names are read from the open project's `GameSource` (`GameSource::ui_color`
and `GameSource::cell_text`); without a project they are unknown. It changes
nothing and needs no project.

`string_hints(sourceBinding)` returns what a machine translation request
tells the model about one string, from `aeria_model::hints` over the
string's entry (`Session::entry`): the game's names in its source with the
project's translations, each with every name sheet that has it, the row's
name for another form of it, and the coordinate of the string its
translation comes from (`Session::coordinate_of`), the glossary terms of its source (with their notes,
forbidden variants, and whether a term exception keeps one out), its speaker
label with the name it stands for, the kind of a quest's text, the length of
an interface label, and the texts whose line varies with the player
character's gender. The project's translated names are read once and again
only when a file of a name sheet changed (`hints::NamesCache`).
`check_draft(sourceBinding, text)` checks a translation before it is saved:
the issues of `Session::check_text`, the same checks a save runs, the
source's game data the text lacks (`aeria_se::missing_game_data`), the names
of the hints whose translation it uses
(`aeria_knowledge::uses_translation`), and the characters it shows. Neither
command writes anything.

`game_glyph_font()` returns, as raw bytes, a TrueType font of the private use
glyphs of the open project's game font (`GameSource::private_glyphs`, drawn
by `aeria_fonts::bitmap_font`), and `game_icon(id)` an inline icon of
`<icon>` macros (`GameSource::icon`): its width and height as little-endian
16-bit numbers, then RGBA pixels. Both are empty without a project or when
the game has nothing to show.

Filesystem, game reading, row paging, and saves run inside Tauri blocking
workers. The async command handlers do not hold `DesktopState` across an
await. `set_translation_target(sourceBinding, targetMacro)` and
`set_translation_note(sourceBinding, note)` save one string through the
session, which makes its saves one at a time, and return the string's
`TranslationOverlayDto` (or `null` for a string left untranslated and without
a note); the renderer patches that cell instead of reloading the sheet. A
translation the checks refuse fails with `translationInvalid`, whose message
lists the problems.

Project search commands (`project_search`, `project_search_prepare`,
`project_search_cancel`, `project_search_entries`, `project_replace_preview`,
`project_replace_apply`, `project_edit_undo`, and `project_retranslate`)
delegate to the open session's search corpus and `Session::apply_edits`;
see [`search.md`](./search.md#project-search). `project_search_prepare`
reads the files changed since the last search, so the next one does not
wait. A new search or preview cancels the one in progress. `SearchState` keeps the inverse of the last bulk edit
for `project_edit_undo`. `project_retranslate` clears the strings and starts
a machine translation run of exactly them; it refuses while a run goes.

Git collaboration commands (`git_overview`, `git_initialize`,
`git_set_identity`, `git_set_remote`, `git_remove_remote`,
`git_remote_branches`, `git_set_upstream`, `git_pending_changes`,
`git_pending_sheet_changes`,
`git_project_changes`, `git_checkpoint`, `git_log`, `git_commit_changes`,
`git_string_history`, `git_branches`, `git_create_branch`,
`git_switch_branch`, `git_state_stamp`, `git_delete_branch`,
`git_delete_remote_branch`, and `git_clone_repository`) delegate to `aeria-git` for the active project's
repository root; see [`git.md`](./git.md). The Git executable is selected
once at application setup (override, bundled runtime, then `PATH`) and kept in
`DesktopState`. Checkpoint, the integration step of pull, and branch switches
hold the session's writes so they cannot
interleave with saves; fetch and push run without them. An integration whose
result is for another game version is rolled back. Strings changed
differently on both sides are returned in the pull result, not as an error,
so the renderer can collect a resolution per string and pull again. String
changes carry each string's coordinate in the game when it has one.
`git_pending_changes` returns how many strings have uncommitted changes and
the first 500 of those changes, and `git_pending_sheet_changes(sheetName)`
every change of one sheet's files. Both read the changed PO files through a
cache in `DesktopState` that reads a file again only when its size or
modification time, or `HEAD`, changed. Git
failures map to stable `git*` error codes such as `gitUnavailable`,
`gitIdentityMissing`, `gitMergeConflict`, `gitIncomingRejected`, and
`gitInvalidSettings`.

`model_account`, `model_sign_in_start`, `model_open_sign_in_page`,
`model_sign_in_poll`, `model_sign_out`, and `model_list` sign in to a ChatGPT
subscription and list its models; `translation_name_sheets` lists the name
sheets a run translates first, and
`translation_start(scope, fuzzy, model, effort)`, `translation_status`, and
`translation_stop` run [machine translation](./translate.md). A scope names
sheets, and folders of sheets ending with `/`; empty is the whole project. One
run goes at a time (`translationRunning`); it runs on the async runtime and
its status is read while it goes. Model failures map to `model*` codes such as
`modelSignInRequired` and `modelUsageLimit`.

`project_knowledge`, `save_knowledge_style`, and `save_knowledge_terms` read
and write the [project knowledge](../formats/knowledge-v1.md) files for the
editor dialog. A save replaces the file through a temporary file and rename
only if it still has the content the dialog loaded, so a file changed since,
by hand or by Git, is reported as `projectKnowledgeConflict` and not
overwritten; an invalid entry or a file over the size limit is
`projectKnowledgeInvalid`.

A background thread started at setup asks the session every 1.5 seconds which
of the sheets the editor has shown changed on disk, by Git, by hand, or by a
machine translation run, and emits `project://files-changed` with their names;
the renderer reloads the open sheet. The same thread emits `project://changed`
when the project's files changed in any way the session knows of: the session
counts its own writes (a save, a bulk edit, machine translation) and Git
operations that change the working tree report themselves to it. Views of the
whole project, such as search, read their result again.

Commands that require an active project report `noProjectOpen` before
validating project-scoped payload.

React has no direct filesystem access. Strings cross IPC by their coordinate
(`SourceBinding`) and, in Git changes, their `msgctxt`.

Opening and creating a project run no child process and cannot be cancelled;
updating runs Git to check the working tree and commit the update. After the new session is installed, the desktop attempts the
recent-project upsert under the registry lock only.
