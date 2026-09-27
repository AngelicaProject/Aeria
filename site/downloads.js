// Points the download buttons at the current stable and nightly builds.
// The Pages workflow writes releases.json on every deploy; without it (a
// local preview), the GitHub API is asked directly. If both fail, the
// buttons keep linking to the release pages.
const REPO = "AngelicaProject/Aeria";
const snapshotUrl = new URL("releases.json", document.currentScript.src);

async function fetchJson(url) {
  const response = await fetch(url, { headers: { Accept: "application/vnd.github+json" } });
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`${url}: ${response.status}`);
  return response.json();
}

async function currentReleases() {
  try {
    const response = await fetch(snapshotUrl, { cache: "no-cache" });
    if (response.ok) return await response.json();
  } catch (e) {}
  const api = `https://api.github.com/repos/${REPO}/releases`;
  const [stable, nightly] = await Promise.all([fetchJson(`${api}/latest`), fetchJson(`${api}/tags/nightly`)]);
  return { stable, nightly };
}

function findAsset(release, pattern) {
  const asset = release?.assets?.find((a) => pattern.test(a.name));
  return asset && asset.browser_download_url.startsWith(`https://github.com/${REPO}/releases/download/`) ? asset : null;
}

function render(row, release, strings) {
  const installer = findAsset(release, /^Aeria_.+_x64-setup\.exe$/);
  const portable = findAsset(release, /^Aeria_.+_x64-portable\.zip$/);
  const version = row.querySelector(".ver");
  if (!installer) {
    row.classList.add("missing");
    version.textContent = version.dataset.none || "";
    return false;
  }
  const date = new Date(release.published_at).toLocaleDateString(document.documentElement.lang, {
    day: "numeric",
    month: "long",
    year: "numeric",
  });
  version.textContent = `${installer.name.replace(/^Aeria_(.+)_x64-setup\.exe$/, "$1")} · ${date}`;
  for (const [link, asset] of [
    [row.querySelector('[data-asset="installer"]'), installer],
    [row.querySelector('[data-asset="portable"]'), portable],
  ]) {
    if (!link) continue;
    if (!asset) {
      link.hidden = true;
      continue;
    }
    link.href = asset.browser_download_url;
    const size = link.querySelector(".size");
    if (size) size.textContent = `${Math.round(asset.size / 1048576)} ${strings.mb}`;
  }
  return true;
}

(async () => {
  const box = document.querySelector(".downloads");
  if (!box) return;
  let releases;
  try {
    releases = await currentReleases();
  } catch (e) {
    return;
  }
  const strings = { mb: box.dataset.mb || "MB" };
  const hasStable = render(box.querySelector('[data-channel="stable"]'), releases.stable, strings);
  render(box.querySelector('[data-channel="nightly"]'), releases.nightly, strings);
  // Without a stable release, the nightly build is the one to take.
  if (!hasStable) box.querySelector('[data-channel="nightly"] .button')?.classList.remove("secondary");
})();
