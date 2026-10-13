//! A quiet, low-contrast dark theme with a single accent color.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
    Theme, Visuals, vec2,
};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(22, 22, 24);
pub const SURFACE: Color32 = Color32::from_rgb(27, 27, 30);
pub const BORDER: Color32 = Color32::from_rgb(36, 36, 40);
pub const HOVER: Color32 = Color32::from_rgb(34, 34, 38);
pub const POPUP: Color32 = Color32::from_rgb(30, 30, 34);
pub const VIEW: Color32 = Color32::from_rgb(187, 154, 247);
pub const TEXT: Color32 = Color32::from_rgb(212, 212, 216);
pub const MUTED: Color32 = Color32::from_rgb(122, 122, 130);
pub const FAINT: Color32 = Color32::from_rgb(80, 80, 88);
pub const ACCENT: Color32 = Color32::from_rgb(122, 162, 247);
pub const OK: Color32 = Color32::from_rgb(158, 206, 106);
pub const WARN: Color32 = Color32::from_rgb(224, 175, 104);
pub const ERROR: Color32 = Color32::from_rgb(247, 118, 142);
pub const STRING: Color32 = Color32::from_rgb(169, 199, 126);
/// Geometry values and map features.
pub const GEO: Color32 = Color32::from_rgb(115, 218, 202);

/// Prefer native macOS fonts (and JetBrains Mono if installed), keeping
/// egui's bundled fonts as fallbacks for symbols.
fn install_fonts(ctx: &egui::Context) {
    let home = std::env::var("HOME").unwrap_or_default();
    let ui_fonts = ["/System/Library/Fonts/SFNS.ttf".to_string()];
    let mono_fonts = [
        format!("{home}/Library/Fonts/JetBrainsMonoNLNerdFont-Regular.ttf"),
        format!("{home}/Library/Fonts/JetBrainsMono-Regular.ttf"),
        "/System/Library/Fonts/SFNSMono.ttf".to_string(),
    ];

    let mut fonts = FontDefinitions::default();
    let mut add = |family: FontFamily, paths: &[String], name: &str| {
        if let Some(bytes) = paths.iter().find_map(|p| std::fs::read(p).ok()) {
            fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
            fonts.families.entry(family).or_default().insert(0, name.into());
        }
    };
    add(FontFamily::Proportional, &ui_fonts, "ui");
    add(FontFamily::Monospace, &mono_fonts, "mono");

    // Khmer has no glyphs in SF / JetBrains Mono: fall back to a Khmer font
    // right after the primary one, for both families.
    let khmer_fonts = [
        format!("{home}/Library/Fonts/KantumruyPro-VariableFont_wght.ttf"),
        "/Library/Fonts/KantumruyPro-VariableFont_wght.ttf".to_string(),
        "/System/Library/Fonts/Supplemental/Khmer Sangam MN.ttf".to_string(),
    ];
    if let Some(bytes) = khmer_fonts.iter().find_map(|p| std::fs::read(p).ok()) {
        crate::shaping::init(bytes.clone());
        fonts.font_data.insert("khmer".into(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let list = fonts.families.entry(family).or_default();
            list.insert(1.min(list.len()), "khmer".into());
        }
    }
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        let mut v = Visuals::dark();
        v.override_text_color = Some(TEXT);
        v.weak_text_color = Some(FAINT);
        v.panel_fill = BG;
        v.window_fill = SURFACE;
        v.extreme_bg_color = Color32::from_rgb(33, 33, 37);
        v.faint_bg_color = Color32::from_rgb(25, 25, 28);
        v.code_bg_color = SURFACE;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        v.hyperlink_color = ACCENT;
        v.error_fg_color = ERROR;
        v.warn_fg_color = WARN;
        v.selection.bg_fill = ACCENT.gamma_multiply(0.28);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.text_cursor.stroke = Stroke::new(1.5, ACCENT);

        let r = CornerRadius::same(5);
        let w = &mut v.widgets;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = r;
        for (state, fill) in [
            (&mut w.inactive, Color32::from_rgb(34, 34, 38)),
            (&mut w.hovered, Color32::from_rgb(44, 44, 50)),
            (&mut w.active, Color32::from_rgb(52, 52, 58)),
            (&mut w.open, Color32::from_rgb(40, 40, 45)),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::NONE;
            state.corner_radius = r;
            state.expansion = 0.0;
        }
        w.hovered.bg_stroke = Stroke::new(1.0, BORDER);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        w.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        style.visuals = v;

        style.spacing.item_spacing = vec2(8.0, 5.0);
        style.spacing.button_padding = vec2(9.0, 4.0);
        style.spacing.interact_size.y = 24.0;
        style.spacing.indent = 14.0;
        style.spacing.scroll.floating = true;
        style.spacing.scroll.bar_width = 8.0;

        style.text_styles = [
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(18.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        ]
        .into();
    });
}
