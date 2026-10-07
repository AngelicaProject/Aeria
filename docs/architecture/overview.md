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
  |          |          |        |
 game     project     Git    model
 source   (po/ files)          (machine translation)
   |          |
installed   filesystem
game (read-only)
```

## Authority

State belongs to the narrowest layer that is allowed to be authoritative for it.

### Rust application/domain

Authoritative for:

- project compatibility with the installed game
- the project's PO files: reading, writing, and checking translations
- macro structure and validation
- game updates
- Git operations and status
- machine translation runs
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
- **The project's files**, usually a Git repository: canonical project state, the PO files of [`po-project.md`](./po-project.md) and the project settings and knowledge.
- **Local cache and preferences**: rebuildable indexes, the sheet catalog, search data, and machine-local preferences and credentials.

Deleting local cache must never delete a user's translation work.

## Crates

- `aeria-core`: domain types shared by the game source, the project, and export: game versions, sheet layout hashes, and language tags. It knows nothing of Tauri, Git, or persistence.
- `aeria-sqpack`: read-only access to an installed game's SqPack archives and Excel sheets: the sheet list, sheet headers, and the rows and String cell bytes of a sheet in one language. It never writes to the installation. Its `testing` feature writes synthetic installations for tests.
- `aeria-source`: the installed game as the translation source: sheets in the source language, String cell text, sheet layouts, translation permission, row keys, the dialogue flow traced from quest scripts, and the sheet catalog.
- `aeria-se`: structured FFXIV string parsing, syntax tree, validation, and rendering model.
- `aeria-po`: the project as PO files: the format, entry identity, where each string's file is, files made from the game, the join of a game update, the checks of a translation, and the session the desktop reads and writes through (see [`po-project.md`](./po-project.md)).
- `aeria-knowledge`: the project knowledge in `aeria-knowledge/` (style and terms): reading and checking it, and the rules every translation follows (see [`knowledge.md`](./knowledge.md)).
- `aeria-model`: machine translation: the ChatGPT subscription sign-in, the Codex Responses client, and the run that translates the untranslated strings of `po/` (see [`translate.md`](./translate.md)).
- `aeria-search`: local indexing, source search, translation memory, and query services.
- `aeria-git`: repository operations and Git integration per string of the PO files, including HTTPS host credentials from the Git credential helper for forge adapters, and the Aeria Guard workflow template.
- `aeria-export`: Harmonia pack generation: selecting translations from `po/`, validation, the Pack Format v1 writer, signing, transport compression, and feed entries, and Pack Settings v1. String encoding is injected (`StringEncoder`); production uses the `aeria-se` codec.
- `aeria-fonts`: glyphs for game fonts that lack target-language characters: Font Settings v1, the supported game font sizes and their native metrics, the bundled recommended source fonts, rasterization, and the optional `FONTS` pack section. `aeria-export` writes the section into the pack.
- `aeria-guard`: the `aeria-guard` command for the CI of translation repositories: the integrity, translation, and changes stages of [Aeria Guard](./git.md#aeria-guard) and the review of a change, built on the same checks as the desktop. It needs no game and does not depend on Tauri; the Aeria Guard action (`guard/action.yml`) builds it from source.
- `aeria-publish`: pack publishing adapters: signing keys in the OS credential store, GitHub releases over the REST API, and the feed workflow template. It receives the GitHub credential from its caller and never stores it.

`apps/desktop/src-tauri` is an adapter/composition layer, not the home of domain logic.
