# Documentation standards

Documentation is maintained alongside the code and is the source of truth for product behavior, architecture, persisted formats, and development standards.

## Structure and navigation

[`docs/README.md`](../README.md) is the top-level documentation index. Each top-level section (`product`, `architecture`, `development`, and `formats`) has its own `README.md` index.

When adding, moving, or deleting a document:

- update the owning section index;
- update `docs/README.md` when top-level task routing changes;
- keep relative links valid;
- avoid orphaned documents that are not reachable from an index.

## Keep one source of truth

Put a rule in the document that owns it and link to that document elsewhere. Avoid copying the same architectural or development rule into several files because duplicated guidance drifts.

Examples:

- product invariants belong in `docs/product/principles.md`;
- subsystem ownership belongs in `docs/architecture/`;
- persisted contracts belong in `docs/formats/`;
- implementation conventions belong in `docs/development/`.

Root files such as `README.md`, `CONTRIBUTING.md`, and `AGENTS.md` should provide orientation and route readers to canonical documentation rather than restating it. Files under `.ai/` contain task-specific execution guidance and should link back to durable rules here instead of becoming a second project specification.

## Update documentation with behavior

Update the relevant documentation in the same change when modifying:

- user-visible behavior;
- architecture boundaries;
- persisted or exported formats;
- compatibility guarantees;
- development, CI, or release requirements.

## Writing style

Write directly and describe current behavior or explicit intended contracts. Avoid promotional language, implementation history, speculative architecture, prompt transcripts, and process commentary that does not help a contributor understand the project.

Prefer concrete statements and examples where ambiguity would affect implementation. Mark proposals and unresolved decisions clearly instead of presenting them as implemented behavior.
