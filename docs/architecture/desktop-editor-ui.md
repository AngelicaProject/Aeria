# Desktop editor UI

The desktop renderer provides the first human translation-editing vertical slice over the existing Tauri application boundary.

## Flow

```text
project launcher
→ active project header
→ sheet selection
→ bounded translation rows
→ selected row with per-cell source/target editors
→ explicit target and note persistence
→ immediate review-state persistence
```

The launcher first loads the local Recent projects registry. Loading is
presentation-only: the renderer receives typed recent-project DTOs and cheap
filesystem availability states (`ready`, `repositoryMissing`,
`sourcePackageMissing`, or `repositoryAndSourceMissing`). It does not read
`projects-v1.json` or inspect app-data paths directly. Missing entries remain
visible and offer **Remove from recents**; ready entries offer **Open**.

The launcher can also open an existing project with a repository root and HSP
source-package path, or create a project from a repository root, game
installation, and one of Atlas's supported source languages (`en`, `ja`, `de`, or
`fr`). Target language is configured after creation through the project settings
boundary. During creation it displays typed Atlas phase and
progress events and offers cancellation. Raw stdout is never exposed to the
renderer. Once a project is active, the renderer uses the sheets in
`ProjectSummaryDto`, pages one sheet at a time with a limit of 100 entries, and
offers an explicit **Load more** action.

The renderer owns only ephemeral drafts, navigation, paging, and loading/error presentation. Rust remains authoritative for source bindings, workspace state, validation, classification, and mutation semantics. The middle pane selects a logical row by `sheetName`, `rowId`, and `subrowId`; the editor renders context cells read-only and every translatable cell in physical column order. Until EXDSchema exists, fields are labelled `Column N`.

Each cell has an independent target draft, note draft, translation-unit ID,
and review state. Target and note text do not autosave: both use explicit save
actions. Review-state changes persist immediately. Mutations still address one
cell-level `SourceBinding` at a time. A row is dirty when any contained cell
has a dirty target or note. After every successful mutation, the renderer
re-reads the bounded row pages currently visible from Rust, rebuilds drafts
from authoritative DTOs, and restores the selected row coordinate.

Unsaved target or note drafts are marked and protected by a discard confirmation when changing rows, changing sheets, closing the project, or performing a mutation that would refresh away another dirty cell draft. The UI distinguishes an absent translation overlay from an overlay whose target is explicitly empty.

Row list previews show the `rowId:subrowId` coordinate, the first
guidance-allowed source text, and a neutral translated-fields count. Blocked
source cells remain context only and are never used as permission heuristics.
`Load more` continues to be explicit; a source page may return zero visible
rows while its row cursor still has more source work.

This first editor intentionally has no file picker, search or filtering, virtualization, structured macro controls, source update/rebase UI, export, Git controls, or AI controls. Macro text is shown literally with whitespace preserved.

Registry load failures show a dismissible, non-blocking launcher warning while
manual Open project and Create project remain available. After dismissal, the
Recent projects section remains in a stable unavailable state rather than
returning to its loading state. A successful project launch with a local
registry write warning enters the editor normally and shows the warning at the
application level. The launcher never automatically reopens the last project.
