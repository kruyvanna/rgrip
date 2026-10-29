//! The Activity tab: the server's sessions from `pg_stat_activity`, who blocks
//! whom, and cancel / terminate for the selected one. It polls on its own
//! while it is the visible tab and stops after an error until you refresh.

use crate::icons::Svg;
use crate::theme;
use eframe::egui::{self, Color32, Margin, RichText, Sense, TextEdit, Ui};
use egui_extras::{Column, TableBuilder};
use rgrip_core::activity::{self, Flags, Session};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Auto-refresh intervals offered (seconds).
const INTERVALS: [u64; 4] = [2, 3, 5, 10];
const ROW_H: f32 = 22.0;

/// What the tab asks the app to do.
#[derive(Default)]
pub struct Outcome {
    pub refresh: bool,
    /// `(pid, terminate)`: ask first, then signal that backend.
    pub signal: Option<(i64, bool)>,
}

pub struct ActivityTab {
    sessions: Vec<Session>,
    flags: Vec<Flags>,
    /// Sessions each pid holds up.
    blocks: HashMap<i64, usize>,
    show_idle: bool,
    filter: String,
    auto: bool,
    interval: u64,
    /// By pid, so it survives a refresh.
    selected: Option<i64>,
    /// A request is out (or waiting for the connection).
    pub loading: bool,
    /// Waiting for the connection before the request goes out.
    pub pending: bool,
    error: Option<String>,
    updated: Option<Instant>,
    last_request: Option<Instant>,
    /// Refresh on the next frame (after a cancel / terminate).
    refresh_soon: bool,
    /// Result of the last cancel / terminate.
    status: Option<(String, bool)>,
}

impl ActivityTab {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            flags: Vec::new(),
            blocks: HashMap::new(),
            show_idle: false,
            filter: String::new(),
            auto: true,
            interval: 3,
            selected: None,
            loading: false,
            pending: false,
            error: None,
            updated: None,
            last_request: None,
            refresh_soon: false,
            status: None,
        }
    }

    /// A refresh was sent (or queued for the connection).
    pub fn requested(&mut self) {
        self.loading = true;
        self.last_request = Some(Instant::now());
        self.refresh_soon = false;
    }

    pub fn loaded(&mut self, result: Result<Vec<Session>, String>) {
        self.loading = false;
        self.pending = false;
        match result {
            Ok(sessions) => {
                self.flags = activity::classify(&sessions);
                self.blocks.clear();
                for p in sessions.iter().flat_map(|s| s.blocked_by.iter()) {
                    *self.blocks.entry(*p).or_default() += 1;
                }
                self.sessions = sessions;
                self.error = None;
                self.updated = Some(Instant::now());
            }
            // Keep the last rows on screen; auto-refresh pauses until Refresh.
            Err(e) => self.error = Some(e),
        }
    }

    pub fn failed(&mut self, error: String) {
        self.loading = false;
        self.pending = false;
        self.error = Some(error);
    }

    /// The outcome of a cancel / terminate: shown in the status line.
    pub fn signalled(&mut self, pid: i64, terminate: bool, result: Result<bool, String>) {
        let what = if terminate { "Terminate session" } else { "Cancel query" };
        self.status = Some(match result {
            Ok(true) => (format!("{what} {pid}: done (the server returned true)"), true),
            Ok(false) => (format!("{what} {pid}: the server returned false (the session is gone, or isn't yours to signal)"), false),
            Err(e) => (format!("{what} {pid}: {e}"), false),
        });
        self.refresh_soon = true;
    }

    /// The user and query of a session, for the confirmation.
    pub fn describe(&self, pid: i64) -> Option<(&str, &str)> {
        self.sessions.iter().find(|s| s.pid == pid).map(|s| (s.user.as_str(), s.query.as_str()))
    }

    pub fn ui(&mut self, ui: &mut Ui, read_only: bool) -> Outcome {
        let mut out = Outcome::default();
        // Polling: first load, after a signal, and on the interval; never while
        // a request is out or after an error (that needs a click on Refresh).
        let interval = Duration::from_secs(self.interval);
        let due = match self.last_request {
            None => true,
            Some(t) => self.refresh_soon || (self.auto && t.elapsed() >= interval),
        };
        if !self.loading && (due && self.error.is_none() || self.refresh_soon) {
            out.refresh = true;
        }
        if self.auto && self.error.is_none() {
            ui.ctx().request_repaint_after(interval);
        }

        egui::Frame::new().inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!self.loading, egui::Button::new("Refresh")).clicked() {
                    self.error = None;
                    out.refresh = true;
                }
                ui.checkbox(&mut self.show_idle, "Show idle");
                ui.add(TextEdit::singleline(&mut self.filter).hint_text("Filter sessions").desired_width(180.0));
                ui.checkbox(&mut self.auto, "Auto-refresh");
                egui::ComboBox::from_id_salt("activity-interval").selected_text(format!("{} s", self.interval)).width(60.0).show_ui(ui, |ui| {
                    for s in INTERVALS {
                        ui.selectable_value(&mut self.interval, s, format!("{s} s"));
                    }
                });
                ui.separator();
                let selected = self.selected.filter(|p| self.sessions.iter().any(|s| s.pid == *p));
                let no_row = if selected.is_none() { "Select a session first" } else { "" };
                let hint = |what: &str| if read_only { "This connection is read-only".to_string() } else if no_row.is_empty() { what.to_string() } else { no_row.to_string() };
                let can = selected.is_some() && !read_only;
                if ui.add_enabled(can, egui::Button::new("Cancel query")).on_hover_text(hint("Stop the selected session's running query (pg_cancel_backend)")).on_disabled_hover_text(hint("")).clicked() {
                    out.signal = selected.map(|p| (p, false));
                }
                let terminate = egui::Button::new(RichText::new("Terminate session").color(if can { theme::ERROR() } else { theme::FAINT() }));
                if ui.add_enabled(can, terminate).on_hover_text(hint("Disconnect the selected session and roll back its transaction (pg_terminate_backend)")).on_disabled_hover_text(hint("")).clicked() {
                    out.signal = selected.map(|p| (p, true));
                }
                if ui.add_enabled(selected.is_some(), egui::Button::new("Copy query")).clicked()
                    && let Some(s) = self.sessions.iter().find(|s| Some(s.pid) == selected)
                {
                    ui.ctx().copy_text(s.query.clone());
                }
            });
        });
        let rect = ui.available_rect_before_wrap();
        ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0_f32, theme::BORDER()));

        // Status line (bottom), so the table takes what is left.
        egui::TopBottomPanel::bottom(ui.id().with("activity-status")).frame(egui::Frame::new().inner_margin(Margin::symmetric(10, 4))).show_separator_line(true).show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if let Some(e) = &self.error {
                    ui.colored_label(theme::ERROR(), format!("{e}  (auto-refresh paused, press Refresh to retry)"));
                } else if let Some((msg, ok)) = &self.status {
                    ui.colored_label(if *ok { theme::OK() } else { theme::WARN() }, msg);
                }
                let blocked = self.flags.iter().filter(|f| f.blocked).count();
                let shown = activity::visible(&self.sessions, &self.flags, self.show_idle, &self.filter).len();
                let mut text = format!("{shown} of {} sessions", self.sessions.len());
                if blocked > 0 {
                    text += &format!(" · {blocked} blocked");
                }
                if let Some(t) = self.updated {
                    text += &format!(" · updated {}s ago", t.elapsed().as_secs());
                }
                if self.loading {
                    text += " · loading…";
                }
                ui.label(RichText::new(text).color(theme::MUTED()));
            });
        });

        if self.sessions.is_empty() && self.error.is_none() {
            ui.centered_and_justified(|ui| {
                if self.loading || self.updated.is_none() { ui.spinner(); } else { ui.label(RichText::new("No other sessions").color(theme::MUTED())); }
            });
            return out;
        }
        let rows = activity::visible(&self.sessions, &self.flags, self.show_idle, &self.filter);
        self.table(ui, &rows, read_only, &mut out);
        out
    }

    fn table(&mut self, ui: &mut Ui, rows: &[usize], read_only: bool, out: &mut Outcome) {
        let mut select = None;
        let mut copy = None;
        let mut signal = None;
        egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let header = |text: &str| RichText::new(text).strong().color(theme::MUTED());
            TableBuilder::new(ui)
                .striped(false)
                .sense(Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::initial(60.0).at_least(40.0).resizable(true))
                .column(Column::initial(90.0).at_least(40.0).resizable(true))
                .column(Column::initial(100.0).at_least(40.0).resizable(true))
                .column(Column::initial(110.0).at_least(40.0).resizable(true))
                .column(Column::initial(100.0).at_least(40.0).resizable(true))
                .column(Column::initial(130.0).at_least(40.0).resizable(true))
                .column(Column::initial(130.0).at_least(40.0).resizable(true))
                .column(Column::initial(70.0).at_least(40.0).resizable(true))
                .column(Column::initial(70.0).at_least(40.0).resizable(true))
                .column(Column::initial(120.0).at_least(40.0).resizable(true))
                .column(Column::initial(110.0).at_least(40.0).resizable(true))
                .column(Column::remainder().at_least(300.0).clip(true))
                .header(24.0, |mut h| {
                    for t in ["PID", "User", "Database", "Application", "Client", "State", "Wait", "Running", "Xact", "Blocking", "Type", "Query"] {
                        h.col(|ui| {
                            ui.add_space(6.0);
                            ui.label(header(t));
                        });
                    }
                })
                .body(|body| {
                    body.rows(ROW_H, rows.len(), |mut row| {
                        let i = rows[row.index()];
                        let (s, f) = (&self.sessions[i], self.flags[i]);
                        let selected = self.selected == Some(s.pid);
                        row.set_selected(selected);
                        let tint = if f.blocked { Some(theme::ERROR()) } else if f.blocking || f.idle_in_tx || f.long_running { Some(theme::WARN()) } else { None };
                        let strength = if f.blocked || f.blocking { 0.2 } else { 0.1 };
                        let text = |ui: &mut Ui, t: &str, color: Color32| {
                            ui.add_space(6.0);
                            ui.add(egui::Label::new(RichText::new(t).color(color)).truncate());
                        };
                        let cell = |row: &mut egui_extras::TableRow<'_, '_>, t: &str, color: Color32| {
                            row.col(|ui| {
                                if let Some(c) = tint && !selected {
                                    ui.painter().rect_filled(ui.max_rect().expand2(egui::vec2(0.0, 1.0)), 0.0, theme::tint(theme::SURFACE(), c, strength));
                                }
                                text(ui, t, color);
                            });
                        };
                        let (text_c, muted) = (theme::TEXT(), theme::MUTED());
                        cell(&mut row, &s.pid.to_string(), text_c);
                        cell(&mut row, &s.user, text_c);
                        cell(&mut row, &s.database, text_c);
                        cell(&mut row, &s.application, muted);
                        cell(&mut row, &s.client, muted);
                        let state_color = if f.idle_in_tx { theme::WARN() } else if s.is_active() { theme::OK() } else { muted };
                        cell(&mut row, if s.state.is_empty() { "-" } else { &s.state }, state_color);
                        cell(&mut row, &s.wait(), muted);
                        cell(&mut row, &activity::format_duration(s.running_secs()), if f.long_running { theme::WARN() } else { text_c });
                        cell(&mut row, &activity::format_duration(s.xact_secs), muted);
                        let (blocking, blocking_color) = if f.blocked {
                            (format!("blocked by {}", s.blocked_by.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")), theme::ERROR())
                        } else if f.blocking {
                            (format!("blocking {}", self.blocks.get(&s.pid).copied().unwrap_or(0)), theme::WARN())
                        } else {
                            (String::new(), muted)
                        };
                        cell(&mut row, &blocking, blocking_color);
                        cell(&mut row, &s.backend_type, muted);
                        // One line of the query; the whole text is on hover.
                        let one_line = s.query.split_whitespace().collect::<Vec<_>>().join(" ");
                        row.col(|ui| {
                            if let Some(c) = tint && !selected {
                                ui.painter().rect_filled(ui.max_rect().expand2(egui::vec2(0.0, 1.0)), 0.0, theme::tint(theme::SURFACE(), c, strength));
                            }
                            ui.add_space(6.0);
                            let l = ui.add(egui::Label::new(RichText::new(&one_line).monospace().size(12.0).color(theme::TEXT())).truncate());
                            if !s.query.is_empty() {
                                l.on_hover_text(RichText::new(s.query.chars().take(2000).collect::<String>()).monospace().size(12.0));
                            }
                        });
                        let resp = row.response();
                        if resp.clicked() || resp.secondary_clicked() {
                            select = Some(s.pid);
                        }
                        resp.context_menu(|ui| {
                            if crate::icons::menu_item(ui, Svg::Ban, "Cancel query", !read_only).clicked() {
                                signal = Some((s.pid, false));
                                ui.close();
                            }
                            if crate::icons::menu_item(ui, Svg::Power, "Terminate session", !read_only).clicked() {
                                signal = Some((s.pid, true));
                                ui.close();
                            }
                            ui.separator();
                            if crate::icons::menu_item(ui, Svg::Copy, "Copy query", !s.query.is_empty()).clicked() {
                                copy = Some(s.query.clone());
                                ui.close();
                            }
                            if crate::icons::menu_item(ui, Svg::Copy, "Copy PID", true).clicked() {
                                copy = Some(s.pid.to_string());
                                ui.close();
                            }
                        });
                    });
                });
        });
        if let Some(p) = select {
            self.selected = Some(p);
        }
        if let Some(t) = copy {
            ui.ctx().copy_text(t);
        }
        if signal.is_some() {
            out.signal = signal;
        }
    }
}
