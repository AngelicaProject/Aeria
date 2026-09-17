# Aeria agent instructions

The `.ai/` directory contains small task-specific instruction modules referenced by the repository root `AGENTS.md`.

These files are procedural guidance for coding tools. They are not the source of truth for product behavior, architecture, persisted formats, or contributor policy. Durable project rules belong in `docs/` and are linked from the modules here.

## Modules

- [`development.md`](./development.md) — planning, implementation scope, and completion discipline.
- [`git.md`](./git.md) — branches, commits, rebases, pushes, and remote verification.
- [`pull-requests.md`](./pull-requests.md) — pull request titles, descriptions, readiness, and updates.
- [`code-review.md`](./code-review.md) — review order, severity, and verdict rules.
- [`ci.md`](./ci.md) — local checks, GitHub Actions failures, and CI verification.

Documentation work also loads [`docs/AGENTS.md`](../docs/AGENTS.md).

## Source-of-truth rule

When a module needs a durable project rule, put that rule in the appropriate document under `docs/` and link to it here. Do not maintain parallel copies that can drift.

Agent-specific execution rules may stay in `.ai/` when they describe how a tool should operate rather than how Aeria itself works.
