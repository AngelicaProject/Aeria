# Project session

`ProjectSession` is the Rust application-layer representation of one opened
Aeria project. It owns the local repository root, source-package path,
validated `aeria-hsp::SourcePackage`, verified immutable `HxsSnapshot`, loaded
sparse `Workspace`, and `WorkspaceStore`.

An HSP is Aeria's source handoff artifact. Its embedded HXS remains immutable
source data; its embedded HSG is the sole translation-permission authority.
The HXS is materialized into a disposable caller-provided local cache. HSP
and cache paths are runtime configuration only and are not persisted in the
workspace. React does not own authoritative project state.

## Opening an existing project

`ProjectSession::open(repository_root, source_package_path, cache_root)` performs these steps in order:

```text
validate HSP archive, HSG, embedded HXS, and their relationships
→ materialize and verify HXS in the managed cache
→ read and validate the stored workspace (v2, or v1 for migration)
→ require the same source language
→ require that no source update is needed
→ activate the workspace and create the session
```

`ProjectSession::open` validates the HSP once and delegates the already
validated package to `ProjectSession::open_from_source_package(repository_root,
source_package)`. The latter owns workspace loading, compatibility, and
session construction. It uses `source_package.package_path()` for the runtime
HSP path and never adds that path to the workspace.

A different source language is a `Compatibility` failure. A workspace that is
not current for the package fails with `SourceUpdateRequired` and one
requirement: a Workspace Format v1 migration, a different `contentId`, or
bound units no longer permitted by the package guidance. A package whose
`contentId` equals the workspace's opens directly even when its game version
or `snapshotId` differs. Opening never writes.

## Opening with a source update

`ProjectSession::preview_source_update(repository_root, &source_package)`
plans the update against the package without writing and returns a
`SourceUpdateReport` with the plan and the stored format version.

`ProjectSession::open_with_source_update(repository_root, source_package)`
opens a current workspace directly, or else plans the update, publishes the
changed shards and then the manifest, reloads the published state, and
returns the session with the applied report. The workflow and its guarantees
are described in [`rebase.md`](./rebase.md).

## Initializing a project

`ProjectSession::initialize(repository_root, source_package_path, cache_root,
target_language)` performs these steps:

```text
validate HSP and verify its embedded HXS/HSG
→ create workspace metadata from the verified HXS
→ atomically initialize the workspace (Workspace Format v2)
→ create session
```

Existing `.aeria/` state is never replaced. Invalid HXS input and invalid
project inputs fail before new managed state is published.

Atlas-created projects use
`ProjectSession::initialize_from_source_package(repository_root, source_package,
target_language)`. The desktop creation workflow opens the published HSP once,
checks its package identity against Atlas's completed event, and transfers that
validated `SourcePackage` into the session. The constructor does not reopen the
archive, so source evidence validation is not repeated before workspace
initialization.

## Reloading after repository changes

`ProjectSession::reload_workspace()` reloads the workspace from disk after
the repository changed outside the ordinary mutation path, such as a Git
merge. The reloaded workspace passes the same validation and compatibility
checks as opening; a reload that would require a source update fails. On failure the session
keeps its previous state; the Git integration that triggered the reload is
then rolled back.

`ProjectSession::reload_and_reconcile_workspace()` is the reload used after
Git operations. When the reloaded state records the session's source content
but some bound units do not describe it (a source facts mismatch or a guidance
permission change), it applies the deterministic source update and returns
its report. State that records other source content, or an older format, is
never reconciled here and fails with `SourceUpdateRequired`, because it needs
that source's package.

## Ownership and scope

```text
ProjectSession
├── local repository root
├── local HSP path
├── validated SourcePackage and GuidanceIndex
├── verified immutable HxsSnapshot
├── loaded sparse Workspace
└── WorkspaceStore
```

The session does not enumerate or materialize the full HXS source corpus.
Search, indexes, and caches are disposable future layers. A source update is
a separate explicit workflow that runs only through
`open_with_source_update`. Ordinary one-unit mutation and
persistence orchestration belongs to the focused session mutation layer; callers
still do not receive unrestricted mutable access to the workspace or store.

The session also exposes the read-only bounded translation browsing layer
described in [`translation-read.md`](./translation-read.md), and the
transactional ordinary-editor mutation layer described in
[`translation-mutations.md`](./translation-mutations.md). Reads compose one
verified per-sheet HXS source page with sparse Workspace state. Mutations
verify the exact current source, apply existing Workspace domain semantics,
and persist one canonical shard without giving callers mutable persistence
access.

The desktop application boundary described in
[`desktop-application-boundary.md`](./desktop-application-boundary.md) owns
the process-level active-session slot and delegates open/initialize, bounded
reads, and ordinary mutations to this API.
