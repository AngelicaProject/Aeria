# Git workflow

## Branch names

Use short lowercase names that describe one coherent change. Default prefixes:

- `feature/` — new product or implementation capability.
- `fix/` — bug or correctness fix.
- `docs/` — documentation and repository-guidance changes only.
- `refactor/` — behavior-preserving structural work.
- `chore/` — maintenance that does not fit the categories above.

Use hyphens inside the descriptive part. Avoid personal names, generated identifiers, and vague names such as `changes`, `work`, or `update`.

## Commit messages

Aeria commits use a Conventional Commit-style subject:

```text
<type>(<optional-scope>): <imperative summary>
```

Common types are `feat`, `fix`, `refactor`, `test`, `docs`, `build`, `ci`, and `chore`.

Examples:

```text
feat(hxs): add verified HXS v1 reader
fix(rebase): preserve ambiguous source matches

docs: organize contributor guidance
```

Keep the subject concise, imperative, and without a trailing period. Add a body when the reason, trade-off, compatibility constraint, or migration detail is not obvious from the diff. Explain why the change is necessary rather than narrating every edited file.

## Commit scope

Each commit should represent one coherent review and rollback concern. Tests that directly validate an implementation normally travel with that implementation. Separate unrelated documentation, tooling, migration, or refactoring work when it can be reviewed and reverted independently.

## Updating a branch

Prefer rebasing a feature branch onto current `main` over merging `main` into the feature branch. After history changes, rerun the checks relevant to the affected code.

Do not rewrite shared history or force-push `main`.

## Push completion

A push is complete only after the remote branch is observed at the intended commit. When a task depends on specific generated files, lockfiles, or documentation being present, verify them on the remote branch before reporting completion.
