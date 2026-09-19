# Aeria documentation

Documentation is maintained with the code it describes and is the source of truth for product behavior, architecture, persisted formats, and development standards.

## Start here

- [Product](./product/README.md) — what Aeria does and the invariants the product must preserve.
- [Architecture](./architecture/README.md) — subsystem boundaries, ownership, and major data flows.
- [Development](./development/README.md) — contribution workflow, implementation standards, testing, CI, and releases.
- [Formats](./formats/README.md) — versioned persisted and exported contracts.

## Find documentation by task

| Task | Read first |
| --- | --- |
| Understand product scope or user behavior | [`product/README.md`](./product/README.md) |
| Change a subsystem boundary or data flow | [`architecture/README.md`](./architecture/README.md) |
| Work with HXS source snapshots | [`architecture/source.md`](./architecture/source.md) |
| Open or initialize an Aeria project session | [`architecture/project-session.md`](./architecture/project-session.md) |
| Work with SeString / macro structure | [`architecture/strings.md`](./architecture/strings.md) |
| Change translation identity or rebase | [`architecture/identity.md`](./architecture/identity.md), [`architecture/rebase.md`](./architecture/rebase.md) |
| Change workspace persistence | [`architecture/workspace.md`](./architecture/workspace.md), [`formats/workspace-v1.md`](./formats/workspace-v1.md) |
| Change Git collaboration behavior in Aeria | [`architecture/git.md`](./architecture/git.md) |
| Change translation assistance | [`architecture/ai.md`](./architecture/ai.md) |
| Change export behavior | [`architecture/export.md`](./architecture/export.md), [`formats/pack-v1.md`](./formats/pack-v1.md) |
| Implement Rust or frontend code | [`development/README.md`](./development/README.md) |
| Add or upgrade a dependency | [`development/dependencies.md`](./development/dependencies.md) |
| Add or update tests/fixtures | [`development/testing.md`](./development/testing.md) |
| Prepare a branch, commit, or pull request | [`development/git.md`](./development/git.md), [`development/pull-requests.md`](./development/pull-requests.md) |
| Review a change | [`development/code-review.md`](./development/code-review.md) |
| Investigate CI | [`development/ci.md`](./development/ci.md) |
| Write or reorganize documentation | [`development/documentation.md`](./development/documentation.md) |

## Source-of-truth rule

Put each durable rule in the document that owns it and link to that document elsewhere. When documentation and implementation disagree, resolve the discrepancy in the same change rather than allowing parallel sources of truth to persist.
