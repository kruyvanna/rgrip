use super::*;

/// Vertical offset that shows `row` a couple of rows below the top edge.
pub(super) fn reveal_offset(row: usize, row_h: f32) -> f32 {
    (row as f32 - 2.0).max(0.0) * row_h
}

/// Scroll the scrolling half so `col` is near its left edge, and flash it.
pub(super) fn jump(state: &mut GridState, col: usize, rest: &[usize], widths: &[f32], now: f64) {
    if let Some(p) = rest.iter().position(|&c| c == col) {
        state.scroll_x = Some(rest[..p].iter().map(|&c| widths[c]).sum());
    }
    state.flash = Some((col, now));
}

/// The "Go to column" popup. Returns the chosen column.
pub(super) fn goto_popup(ui: &mut Ui, id: Id, rs: &ResultSet, state: &mut GridState, anchor: egui::Rect) -> Option<usize> {
    let ctx = ui.ctx().clone();
    let g = state.goto.as_mut()?;
    // Keys first, so the text field doesn't swallow them.
    let (up, down, enter, esc) = ctx.input_mut(|i| {
        let n = egui::Modifiers::NONE;
        (i.consume_key(n, egui::Key::ArrowUp), i.consume_key(n, egui::Key::ArrowDown), i.consume_key(n, egui::Key::Enter), i.consume_key(n, egui::Key::Escape))
    });
    if esc {
        state.goto = None;
        return None;
    }
    let mut chosen = None;
    let mut moved = false;
    let area = egui::Area::new(id.with("goto"))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.left_bottom() + egui::vec2(4.0, 2.0))
        .show(&ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(260.0);
                let te = ui.add(egui::TextEdit::singleline(&mut g.query).hint_text("Search column…").desired_width(f32::INFINITY));
                if g.focus {
                    te.request_focus();
                    g.focus = false;
                }
                let q = g.query.to_lowercase();
                let matches: Vec<usize> = (0..rs.columns.len()).filter(|&c| q.is_empty() || rs.columns[c].to_lowercase().contains(&q)).collect();
                if te.changed() {
                    g.highlight = 0;
                }
                if down && g.highlight + 1 < matches.len() {
                    g.highlight += 1;
                    moved = true;
                }
                if up && g.highlight > 0 {
                    g.highlight -= 1;
                    moved = true;
                }
                g.highlight = g.highlight.min(matches.len().saturating_sub(1));
                if enter {
                    chosen = matches.get(g.highlight).copied();
                }
                ui.add_space(4.0);
                if matches.is_empty() {
                    ui.label(RichText::new("No matching columns").size(12.0).color(theme::MUTED()));
                }
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (k, &c) in matches.iter().enumerate() {
                        let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), Sense::click());
                        if k == g.highlight {
                            ui.painter().rect_filled(rect, 4.0, theme::ACCENT().gamma_multiply(0.18));
                            if moved {
                                ui.scroll_to_rect(rect, None);
                            }
                        } else if r.hovered() {
                            ui.painter().rect_filled(rect, 4.0, theme::HOVER());
                        }
                        let p = ui.painter().with_clip_rect(rect.with_max_x(rect.right() - 40.0));
                        p.text(rect.left_center() + egui::vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, &rs.columns[c], egui::FontId::proportional(12.5), theme::TEXT());
                        ui.painter().text(rect.right_center() - egui::vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, format!("#{}", c + 1), egui::FontId::proportional(11.0), theme::MUTED());
                        if r.clicked() {
                            chosen = Some(c);
                        }
                    }
                });
            });
        });
    // A press outside the popup dismisses it.
    let outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.pointer_interact_pos().is_some_and(|p| !area.response.rect.contains(p));
    if chosen.is_some() || outside {
        state.goto = None;
    }
    chosen
}
