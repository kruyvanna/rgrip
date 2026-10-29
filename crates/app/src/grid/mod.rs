//! Virtualized result grid.
//!
//! Only visible rows are laid out, and everything derived from the whole
//! result (column widths, geometry columns, geometry summaries) is computed
//! once per result set, so large pages with very long values stay smooth.

use crate::theme;
use crate::widgets::{self, Icon};
use eframe::egui::{self, Align, Id, Label, Layout, RichText, Sense, Stroke, TextStyle, Ui};
use egui_extras::{Column, TableBuilder};
use rgrip_core::ResultSet;
use rgrip_core::datetime::{self as dt, Temporal, TemporalKind};
use std::collections::{BTreeMap, HashMap};

mod columns;
mod editor;
mod goto;
mod menus;
mod paint;
#[cfg(test)]
mod perf;
#[cfg(test)]
mod tests;
mod viewer;

use self::{columns::*, editor::*, goto::*, menus::*, paint::*, viewer::*};

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
    /// Column of the last right-clicked cell (for cell actions in the menu).
    menu_col: Option<usize>,
    /// Frozen (pinned left) columns by name, in the order they were frozen.
    /// By name so they survive reloads and re-runs.
    pub frozen: Vec<String>,
    /// Column order by name, set by dragging headers (empty: as queried).
    /// Columns not listed follow in result order.
    pub order: Vec<String>,
    /// One-shot: drop the table's stored widths, which are kept by position.
    reset_widths: bool,
    /// Shared vertical offset of the frozen and scrolling halves.
    scroll_y: f32,
    /// One-shot: scroll this row into view (set when the map selects a row).
    pub reveal: Option<usize>,
    /// One-shot horizontal offset for the scrolling half (screenshot scenes).
    pub scroll_x: Option<f32>,
    /// The "Go to column" popup, when open.
    goto: Option<Goto>,
    /// Screenshot scenes: open this row's right-click menu once.
    pub demo_menu: Option<usize>,
    /// Column briefly highlighted after a jump, with the time it started.
    flash: Option<(usize, f64)>,
    /// The value popup (double-click a JSON cell, or "View value").
    viewer: Option<Viewer>,
    cache: Option<Cache>,
}

struct Viewer {
    /// Column name and 1-based row number, for the title.
    column: String,
    row: usize,
    text: String,
    /// Pretty-printed, when the value is a JSON object or array.
    json: Option<String>,
}

impl Viewer {
    fn new(column: &str, row: usize, text: &str) -> Self {
        Self { column: column.to_string(), row, text: text.to_string(), json: rgrip_core::format::pretty_json(text) }
    }
}

struct Goto {
    query: String,
    highlight: usize,
    focus: bool,
}

const FLASH_SECS: f64 = 1.2;

/// Where the rows came from, for "Copy as SQL" / "Copy for AI agent".
pub struct Source<'a> {
    pub table: &'a str,
    pub sql: Option<&'a str>,
    /// Foreign keys of the table, for "Related" in the row menu.
    pub relations: &'a [rgrip_core::Relation],
    /// Matching rows per relation for the rows the menu acts on (empty: not
    /// counted yet; `None`: couldn't count).
    pub counts: &'a [Option<i64>],
    /// Rows can be deleted from the table (by primary key).
    pub deletable: bool,
    /// The cell menu can narrow the table's filter to (or away from) a value.
    pub filterable: bool,
}

/// Per-result-set derived data.
struct Cache {
    key: (usize, usize, usize),
    widths: Vec<f32>,
    geom_cols: Vec<usize>,
    /// (row, col) → geometry summary and WKT preview.
    geoms: HashMap<(usize, usize), (String, String)>,
}

enum Freeze {
    Add(String),
    Remove(String),
    Clear,
}

pub enum GridEvent {
    HeaderClicked(usize),
    /// A cell was edited (or set to NULL); `value` may equal the original.
    Change { row: usize, col: usize, value: Option<String> },
    /// Drop the pending change of a cell.
    Revert { row: usize, col: usize },
    /// Open the rows related to `rows` through `source.relations[relation]`.
    Follow { relation: usize, rows: Vec<usize> },
    /// The row menu opened for `rows`: a chance to count related rows.
    RowsMenu { rows: Vec<usize> },
    /// The lookup icon of a relation column: show the table it references.
    Lookup { relation: usize },
    /// Delete these rows from the table (asks first).
    Delete { rows: Vec<usize> },
    /// Add `col = value` (or `<>` with `exclude`) to the table's filter.
    FilterValue { row: usize, col: usize, exclude: bool },
    /// Add pending copies of these rows.
    Duplicate { rows: Vec<usize> },
    /// Drop these pending new rows.
    RemoveNew { rows: Vec<usize> },
}

/// Inline editing, for table tabs whose rows have a primary key.
pub struct Editing<'a> {
    /// Pending changes: (row, col) → new value.
    pub changes: &'a BTreeMap<(usize, usize), Option<String>>,
    /// The cell being edited.
    pub editor: &'a mut Option<CellEditor>,
    /// Date/time kind of each column, for the picker.
    pub kinds: &'a [Option<TemporalKind>],
    /// Per column: can't be edited (e.g. an expression in a query result).
    /// Empty: every column can.
    pub locked: &'a [bool],
    /// Rows from this index on are pending new rows (`usize::MAX`: none).
    pub added_from: usize,
    /// Only the pending new rows can be edited (the table has no key to
    /// find the others by).
    pub existing_locked: bool,
    /// The row menu offers Duplicate row.
    pub insertable: bool,
}

impl Editing<'_> {
    fn can_edit(&self, row: usize, col: usize) -> bool {
        (!self.existing_locked || row >= self.added_from) && (self.locked.is_empty() || !self.locked.get(col).copied().unwrap_or(true))
    }
}

pub struct CellEditor {
    row: usize,
    col: usize,
    text: String,
    focus: bool,
    /// Month shown by the date picker.
    view: Option<(i32, u32)>,
}

impl CellEditor {
    /// Start editing a cell with its current (possibly pending) value.
    pub(crate) fn open(row: usize, col: usize, value: Option<&str>) -> Self {
        Self { row, col, text: value.unwrap_or_default().to_string(), focus: true, view: None }
    }
}

pub struct Sort<'a> {
    pub column: &'a str,
    pub ascending: bool,
}

const CHAR_W: f32 = 7.6;
const PAD: f32 = 10.0;

impl GridState {
    /// Screenshot scenes: show the Go to column popup with `query` typed.
    pub fn open_goto(&mut self, query: &str) {
        self.goto = Some(Goto { query: query.to_string(), highlight: 0, focus: true });
    }

    /// State for a new result: selection and scroll reset, frozen columns and order kept.
    pub fn fresh(old: &GridState) -> Self {
        Self { frozen: old.frozen.clone(), order: old.order.clone(), ..Default::default() }
    }

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
    mut edit: Option<Editing<'_>>,
) -> Option<GridEvent> {
    let mut event = None;
    let row_h = ui.text_style_height(&TextStyle::Monospace) + 10.0;
    let gutter_w = (((row_offset + rs.rows.len()).max(1) as f32).log10().floor() + 1.0) * CHAR_W + 22.0;

    // Tight cells with our own padding, and barely-there column dividers.
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    // Resize lines only show on hover; header cells draw their own dividers.
    ui.visuals_mut().widgets.noninteractive.bg_stroke = Stroke::NONE;

    keyboard(ui, rs, state);
    // Both halves follow `scroll_y`, so setting it moves the whole grid.
    if let Some(i) = state.reveal.take() {
        state.scroll_y = reveal_offset(i, row_h);
    }
    let selection = state.selection.clone();
    let mut clicked_row = None;
    let mut menu_row = None;
    let geom = !rs.geometry_columns().is_empty();
    let mut menu_col = state.menu_col;
    // A cell to show in the value popup: (row, column).
    let mut view: Option<(usize, usize)> = None;
    // Row of the last plain click: a double-click's first click selects it,
    // so quick clicks on two rows don't count as one.
    let anchor = state.anchor;
    let demo_menu = state.demo_menu.take();
    // Frozen columns (by name) that exist in this result, in freeze order.
    let frozen: Vec<usize> = state.frozen.iter().filter_map(|n| rs.columns.iter().position(|c| c == n)).collect();
    let rest: Vec<usize> = display_order(&rs.columns, &state.order).into_iter().filter(|c| !frozen.contains(c)).collect();
    let header_id = |salt: &str, c: usize| id.with(("header", salt, c));
    // The header being dragged to a new place: (in the frozen half, column).
    let drag: Option<(bool, usize)> = ui.ctx().dragged_id().and_then(|d| {
        frozen.iter().map(|&c| (true, c)).chain(rest.iter().map(|&c| (false, c))).find(|&(f, c)| header_id(if f { "frozen" } else { "main" }, c) == d)
    });
    // Header cells as laid out, for the drop position: (frozen half, column, rect).
    let mut headers: Vec<(bool, usize, egui::Rect)> = Vec::new();
    let mut dropped: Option<(bool, usize)> = None;
    let reset_widths = std::mem::take(&mut state.reset_widths);
    let reordered = !state.order.is_empty();
    let mut reset_order = false;
    let scroll_y = state.scroll_y;
    let scroll_x = state.scroll_x.take();
    let flash_state = state.flash;
    let cache = state.cache(rs);
    let cache_widths = cache.widths.clone();
    let changes = edit.as_ref().map(|e| e.changes);
    let kinds = edit.as_ref().map(|e| e.kinds);
    let added_from = edit.as_ref().map_or(usize::MAX, |e| e.added_from);
    let mut close_editor = false;

    let mut freeze: Option<Freeze> = None;
    let mut open_goto = false;
    let mut goto_anchor: Option<egui::Rect> = None;
    let now = ui.input(|i| i.time);
    // Highlight strength of the jumped-to column, fading out.
    let flash = flash_state.map(|(c, t)| (c, (1.0 - (now - t) / FLASH_SECS).clamp(0.0, 1.0) as f32)).filter(|f| f.1 > 0.0);
    if flash.is_some() {
        ui.ctx().request_repaint();
    }
    let sortable = sort.is_some();

    // Draws one half: the frozen part (row numbers + frozen columns, no
    // horizontal scroll) or the scrolling rest. Returns its vertical offset.
    let mut draw = |ui: &mut Ui, salt: &str, cols: &[usize], gutter: bool| -> f32 {
        let ctx = ui.ctx().clone();
        let mut table = TableBuilder::new(ui)
            .id_salt(id.with(salt))
            .striped(true)
            .resizable(!gutter)
            .sense(Sense::click())
            .auto_shrink([false, false])
            .vertical_scroll_offset(scroll_y)
            .cell_layout(Layout::left_to_right(Align::Center));
        if gutter {
            table = table
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .column(Column::exact(gutter_w));
            for &c in cols {
                table = table.column(Column::exact(cache.widths[c]).clip(true));
            }
        } else {
            for &c in cols {
                table = table.column(Column::initial(cache.widths[c]).at_least(40.0).clip(true));
            }
            if reset_widths {
                table.reset();
            }
        }
        let out = table
            .header(row_h + 4.0, |mut header| {
                if gutter {
                    header.col(|ui| {
                        header_bg(ui);
                        let rect = ui.max_rect();
                        goto_anchor = Some(rect);
                        let r = ui.interact(rect, id.with("goto-btn"), Sense::click());
                        let tint = if r.hovered() { theme::TEXT() } else { theme::FAINT() };
                        widgets::paint_icon_tinted(ui.painter(), rect.center(), Icon::Columns, Some(tint));
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Go to column  ⌘J").clicked() {
                            open_goto = true;
                        }
                    });
                }
                for &i in cols {
                    let name = &rs.columns[i];
                    header.col(|ui| {
                        header_bg(ui);
                        let rect = ui.max_rect();
                        if let Some((c, a)) = flash && c == i {
                            ui.painter().rect_filled(rect, 0.0, theme::ACCENT().gamma_multiply(0.4 * a));
                        }
                        // The whole cell takes clicks and drags (to reorder); the
                        // lookup icon, added later, sits on top.
                        let r = ui.interact(rect, header_id(salt, i), Sense::click_and_drag());
                        headers.push((gutter, i, rect));
                        if drag == Some((gutter, i)) {
                            ui.painter().rect_filled(rect, 0.0, theme::HOVER());
                        }
                        if r.drag_stopped() {
                            dropped = Some((gutter, i));
                        }
                        ui.add_space(PAD);
                        // The outgoing relation this column is part of: gets a lookup icon.
                        let lookup = source.relations.iter().position(|r| r.outgoing && r.columns.iter().any(|(here, _)| here == name));
                        // Keep the right edge free for the sort indicator and lookup icon.
                        let label_w = rect.width() - PAD - if sortable { 22.0 } else { 0.0 } - if lookup.is_some() { 20.0 } else { 0.0 };
                        ui.allocate_ui_with_layout(egui::vec2(label_w.max(8.0), rect.height()), Layout::left_to_right(Align::Center), |ui| {
                            ui.add(Label::new(RichText::new(name).size(12.5).color(theme::TEXT())).truncate().selectable(false));
                        });
                        if let Some(s) = sort.as_ref() {
                            let c = egui::pos2(rect.right() - 13.0, rect.center().y);
                            if s.column == name {
                                paint_sort(ui.painter(), c, Some(s.ascending), theme::ACCENT());
                            } else if ui.rect_contains_pointer(rect) {
                                paint_sort(ui.painter(), c, None, theme::FAINT());
                            }
                        }
                        if r.clicked() {
                            event = Some(GridEvent::HeaderClicked(i));
                        }
                        if let Some(ri) = lookup {
                            let x = rect.right() - if sortable { 34.0 } else { 13.0 };
                            let icon = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(16.0, 16.0));
                            let hit = ui.interact(icon.expand(2.0), id.with(("lookup", salt, i)), Sense::click());
                            let tint = if hit.hovered() { theme::ACCENT() } else { theme::FAINT() };
                            crate::icons::paint(ui, crate::icons::Svg::Search, icon.shrink(1.5), tint);
                            let rel = &source.relations[ri];
                            let other = if rel.other_schema == "public" { rel.other_table.clone() } else { format!("{}.{}", rel.other_schema, rel.other_table) };
                            if hit.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("Look up in {other}")).clicked() {
                                event = Some(GridEvent::Lookup { relation: ri });
                            }
                        }
                        let is_frozen = frozen.contains(&i);
                        r.context_menu(|ui| {
                        crate::app::menu_layout(ui);
                            ui.label(RichText::new(name).small().color(theme::MUTED()));
                            if is_frozen {
                                if crate::icons::menu_item(ui, crate::icons::Svg::Pin, "Unfreeze column", true).clicked() {
                                    freeze = Some(Freeze::Remove(name.clone()));
                                    ui.close();
                                }
                            } else if ui
                                .add_enabled_ui(rest.len() > 1, |ui| crate::icons::menu_item(ui, crate::icons::Svg::Pin, "Freeze column", rest.len() > 1))
                                .inner
                                .on_hover_text("Keep it on the left while scrolling sideways")
                                .clicked()
                            {
                                freeze = Some(Freeze::Add(name.clone()));
                                ui.close();
                            }
                            if crate::icons::menu_item(ui, crate::icons::Svg::Search, "Go to column…  ⌘J", true).clicked() {
                                open_goto = true;
                                ui.close();
                            }
                            if reordered
                                && crate::icons::menu_item(ui, crate::icons::Svg::Undo, "Reset column order", true)
                                    .on_hover_text("Back to the order of the query; drag headers to reorder")
                                    .clicked()
                            {
                                reset_order = true;
                                ui.close();
                            }
                            ui.separator();
                            let all: Vec<usize> = (0..rs.rows.len()).collect();
                            copy_column_items(ui, rs, &all, i, "Copy column values");
                            if !frozen.is_empty() && crate::icons::menu_item(ui, crate::icons::Svg::Pin, "Unfreeze all", true).clicked() {
                                freeze = Some(Freeze::Clear);
                                ui.close();
                            }
                        });
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, rs.rows.len(), |mut row| {
                    let i = row.index();
                    row.set_selected(selection.contains(&i));
                    if gutter {
                        row.col(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(8.0);
                                // A pending new row has no number yet.
                                let (text, color) = if i >= added_from { ("+".to_string(), theme::ACCENT()) } else { ((row_offset + i + 1).to_string(), theme::FAINT()) };
                                ui.label(RichText::new(text).size(11.0).color(color));
                            });
                        });
                    }
                    for &col in cols {
                        let pending = changes.and_then(|c| c.get(&(i, col)));
                        let kind = kinds.and_then(|k| k.get(col).copied().flatten());
                        let value = pending.map_or(rs.rows[i][col].as_deref(), |v| v.as_deref());
                        let editing = edit.as_mut().and_then(|e| e.editor.as_mut()).filter(|ed| ed.row == i && ed.col == col);
                        let (_, cell_resp) = row.col(|ui| {
                            if pending.is_some() || i >= added_from {
                                ui.painter().rect_filled(ui.max_rect(), 0.0, theme::ACCENT().gamma_multiply(0.16));
                            }
                            if let Some((c, a)) = flash && c == col {
                                ui.painter().rect_filled(ui.max_rect(), 0.0, theme::ACCENT().gamma_multiply(0.16 * a));
                            }
                            ui.add_space(PAD);
                            match editing {
                                Some(ed) => {
                                    if let EditorOutcome::Close(e) = cell_editor(ui, ed, rs.rows[i][col].as_deref(), kind) {
                                        close_editor = true;
                                        if e.is_some() {
                                            event = e;
                                        }
                                    }
                                }
                                None => cell(ui, cache, i, col, value),
                            }
                        });
                        // Double-click: JSON (or any value, read-only) opens the
                        // value popup; other editable cells open the editor.
                        if cell_resp.double_clicked() && !cache.geom_cols.contains(&col) {
                            let json = value.is_some_and(|v| matches!(v.trim_start().as_bytes().first(), Some(b'{' | b'[')));
                            match edit.as_mut() {
                                Some(e) if !json && e.can_edit(i, col) => *e.editor = Some(CellEditor::open(i, col, value)),
                                _ if value.is_some() && anchor == Some(i) => view = Some((i, col)),
                                _ => {}
                            }
                        }
                        if cell_resp.secondary_clicked() {
                            menu_col = Some(col);
                        }
                    }
                    let response = row.response();
                    if !gutter && demo_menu == Some(i) {
                        menu_col = cols.get(1).or(cols.first()).copied();
                        // Context menus open at a stored pointer position.
                        let at = response.rect.left_center() + egui::vec2(180.0, 0.0);
                        #[allow(deprecated)] // the Popup API has no "open at" for context menus
                        ctx.memory_mut(|m| m.open_popup_at(egui::Popup::default_response_id(&response), at));
                        menu_row = Some(i);
                    }
                    if response.clicked() {
                        clicked_row = Some(i);
                    }
                    if response.secondary_clicked() {
                        menu_row = Some(i);
                    }
                    response.context_menu(|ui| {
                        crate::app::menu_layout(ui);
                        // The menu acts on the selection (or just this row).
                        let rows: Vec<usize> = if selection.contains(&i) { selection.iter().copied().collect() } else { vec![i] };
                        // Cell actions for the right-clicked cell.
                        if let Some(col) = menu_col.filter(|c| *c < rs.columns.len()) {
                            ui.label(RichText::new(&rs.columns[col]).small().color(theme::MUTED()));
                            let shown = changes.and_then(|c| c.get(&(i, col))).map_or(rs.rows[i][col].as_deref(), |v| v.as_deref());
                            if crate::icons::menu_item(ui, crate::icons::Svg::Copy, "Copy value", shown.is_some()).clicked() {
                                ui.ctx().copy_text(shown.unwrap_or_default().to_string());
                                ui.close();
                            }
                            if crate::icons::menu_item(ui, crate::icons::Svg::Search, "View value", shown.is_some()).on_hover_text("JSON indented; or double-click a JSON cell").clicked() {
                                view = Some((i, col));
                                ui.close();
                            }
                            if rows.len() > 1 {
                                copy_column_items(ui, rs, &rows, col, &format!("Copy {} ({} values)", rs.columns[col], rows.len()));
                            }
                            // Filtering by a value of a row that doesn't exist yet makes no sense.
                            if source.filterable && i < added_from {
                                ui.separator();
                                let shown = rs.rows[i][col].as_deref();
                                if crate::icons::menu_item(ui, crate::icons::Svg::Search, "Filter by this value", true).on_hover_text(rgrip_core::edit::value_filter(&rs.columns[col], shown, false)).clicked() {
                                    event = Some(GridEvent::FilterValue { row: i, col, exclude: false });
                                    ui.close();
                                }
                                if crate::icons::menu_item(ui, crate::icons::Svg::Ban, "Exclude this value", true).on_hover_text(rgrip_core::edit::value_filter(&rs.columns[col], shown, true)).clicked() {
                                    event = Some(GridEvent::FilterValue { row: i, col, exclude: true });
                                    ui.close();
                                }
                            }
                            if edit.as_ref().is_some_and(|e| e.can_edit(i, col)) && !cache.geom_cols.contains(&col) {
                                ui.separator();
                            }
                        }
                        if let (Some(e), Some(col)) = (edit.as_mut(), menu_col.filter(|c| !cache.geom_cols.contains(c))) && e.can_edit(i, col) {
                            let pending = e.changes.get(&(i, col));
                            let value = pending.map_or(rs.rows[i][col].as_deref(), |v| v.as_deref());
                            if crate::icons::menu_item(ui, crate::icons::Svg::Pencil, "Edit cell", true).on_hover_text("Or double-click it").clicked() {
                                *e.editor = Some(CellEditor::open(i, col, value));
                                ui.close();
                            }
                            if crate::icons::menu_item(ui, crate::icons::Svg::Ban, "Set NULL", value.is_some()).clicked() {
                                event = Some(GridEvent::Change { row: i, col, value: None });
                                ui.close();
                            }
                            if pending.is_some() && crate::icons::menu_item(ui, crate::icons::Svg::Undo, "Revert cell", true).clicked() {
                                event = Some(GridEvent::Revert { row: i, col });
                                ui.close();
                            }
                            ui.separator();
                        }
                        if menu_col.is_some() {
                            ui.separator();
                        }
                        if let Some(e) = rows_menu(ui, rs, &rows, source, geom) {
                            event = Some(e);
                        }
                        if let Some(e) = edit.as_ref().filter(|e| e.insertable) {
                            ui.separator();
                            let new: Vec<usize> = rows.iter().copied().filter(|&r| r >= e.added_from).collect();
                            if crate::icons::menu_item(ui, crate::icons::Svg::Copy, if rows.len() == 1 { "Duplicate row" } else { "Duplicate rows" }, true)
                                .on_hover_text("Add a copy as a new row, without its key")
                                .clicked()
                            {
                                event = Some(GridEvent::Duplicate { rows: rows.clone() });
                                ui.close();
                            }
                            if !new.is_empty() && crate::icons::menu_item(ui, crate::icons::Svg::Undo, if new.len() == 1 { "Remove new row" } else { "Remove new rows" }, true).clicked() {
                                event = Some(GridEvent::RemoveNew { rows: new });
                                ui.close();
                            }
                        }
                        // New rows aren't in the table yet: nothing to delete.
                        if source.deletable && rows.iter().all(|&r| r < added_from) {
                            ui.separator();
                            let label = if rows.len() == 1 { "Delete row".to_string() } else { format!("Delete {} rows", rows.len()) };
                            if crate::icons::danger_item(ui, crate::icons::Svg::Trash, &label).clicked() {
                                event = Some(GridEvent::Delete { rows });
                                ui.close();
                            }
                        }
                    });
                });
            });
        out.state.offset.y
    };

    let left_w = gutter_w + frozen.iter().map(|&c| cache_widths[c]).sum::<f32>();
    let mut offsets = [scroll_y; 2];
    let mut main_clip = egui::Rect::NOTHING;
    let grid_rect = ui.horizontal_top(|ui| {
        let h = ui.available_height();
        ui.allocate_ui_with_layout(egui::vec2(left_w, h), Layout::top_down(Align::Min), |ui| {
            ui.set_width(left_w);
            offsets[0] = draw(ui, "frozen", &frozen, true);
        });
        if !frozen.is_empty() {
            // Edge of the frozen block.
            let x = ui.cursor().left();
            ui.painter().vline(x, ui.max_rect().y_range(), Stroke::new(1.5_f32, theme::ACCENT().gamma_multiply(0.6)));
        }
        let mut scroll = egui::ScrollArea::horizontal().id_salt(id.with("h")).auto_shrink([false, false]);
        if let Some(x) = scroll_x {
            scroll = scroll.horizontal_scroll_offset(x);
        }
        scroll
            .show(ui, |ui| {
                // The table stacks header and body: it needs a vertical layout.
                ui.vertical(|ui| offsets[1] = draw(ui, "main", &rest, false));
                main_clip = ui.clip_rect();
                // Dragging a header near an edge scrolls sideways.
                if drag.is_some_and(|(f, _)| !f)
                    && let Some(p) = ui.ctx().pointer_hover_pos()
                {
                    const EDGE: f32 = 40.0;
                    let dx = (main_clip.left() + EDGE - p.x).max(0.0) - (p.x - main_clip.right() + EDGE).max(0.0);
                    if dx != 0.0 {
                        ui.scroll_with_delta(egui::vec2(dx.clamp(-EDGE, EDGE) * 0.5, 0.0));
                        ui.ctx().request_repaint();
                    }
                }
            });
    })
    .response
    .rect;
    let pointer = ui.ctx().pointer_interact_pos();
    // Where the header `c` would land in its half: the slot among the other
    // headers, and the x of the gap.
    let slot = |f: bool, c: usize, x: f32| -> (usize, f32) {
        let others: Vec<egui::Rect> = headers.iter().filter(|h| h.0 == f && h.1 != c).map(|h| h.2).collect();
        let k = others.iter().filter(|r| r.center().x < x).count();
        let gap = others.get(k).map(|r| r.left()).or(others.last().map(|r| r.right())).unwrap_or(x);
        (k, gap)
    };
    if let (Some((f, c)), Some(p)) = (drag, pointer) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let (_, x) = slot(f, c, p.x);
        let clip = if f { grid_rect.with_max_x(grid_rect.left() + left_w) } else { main_clip.intersect(grid_rect) };
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, id.with("col-drag")));
        painter.with_clip_rect(clip.expand(1.0)).vline(x, grid_rect.y_range(), Stroke::new(2.0_f32, theme::ACCENT()));
        let g = painter.layout_no_wrap(rs.columns[c].clone(), egui::FontId::proportional(12.5), theme::TEXT());
        let tag = egui::Rect::from_min_size(p + egui::vec2(12.0, -10.0), g.size() + egui::vec2(16.0, 8.0));
        painter.rect(tag, 4.0, theme::SURFACE(), Stroke::new(1.0_f32, theme::ACCENT()), egui::StrokeKind::Inside);
        painter.galley(tag.min + egui::vec2(8.0, 4.0), g, theme::TEXT());
    }
    if reset_order {
        state.order.clear();
        state.reset_widths = true;
    }
    if let (Some((f, c)), Some(p)) = (dropped, pointer) {
        let (k, _) = slot(f, c, p.x);
        let name = &rs.columns[c];
        if f {
            let shown: Vec<String> = frozen.iter().map(|&c| rs.columns[c].clone()).collect();
            let mut names = move_to(&shown, name, k);
            names.extend(state.frozen.iter().filter(|n| !shown.contains(n)).cloned());
            state.frozen = names;
        } else {
            let moved = move_to(&rest, &c, k);
            if moved != rest {
                // Place it in the full order next to its new neighbour, so
                // frozen columns keep their spot for when they're unfrozen.
                let mut all: Vec<usize> = display_order(&rs.columns, &state.order).into_iter().filter(|&x| x != c).collect();
                let at = match moved.get(k + 1) {
                    Some(next) => all.iter().position(|x| x == next).unwrap_or(all.len()),
                    None => moved.get(k.wrapping_sub(1)).and_then(|prev| all.iter().position(|x| x == prev)).map_or(all.len(), |i| i + 1),
                };
                all.insert(at, c);
                state.order = all.iter().map(|&c| rs.columns[c].clone()).collect();
                // The table keeps widths by position: carry the current ones
                // over by column and start it afresh.
                if let Some(cache) = state.cache.as_mut() {
                    for &(_, col, rect) in headers.iter().filter(|h| !h.0) {
                        cache.widths[col] = rect.width();
                    }
                }
                state.reset_widths = true;
            }
        }
        ui.ctx().request_repaint();
    }
    // Whichever half was scrolled leads; the other follows next frame.
    let moved = offsets.iter().copied().find(|y| (y - scroll_y).abs() > 0.5);
    if let Some(y) = moved {
        state.scroll_y = y;
        ui.ctx().request_repaint();
    }
    match freeze {
        Some(Freeze::Add(name)) => state.frozen.push(name),
        Some(Freeze::Remove(name)) => state.frozen.retain(|n| *n != name),
        Some(Freeze::Clear) => state.frozen.clear(),
        None => {}
    }
    state.menu_col = menu_col;
    if let Some((r, c)) = view {
        let pending = edit.as_ref().and_then(|e| e.changes.get(&(r, c)));
        let value = pending.map_or(rs.rows[r][c].as_deref(), |v| v.as_deref());
        state.viewer = value.map(|v| Viewer::new(&rs.columns[c], row_offset + r + 1, v));
    }
    viewer_ui(ui.ctx(), &mut state.viewer);
    let opened_menu = menu_row;
    if flash.is_none() {
        state.flash = None;
    }
    if open_goto && state.goto.is_none() {
        state.goto = Some(Goto { query: String::new(), highlight: 0, focus: true });
    }
    if state.goto.is_some() {
        let anchor = goto_anchor.unwrap_or_else(|| ui.min_rect());
        if let Some(col) = goto_popup(ui, id, rs, state, anchor) {
            jump(state, col, &rest, &cache_widths, now);
            ui.ctx().request_repaint();
        }
    }
    if close_editor && let Some(e) = edit.as_mut() {
        *e.editor = None;
    }
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
    if opened_menu.is_some() && event.is_none() && !source.relations.is_empty() {
        event = Some(GridEvent::RowsMenu { rows: state.selection.iter().copied().collect() });
    }
    event
}

/// ⌘C copies the selected rows and ⌘A selects all, unless a text field has focus.
fn keyboard(ui: &Ui, rs: &ResultSet, state: &mut GridState) {
    if ui.ctx().memory(|m| m.focused().is_some()) {
        return;
    }
    let hovered = ui.ui_contains_pointer();
    if (hovered || !state.selection.is_empty()) && state.goto.is_none() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::J)) {
        state.goto = Some(Goto { query: String::new(), highlight: 0, focus: true });
    }
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

