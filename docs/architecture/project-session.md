# Project session

`ProjectSession` is the Rust application-layer representation of one opened
Aeria project. It owns the local repository root, source-package path,
validated `aeria-hsp::SourcePackage`, verified immutable `HxsSnapshot`, loaded
sparse `Workspace`, and `WorkspaceStore`.

An HSP is Aeria's source handoff artifact. Its embedded HXS remains immutable
source data; its embedded HSG is the sole translation-permission authority.
The HXS is materialized into a disposable caller-provided local cache. HSP
and cache paths are runtime configuration only and are not persisted in
Workspace Format v1. React does not own authoritative project state.

## Opening an existing project

`ProjectSession::open(repository_root, source_package_path, cache_root)` performs these steps in order:

```text
validate HSP archive, HSG, embedded HXS, and their relationships
→ materialize and verify HXS in the managed cache
→ load Workspace Format v1
→ verify workspace/source compatibility
→ reject existing units blocked by HSG
→ create session
```

The compatibility check reuses the workspace source-binding contract for
source language, HXS content ID, and HXS snapshot ID. An incompatible source
is an open failure. Opening does not update workspace metadata, invoke source
update/rebase, or migrate project state.

## Initializing a project

`ProjectSession::initialize(repository_root, source_package_path, cache_root,
target_language)` performs these steps:

```text
validate HSP and verify its embedded HXS/HSG
→ create workspace metadata from the verified HXS
→ atomically initialize Workspace Format v1
→ create session
```

Existing `.aeria/` state is never replaced. Invalid HXS input and invalid
project inputs fail before new managed state is published.

Atlas-created projects use
`ProjectSession::initialize_from_source_package(repository_root, source_package,
target_language)`. The desktop creation workflow opens the published HSP once,
checks its package identity against Atlas's completed event, and transfers that
validated `SourcePackage` into the session. The constructor does not reopen the
archive, so source evidence validation is not repeated before Workspace Format
v1 initialization.

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
Search, indexes, and caches are disposable future layers. Source
update/rebase is a separate explicit workflow. Ordinary one-unit mutation and
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
