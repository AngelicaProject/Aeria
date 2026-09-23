# Collaboration Settings v1

Status: **implemented in `aeria-git`**.

Collaboration Settings v1 is the project-shared collaboration policy. It is a
single optional file, `aeria-collaboration.json`, in the project root next to
`.aeria/`. It is committed with the project so every collaborator follows the
same workflow. It is not part of Workspace Format v1, and the Workspace
Format v1 reader ignores it like any other project-root file.

The behavior it selects is described in
[`../architecture/git.md`](../architecture/git.md#branches-and-collaboration-policy).

## Absence

A missing file means the `direct` policy with no main branch. Aeria does not
create the file until a policy is set explicitly.

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
| `policy` | `"direct"` or `"pull-request"` | yes |
| `mainBranch` | a valid Git branch name, or JSON `null` | no for `direct`; yes for `pull-request` |

`mainBranch` names the branch contributions are reviewed into. The canonical
writer always emits the field, using `null` when no main branch is set.

## Reader contract

- A leading UTF-8 BOM is accepted and ignored.
- The document must be one JSON object. Unknown fields are rejected.
- A missing or non-integer `formatVersion` is invalid. A `formatVersion`
  other than `1` is rejected as unsupported rather than interpreted; a newer
  file requires a newer Aeria.
- `policy` must be exactly one of the two values above.
- `mainBranch` may be absent or `null`; otherwise it must be a non-empty
  string. The `pull-request` policy without a main branch is invalid.
- Invalid settings are reported as errors; Aeria never falls back to a
  default policy for a file that exists but is invalid.

The file contains no credentials, remote URLs, identities, or local paths.
