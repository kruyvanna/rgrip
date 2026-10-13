// Single source of truth for the donate link (GitHub Sponsors: 0% fees,
// available in Cambodia).
const DONATE_URL = "https://github.com/sponsors/yoouTaro";
document.querySelectorAll("[data-donate]").forEach((a) => (a.href = DONATE_URL));
document.getElementById("year").textContent = new Date().getFullYear();

// Download buttons: the newest release's zip (all releases are pre-releases,
// which GitHub's /releases/latest skips). Falls back to the releases page.
fetch("https://api.github.com/repos/yoouTaro/rgrip/releases?per_page=5")
  .then((r) => (r.ok ? r.json() : []))
  .then((releases) => {
    const release = releases.find((r) => !r.draft && r.assets.some((a) => a.name.endsWith(".zip")));
    if (!release) return;
    const zip = release.assets.find((a) => a.name.endsWith(".zip"));
    document.querySelectorAll("[data-download]").forEach((a) => {
      a.href = zip.browser_download_url;
      a.textContent = `Download for macOS · ${release.tag_name.replace(/^v/, "")}`;
    });
  })
  .catch(() => {});
