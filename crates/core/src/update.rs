//! Over-the-air updates from GitHub Releases.
//!
//! Check: list releases (pre-releases included), pick the highest version
//! newer than ours that has a zip for this architecture. Install: download,
//! verify the SHA-256 published in the release notes, unpack next to the
//! running `.app`, then swap bundles (keeping the old one until the new one
//! is in place).

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO: &str = "yoouTaro/rgrip";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: semver::Version,
    /// Release page, for "what's new".
    pub page_url: String,
    pub asset_url: String,
    pub asset_name: String,
    /// From a "SHA-256: `…`" line in the release notes.
    pub sha256: Option<String>,
}

pub fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" }
}

pub fn asset_name(version: &str, arch: &str) -> String {
    format!("rgrip-{version}-macos-{arch}.zip")
}

/// The API base can be overridden (local testing / mirrors).
fn api_base() -> String {
    std::env::var("RGRIP_UPDATE_API").unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}"))
}

/// Pick the newest release above `current` with an asset for `arch`.
pub fn pick_update(releases_json: &str, current: &str, arch: &str) -> Result<Option<Release>, String> {
    let current = semver::Version::parse(current).map_err(|e| format!("bad current version: {e}"))?;
    let releases: Vec<serde_json::Value> =
        serde_json::from_str(releases_json).map_err(|e| format!("Unexpected response from GitHub: {e}"))?;
    let mut best: Option<Release> = None;
    for r in &releases {
        if r["draft"].as_bool().unwrap_or(false) {
            continue;
        }
        let tag = r["tag_name"].as_str().unwrap_or_default();
        let Ok(version) = semver::Version::parse(tag.trim_start_matches('v')) else { continue };
        if version <= current || best.as_ref().is_some_and(|b| b.version >= version) {
            continue;
        }
        let wanted = asset_name(&version.to_string(), arch);
        let Some(asset) = r["assets"].as_array().into_iter().flatten().find(|a| a["name"] == wanted) else {
            continue;
        };
        best = Some(Release {
            page_url: r["html_url"].as_str().unwrap_or_default().to_string(),
            asset_url: asset["browser_download_url"].as_str().unwrap_or_default().to_string(),
            asset_name: wanted,
            sha256: parse_sha256(r["body"].as_str().unwrap_or_default()),
            version,
        });
    }
    Ok(best)
}

/// Find a 64-hex-digit checksum on a line mentioning SHA-256.
pub fn parse_sha256(notes: &str) -> Option<String> {
    notes.lines().filter(|l| l.to_ascii_lowercase().contains("sha-256") || l.to_ascii_lowercase().contains("sha256")).find_map(|l| {
        l.split(|c: char| !c.is_ascii_hexdigit())
            .find(|w| w.len() == 64)
            .map(str::to_ascii_lowercase)
    })
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("rgrip/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())
}

/// GitHub's public releases feed: not under the API's rate limit
/// (60 requests an hour per address without a login).
fn feed_url() -> String {
    std::env::var("RGRIP_UPDATE_FEED").unwrap_or_else(|_| format!("https://github.com/{REPO}/releases.atom"))
}

pub async fn check() -> Result<Option<Release>, String> {
    match check_at(&api_base(), CURRENT).await {
        // Rate-limited: the feed has the versions and checksums too.
        Err(e) if e.contains("HTTP 403") || e.contains("HTTP 429") => check_feed(&feed_url(), CURRENT).await.map_err(|_| {
            "GitHub is limiting update checks from your network right now. Try again in a few minutes.".to_string()
        }),
        other => other,
    }
}

/// [`check`] from the releases Atom feed.
pub async fn check_feed(url: &str, current: &str) -> Result<Option<Release>, String> {
    let resp = client()?.get(url).send().await.map_err(|e| format!("Update check failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Update check failed: HTTP {}", resp.status()));
    }
    let text = resp.text().await.map_err(|e| format!("Update check failed: {e}"))?;
    pick_update_from_feed(&text, current, arch())
}

/// Like [`pick_update`], from the Atom feed. The feed lists no assets, so the
/// zip's URL follows GitHub's download pattern (a missing one fails the
/// download, not the check).
pub fn pick_update_from_feed(atom: &str, current: &str, arch: &str) -> Result<Option<Release>, String> {
    let current = semver::Version::parse(current).map_err(|e| format!("bad current version: {e}"))?;
    let mut best: Option<Release> = None;
    for entry in atom.split("<entry>").skip(1) {
        let Some(page) = entry.split("href=\"").nth(1).and_then(|s| s.split('"').next()) else { continue };
        let Some(tag) = page.split("/releases/tag/").nth(1) else { continue };
        let Ok(version) = semver::Version::parse(tag.trim_start_matches('v')) else { continue };
        if version <= current || best.as_ref().is_some_and(|b| b.version >= version) {
            continue;
        }
        let notes = entry.split("<content").nth(1).and_then(|s| s.split("</content>").next()).unwrap_or_default();
        let base = page.split("/releases/tag/").next().unwrap_or_default();
        let name = asset_name(&version.to_string(), arch);
        best = Some(Release {
            page_url: page.to_string(),
            asset_url: format!("{base}/releases/download/{tag}/{name}"),
            asset_name: name,
            sha256: parse_sha256(notes),
            version,
        });
    }
    Ok(best)
}

/// [`check`] against a specific API base (`…/repos/owner/name`) and version.
pub async fn check_at(api: &str, current: &str) -> Result<Option<Release>, String> {
    let url = format!("{api}/releases?per_page=30");
    let resp = client()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Update check failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Update check failed: HTTP {}", resp.status()));
    }
    let text = resp.text().await.map_err(|e| format!("Update check failed: {e}"))?;
    pick_update(&text, current, arch())
}

/// Download the release zip into `dir`, verifying its checksum.
pub async fn download(release: &Release, dir: &Path) -> Result<PathBuf, String> {
    let resp = client()?
        .get(&release.asset_url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Download failed: HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("Download failed: {e}"))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    match &release.sha256 {
        Some(expected) if *expected != digest => {
            return Err(format!("Checksum mismatch: expected {expected}, got {digest}. Update not installed."));
        }
        Some(_) => {}
        None => return Err("The release has no SHA-256 checksum; refusing to install.".into()),
    }
    let path = dir.join(&release.asset_name);
    std::fs::write(&path, &bytes).map_err(|e| format!("Cannot save update: {e}"))?;
    Ok(path)
}

/// The `.app` bundle the current executable runs from, if any.
pub fn current_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

/// Replace the bundle at `app` with the one inside `zip`.
pub fn install(zip: &Path, app: &Path) -> Result<(), String> {
    let parent = app.parent().ok_or("Bundle has no parent directory")?;
    let staging = parent.join(format!(".rgrip-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("Cannot unpack update: {e}"))?;
    let cleanup = |r: Result<(), String>| {
        let _ = std::fs::remove_dir_all(&staging);
        r
    };

    let status = Command::new("ditto").arg("-x").arg("-k").arg(zip).arg(&staging).status();
    if !status.is_ok_and(|s| s.success()) {
        return cleanup(Err("Cannot unpack the update archive".into()));
    }
    let new_app = staging.join("rgrip.app");
    if !new_app.join("Contents/MacOS/rgrip").is_file() {
        return cleanup(Err("The update archive does not contain rgrip.app".into()));
    }

    // Swap: old → backup, new → place; roll back if the second step fails.
    let backup = parent.join(format!(".rgrip-old-{}.app", std::process::id()));
    std::fs::rename(app, &backup).map_err(|e| format!("Cannot replace {}: {e}", app.display()))?;
    if let Err(e) = std::fs::rename(&new_app, app) {
        let _ = std::fs::rename(&backup, app);
        return cleanup(Err(format!("Cannot install update: {e}")));
    }
    let _ = std::fs::remove_dir_all(&backup);
    cleanup(Ok(()))
}

/// Start the (new) bundle; the caller should exit right after.
pub fn relaunch(app: &Path) -> Result<(), String> {
    Command::new("open").arg("-n").arg(app).spawn().map(|_| ()).map_err(|e| format!("Cannot relaunch: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "bcf0e04e428a0550756ddf7d9ca597a0c8000d3d304e4212d5701987ff91df4d";

    fn release(tag: &str, draft: bool, arches: &[&str]) -> serde_json::Value {
        let v = tag.trim_start_matches('v');
        serde_json::json!({
            "tag_name": tag,
            "draft": draft,
            "html_url": format!("https://github.com/{REPO}/releases/tag/{tag}"),
            "body": format!("Notes\n\nSHA-256: `{SHA}`"),
            "assets": arches.iter().map(|a| serde_json::json!({
                "name": asset_name(v, a),
                "browser_download_url": format!("https://example.com/{}", asset_name(v, a)),
            })).collect::<Vec<_>>(),
        })
    }

    fn pick(releases: Vec<serde_json::Value>, current: &str) -> Option<String> {
        let json = serde_json::to_string(&releases).unwrap();
        pick_update(&json, current, "arm64").unwrap().map(|r| r.version.to_string())
    }

    #[test]
    fn picks_newest_matching_release() {
        let rs = vec![
            release("v0.0.1-alpha", false, &["arm64"]),
            release("v0.0.2-alpha", false, &["arm64"]),
            release("v0.0.3-alpha", false, &["x86_64"]), // no asset for us
            release("v0.1.0", true, &["arm64"]),          // draft
            release("nightly", false, &["arm64"]),        // not a version
        ];
        assert_eq!(pick(rs, "0.0.1-alpha"), Some("0.0.2-alpha".into()));
    }

    #[test]
    fn version_ordering() {
        let rs = vec![release("v0.0.1-alpha", false, &["arm64"]), release("v0.0.1", false, &["arm64"])];
        // A release beats its own pre-release.
        assert_eq!(pick(rs.clone(), "0.0.1-alpha"), Some("0.0.1".into()));
        assert_eq!(pick(rs.clone(), "0.0.1"), None);
        assert_eq!(pick(vec![release("v0.0.1-beta", false, &["arm64"])], "0.0.1-alpha"), Some("0.0.1-beta".into()));
        assert_eq!(pick(vec![], "0.0.1-alpha"), None);
    }

    #[test]
    fn release_fields() {
        let json = serde_json::to_string(&vec![release("v0.0.2-alpha", false, &["arm64"])]).unwrap();
        let r = pick_update(&json, "0.0.1-alpha", "arm64").unwrap().unwrap();
        assert_eq!(r.asset_name, "rgrip-0.0.2-alpha-macos-arm64.zip");
        assert_eq!(r.sha256.as_deref(), Some(SHA));
        assert!(r.page_url.ends_with("/v0.0.2-alpha"));
    }

    #[test]
    fn sha_parsing() {
        assert_eq!(parse_sha256(&format!("SHA-256: `{}`", SHA.to_uppercase())).as_deref(), Some(SHA));
        assert_eq!(parse_sha256(&format!("sha256 {SHA}")).as_deref(), Some(SHA));
        assert_eq!(parse_sha256(&format!("commit {SHA}")), None);
        assert_eq!(parse_sha256("SHA-256: abc"), None);
    }

    #[test]
    fn picks_from_the_feed() {
        let atom = r#"<feed><entry>
    <link rel="alternate" type="text/html" href="https://github.com/yoouTaro/rgrip/releases/tag/v0.0.10-alpha"/>
    <content type="html">&lt;p&gt;SHA-256: &lt;code&gt;febff09636b011ca25b96c1639cadf7aefb6add164f1bbb5db09e0121224b7a6&lt;/code&gt;&lt;/p&gt;</content>
  </entry><entry>
    <link rel="alternate" type="text/html" href="https://github.com/yoouTaro/rgrip/releases/tag/v0.0.9-alpha"/>
    <content type="html">&lt;p&gt;SHA-256: &lt;code&gt;aea408f46edf58e956127affbb129594fdfeab10671daf3000fbe26e8fd1471b&lt;/code&gt;</content>
  </entry></feed>"#;
        let r = pick_update_from_feed(atom, "0.0.9-alpha", "arm64").unwrap().unwrap();
        assert_eq!(r.version.to_string(), "0.0.10-alpha");
        assert_eq!(r.asset_url, "https://github.com/yoouTaro/rgrip/releases/download/v0.0.10-alpha/rgrip-0.0.10-alpha-macos-arm64.zip");
        assert_eq!(r.sha256.as_deref(), Some("febff09636b011ca25b96c1639cadf7aefb6add164f1bbb5db09e0121224b7a6"));
        assert_eq!(pick_update_from_feed(atom, "0.0.10-alpha", "arm64").unwrap(), None);
    }

}
