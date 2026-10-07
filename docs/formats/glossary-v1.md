# Glossary Format v1

Status: **implemented in `aeria-knowledge`**.

Glossary Format v1 is the format of the project's terms,
`aeria-knowledge/terms.csv`, part of the
[project knowledge](./knowledge-v1.md). It is committed with the project,
reviewed and merged through Git like any other file, and meant to be edited
by hand, in a spreadsheet, or in the desktop's knowledge editor.

How Aeria uses it is described in
[`../architecture/knowledge.md`](../architecture/knowledge.md#project-knowledge).

## Absence

A missing file means the project has no terms.

## Encoding

- UTF-8, with an optional leading BOM that is ignored.
- RFC 4180 CSV: comma-separated fields, fields quoted with `"` when they
  contain a comma, quote, or line break, and `""` for a quote inside a quoted
  field. CRLF and LF line endings are both accepted.
- At most 8 MiB and 100,000 entries.

## Header

The first record names the columns, in any order:

| Column | Required | Meaning |
| --- | --- | --- |
| `term` | yes | The term's headword as written in the project's source language. It names the term: in term exceptions and wherever the term is shown. |
| `translation` | yes | The translation to use. |
| `forms` | no | Other forms of the term in the source, separated by `;`: inflected forms (`linkshells` beside `linkshell`) and other wordings of the same term (`the Scions` beside `Scions of the Seventh Dawn`). Each is matched as the term. |
| `note` | no | Guidance for translators, such as usage, gender, or declension. |
| `folder` | no | The folder the term is filed in: folder names from the top, separated by `/` (`Lore/Places`). Empty for a term at the top. Folders only organize terms for people; they never change how a term is matched, checked, or sent to machine translation. |
| `case` | no | `yes`, `true`, or `1`, ignoring case, when every form of the term matches only with its case as written, but for a capital first letter: `the Maelstrom` matches `The Maelstrom` and not `the maelstrom`. Anything else, or an empty field, matches ignoring case. |

A missing `term` or `translation` column, a repeated column, or any other
column name makes the whole file invalid. Aeria reports the problem and uses
no entries from the file; it never guesses at an unknown column.

## Records

Each following record is one entry. Surrounding whitespace in every field,
and in every form, is ignored, and records whose fields are all empty are
skipped. Empty forms are dropped. A folder's names are trimmed and empty
names dropped, so ` Lore / Places/` is `Lore/Places`. A record is excluded,
and reported with its line number, when:

- its `term` or `translation` is empty;
- its headword or one of its forms repeats a headword or form of an earlier
  term, or another of its own, compared case-insensitively.

Excluded records do not make the file invalid; the remaining entries are used.

## Canonical form

When the desktop's knowledge editor saves the file, it writes the header
`term,translation,forms,note,folder`, with `,case` after it only when an
entry matches case, one record per entry in the editor's order, LF line
endings, quoting only where required, forms joined with `; `, folders in
their trimmed form, and `yes` or an empty field for `case`.
The editor shows excluded records and drops them only after the user
confirms; it also replaces a file that cannot be read at all only after
confirmation. The editor never writes an entry that would be excluded.

Example:

```csv
term,translation,forms,note,folder
linkshell,линкшелл,linkshells,,Interface
Scions of the Seventh Dawn,Потомки Седьмой Зари,the Scions; Scions,,Lore/Organizations
"Sage, elder",Старейшина,,Title in Sharlayan,
```

## Matching

Terms are matched in the text of a string, never in its macros: each tag
with its arguments counts as a space, so the sheet name in
`<sheet Aetheryte $n1 8>` is not the word *Aetheryte*, while text between
tags, such as the branches of a condition, is text. Each form of a term,
its headword included, matches that text case-insensitively, or with its
case when its entry says so, as a whole word when the form starts and ends
with a letter, digit, or underscore. Forms of one term that overlap, such as
`the Scions` and `Scions of the Seventh Dawn` in `the Scions of the Seventh
Dawn`, are one occurrence of the term. Where a longer term's match covers a
shorter term's, the words are the longer term's alone: `Scions of the
Seventh Dawn` is not also `Scions`. A string's term exceptions take their
terms out of matching for that string; an exception names a term by its
headword, and one that names another of its forms takes the term out too (see
[`../architecture/po-project.md`](../architecture/po-project.md#term-exceptions)).
Matching selects the terms a batch of machine translation needs and drives
the checks of a translation; it never changes a translation.

- An **inflected form** of a word of a translation is its stem (the word
  without its final vowels, `й`, and `ь`), then letters of that kind, then
  a declension ending of at most four letters in all: `а`, `ом`, `ами`,
  `ого`, `ых`, and the other endings of nouns and adjectives (`ENDINGS` in
  `aeria-knowledge`). Any other letters make another word: `Этерис` is not
  a form of `этер`, nor `эфироита` of `эфирит`.
- A term whose translation does not seem to be used is reported as advice:
  each word of the translation must appear in the target, as written or
  inflected, so `эфирита` uses `эфирит` and `эфироита` does not. Stems with a vowel that drops when declined (`боец`,
  `бойца`) are not recognized and give advice wrongly. A term the
  translation keeps as written at least as many times as the source has it
  is part of a name left untranslated, such as the song title in
  `Scions & Sinners`, and gives no advice. The source's occurrences are
  counted over all forms of the term, and so are the forms the translation
  keeps.
