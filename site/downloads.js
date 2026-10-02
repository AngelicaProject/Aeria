// Points the download button at the current build: the stable release, or
// the nightly one while there is no stable release. The Pages workflow writes
// releases.json on every deploy; without it (a local preview), the GitHub API
// is asked directly. If both fail, the links keep pointing at the release pages.
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

// The installer, portable archive, version, and date of a release, or null without an installer.
function build(release) {
  const installer = findAsset(release, /^Aeria_.+_x64-setup\.exe$/);
  if (!installer) return null;
  return {
    installer,
    portable: findAsset(release, /^Aeria_.+_x64-portable\.zip$/),
    version: installer.name.replace(/^Aeria_(.+)_x64-setup\.exe$/, "$1"),
    date: new Date(release.published_at).toLocaleDateString(document.documentElement.lang, {
      day: "numeric",
      month: "long",
      year: "numeric",
    }),
  };
}

(async () => {
  const box = document.querySelector(".get");
  if (!box) return;
  let releases;
  try {
    releases = await currentReleases();
  } catch (e) {
    return;
  }
  const stable = build(releases.stable);
  const nightly = build(releases.nightly);
  const current = stable || nightly;
  if (!current) return;
  const link = (role) => box.querySelector(`[data-role="${role}"]`);
  const mb = `${Math.round(current.installer.size / 1048576)} ${box.dataset.mb || "MB"}`;
  link("installer").href = current.installer.browser_download_url;
  const label = stable ? box.dataset.stable : box.dataset.nightly;
  const facts = [`${label} ${current.version}`, current.date, mb];
  if (!stable && box.dataset.unstable) facts.push(box.dataset.unstable);
  box.querySelector(".ver").textContent = facts.join(" · ");
  if (current.portable) link("portable").href = current.portable.browser_download_url;
  else link("portable").hidden = true;
  // The nightly link is for the build that is not the button's.
  if (!stable) link("nightly").hidden = true;
  else if (nightly) {
    link("nightly").href = nightly.installer.browser_download_url;
    link("nightly").title = `${nightly.version} · ${nightly.date}`;
  }
})();
