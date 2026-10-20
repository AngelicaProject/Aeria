# Project knowledge and translation rules

What a translation follows besides its source: the project's own decisions,
kept as files in the project, and the rules every translation follows,
whoever writes it. Machine translation sends both with every request (see
[`translate.md`](./translate.md)); the checks of
[`po-project.md`](./po-project.md#checking) enforce what can be checked.

## The terms are the one glossary

What the project agreed to render one way is a term, whatever it is: a lore
word, a name of a person, a place, or a thing, or a word that names are
made of, such as a material or a kind of item. A term is a name of the
source, its translation as it stands alone, a category saying what the name
is (`место`, `существо`, `организация`), and a note. The terms come from two
places:

- `terms.csv`, written by a person or an agent;
- the sheets the project declared part of the glossary (`term-sheets.csv`):
  sheets where a row is a name the rest of the game refers to, such as the
  titles of quests or the names of characters. Each name of such a sheet is
  a term with the sheet's category: its translation is the one its strings
  have most often, so a name translated after a game update is a term at
  once, and a row translated another way does not take the name.

A term of `terms.csv` takes the place of the sheet's term of its name and
category: it is how a person corrects or settles a name of a sheet, and the
editor writes a change of a sheet's term there. A name may mean several
things in several categories, such as `Mist`, a district and a creature:
each meaning is a term, and machine translation gets every meaning of a name
found in a string, with its category and note, and uses the one the string
means.

The terms say how a name is translated, not how a sentence writes it: the
translation is the name as it stands alone, and a sentence puts it in the
number, case, and capitalization it needs. Whether a translation does so is
the model's and the person's judgment: no check reads a term in a
translation. Each translation records the version of each term of its
source it was written with, so a term changed in the glossary reaches
exactly the strings written with its old version: the glossary counts them
and a correction run brings them to the term (see
[`translate.md`](./translate.md#following-changed-terms)), whatever the
string is, a row of the term's own sheet included. The glossary never
writes a string itself.
`aeria_po::project_knowledge` reads the terms of the declared sheets, and a
project's session reads them again when a file of a declared sheet changes.

The other strings of the game that are names (items, actions, characters)
are strings like any other: they follow the terms as a line of dialogue
does. A name that is neither a term
of the file nor of a declared sheet is translated where it occurs, and
nothing ties its occurrences together.

Context comes from the game: a quest or cutscene is one file with its
journal, objectives, and dialogue, each line names its speaker, and every
string carries the other client languages.

## Project knowledge

The knowledge is the `aeria-knowledge` directory at the project root:

- `style.md`: how the translation reads, written by a person: how formal it
  is, how the player and other characters are addressed, how names are
  rendered, the tone of each kind of text. A new project starts with the
  usual decisions written down (see
  [`knowledge-v1.md`](../formats/knowledge-v1.md#files)), for the
  translators to change rather than to write from nothing;
- `term-sheets.csv`: the sheets of the game that are part of the glossary,
  each with the category and note of its names;
- `terms.csv`: the terms, with their translation, category, note, other
  wordings in the source (`the Scions` beside `Scions of the Seventh
  Dawn`);
- `instructions/`: what a person asks machine translation to do to the
  translations of a run, one Markdown file each, such as how to fix the
  agreement with the player character's gender. Every review of the
  corpus is such an instruction, not a mode of Aeria.

The format is [Project Knowledge Format v1](../formats/knowledge-v1.md);
`aeria-knowledge` owns reading and checking it. The knowledge is committed with the translations (see
[`git.md`](./git.md#checkpoint)), and a new project gets the files without
entries.

The desktop edits the terms in the glossary tab, the style in the style
dialog, and the instructions in the editing tab (see
[`desktop-editor-ui.md`](./desktop-editor-ui.md#glossary-tab) and
[`desktop-editor-ui.md`](./desktop-editor-ui.md#editing-tab)).
A file that cannot be read,
and rows the format excludes, are reported with their file and line and left
out; the rest of the knowledge is used.

## Translation rules

`aeria_knowledge::rules` holds the rules every translation follows:

- what the game's texts are: the Japanese original and three finished
  localizations, what each is evidence for, and that content comes only from
  the source line;
- how a line reads as if written in the target language, and the known causes
  of dry, machine-written text;
- how the player character is addressed without assuming a gender, with
  conditions on `$gn4` in gendered languages only for words about the player
  character (whom a line speaks to is read from the scene: a "you" said to
  another character agrees with that character's own gender), and how a comparison with the
  player's name (`<if ($gs1 == $gs2)>`) splits a message between the player
  character and someone else, whose words agree inside each branch;
- how translations are written as macro text, and that a name the game fills
  in never declines, so a line puts it where its stored form is right; that
  `\<sigh>` and `\<click>` are sounds of the English localization, said in
  the line itself as the Japanese does (Эх…, Бип!), and a gesture by the
  line's own words (Хе-хе), never dropped, unless the style says otherwise;
- for Russian, notes on living language: officialese, calques, and the
  other marks of machine-written text, which the model weighs; no check
  looks for them.

`style.md` takes precedence over the style defaults of these rules.
