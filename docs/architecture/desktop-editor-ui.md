# Desktop editor UI

The desktop renderer provides the human translation-editing vertical slice over the existing Tauri application boundary.

## Flow

```text
project launcher
→ workbench with the first sheet that has translatable strings
→ sheet selection
→ bounded translation rows in the strings list
→ selected occurrence in the editor below the list
→ explicit target and note persistence
→ immediate review-state persistence
```

The renderer owns only ephemeral drafts, navigation, paging, filtering of loaded
rows, and loading/error presentation. Rust remains authoritative for source
bindings, workspace state, validation, classification, and mutation semantics.

## Launcher

The launcher is a single screen. The left column holds the product name, the
**Open project** and **New project** actions, and the application version from
`app_info`. The right panel shows Recent projects by default; choosing an action
replaces it with that action's form, and **Back** returns to the list. Settings
open in a dialog from the titlebar.

Recent projects load from the local registry. Loading is presentation-only: the
renderer receives typed recent-project DTOs and cheap filesystem availability
states (`ready`, `repositoryMissing`, `sourcePackageMissing`, or
`repositoryAndSourceMissing`). It does not read `projects-v1.json` or inspect
app-data paths directly. Each row shows the repository name and path, source
language, game version, and when it was last opened. Missing entries remain
visible with their availability state and offer **Remove from recent projects**;
ready entries open on click. A name filter appears once more than three
projects are listed.

Open project takes a repository root and HSP source-package path. New project
takes a repository folder, game installation, and one of Atlas's supported
source languages (`en`, `ja`, `de`, or `fr`). Until project settings can choose
a real target language, creation uses the explicit neutral compatibility tag
`und`; it is never displayed as a user translation target and does not
reinterpret existing overlays. While Atlas runs, the form shows the current
phase, per-sheet progress when Atlas reports a sheet index and count, and a
cancel action.

Registry load failures show a dismissible, non-blocking launcher warning while
manual Open project and New project remain available. After dismissal, the
Recent projects section remains in a stable unavailable state rather than
returning to its loading state. A successful project launch with a local
registry write warning enters the editor normally and shows the warning at the
application level. The launcher never automatically reopens the last project.

## Workbench layout

```text
titlebar: menus · project / sheet · layout toggles · window controls
left rail | Sheets or Search | sheet tabs             | Git or AI | right rail
          |                  | strings list           |           |
          |                  | (resizable split)      |           |
          |                  | translation editor     |           |
          |                  | optional bottom panel  |           |
status bar
```

The titlebar carries the File, Translation, Go, and View menus, a command
center showing the project and active sheet, and toggles for the left, bottom,
and right regions. Double-clicking an empty titlebar area maximizes or restores
the window natively. Left and
right docks, the editor height, and the bottom panel are resizable. Dock panels
keep serializable presentation state, can move between regions, and floatable
tools open in real Tauri webview windows that share the active Rust project
session. The right dock opens on Git; the bottom panel (Tasks, Git changes,
Diagnostics) is hidden by default and shows truthful unavailable states.

Center documents use preview tabs for single-click sheet browsing. A
double-click pins a preview, and starting a local draft pins it automatically.
Tabs can be activated, closed, reordered, and closed with Ctrl+W; each tab's
dirty indicator is presentation state only.

## Command palette

The command center (Ctrl+P) opens a palette at the top of the workbench. The
input prefix selects its mode:

| Prefix | Mode |
| --- | --- |
| none | Go to a sheet by fuzzy name match; empty input lists recently opened sheets first |
| `>` | Run a workbench command, including theme switching (Ctrl+Shift+P) |
| `:` | Go to `row`, `row:subrow`, or `row:subrow:column` in the active sheet (Ctrl+G) |
| `#` | Project string search; shown as unavailable until a search backend exists |
| `?` | List the prefixes |

Going to a row that is not loaded pages the sheet so it starts at that row. The
list then shows a "Showing rows from" notice with **Load from the start**, and
**Load more** continues forward. A coordinate without a translatable string
selects the first loaded row and shows a warning.

## Strings list

The list receives row pages but renders a flattened occurrence view: one
`TranslationCellDto` is one lane, identified by `sheetName`, `rowId`,
`subrowId`, and `columnIndex`. A logical row remains the grouping context: the
`rowId:subrowId` coordinate appears on its first lane and continuation lanes
show a connector. Each lane shows its review state, `col N` field identity, and
single-line source and target previews in which macro spans are tinted. Until
EXDSchema exists, fields are labelled by column. Blocked source cells remain
context only and are never used as permission heuristics.

The list is virtualized, pages one sheet at a time with a limit of 100 entries,
and keeps **Load more** explicit; a source page may return zero visible rows
while its row cursor still has more source work. A text filter and a review
state filter (untranslated, draft, needs review, reviewed) narrow the loaded
occurrences only and are labelled as such. The toolbar shows sheet-wide
coverage from `translation_progress`, never a figure derived from loaded pages.
Strings with uncommitted Git changes carry a gutter marker (added or modified)
derived from `git_pending_changes`.

## Translation editor

The editor sits below the list and edits one occurrence at a time. Its bar shows
the occurrence's review state and coordinate, field tabs for multi-cell rows,
a Text / In-game view switch, the review-state control, and Revert. The In-game
view is disabled with a truthful unavailable state until a semantic preview
exists; the layout reserves it so a preview can replace the text panes without
restructuring the editor.

Source and target sit side by side, with the translator note beside them (or
below them in a narrow document). Source is read-only; target is a CodeMirror
editor. Both highlight Lumina macro spans with a presentation-only scanner;
Rust remains the authority for parsing and validation, and macro text is never
rewritten by the highlighter. Row context cells are available in a collapsible
section under the source. **Copy source to target** replaces the target draft
with the source macro text.

Each cell has an independent target draft, note draft, translation-unit ID,
and review state. Target and note text do not autosave: both use explicit save
actions. Empty or whitespace-only target drafts cannot be saved; the editor
keeps Save disabled until the target contains non-whitespace content, and the
Rust mutation API enforces the same rule. Review-state changes persist
immediately. Mutations still address one cell-level `SourceBinding` at a time,
and each successful mutation patches that cell from the returned
`TranslationOverlayDto`. A row is dirty when any contained cell has a dirty
target or note.

When the selected string has uncommitted Git changes, the target pane shows a
word-level diff between the last checkpoint and the current draft, or notes that
the string is new since then. The diff is presentation only and can be hidden.

**Save & next** saves the target, waits until the saved overlay is applied and
no dirty draft remains, then selects the next occurrence in the filtered list
and, unless disabled in settings, focuses its target. When there is nothing to
save it only moves on.

Unsaved target or note drafts are marked and protected by a discard
confirmation when changing rows, changing sheets, closing the project, or
performing a mutation that would refresh away another dirty cell draft. The UI
distinguishes an absent translation overlay from an overlay whose target is
explicitly empty.

## Sheets explorer

The Sheets tool presents the already-loaded `ProjectSheetDto[]`: slash-separated
names form collapsible folders for display only, while the canonical sheet name
is passed unchanged to Rust. Its per-sheet translatable-cell count comes from the
validated HSG permission index, not the HXS physical row count, and each sheet
with translations shows a coverage bar from `translation_progress`. Sheets with
no permitted source strings are hidden by default. Hovering or focusing the
Sheets dock reveals icon actions to show empty sheets, open the name filter,
reveal the active sheet, and collapse folders. The filter takes space only while
open, and Ctrl+F focuses it. Reveal active expands the selected sheet's folders
and clears only the filters that hide it: the name filter when the sheet name
does not match, and the empty-sheet filter only when the active sheet itself
has no translatable strings. Collapse all clears the name filter and closes
every folder. The flattened visible tree is virtualized for large source
catalogs. Project Search is a separate workbench tool and shows a truthful
unavailable state until a domain search backend exists.

## Git dock

The Git dock is a provisional functional panel over the commands in
[`desktop-application-boundary.md`](./desktop-application-boundary.md). The
Collaboration view opens with a summary of the branch, its sync state, and Sync
and Refresh actions (or remote setup when no remote exists). It offers
repository initialization, a per-string "Mine / Server" choice for same-unit
sync conflicts, and the contribution status and "Finish contribution" under the
pull-request policy. Pending translation-unit changes are grouped by sheet in
source order with an A/M/D marker; clicking one opens that string in the editor
(paging to it if needed) with its checkpoint diff. The checkpoint composer also
edits the translator name and optional email; only a name is required, matching
`aeria-git`, and the composer explains why Checkpoint is unavailable. The view
also shows the
history of the selected string with who translated and reviewed it and a
"Restore this text" action that saves the historical target through the
ordinary target mutation (resetting review to `draft`). The Advanced view adds
project history with per-commit unit changes that open the same way, branches (switch and create), the
review policy toggle, the Git runtime, contributor counts, and the raw
working-tree file list. A sync, branch switch, or finished contribution that
changed the workspace reloads the current sheet and progress.

## Keyboard

Shortcuts are listed in `src/shortcuts.ts` and shown under Settings → Keyboard
shortcuts. Handlers live with the features that own them.

## Appearance

Themes are renderer presets from `ui/theme/registry.ts`. Most are adaptations
of popular editor themes (`ui/theme/editorThemes.ts`: One Dark Pro, the
default, plus Dracula, Tokyo Night, GitHub, Visual Studio Code, Nord, Gruvbox,
Monokai Pro, Night Owl, Rosé Pine, Ayu, Solarized, Palenight, Kanagawa, and
Everforest) mapped onto Aeria's layered tokens; Catppuccin, Aeria's own themes,
and High Contrast Dark are also available.

Settings open as a dialog with Appearance, Editor, Workflow, Keyboard
shortcuts, and About sections and a search across all settings. Theme, accent,
Reduce transparency, interface zoom (webview zoom), editor text size, macro
highlighting, control-character display, strings list density, and focusing the
next target after Save & next are per-machine renderer preferences kept in local
storage; they are never project data. Components consume semantic tokens from `ui/theme/tokens.css`, which
derive surfaces, lines, and state colors from each theme's palette.

On supported Windows versions, the launcher, workbench, and tool windows use the
system Acrylic backdrop and follow the selected light or dark theme. The
backdrop is visible through the titlebar, activity rails, launcher sidebar, and
gaps between panels; the titlebar has no separate fill or dividing border.
Panels, the document, and dialogs use opaque theme surfaces with rounded
corners. Reduce transparency, High Contrast Dark, and other platforms use an
opaque theme-colored backdrop.

This editor intentionally has no structured macro controls, source update or
rebase UI, or export. The workbench retains truthful AI dock slots, bottom-panel
tabs, and status/layout infrastructure even when those backends are unavailable.
