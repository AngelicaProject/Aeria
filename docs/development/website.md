# Website

The project landing page is a static site in `site/`, published to GitHub
Pages at <https://angelicaproject.github.io/Aeria/>.

## Contents

- `site/index.html` is the English page and `site/ru/index.html` the Russian
  one. Both carry the same content; change them together. There is no build
  step and no external fonts or libraries.
- `site/style.css` is shared by both pages. It uses the system sans-serif
  font (monospace only for tags) and flat colors on a warm light
  background, regardless of the system theme.
- `site/lang.js` remembers the language a visitor picks with the switch in
  the masthead. Until they pick one, the English page sends browsers whose
  language is Russian to `ru/`.
- The page describes Aeria plainly, as a community tool for fan
  translations in any language; it is not written as product marketing.
  Describe only what is implemented. The page is plain: the name, one sentence on what
  Aeria is, a bulleted list of what it does, one concrete sentence per item
  (machine translation, see
  [`../architecture/translate.md`](../architecture/translate.md), among them), the
  tag example, installing a translation in the game, and development links.
- The string specimen shows an English source line translated in several
  fan projects (Russian, Spanish, Portuguese), none of which the game ships.
- `site/downloads.js` fills the download block with direct links to the
  installer and portable archive of the current stable and nightly releases
  (see [`releases.md`](./releases.md#distribution)), with their version,
  date, and size. It reads `releases.json`, which the Pages workflow writes;
  without it, as in a local preview, it asks the GitHub API. If both fail,
  the buttons keep linking to the release pages. A channel without a release
  is shown as such, and without a stable release the nightly button becomes
  the primary one.
- The Harmonia section gives players short installation steps. They follow
  the Installing and Using it sections of the
  [Harmonia README](https://github.com/AngelicaProject/Harmonia#installing),
  including its custom repository address; update the steps when that README
  changes. `site/copy.js` handles the button that copies the address.
- `site/icon.png` and `site/favicon.png` are copies of
  `apps/desktop/src-tauri/icons/128x128@2x.png` and `32x32.png`. Copy them
  again when the application icon changes.

The page describes the product as documented in
[`../product/vision.md`](../product/vision.md) and
[`../product/principles.md`](../product/principles.md). When those change in a
way the pages state, update both pages in the same change. When release
asset names change, update the patterns in `downloads.js`.

## Publishing

`.github/workflows/pages.yml` deploys `site/` with `actions/deploy-pages` on
every push to `main` that touches `site/` or the workflow, after every
successful Release workflow run, and on manual dispatch. Before uploading, it
lists the repository's releases and writes `site/releases.json` with the
newest non-draft, non-pre-release release (stable) and the `nightly` release,
each `null` when absent. The file is ignored by Git; a failed API call fails
the deploy rather than publishing wrong links. The repository's Pages source must be set to
**GitHub Actions** (Settings → Pages).

To preview locally, serve the folder with any static file server, for example:

```text
python -m http.server 4173 --directory site
```
