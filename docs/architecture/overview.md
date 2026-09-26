# Architecture overview

Aeria is a desktop application with a React/TypeScript interface and a Rust application core.

```text
React / TypeScript
       |
   typed Tauri IPC
       |
Tauri application boundary
       |
Rust application/domain crates
  |       |       |      |
 HSP/HXS Workspace Git  AI
   |              |
SQLite indexes/cache  filesystem
```

## Authority

State belongs to the narrowest layer that is allowed to be authoritative for it.

### Rust application/domain

Authoritative for:

- source bindings and project compatibility
- translation workspace state
- macro structure and validation
- rebase decisions
- Git operations and status
- export correctness
- AI job orchestration and validation

### React renderer

Authoritative only for presentation state, for example:

- selected tabs and panes
- panel sizes/layout
- temporary editor interaction state
- dialogs and transient filters

The renderer may cache Rust data for display, but cached data is never the canonical project state.

## Persistence

Three stores have distinct roles:

- **HXS**: immutable source snapshots.
- **HSP/HSG**: validated source handoff and exact translation permission.
- **Git workspace**: canonical user-authored translation/project state.
- **Local SQLite/cache**: rebuildable indexes, search data, AI jobs, and other machine-local acceleration/state.

Deleting local cache must never delete a user's translation work.

## Crates

The desktop-only aeria-atlas crate owns bounded Harmonia Atlas process
invocation: the `package` command with JSONL protocol v1 parsing, and the
`encode` command used by export. It does not depend on Tauri, React,
aeria-workspace, aeria-hsp, or aeria-export.

- `aeria-core`: domain types and application contracts that should not know Tauri, Git implementation details, or SQLite.
- `aeria-hxs`: HXS reader and verifier.
- `aeria-hsp`: HSP/HSG reader, relationship validator, source cache materializer, and guidance index.
- `aeria-se`: structured FFXIV string parsing, syntax tree, validation, and rendering model.
- `aeria-workspace`: versioned translation workspace model, deterministic serialization, and application of planned source updates.
- `aeria-rebase`: deterministic source update planning.
- `aeria-search`: local indexing, source search, translation memory, and query services.
- `aeria-ai`: provider-neutral AI orchestration and validated batch jobs.
- `aeria-git`: repository operations and semantic Git integration, including HTTPS host credentials from the Git credential helper for forge adapters, and the merge check workflow template.
- `aeria-export`: Harmonia pack generation: unit selection, validation, the Pack Format v1 writer, signing, transport compression, and feed entries, and Pack Settings v1. String encoding is injected (`StringEncoder`), so it does not depend on aeria-atlas.
- `aeria-fonts`: glyphs for game fonts that lack target-language characters: Font Settings v1, the supported game font sizes and their native metrics, the bundled recommended source fonts, rasterization, and the optional `FONTS` pack section. `aeria-export` writes the section into the pack.
- `aeria-check`: the `aeria-check` command for the CI of translation repositories: the integrity, translation, and merge stages of the [merge check](./git.md#merge-check-ci), built on the same readers as the desktop. It needs no game source and does not depend on Tauri.
- `aeria-publish`: pack publishing adapters: signing keys in the OS credential store, GitHub releases over the REST API, and the feed workflow template. It receives the GitHub credential from its caller and never stores it.

`apps/desktop/src-tauri` is an adapter/composition layer, not the home of domain logic.
