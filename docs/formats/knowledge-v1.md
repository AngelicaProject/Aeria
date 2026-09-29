# Project Knowledge Format v1

Status: **implemented in `aeria-knowledge`**.

Project knowledge is what a project decided about style, terms, characters,
and story, and the lessons it learned. It is the `aeria-knowledge` directory
at the project root, next to `.aeria/`. It is committed with the project and
shared through Git like any other file, and it is not part of the Workspace
Format. People and agents edit its files directly. How Aeria uses it is
described in [`../architecture/agents.md`](../architecture/agents.md#project-knowledge).

## Files

Every file is optional, UTF-8 with an optional leading BOM that is ignored,
at most 8 MiB, with CRLF or LF line endings. Aeria writes LF.

| File | Format | Entries |
| --- | --- | --- |
| `style.md` | Sections | One per text domain, keyed by the domain |
| `terms.csv` | [Glossary Format v1](./glossary-v1.md) | One per term |
| `characters.md` | [Voice Profiles Format v1](./voices-v1.md) | One per character |
| `story.md` | Sections | One per quest or cutscene sheet, keyed by the sheet name |
| `lessons.md` | Sections | One per lesson, keyed by a short identifier |

A file that cannot be read is reported and left out. Rows and profiles the
referenced formats exclude, `style.md` sections with an unknown domain, and
lessons with an unknown domain are reported with their line; the rest of the
knowledge is used.

## Settled entries

An entry a person decided is **settled**: agents follow it and do not change
it without asking a person. A term is settled by its `settled` column (see
[Glossary Format v1](./glossary-v1.md)); a section or a character profile by
`settled=yes` in its metadata line. The desktop's knowledge editor marks a
term settled when a person edits it.

## Sections

A section file is Markdown. A line that starts with `## ` starts a section;
the rest of the line, trimmed, is its key. Its text is every following line
up to the next such line or the end of the file, trimmed. Text before the
first section is for people and is ignored. A section with an empty key or
text is ignored, and a later section with the same key, compared ignoring
case, replaces an earlier one.

The first non-blank line of a section may be a metadata line:

```markdown
## journal
<!-- aeria: settled=yes -->
Journal entries speak to the player character with «вы» and avoid words
that agree with the player character's gender.
```

The metadata line is `<!-- aeria:` followed by `name=value` pairs separated
by `;` and `-->`. Names and values are trimmed. `settled` is settled when its
value is `yes`, `true`, or `1`, ignoring case. Other names are for people and
have no effect.

### Domains

`style.md` keys are the text domains: `general`, `journal`, `objective`,
`system`, `dialogue`, `names`, `items`, `actions`, `interface`, and `lore`.
`general` holds the conventions for all text. A section with another key is
reported and not used.

### Lessons

A lesson's metadata may name `domain`, the domain the lesson applies to;
without it, or with `general`, the lesson applies to all text. A lesson that
no longer helps is deleted.
