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
source-package path. The Create surface accepts a repository root, game
installation, and one of Atlas's supported source languages (`en`, `ja`, `de`, or
`fr`). Until project settings can choose a real target language, creation uses
the explicit neutral compatibility tag `und`; it is never displayed as a user
translation target and does not reinterpret existing overlays. Once a project is active, the
renderer uses the sheets in
`ProjectSummaryDto`, pages one sheet at a time with a limit of 100 entries, and
offers an explicit **Load more** action.

The renderer owns only ephemeral drafts, navigation, paging, and loading/error presentation. Rust remains authoritative for source bindings, workspace state, validation, classification, and mutation semantics. The middle pane receives row pages but renders a flattened occurrence view: one `TranslationCellDto` is one Lens lane, identified by `sheetName`, `rowId`, `subrowId`, and `columnIndex`. A logical row remains the grouping context. The editor selects one occurrence at a time and keeps compact field navigation for multi-cell rows; unrelated drafts remain in the row-local draft map. Until EXDSchema exists, fields are labelled `Column N`.

Each cell has an independent target draft, note draft, translation-unit ID,
and review state. Target and note text do not autosave: both use explicit save
actions. Review-state changes persist immediately. Mutations still address one
cell-level `SourceBinding` at a time. A row is dirty when any contained cell
has a dirty target or note. After every successful mutation, the renderer
re-reads the bounded row pages currently visible from Rust, rebuilds drafts
from authoritative DTOs, and restores the selected row coordinate.

Unsaved target or note drafts are marked and protected by a discard confirmation when changing rows, changing sheets, closing the project, or performing a mutation that would refresh away another dirty cell draft. The UI distinguishes an absent translation overlay from an overlay whose target is explicitly empty.

Lens previews show one occurrence per lane with the `rowId:subrowId`
coordinate on the first lane, its `col N` field identity, clipped source and
target previews, and that occurrence's review state. Blocked source cells remain
context only and are never used as permission heuristics.
`Load more` continues to be explicit; a source page may return zero visible
rows while its row cursor still has more source work.

The Sheets tool is an IDE-style presentation of the already-loaded
`ProjectSheetDto[]`: slash-separated names form collapsible folders for display
only, while the canonical sheet name is passed unchanged to Rust. Quick Find
is a transient renderer-only overlay (Ctrl+F or the Sheets header action), not a
permanent filter field. Project Search is a separate workbench tool and shows a
truthful unavailable state until a domain search backend exists.

Center documents use preview tabs for single-click sheet browsing. A second
click pins a preview, and starting a local draft pins it automatically.
Pinned/preview tabs can be activated, closed, reordered, and closed with
Ctrl+W; each tab's dirty indicator is presentation state only. Dock panels
retain serializable presentation state, can be resized, and floatable tools
use real Tauri webview windows that share the active Rust project session.

This first editor intentionally has no structured macro controls, source
update/rebase UI, or export. The workbench retains truthful Git and AI dock
slots, bottom-panel tabs, and status/layout infrastructure even when those
backends are unavailable. Macro text is shown literally with whitespace
preserved.

Registry load failures show a dismissible, non-blocking launcher warning while
manual Open project and Create project remain available. After dismissal, the
Recent projects section remains in a stable unavailable state rather than
returning to its loading state. A successful project launch with a local
registry write warning enters the editor normally and shows the warning at the
application level. The launcher never automatically reopens the last project.
