# Voice Profiles Format v1

Status: **implemented in `aeria-ai`**.

Voice Profiles Format v1 describes how characters speak in the target
language. It is a single optional file, `aeria-voices.md`, in the project
root next to `.aeria/`. It is committed with the project, reviewed and merged
through Git like any other file, and meant to be edited by hand. It is not
part of the Workspace Format.

How Aeria uses it is described in
[`../architecture/ai.md`](../architecture/ai.md#voice-profiles).

## Absence

A missing file means the project has no voice profiles. Aeria creates the file
only when the user approves a voice profile change or saves the voice profile
editor.

## Encoding

- UTF-8 Markdown, with an optional leading BOM that is ignored.
- CRLF and LF line endings are both accepted.
- At most 256 KiB.

## Profiles

A line that starts with `## ` (a level-2 heading) starts a profile. The
profile's text is every following line up to the next such line or the end of
the file, without surrounding blank lines. Other headings, lists, and code
blocks are part of the text; a line starting with `## ` always starts a
profile, even inside a code block.

The heading names the character by one or more speaker labels, separated by
commas:

```markdown
## URIANGER

Archaic, formal speech. Addresses everyone formally; …

## ALPHINAUD, ALPHINAUD_YOUNG

Polite and bookish; …
```

A speaker label is the label the game's dialogue keys use for a speaker (see
[`../architecture/source.md`](../architecture/source.md#dialogue)): ASCII
letters, digits, and `_`, with at least one letter. Labels are compared
ignoring case and written in upper case. At most 20 labels name one profile.

Text before the first profile is for people and is not used.

## Ignored profiles

A profile is ignored, and reported with the line of its heading, when:

- its heading contains something that is not a speaker label, such as
  `## Urianger Augurelle`;
- it names more than 20 labels;
- its text is empty.

A label that an earlier profile already names is ignored in the later
profile and reported; the later profile keeps its other labels. Ignored
profiles do not make the file invalid; the remaining profiles are used.

## Canonical form

When Aeria writes the file after an approved change, it keeps the text before
the first profile and each profile's text, trimmed, in the existing order with
new profiles appended. Each profile is written as `## ` and its labels in upper
case joined with `, `, a blank line, its text, and a blank line; the file ends
with one LF. A profile whose labels were all removed is dropped. Aeria refuses
to apply an Angelica change to a file that has ignored profiles, because a
rewrite would change what they mean, and the voice profile editor refuses to
save text with ignored profiles.
