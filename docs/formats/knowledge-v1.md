# Project Knowledge Format v1

Status: **implemented in `aeria-knowledge`**.

Project knowledge is what a person decided about the translation that the
game cannot tell: how it reads, and the terms it renders the same way that are
not strings of the game. Names of people, places, items, actions, and other
things the game names are strings of their sheets in `po/`; their
translations are the project's glossary for them, and they are not repeated
here.

The knowledge is the `aeria-knowledge` directory at the project root, next to
`po/`. It is committed with the project and shared through Git like any other
file. How Aeria uses it is described in
[`../architecture/knowledge.md`](../architecture/knowledge.md#project-knowledge).

## Files

Creating a project writes both files: `style.md` with a starting style of the
usual decisions (address following the French localization's tu and vous,
how names and places are written, short interface labels, the target
language's punctuation) for the translators to change, and `terms.csv` with
its header only. Every file is
optional, UTF-8 with an optional leading BOM that is ignored, at most 8 MiB,
with CRLF or LF line endings. Aeria writes LF.

| File | Format | Holds |
| --- | --- | --- |
| `style.md` | Markdown, free text | How the translation reads: how formal it is, how the player and other characters are addressed, how names are rendered, the tone of each kind of text |
| `terms.csv` | [Glossary Format v1](./glossary-v1.md) | Terms that are not strings of the game, such as lore words, with their other forms, filed in folders |

`style.md` is written by a person and used as a whole. A file that cannot be
read is reported and left out. Rows of `terms.csv` that its format excludes
are reported with their line; the rest are used.
