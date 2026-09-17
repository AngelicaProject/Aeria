# Git workflow instructions

Canonical branch and commit conventions live in `docs/development/git.md`.

## Before changing repository history

- Inspect the current branch, working tree, and relevant remote state.
- Do not overwrite unrelated user changes.
- Do not amend, squash, rebase, force-push, or otherwise rewrite existing user commits unless the task requires it or the user explicitly asks.
- Never force-push `main`.

## Branches and commits

- Use a focused branch for non-trivial work.
- Keep each commit to one coherent review concern.
- Follow the commit message convention in `docs/development/git.md`.
- Do not mix opportunistic cleanup with the requested change.
- Prefer rebasing a feature branch onto current `main` over merging `main` into it when history needs updating.

## Push verification

A successful local commit is not a successful push.

After pushing:

1. Fetch or otherwise query the remote branch.
2. Verify the remote head SHA is the commit that was intended to be pushed.
3. If specific files or lockfiles were part of the task, verify they exist on the remote branch.
4. Do not report the push as complete until remote state has been observed.

If a pull request is involved, load `.ai/pull-requests.md` before creating or updating it.
