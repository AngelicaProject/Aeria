# Website

The project landing page is a static site in `site/`, published to GitHub
Pages at <https://angelicaproject.github.io/Aeria/>.

## Contents

- `site/index.html` is the English page and `site/ru/index.html` the Russian
  one. Both carry the same content; change them together. There is no build
  step, and nothing is loaded from other sites except the GitHub API
  (below).
- The page is a minimal single column: a headline, one paragraph on what
  Aeria does, the download button with its release line, five one- or two-sentence
  points whose subject is Aeria, the game, or the project,
  one line for players with a link to Harmonia, and a footer with project
  links and the trademark notice. It describes Aeria plainly, as a community
  tool for fan translations in any language, holds only what a visitor can
  use, describes only what is implemented, and leaves out how machine
  translation is provided.
- `site/style.css` is shared by both pages: the system font, black on white,
  and white on black when the system prefers a dark theme.
- `site/lang.js` remembers the language a visitor picks in the header. Until
  they pick one, the English page sends browsers whose language is Russian
  to `ru/`.
- `site/downloads.js` points the download button at the installer of the
  current stable release, or of the nightly release while there is no stable
  one, and writes its version, date, and size under the button; the portable
  archive and the nightly build are links below it (see
  [`releases.md`](./releases.md#distribution)). It reads `releases.json`,
  which the Pages workflow writes; without it, as in a local preview, it asks
  the GitHub API. If both fail, the links keep pointing at the release pages.
- `site/icon.png` (the link preview image) and `site/favicon.png` are copies
  of `apps/desktop/src-tauri/icons/128x128@2x.png` and `32x32.png`. Copy them
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
