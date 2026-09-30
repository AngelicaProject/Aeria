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

The renderer owns only ephemeral drafts, navigation, sheet loading, filtering of
loaded rows, and loading/error presentation. Rust remains authoritative for source
bindings, workspace state, validation, classification, and mutation semantics.

## Launcher

The interface never shows internal names of the game data such as SqPack,
Excel, or sheet layout hashes. It calls the project's source the *game* and
its version the *game version*; "source text" is used only for the original
of a single string.

The launcher is a single screen. The left column holds the product name, the
**Open project**, **Clone project**, **New project**, and **Update project**
actions, and the application version from
`app_info`. The right panel shows Recent projects by default; choosing an action
replaces it with that action's form, and **Back** returns to the list. Settings
open in a dialog from the titlebar.

Recent projects load from the local registry. Loading is presentation-only: the
renderer receives typed recent-project DTOs and cheap filesystem availability
states (`ready` or `repositoryMissing`). It does not read `projects-v2.json` or inspect
app-data paths directly. Each row shows the repository name and path, source
language, game version, and when it was last opened. Missing entries remain
visible with their availability state and offer **Remove from recent projects**;
ready entries open on click. A name filter appears once more than three
projects are listed.

Open project takes only a repository root. Aeria reads the source language
from the workspace manifest and opens the configured game installation in that
language. When the installed game is newer than the project's game version,
nothing is written and the launcher asks for confirmation with the planned
counts, as for Update project; a game older than the project is refused.
Opening a recent project follows the same rule.

New and cloned projects go to `Documents/Aeria` unless another location is
chosen. New project takes a project name, which becomes the repository folder
name, an optional location, and one of the supported source languages
(`en`, `ja`, `de`, or `fr`). The project folder and any missing parents are
created before the game is read, so an unusable path fails at once; when
creation then fails or is cancelled, the folder is removed again if it is
still empty. New project also takes the target language: a common language
from a list or any BCP 47 tag typed after *Other…*; `und` is not accepted.
The interface language is suggested when it is not the source language, and
creation is refused until a language is chosen. While the game is read, the form shows a
*Reading the game* state; the operation cannot be cancelled.

### Game installation

The game installation is an application setting, not a field of each form.
Settings → Game shows the installation in use and every installation
detected on this computer; the user can choose another folder (picking the
inner `game` folder selects its parent) or return to automatic detection.
A chosen folder is stored in `game-settings.json` in the app-data directory;
without one, Aeria uses the first detected installation. Detection checks the
Square Enix launcher's installation record, Steam libraries, XIVLauncher's
configured game path, and the default installation folders on Windows, and
XIVLauncher.Core and Steam on Linux. A folder counts as an installation only
when it has `game/sqpack` and a non-empty `game/ffxivgame.ver`; the game data
itself is validated when it is read (see [`source.md`](./source.md)). Launcher forms show the installation in use with a
shortcut to this setting, and jobs fail with `gameInstallationRequired` or
`gameInstallationInvalid` when none is usable. While a job runs, its progress
takes the place of that row so the form does not grow.

Registry load failures show a dismissible, non-blocking launcher warning while
manual Open project and New project remain available. After dismissal, the
Recent projects section remains in a stable unavailable state rather than
returning to its loading state. A successful project launch with a local
registry write warning enters the editor normally and shows the warning at the
application level. The launcher never automatically reopens the last project.

## Workbench layout

```text
titlebar: menus · project / sheet · layout toggles · window controls
left rail | Sheets or Search | sheet tabs             | Git             | right rail
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
right docks, the editor height, and the bottom panel are resizable; a drag
previews the size through the region's CSS variable and commits it to layout
state on release, so dragging does not re-render the workbench. Dock panels
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

Going to a row always keeps the list in sheet order from the top: the string is
selected and scrolled into view once the sheet has loaded far enough to contain
it. A
coordinate without a translatable string shows a warning and keeps the current
selection (or selects the first row of a newly opened sheet).

## Strings list

The list receives row pages but renders a flattened occurrence view: one
`TranslationCellDto` is one lane, identified by `sheetName`, `rowId`,
`subrowId`, and `columnIndex`. A logical row remains the grouping context: the
`rowId:subrowId` coordinate appears on its first lane and continuation lanes
show a connector. Each lane shows its review state, `col N` field identity, and
single-line source and target previews in which macro spans are tinted. Until
EXDSchema exists, fields are labelled by column. Blocked source cells remain
context only and are never used as permission heuristics.

The list is virtualized and always holds the whole sheet. Opening a sheet reads
it with bounded `page_translation_rows` calls of 256 source rows. The sheet
selection and loading state paint before any rows render. A sheet that loads
within 150 ms then appears in one piece; a slower sheet shows the rows read by
then and the rest once it is complete, so the list updates at most twice per
load. A progress line under the list header tracks the strings read without
re-rendering the list.
Pages that contain no visible rows are simply skipped, and there is no manual
**Load more**. Reloading the open sheet after a Git operation keeps the current
rows and selection on screen and swaps in the new rows once complete. Overlays
saved while a sheet streams are applied to pages read before the save. A text
filter, a review state filter (untranslated, draft, needs review, reviewed),
and a string kind toggle pair (text only or formatting only; pressing the
active one again shows both) narrow the list; while the sheet is still
loading they cover the rows loaded so far and grow as the rest arrives. Formatting-only strings (no letters outside macros, such as
`...` or a number format) show a small `fmt` tag in the list and a
**Formatting** chip in the editor's source header; they stay translatable. The toolbar shows sheet-wide
coverage from `translation_progress`, never a figure derived from loaded pages.
Strings with uncommitted Git changes carry a gutter marker (added or modified)
derived from `git_pending_changes`.

### Scene view

Quest (`quest/…`) and cutscene (`cut_scene/…`) sheets whose rows carry
dialogue keys can also be shown as a scene: a **Strings / Scene** switch
at the start of the list toolbar, remembered as the `dialogueView`
preference (scene by default). The structure comes from `sheet_dialogue`
(see [`source.md`](./source.md#dialogue)) and is presentation only. The
scene is read as soon as the sheet is selected, beside its strings, and
the switch shows for every quest and cutscene sheet until the read says it
has no scene. While a scene loads, the scene view never shows the strings
list in its place: it keeps the scene shown last, dimmed and inert, with
the strings it was shown with, or says it is loading when there is none.
The scene shows as soon as the sheet's strings start to arrive; lines whose
strings have not arrived yet are inert until they do. The
scene is one virtualized list of rows indented by depth, with a guide line
per level. It reads like a script: scene titles are large with a rule
above them, each speaker's name stands above their lines, a choice is a
caption in the choice color with its question in italics and its answers
as outlined pills, and the script's conditions and loops are captions in
the flow color. What a row is nested in draws a rail per level in that
container's color: an answer in the choice color, a branch or loop in the
flow color, and a cutscene or the unplayed lines in a neutral one. The
colors are the semantic tokens `--scene-choice*`, `--scene-flow*`, and
`--scene-rail`, derived from each theme's accent and warning. In a narrow
list, the translation moves under the source. It starts with the quest's name, in translation when there is
one, which opens its string in the `Quest` sheet, and the other versions of
the quest, quests with the same name (see
[`source.md`](./source.md#dialogue)), each of which opens its sheet in a
pinned tab. The journal entries and the objectives follow.

A quest with a script (see [`source.md`](./source.md#quest-scripts)) then
reads scene by scene, in the order the game plays its lines. A scene that
offers the quest is titled *Accepting the quest*, one that completes it
*Completing the quest*, and others *Scene N*. The script's other functions
with lines follow, titled by what they do where the name tells (*Balloons
over characters*, *While escorting or chasing*, *Menu labels*, *Said in
chat*, *When enemies appear*, *Battle stage*, and so on) with the function's
name beside the title. Scenes and functions of the quest's battle scripts
are titled *Quest battle · …* with the battle script's name. Within a
scene:

- consecutive lines of one speaker sit under one name;
- a choice shows its question and its answers, like the game's choice
  list; an answer that is a line of the sheet can be selected like one,
  and what each answer leads to is nested under it and folds. An answer
  nothing follows in the scene shows a dash instead of a fold toggle. A
  grayed-out answer has a dashed outline and says it cannot be picked, and
  one the player can pick only sometimes says when, such as *Can be picked
  only if …*;
- other branches read *If …* and *Otherwise*, with the keyword in the flow
  color, naming common conditions in words (the player's sex, race, class
  or job, a completed quest, the reward) and others as code, such as
  `IsInstanceContentUnlocked(…) > 0`; joined tests read *… or …* and
  *… and …*, a comparison with `true` or `false` reads as the fact or its
  negation (*quest «A» is not complete*), and a branch on an answer names
  the answer. A quest variable reads as the quest's name, and one quest
  test joined over several quests is said once, such as *one of these
  quests is complete: «A», «B», «C»*; one value compared with several reads
  *the player's class or job is one of: …*, also among other tests of the
  same group, except values the script computes, which may differ though
  they read the same. A group inside another reads in brackets. The
  wording lives in `sceneConditions.ts`;
- a loop reads *Can repeat from here*, and a jump back to it *Back to where
  it can repeat*;
- a cutscene shows its name in the quest's script, such as
  `CUT_SCENE_01`, with its file's path in the tooltip, and folds over the
  lines its file names, in row order, with links to other sheets it plays
  lines of. A link names the cutscene file it opens at, such as
  `cut_scene/065/VoiceMan_06505 | aktkmm10330`: one voiced cutscene sheet
  often holds the cutscenes of several quests, one block of rows each. Its
  tooltip says that the order within the cutscene is not read;
- markers show where the quest is accepted or complete, or the dialogue
  ends until the player talks again.

Answers, branches, and loops fold. An untraced scene says so and lists its
lines in code order. A line the script plays from another sheet shows its
key. After the scripts, each cutscene file that names lines no scene plays
is a section titled *Cutscene* with its path and those lines, under which
*Played in* links each quest scene that plays the file, such as *The
Coming Dawn · Scene 17*, or says that no quest script plays it. Lines still
left follow in two groups: battle talk, whose keys say `BATTLETALK`, and
lines named by no script or cutscene file. When the script cannot be read,
the scene says why.

A cutscene sheet reads cutscene file by cutscene file the same way, with
lines no cutscene names at the end. A link to a cutscene opens the other
sheet's scene at it, and marks it for a moment: a cutscene sheet at the
cutscene file's section, a quest at the cutscene its scene plays. Quests without a script and cutscene
sheets no cutscene file names are grouped in row order: speech
with consecutive lines of one speaker under one name, `SYSTEM` labels as
system text, runs of `Q<n>` and `A<n>` labels as a player choice with its
question and answers, and other rows under *Other text* with their row
keys. Row order follows the script but not its branches. The lines of a
cutscene, in a quest's scene or in a cutscene file's section, are grouped
the same way. A choice read from row order says *in row order*; its
tooltip says that the lines after it may depend on the answer, which the
cutscene's timeline decides and Aeria does not read.

A speaker's name is the speaker label in title case (`AMHGARANJY_GEVA` reads
*Amhgaranjy Geva*), and the label itself is in its tooltip. Each line shows
its review state, source, and translation, wrapped and with a line break at
each `<br>`. Selecting a line opens it in the editor, and the arrow keys,
Save & next, and Approve & next move through strings as in the list. The
list's filters stay available; lines they exclude are dimmed rather than
hidden, so the scene stays whole. A row the game does not allow translating
is shown without a translation and cannot be selected.

## Translation editor

The editor sits below the list and edits one occurrence at a time. Its bar shows
the occurrence's review state and coordinate, field tabs for multi-cell rows,
the review-state control, and Revert. The source and target panes each have
their own Text / Code switch (see below).

Source and target sit side by side, with the translator note beside them (or
below them in a narrow document). Source is read-only; target is a CodeMirror
editor. The document of both is always the exact macro text; how tags are
drawn is presentation only, Rust remains the authority for parsing and
validation, and macro text is never rewritten by the presentation.

- **Text** is for translating. Tags are drawn as compact chips a translator
  reads past: conditions, values (`Item · $n1`), and game icons as their
  images. Conditions read in words from what `macro_view` reports: `if
  class = monk` for `<if ($gn68 == 20)>` with the class named from the
  game's `ClassJob` sheet, `if level ≥ 94`, `if player is female`; other
  parameters keep their code. `<else>` is `otherwise`, `<case>` is
  `case 1`, `case 2`, and a closing tag is `end`. A value branch such as
  `{240}` reads as the value, marked as not translatable. A condition whose
  branches hold no text to translate, only values and text without letters,
  is a single chip listing its distinct values, such as `10 / 5`; its
  tooltip gives the branches in words (`class = monk → (level ≥ 72 → 10,
  otherwise 5), otherwise 5`), and Code mode edits them. Lines start only
  where the text has `<br>`, so what reads as one line is one line. When a
  branch begins with `<br>`, as in `<if …><br>Combo bonus: …</if>`, the
  whole line depends on the condition: its line starts before the condition
  chips, and a cursor before them stays at the end of the line above, where
  typed text belongs. Formatting pairs vanish
  into the text they format: text inside `<ui-color 504>…</ui-color>` is
  drawn in that color, italics and bold as such, and a color with its
  outline (`<ui-color 504><ui-edge-color 505>`) is one thin marker in the
  color at each edge. `<br>` is a small `↵` followed by a real line break,
  and the cursor after it sits on the new line. Chips and markers are
  atomic: the cursor steps over them, Backspace deletes a whole tag, and a
  tag with an error is outlined in red. Colors of `<ui-color>` come from the
  game through `macro_view`. An idiom (see
  [`strings.md`](./strings.md#idioms)), such as
  `<split " " 1><string $gs1></split>`, is one chip that reads as its
  meaning, `player's first name`, with its macros in the tooltip; picking it
  inserts them. The strings list and the scene show it the same way, and
  Code mode shows its macros. A speaker name (see
  [`strings.md`](./strings.md#speaker-names)) reads `speaker ???: the
  line`: its markers are chips, and the name stays text to translate. When
  the translation lacks the speaker name the source starts with, or starts
  with one the source lacks, the translation's footer says so; the text is
  still valid and can be saved.
- **Picking tags from the source.** Clicking a chip of the source inserts
  its tag into the translation at the cursor: a value, an icon, or a line
  break as its tag, and an opening condition chip as the whole condition
  block with the source branches, for the translator to translate. Clicking
  either marker of a formatting pair wraps the translation's selection in
  the whole pair, or inserts the empty pair with the cursor inside.
- **Inserting macros.** The target pane's **Insert a macro** button, and
  the target editor's context menu under its Cut, Copy, and Paste, offer the
  macros of `aeria_se::catalog::INSERTIONS` (see
  [`strings.md`](./strings.md#insertions)) through the `macro_insertions`
  command, grouped: the player character's full name, first name, last name,
  class or job, and race; a choice by the player character's gender, and one
  only for a race or a class or job, each chosen from the game's rows in a
  submenu; and italics, a capital first letter, and a non-breaking space. A
  value replaces the selection, formatting wraps it, and a choice writes
  the selection, or the word before the cursor, into both branches, such as
  `<if $gn4>застыла<else>застыла</if>`, for the translator to reword.
- **Code** shows the macro text with every tag written out and highlighted,
  for editing tag arguments; a speaker name's markers are highlighted as
  syntax, as they are in the strings list.

In every mode Enter inserts `<br>`, the game's line break.

The editors also show what Rust reads from the text, through the
`macro_view` command (debounced while typing, and ignored when it describes
an older text):

- **Hovers.** Hovering a tag shows what it does and its arguments in the
  interface language, for example `<sheet>` with its sheet, row (`$n1`,
  "number parameter 1 of the string"), and column. Summaries, argument names, and family names come
  from the macro catalog and are localized in the renderer.
- **Errors.** Diagnostics are underlined, and hovering one shows its message,
  such as `<colour> is not a macro; did you mean <color>?`.
- **Modes.** Each of the source and target panes switches between
  **Text** and **Code**; both open as text. The modes are local preferences
  (`sourcePaneMode`, `targetPaneMode`). Switching strings keeps the previous
  view until the new one arrives, requests it without delay (only typing is
  debounced), and shows strings seen before at once from a cache.
- **Game symbols.** Game text writes some symbols as private use characters
  that only the game font draws, such as `U+E03C`, the high-quality mark.
  When a project opens, the renderer loads a font of these glyphs made from
  the project's game (`game_glyph_font`) and names it first in every font
  stack, limited to the private use area, so the string list and the
  editors show the symbols instead of empty boxes, and every other
  character keeps the interface fonts.

Row context cells are available in a collapsible section under the source. **Copy source to target** replaces the target draft
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
game's translation permission (see [`source.md`](./source.md)), not the
sheet's physical row count, and each sheet
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
unavailable state until it is implemented.

## Git dock

The Git dock is the Git view over the commands in
[`desktop-application-boundary.md`](./desktop-application-boundary.md). It
follows the repository by itself (see [`git.md`](./git.md#git-in-the-desktop))
and has no refresh button.

- The summary shows the branch with its switcher, the sync state, and a
  button to Settings → Repository (or, without a remote, a link to connect
  one). With a remote, a toolbar offers Fetch, Pull (with the commits to
  pull), Push (with the commits to push), and Sync. Pull is disabled with
  uncommitted translations; Push is disabled on the main branch and while
  the upstream has commits the branch lacks.
- With a github.com `origin`, the dock offers the merge check workflow (see
  [`git.md`](./git.md#merge-check-ci)) in a card that lists its three
  stages, with "Add workflow" or, when the file differs, "Update workflow".
  The card can be dismissed for the session. After adding, the dock says to
  commit and push it and links to the branch settings on GitHub, where the
  check is made required and branches can be required to be up to date. Development builds and projects in a repository
  subfolder show why the workflow cannot be added.
- The branch switcher is a popover below the branch name, not a list over
  it: a filter field, local branches (the current one first and checked,
  then the main branch, then by name; each with its upstream, a main-branch
  or merged badge, and why it is blocked), remote branches without a local
  one (choosing one checks it out as a tracking branch), and "New branch…",
  which creates a branch from `HEAD` and checks it out. On the main
  branch it explains that the next checkpoint starts a contribution branch.
- It offers repository initialization, a per-string "Current (yours) /
  Incoming" choice for same-unit merge conflicts, and, on a contribution
  branch, a Pull request section with the branch's push state, its unmerged
  commits, and "Switch to `<main>` and delete this branch" once it is merged
  — or, in a repository without a remote, "Merge into `<main>`" with a
  confirmation.
- The dock uses Git's established terms in every interface language rather
  than inventing its own: the checkpoint button is labelled Commit, the
  identity form edits `user.name` and `user.email`, and states and results
  speak of commits, branches, upstream, push, pull, merge, and pull
  requests. Russian keeps the English command names where translators know
  them (Commit, Merge, push, pull, upstream) and the common loanwords
  («коммит», «смержить»).
- The branch switcher disables branches that hold the project in a state the
  open session cannot load (no project, an older Workspace Format, or another
  game source) and says why; when the main branch is such a branch, the dock
  explains that the work is not merged into it yet.
- **Changes** (collapsible; the dock remembers whether it is open while the
  window lives) starts with the composer, which commits everything below and
  edits the translator name and optional email. Below it are the
  translation-unit changes grouped by sheet with an A/M/D marker (clicking
  one opens the string with its checkpoint diff) and project file changes
  grouped by area, as described in
  [`git.md`](./git.md#project-file-changes).
- The translation changes are a virtualized list of at most 480 px. With 12
  or more changes it offers a search (sheet name or text, or an exact `row`
  or `row:subrow`), a filter by kind with counts, and collapse or expand all
  sheets; a search or kind filter opens every matching sheet. With more than
  50 changes in several sheets, sheets start collapsed. The sheet whose
  changes are at the top stays pinned above them. A row names its column
  only when the sheet's changes span several columns, and shows what
  changed (text, review, note) only for modified strings. Filters and open
  sheets are kept per list (the uncommitted changes, or each commit) while
  the window lives. Commit tabs use the same list.
- **History** fills the rest of the dock: commits with a lane graph, branch
  and tag labels, and time, loading older commits while scrolling. Clicking
  one opens a commit tab.

A commit tab shows the message, author, time, full id, labels, translation
changes (which open in the editor), and project file changes.

Dialogs that write project files (export, fonts, project knowledge)
refresh the dock when they close. A sync, branch switch, or finished
contribution that changed the workspace reloads the current sheet and
progress, as does a write of an agent through the `aeria` command (see
[`agents.md`](./agents.md#sharing-a-project-between-processes)).

## String history

The translation editor's side pane has three tabs: Note, Languages, and
History. The open tab is a local preference (`sidePaneTab`), so it stays
when another string is selected and after a restart. Languages shows the selected source text in the game's other client
languages, stacked in the source pane's chip or code view, so a translator
can compare how each language uses tags such as conditions; a tag clicked
there is added to the translation as from the source pane. A language
without the string says so. History shows who translated and reviewed the
selected string and every committed change to it; "Use this text" puts a historical text into the editor as an
unsaved draft.

## Project knowledge dialog

The project knowledge dialog opens from the Translation menu (**Terms**,
**Style**) and the command palette. It edits the two files of the
[project knowledge](./agents.md#project-knowledge):
`aeria-knowledge/terms.csv` and `style.md`.

The Terms tab is a table of term, translation, note, forbidden variants
(separated by `;`), and **Settled**, with a filter, **Add term**, and a remove
button per row; at most 300 filtered rows are shown at once. Editing or adding
a row marks it settled; the checkbox changes that. Rows with an empty term or
translation, or a term repeated case-insensitively, are marked and block
saving. Rows the file excludes are listed with their line numbers; saving
removes them only after confirmation. The Style tab is a Markdown text area
for `style.md` and shows its size against the 8 MiB limit. Each tab
has **Revert** and **Save**; closing with unsaved changes asks first. A save
fails, without writing, when the file changed since it was loaded, for
example because an agent wrote it.

## Export

**Export pack…** opens from the File menu and the command palette. The dialog
shows the source and target languages, the game version, and the commit the
pack will record, and warns when translations or `aeria-pack.json` have
uncommitted changes. Its sections edit the pack settings, manage the signing
key (create, import or save a backup, remove from this computer), take the
release parameters, and, when `origin` is on GitHub, show the feed URL and add
or replace the feed workflow. **Check GitHub** reads the latest release number
and raises the release number above it. **Export to folder…** and **Publish to
GitHub** are enabled only when their preconditions hold; publishing asks for
confirmation. The behavior behind the dialog is in [`export.md`](./export.md).

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

Settings open as a dialog with Appearance, Editor, Workflow, Game, Agents,
Project, Repository, Keyboard shortcuts, and About sections and a search across all settings. Theme, accent,
Reduce transparency, interface zoom (webview zoom), editor text size, macro
highlighting, control-character display, strings list density, and focusing the
next target after Save & next are per-machine renderer preferences kept in local
storage; they are never project data. Components consume semantic tokens from `ui/theme/tokens.css`, which
derive surfaces, lines, and state colors from each theme's palette.

The Agents section shows whether the `aeria` command is installed and on
`PATH` and the open project's `AGENTS.md` and `CLAUDE.md`, with **Connect agents** (or **Update**
when everything is in place) that sets them all up; see
[`agents.md`](./agents.md#discovery).

The Project section changes the open project's target language with the
same picker as New project. A listed language is saved when chosen; a typed
tag on Enter or when the field loses focus. Saving rewrites only the
workspace manifest, which collaborators receive through Git; units, targets,
and IDs are unchanged. Projects created before the language could be chosen
carry `und`, which the section shows as no language with a warning that
export needs one.

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
reattachment of detached translations, or export. The workbench retains truthful bottom-panel
tabs, and status/layout infrastructure even when those backends are unavailable.
