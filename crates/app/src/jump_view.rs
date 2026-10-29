//! The jump list dialog (⌘K): tabs the user bookmarked, with a search box and
//! the SQL of the selected console. The data and its rules live in `rgrip_core::jumps`.

use crate::theme;
use eframe::egui::{self, Align, Button, Frame, Id, Key, Layout, Margin, Modifiers, RichText, Sense, Stroke, TextEdit, Ui, vec2};
use rgrip_core::jumps::{JumpItem, JumpState};

const WIDTH: f32 = 720.0;
const ROW_H: f32 = 30.0;

pub enum Outcome {
    Keep,
    Close,
    Open(String),
    Remove(String),
    /// Bookmark the tab that is active behind the dialog.
    AddCurrent,
}

pub struct JumpView {
    query: String,
    /// Position in the filtered list.
    selected: usize,
    focus: bool,
}

impl JumpView {
    pub fn new() -> Self {
        Self { query: String::new(), selected: 0, focus: true }
    }

    /// `conn_name` turns a connection id into what the user calls it; `can_add` is whether the active tab can be bookmarked.
    pub fn ui(&mut self, ctx: &egui::Context, items: &[JumpItem], conn_name: &dyn Fn(&str) -> String, can_add: bool) -> Outcome {
        let mut out = Outcome::Keep;
        let shown: Vec<&JumpItem> = items.iter().filter(|i| matches(i, &self.query, conn_name)).collect();
        self.selected = self.selected.min(shown.len().saturating_sub(1));
        // Before the search field sees them, or it keeps Up / Down / Enter.
        let (up, down, enter) = ctx.input_mut(|i| {
            (i.consume_key(Modifiers::NONE, Key::ArrowUp), i.consume_key(Modifiers::NONE, Key::ArrowDown), i.consume_key(Modifiers::NONE, Key::Enter))
        });
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        if down && self.selected + 1 < shown.len() {
            self.selected += 1;
        }
        if enter && let Some(i) = shown.get(self.selected) {
            out = Outcome::Open(i.id.clone());
        }
        let modal = egui::Modal::new(Id::new("jump-list"))
            .frame(Frame::new().fill(theme::SURFACE()).stroke(Stroke::new(1.0_f32, theme::BORDER())).corner_radius(12).inner_margin(Margin::same(20)))
            .show(ctx, |ui| {
                ui.set_width(WIDTH.min(ctx.content_rect().width() - 80.0));
                ui.label(RichText::new("Jump list").heading().color(theme::TEXT()));
                ui.add_space(8.0);
                let search = ui.add(TextEdit::singleline(&mut self.query).hint_text("Search saved tabs").desired_width(f32::INFINITY));
                if std::mem::take(&mut self.focus) {
                    search.request_focus();
                }
                if search.changed() {
                    self.selected = 0;
                }
                ui.add_space(8.0);
                egui::ScrollArea::vertical().id_salt("jump-list-rows").max_height(280.0).auto_shrink([false, true]).show(ui, |ui| {
                    if shown.is_empty() {
                        let text = if items.is_empty() { "Nothing saved yet. Add a tab with ⌘⇧K or its right-click menu." } else { "No saved tab matches" };
                        ui.add_space(8.0);
                        ui.label(RichText::new(text).color(theme::FAINT()));
                    }
                    for (n, item) in shown.iter().enumerate() {
                        let (resp, remove) = row(ui, item, &conn_name(&item.conn), n == self.selected);
                        if resp.clicked() {
                            out = Outcome::Open(item.id.clone());
                        }
                        if remove {
                            out = Outcome::Remove(item.id.clone());
                        }
                        if n == self.selected && (up || down) {
                            resp.scroll_to_me(None);
                        }
                    }
                });
                if let Some(JumpState::Console { sql, .. }) = shown.get(self.selected).map(|i| &i.state) {
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().id_salt("jump-list-sql").max_height(130.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(egui::Label::new(RichText::new(sql).monospace().size(12.5).color(theme::TEXT())).selectable(true));
                    });
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(can_add, Button::new("Add current tab")).on_hover_text("⌘⇧K").clicked() {
                        out = Outcome::AddCurrent;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            out = Outcome::Close;
                        }
                    });
                });
            });
        if modal.should_close() && matches!(out, Outcome::Keep) {
            out = Outcome::Close;
        }
        out
    }
}

fn kind_label(s: &JumpState) -> &'static str {
    match s {
        JumpState::Console { .. } => "Console",
        JumpState::Table { .. } => "Table",
        JumpState::Diagram { .. } => "Diagram",
        JumpState::Activity { .. } => "Activity",
    }
}

/// The first non-empty line of a console's SQL, or the table's filter.
fn detail(s: &JumpState) -> String {
    match s {
        JumpState::Console { sql, .. } => sql.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string(),
        JumpState::Table { filter, .. } => filter.clone(),
        JumpState::Diagram { filter, .. } => filter.clone(),
        JumpState::Activity { filter, .. } => filter.clone(),
    }
}

fn matches(i: &JumpItem, query: &str, conn_name: &dyn Fn(&str) -> String) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let sql = if let JumpState::Console { sql, .. } = &i.state { sql.as_str() } else { "" };
    [i.title.as_str(), sql, &detail(&i.state), &conn_name(&i.conn), kind_label(&i.state)].iter().any(|h| h.to_lowercase().contains(&q))
}

/// One bookmark: kind, title, connection, a one-line detail; ✕ on the right removes it.
fn row(ui: &mut Ui, item: &JumpItem, conn: &str, selected: bool) -> (egui::Response, bool) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let p = ui.painter().with_clip_rect(rect);
    if selected {
        p.rect_filled(rect, 4.0, theme::ACCENT().gamma_multiply(0.25));
    } else if resp.hovered() {
        p.rect_filled(rect, 4.0, theme::BORDER().gamma_multiply(0.5));
    }
    let font = egui::FontId::proportional(12.5);
    let at = |x: f32, text: String, color| {
        p.text(egui::pos2(rect.left() + x, rect.center().y), egui::Align2::LEFT_CENTER, text, font.clone(), color);
    };
    at(8.0, kind_label(&item.state).to_string(), theme::MUTED());
    at(80.0, item.title.clone(), theme::TEXT());
    at(260.0, conn.to_string(), theme::MUTED());
    at(380.0, detail(&item.state), theme::FAINT());
    let x = egui::Rect::from_center_size(egui::pos2(rect.right() - 16.0, rect.center().y), vec2(20.0, 20.0));
    let remove = ui.interact(x, Id::new(("jump-remove", &item.id)), Sense::click());
    if resp.hovered() || remove.hovered() || selected {
        p.text(x.center(), egui::Align2::CENTER_CENTER, "✕", font, if remove.hovered() { theme::ERROR() } else { theme::MUTED() });
    }
    (resp, remove.clicked())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(sql: &str) -> JumpItem {
        JumpItem::new("t".into(), "c".into(), JumpState::Console { sql: sql.into(), cursor_start: 0, cursor_end: 0, database: None, schema: None, more_schemas: vec![] })
    }

    #[test]
    fn search_covers_sql_and_connection() {
        let name = |_: &str| "Prod".to_string();
        let i = item("\nselect * from orders");
        assert!(matches(&i, "ORDERS", &name));
        assert!(matches(&i, "prod", &name));
        assert!(!matches(&i, "users", &name));
        assert_eq!(detail(&i.state), "select * from orders");
    }

    #[test]
    fn keys_pick_and_escape_closes() {
        let ctx = egui::Context::default();
        let items = vec![item("a"), item("b")];
        let name = |_: &str| String::new();
        let mut view = JumpView::new();
        let mut run = |events: Vec<egui::Event>| {
            let mut out = Outcome::Keep;
            let input = egui::RawInput { events, ..Default::default() };
            let _ = ctx.run(input, |ctx| out = view.ui(ctx, &items, &name, true));
            out
        };
        let key = |k| egui::Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: Modifiers::NONE };
        assert!(matches!(run(vec![key(Key::ArrowDown)]), Outcome::Keep));
        match run(vec![key(Key::Enter)]) {
            Outcome::Open(id) => assert_eq!(id, items[1].id),
            _ => panic!("expected Open"),
        }
        assert!(matches!(run(vec![key(Key::Escape)]), Outcome::Close));
    }
}
