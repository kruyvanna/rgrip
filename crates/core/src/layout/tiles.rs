use super::*;

/// Fetch basemap tiles (blocking; call off the UI thread). Tiles are cached
/// on disk, so re-renders and re-exports don't fetch them again; a tile that
/// fails is simply left out.
/// With `network` off, only what's on disk (instant, for a first preview).
pub fn fetch_tiles(basemap: Basemap, z: u8, tiles: &[(u32, u32)], network: bool) -> std::collections::HashMap<(u32, u32), Vec<u8>> {
    let dir = std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join("Library/Caches/rgrip/layout-tiles").join(basemap.label()).join(z.to_string()));
    let mut out = std::collections::HashMap::new();
    let mut missing = Vec::new();
    for &(x, y) in tiles {
        match dir.as_ref().and_then(|d| std::fs::read(d.join(format!("{x}_{y}"))).ok()) {
            Some(bytes) => {
                out.insert((x, y), bytes);
            }
            None => missing.push((x, y)),
        }
    }
    if missing.is_empty() || !network {
        return out;
    }
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return out };
    let Ok(client) = reqwest::Client::builder()
        .user_agent(concat!("rgrip/", env!("CARGO_PKG_VERSION"), " (desktop PostgreSQL client; map layout)"))
        .timeout(std::time::Duration::from_secs(20))
        .build()
    else {
        return out;
    };
    let fetched = rt.block_on(async {
        use futures_util::StreamExt;
        futures_util::stream::iter(missing.into_iter().filter_map(|(x, y)| basemap.tile_url(z, x, y).map(|u| (x, y, u))))
            .map(|(x, y, url)| {
                let client = client.clone();
                async move {
                    let resp = client.get(url).send().await.ok()?;
                    if !resp.status().is_success() {
                        return None;
                    }
                    Some(((x, y), resp.bytes().await.ok()?.to_vec()))
                }
            })
            // A few at a time: polite to the tile servers.
            .buffer_unordered(6)
            .collect::<Vec<_>>()
            .await
    });
    for ((x, y), bytes) in fetched.into_iter().flatten() {
        if let Some(d) = &dir
            && std::fs::create_dir_all(d).is_ok()
        {
            let _ = std::fs::write(d.join(format!("{x}_{y}")), &bytes);
        }
        out.insert((x, y), bytes);
    }
    out
}
