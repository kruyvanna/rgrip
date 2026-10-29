//! The "Query history" dialog: one connection's past console runs, newest
//! first, with a search box, the full SQL of the selected run, and actions
//! to reopen or copy it. The data and its rules live in `rgrip_core::history`.

use crate::theme;
use eframe::egui::{self, Align, Button, Frame, Id, Layout, Margin, RichText, Sense, Stroke, TextEdit, Ui, vec2};
use rgrip_core::history::{self, Entry};

const WIDTH: f32 = 820.0;
const ROW_H: f32 = 24.0;

pub enum Outcome {
    Keep,
    Close,
    /// A new console with this entry's SQL, on its database.
    Open(Entry),
    /// Forget this connection's history (already confirmed).
    Clear,
}

pub struct HistoryView {
    pub conn: String,
    name: String,
    entries: Vec<Entry>,
    query: String,
    /// Index into `entries` (stays put while the search narrows the list).
    selected: Option<usize>,
    confirm_clear: bool,
    focus: bool,
}

impl HistoryView {
    pub fn new(conn: String, name: String) -> Self {
        let entries = history::load(&conn);
        Self { conn, name, entries, query: String::new(), selected: None, confirm_clear: false, focus: true }
    }

    pub fn cleared(&mut self) {
        self.entries.clear();
        self.selected = None;
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        let mut out = Outcome::Keep;
        let modal = egui::Modal::new(Id::new("query-history"))
            .frame(Frame::new().fill(theme::SURFACE()).stroke(Stroke::new(1.0_f32, theme::BORDER())).corner_radius(12).inner_margin(Margin::same(20)))
            .show(ctx, |ui| {
                ui.set_width(WIDTH.min(ctx.content_rect().width() - 80.0));
                ui.label(RichText::new(format!("Query history · {}", self.name)).heading().color(theme::TEXT()));
                ui.add_space(8.0);
                let search = ui.add(TextEdit::singleline(&mut self.query).hint_text("Search SQL").desired_width(f32::INFINITY));
                if std::mem::take(&mut self.focus) {
                    search.request_focus();
                }
                ui.add_space(8.0);
                let shown = history::search(&self.entries, &self.query);
                let mut pick = None;
                egui::ScrollArea::vertical().id_salt("history-list").max_height(280.0).auto_shrink([false, true]).show(ui, |ui| {
                    if shown.is_empty() {
                        let text = if self.entries.is_empty() { "Nothing here yet. Statements you run in a console are kept." } else { "No run matches" };
                        ui.add_space(8.0);
                        ui.label(RichText::new(text).color(theme::FAINT()));
                    }
                    let now = history::now();
                    for e in &shown {
                        let i = self.entries.iter().position(|x| std::ptr::eq(x, *e)).unwrap_or_default();
                        if row(ui, e, now, self.selected == Some(i)).clicked() {
                            pick = Some(i);
                        }
                    }
                });
                if pick.is_some() {
                    self.selected = pick;
                }
                ui.add_space(8.0);
                let current = self.selected.and_then(|i| self.entries.get(i));
                egui::ScrollArea::vertical().id_salt("history-sql").max_height(170.0).min_scrolled_height(90.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match current {
                        Some(e) => {
                            ui.add(egui::Label::new(RichText::new(&e.sql).monospace().size(12.5).color(theme::TEXT())).selectable(true));
                        }
                        None => {
                            ui.label(RichText::new("Pick a run to see its SQL").color(theme::FAINT()));
                        }
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if self.confirm_clear {
                        ui.label(RichText::new("Delete all history of this connection?").color(theme::WARN()));
                        if ui.add(Button::new(RichText::new("Clear").color(theme::BG())).fill(theme::ERROR())).clicked() {
                            out = Outcome::Clear;
                            self.confirm_clear = false;
                        }
                        if ui.button("Keep").clicked() {
                            self.confirm_clear = false;
                        }
                    } else if ui.add_enabled(!self.entries.is_empty(), Button::new("Clear history…")).clicked() {
                        self.confirm_clear = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            out = Outcome::Close;
                        }
                        if ui.add_enabled(current.is_some(), Button::new(RichText::new("Open in console").color(theme::BG())).fill(theme::ACCENT())).clicked()
                            && let Some(e) = current
                        {
                            out = Outcome::Open(e.clone());
                        }
                        if ui.add_enabled(current.is_some(), Button::new("Copy")).clicked()
                            && let Some(e) = current
                        {
                            ui.ctx().copy_text(e.sql.clone());
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

/// One run: when, how long, rows or an error mark, the first line of SQL.
fn row(ui: &mut Ui, e: &Entry, now: u64, selected: bool) -> egui::Response {
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
    at(8.0, ago(now, e.at), theme::MUTED());
    at(88.0, duration(e.millis), theme::MUTED());
    if e.error {
        at(160.0, "error".into(), theme::ERROR());
    } else if let Some(n) = e.rows {
        at(160.0, format!("{n} row{}", if n == 1 { "" } else { "s" }), theme::MUTED());
    }
    let first = e.sql.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    p.text(egui::pos2(rect.left() + 240.0, rect.center().y), egui::Align2::LEFT_CENTER, first, egui::FontId::monospace(12.0), theme::TEXT());
    resp
}

fn ago(now: u64, at: u64) -> String {
    let s = now.saturating_sub(at);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86399 => format!("{}h ago", s / 3600),
        86400..=1_209_599 => format!("{}d ago", s / 86400),
        1_209_600..=5_183_999 => format!("{}w ago", s / 604800),
        _ => format!("{}mo ago", s / 2_592_000),
    }
}

fn duration(ms: u64) -> String {
    match ms {
        0..=999 => format!("{ms} ms"),
        1000..=59_999 => format!("{:.1} s", ms as f64 / 1000.0),
        _ => format!("{}m {}s", ms / 60_000, ms % 60_000 / 1000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_time_and_duration() {
        assert_eq!(ago(100, 100), "just now");
        assert_eq!(ago(1000, 100), "15m ago");
        assert_eq!(ago(100_000, 100), "1d ago");
        assert_eq!(duration(12), "12 ms");
        assert_eq!(duration(1500), "1.5 s");
        assert_eq!(duration(125_000), "2m 5s");
    }
}
