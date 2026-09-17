# Aeria project guidelines

This file is the repository entry point for coding tools. Durable project rules live in `docs/`; task-specific execution guidance lives in `.ai/`. Do not duplicate canonical documentation here.

## Context loading

Load the instruction modules that match the task before planning, implementing, reviewing, committing, or opening a pull request:

- Any code change: `.ai/development.md`
- Git branches, commits, rebases, or pushes: `.ai/git.md`
- Pull requests: `.ai/pull-requests.md`
- Code review or review feedback: `.ai/code-review.md`
- CI or workflow failures: `.ai/ci.md`
- Documentation changes: `docs/AGENTS.md`

Then load the canonical project documentation for the affected area:

- Product behavior and invariants: `docs/product/README.md`
- Architecture and subsystem ownership: `docs/architecture/README.md`
- Source snapshots / HXS: `docs/architecture/source.md`
- Structured strings / SeString macros: `docs/architecture/strings.md`
- Translation identity: `docs/architecture/identity.md`
- Workspace persistence: `docs/architecture/workspace.md` and `docs/formats/workspace-v1.md`
- Source update / rebase: `docs/architecture/rebase.md`
- Git collaboration inside Aeria: `docs/architecture/git.md`
- Translation assistance: `docs/architecture/ai.md`
- Runtime pack export: `docs/architecture/export.md` and `docs/formats/pack-v1.md`
- Rust implementation: `docs/development/rust.md`
- Frontend implementation: `docs/development/frontend.md`
- Dependencies and toolchains: `docs/development/dependencies.md`
- Tests and fixtures: `docs/development/testing.md`
- Releases and packaging: `docs/development/releases.md`
- Contribution workflow: `docs/development/README.md`

Start from `docs/README.md` when the correct document is not obvious.

## Always-on rules

- Make the smallest coherent change that solves the requested problem.
- Do not expand a milestone into adjacent subsystems unless the extra work is required for correctness.
- Preserve the product invariants in `docs/product/principles.md` and the ownership boundaries in `docs/architecture/overview.md`.
- Treat HXS and released persisted formats as contracts. Do not invent consumer-only compatibility rules or silently reinterpret unknown data.
- Deterministic identity, rebase, migration, merge, and export behavior must not depend on AI judgment.
- Invalid or structurally unsafe translated strings must never be persisted as successful translations.
- Do not introduce first-party `unsafe` Rust without a narrowly scoped documented justification and review.
- Do not weaken tests, validation, or CI gates to make a change pass.
- Never claim a command, push, CI run, or remote state succeeded unless it was actually observed.
- When behavior, formats, architecture, or development requirements change, update the canonical documentation in the same change.

## Project notes

- Default branch: `main`.
- Windows is the first supported desktop target; Linux follows.
- Production Aeria is a Rust/Tauri desktop application with bundled web assets.
- Node.js and pnpm are development/build tools, not installed application runtimes.
