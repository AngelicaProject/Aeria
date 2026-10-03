# Continuous integration

GitHub Actions configuration lives in `.github/workflows/ci.yml`. This document summarizes the expected gates; the workflow file remains authoritative for exact runner configuration.

CI runs for every pull request and for every push to `main`. A new push to a
pull request cancels its running checks; runs on `main` are never cancelled,
because a stable release waits for CI on its exact commit. The workflow has
read-only repository permissions.

## Current gates

Frontend CI runs:

```text
pnpm install --frozen-lockfile
pnpm test
pnpm typecheck
pnpm build
```

Rust CI runs:

```text
cargo fmt --check
cargo build --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

No Rust job requires a game installation: source tests run over synthetic
SqPack fixtures.

The project's PO files also run their focused locked test suite on
`windows-latest` because Windows is the first production desktop target:

```text
cargo test -p aeria-po --locked
```

The Windows desktop job stages the pinned MinGit runtime and runs the local project-registry persistence and
recovery suite because `aeria-projects` has Windows-specific publication and
restore behavior:

```text
cargo test -p aeria-projects --all-targets --locked
```

The Windows desktop job then reruns the path-handling suites (`aeria-sqpack`,
`aeria-source`, `aeria-po`, `aeria-search`, `aeria-projects`,
`aeria-git` with the bundled MinGit, and the desktop library)
with `TMP` and `TEMP` pointing at a Cyrillic folder with a space, so every
file those tests create lives under such a path. See
[`testing.md`](./testing.md#paths).

When CI gains or removes a project-wide quality gate, update this document with the workflow change.

## Releases

`.github/workflows/release.yml` publishes stable releases from `v*` tags and
the rolling `nightly` pre-release after every successful CI run of a push to
`main`. It builds on `windows-latest` with the same pinned MinGit
and toolchain, and signs installers with the updater key secrets.
See [`releases.md`](./releases.md#release-workflow).

`pnpm test` also runs `tools/release/version.test.mjs`, which covers the
nightly version rule and the updater feed format.

## Website

`.github/workflows/pages.yml` deploys the static landing page in `site/` to
GitHub Pages after pushes to `main` that change it and after every successful
release, so its download links name the current builds. See
[`website.md`](./website.md).

## Local validation

During implementation, run focused checks for fast feedback. Before requesting review, run the broader checks required by the affected area. Use locked/frozen dependency resolution where CI does.

A local pass and a CI pass are different evidence. Report them separately.

## Failure handling

When CI fails:

1. confirm the run belongs to the current pull request head;
2. identify the specific failing job and step;
3. inspect the logs before changing code;
4. reproduce locally when practical;
5. fix the root cause rather than weakening the gate.

Do not add retries, exclusions, warning suppression, or looser assertions unless there is evidence the failure is infrastructure-related and the mitigation is appropriate for the project.
