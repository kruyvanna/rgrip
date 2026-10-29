use super::*;

/// The in-cell text field. Enter (or clicking away) keeps the edit, Esc
/// drops it; either way the editor closes.
pub(super) fn cell_editor(ui: &mut Ui, ed: &mut CellEditor, original: Option<&str>, kind: Option<TemporalKind>) -> EditorOutcome {
    let out = egui::TextEdit::singleline(&mut ed.text)
        .font(TextStyle::Monospace)
        .frame(false)
        .margin(egui::Margin::ZERO)
        .desired_width(f32::INFINITY)
        .show(ui);
    let r = out.response;
    if std::mem::take(&mut ed.focus) {
        r.request_focus();
        // Select everything so typing replaces the value.
        let mut state = out.state;
        let n = ed.text.chars().count();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
        state.store(ui.ctx(), r.id);
    }
    let cell = ui.max_rect();
    ui.painter().rect_stroke(cell.shrink(1.0), 2.0, Stroke::new(1.0_f32, theme::ACCENT()), egui::StrokeKind::Inside);
    // An empty edit of a NULL keeps it NULL.
    let commit = |ed: &CellEditor| {
        let value = if ed.text.is_empty() && original.is_none() { None } else { Some(ed.text.clone()) };
        EditorOutcome::Close(Some(GridEvent::Change { row: ed.row, col: ed.col, value }))
    };
    let Some(kind) = kind else {
        if !r.lost_focus() {
            return EditorOutcome::Editing;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return EditorOutcome::Close(None);
        }
        return commit(ed);
    };
    // Date/time: the picker takes clicks, so losing text focus doesn't end
    // the edit. Enter / Done / a click elsewhere keeps it, Esc drops it.
    let (popup, done) = picker(ui, cell, ed, kind);
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        return EditorOutcome::Close(None);
    }
    let clicked_away = ui.input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !cell.contains(p) && !popup.contains(p))
    });
    if done || clicked_away || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        return commit(ed);
    }
    EditorOutcome::Editing
}

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// Local wall-clock now and the local UTC offset, in seconds.
fn now_local() -> (i64, i64) {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    #[cfg(unix)]
    {
        let t = unix as libc::time_t;
        let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
        // SAFETY: localtime_r writes into `tm` and returns null on failure.
        if !unsafe { libc::localtime_r(&t, tm.as_mut_ptr()) }.is_null() {
            #[allow(clippy::unnecessary_cast)] // `c_long` isn't i64 everywhere
            return (unix, unsafe { tm.assume_init() }.tm_gmtoff as i64);
        }
    }
    (unix, 0)
}

/// Calendar and/or time fields under the cell. Edits `ed.text`; returns the
/// popup's rect and whether Done was clicked.
fn picker(ui: &Ui, cell: egui::Rect, ed: &mut CellEditor, kind: TemporalKind) -> (egui::Rect, bool) {
    let (unix, offset) = now_local();
    let now = dt::from_unix(unix, offset);
    let parsed = dt::parse(kind, &ed.text);
    // Unparseable text (empty, `infinity`, …): start from now.
    let mut t = parsed.clone().unwrap_or_else(|| Temporal { frac: String::new(), ..now.clone() });
    if parsed.is_none() && !kind.has_tz() {
        t.tz.clear();
    }
    let (mut vy, mut vm) = ed.view.unwrap_or((t.year, t.month));
    let mut changed = false;
    let mut done = false;
    let area = egui::Area::new(Id::new(("cell-picker", ed.row, ed.col)))
        .order(egui::Order::Foreground)
        .fixed_pos(cell.left_bottom() + egui::vec2(0.0, 2.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                if kind.has_date() {
                    ui.horizontal(|ui| {
                        if ui.small_button("‹").clicked() {
                            (vy, vm) = if vm == 1 { (vy - 1, 12) } else { (vy, vm - 1) };
                        }
                        let title = format!("{} {vy}", MONTHS[vm as usize - 1]);
                        ui.add_sized([150.0, 18.0], Label::new(RichText::new(title).strong()));
                        if ui.small_button("›").clicked() {
                            (vy, vm) = if vm == 12 { (vy + 1, 1) } else { (vy, vm + 1) };
                        }
                    });
                    egui::Grid::new("days").spacing([2.0, 2.0]).show(ui, |ui| {
                        for d in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] {
                            ui.add_sized([26.0, 16.0], Label::new(RichText::new(d).small().color(theme::FAINT())));
                        }
                        ui.end_row();
                        let lead = dt::weekday(vy, vm, 1);
                        for _ in 0..lead {
                            ui.label("");
                        }
                        for d in 1..=dt::days_in_month(vy, vm) {
                            let selected = parsed.is_some() && (t.year, t.month, t.day) == (vy, vm, d);
                            let today = (now.year, now.month, now.day) == (vy, vm, d);
                            let mut b = egui::Button::new(RichText::new(d.to_string()).size(12.0)).selected(selected).min_size(egui::vec2(26.0, 22.0));
                            if today && !selected {
                                b = b.stroke(Stroke::new(1.0_f32, theme::ACCENT()));
                            }
                            if ui.add(b).clicked() {
                                (t.year, t.month, t.day) = (vy, vm, d);
                                changed = true;
                            }
                            if (lead + d) % 7 == 0 {
                                ui.end_row();
                            }
                        }
                    });
                }
                if kind.has_time() {
                    ui.horizontal(|ui| {
                        let two = |n: f64, _| format!("{:02}", n as u32);
                        changed |= ui.add(egui::DragValue::new(&mut t.hour).range(0..=23).custom_formatter(two)).changed();
                        ui.label(":");
                        changed |= ui.add(egui::DragValue::new(&mut t.minute).range(0..=59).custom_formatter(two)).changed();
                        ui.label(":");
                        changed |= ui.add(egui::DragValue::new(&mut t.second).range(0..=59).custom_formatter(two)).changed();
                        if kind.has_tz() && !t.tz.is_empty() {
                            ui.label(RichText::new(&t.tz).color(theme::MUTED()));
                        }
                    });
                }
                ui.horizontal(|ui| {
                    let label = if kind.has_time() { "Now" } else { "Today" };
                    if ui.button(label).clicked() {
                        t = Temporal { frac: String::new(), ..now.clone() };
                        (vy, vm) = (t.year, t.month);
                        changed = true;
                    }
                    let fill = egui::Button::new(RichText::new("Done").color(theme::BG())).fill(theme::ACCENT());
                    done = ui.add(fill).on_hover_text("Enter").clicked();
                });
            });
        });
    if changed {
        ed.text = dt::format(kind, &t);
    }
    ed.view = Some((vy, vm));
    (area.response.rect, done)
}

pub(super) enum EditorOutcome {
    Editing,
    /// Close the editor, with the change to apply (none when cancelled).
    Close(Option<GridEvent>),
}
