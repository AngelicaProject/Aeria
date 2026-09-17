# Code review instructions

Canonical review standards live in `docs/development/code-review.md`.

Review the change in this order:

1. **Scope and intent** — understand the requested outcome and check for unrelated scope.
2. **Contracts and architecture** — verify product invariants, subsystem ownership, persistence formats, and compatibility rules.
3. **Correctness and data safety** — look for regressions, silent fallback behavior, corruption risks, nondeterminism, unsafe writes, and missing error handling.
4. **Tests** — verify important behavior and failure modes have meaningful coverage, including independent conformance vectors when cross-implementation compatibility matters.
5. **Documentation** — ensure changed behavior or contracts are reflected in the owning documents and indexes.
6. **Maintainability** — naming, complexity, duplication, and smaller improvements.

## Severity

Classify feedback clearly:

- **Blocker** — must be fixed before merge.
- **Improvement** — meaningful but non-blocking.
- **Nitpick** — minor style or consistency issue.

Any unresolved blocker means the verdict is request changes. Nitpicks alone do not block approval.

## Review evidence

- Review the current pull request head, not a stale commit.
- Verify CI against that exact head when CI status affects the verdict.
- Prefer concrete file/behavior references over speculative concerns.
- If correctness depends on an external contract such as HXS, compare the implementation against the authoritative contract rather than against its own tests alone.
