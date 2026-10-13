//! Virtualized result grid.

use crate::theme;
use eframe::egui::{self, Align, Id, Label, Layout, RichText, Sense, Stroke, TextStyle, Ui};
use egui_extras::{Column, TableBuilder};
use rgrip_core::ResultSet;

#[derive(Default)]
pub struct GridState {
    pub selected: Option<usize>,
}

pub enum GridEvent {
    HeaderClicked(usize),
}

pub struct Sort<'a> {
    pub column: &'a str,
    pub ascending: bool,
}

/// `row_offset` is added to the row-number gutter (for paged views).
pub fn show(
    ui: &mut Ui,
    id: Id,
    rs: &ResultSet,
    state: &mut GridState,
    sort: Option<Sort<'_>>,
    row_offset: usize,
) -> Option<GridEvent> {
    let mut event = None;
    let row_h = ui.text_style_height(&TextStyle::Monospace) + 10.0;
    let char_w = 7.6;
    let gutter_w = (((row_offset + rs.rows.len()).max(1) as f32).log10().floor() + 1.0) * char_w + 22.0;
    const PAD: f32 = 10.0;

    // Tight cells with our own padding, and barely-there column dividers.
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    // Resize lines only show on hover; header cells draw their own dividers.
    ui.visuals_mut().widgets.noninteractive.bg_stroke = Stroke::NONE;

    copy_selection_on_cmd_c(ui, rs, state);
    let geom_cols = rgrip_core::geo::geometry_columns(rs);

    egui::ScrollArea::horizontal()
        .id_salt(id.with("h"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut table = TableBuilder::new(ui)
                .id_salt(id)
                .striped(true)
                .resizable(true)
                .sense(Sense::click())
                .auto_shrink([false, false])
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::exact(gutter_w));
            for (i, name) in rs.columns.iter().enumerate() {
                let widest = rs
                    .rows
                    .iter()
                    .take(100)
                    .map(|r| r[i].as_deref().map_or(4, |s| s.lines().next().unwrap_or("").chars().count()))
                    .max()
                    .unwrap_or(0)
                    .max(name.chars().count() + 2);
                let w = (widest as f32 * char_w + 2.0 * PAD + 8.0).clamp(64.0, 360.0);
                table = table.column(Column::initial(w).at_least(40.0).clip(true));
            }

            table
                .header(row_h + 4.0, |mut header| {
                    header.col(|ui| header_bg(ui));
                    for (i, name) in rs.columns.iter().enumerate() {
                        header.col(|ui| {
                            header_bg(ui);
                            ui.add_space(PAD);
                            let mut text = name.clone();
                            if let Some(s) = sort.as_ref().filter(|s| s.column == name) {
                                text += if s.ascending { "  ↑" } else { "  ↓" };
                            }
                            let r = ui.add(
                                Label::new(RichText::new(text).size(12.5).color(theme::TEXT))
                                    .truncate()
                                    .selectable(false)
                                    .sense(Sense::click()),
                            );
                            if r.clicked() {
                                event = Some(GridEvent::HeaderClicked(i));
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_h, rs.rows.len(), |mut row| {
                        let i = row.index();
                        row.set_selected(state.selected == Some(i));
                        row.col(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new((row_offset + i + 1).to_string())
                                        .size(11.0)
                                        .color(theme::FAINT),
                                );
                            });
                        });
                        for (col, value) in rs.rows[i].iter().enumerate() {
                            let is_geom = geom_cols.contains(&col);
                            row.col(|ui| {
                                ui.add_space(PAD);
                                let r = match value {
                                    None => ui.add(
                                        Label::new(RichText::new("NULL").italics().color(theme::FAINT)).selectable(false),
                                    ),
                                    // Hex EWKB is unreadable: show a summary, WKT on hover.
                                    Some(s) if is_geom => {
                                        let g = rgrip_core::geo::parse_hex(s);
                                        let text = g.as_ref().map_or_else(|| s.clone(), |g| g.summary());
                                        let r = ui.add(
                                            Label::new(RichText::new(text).monospace().color(theme::GEO)).truncate().selectable(false),
                                        );
                                        match g {
                                            Some(g) => r.on_hover_text(truncate(&g.geometry.to_wkt(), 1500)),
                                            None => r,
                                        }
                                    }
                                    // Khmer needs real shaping, which egui's text layout lacks.
                                    Some(s) if crate::shaping::is_complex(s) => {
                                        crate::shaping::label(ui, &single_line(s), 13.5, theme::TEXT)
                                    }
                                    Some(s) => ui.add(
                                        Label::new(RichText::new(single_line(s)).monospace()).truncate().selectable(false),
                                    ),
                                };
                                if let Some(s) = value.as_ref().filter(|_| !is_geom) {
                                    if s.len() > 40 || s.contains('\n') {
                                        r.on_hover_text(truncate(s, 2000));
                                    }
                                }
                            });
                        }
                        let response = row.response();
                        if response.clicked() {
                            state.selected = Some(i);
                        }
                        response.context_menu(|ui| {
                            state.selected = Some(i);
                            if ui.button("Copy row").clicked() {
                                ui.ctx().copy_text(row_tsv(&rs.rows[i]));
                                ui.close();
                            }
                            if ui.button("Copy row with header").clicked() {
                                ui.ctx()
                                    .copy_text(format!("{}\n{}", rs.columns.join("\t"), row_tsv(&rs.rows[i])));
                                ui.close();
                            }
                            if ui.button("Copy all as TSV").clicked() {
                                ui.ctx().copy_text(all_tsv(rs));
                                ui.close();
                            }
                        });
                    });
                });
        });
    event
}

/// ⌘C copies the selected row, unless a text field has focus.
fn copy_selection_on_cmd_c(ui: &Ui, rs: &ResultSet, state: &GridState) {
    let Some(i) = state.selected.filter(|&i| i < rs.rows.len()) else {
        return;
    };
    if ui.ctx().memory(|m| m.focused().is_some()) {
        return;
    }
    if ui.input(|inp| inp.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
        ui.ctx().copy_text(row_tsv(&rs.rows[i]));
    }
}

fn single_line(s: &str) -> String {
    if s.contains('\n') {
        s.replace('\n', "↵ ")
    } else {
        s.to_owned()
    }
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

fn row_tsv(row: &[Option<String>]) -> String {
    row.iter()
        .map(|v| v.as_deref().unwrap_or("NULL"))
        .collect::<Vec<_>>()
        .join("\t")
}

fn all_tsv(rs: &ResultSet) -> String {
    let mut out = rs.columns.join("\t");
    for row in &rs.rows {
        out.push('\n');
        out += &row_tsv(row);
    }
    out
}

fn header_bg(ui: &mut Ui) {
    let r = ui.max_rect();
    let p = ui.painter();
    p.rect_filled(r, 0.0, theme::BG);
    p.hline(r.x_range(), r.bottom(), Stroke::new(1.0, theme::BORDER));
    p.vline(r.right(), r.y_range().shrink(7.0), Stroke::new(1.0, theme::BORDER));
}
