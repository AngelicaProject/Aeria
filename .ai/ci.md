# CI instructions

Canonical CI expectations live in `docs/development/ci.md` and `.github/workflows/ci.yml`.

## Local validation

Run the smallest useful checks while iterating, then the broader checks required for the affected area before requesting review.

Do not modify CI configuration, suppress warnings, relax assertions, or remove tests merely to obtain a green pipeline.

## Investigating failures

1. Confirm the failing run belongs to the current branch and exact head SHA.
2. Identify the failing job and step before changing code.
3. Read the relevant logs and reproduce locally when practical.
4. Fix the underlying issue rather than adding retries, exclusions, or weaker gates without a demonstrated infrastructure reason.
5. Re-run the relevant local checks before pushing.

## Reporting status

- Distinguish local validation from GitHub Actions results.
- Do not say CI is green while jobs are pending, skipped unexpectedly, or belong to an older commit.
- When a workflow is rerun, report the result of the latest attempt for the current head.
