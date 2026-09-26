# Project session

`ProjectSession` is the Rust application-layer representation of one opened
Aeria project. It owns the local repository root, the opened game source
(`aeria-source::GameSource`), the loaded sparse `Workspace`, and its
`WorkspaceStore`.

The game path is runtime configuration and is not persisted in the
workspace. React does not own authoritative project state.

## Opening an existing project

`ProjectSession::open(repository_root, source)` performs these steps in
order:

```text
read and validate the stored workspace
→ require the game's source language
→ require a game version not older than the project's
→ plan the source update against the game
→ require that the plan changes nothing
→ activate the workspace and create the session
```

The plan reads every sheet that holds units once. A project is current for
the game when its `gameVersion` equals the game's version and the plan
changes no unit. Otherwise opening fails:

| Condition | Error |
| --- | --- |
| Another source language | `Compatibility` |
| The game is older than the project | `GameOutdated` |
| The game is newer than the project | `SourceUpdateRequired(GameUpdated)` |
| Same version, but units do not describe the game | `SourceUpdateRequired(SourceFactsMismatch)` |

Opening never writes.

## Opening with a source update

`ProjectSession::preview_source_update(repository_root, &source)` plans the
update without writing and returns a `SourceUpdateReport` with the plan.

`ProjectSession::open_with_source_update(repository_root, source)` opens a
current workspace directly, or else applies the plan, publishes the changed
shards and then the manifest, reloads the published state, and returns the
session with the applied report. The workflow and its guarantees are
described in [`rebase.md`](./rebase.md).

## Initializing a project

`ProjectSession::initialize(repository_root, source, target_language)`
creates workspace metadata from the game's source language and version and
atomically publishes a new `.aeria/` directory in Workspace Format v3.
Existing `.aeria/` state is never replaced.

## Reloading after repository changes

`ProjectSession::reload_workspace()` reloads the workspace from disk after
the repository changed outside the ordinary mutation path, such as a Git
merge. The reloaded workspace passes the same checks as opening; a reload
that would require a source update fails. On failure the session keeps its
previous state; the Git integration that triggered the reload is then rolled
back.

`ProjectSession::reload_and_reconcile_workspace()` is the reload used after
Git operations. When the reloaded state records the session's game version
but some units do not describe the game (a source facts mismatch), it applies
the deterministic source update and returns its report. State that records
another game version is never reconciled here and fails with
`SourceUpdateRequired`.

## Ownership and scope

```text
ProjectSession
├── local repository root
├── GameSource (shared, with its sheet cache and catalog)
├── loaded sparse Workspace
└── WorkspaceStore
```

The session does not enumerate the source corpus. The game source reads
sheets on demand and keeps recently read ones in memory. A source update is
a separate explicit workflow that runs only through `open_with_source_update`
and `reload_and_reconcile_workspace`. Ordinary one-unit mutation and
persistence belong to the focused session mutation layer; callers do not
receive unrestricted mutable access to the workspace or store.

The session also exposes the read-only bounded translation browsing layer
described in [`translation-read.md`](./translation-read.md), and the
transactional ordinary-editor mutation layer described in
[`translation-mutations.md`](./translation-mutations.md).

The desktop application boundary described in
[`desktop-application-boundary.md`](./desktop-application-boundary.md) owns
the process-level active-session slot and delegates open, initialize,
bounded reads, and ordinary mutations to this API.
