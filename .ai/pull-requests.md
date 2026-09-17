# Pull request instructions

Canonical pull request standards live in `docs/development/pull-requests.md`. Read them before creating or materially updating a pull request.

## Before opening a pull request

- Review the complete diff against the target branch, not only the last commit.
- Confirm the branch contains only the intended scope.
- Run the required local checks and record the exact commands that actually ran.
- Update documentation required by the change before requesting review.

## Title

Write a human-readable outcome-oriented title. Pull request titles are not commit messages: do not prepend Conventional Commit prefixes such as `feat:`, `fix:`, or `chore:`.

The title should let a reviewer understand the result without opening the diff.

## Description

Use the structure in `docs/development/pull-requests.md` and describe the actual final diff.

Do not:

- use vague descriptions such as "implements milestone" or "various fixes";
- paste an implementation diary or prompt transcript;
- claim tests or CI passed if they were not observed;
- put `Do not merge`, `WIP`, or similar readiness warnings in the description.

If the pull request is not ready to merge, make it a draft.

## After pushing updates

- Re-read the pull request title and description after significant scope changes and update them if needed.
- Verify the pull request head SHA matches the pushed branch.
- Check CI for that exact head SHA before reporting it as green.
- Resolve review findings in code, tests, or documentation rather than merely explaining them away when a fix is appropriate.
