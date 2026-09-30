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
| `term` | yes | The term as written in the project's source language. |
| `translation` | yes | The translation to use. |
| `note` | no | Guidance for translators, such as usage, gender, or declension. |
| `forbidden` | no | Translations that must not be used, separated by `;`. |
| `settled` | no | `yes`, `true`, or `1`, ignoring case, when a person decided the term. Anything else, or an empty field, is not settled. |

A missing `term` or `translation` column, a repeated column, or any other
column name makes the whole file invalid. Aeria reports the problem and uses
no entries from the file; it never guesses at an unknown column.

## Records

Each following record is one entry. Surrounding whitespace in every field is
ignored, and records whose fields are all empty are skipped. A record is
excluded, and reported with its line number, when:

- its `term` or `translation` is empty;
- its term repeats an earlier term, compared case-insensitively.

Excluded records do not make the file invalid; the remaining entries are used.

## Canonical form

When the desktop's knowledge editor saves the file, it writes the header
`term,translation,note,forbidden,settled`, one record per entry in the
editor's order, LF line endings, quoting only where required, forbidden
variants joined with `; `, and `yes` or an empty field for `settled`. The
editor shows excluded records and drops them only after the user confirms;
it also replaces a file that cannot be read at all only after confirmation.
The editor never writes an entry that would be excluded.

Example:

```csv
term,translation,note,forbidden,settled
Aether,Эфир,,Этер,yes
"Sage, elder",Старейшина,Title in Sharlayan,,
```

## Matching

A term matches text case-insensitively, as a whole word when the term starts
and ends with a letter, digit, or underscore. Matching selects the terms a
batch of machine translation needs and drives the checks of a translation: a forbidden variant in
a translation rejects it, and a term whose translation does not seem to be
used, allowing for inflected endings, is reported as advice. Matching never
changes a translation.
