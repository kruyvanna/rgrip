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
    // Menu icons (Lucide, ISC).
    X,
    CopyX,
    ArrowRightToLine,
    CircleX,
    Terminal,
    Link,
    Power,
    Droplet,
    Pencil,
    Trash,
    Table,
    Copy,
    Image,
    Bot,
    Download,
    GitBranch,
    Pin,
    Search,
    Ban,
    Undo,
    FolderOpen,
    Text,
    Settings,
    Lock,
    Upload,
    Map,
    ChevronRight,
    ChevronDown,
    ArrowUp,
    ArrowDown,
    Locate,
}

impl Svg {
    fn source(self) -> std::borrow::Cow<'static, str> {
        match self {
            Svg::GitHub => std::borrow::Cow::Borrowed(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="white" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>"#),
            Svg::Refresh => std::borrow::Cow::Borrowed(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16"/><path d="M16 16h5v5"/></svg>"#),
            Svg::X => lucide(r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#),
            Svg::CopyX => lucide(r#"<line x1="12" x2="18" y1="12" y2="18"/><line x1="12" x2="18" y1="18" y2="12"/><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#),
            Svg::ArrowRightToLine => lucide(r#"<path d="M17 12H3"/><path d="m11 18 6-6-6-6"/><path d="M21 5v14"/>"#),
            Svg::CircleX => lucide(r#"<circle cx="12" cy="12" r="10"/><path d="m15 9-6 6"/><path d="m9 9 6 6"/>"#),
            Svg::Terminal => lucide(r#"<polyline points="4 17 10 11 4 5"/><line x1="12" x2="20" y1="19" y2="19"/>"#),
            Svg::Link => lucide(r#"<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>"#),
            Svg::Power => lucide(r#"<path d="M12 2v10"/><path d="M18.4 6.6a9 9 0 1 1-12.77.04"/>"#),
            Svg::Droplet => lucide(r#"<path d="M12 22a7 7 0 0 0 7-7c0-2-1-3.9-3-5.5s-3.5-4-4-6.5c-.5 2.5-2 4.9-4 6.5C6 11.1 5 13 5 15a7 7 0 0 0 7 7z"/>"#),
            Svg::Pencil => lucide(r#"<path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/><path d="m15 5 4 4"/>"#),
            Svg::Trash => lucide(r#"<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/><line x1="10" x2="10" y1="11" y2="17"/><line x1="14" x2="14" y1="11" y2="17"/>"#),
            Svg::Table => lucide(r#"<path d="M12 3v18"/><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/>"#),
            Svg::Copy => lucide(r#"<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#),
            Svg::Image => lucide(r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#),
            Svg::Bot => lucide(r#"<path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/>"#),
            Svg::Locate => lucide(r#"<line x1="2" x2="5" y1="12" y2="12"/><line x1="19" x2="22" y1="12" y2="12"/><line x1="12" x2="12" y1="2" y2="5"/><line x1="12" x2="12" y1="19" y2="22"/><circle cx="12" cy="12" r="7"/>"#),
            Svg::ChevronRight => lucide(r#"<path d="m9 18 6-6-6-6"/>"#),
            Svg::ChevronDown => lucide(r#"<path d="m6 9 6 6 6-6"/>"#),
            Svg::ArrowUp => lucide(r#"<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>"#),
            Svg::ArrowDown => lucide(r#"<path d="M12 5v14"/><path d="m19 12-7 7-7-7"/>"#),
            Svg::Map => lucide(r#"<path d="M14.106 5.553a2 2 0 0 0 1.788 0l3.659-1.83A1 1 0 0 1 21 4.619v12.764a1 1 0 0 1-.553.894l-4.553 2.277a2 2 0 0 1-1.788 0l-4.212-2.106a2 2 0 0 0-1.788 0l-3.659 1.83A1 1 0 0 1 3 19.381V6.618a1 1 0 0 1 .553-.894l4.553-2.277a2 2 0 0 1 1.788 0z"/><path d="M15 5.764v15"/><path d="M9 3.236v15"/>"#),
            Svg::Download => lucide(r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" x2="12" y1="15" y2="3"/>"#),
            Svg::GitBranch => lucide(r#"<line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/>"#),
            Svg::Pin => lucide(r#"<line x1="12" x2="12" y1="17" y2="22"/><path d="M5 17h14v-1.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1v4.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24Z"/>"#),
            Svg::Search => lucide(r#"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"#),
            Svg::Ban => lucide(r#"<circle cx="12" cy="12" r="10"/><path d="m4.9 4.9 14.2 14.2"/>"#),
            Svg::Undo => lucide(r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>"#),
            Svg::FolderOpen => lucide(r#"<path d="m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2"/>"#),
            Svg::Settings => lucide(r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>"#),
            Svg::Lock => lucide(r#"<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>"#),
            Svg::Upload => lucide(r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" x2="12" y1="3" y2="15"/>"#),
            Svg::Text => lucide(r#"<path d="M17 6.1H3"/><path d="M21 12.1H3"/><path d="M15.1 18H3"/>"#),
            Svg::Heart => std::borrow::Cow::Borrowed(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z"/></svg>"#),
        }
    }
}

/// A Lucide icon body in its standard 24×24 stroked frame.
fn lucide(body: &str) -> std::borrow::Cow<'static, str> {
    std::borrow::Cow::Owned(format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"#
    ))
}

/// White-on-transparent pixels of `svg` at `px` pixels square.
fn rasterize(svg: Svg, px: u32) -> Option<ColorImage> {
    let tree = usvg::Tree::from_str(&svg.source(), &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    let scale = px as f32 / tree.size().width().max(tree.size().height());
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some(ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data()))
}

/// The icon's texture at `size` points, cached per icon and pixel size.
fn texture(ctx: &egui::Context, svg: Svg, size: f32) -> Option<TextureHandle> {
    let px = (size * ctx.pixels_per_point()).round().max(1.0) as u32;
    let id = egui::Id::new(("svg-icon", svg, px));
    ctx.data_mut(|d| d.get_temp::<TextureHandle>(id)).or_else(|| {
        let image = rasterize(svg, px)?;
        let texture = ctx.load_texture(format!("icon-{px}"), image, TextureOptions::LINEAR);
        ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
        Some(texture)
    })
}

/// The icon as an egui image (for buttons and menu items), tinted.
pub fn image(ctx: &egui::Context, svg: Svg, size: f32, tint: Color32) -> egui::Image<'static> {
    match texture(ctx, svg, size) {
        Some(t) => egui::Image::new((t.id(), egui::vec2(size, size))).tint(tint),
        None => egui::Image::new((egui::TextureId::default(), egui::Vec2::ZERO)),
    }
}

/// A menu item with an icon. Disabled items fade their icon too.
pub fn menu_item(ui: &mut Ui, svg: Svg, label: impl Into<egui::WidgetText>, enabled: bool) -> egui::Response {
    let tint = if enabled { crate::theme::MUTED() } else { crate::theme::FAINT().gamma_multiply(0.6) };
    ui.add_enabled(enabled, egui::Button::image_and_text(image(ui.ctx(), svg, 15.0, tint), label))
}

/// A destructive menu item: icon and label in the error color.
pub fn danger_item(ui: &mut Ui, svg: Svg, label: &str) -> egui::Response {
    let red = crate::theme::ERROR();
    ui.add(egui::Button::image_and_text(image(ui.ctx(), svg, 15.0, red), egui::RichText::new(label).color(red)))
}

/// A submenu button with an icon.
pub fn submenu<R>(ui: &mut Ui, svg: Svg, label: &str, add: impl FnOnce(&mut Ui) -> R) -> egui::Response {
    let img = image(ui.ctx(), svg, 15.0, crate::theme::MUTED());
    ui.menu_image_text_button(img, label, add).response
}

/// Draw `svg` in `rect`, tinted. Textures are cached per icon and pixel size.
pub fn paint(ui: &Ui, svg: Svg, rect: Rect, tint: Color32) {
    if let Some(t) = texture(ui.ctx(), svg, rect.width().max(rect.height())) {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(t.id(), rect, uv, tint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_rasterize() {
        for svg in [Svg::GitHub, Svg::Refresh, Svg::Heart, Svg::X, Svg::CopyX, Svg::ArrowRightToLine, Svg::CircleX, Svg::Terminal, Svg::Link, Svg::Power, Svg::Droplet, Svg::Pencil, Svg::Trash, Svg::Table, Svg::Copy, Svg::Image, Svg::Bot, Svg::Download, Svg::GitBranch, Svg::Pin, Svg::Search, Svg::Ban, Svg::Undo, Svg::FolderOpen, Svg::Text, Svg::Settings, Svg::Lock] {
            let img = rasterize(svg, 32).expect("valid svg");
            assert!(img.pixels.iter().any(|p| p.a() > 200), "drew something");
        }
    }
}
