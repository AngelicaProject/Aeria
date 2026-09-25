# Runtime export

Aeria exports a versioned translation pack consumed by the in-game Harmonia
plugin.

Status: **implemented**. `aeria-export` collects, validates, writes, signs and
compresses packs and produces feed entries; `aeria-atlas` runs the Atlas
`encode` command; `aeria-publish` stores signing keys, creates GitHub releases,
and supplies the feed workflow; the desktop Export dialog drives them. The
contracts are [`../formats/pack-v1.md`](../formats/pack-v1.md),
[`../formats/feed-v1.md`](../formats/feed-v1.md), and
[`../formats/pack-settings-v1.md`](../formats/pack-settings-v1.md).

The pack is designed for fast, reliable runtime lookup and does not mirror the
editable Git workspace representation.

## Requirements

- explicit format version
- deterministic output for identical project, source, policy, release
  parameters, and signing key
- validation before emission; any failed string fails the export
- no partially written final pack; atomic replacement where supported
- every exported translation is bound to the exact source bytes it was made
  for, so the consumer never applies it to changed source text

## Pipeline

1. **Preconditions.** The project has a verified source whose `contentId`
   matches the workspace, and `aeria-pack.json` is valid. Translation data and
   `aeria-pack.json` have no uncommitted changes, because the manifest records
   `HEAD` as `project.commit`; the project stays locked from this check until
   collection ends. Export is unavailable in a degraded workspace.
2. **Select units.** Bound units with a target, filtered by the content
   policy: `reviewed` exports only `reviewed` units; `all` also exports
   `draft` and `needs-review` units as unreviewed cells. The policy is chosen
   per export and recorded in the pack manifest. Units whose source occurrence
   has no `rawValueHash` are skipped and listed in the export report.
3. **Validate.** Each target is parsed and semantically validated by
   `aeria-se`, as on save.
4. **Encode.** Target macro strings are sent in batches of 4096 to the
   Harmonia Atlas sidecar's `encode` command (`AtlasEncodeRunner`), which uses
   Lumina, the same library that produced HXS `macro_text`. Atlas returns the
   SeString bytes and confirms that decoding them and encoding the result again
   gives the same bytes; Lumina may print numbers differently from how they
   were typed, so the check is on bytes. A rejected string, a `0x00` byte, or a
   string longer than 65535 bytes fails the export. `collect_project` takes the
   encoder as a `StringEncoder` so the pipeline is testable without Atlas.
5. **Layout.** For every exported sheet, all String columns from the verified
   HXS sheet metadata form its layout; each unit's `columnIndex` becomes its
   string ordinal.
6. **Fonts.** When `aeria-fonts.json` exists, `aeria-fonts` renders the
   configured characters for every size of every listed game font (below) and
   the result becomes the pack's `FONTS` section.
7. **Write.** `write_pack` validates the input again, writes the canonical
   sections, computes `packHash`, and optionally appends the signature block
   (`PackSigner`, deterministic ECDSA P-256; a `KeyEndorsement` for key
   rotation). `write_file_atomically` publishes the file, and
   `compress_for_transport` derives the `.hpk.br` file.

## Game font glyphs

The Latin display fonts of the game (Jupiter, TrumpGothic, MiedingerMid) have
no Cyrillic, so window titles and similar labels show dashes. Aeria does not
ship replacement fonts; it renders only the missing glyphs, fitted to the
native metrics of each size, and Harmonia adds them to the game's current
fonts. A game update that changes the native fonts therefore keeps working,
and a size whose metrics changed is skipped by Harmonia rather than misdrawn.

`aeria-fonts` owns the settings ([`font-settings-v1.md`](../formats/font-settings-v1.md)),
the table of supported game font sizes with their measured metrics, the
bundled recommended fonts, and rasterization with `swash` (outlines, variable
font axes, no hinting). For a size it:

1. applies the axis values and computes the pixel size at which the source
   capital `H` is as tall as the native capitals, times `scale`;
2. renders each character (its uppercase form with `caseMapping: upper`),
   scaled horizontally by `widthScale`, as 8-bit coverage;
3. places the glyph on the native baseline (`ascent + baselineShift`), starts
   its bitmap at the pen position, trims it to its ink rows, and rounds the
   advance plus `tracking × capHeight` to whole pixels.

Identical settings, font files, and Aeria build produce identical glyphs. The
section carries each source's family, copyright, license identifier, full
license text, and file hash, so the OFL attribution travels with the pack.
The font settings and `fonts/` must be committed like `aeria-pack.json`; saving them never commits.

The Export dialog has a *Game fonts* section: it shows whether fonts are
configured, offers *Use recommended fonts*, lets the maintainer pick another
source file per game font and adjust axes and scales, and previews every size
as the game would draw it, at 1:1 and magnified, over guides for the native
line box, baseline, and capital height. The preview renders from the unsaved
settings; export uses the committed ones.

The export report (`ExportReport`) lists what was left out: detached units,
empty targets, unreviewed units under the `reviewed` policy, and units whose
source has no raw-value hash.

The manifest takes `packId`, `title`, `publisher`, `license`, and
`minHarmonia` from `aeria-pack.json`; `release` and `contentPolicy` from the
export; `target.language` from the workspace; `source` from the verified HXS;
`exporter.aeria` from the Aeria build; and `exporter.atlas` from
`harmonia-atlas --version` of the sidecar that encoded the strings.

## Desktop flow

*File → Export pack…* opens the Export dialog. Its sections are listed on
the left; the dialog opens on *Pack* until the pack settings exist and on
*Release* afterwards.

- **Release** shows what the export needs (pack settings saved, everything
  committed, the project key on this computer, font glyphs, pushed to
  GitHub) with a link to the section that fixes each item, then the release
  parameters:
  - content policy, with what each choice means for players;
  - the release number (`sequence`), which is not editable: one more than
    the highest local `harmonia/<n>` tag. Publishing raises it above the
    latest GitHub release when the local tags are behind, so numbers never
    repeat; the result shows the number used;
  - an optional version label (`version`) shown to players, today's date
    (`YYYY.MM.DD`) when empty;
  - an optional changelog and, with a GitHub target, the channel.

  The footer *Save pack file…* writes `<packId>-<sequence>.hpk` to a folder,
  signed when the project key is on this computer and unsigned otherwise;
  *Publish to GitHub* publishes (below).
- **Pack** edits the pack settings. Only the name and author are shown
  directly, each with an example; website, license, and minimum Harmonia
  version are under *More settings*. The pack ID is shown there read-only: it
  is made once when the pack is created, from the GitHub repository
  (`owner-name`, unique to the publisher) or, without one, from the name, and
  the dialog never changes it, because Harmonia pins trust and finds updates
  by it.
- **Game fonts** edits the font settings (below).
- **Signing** manages the signing key.
- **GitHub**, shown when `origin` is a GitHub repository, shows the feed URL
  and adds or updates the feed workflow.

The result of an export reports the pack hash, the signer, font glyph counts,
and what the export report left out.

## Signing

Packs are signed with a per-pack ECDSA P-256 key. The private key is stored
only in the OS credential store (Windows Credential Manager, the Secret
Service on Linux, or the macOS Keychain) under the service `Aeria` and the
account `pack-signing/<packId>`. It is never written to the project, settings,
logs, errors, or IPC responses; the renderer sees only fingerprints.

`aeria-pack.json` records the fingerprint of the project key. Creating the
first key records it. Saving the pack settings or recording a fingerprint
only writes the file; like every project file it is committed by the next
checkpoint, and the Release checklist names what is uncommitted and opens
the Git dock. A key whose fingerprint differs from the recorded one
cannot sign or publish, and importing it is refused. Creating a new project
key when one is recorded requires explicit confirmation, because Harmonia then
asks every player to trust the new publisher.

Several maintainers share the key through a backup file written on explicit
request (*Save backup*) and imported on the other computer (*Import
backup*). The backup is JSON:

```json
{
  "format": "aeria-signing-key",
  "version": 1,
  "packId": "ru-main",
  "fingerprint": "<64 hex>",
  "secretKey": "<64 hex>"
}
```

Import requires that `fingerprint` matches `secretKey`; `packId` is
informational. Local exports may be unsigned; published packs are always
signed.

> Open decision: the writer supports key rotation through `KeyEndorsement`,
> but the desktop does not offer rotation yet.

## Publishing

Publishing needs no Aeria service. It requires that `origin` is a github.com
repository, that `HEAD` is pushed (the branch has an upstream and no unpushed
commits), and that the stored key is the project key. Aeria then:

1. asks the Git credential helper for a `github.com` credential
   (`git credential fill`; with the bundled MinGit this is Git Credential
   Manager, which signs in through the browser when needed) and reports the
   outcome back with `approve` or `reject`. Aeria stores no GitHub token;
2. lists the releases and, when the requested `sequence` is not higher than
   every `harmonia/<n>` release (drafts included), uses the next free number;
3. builds and signs the pack, derives the `.hpk.br` file, and writes the feed
   entry with the release download URL;
4. creates the release `harmonia/<sequence>` on `HEAD` as a draft (a
   pre-release for the `testing` channel), uploads `<packId>-<sequence>.hpk.br`
   and `feed-entry.json`, and publishes it. If an upload or the publication
   fails, the draft is deleted. Upload URLs returned by GitHub must be on
   `uploads.github.com`, so the token is sent only to GitHub.

The feed is rebuilt by the repository workflow
`.github/workflows/harmonia-feed.yml`, which the Export dialog adds or
replaces from the template in `crates/aeria-publish/templates/`. It is a
project file: it shows in the Git dock's changes and the next checkpoint
commits it. GitHub runs release workflows from the default branch, so the
workflow must reach the main branch through a pull request before releases
reach the feed; the Export dialog checks the main branch on GitHub (as of the
last fetch) and warns until it has the current workflow. Releases published
before that need one manual run of the workflow (Actions → Harmonia feed →
Run workflow). The workflow runs on `release` events
(`published`, `unpublished`, `edited`, `deleted`) and on demand, and builds
the feed only in a run on the default branch. A release event runs with the
release tag as its ref, which the default `github-pages` environment refuses
to deploy, so it only starts an on-demand run on the default branch; an
on-demand run on another branch does nothing. A newer feed run cancels an
older one still in progress. It reads
`aeria-pack.json`, downloads the `feed-entry.json` asset of every published
`harmonia/<n>` release, checks that each entry's `sequence` matches its tag
and that no sequence repeats, keeps the ten newest releases per channel, and
deploys `harmonia/feed-v1.json` to GitHub Pages. GitHub Pages must use
GitHub Actions as its source. The workflow needs no signing key.

Packs cannot be built in CI because building requires game data from the
local installation.
