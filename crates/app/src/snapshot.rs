//! Render the SVG table export to a PNG for the clipboard, so it pastes
//! as a picture in chat apps.

use resvg::{tiny_skia, usvg};
use std::sync::{Arc, OnceLock};

/// Keeps very long tables from allocating hundreds of MB.
const MAX_PIXELS: f32 = 40_000_000.0;

/// System fonts, loaded once (Khmer and other scripts come from here).
fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

/// First face that has the char, skipping macOS's LastResort font, which
/// "has" every char but draws it as a box (usvg's default stops there).
fn fallback(c: char, exclude: &[usvg::fontdb::ID], db: &mut Arc<usvg::fontdb::Database>) -> Option<usvg::fontdb::ID> {
    let has_char = |id| {
        db.with_face_data(id, |data, index| {
            rustybuzz::Face::from_slice(data, index).and_then(|f| f.glyph_index(c)).is_some()
        })
        .unwrap_or(false)
    };
    db.faces()
        .filter(|f| !exclude.contains(&f.id) && !f.families.iter().any(|(name, _)| name.contains("LastResort")))
        .find(|f| has_char(f.id))
        .map(|f| f.id)
}

fn options() -> usvg::Options<'static> {
    usvg::Options {
        fontdb: fonts(),
        font_resolver: usvg::FontResolver {
            select_font: usvg::FontResolver::default_font_selector(),
            select_fallback: Box::new(fallback),
        },
        ..Default::default()
    }
}

/// Rasterise at `scale` × the SVG's CSS-pixel size (96 per inch).
pub fn render_at(svg: &str, scale: f32) -> Option<tiny_skia::Pixmap> {
    let tree = usvg::Tree::from_str(svg, &options()).ok()?;
    let size = tree.size();
    let mut pixmap = tiny_skia::Pixmap::new((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some(pixmap)
}

/// Rasterise at 2× (less for huge tables) for a crisp image on HiDPI screens.
pub fn render(svg: &str) -> Option<tiny_skia::Pixmap> {
    let tree = usvg::Tree::from_str(svg, &options()).ok()?;
    let size = tree.size();
    let scale = 2.0_f32.min((MAX_PIXELS / (size.width() * size.height())).sqrt());
    let mut pixmap = tiny_skia::Pixmap::new((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some(pixmap)
}

/// Put the rows on the clipboard as a PNG of the SVG table. Only the PNG:
/// chat apps (Telegram…) take an SVG flavour as an `.svg` file attachment.
pub fn copy_table(ctx: &eframe::egui::Context, rs: &rgrip_core::ResultSet, table: &str) {
    let _ = table;
    let Some(pixmap) = render(&rgrip_core::export::svg_for_raster(rs)) else { return };
    #[cfg(target_os = "macos")]
    if let Ok(png) = pixmap.encode_png() {
        let _ = ctx;
        copy_png_macos(&png);
    }
    #[cfg(not(target_os = "macos"))]
    ctx.copy_image(eframe::egui::ColorImage::from_rgba_premultiplied(
        [pixmap.width() as usize, pixmap.height() as usize],
        pixmap.data(),
    ));
}

/// The "Copy as image" PNG, as bytes (for screenshot scenes).
#[cfg(debug_assertions)]
pub fn table_png(rs: &rgrip_core::ResultSet) -> Option<Vec<u8>> {
    render(&rgrip_core::export::svg_for_raster(rs))?.encode_png().ok()
}

/// `public.png` directly (egui's copy goes through TIFF).
#[cfg(target_os = "macos")]
fn copy_png_macos(png: &[u8]) {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSData, NSString};
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    pb.setData_forType(Some(&NSData::with_bytes(png)), &NSString::from_str("public.png"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rgrip_core::ResultSet;

    /// Writes the real clipboard: `cargo test -p rgrip clipboard -- --ignored`.
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn clipboard_has_png_only() {
        use objc2_app_kit::NSPasteboard;
        let rs = ResultSet {
            columns: vec!["id".into()],
            rows: vec![vec![Some("1".into())]],
            ..Default::default()
        };
        copy_table(&eframe::egui::Context::default(), &rs, "t");
        let types = NSPasteboard::generalPasteboard().types().unwrap();
        let types: Vec<String> = types.iter().map(|t| t.to_string()).collect();
        assert!(types.iter().any(|x| x == "public.png"), "{types:?}");
        assert!(!types.iter().any(|x| x.contains("svg") || x.contains("text")), "{types:?}");
    }

    #[test]
    fn renders_the_svg_table() {
        let s = |v: &str| Some(v.to_string());
        let rs = ResultSet {
            columns: vec!["id".into(), "name".into()],
            rows: vec![
                vec![s("1"), s("Untitled polygon")],
                vec![s("2"), s("ផ្សារធំថ្មី")],
                vec![s("3"), None],
                vec![s("4"), s("28ម៉ែត្រការ៉េរឹទ្ធាឃឹង")],
                vec![s("5"), s("<div>ផ្លូវទំនប់កប់ស្រូវ សង្កាត់ព្រែកព្នៅ ខណ្ឌព្រែកព្នៅ</div>")],
                vec![s("6"), s("សង្កាត់ទន្លេបាសាក់ ខណ្ឌចំការមន រាជធានីភ្នំពេញ")],
            ],
            ..Default::default()
        };
        let pixmap = render(&rgrip_core::export::svg_for_raster(&rs)).unwrap();
        assert!(pixmap.width() > 200 && pixmap.height() > 200);
        // Text was drawn: some pixels are dark.
        assert!(pixmap.pixels().iter().any(|p| p.red() < 80 && p.alpha() == 255));
        if let Ok(path) = std::env::var("RGRIP_PNG_SAMPLE") {
            std::fs::write(path, pixmap.encode_png().unwrap()).unwrap();
        }
    }
}
