//! End-to-end update flow against a local fake of the GitHub API.

use rgrip_core::update;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A minimal `rgrip.app` whose binary is just a marker file.
fn fake_app(dir: &Path, marker: &str) -> std::path::PathBuf {
    let app = dir.join("rgrip.app");
    std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    std::fs::write(app.join("Contents/MacOS/rgrip"), marker).unwrap();
    app
}

/// Serve `/releases` (JSON built from the server's own base URL) and
/// `/asset.zip` (bytes) on a random port. Returns the base URL.
async fn serve(releases: impl Fn(&str) -> String, zip: Vec<u8>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let releases = releases(&base);
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let (releases, zip) = (releases.clone(), zip.clone());
            tokio::spawn(async move {
                let mut buf = vec![0; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let (ctype, body) = if req.starts_with("GET /releases") {
                    ("application/json", releases.into_bytes())
                } else {
                    ("application/zip", zip)
                };
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&body).await;
            });
        }
    });
    base
}

fn releases_json(base: &str, version: &str, sha: &str) -> String {
    serde_json::json!([{
        "tag_name": format!("v{version}"),
        "draft": false,
        "html_url": format!("{base}/release"),
        "body": format!("New stuff\n\nSHA-256: `{sha}`"),
        "assets": [{
            "name": update::asset_name(version, update::arch()),
            "browser_download_url": format!("{base}/asset.zip"),
        }],
    }])
    .to_string()
}

#[tokio::test]
async fn check_download_verify_install() {
    let tmp = std::env::temp_dir().join(format!("rgrip-update-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);

    // The "new" release: a zipped app with a different binary.
    let build = tmp.join("build");
    let new_app = fake_app(&build, "new version");
    let zip_path = tmp.join("release.zip");
    assert!(Command::new("ditto").args(["-c", "-k", "--keepParent"]).arg(&new_app).arg(&zip_path).status().unwrap().success());
    let zip = std::fs::read(&zip_path).unwrap();
    let sha = format!("{:x}", Sha256::digest(&zip));

    // The installed app that will be replaced.
    let installed = fake_app(&tmp.join("Applications"), "old version");

    // Checksum OK → installs.
    let base = serve(|base| releases_json(base, "0.0.2-alpha", &sha), zip.clone()).await;
    let release = update::check_at(&base, "0.0.1-alpha").await.unwrap().expect("update found");
    assert_eq!(release.version.to_string(), "0.0.2-alpha");
    assert_eq!(update::check_at(&base, "0.0.2-alpha").await.unwrap(), None, "no update when current");

    let downloaded = update::download(&release, &tmp).await.unwrap();
    update::install(&downloaded, &installed).unwrap();
    assert_eq!(std::fs::read_to_string(installed.join("Contents/MacOS/rgrip")).unwrap(), "new version");
    // No staging or backup bundles left behind.
    let leftovers: Vec<_> = std::fs::read_dir(installed.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(leftovers, ["rgrip.app"]);

    // Tampered download → rejected before anything is written.
    let mut bad = release.clone();
    bad.sha256 = Some("0".repeat(64));
    let err = update::download(&bad, &tmp).await.unwrap_err();
    assert!(err.contains("Checksum mismatch"), "{err}");
    bad.sha256 = None;
    assert!(update::download(&bad, &tmp).await.unwrap_err().contains("no SHA-256"));

    // A zip without an app is refused and the installed app is untouched.
    let junk = tmp.join("junk.zip");
    std::fs::write(tmp.join("readme.txt"), "x").unwrap();
    assert!(Command::new("ditto").args(["-c", "-k"]).arg(tmp.join("readme.txt")).arg(&junk).status().unwrap().success());
    assert!(update::install(&junk, &installed).unwrap_err().contains("does not contain rgrip.app"));
    assert_eq!(std::fs::read_to_string(installed.join("Contents/MacOS/rgrip")).unwrap(), "new version");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Read-only check against the real GitHub API (network).
#[tokio::test]
#[ignore]
async fn live_check() {
    println!("{:?}", update::check().await);
}
