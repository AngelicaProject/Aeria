# Aeria project guidelines

Repository-wide standards live in `docs/`. Treat those documents as the source of truth rather than duplicating their contents here.

## Context loading

Before planning, implementing, or reviewing a change, read the documents relevant to the task:

- Product behavior or scope: `docs/product/vision.md` and `docs/product/principles.md`
- Architecture or subsystem boundaries: `docs/architecture/overview.md` and the relevant file in `docs/architecture/`
- Rust changes: `docs/development/rust.md`
- Frontend changes: `docs/development/frontend.md`
- Dependencies or toolchain changes: `docs/development/dependencies.md`
- Tests and fixtures: `docs/development/testing.md`
- Releases and packaging: `docs/development/releases.md`
- Persisted or exported formats: the relevant specification in `docs/formats/`
- Contributions and change workflow: `CONTRIBUTING.md`

If implementation behavior, a persisted format, or an architectural invariant changes, update the canonical documentation in the same change.

## Always-on rules

- Make the smallest coherent change that solves the problem.
- Preserve the product invariants documented in `docs/product/principles.md`.
- Keep subsystem ownership and boundaries documented in `docs/architecture/overview.md`.
- Do not introduce hidden compatibility behavior or silently guess ambiguous translation identity.
- Do not write invalid or structurally unsafe translations as successful results.
- Do not introduce `unsafe` first-party Rust without a narrowly scoped, documented justification and review.
- Do not add a long-lived runtime process or application-level transport boundary without documenting the architectural change first.
- Verify current stable upstream versions before introducing or upgrading substantial dependencies. Do not choose an old version from memory.
- Add or update regression coverage for parser, workspace migration, rebase, merge, and export behavior when those areas change.

## Project notes

- Default branch: `main`.
- Windows is the first supported desktop target; Linux follows.
- Production Aeria is a Rust/Tauri desktop application with bundled web assets.
- Node.js and pnpm are development/build tools, not installed application runtimes.
