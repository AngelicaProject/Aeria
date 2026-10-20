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
- At most 64 MiB. The entries are not counted: a project that keeps every
  name it agrees on as a term has tens of thousands.

## Header

The first record names the columns, in any order:

| Column | Required | Meaning |
| --- | --- | --- |
| `term` | yes | The term's headword as written in the project's source language. It names the term: in term exceptions and wherever the term is shown. |
| `translation` | yes | The translation to use. |
| `forms` | no | Other forms of the term in the source, separated by `;`: inflected forms (`linkshells` beside `linkshell`) and other wordings of the same term (`the Scions` beside `Scions of the Seventh Dawn`). Each is matched as the term. |
| `note` | no | Guidance for translators, such as usage, gender, or declension. |
| `case` | no | `yes`, `true`, or `1`, ignoring case, when every form of the term matches only with its case as written, but for a capital first letter: `the Maelstrom` matches `The Maelstrom` and not `the maelstrom`. Anything else, or an empty field, matches ignoring case. |
| `category` | yes | What the term names in this meaning, as a short label (`организация`, `существо`): a name may have one meaning per category. |

A missing `term`, `translation`, or `category` column, a repeated column, or any other
column name makes the whole file invalid. Aeria reports the problem and uses
no entries from the file; it never guesses at an unknown column.

## Records

Each following record is one entry. Surrounding whitespace in every field,
and in every form, is ignored, and records whose fields are all empty are
skipped. Empty forms are dropped. A record is excluded, and reported with
its line number, when:

- its `term`, `translation`, or `category` is empty;
- its headword or one of its forms repeats a headword or form of an earlier
  term, or another of its own, compared case-insensitively, but for another
  meaning of the same name: a record whose headword is an earlier term's,
  ignoring case, is that name's meaning when the categories differ,
  ignoring case, and the meanings of a name share their forms. A form of
  another name stays a repeat.

Example of a name with two meanings:

```csv
term,translation,forms,note,category
Maelstrom,Мальстрём,,Великая компания Лимсы Ломинсы,организация
maelstrom,водоворот,maelstroms,,существо
```

Excluded records do not make the file invalid; the remaining entries are used.

## Canonical form

When the desktop's knowledge editor saves the file, it writes the header
`term,translation,forms,note`, then `,case` only when an entry matches
case, then `,category`, one record per entry in the editor's order, LF line endings, quoting only where
required, forms joined with `; `, and `yes` or an empty field for `case`.
The editor shows excluded records and drops them only after the user
confirms; it also replaces a file that cannot be read at all only after
confirmation. The editor never writes an entry that would be excluded.

Example:

```csv
term,translation,forms,note,category
linkshell,линкшелл,linkshells,,игровая система
Scions of the Seventh Dawn,Потомки Седьмой Зари,the Scions; Scions,,организация
"Sage, elder",Старейшина,,Title in Sharlayan,титул
```

## Matching

Terms are matched in the text of a string, never in its macros: each tag
with its arguments counts as a space, so the sheet name in
`<sheet Aetheryte $n1 8>` is not the word *Aetheryte*, while text between
tags, such as the branches of a condition, is text. Each form of a term,
its headword included, matches that text case-insensitively, or with its
case when its entry says so, as a whole word when the form starts and ends
with a letter, digit, or underscore. A form also matches with its last
word in the English plural: with `s`, with `es` after `s`, `x`, `z`, `ch`,
or `sh`, and with `ies` for a `y` after a consonant (`gladiators`,
`ahrimans`, `fairies`); another plural, such as `dwarves`, is a form of its
own. Forms of one term that overlap, such as
`the Scions` and `Scions of the Seventh Dawn` in `the Scions of the Seventh
Dawn`, are one occurrence of the term. Where a longer term's match covers a
shorter term's, the words are the longer term's alone: `Scions of the
Seventh Dawn` is not also `Scions`. The meanings of one name match where
the name does, each as its record says. A string's term exceptions take
their terms out of matching for that string; an exception names a term by
its headword, every meaning of the name included, and one that names
another of its forms takes the term out too (see
[`../architecture/po-project.md`](../architecture/po-project.md#term-exceptions)).
Matching selects the terms a batch of machine translation needs, counts the
strings that use each term, and records the terms a translation was written
with (see
[`../architecture/po-project.md`](../architecture/po-project.md#terms-written-with)).
Terms are matched in sources only: whether a translation renders a term is
the model's and the person's judgment, never read from the translation's
words. Matching never changes a translation.

