# Contribution workflow

Use the [development index](./README.md) to locate the standards relevant to a change.

## Choose the scope

Keep each change focused on one coherent problem. Avoid opportunistic refactors unless they are necessary to make the requested change correct or maintainable.

Before implementation, identify the product invariant, architecture boundary, persisted format, or user behavior affected by the change. Read the corresponding documentation first.

## Implement

Follow the standards for the subsystem being changed. Prefer explicit, testable behavior over hidden conventions.

If a change affects a documented contract, update the canonical document with the implementation. Do not add duplicate rules to unrelated files.

## Verify

Run the smallest relevant test set while iterating, then run the broader checks appropriate to the affected area before review. See [`testing.md`](./testing.md) and [`ci.md`](./ci.md).

Changes to correctness-sensitive areas require regression coverage. These include structured string parsing, HXS compatibility, workspace persistence and migration, rebase, semantic merge, and export.

## Prepare the change for review

- Follow [`git.md`](./git.md) for branch and commit conventions.
- Follow [`pull-requests.md`](./pull-requests.md) for pull request title, description, and readiness.
- Keep the final branch diff limited to the intended problem.
- Update documentation and section indexes when contracts or guidance change.

## Review

Review follows [`code-review.md`](./code-review.md). A reviewer should be able to answer:

1. What problem does this solve?
2. What behavior or contract changes?
3. Why is this approach appropriate for the existing architecture?
4. How was it verified?
5. Does documentation need to change?

Prioritize correctness, data safety, compatibility, understandable ownership, and regression resistance over stylistic cleverness.
