# Harmonia Feed Format v1

Status: **implemented**. Harmonia reads feeds; `aeria-export` produces the
`feed-entry.json` object of a release (`feed_entry`); Aeria creates the GitHub
releases and supplies the feed workflow
([`../architecture/export.md`](../architecture/export.md#publishing)).

A feed is a small JSON document that tells Harmonia which
[Harmonia packs](./pack-v1.md) a project has published and where to download
them. The feed is a hint, not a trust anchor. Every decision that matters
(pack identity, sequence, source compatibility, publisher) is verified against
the downloaded pack itself, so the feed needs no signature of its own.

## Hosting

The recommended layout uses only GitHub features of the translation
repository:

| Artifact | Location |
| --- | --- |
| Pack | Release asset `<packId>-<sequence>.hpk.br` on release tag `harmonia/<sequence>` |
| Release entry | Release asset `feed-entry.json` on the same release: the exact `releases[]` object below |
| Feed | `https://<owner>.github.io/<repo>/harmonia/feed-v1.json` on GitHub Pages |

`testing` releases are published as GitHub pre-releases.

Aeria creates the release and uploads both assets. A GitHub Actions workflow,
supplied by Aeria as a template for the translation repository
(`.github/workflows/harmonia-feed.yml`), is started by `release` events, runs
on the repository's default branch, collects the
`feed-entry.json` assets of the published releases, takes `packId`, `title`,
and `publisherKeyFingerprint` from the committed
[`aeria-pack.json`](./pack-settings-v1.md), and deploys the feed to Pages. The
workflow never needs signing keys. Pack files are never stored in Pages or in
Git history.

Any other static HTTPS host works as long as it serves the same documents.

## Document

UTF-8 JSON without BOM.

```json
{
  "format": "harmonia-feed",
  "version": 1,
  "packId": "ru-main",
  "title": "Russian translation",
  "homepage": "https://github.com/<owner>/<repo>",
  "publisherKeyFingerprint": "<64 hex>",
  "releases": [
    {
      "sequence": 42,
      "version": "2026.09.25",
      "channel": "stable",
      "packHash": "sha256:<hex>",
      "source": {
        "language": "en",
        "gameVersion": "2026.08.12.0000.0000",
        "contentId": "sha256:<hex>"
      },
      "target": { "language": "ru" },
      "contentPolicy": "reviewed",
      "minHarmonia": "1.4.0",
      "download": {
        "url": "https://github.com/<owner>/<repo>/releases/download/harmonia/42/ru-main-42.hpk.br",
        "encoding": "br",
        "size": 15234567,
        "sha256": "<hex>",
        "unpackedSize": 61234567
      },
      "changelog": "Patch 7.35 main scenario."
    }
  ]
}
```

| Field | Rule |
| --- | --- |
| `format`, `version` | exactly `"harmonia-feed"` and `1` |
| `packId` | equals every listed pack's manifest `packId` |
| `publisherKeyFingerprint` | fingerprint of the current signing key, or `null` for unsigned feeds; shown to the user when the feed is added |
| `releases` | sorted by `sequence` descending; the generator keeps at most the 10 newest per channel |
| `sequence`, `version`, `channel`, `source`, `target`, `contentPolicy`, `minHarmonia` | copies of the pack manifest |
| `packHash` | the pack's `packHash` |
| `download.encoding` | `br` for `.hpk.br`, `identity` for a plain `.hpk` |
| `download.size`, `download.sha256` | size and SHA-256 of the downloaded bytes |
| `download.unpackedSize` | size of the `.hpk` after decoding |
| `changelog` | optional display text |

Readers ignore unknown fields in version 1 documents. An incompatible change
publishes a new document (`feed-v2.json`) next to this one instead of changing
`version`, so installed Harmonia versions keep receiving updates they can read.

## Release selection (Harmonia)

1. Keep releases whose channel the user follows, whose `minHarmonia` is
   satisfied, and whose `source.language` equals the client language.
2. Prefer a release whose `source.gameVersion` equals the running game
   version; among equals, the highest `sequence`.
3. Offer or install it only when its `sequence` is higher than the installed
   pack's, or when it matches the game version and the installed pack does
   not. A lower `sequence` is installed only by explicit user action.

Checks run on the configured interval, on demand, and once after the running
game version changes. Requests send `If-None-Match` with the last `ETag`.

## Download and install (Harmonia)

1. Stream to a staging file, aborting beyond `download.size`; verify
   `download.sha256`.
2. Decode with an output limit of `download.unpackedSize`.
3. Verify the pack completely (see [pack reader requirements](./pack-v1.md#reader-requirements-harmonia)).
4. Verify that the pack's `packHash`, `packId`, `sequence`, and `source` equal
   the feed entry, and apply the publisher trust rules below.
5. Move the file atomically to `packs/<packId>/<packHash>.hpk` and atomically
   update the installed-pack record. The new pack becomes active on the next
   game start; the previous file is kept for rollback until then.

## Publisher trust (Harmonia)

Trust is pinned per `packId` to a signing key fingerprint. Feed installs and
manual imports follow the same rules:

| Pack | Pinned key for `packId` | Result |
| --- | --- | --- |
| Signed, key = pinned | any | accepted |
| Signed, endorsed by the pinned key | pinned | accepted; pin moves to the new key |
| Signed | none | user confirms the fingerprint; key is pinned |
| Signed, other key, no valid endorsement | pinned | rejected for feeds; manual import requires the user to explicitly replace the pin |
| Unsigned | any | rejected for feeds; manual import only after an explicit "unsigned build" confirmation, shown as unsigned in the UI |

Manual import accepts a single `.hpk` or `.hpk.br` file; the signature is
inside the pack.
