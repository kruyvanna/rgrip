//! Virtualized result grid.
//!
//! Only visible rows are laid out, and everything derived from the whole
//! result (column widths, geometry columns, geometry summaries) is computed
//! once per result set, so large pages with very long values stay smooth.

use crate::theme;
use eframe::egui::{self, Align, Id, Label, Layout, RichText, Sense, Stroke, TextStyle, Ui};
use egui_extras::{Column, TableBuilder};
use rgrip_core::ResultSet;
use std::collections::HashMap;

/// Characters of a value laid out in a cell; a cell shows one line anyway.
const CELL_CHARS: usize = 240;
/// Characters shown in a hover tooltip.
const HOVER_CHARS: usize = 2000;

#[derive(Default)]
pub struct GridState {
    pub selected: Option<usize>,
    cache: Option<Cache>,
}

/// Per-result-set derived data.
struct Cache {
    key: (usize, usize, usize),
    widths: Vec<f32>,
    geom_cols: Vec<usize>,
    /// (row, col) → geometry summary and WKT preview.
    geoms: HashMap<(usize, usize), (String, String)>,
}

pub enum GridEvent {
    HeaderClicked(usize),
}

pub struct Sort<'a> {
    pub column: &'a str,
    pub ascending: bool,
}

const CHAR_W: f32 = 7.6;
const PAD: f32 = 10.0;

/// The first `max` chars of `s` on one line (newlines shown as ↵).
fn cell_text(s: &str, max: usize) -> String {
    let cut = s.char_indices().nth(max).map_or(s.len(), |(i, _)| i);
    let head = &s[..cut];
    let mut out = if head.contains('\n') { head.replace('\n', "↵ ") } else { head.to_owned() };
    if cut < s.len() {
        out.push('…');
    }
    out
}

fn column_widths(rs: &ResultSet) -> Vec<f32> {
    rs.columns
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let widest = rs
                .rows
                .iter()
                .take(100)
                // Bounded scan: long values are clamped to the max width anyway.
                .map(|r| r[i].as_deref().map_or(4, |s| s.chars().take(60).take_while(|c| *c != '\n').count()))
                .max()
                .unwrap_or(0)
                .max(name.chars().count() + 2);
            (widest as f32 * CHAR_W + 2.0 * PAD + 8.0).clamp(64.0, 360.0)
        })
        .collect()
}

impl GridState {
    fn cache(&mut self, rs: &ResultSet) -> &mut Cache {
        let key = (rs as *const ResultSet as usize, rs.rows.len(), rs.columns.len());
        if self.cache.as_ref().is_none_or(|c| c.key != key) {
            self.cache = Some(Cache {
                key,
                widths: column_widths(rs),
                geom_cols: rs.geometry_columns().to_vec(),
                geoms: HashMap::new(),
            });
        }
        self.cache.as_mut().unwrap()
    }
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
    let gutter_w = (((row_offset + rs.rows.len()).max(1) as f32).log10().floor() + 1.0) * CHAR_W + 22.0;

    // Tight cells with our own padding, and barely-there column dividers.
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    // Resize lines only show on hover; header cells draw their own dividers.
    ui.visuals_mut().widgets.noninteractive.bg_stroke = Stroke::NONE;

    copy_selection_on_cmd_c(ui, rs, state);
    let selected = state.selected;
    let mut clicked_row = None;
    let cache = state.cache(rs);

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
            for w in &cache.widths {
                table = table.column(Column::initial(*w).at_least(40.0).clip(true));
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
                        row.set_selected(selected == Some(i));
                        row.col(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(8.0);
                                ui.label(RichText::new((row_offset + i + 1).to_string()).size(11.0).color(theme::FAINT));
                            });
                        });
                        for (col, value) in rs.rows[i].iter().enumerate() {
                            row.col(|ui| {
                                ui.add_space(PAD);
                                cell(ui, cache, i, col, value.as_deref());
                            });
                        }
                        let response = row.response();
                        if response.clicked() {
                            clicked_row = Some(i);
                        }
                        response.context_menu(|ui| {
                            clicked_row = Some(i);
                            if ui.button("Copy row").clicked() {
                                ui.ctx().copy_text(row_tsv(&rs.rows[i]));
                                ui.close();
                            }
                            if ui.button("Copy row with header").clicked() {
                                ui.ctx().copy_text(format!("{}\n{}", rs.columns.join("\t"), row_tsv(&rs.rows[i])));
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
    if clicked_row.is_some() {
        state.selected = clicked_row;
    }
    event
}

fn cell(ui: &mut Ui, cache: &mut Cache, row: usize, col: usize, value: Option<&str>) {
    let Some(s) = value else {
        ui.add(Label::new(RichText::new("NULL").italics().color(theme::FAINT)).selectable(false));
        return;
    };
    if cache.geom_cols.contains(&col) {
        // Hex EWKB is unreadable: show a summary, WKT on hover.
        let (summary, wkt) = cache.geoms.entry((row, col)).or_insert_with(|| match rgrip_core::geo::parse_hex(s) {
            Some(g) => (g.summary(), cell_text(&g.geometry.to_wkt(), HOVER_CHARS)),
            None => (cell_text(s, CELL_CHARS), String::new()),
        });
        let r = ui.add(Label::new(RichText::new(summary.as_str()).monospace().color(theme::GEO)).truncate().selectable(false));
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
        crate::shaping::label(ui, &text, 13.5, theme::TEXT)
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

#[cfg(test)]
mod tests {
    use super::cell_text;

    #[test]
    fn cell_text_is_bounded() {
        assert_eq!(cell_text("a\nb", 10), "a↵ b");
        assert_eq!(cell_text("abcdef", 3), "abc…");
        assert_eq!(cell_text("ភ្នំពេញ", 3).chars().count(), 4);
        assert_eq!(cell_text(&"x".repeat(100_000), 240).chars().count(), 241);
    }
}

#[cfg(test)]
mod perf {
    //! `cargo test --release -p rgrip grid_frame_time -- --ignored --nocapture`
    use super::*;
    use std::time::Instant;

    /// A page shaped like a real PostGIS table: long coordinate text,
    /// Khmer HTML, NULLs and a geometry column.
    fn heavy_page() -> ResultSet {
        let coords: String = (0..1500).map(|i| format!("104.91{i:05},11.61{i:05},0 ")).collect();
        let khmer = "<div>ផ្លូវកោះពេជ្រ&nbsp;<span>សង្កាត់ទន្លេបាសាក់ ខណ្ឌចំការមន រាជធានីភ្នំពេញ</span></div>".repeat(20);
        let geom = "0101000020E61000008F537424973B5A40AA8251499D202740".to_string();
        let rows = (0..1000)
            .map(|i| {
                vec![
                    Some(i.to_string()),
                    Some(format!("Untitled polygon {i}")),
                    if i % 3 == 0 { Some(khmer.clone()) } else { None },
                    Some(coords.clone()),
                    Some("Polygon".into()),
                    Some(geom.clone()),
                ]
            })
            .collect();
        ResultSet {
            columns: ["id", "name", "description", "coordinates", "geometry_type", "geom"].map(String::from).to_vec(),
            rows,
            ..Default::default()
        }
    }

    #[test]
    #[ignore]
    fn grid_frame_time() {
        let rs = heavy_page();
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(2.0);
        crate::theme::apply(&ctx);
        let mut state = GridState::default();
        let mut frame = |scroll: f32| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                events: vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, scroll),
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            };
            let start = Instant::now();
            let out = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, egui::Id::new("perf"), &rs, &mut state, None, 0);
                });
            });
            // Tessellation runs every frame in the real app too.
            let _ = ctx.tessellate(out.shapes, 2.0);
            start.elapsed()
        };
        for _ in 0..5 {
            frame(0.0);
        }
        let n = 60;
        let total: std::time::Duration = (0..n).map(|i| frame(if i % 2 == 0 { -40.0 } else { 0.0 })).sum();
        println!("grid frame: {:.2} ms avg over {n} frames", total.as_secs_f64() * 1000.0 / n as f64);
    }
}
