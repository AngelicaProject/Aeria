# Pack Settings v1

Status: **implemented in `aeria-export`**.

Pack Settings v1 is the project-shared identity of the project's
[Harmonia pack](./pack-v1.md). It is a single optional file,
`aeria-pack.json`, in the project root next to `.aeria/`. It is committed with
the project so every maintainer exports the same pack, and the repository's
feed workflow reads it to build the [feed](./feed-v1.md) without Aeria. It is
not part of the workspace format; the workspace reader ignores it like any
other project-root file.

How the settings are used is described in
[`../architecture/export.md`](../architecture/export.md).

## Absence

A missing file means the project has no pack yet. Aeria does not create the
file until the settings are saved explicitly, and it does not export without
them.

Saving the settings in Aeria, or recording a new key fingerprint, only writes
the file. Like every project file it is committed by the next checkpoint.

## Canonical form

UTF-8 JSON with two-space indentation, the field order below, and one LF after
the closing `}`:

```json
{
  "formatVersion": 1,
  "packId": "ru-main",
  "title": "Russian translation",
  "publisher": {
    "name": "Example team",
    "url": null
  },
  "license": "CC-BY-NC-SA-4.0",
  "minHarmonia": "0.1.0",
  "signingKeyFingerprint": "<64 lowercase hex>"
}
```

| Field | Rule | Required |
| --- | --- | --- |
| `formatVersion` | the JSON integer `1` | yes |
| `packId` | `[a-z0-9][a-z0-9-]{0,63}`; the manifest `packId` of every pack | yes |
| `title` | non-empty display string without surrounding whitespace | yes |
| `publisher.name` | as `title` | yes |
| `publisher.url` | as `title`, or `null` | no |
| `license` | as `title`, or `null` | no |
| `minHarmonia` | two to four dot-separated decimal numbers | yes |
| `signingKeyFingerprint` | fingerprint of the publisher key (SHA-256 of the 65-byte public key, as in the pack signature block), or `null` before a key is chosen | no |

The canonical writer always emits every field, using `null` for absent
optional values.

`signingKeyFingerprint` pins the key that signs published packs. Aeria
publishes only with a stored key whose fingerprint equals it, and the feed
workflow copies it to the feed's `publisherKeyFingerprint`.

## Reader contract

- A leading UTF-8 BOM is accepted and ignored.
- The document must be one JSON object. Unknown fields, in the object and in
  `publisher`, are rejected.
- A missing or non-integer `formatVersion` is invalid. A `formatVersion`
  other than `1` is rejected as unsupported rather than interpreted; a newer
  file requires a newer Aeria.
- Invalid settings are reported as errors; Aeria never falls back to defaults
  for a file that exists but is invalid.

The file contains no private keys, credentials, remote URLs, or local paths.
