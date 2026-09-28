# Project Knowledge Format v1

Status: **implemented in `aeria-ai`**.

Project knowledge is what a project decided about style, terms, characters,
and story, and the lessons its agents learned. It has two layers, both
committed with the project and shared through Git. How Aeria uses them is
described in [`../architecture/localization-system.md`](../architecture/localization-system.md#project-knowledge).

## Layers

- **Human**: `aeria-guidance.md`, `aeria-glossary.csv`
  ([Glossary Format v1](./glossary-v1.md)), and `aeria-voices.md`
  ([Voice Profiles Format v1](./voices-v1.md)) at the project root. People
  write them. Agents never change them, so their entries are locked.
- **Agents**: the files of the `aeria-knowledge` directory at the project
  root. Aeria's agents write them without asking; people may edit them too.

Where both layers have an entry for the same glossary term (compared
ignoring case) or speaker label, the human entry is used and the agent entry
is ignored. Agents never write an entry for a term or label the human layer
has.

## Agent files

Every file is optional, UTF-8 with an optional leading BOM that is ignored,
at most 4 MiB, with CRLF or LF line endings. Aeria writes LF.

| File | Format | Entries |
| --- | --- | --- |
| `style.md` | Sections | One per text domain, keyed by the domain |
| `terms.csv` | [Glossary Format v1](./glossary-v1.md) | One per term |
| `characters.md` | [Voice Profiles Format v1](./voices-v1.md) | One per character |
| `story.md` | Sections | One per quest or cutscene sheet, keyed by the sheet name |
| `lessons.md` | Sections | One per lesson, keyed by a short identifier |

A file that cannot be read, or rows and profiles the referenced formats
exclude, are reported as project knowledge problems and left out; the rest
of the knowledge is used.

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
<!-- aeria: source=study; updated=2026-09-29 -->
Journal entries speak to the player character with «вы» and avoid words
that agree with the player character's gender.
```

The metadata line is `<!-- aeria:` followed by `name=value` pairs separated
by `;` and `-->`. Names and values are trimmed. Names Aeria does not use are
kept when it rewrites the file and have no effect.

### Domains

`style.md` keys are the text domains: `general`, `journal`, `objective`,
`system`, `dialogue`, `names`, `items`, `actions`, `interface`, and `lore`. A
section with another key is kept and not used.

### Lessons

A lesson's metadata may name:

- `status`: `trial` (the default: proposed from findings and in use until it
  is evaluated), `active` (kept after an evaluation or by a person), or
  `dropped` (did not help; kept so it is not proposed again and never used).
- `domain`: the domain the lesson applies to; without it the lesson applies
  to all text.
- `findings`, `job`, `effect`, and `evaluated`: its provenance and measured
  effect, for people.

## Writing

Aeria rewrites an agent file whole: it reads the current file, applies the
change, writes the result to `.<file>.partial` in the same directory, syncs
it, and renames it over the file. Writes within one Aeria process are
serialized, so parallel job workers never lose each other's entries.
Section files are written with a `# Style`, `# Story`, or `# Lessons` title
and a line for people, then the sections in order, each with its metadata
line when it has metadata.
