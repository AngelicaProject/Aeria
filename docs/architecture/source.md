# Game source

The source of an Aeria project is the installed game. Aeria reads the Excel
sheets of the game directly through `aeria-sqpack` and derives every source
fact it needs from them: strings, sheet layouts, translation permission, and
row keys. Nothing about the source is extracted into an intermediate
artifact, and nothing about it is cached across processes.

`aeria-source` owns this model. It depends on `aeria-sqpack` for the file
formats and on `aeria-se` to print string bytes as macro text.

## Opening a source

`GameSource::open(game_path, language)` takes the game folder, the one that
contains `game/sqpack`, and a source language (`ja`, `en`, `de`, or `fr`).
It reads:

- the game version, the trimmed text of `game/ffxivgame.ver`. All Excel
  files are in the base `ffxiv` repository, so expansion version files do
  not describe the source;
- the sheet list, `exd/root.exl`, restricted to sheets that have a header.

A game version is a dot-separated list of decimal numbers, such as
`2026.09.15.0000.0000`. Versions are compared component by component; a
version that does not have this form is rejected.

`GameSource::current_version` reads the version file again. Callers that
keep a source open while the game can be patched, such as export, compare it
with the version the source was opened with and stop when it changed.

## Sheets

`GameSource::sheet(name)` reads one sheet in the source language and returns
one of:

- `Missing`: the sheet is not in the sheet list;
- `Unavailable(reason)`: the sheet is listed but cannot be read, because its
  variant or a column type is unknown, its pages are malformed, or it has
  neither data in the source language nor language-neutral data;
- a `SourceSheet`.

A sheet in a language it does not provide is read from its language-neutral
data, as the game does.

A `SourceSheet` holds its rows in `(row, subrow)` order and, for every row,
the bytes of each String column. A subrow is numbered by its position in its
row. Recently read sheets are kept in memory and read again when evicted.

### Layout

A sheet's *layout* is the ordered list of its String columns, each as its
column index and its byte offset in the row. It is exactly what Harmonia
compares before it touches a sheet (see
[`../formats/pack-v1.md`](../formats/pack-v1.md#reader-requirements-harmonia)).

The layout hash identifies a layout in persisted data. It is the first 8
bytes of

```text
SHA-256("aeria.sheet-layout.v1" || for each String column: u32le(index) || u16le(offset))
```

written as 16 lowercase hexadecimal characters. Non-String columns do not
take part: a patch that adds or moves a number column leaves the layout, and
every translation of the sheet, unchanged.

### String text

`SourceSheet::text(row, subrow, column)` prints the String cell with
`aeria_se::codec::decode`. The printed macro text is what translators see
and what translation units store (see
[`../formats/workspace-v3.md`](../formats/workspace-v3.md)). Two cells whose
texts are equal are the same source for Aeria even when their bytes differ.
Export and Harmonia compare bytes instead (see
[`export.md`](./export.md)).

## Game glyphs

Besides sheets, `aeria-source` reads what the interface needs to show game
text as the game does. `GameSource::private_glyphs` cuts the private use
glyphs (such as `U+E03C`, the high-quality mark) out of the largest game
font, `common/font/axis_36.fdt` and its `font*.tex` pages, as coverage with
the font's metrics. `GameSource::icon` cuts an inline icon of `<icon>`
macros out of `common/font/fonticon_xinput.tex` at double size, by the
positions in `common/font/gfdata.gfd`, following its redirects. Textures are
`SqPack` texture files, which `aeria-sqpack` reads like standard files.

## Translation permission

A String cell may be translated only when its text is language-dependent.
Keys, file names, and other identifiers are the same in every language, and
changing them breaks the game.

For one sheet, the *evidence languages* are the global client languages
Japanese, English, German, and French. The sheet is *comparable* when it
can be read in every evidence language and, in all of them, has the same
variant, the same columns (type and offset), and the same list of
`(row, subrow)` coordinates. A sheet with only language-neutral data reads
the same in every language, so none of its cells is translatable. In a
comparable sheet, a cell is translatable when its source bytes are not empty
and differ from the bytes of the same cell in at least one other evidence
language. Every cell of a sheet that is not comparable is not translatable.

Permission is computed for a whole sheet the first time it is needed and
kept with the sheet. It depends only on the game data, so every collaborator
on the same game version computes the same permission.

A translatable cell whose text contains no letters outside protected
structure, such as `...`, a `0` placeholder, or only number formatting, is
still translatable. The read path marks it `formatting_only` so the editor
can label and filter it; see `MacroString::is_formatting_only` in
`aeria-se`.

## Row keys

Some sheets number their rows as a sequence and carry a stable text key in a
String column, for example `TEXT_..._000_000` in quest dialogue. Inserting a
line renumbers every later row, so the row ID is not a durable coordinate
there, but the key is.

A String column is the sheet's *row key column* when:

1. the sheet has at least two rows;
2. every row has a non-empty text in the column;
3. the texts are unique across rows; and
4. none of the column's cells is translatable, which means the text is the
   same in every evidence language.

When several columns qualify, the lowest column index is used. The key of a
row is the text of its row key cell. Condition 4 excludes translatable text:
a translated name that is merely unique must never act as an identity key.

## Dialogue

Quest sheets (`quest/…/<ID>`) and cutscene sheets (`cut_scene/…/<ID>`) with
row keys describe their rows in the keys. This structure is context for
translators (the editor's scene view) and agents; it never affects
identity, permission, or any persisted data.

`GameSource::dialogue` returns the rows of such a sheet that have text, in row
order, each with its key, the first non-empty String cell other than the key,
and a role read from the key. After the `TEXT_<ID>_` prefix, where `<ID>` is
the last segment of the sheet name and case is ignored:

| Rest of the key | Role | Example |
| --- | --- | --- |
| `SEQ_<n>` | Quest journal entry | `TEXT_MANFST004_00124_SEQ_00` |
| `TODO_<n>` | Objective | `TEXT_MANFST004_00124_TODO_02` |
| `<label>_<n>_<n>` | Speech by `<label>` | `TEXT_MANFST004_00124_MIOUNNE_000_1` |
| `<n>_<label>` | Speech by `<label>` | `TEXT_VOICEMAN_02400_000010_ILBERD` |
| `<label>_<n>_<n>_NONE_VOICE` | Speech by `<label>`, unvoiced | `TEXT_VOICEMAN_05004_Q4_000_001_NONE_VOICE` |
| anything else | Other | `TEXT_JOBRDM501_02577_QIB_001_XRHUNTIA_BATTLETALK_16` |

`<n>` is a run of digits. A speaker label is one or more `_`-separated
segments of ASCII letters and digits, none of them only digits, such as
`URIANGER`, `AMHGARANJY_GEVA`, or `SYSTEM_NONE_VOICE`. Labels are the game's
internal names: `SYSTEM` is system text, labels such as `Q1` and `A1` are
usually a player choice's question and answers, and one character can have
several labels. A key without the prefix, or one that matches no rule, is
*other*; Aeria never guesses a speaker.

Row order follows the script, but not its branches: the order in which the
game plays a quest's lines is decided by its script (see
[Quest scripts](#quest-scripts)). Who a line is addressed to is not
recorded.

`GameSource::quest_row` links a quest sheet to its `Quest` row: the only row
with a String cell that is not translatable and holds the sheet's ID, such
as `ManFst004_00124`. The row's translatable cells hold the quest's name.
When no row or more than one row holds the ID, the sheet has no quest.
`Quest` is read once per source.

`GameSource::quest_versions` lists the other quest sheets whose `Quest` row
has the same name: the source text of the row's first non-empty translatable
cell, compared exactly. The game has several quests of one name, such as
`ClsArc000_00021` and `ClsArc998_00131`, both *Way of the Archer*, and gives
a player one of them. Which one is decided before any of their scenes, so
no quest script records the choice; Aeria lists the versions and does not
say which player gets which. On the current game, 150 quests have other
versions, at most seven each.

`GameSource::speakers` and `GameSource::speaker_lines` find the speech of one
label across every quest and cutscene sheet, in sheet-name and row order. The
first call reads every dialogue sheet on up to four threads, a few seconds on
the current game, and keeps an index of the lines in memory.

## Quest scripts

Every quest sheet `quest/…/<ID>` has a compiled Lua 5.1 script at
`game_script/quest/…/<ID>.luab`. Its scene functions, assigned as
`<table>.OnScene<number>`, play the quest's lines and ask the player's
choices; its other functions are handlers the game calls on events, such as
`GetBalloonTalkArgs` for balloons over characters or `PLANDEF_OnChasingTalk…`
during escorts. `GameSource::quest_script` reads the script and traces every
function with lines into a tree (`QuestScript`, in
`aeria-source/src/script/`): scenes in scene order, then the other
functions in code order with the name they are assigned to. When a name is
assigned several times, only the last function runs and only it is traced.

A quest with battles has a battle script per battle beside it, named after
the quest: `ClsRog250_00148` has `ClsRog250Btl_00148` and may have
`ClsRog250Btl2_00148` up to `Btl9`. Its scenes and handlers, such as the
battle's stages `OnSequence<number>`, are traced like the quest's and are
marked with the battle script's name. Like
the dialogue structure, the tree is context for translators; it never
affects identity, permission, or persisted data. The script is read on
demand, in milliseconds.

The reader accepts only the chunk format the game uses: Lua 5.1, little
endian, 4-byte `int`, `size_t`, and instructions, and 8-byte floating-point
numbers. Any other or malformed chunk is `SourceError::Script`; nothing is
reinterpreted. A quest sheet without a script has none, and other sheets
never have one.

A scene is traced as follows:

- The instructions form a graph whose edges pass through chains of
  unconditional jumps. Each conditional test is a branch whose two sides
  are traced to the test's immediate post-dominator, where they meet. A
  backward edge marks a loop, traced to the first post-dominator outside
  its natural body; a jump back to a loop being traced is a *repeat*.
- Registers are followed along each path to name called functions, the
  text keys passed to them, and the values tests compare. Where paths meet
  with different calls in one register, as in `A() or B()`, the value is
  one of them. Where the two sides of a branch leave `true` and `false`
  (or booleans computed the same way) in a register, as in
  `local ready = a and b`, the register holds that condition, and a test of
  it is the condition itself. The registers after a function's parameters
  start as `nil`, as the Lua VM clears them and the compiler writes no
  `LOADNIL` for them. A register that is `nil` rather than `false` on some
  path holds the condition for a truth test or `== true`, but not for
  `== false`, which `nil` does not satisfy. Tests both sides of such a
  branch share are taken out, as the compiler repeats the rest of an `or`
  or `and` chain after each test in it: `(h and a and x) or (not h and a
  and y)` is `a and (h and x or not h and y)`, and `(h and (a or x)) or (not
  h and (a or y))` is `a or (h and x) or (not h and y)`. Other values the
  script computes are unknown.
- A choice's id is the address of its call, so the same menu reached on
  several paths is one choice, and the paths compare equal when joined.
- A text returned by a function or stored in a table (`SETTABLE`,
  `SETLIST`) is a line, as balloons and menus hand their text to the game
  that way. An argument chosen earlier from several texts lists each of
  them, and a question chosen that way gives a choice several prompts.
- A call with a variable number of arguments (`B = 0`) passes the registers
  up to the open results of the call or `...` before it, as in
  `Menu(q, unpack(answers))`; such a menu's answers cannot be counted and
  it has none. An answer that is not a text key, such as an entry of a
  list, is an answer the script passes.
- A loop's body is the natural loop limited to the range from its header
  to its last jump back, since `FORPREP` enters a `for` loop at its end.
- A `for` loop writes its counter and variable each round, and `for … in`
  its control value and variables, so in the body those registers hold
  unknown values, not what they held before the loop: in
  `Talk(key); for i = 1, 1 do SetNpcTradeItem(i) end`, `i` reuses the
  register of `key` and is not a line.
- A field key is a constant, or a register holding a string constant: a
  function with more than 256 constants loads the key first
  (`LOADK r5, "TEXT_…"; GETTABLE r5, r0, r5`).
- A call with a `TEXT_…` field argument is a line (`Talk`, `SystemTalk`,
  and others). `QuestOffer` is a choice to accept or decline; `YesNo` and
  `YesNoQuestBattle` ask their first text argument and answer with the next
  two; `Menu` asks its first and answers with the rest, by number.
  `GrayoutMenu` is a menu whose every answer is followed by a flag:
  `MENU_FLAG_DISABLE` shows it grayed out, so the player cannot pick it,
  `MENU_FLAG_ENABLE` does not, and another value may do either. A menu
  whose answers are built at run time has no options.
  `PlayCutScene`, `CancelEventScene`, `QuestAccepted`, and `QuestCompleted`
  are markers.
- Branches and loops that contain no line, choice, or marker are dropped:
  most conditions, such as the player's race or sex, only choose
  animations.
- A branch that asks the same question with the same answers on both
  sides, only grayed out differently, as a script that asks `Menu` when
  the player can afford an answer and `GrayoutMenu` otherwise, is one
  choice. An answer available on one side only is available when that
  side's condition holds, and tests of either choice's answer are tests of
  the one choice.
- Branches on a choice's answer that follow the choice move into its
  options. Scripts often test the answer in consecutive `if`s that each end
  by asking again; a later branch applies only to the options that did not.
- The compiler turns `if a or b then X else Y` into a test of `a` that
  leads to `X` or to a test of `b`, and a test of `b` that leads to the
  same `X` or to `Y`; `and` is the same with the sides swapped. Where a
  test leads, after code that only computes the next test's values, to a
  test that shares one of its targets, the two are traced as one branch
  whose guard is *any* or *all* of the tests, inverting the inner test when
  the shared target is on its other side. The registers where both tests
  lead to the same code are what either path leaves, as in
  `x = A() or B() or C()`. Traced test by test, an `elseif` ladder of such
  conditions would trace the rest of the ladder twice at each rung; recent
  quests choose lines and animations by long ladders of this kind. A test
  of a choice's answer is not joined this way, so that its branches can
  move into the choice's options. Branches that still have this shape
  after tracing are joined the same way. A branch then reads in whichever
  direction has fewer negated tests: `if a and b` rather than `if not a or
  not b` with the sides swapped.
- A test a branch around it already made is left out of a branch's
  condition: inside `if a then`, `if a and b` is `if b`. A condition made
  only of such tests is kept whole, and a loop starts over, as what its
  tests read may change between its rounds.

A scene whose code does not have this shape, or that takes more than 20,000
steps to trace, is marked untraced and lists its lines, choices, and markers
in code order without branches.

### Quest variables

The first argument of `IsQuestCompleted`, `IsQuestAccepted`, and
`IsQuestAcceptQualified` is usually a script variable such as `QUEST0`.
Its value in the quest's `Quest` row (read as for cutscenes below) is the
`Quest` row of another quest, so the operand becomes a quest reference with
the variable, that row, and the quest's name and sheet from the quest index
when they are known. A variable without a value stays a field. Variables of
other functions, such as actors, are not resolved. On the current game,
2,152 quest arguments resolve and 2,141 of them have a name; 58 stay fields.

### Cutscenes

`PlayCutScene(<variable>)` names one of the quest's script variables. The
quest's `Quest` row holds them: each non-empty String column of the row is a
variable name, and the `UInt32` column whose data starts four bytes after it
holds its value, such as `CUT_SCENE_01 = 10`. The value is a row of the
`Cutscene` sheet, whose first String column is a path such as
`ffxiv/clsarc/clsarc00110/clsarc00110`, and the cutscene is the file
`cut/<path>.cutb`. The quest variables are read from the raw `Quest` sheet
once per source. A battle script's cutscene variables are those of the
`QuestBattle` rows that the quest's `QUESTBATTLE…` variables name, read the
same way; a name with different values in two rows is left unresolved.

`GameSource::cutscenes_naming` finds every cutscene file that names keys of
a dialogue sheet, with those keys in row order: some cutscenes are played
by battles or duties rather than by the quest's scripts, and cutscene
sheets `cut_scene/…` are played only by cutscene files. The first call reads
every file of the `Cutscene` sheet on up to four threads, about one second
on the current game, and keeps an index in memory. A file that is missing
or is not a cutscene file is left out of the index.

A resolved cutscene in a traced scene keeps its `Cutscene` row and path.
`GameSource::cutscene_plays` finds, for `Cutscene` rows, the scenes and
handlers of quests' scripts that play them: the quests whose `Quest` row,
or a `QuestBattle` row it names, holds one of the rows as a value are
traced, and only the cutscenes their scripts play count. The answer for
each row is kept for the source, so a sheet read again finds its cutscenes'
quests at once. On the current game a sheet's first read takes about 20 to
80 ms in a release build; voiced cutscene sheets' cutscenes usually belong
to a few quests of one story.

A cutscene file holds its timelines as `TMLB` blocks of nodes: a choice
node names the question's key, and the lines that answer each choice can
sit in a later block at the same moment, one for each answer, as in
`ex3/lucact/lucact05100`. Which answer plays which line is decided there,
in a structure no public documentation describes.

Aeria reads which text keys a cutscene file names, not its timelines: the
file must start with `CUTB` and record its own length, and a key is a
NUL-terminated string of ASCII letters, digits, and `_` that starts with
`TEXT_`. Any other file is `SourceError::Cutscene`. The cutscene node lists
the keys of the quest sheet in row order, which is not necessarily the order
the cutscene shows them, and the other dialogue sheets it names keys of,
such as `cut_scene/…` sheets, found by the sheet ID after `TEXT_`. A link
that is missing, such as a variable without a value, leaves the cutscene
without lines.

On the current game, every scene and handler of the quest scripts and
their battle scripts is traced. Of the 217,875 speech rows of quest
sheets:

| Where | Rows |
| --- | --- |
| Played by a scene, handler, or battle script, or a cutscene they play | 213,455 (98.0%) |
| Named only by a cutscene file the scripts do not play | 1,473 (0.7%) |
| Battle talk, marked `…_BATTLETALK` in the key, named by no file | 817 (0.4%) |
| Named by no script or cutscene file | 2,130 (1.0%) |

No quest script names a line its tracing misses. The rows named by no file
are mostly system text, battle talk, placeholder (`DUMMY`) and test quests;
the game may show them by row number from its own code, or not at all.

## Performance

Reading every sheet of one language takes about one to two seconds on one
thread. A sheet is read on demand and takes milliseconds. Permission reads
the sheet in the three other evidence languages as well. Opening a project
reads every sheet that holds translation units, once.

## Degraded workspace

A project whose game is older than the project's game version, or that has
no game folder configured, cannot be edited. Git history, the manifest, the
glossary, and translations remain inspectable in the repository, but editing,
preview, assisted translation, and export require the game at the project's
version or newer.

## Additional source languages

Additional official languages are local context for a translator. They are
read from the same installation and are never recorded in the project.

`GameSource::cell_in_other_languages(sheet, row, subrow, column)` returns
one cell's macro text in every evidence language other than the source
language. A language has no text when the sheet cannot be read in it, when
it has another variant or other String columns there, or when it lacks the
row. The other-language sheets of the last two sheets asked about are kept
in memory.
