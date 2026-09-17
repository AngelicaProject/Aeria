# Development documentation

This section defines how changes are implemented, validated, reviewed, and shipped.

## Change workflow

- [`contributing.md`](./contributing.md) — end-to-end contribution workflow and scope discipline.
- [`git.md`](./git.md) — branch names, commit messages, history, and push expectations.
- [`pull-requests.md`](./pull-requests.md) — pull request titles, descriptions, readiness, and review preparation.
- [`code-review.md`](./code-review.md) — review methodology, severity, and merge verdicts.
- [`ci.md`](./ci.md) — current CI gates and failure investigation.

## Implementation

- [`standards.md`](./standards.md) — project-wide reliability and implementation standards.
- [`rust.md`](./rust.md) — Rust conventions and crate-boundary guidance.
- [`frontend.md`](./frontend.md) — React/TypeScript and Tauri renderer boundaries.
- [`dependencies.md`](./dependencies.md) — dependency selection, upgrades, and lockfiles.

## Quality and maintenance

- [`testing.md`](./testing.md) — test strategy, fixtures, regression coverage, and fuzz/property testing.
- [`documentation.md`](./documentation.md) — documentation ownership, structure, and writing standards.
- [`releases.md`](./releases.md) — packaging, release channels, signing, and update expectations.

For product or architecture decisions, use the corresponding indexes under [`../product/`](../product/README.md) and [`../architecture/`](../architecture/README.md) instead of adding development-only rules here.
