# Project Knowledge Format v1

Status: **implemented in `aeria-knowledge`**.

Project knowledge is what a person decided about the translation: how it
reads, and the terms it renders the same way, the names of people, places,
and things it agreed on among them.

The knowledge is the `aeria-knowledge` directory at the project root, next to
`po/`. It is committed with the project and shared through Git like any other
file. How Aeria uses it is described in
[`../architecture/knowledge.md`](../architecture/knowledge.md#project-knowledge).

## Files

Creating a project writes every file: `style.md` with a starting style of the
usual decisions (address following the French localization's tu and vous,
how names and places are written, short interface labels, the target
language's punctuation) for the translators to change, and `terms.csv` and
`term-sheets.csv` with their headers only. Every file is
optional, UTF-8 with an optional leading BOM that is ignored, with CRLF or
LF line endings, `style.md` at most 8 MiB and `terms.csv` at most 64 MiB.
Aeria writes LF.

| File | Format | Holds |
| --- | --- | --- |
| `style.md` | Markdown, free text | How the translation reads: how formal it is, how the player and other characters are addressed, how names are rendered, the tone of each kind of text |
| `terms.csv` | [Glossary Format v1](./glossary-v1.md) | The terms: lore words and the names the project agreed on, with their other forms and categories |
| `term-sheets.csv` | [Declared sheets](#declared-sheets) | The sheets of the game that are part of the glossary |

`style.md` is written by a person and used as a whole. A file that cannot be
read is reported and left out. Rows of `terms.csv` that its format excludes
are reported with their line; the rest are used.

## Declared sheets

`term-sheets.csv` lists the sheets of the game whose names are terms: a
sheet where a row is a name the rest of the game refers to, such as the
titles of quests. It is RFC 4180 CSV as `terms.csv` is, with a header of
these columns in any order:

| Column | Required | Meaning |
| --- | --- | --- |
| `sheet` | yes | The sheet's name as the game has it, such as `Quest` or `custom/000/Names`. |
| `column` | no | The column of the strings that are names; 0 when absent or empty. |
| `category` | yes | What the sheet's names are, as a short label (`задание`, `место`): the category of its terms (see [Glossary Format v1](./glossary-v1.md#header)). |
| `note` | no | What the sheet's names are and where they apply, such as that they are titles of quests. It is the note of every term of the sheet. |

Another column name, a repeated column, or a missing `sheet` or `category`
column makes the file invalid; it is reported and no sheet is declared. A
record with an empty sheet or category, a column that is not a number, or a sheet and column declared
above is reported with its line and left out. Aeria writes the header
`sheet,column,category,note`, one record per sheet in the order they were
declared, and LF line endings.

### The terms of the sheets

A name of a declared sheet is the source of a string of its column that is
translated, not fuzzy, plain text without macros, and has a letter (`1` or
`???` names nothing); sources that differ only in case are one name. The strings of one name in the sheets of one
category give one term:

- its translation is the one most of those strings have, compared as
  written; between translations as frequent, the one of the first string
  in the order the sheets are declared and in file and entry order. A
  string with a term exception for the name, a person's (see
  [`../architecture/po-project.md`](../architecture/po-project.md#term-exceptions)),
  means another thing and does not count;
- its headword is the source of the first of those strings, and the other
  spellings of the name are its forms; a name matches as a sheet writes
  it, with its case but for a capital first letter, so `In the Mist` is not
  the words of a sentence;
- its category is the sheets' category, and its note the sheets' note, then
  the translator notes of its strings (their `# ` lines), each once, each
  on its own line.

A name translated alike in several categories, ignoring case, is one term,
of the category where it first appears: a character and the same character
as a foe.

A record of `terms.csv` of a category the sheets declare, ignoring case,
takes the note of that category before its own: the notes of the
category's declared sheets, each once, in the order they are declared,
then the lines of its own note the category does not already say, each on
its own line. The category's note is said wherever the term is explained,
to machine translation and in the string guide, but is not part of the
record as written: `terms.csv` keeps its own note, and a model's decision
on the term holds while its own note is unchanged (see
[`../architecture/po-project.md`](../architecture/po-project.md#model-exceptions)).

A record of `terms.csv` with the name as its headword or a form, ignoring
case, takes the place of the sheets' term of that name in its own category,
ignoring case; a sheets' term of the name in another category stays another
meaning of the name. The terms of the
sheets come after those of `terms.csv`, in the order their names first
appear. A record that takes a name's place with the same translation, no
note, and no other forms than the sheets' spellings of the name says no
more than the sheets: taking it out of `terms.csv` changes nothing. As for any term, the note goes to machine translation and to the
string guide, and a change of a term's translation, form, note, or category
asks a correction run to decide its strings again.
