# Glossary Format v1

Status: **implemented in `aeria-ai`**.

Glossary Format v1 is the project-shared glossary. It is a single optional
file, `aeria-glossary.csv`, in the project root next to `.aeria/`. It is
committed with the project, reviewed and merged through Git like any other
file, and meant to be edited by hand or in a spreadsheet. It is not part of
the Workspace Format.

How Aeria uses it is described in
[`../architecture/ai.md`](../architecture/ai.md#guidance-and-glossary).

## Absence

A missing file means the project has no glossary. Aeria creates the file only
when the user approves a glossary change or saves the glossary editor.

## Encoding

- UTF-8, with an optional leading BOM that is ignored.
- RFC 4180 CSV: comma-separated fields, fields quoted with `"` when they
  contain a comma, quote, or line break, and `""` for a quote inside a quoted
  field. CRLF and LF line endings are both accepted.
- At most 2 MiB and 20,000 entries.

## Header

The first record names the columns, in any order:

| Column | Required | Meaning |
| --- | --- | --- |
| `term` | yes | The term as written in the project's source language. |
| `translation` | yes | The translation to use. |
| `note` | no | Guidance for translators, such as usage or context. |
| `forbidden` | no | Translations that must not be used, separated by `;`. |

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

When Aeria writes the file after an approved change or from the glossary
editor, it writes the header
`term,translation,note,forbidden`, one record per entry in the existing order
with new entries appended, LF line endings, quoting only where required, and
forbidden variants joined with `; `. Aeria refuses to apply an Angelica
change to a file that has excluded records, because a rewrite would drop
them. The glossary editor shows excluded records and drops them only after
the user confirms; it also replaces a file that cannot be read at all only
after confirmation. The editor never writes an entry that would be
excluded.

Example:

```csv
term,translation,note,forbidden
Aether,Эфир,,Этер
"Sage, elder",Старейшина,Title in Sharlayan,
```

## Matching

A term matches text case-insensitively, as a whole word when the term starts
and ends with a letter, digit, or underscore. Matching is used to show
entries with the strings that contain them and for advisory checks; it never
changes a translation.
