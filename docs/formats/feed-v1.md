# Harmonia Feed Format v1

Status: **implemented**. Harmonia reads feeds; `aeria-export` produces the
`feed-entry.json` object of a release (`feed_entry`); Aeria creates the GitHub
releases and supplies the feed workflow
([`../architecture/export.md`](../architecture/export.md#publishing)).

A feed is a small JSON document that tells Harmonia the newest
[Harmonia pack](./pack-v1.md) of each channel a project has published and
where to download it. The feed is a hint, not a trust anchor. Every decision
that matters (version, game, publisher key) is verified against
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
`feed-entry.json` assets of the published releases, takes `title` and
`publisherKeyFingerprint` from the committed
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
4. Verify that the pack's `packHash`, `version`, and `game` equal the feed
   entry, and apply the publisher trust rules below.
5. Install the file atomically as the feed's translation, replacing its
   previous pack. The new pack becomes active on the next game start; the
   previous file is kept for rollback until then.

## Publisher trust (Harmonia)

A pack carries no identifier. Harmonia keeps each installed translation under
a name of its own and pins one signing key fingerprint to it: the key of its
first signed pack, which the user confirmed. A translation added by its feed
link also remembers the feed; the feed's `publisherKeyFingerprint` is shown
before the first download. A pack updates a translation when it comes from
that translation's feed, or, from a file, when it is signed by the
translation's pinned key:

| Pack | Result |
| --- | --- |
| Signed by the pinned key of the translation it updates | accepted |
| Signed by a key the pinned key endorsed | accepted; the pin moves to the new key |
| From a feed with nothing installed yet, signed | the user confirms the fingerprint; a new translation with that key pinned |
| From the feed of a translation, signed by another key without an endorsement | rejected |
| From a file, signed by a key no translation pins | the user confirms the fingerprint; a new translation with that key pinned |
| Unsigned, from a feed | rejected |
| Unsigned, from a file | only after an explicit "unsigned build" confirmation; shown as unsigned. It replaces an installed unsigned translation with the same `title` and `team.name`, or becomes a new one |

A translation installed from a file is connected to a feed only when the
feed's `publisherKeyFingerprint` is the translation's pinned key.

Manual import accepts a single `.hpk` or `.hpk.br` file; the signature is
inside the pack.
