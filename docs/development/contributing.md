# Contribution workflow

## Choose the scope

Keep each change focused on one coherent problem. Avoid opportunistic refactors unless they are necessary to make the requested change correct or maintainable.

Before implementation, identify the product invariant, architecture boundary, persisted format, or user behavior affected by the change. Read the corresponding documentation first.

## Implement

Follow the standards for the subsystem being changed. Prefer explicit, testable behavior over hidden conventions.

If a change affects a documented contract, update the canonical document with the implementation. Do not add duplicate rules to unrelated files.

## Verify

Run the smallest relevant test set while iterating, then run the broader checks appropriate to the affected area before review.

Changes to correctness-sensitive areas require regression coverage. These include structured string parsing, HXS compatibility, workspace persistence and migration, rebase, semantic merge, and export.

## Review

A pull request should make it possible to answer:

1. What problem does this solve?
2. What behavior or contract changes?
3. Why is this approach appropriate for the existing architecture?
4. How was it verified?
5. Does documentation need to change?

Review should prioritize correctness, data safety, compatibility, understandable ownership, and regression resistance over stylistic cleverness.
