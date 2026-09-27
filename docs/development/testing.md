# Testing strategy

Tests protect contracts, not implementation trivia.

Prioritize deterministic fixtures and regression cases around the highest-risk boundaries: source verification, structured strings, workspace persistence, rebase, Git merge behavior, AI validation, and export.

## Test layers

- Unit tests for pure domain rules and parsers.
- Golden/round-trip tests for structured strings and serialization.
- Property/fuzz tests for parsers and invariants.
- Integration tests over synthetic SqPack game folders and workspace repositories.
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
- `aeria-workspace` `tests/session.rs`: game updates that are previewed,
  applied, and repeated after interruption, chains of patches, and units
  merged from branches on an older game version.
- `aeria-git` `merge.rs` unit tests, including an exhaustive three-way merge
  check, and `tests/collaboration.rs` for sync, conflicts, reconciliation
  commits, and per-unit history.

A new invariant check should be confirmed to fail against a deliberately
broken implementation before it is relied on.

Tests must not depend on a user's installed game, credentials, network availability, or private repository data.

## Game strings

`aeria-se` pins its macro text with golden vectors of bytes and text for
every form and every catalog entry
(`crates/aeria-se/tests/fixtures/macro_text.golden.txt`). A new catalog
entry needs a vector; the test fails otherwise.

The well-formed string rule of the pack format has its own vectors
(`crates/aeria-se/tests/fixtures/well_formed.vectors.txt`): every golden
vector must appear there as accepted, and Harmonia runs a copy of the file
against its pack reader. Change both copies in the same change.

Changes to the byte model, the printer, or the parser are also checked
against a real game with the ignored test
`crates/aeria-source/tests/game_corpus.rs`: every distinct string in all four
source languages must print as macro text that encodes back to its bytes, and
every string without raw bytes must be well-formed. The test prints the macro
codes the catalog does not name, so run it after a game patch to see which
new macros need a catalog entry. Run it manually with `AERIA_GAME_PATH` set;
it is not part of CI, which has no game installation.

## Harmonia pack interop

`aeria-export` compares its output with the committed fixtures
`crates/aeria-export/tests/fixtures/harmonia-interop.hpk` and
`harmonia-interop-fonts.hpk` (with a `FONTS` section), and the Harmonia
repository reads copies of the same files in its tests. Regenerate it with
`AERIA_UPDATE_FIXTURES=1 cargo test -p aeria-export` only for an intended
format change, and update Harmonia's copies in the same change.

`aeria-fonts` tests render the bundled recommended fonts for every supported
game font size; they need no game data because the native metrics are a table
in the crate.

`aeria-git` and desktop Git tests require a Git executable: `AERIA_GIT_PATH`
when set (Windows CI points it at the staged MinGit), otherwise `git` on
`PATH`. They isolate Git from user and system configuration
(`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_NOSYSTEM`) and use only temporary
repositories and local bare remotes.

## Paths

Every operation that takes or produces a path must work with non-ASCII
characters and spaces, as under a Russian Windows user profile
(`C:\Users\Анна Иванова\...`) or a game in `Program Files (x86)`:

- Pass paths to the filesystem and to child processes (Git) as
  `Path`/`OsStr` arguments, never through a shell or a lossy conversion.
- Code that treats a path as a string (parsing, prefix stripping, joining,
  display, URL validation) needs a unit test with a Cyrillic case and a
  space, in Rust and in the renderer.
- Tests create their files under the system temporary folder, so running a
  suite with `TMP`/`TEMP` (or `TMPDIR`) set to a Cyrillic folder with a
  space exercises every filesystem path it touches. Windows CI does this
  (see [`ci.md`](./ci.md)); run it locally the same way after changing path
  handling.
- Keep that temporary folder short and outside any Git repository. SQLite
  on Windows rejects paths longer than 260 characters, and Git tests create
  repositories that must not be nested in another one.
