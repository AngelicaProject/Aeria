# Project knowledge and translation rules

What a translation follows besides its source: the project's own decisions,
kept as files in the project, and the rules every translation follows,
whoever writes it. Machine translation sends both with every request (see
[`translate.md`](./translate.md)); the checks of
[`po-project.md`](./po-project.md#checking) enforce what can be checked.

## The game is its own glossary

Names of people, places, monsters, items, actions, and statuses are strings of
their sheets in `po/`, and their translations are how the project renders them
everywhere. Context comes from the game too: a quest or cutscene is one file
with its journal, objectives, and dialogue, each line names its speaker, and
every string carries the other client languages. The project knowledge holds
only what the game cannot tell.

## Project knowledge

The knowledge is the `aeria-knowledge` directory at the project root:

- `style.md`: how the translation reads, written by a person: how formal it
  is, how the player and other characters are addressed, how names are
  rendered, the tone of each kind of text. A new project starts with the
  usual decisions written down (see
  [`knowledge-v1.md`](../formats/knowledge-v1.md#files)), for the
  translators to change rather than to write from nothing;
- `terms.csv`: terms that are not strings of the game, such as lore words,
  with their translation, their other forms in the source (`linkshells`
  beside `linkshell`, `the Scions` beside `Scions of the Seventh Dawn`), a
  note, and the folder people file them in.

The format is [Project Knowledge Format v1](../formats/knowledge-v1.md);
`aeria-knowledge` owns reading and checking it. The knowledge is committed with the translations (see
[`git.md`](./git.md#checkpoint)), and a new project gets both files without
entries.

The desktop edits the terms and the style in the project knowledge dialog
(see [`desktop-editor-ui.md`](./desktop-editor-ui.md#project-knowledge-dialog)),
which also lists glossary candidates: names the project translates in
several ways (see [`search.md`](./search.md#glossary-candidates)), and the
phrases its translations open sentences with (see
[`search.md`](./search.md#openers)).
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
- for Russian, notes on living language and `machine_phrasing`, a word-level
  check for officialese and calques: only words that are almost never right
  in a game's text, since what a line's punctuation and links should be
  depends on its source and its kind, which one line does not show. It is
  reported as advice.

`style.md` takes precedence over the style defaults of these rules.
