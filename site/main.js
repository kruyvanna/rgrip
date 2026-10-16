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
    const version = release.tag_name.replace(/^v/, "").replace(/-alpha$/, "");
    document.querySelectorAll("[data-download]").forEach((a) => {
      a.href = zip.browser_download_url;
      a.querySelector("span").textContent = `Download for macOS · ${version}`;
    });
    document.querySelectorAll("[data-version]").forEach((el) => (el.textContent = version));
    showReleases(releases.filter((r) => !r.draft).slice(0, 4));
  })
  .catch(() => {});

// Split-flap boards. The text is in the HTML; this turns each character into
// a flap cell. Hero rows are padded with blank cells to the longest row, like
// a real board. A cascade flips the cells through a few characters before
// they settle: once when a board comes into view, and again on a row you
// point at.
const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
const GLYPHS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.·-";

function cell(ch, cls) {
  const c = document.createElement("span");
  c.className = "cell" + (cls ? " " + cls : "");
  c.textContent = ch === " " ? " " : ch;
  c.dataset.ch = c.textContent;
  c.setAttribute("aria-hidden", "true");
  return c;
}

function build(el) {
  const rows = el.querySelectorAll(".row").length ? [...el.querySelectorAll(".row")] : [el];
  const width = Math.max(...rows.map((r) => r.textContent.trim().length));
  // Keep the words for screen readers; the cells are decoration.
  if (!el.hasAttribute("aria-label")) el.setAttribute("aria-label", el.textContent.trim());
  rows.forEach((row) => {
    const text = row.textContent.trim();
    row.textContent = "";
    if (row === el) {
      const inner = document.createElement("span");
      inner.className = "row";
      el.appendChild(inner);
      row = inner;
    }
    text.split(" ").forEach((word, i) => {
      if (i) row.appendChild(cell(" ", "blank sp"));
      const w = document.createElement("span");
      w.className = "word";
      [...word].forEach((ch) => w.appendChild(cell(ch)));
      row.appendChild(w);
    });
    if (rows.length > 1) for (let i = text.length; i < width; i++) row.appendChild(cell(" ", "blank pad"));
  });
  el.classList.add("flaps", "ready");
}

function cascade(el, { stagger = 32, rowDelay = 140 } = {}) {
  if (reduce) return;
  const rows = [...el.querySelectorAll(".row")];
  rows.forEach((row, r) => {
    [...row.querySelectorAll(".cell:not(.blank)")].forEach((c, i) => {
      const flips = 3 + ((i * 7 + r * 3) % 5);
      const start = r * rowDelay + i * stagger;
      for (let n = 1; n <= flips; n++) {
        setTimeout(() => {
          const last = n === flips;
          c.textContent = last ? c.dataset.ch : GLYPHS[(Math.random() * GLYPHS.length) | 0];
          c.classList.remove("flip");
          void c.offsetWidth;
          c.classList.add("flip");
        }, start + n * 70);
      }
    });
  });
}

// The releases board, from the same API answer; the rows in the HTML stay
// when the request fails.
function showReleases(list) {
  const body = document.querySelector("[data-releases]");
  if (!body || !list.length) return;
  body.textContent = "";
  list.forEach((r, i) => {
    const tr = document.createElement("tr");
    tr.className = i ? "past" : "now";
    const th = document.createElement("th");
    th.scope = "row";
    th.textContent = r.tag_name.replace(/^v/, "").replace(/-alpha$/, "");
    const date = document.createElement("td");
    date.className = "fig";
    date.textContent = (r.published_at || r.created_at || "").slice(0, 10);
    const status = document.createElement("td");
    const a = document.createElement("a");
    a.href = r.html_url;
    a.textContent = i ? "Notes" : "Latest";
    status.appendChild(a);
    tr.append(th, date, status);
    body.appendChild(tr);
    if (!i) {
      build(th);
      seen.observe(th);
    }
  });
}

const boards = document.querySelectorAll("[data-flaps]");
boards.forEach(build);

const seen = new IntersectionObserver((entries) => {
  entries.forEach((e) => {
    if (!e.isIntersecting) return;
    seen.unobserve(e.target);
    cascade(e.target);
  });
}, { threshold: 0.6 });
boards.forEach((b) => seen.observe(b));

// Pointing at a departure flips its name again.
document.querySelectorAll(".departures tbody tr").forEach((tr) => {
  const flaps = tr.querySelector(".flaps");
  if (!flaps) return;
  let last = 0;
  tr.addEventListener("pointerenter", () => {
    if (Date.now() - last < 900) return;
    last = Date.now();
    cascade(flaps, { stagger: 18 });
  });
});

// The clips are the product demo, so they play even with Reduce Motion on;
// then they get controls so they can be paused. Also start any clip the
// browser didn't autoplay once it scrolls into view.
const clips = document.querySelectorAll("video[autoplay]");
clips.forEach((v) => { if (reduce) v.controls = true; });
const io = new IntersectionObserver((entries) => {
  entries.forEach((e) => { if (e.isIntersecting && e.target.paused && !e.target.dataset.userPaused) e.target.play().catch(() => (e.target.controls = true)); });
}, { threshold: 0.3 });
clips.forEach((v) => {
  io.observe(v);
  v.addEventListener("pause", () => { if (!v.ended && document.visibilityState === "visible" && v.controls) v.dataset.userPaused = "1"; });
});
