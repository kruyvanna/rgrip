// Single source of truth for the donate link (GitHub Sponsors: 0% fees,
// available in Cambodia).
const DONATE_URL = "https://github.com/sponsors/yoouTaro";
document.querySelectorAll("[data-donate]").forEach((a) => (a.href = DONATE_URL));
document.getElementById("year").textContent = new Date().getFullYear();
