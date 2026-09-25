# Continuous integration

GitHub Actions configuration lives in `.github/workflows/ci.yml`. This document summarizes the expected gates; the workflow file remains authoritative for exact runner configuration.

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

The Rust job stages the pinned Harmonia Atlas v0.4.0 Linux sidecar after
verifying the checksum recorded in tools/atlas/version.json, because Tauri
validates configured external binaries during its build. It then runs the real
staged binary's version/help/package-usage smoke test. The Windows Atlas job
stages and hash-verifies the pinned Windows sidecar and runs the equivalent
real-binary smoke test before running the portable aeria-atlas child-process
fixture suite. Neither smoke test requires a game installation.

Workspace persistence also runs its focused locked test suite on
`windows-latest` because Windows is the first production desktop target:

```text
cargo test -p aeria-workspace --locked
```

The Windows Atlas job also runs the local project-registry persistence and
recovery suite because `aeria-projects` has Windows-specific publication and
restore behavior:

```text
cargo test -p aeria-projects --all-targets --locked
```

The Windows Atlas job then reruns the path-handling suites (`aeria-hxs`,
`aeria-hsp`, `aeria-workspace`, `aeria-search`, `aeria-projects`,
`aeria-atlas`, `aeria-git` with the bundled MinGit, and the desktop library)
with `TMP` and `TEMP` pointing at a Cyrillic folder with a space, so every
file those tests create lives under such a path. See
[`testing.md`](./testing.md#paths).

When CI gains or removes a project-wide quality gate, update this document with the workflow change.

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
