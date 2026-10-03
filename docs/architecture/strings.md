# Structured game strings

Game strings are not plain prose. They may contain nested macros, conditions, formatting, runtime values, game references, and other engine-readable constructs.

Aeria must never treat these constructs as disposable decoration.

## Layers

`aeria-se` owns game strings in three layers:

- **Bytes.** The game stores each string as `SeString` bytes.
  `aeria_se::bytes` reads them into the string model and writes them back.
- **Model.** A string is a sequence of text, macros, and raw bytes. A macro
  has a code and argument expressions: integers, strings, named game values,
  parameters, and comparisons.
- **Macro text.** The written form of the model, read and edited by people
  and machine translation, and stored in the project's PO files as the
  source (`msgid`) and translation (`msgstr`). `aeria_se::print` writes it and `aeria_se::parse`
  reads it into a syntax tree with source spans and diagnostics.

`aeria_se::codec` combines them: `decode` prints bytes as macro text, and
`encode` reads macro text back into bytes. `encode_checked` adds the checks a
pack string needs: the bytes are not empty, contain no `0x00`, are at most
65,535 bytes, and encode to the same bytes again after decoding.

### Lossless bytes

`bytes::decode` accepts any input, and `bytes::encode(&bytes::decode(b)) == b`
for every input. Anything that is not canonical text, a canonical macro
payload, or a canonical expression is kept as raw bytes: invalid UTF-8, a
`0x00` byte, a truncated or malformed payload, a non-canonical integer, or an
unnamed game value. Because the model is lossless and the printed form is
canonical, two strings have the same macro text exactly when they have the
same bytes.

Raw bytes print as `<raw XX …>` or, inside a macro, `raw(XX …)`. They show a
string that cannot be written, so macro text containing them has a
`RawBytes` diagnostic: it is invalid as a translation and cannot be encoded.
In the current game such bytes occur only in the binary data of the
`QuestDefineClient` and `CustomTalkDefineClient` sheets.

## Macro text

Macro text is text with tags. Outside tags only three characters are
special and are escaped with a backslash: `\\`, `\<`, and `\{`; inside a
quoted string `\"` is escaped as well. No other escape exists.

### Tags

Every named macro code has one entry in `aeria_se::catalog`, which fixes its
tag name, form, and arguments. There are three forms:

| Form | Written as | Examples |
| --- | --- | --- |
| Inline | `<name arg arg>`, or `<name>` without arguments | `<br>`, `<num $n1>`, `<sheet Item $n1 0>` |
| Pair | an opening tag and a closing tag, each one macro in the bytes | `<i>…</i>`, `<color #FF13212F>…</color>`, `<ui-color 504>…</ui-color>` |
| Block | an element whose content holds its text arguments | `<if ($n1 == 1)>him<else>her</if>`, `<switch $weekday><case>Sunday<case>Monday</switch>`, `<capitalize>…</capitalize>` |

A pair is lexical: `<i>` and `</i>` are two independent macros, and the
closing tag is the macro whose argument closes the pair (`0` for `</i>`,
`$stackcolor` for `</color>`). Unbalanced pairs are valid macro text,
because game strings may open a format in one place and close it in another.
An opening tag omits an argument only where the catalog implies it, as `1`
for `<i>`.

A block's text arguments are its content. With a separator, content parts
are separated by `<else>` (conditions), `<rt>` (ruby text), or introduced by
`<case>` (every case of a switch). Trailing empty parts separated by
`<else>` are omitted: `<if $gn94>x</if>` has an empty else branch. A part
that is not text, such as a number chosen by a condition, is written as a
whole-part value in braces: `<if ($gn68 == 19)>{220}<else>{150}</if>`. Text
in a part always stays text, even when it reads as a number.

A code without a catalog entry, or arguments that do not fit the entry, is
written generically as `<code:XX arg …>` with the code in hexadecimal.

### Values

Inline arguments and brace values are separated by spaces:

| Value | Written as |
| --- | --- |
| Integer | decimal `42`; a color role prints `#AARRGGBB`, and `#hex` is accepted for any integer |
| Text | a bare word such as `Item` when it is a letter or `_` followed by letters, digits, or `_`; otherwise a quoted string `"…"`, which may contain tags |
| Parameter | `$n1` and `$s1` for the string's number and text parameters, `$gn68` and `$gs1` for global numbers and texts; `$n(…)` for a non-integer operand |
| Game value | `$msec`, `$sec`, `$min`, `$hour`, `$day`, `$weekday`, `$month`, `$year`, `$stackcolor` |
| Comparison | `($n1 == 1)` with `==`, `!=`, `<`, `<=`, `>`, or `>=`, always in parentheses |

Spacing inside a tag is free; the printed form uses single spaces.

### Canonical form and round trip

`print` writes one canonical spelling for each model, and
`parse(print(model))` has no diagnostics and yields the same model. The
ignored test `crates/aeria-source/tests/game_corpus.rs` checks every distinct
string of an installed game in all four source languages: each prints as
macro text that encodes back to its bytes, or holds raw bytes. The golden
vectors in `crates/aeria-se/tests/fixtures/macro_text.golden.txt` pin bytes
and text for every form and every catalog entry.

### Catalog

`aeria_se::catalog` is the single table of named macros. Each entry holds
the code, tag name, form, arguments (name, role, and whether inline or
block), required and repeating arguments, semantic family, and a one-line
summary. Printing, parsing, validation, the machine translation instructions, and the editor
all read it, so naming a new macro code is one new entry with a golden
vector.

Tag names and forms are part of the persisted format: the PO files store
macro text, so renaming an entry or changing its form changes stored text
and requires a project format change. Naming a previously unnamed code
changes how its strings print, and a game update then marks those
translations fuzzy.

### Macros added by the game

A game patch can add macro codes or change the arguments of a known one. The
byte layer needs no catalog, so such a macro still reads, prints generically
as `<code:XX …>`, encodes back to the same bytes, and is opaque to the
editor and machine translation. Strings that contain it stay
translatable. A new kind of expression inside a macro is different: its bytes
stay raw, and those strings cannot be translated until the byte model learns
it.

The game corpus test lists the codes the catalog does not name (see
[`../development/testing.md`](../development/testing.md#game-strings)).
Naming one is a catalog entry and a golden vector. Pack readers check only the
catalog-free [well-formed rule](../formats/pack-v1.md#well-formed-strings), so
a new macro never needs a Harmonia release.

### Parsing and diagnostics

`parse` never fails. It returns the source, a syntax tree, and diagnostics
with byte spans, kinds, and messages written for people and the translation
model, such as `<colour> is not a macro; did you mean <color>?`, `<if> is
missing its closing </if>`, or `<sheet> takes 2 or more (sheet, row, column,
parameter, …)`. After an error the parser skips to the end of the tag and
continues, so later text is still read. Nesting is bounded by 128 levels of
tags, quoted strings, and expressions; deeper input is reported, not
recursed into. The deepest game string nests well below that limit.

Block content keeps its span, so a branch can be replaced in the source
text without reprinting the rest.

### Idioms

`catalog::IDIOMS` names constructs of several macros whose meaning game
strings establish, each written as one exact macro text:
`<split " " 1><string $gs1></split>` is the player character's first name
(13,280 strings of the current game) and `<split " " 2><string $gs1></split>`
the last name (850). An idiom is still its macros: macro text, encoding,
validation, and the structure policy treat them as before. The editor shows
it as one value with the macros in its tooltip, and the list of the
source's constructs describes it by its meaning. The editor reads the table from the `macro_idioms`
command. Naming another idiom is one entry whose text is canonical macro
text.

### Insertions

`catalog::INSERTIONS` lists the macros a person may insert while
translating, each in the form the game's dialogue uses for it: the player
character's full name `<string $gs1>` (13,553 strings), first and last name
(the idioms above), class or job `<sheet ClassJob $gn68 0>` (504), and race
`<sheet Race $gn71 0>`; a choice by gender `<if $gn4>…<else>…</if>`
(6,505), and by one race or class or job, `<if ($gn71 == row)>` and
`<if ($gn68 == row)>`, offered for each row of `Race` and `ClassJob`; and
`<i>`, `<capitalize>`, and `<nbsp>`. Every insertion is canonical macro text,
and the globals it reads have an established meaning.

### Speaker names

A line that starts with `(-name-)` is shown with that name in place of the
speaking character's own, such as `(-???-)` for someone not yet introduced
or `(-Exuberant Newcomer-)`. The markers are plain text in the bytes, not a
macro, so macro text and encoding treat them as text; `aeria_se::speaker`
reads them as syntax. A speaker name opens with `(-` at the very start of
the string and closes at the first `-)` after it in text outside macros;
the name between them is text players read and may hold macros, such as
`(-<i>An Introduction to the Heavens</i>-)`. On the current game 8,169
strings of quest, voiced cutscene, `custom`, dungeon, and `DefaultTalk`
sheets start with one, and `(-` occurs nowhere else at the start of a
string.

## Validity

`MacroString::semantic_validation` reports one of:

- **Valid and understood**: no diagnostics, and every macro has a known
  meaning.
- **Valid with opaque constructs**: no diagnostics, but the text contains a
  generic `<code:XX>` macro or a macro whose meaning is not established. They
  are preserved exactly and never interpreted.
- **Invalid**: the text has diagnostics, including raw bytes. It cannot be
  encoded, exported, or saved as a translation.

`MacroString::plain_text` returns what players read: text and the content of
translatable blocks, with a space where a macro separates two pieces. Search
indexes it. `is_formatting_only` reports well-formed text whose readable
text has no letter.

## Semantic families

Every catalog entry has a family that decides the translation policy:

| Family | Meaning | Examples |
| --- | --- | --- |
| Translatable text | a transform whose content is text | `capitalize`, `lower`, `string` |
| Formatting | presentation state; formatting keeps its order | `i`, `b`, `color`, `ui-color` |
| Condition | a choice between text branches | `if`, `switch`, `if-gender`, `if-self` |
| Runtime value | a value supplied at runtime | `num`, `player-name`, `kilo` |
| Game data reference | a value read from game data | `sheet`, `noun-en`, `platform` |
| Layout control | line breaks, spaces, icons, sounds | `br`, `nbsp`, `icon`, `ruby` |
| Opaque protected | a construct whose meaning is not established | `key`, `split`, `edge`, `icon2` |

A block argument is translatable when its role is text and the family is
not opaque. The content of `<split>`, for example, is preserved.

## Editor modes

The desktop editor shows macro text in two ways (see
[`desktop-editor-ui.md`](./desktop-editor-ui.md)):

- **Text**: tags as compact chips, formatting pairs as styled text, and
  `<br>` as a line break, for translating.
- **Code**: the macro text with every tag written out.

The document is always the exact macro text, parsed and validated by Rust
before save. The editor does not evaluate conditions or fill in values. A
person's translation is held to the same structure policy as a machine
translation (see [Structure policy](#structure-policy)).

### Known global parameters

`catalog::GLOBALS` names the global parameters whose meaning game strings
establish, each with the usage that shows it: `$gs1` is the player's name
(`<if ($gs1 == $gs3)>your`), `$gn4` is 1 for a female player character (`A
<if $gn4>woman<else>man</if> your size`). Messages about other people,
such as the party and battle log, name them with `$gs2` and `$gs3` and
compare each with `$gs1` to tell the player character apart; `$gn7` and
`$gn8` are the `ObjStr` row of what they name when it is not a player
(`<if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if>`), and `$gn5` and
`$gn6` are 1 for a female player (`<if $gn5>her<else>his</if>`). `$gn52`–`$gn54` are the Grand
Company ranks, `$gn68` is the class or job as a `ClassJob` row (`($gn68 ==
17)` beside "class is changed to botanist"), `$gn71` is the race as a `Race`
row, and `$gn72` is the level (compared with trait levels). Other globals
have no name until strings establish their meaning.

## Structure of a translation

People and machine translation read and write macro text, as in the editor's
code mode, and localize a string's structure as its language needs. What the
game fills in is kept by a deterministic policy, not by anyone's judgment.
`aeria_se::assisted` owns both parts.

### Constructs

`aeria_se::constructs(source)` lists every macro of a well-formed source
once, in source order, with what it does (from the catalog, with its argument
values and the meaning of a known global, such as `condition = global number
4 (1 when the player character is female, 0 when male)`; an idiom reads as its
meaning) and the rule a translation follows for it:

| Rule | Constructs | A translation |
| --- | --- | --- |
| Game data | runtime values (`num`, `player-name`, `string $gs1`), game data references (`sheet`, `noun-en`), icons, sounds, waits, links, opaque constructs, unknown codes, raw bytes | keeps it; it may move or repeat |
| Formatting | `i`, `b`, `color`, `ui-color`, and their ends | may add, drop, or move it, and closes what it opens as the source does |
| Condition | `if`, `switch`, `if-gender`, `if-self`, and other conditional selection | may reword, restructure, add, or drop it |
| Free | `br`, `nbsp`, `shy`, `hyphen` | may add or drop it |
| Letter case | `upper`, `capitalize`, `title-case`, `lower`, `lower-first` | may add or drop it; the explanation says that game data inside one, such as a name the game stores in lower case, shows in that case only through it, so a case transform stays around game data, with `capitalize` in place of `title-case` in a language that capitalizes only the first word |

A malformed source has no constructs and cannot be translated.
`aeria_se::authoring_reference()` is a short reference of condition syntax
and the known globals for the machine translation instructions.

### Structure policy

`aeria_se::check_assisted_structure(source, target)` accepts a target that:

- is well-formed macro text;
- keeps every piece of game data of the source, compared by its code and
  arguments with translatable text masked: a changed item, sheet, or
  parameter is other data. Game data may move and repeat, anywhere in the
  string, including into or out of a condition's branches, and the target
  adds none the source lacks;
- leaves each formatting macro (`i`, `b`, the colors) no more open at its
  end than the source does (it may close what the source forgot to close),
  and closes no more than the source does of what came before; otherwise
  formatting is the translation's own, since the official localizations
  format about four in five formatted strings differently from the English
  (italics above all), so it may be added, dropped, or moved;
- tests, in conditions it has, only parameters the source uses or globals
  whose meaning is established (see [Known global
  parameters](#known-global-parameters)); constants and game values such as
  `$hour` are always allowed;
- starts with a speaker name exactly when the source does, and does not
  leave a name the source gives empty (see [Speaker names](#speaker-names)).

Everything else is the translation's own: word order, wording, line breaks,
and conditions a language needs, such as a verb that agrees with the player
character's gender (`Чего <if $gn4>застыла<else>застыл</if>?`) where English
has none. The policy is checked on every string of the current game as its
own translation, and every one of the 2,474,046 well-formed strings passes.
Refusals are returned as messages written for the model, so it can correct
its translation.
