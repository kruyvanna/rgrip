use super::*;

/// The value popup: the whole value, JSON indented and colored, selectable.
pub(super) fn viewer_ui(ctx: &egui::Context, viewer: &mut Option<Viewer>) {
    let Some(v) = viewer.as_ref() else { return };
    let mut close = false;
    let screen = ctx.content_rect();
    let modal = egui::Modal::new(Id::new("grid-value-viewer"))
        .frame(egui::Frame::new().fill(theme::SURFACE()).stroke(Stroke::new(1.0_f32, theme::BORDER())).corner_radius(12).inner_margin(egui::Margin::same(16)))
        .show(ctx, |ui| {
            ui.set_width((screen.width() * 0.6).clamp(420.0, 900.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new(&v.column).strong().color(theme::TEXT()));
                ui.label(RichText::new(format!("row {}{}", v.row, if v.json.is_some() { " · JSON" } else { "" })).size(12.0).color(theme::MUTED()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                    if v.json.is_some() && ui.button("Copy compact").on_hover_text("The value as stored, on one line").clicked() {
                        ui.ctx().copy_text(v.text.clone());
                    }
                    if crate::icons::menu_item(ui, crate::icons::Svg::Copy, "Copy", true).clicked() {
                        ui.ctx().copy_text(v.json.clone().unwrap_or_else(|| v.text.clone()));
                    }
                });
            });
            ui.add_space(8.0);
            egui::Frame::new().fill(theme::BG()).corner_radius(8).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                egui::ScrollArea::both().max_height((screen.height() * 0.7).max(200.0)).auto_shrink([false, true]).show(ui, |ui| {
                    let font = TextStyle::Monospace.resolve(ui.style());
                    // Coloring is per frame: skip it for huge values.
                    let job = match v.json.as_ref().filter(|j| j.len() < 200_000) {
                        Some(j) => crate::highlight::json(j, font),
                        None => {
                            let mut job = egui::text::LayoutJob::simple(v.json.clone().unwrap_or_else(|| v.text.clone()), font, theme::TEXT(), ui.available_width());
                            job.wrap.max_width = ui.available_width();
                            job
                        }
                    };
                    ui.add(Label::new(job).selectable(true));
                });
            });
        });
    if close || modal.should_close() {
        *viewer = None;
    }
}
