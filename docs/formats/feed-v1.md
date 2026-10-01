# Harmonia Feed Format v1

Status: **implemented**. Harmonia reads feeds; `aeria-export` produces the
`feed-entry.json` object of a release (`feed_entry`); Aeria creates the GitHub
releases and supplies the feed workflow
([`../architecture/export.md`](../architecture/export.md#publishing)).

A feed is a small JSON document that tells Harmonia the newest
[Harmonia pack](./pack-v1.md) of each channel a project has published and
where to download it. The feed is a hint, not a trust anchor. Every decision
that matters (pack identity, version, game, publisher key) is verified against
the downloaded pack itself, so the feed needs no signature of its own.

## Hosting

The recommended layout uses only GitHub features of the translation
repository:

| Artifact | Location |
| --- | --- |
| Pack | Release asset `pack-<version>.hpk.br` on release tag `harmonia/<version>` |
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
  "packId": "3f6c1a2e-8b4d-4c1f-9a7e-5d2b0c6e1f38",
  "title": "Русский перевод",
  "publisherKeyFingerprint": "<64 hex>",
  "releases": [
    {
      "version": "2026.10.01.0002",
      "channel": "stable",
      "language": "ru",
      "game": { "language": "en", "version": "2026.08.12.0000.0000" },
      "minHarmonia": "0.1.2.0",
      "packHash": "sha256:<hex>",
      "download": {
        "url": "https://github.com/<owner>/<repo>/releases/download/harmonia/2026.10.01.0002/pack-2026.10.01.0002.hpk.br",
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
| `title` | the pack's title, shown when the feed is added; optional for readers |
| `publisherKeyFingerprint` | fingerprint of the current signing key, or `null` for unsigned feeds; shown to the user when the feed is added |
| `releases` | the newest release of each channel, at most one `stable` and one `testing`, newest version first |
| `version`, `channel`, `language`, `game`, `minHarmonia` | copies of the pack manifest |
| `packHash` | the pack's `packHash` |
| `download.encoding` | `br` for `.hpk.br`, `identity` for a plain `.hpk` |
| `download.size`, `download.sha256` | size and SHA-256 of the downloaded bytes |
| `download.unpackedSize` | size of the `.hpk` after decoding |
| `changelog` | optional display text |

The feed holds no release history. Players update the game through the
launcher and Harmonia through Dalamud, so only the newest release of a channel
is installed from a feed; an older release is installed from its file (manual
import).

Readers ignore unknown fields in version 1 documents. An incompatible change
publishes a new document (`feed-v2.json`) next to this one instead of changing
`version`, so installed Harmonia versions keep receiving updates they can read.

## Release selection (Harmonia)

1. Take the `stable` release, and the `testing` release when the user follows
   testing releases; of these, the one with the highest `version`.
2. When its `game.language` differs from the client language, the feed offers
   nothing for this client.
3. When its `minHarmonia` is not satisfied, Harmonia asks the user to update
   Harmonia instead of installing it.
4. Offer or install it only when its `version` is higher than the installed
   pack's. A lower version is installed only by explicit user action.

Checks run on the configured interval, on demand, and once after the running
game version changes. Requests send `If-None-Match` with the last `ETag`.

## Download and install (Harmonia)

1. Stream to a staging file, aborting beyond `download.size`; verify
   `download.sha256`.
2. Decode with an output limit of `download.unpackedSize`.
3. Verify the pack completely (see [pack reader requirements](./pack-v1.md#reader-requirements-harmonia)).
4. Verify that the pack's `packHash`, `packId`, `version`, and `game` equal
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
