# Continuous integration

GitHub Actions configuration lives in `.github/workflows/ci.yml`. This document summarizes the expected gates; the workflow file remains authoritative for exact runner configuration.

Aeria is released for Windows, so CI checks the application on Windows. The
desktop application is not checked on Linux until it is released there; Linux
checks only `aeria-guard`, which translation repositories build and run on
Linux through the Aeria Guard action (`guard/action.yml`).

CI runs for every pull request and for every push to `main`. A new push to a
pull request cancels its running checks; runs on `main` are never cancelled,
because a stable release waits for CI on its exact commit. The workflow has
read-only repository permissions.

## Jobs

| Job | Runner | Guarantees |
| --- | --- | --- |
| `frontend` | `windows-latest` | The renderer and release tooling tests pass, TypeScript type-checks, and the renderer builds. |
| `rust` | `windows-latest` | The workspace is formatted, has no Clippy warnings, and passes all its tests on Windows under a Cyrillic path with the bundled MinGit; line coverage stays above the floor. |
| `guard-linux` | `ubuntu-22.04` | `aeria-guard` passes its tests on Linux, where the Aeria Guard action builds and runs it. |
| `supply-chain` | `ubuntu-latest` | Dependencies have no known vulnerability, no yanked crate, only allowed licenses, and come from crates.io; workflows pass `actionlint` and `zizmor`. |
| `ci` | `ubuntu-latest` | Every job above passed. |

The `main` ruleset requires only `ci`, so a job can be added, renamed, or
removed without changing the ruleset. A new job must be added to the `needs`
of `ci`.

No job requires a game installation: source tests run over synthetic SqPack
fixtures.

### Frontend

```text
pnpm install --frozen-lockfile
pnpm --filter @aeria/desktop test:coverage
node --test tools/release/version.test.mjs
pnpm typecheck
pnpm build
```

`test:coverage` runs the same tests as `pnpm --filter @aeria/desktop test`
and measures the coverage of `src/` with the Node.js test runner.

### Rust

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo llvm-cov --workspace --all-targets --locked --no-report
cargo test --workspace --locked --doc
cargo llvm-cov report --fail-under-lines 75
```

The tests run with `TMP` and `TEMP` pointing at a Cyrillic folder with a
space, so every file they create lives under such a path, and with
`AERIA_GIT_PATH` pointing at the pinned MinGit the release bundles. See
[`testing.md`](./testing.md#paths). `cargo llvm-cov` runs the tests with
coverage instrumentation; on stable Rust it does not run doctests, so they run
separately.

## Coverage

Each run shows the coverage of the frontend and of Rust in its job summary,
and keeps the `lcov.info` files and the Rust HTML report as artifacts. The
tools are the Node.js test runner and `cargo-llvm-cov`, pinned in the
workflow; no external coverage service receives the reports.

Rust line coverage has a floor below the current coverage, so a change that
drops coverage noticeably fails CI. Raise the floor in the workflow as
coverage grows; do not lower it to pass a change. Frontend coverage is
reported, not gated: it counts only the modules the tests load.

Locally, after `cargo install cargo-llvm-cov` and
`rustup component add llvm-tools-preview`:

```text
cargo llvm-cov --workspace --html
pnpm --filter @aeria/desktop test:coverage
```

## Supply chain

```text
cargo deny --locked check
pnpm audit --prod
actionlint
zizmor --format github .github/workflows
```

`cargo deny` applies the policy in `deny.toml`, described in
[`dependencies.md`](./dependencies.md#dependency-checks). A new RustSec
advisory can fail a pull request that did not change dependencies; update the
affected crate, or, when no fix exists and the advisory does not apply to
Aeria, ignore its ID in `deny.toml` with the reason.

`actionlint` checks the workflows and their shell scripts with `shellcheck`.
`zizmor` checks them for security problems. A finding is fixed, not ignored;
an `# zizmor: ignore[...]` comment is allowed only beside an explanation of
why the finding does not apply, as on the `workflow_run` triggers of the
release and the website.

## Workflow rules

- Every action is pinned by commit SHA with its version in a comment.
  Dependabot updates the pins weekly.
- Workflows have read-only permissions by default; a job that writes
  declares exactly the permissions it needs.
- Checkouts do not keep the token (`persist-credentials: false`).
- Expressions such as `${{ steps.x.outputs.y }}` go into `run:` scripts
  through `env`, never directly into the script text.
- A tool downloaded by a workflow is pinned by version and verified by
  checksum, or installed through a pinned action.

When CI gains or removes a project-wide quality gate, update this document with the workflow change.

## Releases

`.github/workflows/release.yml` publishes stable releases from `v*` tags and
the rolling `nightly` pre-release after every successful CI run of a push to
`main`. It builds on `windows-latest` with the same pinned MinGit
and toolchain, and signs installers with the updater key secrets.
See [`releases.md`](./releases.md#release-workflow).

`pnpm test` and CI also run `tools/release/version.test.mjs`, which covers the
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
