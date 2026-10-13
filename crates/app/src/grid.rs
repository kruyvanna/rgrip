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
    /// Selected row indexes (click, ⇧-click range, ⌘-click toggle, ⌘A).
    pub selection: std::collections::BTreeSet<usize>,
    /// Where a ⇧-click range starts.
    anchor: Option<usize>,
    cache: Option<Cache>,
}

/// Where the rows came from, for "Copy as SQL" / "Copy for AI agent".
pub struct Source<'a> {
    pub table: &'a str,
    pub sql: Option<&'a str>,
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
    source: &Source<'_>,
) -> Option<GridEvent> {
    let mut event = None;
    let row_h = ui.text_style_height(&TextStyle::Monospace) + 10.0;
    let gutter_w = (((row_offset + rs.rows.len()).max(1) as f32).log10().floor() + 1.0) * CHAR_W + 22.0;

    // Tight cells with our own padding, and barely-there column dividers.
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    // Resize lines only show on hover; header cells draw their own dividers.
    ui.visuals_mut().widgets.noninteractive.bg_stroke = Stroke::NONE;

    keyboard(ui, rs, state);
    let selection = state.selection.clone();
    let mut clicked_row = None;
    let mut menu_row = None;
    let geom = !rs.geometry_columns().is_empty();
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
                                Label::new(RichText::new(text).size(12.5).color(theme::TEXT()))
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
                        row.set_selected(selection.contains(&i));
                        row.col(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(8.0);
                                ui.label(RichText::new((row_offset + i + 1).to_string()).size(11.0).color(theme::FAINT()));
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
                        if response.secondary_clicked() {
                            menu_row = Some(i);
                        }
                        response.context_menu(|ui| {
                            // The menu acts on the selection (or just this row).
                            let rows: Vec<usize> = if selection.contains(&i) { selection.iter().copied().collect() } else { vec![i] };
                            rows_menu(ui, rs, &rows, source, geom);
                        });
                    });
                });
        });
    if let Some(i) = clicked_row {
        let mods = ui.input(|inp| inp.modifiers);
        match state.anchor.filter(|_| mods.shift) {
            Some(a) => {
                state.selection = (a.min(i)..=a.max(i)).collect();
            }
            None if mods.command => {
                if !state.selection.remove(&i) {
                    state.selection.insert(i);
                }
                state.anchor = Some(i);
            }
            None => {
                state.selection = [i].into();
                state.anchor = Some(i);
            }
        }
    }
    if let Some(i) = menu_row.filter(|i| !state.selection.contains(i)) {
        state.selection = [i].into();
        state.anchor = Some(i);
    }
    event
}

/// Copy / export actions for the given rows.
fn rows_menu(ui: &mut Ui, rs: &ResultSet, rows: &[usize], source: &Source<'_>, geom: bool) {
    use rgrip_core::export::{self, Format};
    let n = rows.len();
    let subset = || export::subset(rs, rows);
    let noun = if n == 1 { "row".to_string() } else { format!("{n} rows") };
    if ui.button(format!("Copy {noun}")).on_hover_text("Tab-separated  ⌘C").clicked() {
        ui.ctx().copy_text(rows_tsv(rs, rows, false));
        ui.close();
    }
    if ui.button("Copy with header").clicked() {
        ui.ctx().copy_text(rows_tsv(rs, rows, true));
        ui.close();
    }
    ui.menu_button("Copy as", |ui| {
        let item = |ui: &mut Ui, label: &str, text: &dyn Fn() -> String| {
            if ui.button(label).clicked() {
                ui.ctx().copy_text(text());
                ui.close();
            }
        };
        item(ui, "CSV", &|| export::export(&subset(), Format::Csv, source.table).trim_start_matches('\u{FEFF}').to_string());
        item(ui, "JSON", &|| export::export(&subset(), Format::Json, source.table));
        item(ui, "Markdown", &|| export::export(&subset(), Format::Markdown, source.table));
        if geom {
            item(ui, "GeoJSON", &|| export::export(&subset(), Format::GeoJson, source.table));
        }
        ui.separator();
        item(ui, "SQL INSERT", &|| export::export(&subset(), Format::SqlInsert, source.table));
        item(ui, "SQL  select … where id in (…)", &|| export::where_in(&subset(), source.table));
    });
    if ui
        .button("Copy for AI agent")
        .on_hover_text("Markdown with the query, column types and rows: paste into an agent")
        .clicked()
    {
        ui.ctx().copy_text(export::for_agent(&subset(), source.table, source.sql));
        ui.close();
    }
    ui.separator();
    ui.menu_button(format!("Save {noun} as"), |ui| {
        for f in Format::ALL.into_iter().filter(|f| *f != Format::GeoJson || geom) {
            if ui.button(format!("{}…", f.label())).clicked() {
                ui.close();
                let name = source.table.rsplit('.').next().unwrap_or("rows");
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name(format!("{name}.{}", f.extension()))
                    .add_filter(f.label(), &[f.extension()])
                    .save_file()
                {
                    let _ = std::fs::write(path, export::export(&subset(), f, source.table));
                }
            }
        }
    });
}

fn rows_tsv(rs: &ResultSet, rows: &[usize], header: bool) -> String {
    let mut lines: Vec<String> = Vec::new();
    if header {
        lines.push(rs.columns.join("\t"));
    }
    lines.extend(rows.iter().filter_map(|&i| rs.rows.get(i)).map(|r| row_tsv(r)));
    lines.join("\n")
}

fn cell(ui: &mut Ui, cache: &mut Cache, row: usize, col: usize, value: Option<&str>) {
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

/// ⌘C copies the selected rows and ⌘A selects all, unless a text field has focus.
fn keyboard(ui: &Ui, rs: &ResultSet, state: &mut GridState) {
    if ui.ctx().memory(|m| m.focused().is_some()) {
        return;
    }
    let hovered = ui.ui_contains_pointer();
    if (hovered || !state.selection.is_empty()) && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
        state.selection = (0..rs.rows.len()).collect();
    }
    if !state.selection.is_empty() && ui.input(|inp| inp.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
        let rows: Vec<usize> = state.selection.iter().copied().filter(|&i| i < rs.rows.len()).collect();
        ui.ctx().copy_text(rows_tsv(rs, &rows, false));
    }
}

fn row_tsv(row: &[Option<String>]) -> String {
    row.iter()
        .map(|v| v.as_deref().unwrap_or("NULL"))
        .collect::<Vec<_>>()
        .join("\t")
}

fn header_bg(ui: &mut Ui) {
    let r = ui.max_rect();
    let p = ui.painter();
    p.rect_filled(r, 0.0, theme::BG());
    p.hline(r.x_range(), r.bottom(), Stroke::new(1.0, theme::BORDER()));
    p.vline(r.right(), r.y_range().shrink(7.0), Stroke::new(1.0, theme::BORDER()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the grid one frame with `events`; returns the selection.
    fn frame(ctx: &egui::Context, rs: &ResultSet, state: &mut GridState, events: Vec<egui::Event>) {
        frame_mods(ctx, rs, state, events, egui::Modifiers::NONE);
    }

    fn frame_mods(ctx: &egui::Context, rs: &ResultSet, state: &mut GridState, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            modifiers,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, Id::new("g"), rs, state, None, 0, &Source { table: "t", sql: None });
            });
        });
    }

    fn click(ctx: &egui::Context, rs: &ResultSet, state: &mut GridState, row: usize, modifiers: egui::Modifiers) {
        // Header ~30pt, rows ~25pt: aim at the middle of `row` in the first column.
        let h = ctx.style().text_styles[&TextStyle::Monospace].size;
        let row_h = ctx.fonts_mut(|f| f.row_height(&egui::FontId::monospace(h))) + 10.0;
        let pos = egui::pos2(120.0, 8.0 + row_h + 4.0 + row_h * (row as f32 + 0.5));
        let press = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        frame_mods(ctx, rs, state, vec![egui::Event::PointerMoved(pos)], modifiers);
        frame_mods(ctx, rs, state, vec![press(true)], modifiers);
        frame_mods(ctx, rs, state, vec![press(false)], modifiers);
    }

    #[test]
    fn multi_row_selection() {
        let rs = ResultSet {
            columns: vec!["id".into(), "name".into()],
            rows: (0..10).map(|i| vec![Some(i.to_string()), Some(format!("row {i}"))]).collect(),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut st = GridState::default();
        frame(&ctx, &rs, &mut st, vec![]);
        click(&ctx, &rs, &mut st, 1, egui::Modifiers::NONE);
        assert_eq!(st.selection.iter().copied().collect::<Vec<_>>(), [1]);
        click(&ctx, &rs, &mut st, 4, egui::Modifiers::SHIFT);
        assert_eq!(st.selection.iter().copied().collect::<Vec<_>>(), [1, 2, 3, 4]);
        click(&ctx, &rs, &mut st, 2, egui::Modifiers::COMMAND);
        assert_eq!(st.selection.iter().copied().collect::<Vec<_>>(), [1, 3, 4]);
        click(&ctx, &rs, &mut st, 6, egui::Modifiers::NONE);
        assert_eq!(st.selection.iter().copied().collect::<Vec<_>>(), [6]);
        let cmd_a = egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        frame_mods(&ctx, &rs, &mut st, vec![cmd_a], egui::Modifiers::COMMAND);
        assert_eq!(st.selection.len(), 10);
        assert_eq!(rows_tsv(&rs, &[1, 3], true), "id\tname\n1\trow 1\n3\trow 3");
    }

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
        crate::theme::apply(&ctx, rgrip_core::config::ThemeChoice::Dark, "rgrip Dark", "rgrip Light");
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
                    show(ui, egui::Id::new("perf"), &rs, &mut state, None, 0, &Source { table: "t", sql: None });
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
