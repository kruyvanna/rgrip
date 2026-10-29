use super::*;

/// A small filled triangle: up/down when sorted, both when `asc` is None.
pub(super) fn paint_sort(p: &egui::Painter, c: egui::Pos2, asc: Option<bool>, color: egui::Color32) {
    let tri = |c: egui::Pos2, up: bool, s: f32| {
        let d = if up { -1.0 } else { 1.0 };
        p.add(egui::Shape::convex_polygon(
            vec![c + egui::vec2(-s, -d * s * 0.6), c + egui::vec2(s, -d * s * 0.6), c + egui::vec2(0.0, d * s * 0.8)],
            color,
            Stroke::NONE,
        ));
    };
    match asc {
        Some(a) => tri(c, a, 4.0),
        None => {
            tri(c + egui::vec2(0.0, -3.5), true, 3.0);
            tri(c + egui::vec2(0.0, 3.5), false, 3.0);
        }
    }
}

pub(super) fn cell(ui: &mut Ui, cache: &mut Cache, row: usize, col: usize, value: Option<&str>) {
    let Some(s) = value else {
        ui.add(Label::new(RichText::new("NULL").italics().color(theme::FAINT())).selectable(false));
        return;
    };
    if cache.geom_cols.contains(&col) {
        // Hex EWKB is unreadable: show a summary, WKT on hover.
        let (summary, wkt) = cache.geoms.entry((row, col)).or_insert_with(|| match rgrip_core::geo::parse_hex(s) {
            Some(g) => (g.summary(), cell_text(&g.geometry.to_wkt(), HOVER_CHARS)),
            None => (cell_text(s, CELL_CHARS), String::new()),
        });
        let r = ui.add(Label::new(RichText::new(summary.as_str()).monospace().color(theme::GEO())).truncate().selectable(false));
        if !wkt.is_empty() {
            let wkt = wkt.clone();
            r.on_hover_ui(|ui| {
                ui.label(RichText::new(wkt).monospace().size(11.5));
            });
        }
        return;
    }
    let text = cell_text(s, CELL_CHARS);
    let r = if crate::shaping::is_complex(&text) {
        // Khmer needs real shaping, which egui's text layout lacks.
        crate::shaping::label(ui, &text, 13.5, theme::TEXT())
    } else {
        ui.add(Label::new(RichText::new(text).monospace()).truncate().selectable(false))
    };
    if s.len() > 40 || s.contains('\n') {
        // Built only while hovered.
        r.on_hover_ui(|ui| {
            ui.set_max_width(520.0);
            ui.label(cell_text(s, HOVER_CHARS));
        });
    }
}

pub(super) fn header_bg(ui: &mut Ui) {
    let r = ui.max_rect();
    let p = ui.painter();
    p.rect_filled(r, 0.0, theme::BG());
    p.hline(r.x_range(), r.bottom(), Stroke::new(1.0_f32, theme::BORDER()));
    p.vline(r.right(), r.y_range().shrink(7.0), Stroke::new(1.0_f32, theme::BORDER()));
}
