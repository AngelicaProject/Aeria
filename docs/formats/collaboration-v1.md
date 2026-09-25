# Collaboration Settings v1

Status: **implemented in `aeria-git`**.

Collaboration Settings v1 records the project's main branch. Translations
reach it only through pull requests; there is no other policy. It is a single
optional file, `aeria-collaboration.json`, in the project root next to
`.aeria/`, committed with the project by a checkpoint. It is not part of Workspace Format v1, and the Workspace
Format v1 reader ignores it like any other project-root file.

The behavior is described in
[`../architecture/git.md`](../architecture/git.md#branches-and-contributions).

## Absence

A missing file means the main branch is detected (see
[`../architecture/git.md`](../architecture/git.md#branches-and-contributions)).
Aeria does not create the file until a main branch is set explicitly.

## Canonical form

UTF-8 JSON with two-space indentation, the field order below, and one LF
after the closing `}`:

```json
{
  "formatVersion": 1,
  "policy": "pull-request",
  "mainBranch": "main"
}
```

| Field | Representation | Required |
| --- | --- | --- |
| `formatVersion` | the JSON integer `1` | yes |
| `policy` | `"pull-request"` | yes |
| `mainBranch` | a valid Git branch name, or JSON `null` | no |

`mainBranch` names the branch contributions are reviewed into; `null` or
absent means it is detected. The canonical writer always emits the field,
using `null` when no main branch is set.

## Reader contract

- A leading UTF-8 BOM is accepted and ignored.
- The document must be one JSON object. Unknown fields are rejected.
- A missing or non-integer `formatVersion` is invalid. A `formatVersion`
  other than `1` is rejected as unsupported rather than interpreted; a newer
  file requires a newer Aeria.
- `policy` must be `"pull-request"`. The former `"direct"` value is rejected
  with an explanation, because direct commits to the main branch are no
  longer supported; setting the main branch in Aeria rewrites the file.
- `mainBranch` may be absent or `null`; otherwise it must be a non-empty
  string.
- Invalid settings are reported as errors; Aeria never falls back to a
  default policy for a file that exists but is invalid.

The file contains no credentials, remote URLs, identities, or local paths.
