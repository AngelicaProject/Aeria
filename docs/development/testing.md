# Testing strategy

Tests protect contracts, not implementation trivia.

Prioritize deterministic fixtures and regression cases around the highest-risk boundaries: source verification, structured strings, workspace persistence, rebase, Git merge behavior, AI validation, and export.

## Test layers

- Unit tests for pure domain rules and parsers.
- Golden/round-trip tests for structured strings and serialization.
- Property/fuzz tests for parsers and invariants.
- Integration tests over synthetic HXS/workspace repositories.
- Desktop IPC tests for capability contracts.
- Frontend component/workflow tests for high-value user flows.
- Packaging smoke tests on supported release targets.

## Source update and collaboration safety

The source update, persistence, and Git layers share one safety contract:
no translation is removed, overwritten, or shown against source text it was
not made for. Changes to these layers keep the following suites passing and
extend them for new cases:

- `aeria-rebase` `tests/rebase.rs`: one test per planner outcome, combined
  patches, row-key edge cases, a bounded exhaustive transition model, and
  `randomized_patches_never_attach_a_translation_to_different_text`, which
  checks the core invariants over seeded random patches and requires every
  kind of outcome (including binding conflicts and reattachment) to occur.
- `aeria-workspace` `tests/update_chains.rs` and `tests/merged_sources.rs`:
  chains of game patches, and workspace state merged from branches on
  different game versions.
- `aeria-git` `merge.rs` unit tests, including an exhaustive three-way merge
  check, and `tests/collaboration.rs` for sync, conflicts, reconciliation
  commits, and per-unit history.

A new invariant check should be confirmed to fail against a deliberately
broken implementation before it is relied on.

Tests must not depend on a user's installed game, credentials, network availability, or private repository data.

`aeria-git` and desktop Git tests require a Git executable: `AERIA_GIT_PATH`
when set (Windows CI points it at the staged MinGit), otherwise `git` on
`PATH`. They isolate Git from user and system configuration
(`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_NOSYSTEM`) and use only temporary
repositories and local bare remotes.
