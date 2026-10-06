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
```

The renderer owns only ephemeral drafts, navigation, sheet loading, filtering of
loaded rows, and loading/error presentation. Rust remains authoritative for the
project's files, validation, classification, and save semantics.

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
from `aeria.json` and opens the configured game installation in that
language. When the installed game is newer than the project's game version,
nothing is written and the launcher asks to update the project to it, naming
both versions; a game older than the project is refused. Opening a recent
project follows the same rule.

New and cloned projects go to `Documents/Aeria` unless another location is
chosen. New project takes a project name, which becomes the repository folder
name, an optional location, and one of the supported source languages
(`en`, `ja`, `de`, or `fr`). The project folder and any missing parents are
created before the game is read, so an unusable path fails at once; when
creation then fails or is cancelled, the folder is removed again if it is
still empty. New project also takes the target language: a common language
from a list or any BCP 47 tag typed after *Other…*; `und` is not accepted.
The interface language is suggested when it is not the source language, and
creation is refused until a language is chosen. While the game is read and the
project's files are written (about a minute for the whole game), the form
shows a *Reading the game* state; the operation cannot be cancelled.

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
session. A floated tool asks the main window, which owns the editor, to
open a string, read changed files again, show the machine translation dialog,
or open a commit, through events sent to the `main` window
(`workbenchEvents.ts`); the main window comes to the front for a string or the
dialog. The right dock opens on Git; the bottom panel holds the
[string guide](#string-guide) and is shown by default.

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
| `#` | Project search in translations and sources: up to 30 strings, Enter opens one in the editor, and the last item shows all in the Search tool |
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
show a connector. Each lane shows the string's state (untranslated,
translated, reviewed by a person, or with a changed source; a reviewed
string's dot has a ring), `col N` field identity, and
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
filter, a state filter (untranslated, translated, reviewed, source changed; translated
means not reviewed),
and a string kind toggle pair (text only or formatting only; pressing the
active one again shows both) narrow the list; while the sheet is still
loading they cover the rows loaded so far and grow as the rest arrives. Formatting-only strings (no letters outside macros, such as
`...` or a number format) show a small `fmt` tag in the list and a
**Formatting** chip in the editor's source header; they stay translatable. The toolbar shows sheet-wide
coverage from `translation_progress`, never a figure derived from loaded pages.
Strings with uncommitted Git changes carry a gutter marker (added or modified)
derived from `git_pending_sheet_changes`, which returns the changes of the
open sheet only.

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
its state, source, and translation, wrapped and with a line break at
each `<br>`. Selecting a line opens it in the editor, and the arrow keys,
Save & next, and Accept & next move through strings as in the list. The
list's filters stay available; lines they exclude are dimmed rather than
hidden, so the scene stays whole. A row the game does not allow translating
is shown without a translation and cannot be selected.

## Translation editor

The editor sits below the list and edits one occurrence at a time. Its bar shows
the string's state and coordinate, field tabs for multi-cell rows, a *Source
changed* mark on a fuzzy string, and Revert. The source and target panes each have
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
with the source macro text. Under the source of a fuzzy string, the source
its translation was written for is shown as a word-level diff against the
current source.

Each cell has an independent target draft and note draft. Target and note text
do not autosave: both use explicit save actions. A save addresses one
`SourceBinding` and patches that cell from the returned
`TranslationOverlayDto`; a translation the checks refuse is not saved, and the
error lists its problems. Saving a fuzzy string's translation accepts it and
removes the mark. A row is dirty when any contained cell has a dirty target or
note.

A person marks a translation reviewed (see
[`po-project.md`](./po-project.md#reviews)) with the button beside Save:
**Save & mark reviewed** saves the draft and marks it at once, and without
edits the same button reads **Mark reviewed** and only marks the saved
translation; a reviewed string without edits does not show it. A reviewed
string shows **Reviewed** in the editor bar, which removes the mark. Nothing else marks a
string: a plain save keeps a mark the string has and adds none. A string whose mark is of another text (the
translation changed by a merge or by hand) shows *Changed after review*. The footer's buttons are one group: when it does not fit
beside the hint, the whole group moves to a second row, right-aligned, and
the hint keeps one line, cut with an ellipsis and whole in its tooltip. In
a translation pane narrower than 600 px the review button shows only its
icon and Save hides its shortcut.

When the selected string has uncommitted Git changes, the target pane shows a
word-level diff between the last checkpoint and the current draft, or notes that
the string is new since then. The diff is presentation only and can be hidden.

**Save & next** saves the target, waits until the saved overlay is applied and
no dirty draft remains, then selects the next occurrence in the filtered list
and, unless disabled in settings, focuses its target. When there is nothing to
save it only moves on.

**Accept & next** (*Still correct & next*; Ctrl+Shift+Enter, also in the
Translation menu and the command palette) is for a string whose source
changed: it saves the target as it is, which accepts the fuzzy string, and
moves on like Save & next. The button shows only on a fuzzy string, where it
does what no other button does; the shortcut works on every string. A translated string
without edits only moves on; an empty target does nothing. When the selected
string has left the filtered list, for example a fuzzy filter after accepting
it, the next and previous strings are found from its place in sheet order.

Unsaved target or note drafts are marked and protected by a discard
confirmation when changing rows, changing sheets, closing the project, or
performing a mutation that would refresh away another dirty cell draft. The UI
distinguishes an absent translation overlay from an overlay whose target is
explicitly empty.

## String guide

The string guide sits in the bottom panel under the document (Ctrl+J shows
or hides it) and can move to the right dock. It is for the person
translating the string in the editor: it shows what a machine translation
request tells the model about that string, which macros the string has and
what a translation may do with each, and what the checks find in the
translation as it is typed, before it is saved. The guide and the request
read a string the same way, so the guide follows every change to what a
request says about one string (see
[`translate.md`](./translate.md#the-request) and principle 11 of
[`principles.md`](../product/principles.md)). The editor publishes its
string and the translation being typed (`ui/editorFocus.ts`), so typing does
not re-render the workbench.

- A line above the columns says what the string is, when the request would:
  who says it (the translated name, or the speaker label in title case; the
  label in the tooltip), a quest's journal entry or objective, the length an
  interface label may have in characters or a name in the world in bytes
  ([`translate.md`](./translate.md#names-in-the-world)) with the
  translation's length against it, and that the line varies with the player
  character's gender in the source or in the French or German text, with
  **Insert a choice**, the gender choice of the insertion menu. A label or
  name longer than its length is advice, amber, as is a gendered line whose
  translation has no condition on `$gn4`.
- **Names and terms** lists the glossary terms of the source, each with its
  translation, forbidden variants, note, and whether a term exception keeps
  it out, and the game's names in the source with the project's
  translations, with a link to the glossary. Under each name is what it
  names, from every name sheet with it (*character, enemy*), the row's name
  when it is another form of it (*place, a form of «The Walk»*), and the
  string its translation comes from, which a click opens in the editor: a
  name is found by its letters alone, so an ordinary word such as *Walk* in
  *A Walk in the Park* can be found as one, and the origin shows it. Each is marked once there is a
  translation: used, not used yet (amber), or a forbidden variant used
  (red). A name counts as used as the glossary counts a term's translation:
  each of its words as written or inflected. A term's **Not this term**
  (on hover) adds a term exception to the string, and **Restore** on an
  excepted term removes it (see
  [`po-project.md`](./po-project.md#term-exceptions)).
- **Macros** lists the parts of the source the editor draws as chips or
  formatting markers, once each with a count, grouped by what a translation
  may do with them: keep (game data), conditions, letter case, formatting,
  and layout; each group's rule is in its tooltip. A part with game data the
  translation lacks is outlined in red, and one it has is checked.
- **Translation** lists the problems (red) and advice (amber) of the checks
  in the translation as typed, worded as in the search results, or says
  that they found nothing. An exception that names no term of the source
  has **Remove**. A string without names, terms, or macros says it is
  translated as ordinary text.

Clicking a name's or term's translation, or a macro part, adds it to the
translation at its cursor, as clicking a source chip does. The guide keeps
the previous string's content, dimmed, until the new string's arrives, and
reads the string again after saves, project knowledge edits, and Git
operations. The translation is checked at once for a new string and after a
250 ms pause while typing. In a dock narrower than 560 px the columns stack
and scroll together.

## Sheets explorer

The Sheets tool presents the already-loaded `ProjectSheetDto[]`: slash-separated
names form collapsible folders for display only, while the canonical sheet name
is passed unchanged to Rust. Its per-sheet translatable-cell count comes from the
game's translation permission (see [`source.md`](./source.md)), not the
sheet's physical row count, and each sheet
with translations shows a coverage bar from `translation_progress`. A sheet
of names (`translation_name_sheets`, the `NAME_SHEETS` of machine
translation) has a mark: its translations go with every request whose
strings use its names, and the model writes them exactly, so a mistake there
spreads through the project. Sheets with
no permitted source strings are hidden by default. Hovering or focusing the
Sheets dock reveals icon actions to show empty sheets, open the name filter,
reveal the active sheet, and collapse folders. The filter takes space only while
open, and Ctrl+F focuses it. Reveal active expands the selected sheet's folders
and clears only the filters that hide it: the name filter when the sheet name
does not match, and the empty-sheet filter only when the active sheet itself
has no translatable strings. Collapse all clears the name filter and closes
every folder. The flattened visible tree is virtualized for large source
catalogs. Project search is the separate Search tool (see
[Search tool](#search-tool)).

## Search tool

The Search tool (left dock by default, floatable) is project search over
`po/` (see [`search.md`](./search.md#project-search)). Its parts, top to
bottom: the query, the scope, the line of the result, the result, and, when
there are any, the notice of a bulk edit and the strings chosen.

- **Query.** One field with a clear button and toggles for match case,
  whole word, and regular expression, and a **Replace** toggle that shows
  the replace field under it, with *preserve case* and **Replace all…**.
  An invalid regular expression is reported under the field, and the last
  result stays, dimmed, until the expression is whole again.
- **Scope.** A line of chips under the query says what the search covers,
  each with its current value: the fields (translation and source by
  default; notes and string ID can be added), the sheets (all, the sheets
  and folders typed, such as `quest/, Addon`, or only the sheets of names),
  the string states, and the checks (none, problems, or advice; what each means is
  its tooltip). A chip that narrows the search is marked, and **Clear
  filters** puts every filter but the fields back. The query and scope are
  kept while the window lives.
- **Before a search** the tool offers untranslated strings, changed
  sources, problems, and advice as one-click starts; it shows no
  explanatory text.
- A search runs 120 ms after typing stops, or on Enter, and cancels the one
  in progress; the result shown stays until the next one arrives, and a
  line under the result's counts runs while a search takes long enough to
  notice. The result is live: when the project's files change (a save, a
  bulk edit, machine translation, a Git operation; `project://changed`),
  the same query runs again, and the open sheets, the sheets read so far,
  and the chosen strings that are still found stay.
- **The line of the result** gives the strings and sheets found (or that
  nothing was; when the filters narrow the search, a link clears them),
  **Collapse all** or **Expand all**, and an actions menu for the whole
  result: choose every string found, **Translate all found again…**, and,
  with a term's issue chosen, **Exception … for all**.
- **Issue.** With a checks filter, one more chip in the scope keeps only
  the strings with one group of issues (`pugilist → кулачный боец` for a
  term not used, *Macros*, *Extra line break*). A project has hundreds of
  groups, one per term, so the chip opens a list rather than laying them
  out: a filter field (Enter takes the first match), *Any issue*, then the
  other kinds and the terms in sections, each with its strings, most
  first. The list is of the result without an issue chosen, so choosing
  one does not narrow it.
- **The result** is one virtualized list grouped by sheet, the files of a
  sheet (`Item/10000.po`, `Item/12000.po`) together, with each sheet's
  count, and its files listed on hover when there are several. While an
  open sheet's strings scroll past the top, its line stays there; a sheet
  whose own line is at the top, or a closed one, does not stick. A sheet is open when strings of it were sent; a sheet with none
  sent is closed and is read when opened, and one with only some sent ends
  with **Show all N**. Each string shows its translation (or that it is
  untranslated) with the matches highlighted from near the first one, a
  chip when its source changed, its row and subrow (and column when it is
  not the first), its source always, the notes or ID when they matched,
  and the first finding of the checks filter, red for a problem and amber
  for advice (all of them on hover). Clicking a string opens it in the
  editor.
- **Keyboard.** Down in the query moves into the result. Up and Down move
  between sheets and strings, Enter opens a string or opens and closes a
  sheet, Space chooses a string, Shift with Up or Down chooses the strings
  moved over, Left goes to a string's sheet or closes it, Right opens it,
  and Escape (or Up from the first line) goes back to the query; Escape in
  the query clears it.
- Problems and advice are worded in the interface language from their data
  (`issueText.ts`), here and wherever a refused translation is reported, such
  as the editor's save error; reasons of the macro structure policy, written
  for the model, stay in English after a localized label.
- **Choosing** works as in a file list, without checkboxes: a click opens
  a string, Ctrl-click (Cmd on macOS) chooses or unchooses it, Shift-click
  chooses every string from the last one clicked, and Ctrl-click on a
  sheet chooses all its strings or none. Chosen strings are tinted with a
  bar at the left; a sheet with strings chosen shows how many of its count.
  The keys are learned on the way: a string's and a sheet's hover actions
  include **Choose**, whose tooltip names Ctrl-click, and the bar of chosen
  strings names Ctrl-click and Shift-click until the person first chooses
  with either (remembered in local storage).
  A sheet or a result with strings not sent is chosen whole: its strings
  are read again without the limit of a search (`project_search_entries`).
  The chosen strings get a bar at the bottom with their count,
  **Translate again…**, and **Exception**, which takes the chosen issue's
  term, or the term every chosen string has a finding for.
- **Actions on one string** show on hover: replace in it (with the replace
  field open), an exception for its term, and translate it again.
- Replacing: a string's replace button replaces in that string at once; a
  sheet's button and **Replace all…** open the replace preview dialog,
  which lists every change before and after with the changed words marked.
  Changes that would break their string show the reason and cannot be
  chosen; the others can be unchecked. Nothing is written before
  **Replace**.
- Translating again (every translation found, listed or not, the chosen
  strings, or one string) clears the translations after a confirmation and
  starts a machine translation run of exactly those strings; the AI
  translation dialog opens on its progress.
- An exception for a term (see
  [`po-project.md`](./po-project.md#term-exceptions)) and a replacement are
  bulk edits that Undo reverts. After a bulk edit a notice gives what was
  written, what was skipped and why (each skipped string opens in the
  editor), and **Undo** for the last bulk edit. The editor reads changed
  sheets again.

## Git dock

The Git dock is the Git view over the commands in
[`desktop-application-boundary.md`](./desktop-application-boundary.md). It
follows the repository by itself (see [`git.md`](./git.md#git-in-the-desktop))
and has no refresh button.

- The summary shows the branch with its switcher, its state against the
  upstream, and a
  button to Settings → Repository (or, without a remote, a link to connect
  one). With a remote, a toolbar offers Fetch, Pull (with the commits to
  pull), and Push (with the commits to push). Pull is disabled with
  uncommitted translations; Push is disabled while the upstream has commits
  the branch lacks.
- With a github.com `origin`, the dock offers the merge check workflow (see
  [`git.md`](./git.md#merge-check-ci)) in a card that lists its three
  stages, with "Add workflow" or, when the file differs, "Update workflow".
  The card can be dismissed for the session. After adding, the dock says to
  commit and push it and links to the branch settings on GitHub, where the
  check is made required and branches can be required to be up to date. Development builds and projects in a repository
  subfolder show why the workflow cannot be added.
- The branch switcher is a popover below the branch name, not a list over
  it: a filter field, local branches (the current one first and checked,
  then by name; each with its upstream and why it is blocked), remote
  branches without a local one (choosing one checks it out as a tracking
  branch), and "New branch…", which creates a branch from `HEAD` and checks
  it out. Every branch but the current one has a delete button, which opens
  the deletion confirmation described in [`git.md`](./git.md#branches).
- It offers repository initialization and a per-string "Current (yours) /
  Incoming" choice for strings both sides changed.
- The dock uses Git's established terms in every interface language rather
  than inventing its own: the checkpoint button is labelled Commit, the
  identity form edits `user.name` and `user.email`, and states and results
  speak of commits, branches, upstream, push, pull, merge, and pull
  requests. Russian keeps the English command names where translators know
  them (Commit, Merge, push, pull, upstream) and the common loanwords
  («коммит», «смержить»).
- The branch switcher disables branches that hold the project in a state the
  open session cannot load (no project, an earlier project format, or another
  game version) and says why.
- **Changes** (collapsible; the dock remembers whether it is open while the
  window lives) starts with the composer, which commits everything below and
  edits the translator name and optional email. Below it are how many strings
  changed and the first 500 of those changes grouped by sheet (a project
  without a first commit has one for every translated string; the rest are
  counted, and a checkpoint commits them all) with a marker (A translated, M changed, D removed,
  • marked; clicking one opens the string with its checkpoint diff) and
  project file changes
  grouped by area, as described in
  [`git.md`](./git.md#project-file-changes).
- The translation changes are a virtualized list of at most 480 px. With 12
  or more changes it offers a search (sheet name or text, or an exact `row`
  or `row:subrow`), a filter by kind with counts, and collapse or expand all
  sheets; a search or kind filter opens every matching sheet. With more than
  50 changes in several sheets, sheets start collapsed. The sheet whose
  changes are at the top stays pinned above them. A row names its column
  only when the sheet's changes span several columns, and says when a
  string's fuzzy mark, note, or review changed. Filters and open
  sheets are kept per list (the uncommitted changes, or each commit) while
  the window lives. Commit tabs use the same list.
- **History** fills the rest of the dock: commits with a lane graph, branch
  and tag labels, and time, loading older commits while scrolling. Clicking
  one opens a commit tab.

A commit tab shows the message, author, time, full id, labels, translation
changes (which open in the editor), and project file changes.

Dialogs that write project files (export, fonts, project knowledge)
refresh the dock when they close. A pull or branch switch that changed the
working tree reloads the current sheet and
progress, and so does a change of a shown sheet's file by Git, by hand, or by
machine translation (`project://files-changed`).

## String history

The translation editor's side pane has three tabs: Note, Languages, and
History; what the checks find is in the [string guide](#string-guide). The open tab is a local preference (`sidePaneTab`), so it stays
when another string is selected and after a restart. Languages shows the selected source text in the game's other client
languages, stacked in the source pane's chip or code view, so a translator
can compare how each language uses tags such as conditions; a tag clicked
there is added to the translation as from the source pane. A language
without the string says so. History shows every committed change to the
selected string with its author, and its uncommitted change; "Use this text"
puts a historical text into the editor as an unsaved draft.

## Project knowledge dialog

The project knowledge dialog opens from the Translation menu (**Terms**,
**Style**) and the command palette. It edits the two files of the
[project knowledge](./knowledge.md#project-knowledge):
`aeria-knowledge/terms.csv` and `style.md`.

The Terms tab is a table of term, translation, note, forbidden variants
(separated by `;`), and **Settled**, with a filter, **Add term**, and a remove
button per row; at most 300 filtered rows are shown at once. Editing or adding
a row marks it settled; the checkbox changes that. Rows with an empty term or
translation, or a term repeated case-insensitively, are marked and block
saving. Rows the file excludes are listed with their line numbers; saving
removes them only after confirmation. The Candidates tab finds the names
the project translates in several ways (`project_term_candidates`, see
[`search.md`](./search.md#glossary-candidates)): each with its translated
strings, its sheets, its renderings with their counts as choices, and
examples of each. **Add to the glossary** adds a term row with the chosen
rendering as its translation, the others as forbidden variants, and match
case on, and shows it in the Terms tab to review and save; **Skip** hides
the candidate on this computer until the skipped ones are shown again. The
Style tab is a Markdown text area
for `style.md` and shows its size against the 8 MiB limit. Each tab
has **Revert** and **Save**; closing with unsaved changes asks first. A save
fails, without writing, when the file changed since it was loaded, for
example by Git.

## Machine translation

**AI translation…** (the interface's name for machine translation) opens from the Translation menu and the command
palette. The dialog chooses what to translate in a tree of the project's
sheets with checkboxes (a folder's box chooses all its sheets), a search, and
quick choices (names, quests, all untranslated, clear); every sheet and folder
shows how many strings it still needs, and a sheet of names its mark. It
says that every request also carries the glossary, the style, and
translated strings of the same file. It opens with the open sheet chosen.
It also chooses whether strings with a changed source are included, shows how
many sheets and strings are chosen, and names the model; without a model it
links to Settings. **Translate** starts a run, and the
dialog shows its progress every second: strings written of the run's strings,
strings refused by the checks, tokens and the share served from the cache,
the pace, and why the run stopped. Each refused string lists its problems in
the interface language and the model's translation, which was not written;
it opens in the editor, a term problem has **Exception**, and **Translate
the refused again** starts a run of the listed strings. The dialog can be hidden
while the run goes; **Stop** stops it, and a stopped run offers
**Continue**. See [`translate.md`](./translate.md).

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

Settings open as a dialog with Appearance, Editor, Workflow, Game, AI
translation, Project, Repository, Keyboard shortcuts, and About sections and a search across all settings. Theme, accent,
Reduce transparency, interface zoom (webview zoom), editor text size, macro
highlighting, control-character display, strings list density, and focusing the
next target after Save & next, and the machine translation model and reasoning
effort are per-machine renderer preferences kept in local storage; they are never project data. Components consume semantic tokens from `ui/theme/tokens.css`, which
derive surfaces, lines, and state colors from each theme's palette.

The Machine translation section says that signing in to a ChatGPT
subscription is unofficial and counts against the plan's Codex limits, signs
in (it shows the code, opens OpenAI's page, and waits) or out, and chooses
the model and its reasoning effort from the account's models.

The Project section changes the open project's target language with the
same picker as New project. A listed language is saved when chosen; a typed
tag on Enter or when the field loses focus. Saving rewrites only
`aeria.json`, which collaborators receive through Git; translations are
unchanged.

On supported Windows versions, the launcher, workbench, and tool windows use the
system Acrylic backdrop and follow the selected light or dark theme. The
backdrop is visible through the titlebar, activity rails, launcher sidebar, and
gaps between panels; the titlebar has no separate fill or dividing border.
Panels, the document, and dialogs use opaque theme surfaces with rounded
corners. Reduce transparency, High Contrast Dark, and other platforms use an
opaque theme-colored backdrop.

The launcher offers **Update project** for an existing repository and an
installed game. After an update the workbench shows a summary: translations
to check because their source changed, translations kept as obsolete, files
written, and the commit. The status bar shows how many strings of the project
have a changed source and filters the list to them. See
[`po-project.md`](./po-project.md#game-updates).

