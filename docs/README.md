# Aeria documentation

Documentation in this directory is part of the project and is maintained with the code it describes.

## Product

- [`product/vision.md`](./product/vision.md) — what Aeria is for and the primary user journey.
- [`product/principles.md`](./product/principles.md) — product invariants that implementation must preserve.

## Architecture

Start with [`architecture/overview.md`](./architecture/overview.md), then read the subsystem document relevant to the change:

- source snapshots and Atlas integration
- structured strings and macros
- translation identity and workspace persistence
- deterministic rebase
- Git collaboration
- translation assistance
- search and indexing
- pack export

## Formats

Specifications under [`formats/`](./formats/) define persisted and exported contracts. Once a format is released, compatibility changes require an explicit migration path.

## Development

Development standards live under [`development/`](./development/):

- contribution workflow
- Rust and frontend conventions
- dependencies and toolchains
- testing and fixtures
- documentation maintenance
- releases and packaging

When a document and implementation disagree, resolve the discrepancy in the same change rather than allowing parallel sources of truth to persist.
