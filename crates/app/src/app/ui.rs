use super::*;

// ---- small widgets ----------------------------------------------------

pub(super) fn empty_state(ui: &mut Ui, actions: &mut Vec<Action>, no_connections: bool) {
    centered(ui, |ui| {
        ui.label(RichText::new("rgrip").size(28.0).color(theme::FAINT()));
        ui.add_space(6.0);
        if no_connections {
            muted(ui, "Add a PostgreSQL connection to get started.");
            ui.add_space(10.0);
            if ui
                .add(Button::new(RichText::new("New connection").color(theme::BG())).fill(theme::ACCENT()))
                .clicked()
            {
                actions.push(Action::NewConnection);
            }
        } else {
            muted(ui, "Open a connection on the left to start a console.");
            muted(ui, "Double-click a table to browse its data.");
        }
    });
}

pub(super) fn toolbar(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .inner_margin(Margin::symmetric(12, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(24.0);
                add(ui);
            });
        });
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, theme::BORDER()));
}

pub(super) fn segment(ui: &mut Ui, selected: bool, text: &str) -> bool {
    let color = if selected { theme::TEXT() } else { theme::MUTED() };
    let r = ui.add(
        Button::new(RichText::new(text).color(color))
            .fill(if selected { theme::SELECTED() } else { Color32::TRANSPARENT })
            .frame_when_inactive(selected),
    );
    r.clicked()
}

/// A full-width, left-aligned selectable list row (monospace).
pub(super) fn list_row(ui: &mut Ui, text: &str, selected: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 28.0), Sense::click());
    let fill = if selected { theme::SELECTED() } else if resp.hovered() { theme::HOVER() } else { Color32::TRANSPARENT };
    ui.painter().rect_filled(rect, 6.0, fill);
    let color = if selected { theme::TEXT() } else { theme::MUTED() };
    let font = TextStyle::Monospace.resolve(ui.style());
    ui.painter().text(rect.left_center() + egui::vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, text, font, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A 24 pt square title-bar button; `paint` draws its icon (hot = hovered).
pub(super) fn title_button(ui: &mut Ui, hover: &str, paint: impl FnOnce(&Ui, egui::Rect, bool)) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::click());
    let hot = resp.hovered();
    if hot {
        ui.painter().rect_filled(rect, 6.0, theme::HOVER());
    }
    paint(ui, rect, hot);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(hover)
}

/// Small borderless button that brightens on hover.
pub(super) fn icon_button(ui: &mut Ui, text: &str) -> egui::Response {
    let resp = ui.add(
        Button::new(RichText::new(text).size(14.0).color(theme::MUTED()))
            .frame(false)
            .min_size(egui::vec2(18.0, 18.0)),
    );
    if resp.hovered() {
        ui.painter().rect_filled(resp.rect, 4.0, theme::HOVER());
        ui.painter().text(resp.rect.center(), egui::Align2::CENTER_CENTER, text, FontId::proportional(14.0), theme::TEXT());
    }
    resp
}

pub(super) fn ghost_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(Button::new(RichText::new(text).color(theme::MUTED())).frame(false))
}

pub(super) fn centered(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.with_layout(Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() / 2.0 - 60.0).max(0.0));
            add(ui);
        });
    });
}

pub(super) fn hint(text: &str) -> RichText {
    RichText::new(text).color(theme::FAINT())
}

/// `12345` as `12,345`.
pub(super) fn group_thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 { format!("-{out}") } else { out }
}

pub(crate) fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

/// Map in a resizable panel on the right, grid in the rest. Both get the
/// shared grid state: the map runs first so a click on it shows in the grid
/// in the same frame; a grid click shows on the map by repainting.
pub(super) fn split_view(ui: &mut Ui, id: Id, state: &mut GridState, map: impl FnOnce(&mut Ui, &mut GridState), grid: impl FnOnce(&mut Ui, &mut GridState)) {
    let w = ui.available_width();
    egui::SidePanel::right(id.with("map"))
        .resizable(true)
        .default_width(w * 0.5)
        .width_range(240.0..=(w - 240.0).max(240.0))
        .frame(Frame::NONE)
        .show_inside(ui, |ui| map(ui, state));
    let before = state.selection.clone();
    egui::CentralPanel::default().frame(Frame::NONE).show_inside(ui, |ui| grid(ui, state));
    if state.selection != before {
        ui.ctx().request_repaint();
    }
}

pub(super) fn muted(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::MUTED()));
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_grouped() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(12345), "12,345");
        assert_eq!(group_thousands(1234567), "1,234,567");
    }
}
