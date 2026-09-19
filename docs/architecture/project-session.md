# Project session

`ProjectSession` is the Rust application-layer representation of one opened
Aeria project. It owns the local repository root, local HXS path, verified
immutable `HxsSnapshot`, loaded sparse `Workspace`, and `WorkspaceStore`.

The HXS artifact is external, immutable, and local to the machine. Its
filesystem path is runtime configuration only; it is not persisted in
Workspace Format v1. React does not own authoritative project state.

## Opening an existing project

`ProjectSession::open` performs these steps in order:

```text
verify HXS
→ load Workspace Format v1
→ verify workspace/source compatibility
→ create session
```

The compatibility check reuses the workspace source-binding contract for
source language, HXS content ID, and HXS snapshot ID. An incompatible source
is an open failure. Opening does not update workspace metadata, invoke source
update/rebase, or migrate project state.

## Initializing a project

`ProjectSession::initialize` performs these steps:

```text
verify HXS
→ create workspace metadata from the verified HXS
→ atomically initialize Workspace Format v1
→ create session
```

Existing `.aeria/` state is never replaced. Invalid HXS input and invalid
project inputs fail before new managed state is published.

## Ownership and scope

```text
ProjectSession
├── local repository root
├── local HXS path
├── verified immutable HxsSnapshot
├── loaded sparse Workspace
└── WorkspaceStore
```

The session does not enumerate or materialize the full HXS source corpus.
Search, indexes, and caches are disposable future layers. Source
update/rebase is a separate explicit workflow. Session-level mutation and
persistence orchestration is intentionally outside this layer; callers do not
receive unrestricted mutable access to the workspace or store.
