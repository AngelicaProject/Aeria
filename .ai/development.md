# Change workflow

Read the relevant project documentation before changing code. Use `docs/README.md` and the section indexes when the owning document is unclear.

## Before implementation

1. Inspect the current implementation, tests, and nearby conventions.
2. Identify the exact behavior, contract, or invariant affected by the task.
3. Define the smallest coherent scope that satisfies the request.
4. Note any correctness-sensitive boundary involved: source validation, string structure, identity, workspace persistence, rebase, merge, export, credentials, or recovery.

Do not start implementing a later subsystem simply because the current change makes it possible.

## During implementation

- Prefer explicit behavior and narrow ownership over speculative abstractions.
- Reuse established project boundaries instead of bypassing them for convenience.
- Do not hide malformed data, unsupported formats, ambiguous identity, or failed validation behind fallback behavior.
- Add regression coverage for behavior that could recur.
- Keep generated, cache, and local-only state out of canonical project data.
- Update the owning documentation when a contract or invariant changes.

Language-specific rules live in `docs/development/rust.md` and `docs/development/frontend.md`. Testing rules live in `docs/development/testing.md`.

## Before completion

1. Review the complete diff for accidental scope growth and unrelated edits.
2. Run the checks appropriate to the affected area.
3. Confirm documentation and indexes are still accurate.
4. Confirm no test, lint, validation, or compatibility check was weakened to make the change pass.
5. Report only checks and remote state that were actually observed.

For commits and pushes, continue with `.ai/git.md`. For a pull request, continue with `.ai/pull-requests.md`.
