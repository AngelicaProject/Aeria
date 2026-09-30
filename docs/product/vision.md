# Product vision

## What Aeria is

Aeria is a desktop application for creating, maintaining, and reviewing FINAL FANTASY XIV translations.

Each project represents one target language. It may be maintained by one person or by hundreds of contributors. Aeria does not require its own collaboration service: projects can use ordinary Git repositories and existing hosting platforms.

## Why Aeria exists

FINAL FANTASY XIV has a very large text corpus, its strings contain game-specific structured constructs, and the source changes continuously as the game is updated. Translating a large patch by hand is expensive, while careless automation can corrupt macros or move translations to the wrong source entries.

Aeria is built around three problems:

1. **Game updates** — move translation work between game versions conservatively and deterministically.
2. **Structured strings** — edit, validate, and preview nested macros and runtime constructs without treating them as plain text.
3. **Large-scale translation** — translate whole sheets by machine in minutes while structural validation, the game's own names, the project's knowledge, and human review stay in control.

## Project model

- One project has exactly one target language.
- Additional official source languages may be enabled locally as optional context for an individual translator.
- The project repository stores the translatable strings of the game as gettext PO files, one per sheet, with their translations; the game remains the source of their structure.
- A project may be used by one person or by a community through branches and pull requests.

## Core user journey

A user should be able to:

1. Open or clone a translation project.
2. Point Aeria at the installed game, which is the source of every project.
3. Search the full source corpus and edit translations with game constructs represented safely and clearly.
4. Machine-translate new or untranslated content, with only structurally valid translations stored and every existing translation kept.
5. Review changes, stage and commit selected work, synchronize with others, and use either a simple or advanced Git workflow.
6. Detect a new game version, update the project deterministically, review translations whose source changed, and continue translating.
7. Export a versioned pack consumed by the in-game Harmonia plugin.

## Non-goals

Aeria is not a hosted translation-management service. It does not require Aeria accounts, a central organization server, repository hosting, or a centralized collaboration database.

Aeria is also not intended to model hundreds of target languages inside one project. Its design is optimized for one large translation at a time.
