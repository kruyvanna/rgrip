// Single source of truth for the donate link (GitHub Sponsors: 0% fees,
// available in Cambodia).
const DONATE_URL = "https://github.com/sponsors/yoouTaro";
document.querySelectorAll("[data-donate]").forEach((a) => (a.href = DONATE_URL));

// Download buttons: the newest release's zip (all releases are pre-releases,
// which GitHub's /releases/latest skips). Falls back to the releases page.
fetch("https://api.github.com/repos/yoouTaro/rgrip/releases?per_page=5")
  .then((r) => (r.ok ? r.json() : []))
  .then((releases) => {
    const release = releases.find((r) => !r.draft && r.assets.some((a) => a.name.endsWith(".zip")));
    if (!release) return;
    const zip = release.assets.find((a) => a.name.endsWith(".zip"));
    const version = release.tag_name.replace(/^v/, "").replace(/-alpha$/, "");  // "alpha" is written next to it
    document.querySelectorAll("[data-download]").forEach((a) => (a.href = zip.browser_download_url));
    document.querySelectorAll("[data-version]").forEach((el) => (el.textContent = version));
  })
  .catch(() => {});

const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;

// The hero screenshot starts tilted back and lies flat as you scroll.
const hero = document.querySelector(".shot-hero");
if (hero && !reduce) {
  let queued = false;
  const tilt = () => {
    queued = false;
    const t = Math.max(0, 1 - window.scrollY / 420);
    hero.style.setProperty("--tilt", t.toFixed(3));
  };
  tilt();
  addEventListener("scroll", () => {
    if (!queued) {
      queued = true;
      requestAnimationFrame(tilt);
    }
  }, { passive: true });
}

// Text rises in once as it scrolls into view. Without JS, or with Reduce
// Motion, everything is simply there.
if (!reduce) {
  document.documentElement.classList.add("js");
  const items = document.querySelectorAll(".hero .pill, .hero h1, .hero .lead, .hero .cta, .hero .fine, .copy");
  items.forEach((el) => el.classList.add("rise"));
  const io = new IntersectionObserver((entries) => {
    entries.forEach((e) => {
      if (!e.isIntersecting) return;
      io.unobserve(e.target);
      // Siblings in one row follow each other by a beat.
      const i = [...e.target.parentElement.children].indexOf(e.target);
      e.target.style.transitionDelay = `${Math.min(i, 6) * 60}ms`;
      e.target.classList.add("in");
    });
  }, { threshold: 0.15 });
  items.forEach((el) => io.observe(el));
}

// The clips are the product demo, so they play even with Reduce Motion on;
// then they get controls so they can be paused. Also start any clip the
// browser didn't autoplay once it scrolls into view.
const clips = document.querySelectorAll("video[autoplay]");
clips.forEach((v) => { if (reduce) v.controls = true; });
const playIo = new IntersectionObserver((entries) => {
  entries.forEach((e) => { if (e.isIntersecting && e.target.paused && !e.target.dataset.userPaused) e.target.play().catch(() => (e.target.controls = true)); });
}, { threshold: 0.3 });
clips.forEach((v) => {
  playIo.observe(v);
  v.addEventListener("pause", () => { if (!v.ended && document.visibilityState === "visible" && v.controls) v.dataset.userPaused = "1"; });
});
