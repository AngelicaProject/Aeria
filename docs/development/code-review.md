# Code review

Review exists to protect correctness, data safety, compatibility, and maintainable subsystem boundaries. Style feedback is secondary to those concerns.

## Review order

Review changes in this order:

1. **Scope and intent** — confirm the change solves the requested problem without unrelated work.
2. **Product and architecture** — check product invariants, subsystem ownership, public interfaces, and persisted-format boundaries.
3. **Correctness and data safety** — check failure handling, determinism, recovery, malformed input, destructive operations, and silent fallback behavior.
4. **Compatibility** — compare implementations against authoritative external or versioned contracts where applicable.
5. **Tests** — confirm important success and failure modes are covered and that tests are independent enough to detect contract drift.
6. **Documentation** — confirm changed contracts and behavior are documented in the owning files.
7. **Maintainability** — consider naming, complexity, duplication, and smaller improvements.

## Feedback severity

- **Blocker** — correctness, data loss/corruption, security, contract incompatibility, missing required coverage, architecture violation, or another issue that must be resolved before merge.
- **Improvement** — meaningful change that would improve the implementation but does not make the current change unsafe or incorrect.
- **Nitpick** — minor style, wording, or local consistency issue.

Any unresolved blocker means request changes. Improvements and nitpicks can be recorded without blocking merge.

## Review evidence

Review the current head commit and its complete diff. When CI matters to the verdict, verify the successful run belongs to that exact head.

For compatibility boundaries such as HXS, workspace formats, or exported packs, do not accept tests that only compare one implementation against another copy of the same logic. Prefer fixed golden vectors or independent producer/consumer fixtures when practical.
