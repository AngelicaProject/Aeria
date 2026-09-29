# Voice Profiles Format v1

Status: **implemented in `aeria-knowledge`**.

Voice Profiles Format v1 describes how characters speak in the target
language. It is the format of `aeria-knowledge/characters.md`, part of the
[project knowledge](./knowledge-v1.md). It is committed with the project,
reviewed and merged through Git like any other file, and meant to be edited
by hand, by agents, or in the desktop's knowledge editor. It is not part of
the Workspace Format.

How Aeria uses it is described in
[`../architecture/agents.md`](../architecture/agents.md#project-knowledge).

## Absence

A missing file means the project has no character voices.

## Encoding

- UTF-8 Markdown, with an optional leading BOM that is ignored.
- CRLF and LF line endings are both accepted.
- At most 8 MiB.

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

## Settled profiles

The first non-blank line of a profile's text may be a metadata line, as in
[section files](./knowledge-v1.md#sections). A profile with `settled=yes`
there is settled: a person decided it, and agents do not change it without
asking. The metadata line is not part of the profile's text.

## Saving

The desktop's knowledge editor saves the text as written and refuses text with
ignored profiles.
