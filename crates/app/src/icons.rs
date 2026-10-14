//! Small SVG icons (brand marks and line icons that are fiddly to paint by
//! hand), rasterised once per size with resvg and drawn tinted.

use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, TextureOptions, Ui};
use resvg::{tiny_skia, usvg};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Svg {
    /// The GitHub mark (Octicons, MIT).
    GitHub,
    /// Lucide "refresh-cw" (ISC).
    Refresh,
    /// Lucide "heart" (ISC).
    Heart,
}

impl Svg {
    fn source(self) -> &'static str {
        match self {
            Svg::GitHub => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="white" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>"#,
            Svg::Refresh => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16"/><path d="M16 16h5v5"/></svg>"#,
            Svg::Heart => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z"/></svg>"#,
        }
    }
}

/// White-on-transparent pixels of `svg` at `px` pixels square.
fn rasterize(svg: Svg, px: u32) -> Option<ColorImage> {
    let tree = usvg::Tree::from_str(svg.source(), &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    let scale = px as f32 / tree.size().width().max(tree.size().height());
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some(ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data()))
}

/// Draw `svg` in `rect`, tinted. Textures are cached per icon and pixel size.
pub fn paint(ui: &Ui, svg: Svg, rect: Rect, tint: Color32) {
    let px = (rect.width().max(rect.height()) * ui.ctx().pixels_per_point()).round().max(1.0) as u32;
    let id = egui::Id::new(("svg-icon", svg, px));
    let texture = ui.ctx().data_mut(|d| d.get_temp::<TextureHandle>(id)).or_else(|| {
        let image = rasterize(svg, px)?;
        let texture = ui.ctx().load_texture(format!("icon-{px}"), image, TextureOptions::LINEAR);
        ui.ctx().data_mut(|d| d.insert_temp(id, texture.clone()));
        Some(texture)
    });
    if let Some(t) = texture {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(t.id(), rect, uv, tint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_rasterize() {
        for svg in [Svg::GitHub, Svg::Refresh, Svg::Heart] {
            let img = rasterize(svg, 32).expect("valid svg");
            assert!(img.pixels.iter().any(|p| p.a() > 200), "drew something");
        }
    }
}
