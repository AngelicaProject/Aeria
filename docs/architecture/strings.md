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
  and the translation agent, and stored in workspaces as the source and
  target text of units. `aeria_se::print` writes it and `aeria_se::parse`
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
summary. Printing, parsing, validation, tagged text, legends, and the editor
all read it, so naming a new macro code is one new entry with a golden
vector.

Tag names and forms are part of the persisted format: workspaces store
macro text, so renaming an entry or changing its form changes stored text
and requires a workspace format change. Naming a previously unnamed code
changes how its strings print, and a source update then reports those units
as changed.

### Macros added by the game

A game patch can add macro codes or change the arguments of a known one. The
byte layer needs no catalog, so such a macro still reads, prints generically
as `<code:XX …>`, encodes back to the same bytes, and is opaque to the
editor, the preview, and the translation agent. Strings that contain it stay
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
agent, such as `<colour> is not a macro; did you mean <color>?`, `<if> is
missing its closing </if>`, or `<sheet> takes 2 or more (sheet, row, column,
parameter, …)`. After an error the parser skips to the end of the tag and
continues, so later text is still read. Nesting is bounded by 128 levels of
tags, quoted strings, and expressions; deeper input is reported, not
recursed into. The deepest game string nests well below that limit.

Block content keeps its span, so a branch can be replaced in the source
text without reprinting the rest.

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

The product grows three complementary views:

- Visual editor with structured macro tokens and controls.
- Raw macro text editor, always parsed and validated before save.
- Preview that shows each tag as it looks in the game: colors, italics,
  line breaks, icons, game data names, and selectable condition branches.

## AI boundary

AI receives structured translatable content with protected placeholders and never writes raw macro text. Structural edits beyond the policy below may later be exposed as explicit validated operations.

### Tagged text

`aeria_se::project` turns a well-formed source string into tagged text. Prose
is plain text with `&`, `<`, and `>` written as `&amp;`, `&lt;`, and `&gt;`.
Each root or nested protected construct becomes a numbered tag in source
order:

- `<x id="N"/>` for a construct without translatable content, including
  opaque constructs and each tag of a pair;
- `<g id="N"><b>…</b>…</g>` for a block whose translatable parts are
  translated in place, one `<b>` per part, such as the branches of `<if>` or
  `<switch>`.

Each tag has a legend entry built from the catalog: its exact spelling, what
it does with its argument values (for example `a value from a game data
sheet, such as a name; sheet = Item, row = number parameter 1, column = 0`),
its family, its branch count, and whether it may repeat. A malformed source
has no projection and is not offered for assisted translation.

`aeria_se::rebuild(source, tagged)` parses a tagged translation, checks it,
and rebuilds the target by copying each construct's exact source spelling
and splicing the translated text into the block content, escaped as macro
text (`\\`, `\<`, `\{`).

### Assisted structure policy

A tagged translation and, independently, the rebuilt macro text
(`aeria_se::check_assisted_structure`) must satisfy:

- every source construct is kept; no construct kind is droppable yet;
- a construct stays in its container, the top level or one branch of one
  construct;
- constructs may move within their container, except that formatting
  constructs keep their relative order, so opening and closing tags cannot
  cross;
- only runtime values without branches, such as a number, may repeat;
- constructs with branches keep their number of branches;
- nothing else may be added. A changed game reference, parameter, argument,
  or opaque construct is a different construct and is rejected.

The text check compares constructs by their code and arguments, with only
the content of translatable branches masked, and then compares the branches
of matching constructs recursively. Refusals are returned as messages written
for the model, so it can correct its translation. This policy, not strict
structure comparison, is the acceptance rule for assisted translation.
