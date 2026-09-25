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
left rail | Sheets or Search | sheet tabs             | Git or Angelica | right rail
          |                  | strings list           |                 |
          |                  | (resizable split)      |                 |
          |                  | translation editor     |                 |
          |                  | optional bottom panel  |                 |
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

The command center (Ctrl+P) sits in the titlebar, centered on the window, and
opens the palette directly below it on the same center line, so neither moves
when menus, actions, or the active sheet change. Focus is shown by a 1px
accent border or outline; fields do not add a glow. The input prefix selects
its mode:

| Prefix | Mode |
| --- | --- |
| none | Go to a sheet by fuzzy name match; empty input lists recently opened sheets first |
| `>` | Run a workbench command, including theme switching (Ctrl+Shift+P) |
| `:` | Go to `row`, `row:subrow`, or `row:subrow:column` in the active sheet (Ctrl+G) |
| `#` | Project string search; shown as unavailable until the Search tool is implemented |
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
while its row cursor still has more source work. A text filter, a review
state filter (untranslated, draft, needs review, reviewed), and a string kind
toggle pair (text only or formatting only; pressing the active one again shows
both) narrow the loaded occurrences only and are
labelled as such. Formatting-only strings (no letters outside macros, such as
`...` or a number format) show a small `fmt` tag in the list and a
**Formatting** chip in the editor's source header; they stay translatable. The toolbar shows sheet-wide
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

**Approve & next** (Ctrl+Shift+Enter, also in the Translation menu and the
command palette) is for quick review: it saves an edited target first, marks
the string reviewed, and moves on like Save & next. A reviewed string without
edits only moves on; an empty target does nothing. When the selected string
has left the filtered list, for example a draft filter after approving it,
the next and previous strings are found from its place in sheet order.

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
unavailable state until it is implemented; the `aeria-search` backend
currently serves Angelica's search tools only.

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

## Angelica panel

The right dock's Angelica tool (named Angelica in every interface language)
is the chat with the agent described in [`ai.md`](./ai.md#angelica). Without
a configured model it shows how to open Settings → AI.

The header switches between the project's conversations, starts a new one,
and deletes the current one. The transcript shows user messages, Angelica's
replies with a small Markdown subset (paragraphs, lists, code, inline code,
bold) rendered as text, so game macros stay visible, collapsible reasoning,
and a collapsible card for each tool call with its arguments, result, and
running, done, or failed state. It follows new output while scrolled to the
bottom.

The composer has a mode picker (Chat, Ask, Auto-draft; Ask by default) and
a chip for the selected occurrence, which the user can turn off so the
selection is not sent. Enter
sends and Shift+Enter adds a line; messages written while Angelica answers are
queued and sent after the turn. The toolbar picks the model and, when the model
accepts efforts, the effort for the next message; the conversation's own
choice, then the default model, is preselected. It also shows the share of the
context window the last request used, when the window is known, the
conversation's tokens, **Stop** while a turn runs, and **Send**.

Above the composer, a collapsible list shows the conversation's pending
proposals and those that could not be applied. Each card shows the string's
location, which opens it in the editor, a word diff from the current
translation to the proposal, a mark when it would replace a reviewed
translation, and **Apply** and **Reject**; several pending proposals can be
applied or rejected together. A proposal to change `aeria-guidance.md` or
`aeria-glossary.csv` shows the file name and a diff of its text. A written translation, from Auto-draft or an
applied proposal, patches its cell like an ordinary save, so other unsaved
drafts and the selection are kept.

A review proposal shows Angelica's reason and how many translations she
suggests approving, with the strings (location, source, and translation) on
demand, **Approve N**, and **Reject**; it is never applied with the others.

A web-access proposal shows the domain and the link Angelica asked for, with
**Allow domain** and **Reject**; it is never applied with the others.

A job proposal shows its sheets, filter and string count, the estimate and
token limit, and the instructions, with **Start job** and **Reject**; job
proposals are never applied with the others. Above the proposals, a
collapsible list shows the project's running and paused jobs and the three
latest finished ones: the scope, status, a progress bar, drafted and problem
counts, tokens, the pause reason, **Pause**, **Resume**, **Cancel**, and
**Retry problems**, and on demand the problem strings, which open in the
editor, and the latest events. Messages Aeria sends Angelica, such as job
reports, appear as notices, and a turn Aeria starts is shown live when its
conversation is open.

The editor's source header has **Draft with Angelica**, which fills the target
with a validated draft for the selected string without saving it.

## Glossary and guidance

**Glossary and guidance** opens from the Translation menu, the command
palette, and the Angelica panel header. It edits the project-shared
`aeria-glossary.csv` and `aeria-guidance.md` described in
[`ai.md`](./ai.md#guidance-and-glossary).

The Glossary tab is a table of term, translation, note, and forbidden
variants (separated by `;`) with a filter, **Add term**, and a remove button
per row; at most 300 filtered rows are shown at once. Rows with an empty term
or translation, or a term repeated case-insensitively, are marked and block
saving. Rows the file excludes are listed with their line numbers; saving
removes them only after confirmation. The Guidance tab is a Markdown text
area with its size against the 64 KiB limit. Each tab has **Revert** and
**Save**; closing with unsaved changes asks first. A save fails, without
writing, when the file changed since it was loaded.

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

Settings open as a dialog with Appearance, Editor, Workflow, AI, Keyboard
shortcuts, and About sections and a search across all settings. Theme, accent,
Reduce transparency, interface zoom (webview zoom), editor text size, macro
highlighting, control-character display, strings list density, and focusing the
next target after Save & next are per-machine renderer preferences kept in local
storage; they are never project data. Components consume semantic tokens from `ui/theme/tokens.css`, which
derive surfaces, lines, and state colors from each theme's palette.

The AI section manages the providers described in
[`ai.md`](./ai.md#provider-boundary). It picks Angelica's default model and,
when the model accepts efforts, its effort, and the same for translation jobs,
which use Angelica's model when none is chosen. **Websites Angelica may read**
lists allowed domains, one per line, saved with its own button. Each provider card edits the name
and base URL (saved on blur), stores or removes the API key through a password
field that is cleared after saving and never refilled, and lists models with
toggles for accepted efforts, an optional context window, and a **Test**
action that reports latency and the answering model or the provider's error.
Saving the first key loads the provider's models; **Update from provider**
reloads them and reports how many were added and removed, and a model can
also be added by ID. A collapsible **Request headers** section edits the
session header and extra headers and warns that header values are not secret
storage. New providers come from preset buttons; the OpenAI-compatible preset
first asks for a base URL. Removing a provider asks for confirmation inline.

A ChatGPT (subscription) card has no base URL, key, or header fields. It
shows the unofficial-use warning, **Sign in with ChatGPT**, and, while a
sign-in waits, the code to enter with a copy action and **Cancel**. A
successful sign-in loads the plan's models; **Sign out** removes the stored
token.

On supported Windows versions, the launcher, workbench, and tool windows use the
system Acrylic backdrop and follow the selected light or dark theme. The
backdrop is visible through the titlebar, activity rails, launcher sidebar, and
gaps between panels; the titlebar has no separate fill or dividing border.
Panels, the document, and dialogs use opaque theme surfaces with rounded
corners. Reduce transparency, High Contrast Dark, and other platforms use an
opaque theme-colored backdrop.

The launcher offers **Update project** for an existing repository and an
installed game, and asks for confirmation with the planned counts before a
package with other content is applied. After an update the workbench shows a
summary; the status bar shows the number of detached translations and opens
their list. See [`rebase.md`](./rebase.md#desktop-workflow).

This editor intentionally has no structured macro controls, manual
reattachment of detached translations, or export. The workbench retains truthful AI dock slots, bottom-panel
tabs, and status/layout infrastructure even when those backends are unavailable.
