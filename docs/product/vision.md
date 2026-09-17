# Product vision

## What Aeria is

Aeria is a desktop application for creating, maintaining, and reviewing FINAL FANTASY XIV translations.

Each project represents one target language. It may be maintained by one person or by hundreds of contributors. Aeria does not require its own collaboration service: projects can use ordinary Git repositories and existing hosting platforms.

## Why Aeria exists

FINAL FANTASY XIV has a very large text corpus, its strings contain game-specific structured constructs, and the source changes continuously as the game is updated. Translating a large patch by hand is expensive, while careless automation can corrupt macros or move translations to the wrong source entries.

Aeria is built around three problems:

1. **Game updates** — move translation work between source snapshots conservatively and deterministically.
2. **Structured strings** — edit, validate, and preview nested macros and runtime constructs without treating them as plain text.
3. **Large-scale translation** — use AI to produce useful drafts quickly while keeping structural validation and human review in control.

## Project model

- One project has exactly one target language.
- Additional official source languages may be enabled locally as optional context for an individual translator.
- The project repository stores translated/project state, not the complete source corpus.
- A project may be used by one person or by a community through branches and pull requests.
- The repository may specify a collaboration policy such as direct-push or pull-request workflow.

## Core user journey

A user should be able to:

1. Open or clone a translation project.
2. Generate a source snapshot from an installed game, or import an existing HXS snapshot.
3. Search the full source corpus and edit translations with game constructs represented safely and clearly.
4. Translate selected new or untranslated content with AI and receive only structurally valid drafts.
5. Review changes, stage and commit selected work, synchronize with others, and use either a simple or advanced Git workflow.
6. Detect a new game version, generate a new snapshot, migrate the project deterministically, resolve ambiguous cases, and continue translating.
7. Export a versioned pack consumed by the in-game Harmonia plugin.

## Non-goals

Aeria is not a hosted translation-management service. It does not require Aeria accounts, a central organization server, repository hosting, or a centralized collaboration database.

Aeria is also not intended to model hundreds of target languages inside one project. Its design is optimized for one large translation at a time.
