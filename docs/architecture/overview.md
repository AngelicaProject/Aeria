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
  |          |       |      |
 game     Workspace Git  AI
 source      |
   |      filesystem
installed game (read-only)
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

### React renderer

Authoritative only for presentation state, for example:

- selected tabs and panes
- panel sizes/layout
- temporary editor interaction state
- dialogs and transient filters

The renderer may cache Rust data for display, but cached data is never the canonical project state.

## Persistence

Three places hold data, with distinct roles:

- **Installed game**: the source, read directly and never written; see [`source.md`](./source.md).
- **Git workspace**: canonical user-authored translation/project state.
- **Local SQLite/cache**: rebuildable indexes, the sheet catalog, search data, and other machine-local acceleration/state.

Deleting local cache must never delete a user's translation work.

## Crates

- `aeria-core`: domain types and application contracts that should not know Tauri, Git implementation details, or SQLite.
- `aeria-sqpack`: read-only access to an installed game's SqPack archives and Excel sheets: the sheet list, sheet headers, and the rows and String cell bytes of a sheet in one language. It never writes to the installation. Its `testing` feature writes synthetic installations for tests.
- `aeria-source`: the installed game as the translation source: sheets in the source language, String cell text, sheet layouts, translation permission, row keys, the dialogue flow traced from quest scripts, and the sheet catalog.
- `aeria-se`: structured FFXIV string parsing, syntax tree, validation, and rendering model.
- `aeria-workspace`: versioned translation workspace model, deterministic serialization, and application of planned source updates.
- `aeria-rebase`: deterministic source update planning.
- `aeria-search`: local indexing, source search, translation memory, and query services.
- `aeria-knowledge`: the project knowledge in `aeria-knowledge/` (style, terms, character voices, story, lessons): reading, checking, and selecting what a scene needs, and the rules every translation follows. Aeria has no built-in model; external agents localize (see [`agents.md`](./agents.md)).
- `aeria-git`: repository operations and semantic Git integration, including HTTPS host credentials from the Git credential helper for forge adapters, and the merge check workflow template.
- `aeria-export`: Harmonia pack generation: unit selection, validation, the Pack Format v1 writer, signing, transport compression, and feed entries, and Pack Settings v1. String encoding is injected (`StringEncoder`); production uses the `aeria-se` codec.
- `aeria-fonts`: glyphs for game fonts that lack target-language characters: Font Settings v1, the supported game font sizes and their native metrics, the bundled recommended source fonts, rasterization, and the optional `FONTS` pack section. `aeria-export` writes the section into the pack.
- `aeria-check`: the `aeria-check` command for the CI of translation repositories: the integrity, translation, and merge stages of the [merge check](./git.md#merge-check-ci), built on the same readers as the desktop. It needs no game source and does not depend on Tauri.
- `aeria-publish`: pack publishing adapters: signing keys in the OS credential store, GitHub releases over the REST API, and the feed workflow template. It receives the GitHub credential from its caller and never stores it.

`apps/desktop/src-tauri` is an adapter/composition layer, not the home of domain logic.
