//! The Diagram tab: an ER diagram of one schema. Tables are boxes you drag,
//! foreign keys are solid lines from the column to the key it points at, and
//! guessed links (see `rgrip_core::relations`) are dashed. Pan by dragging the
//! background, zoom with scroll or pinch. Box positions are remembered per
//! connection and schema in `diagrams.toml`.

use crate::map_export as mx;
use crate::widgets::{self, Icon};
use crate::theme;
use eframe::egui::{self, Align2, CornerRadius, FontId, Margin, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, Vec2, pos2, vec2};
use rgrip_core::TableKind;
use rgrip_core::diagram::{self, Diagram};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const HEAD_H: f32 = 28.0;
const ROW_H: f32 = 20.0;
const PAD: f32 = 8.0;
/// Room left of a column name for the key and link markers.
const MARK_W: f32 = 32.0;
/// Rows shown per table; the rest collapse into a "+N more" line.
const MAX_ROWS: usize = 30;
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 2.5;

pub enum State {
    /// Waiting for the connection before the request goes out.
    Pending,
    Loading,
    Ready,
    Failed(String),
}

/// A link as drawn: from a row of one box to a row of another.
struct Edge {
    link: usize,
    from: usize,
    to: usize,
    from_row: usize,
    to_row: Option<usize>,
    inferred: bool,
}

enum Export {
    /// Draw the next frame clean, then ask for a screenshot.
    Hide,
    Wait(Instant),
}

/// What the tab asks the app to do.
#[derive(Default)]
pub struct Outcome {
    pub reload: bool,
    pub open: Option<String>,
}

pub struct DiagramTab {
    pub schema: String,
    /// Where box positions are saved: connection and schema.
    key: String,
    pub state: State,
    diagram: Diagram,
    edges: Vec<Edge>,
    /// Top-left of each box and its size, in diagram units.
    pos: Vec<Pos2>,
    sizes: Vec<Vec2>,
    placed: bool,
    dragging: Option<usize>,
    saved: BTreeMap<String, [f32; 2]>,
    pan: Vec2,
    zoom: f32,
    fit: bool,
    show_inferred: bool,
    filter: String,
    export: Option<Export>,
    notice: Option<(String, Instant)>,
}

impl DiagramTab {
    /// What a jump list entry keeps: the filter and the inferred-relations toggle (the view refits on load).
    pub fn view_state(&self) -> (&str, bool) {
        (&self.filter, self.show_inferred)
    }

    pub fn set_view_state(&mut self, filter: String, show_inferred: bool) {
        self.filter = filter;
        self.show_inferred = show_inferred;
    }

    pub fn new(schema: &str, key: String) -> Self {
        Self {
            schema: schema.to_string(),
            saved: rgrip_core::config::load_diagram(&key),
            key,
            state: State::Pending,
            diagram: Diagram::default(),
            edges: Vec::new(),
            pos: Vec::new(),
            sizes: Vec::new(),
            placed: false,
            dragging: None,
            pan: Vec2::ZERO,
            zoom: 1.0,
            fit: true,
            show_inferred: true,
            filter: String::new(),
            export: None,
            notice: None,
        }
    }

    pub fn loaded(&mut self, result: Result<Diagram, String>) {
        match result {
            Ok(d) => {
                self.edges = d
                    .links
                    .iter()
                    .enumerate()
                    .filter_map(|(link, l)| {
                        let (a, b) = l.columns.first()?;
                        let row = |t: usize, name: &str| d.tables[t].columns.iter().position(|c| c.name == *name);
                        Some(Edge { link, from: l.from, to: l.to, from_row: row(l.from, a)?, to_row: row(l.to, b), inferred: l.inferred })
                    })
                    .collect();
                self.diagram = d;
                self.placed = false;
                self.fit = true;
                self.state = State::Ready;
            }
            Err(e) => self.state = State::Failed(e),
        }
    }

    /// Measure the boxes, then place them: where they were left, else automatically.
    fn place(&mut self, ui: &Ui, use_saved: bool) {
        let painter = ui.painter();
        let width = |text: &str, size: f32| painter.layout_no_wrap(text.to_string(), FontId::proportional(size), theme::TEXT()).size().x;
        self.sizes = self
            .diagram
            .tables
            .iter()
            .map(|t| {
                let shown = t.columns.len().min(MAX_ROWS);
                let rows = t.columns.iter().take(shown).map(|c| width(&c.name, 12.0) + 18.0 + width(&short_type(&c.data_type), 11.0)).fold(0.0, f32::max);
                let w = (width(&t.name, 13.0) + 2.0 * PAD).max(MARK_W + rows + PAD).max(150.0);
                let lines = shown + usize::from(t.columns.len() > shown);
                vec2(w, HEAD_H + lines as f32 * ROW_H + 6.0)
            })
            .collect();
        let sizes: Vec<(f32, f32)> = self.sizes.iter().map(|s| (s.x, s.y)).collect();
        let edges: Vec<(usize, usize)> = self.diagram.links.iter().map(|l| (l.from, l.to)).collect();
        self.pos = diagram::auto_layout(&sizes, &edges)
            .into_iter()
            .zip(&self.diagram.tables)
            .map(|((x, y), t)| match self.saved.get(&t.name).filter(|_| use_saved) {
                Some(&[x, y]) => pos2(x, y),
                None => pos2(x, y),
            })
            .collect();
        // Tables without a saved spot go below everything placed, not onto it.
        if use_saved {
            let fresh: Vec<usize> = (0..self.pos.len()).filter(|&i| !self.saved.contains_key(&self.diagram.tables[i].name)).collect();
            if !fresh.is_empty() && fresh.len() < self.pos.len() {
                let mut y = (0..self.pos.len()).filter(|i| !fresh.contains(i)).map(|i| self.pos[i].y + self.sizes[i].y).fold(f32::MIN, f32::max) + 60.0;
                let (mut x, mut row_h) = (0.0f32, 0.0f32);
                for i in fresh {
                    if x > 0.0 && x + self.sizes[i].x > 1600.0 {
                        y += row_h + 28.0;
                        x = 0.0;
                        row_h = 0.0;
                    }
                    self.pos[i] = pos2(x, y);
                    x += self.sizes[i].x + 45.0;
                    row_h = row_h.max(self.sizes[i].y);
                }
            }
        }
        self.placed = true;
    }

    fn save(&mut self) {
        self.saved = self.diagram.tables.iter().zip(&self.pos).map(|(t, p)| (t.name.clone(), [p.x, p.y])).collect();
        if let Err(e) = rgrip_core::config::save_diagram(&self.key, self.saved.clone()) {
            self.notice = Some((format!("Couldn't save the layout: {e}"), Instant::now()));
        }
    }

    /// Tables matching the filter and their direct neighbors.
    fn visible(&self) -> Vec<bool> {
        let f = self.filter.trim().to_lowercase();
        if f.is_empty() {
            return vec![true; self.diagram.tables.len()];
        }
        let hit: Vec<bool> = self.diagram.tables.iter().map(|t| t.name.to_lowercase().contains(&f)).collect();
        let mut v = hit.clone();
        for e in self.edges.iter().filter(|e| self.show_inferred || !e.inferred) {
            v[e.from] |= hit[e.to];
            v[e.to] |= hit[e.from];
        }
        v
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Outcome {
        let mut out = Outcome::default();
        match &self.state {
            State::Pending | State::Loading => {
                ui.centered_and_justified(|ui| ui.spinner());
                return out;
            }
            State::Failed(e) => {
                let msg = e.clone();
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.colored_label(theme::ERROR(), msg);
                    if ui.button("Retry").clicked() {
                        out.reload = true;
                    }
                });
                return out;
            }
            State::Ready => {}
        }
        if !self.placed {
            self.place(ui, true);
        }

        // Toolbar.
        egui::Frame::new().inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.show_inferred, "Show inferred");
                if ui.button("Fit").clicked() {
                    self.fit = true;
                }
                let before = self.filter.len();
                ui.add(TextEdit::singleline(&mut self.filter).hint_text("Filter tables").desired_width(160.0));
                if self.filter.len() != before {
                    self.fit = true;
                }
                if ui.button("Re-layout").clicked() {
                    self.place(ui, false);
                    self.fit = true;
                    self.save();
                }
                if ui.button("Export PNG").clicked() {
                    self.export = Some(Export::Hide);
                }
                if ui.button("Reload").clicked() {
                    out.reload = true;
                }
                let inferred = self.edges.iter().filter(|e| e.inferred).count();
                let text = format!("{} tables · {} links · {} inferred", self.diagram.tables.len(), self.edges.len() - inferred, inferred);
                ui.label(egui::RichText::new(text).color(theme::MUTED()));
            });
        });
        let rect = ui.available_rect_before_wrap();
        ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, theme::BORDER()));

        let visible = self.visible();
        let size_of = |s: &Self, i: usize| s.sizes[i] * s.zoom;

        // Fit the shown boxes into the canvas.
        if self.fit {
            self.fit = false;
            let mut bounds: Option<Rect> = None;
            for i in (0..visible.len()).filter(|&i| visible[i]) {
                let r = Rect::from_min_size(self.pos[i], self.sizes[i]);
                bounds = Some(bounds.map_or(r, |b| b.union(r)));
            }
            if let Some(b) = bounds {
                let b = b.expand(30.0);
                self.zoom = (rect.width() / b.width()).min(rect.height() / b.height()).clamp(MIN_ZOOM, 1.0);
                self.pan = rect.size() / 2.0 - b.center().to_vec2() * self.zoom;
            }
        }

        // Canvas: drag to pan, scroll or pinch to zoom around the pointer.
        let bg = ui.interact(rect, ui.id().with("diagram-canvas"), Sense::click_and_drag());
        if bg.dragged() {
            self.pan += bg.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p) && ui.rect_contains_pointer(rect));
        if let Some(p) = pointer {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let wanted = (self.zoom * (scroll / 300.0).exp() * pinch).clamp(MIN_ZOOM, MAX_ZOOM);
            if (wanted - self.zoom).abs() > 1e-4 {
                let f = wanted / self.zoom;
                let at = p - rect.min;
                self.pan = at - (at - self.pan) * f;
                self.zoom = wanted;
            }
        }
        let origin = rect.min + self.pan;
        let z = self.zoom;
        let screen = |s: &Self, i: usize| Rect::from_min_size(origin + s.pos[i].to_vec2() * z, size_of(s, i));

        // Boxes: drag to move, double-click or menu to open.
        let mut hovered_box = None;
        let mut moved = false;
        for i in (0..visible.len()).filter(|&i| visible[i]) {
            let r = screen(self, i);
            // A box being dragged stays interactive when it leaves the canvas.
            let dragging = self.dragging == Some(i);
            if !rect.intersects(r) && !dragging {
                continue;
            }
            let resp = ui.interact(if dragging { r } else { r.intersect(rect) }, ui.id().with(("diagram-box", i)), Sense::click_and_drag());
            if resp.drag_started() {
                self.dragging = Some(i);
            }
            if resp.dragged() {
                self.pos[i] += resp.drag_delta() / z;
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            if resp.drag_stopped() {
                self.dragging = None;
                moved = true;
            }
            if resp.hovered() || resp.dragged() {
                hovered_box = Some(i);
            }
            if resp.double_clicked() {
                out.open = Some(self.diagram.tables[i].name.clone());
            }
            resp.context_menu(|ui| {
                crate::app::menu_layout(ui);
                if crate::icons::menu_item(ui, crate::icons::Svg::Table, "Open table", true).clicked() {
                    out.open = Some(self.diagram.tables[i].name.clone());
                    ui.close();
                }
            });
        }
        if moved {
            self.save();
        }

        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, theme::BG());
        let shown_rows = |s: &Self, t: usize| s.diagram.tables[t].columns.len().min(MAX_ROWS);
        let row_y = |s: &Self, t: usize, row: Option<usize>| {
            let top = origin.y + s.pos[t].y * z;
            match row.filter(|&r| r < shown_rows(s, t)) {
                Some(r) => top + (HEAD_H + (r as f32 + 0.5) * ROW_H) * z,
                None => top + HEAD_H / 2.0 * z,
            }
        };

        // Edges, under the boxes.
        let wide = (1.4 * z).clamp(1.0, 2.5);
        let mut hovered_edge: Option<usize> = None;
        let mut drawn: Vec<(usize, Vec<Pos2>)> = Vec::new();
        for (ei, e) in self.edges.iter().enumerate() {
            if !(visible[e.from] && visible[e.to]) || (e.inferred && !self.show_inferred) {
                continue;
            }
            let (a, b) = (screen(self, e.from), screen(self, e.to));
            let (ya, yb) = (row_y(self, e.from, Some(e.from_row)), row_y(self, e.to, e.to_row));
            let reach = (50.0 * z).max(24.0);
            let (p0, p1, p2, p3) = if b.left() > a.right() + 10.0 {
                (pos2(a.right(), ya), pos2(a.right() + reach, ya), pos2(b.left() - reach, yb), pos2(b.left(), yb))
            } else if b.right() < a.left() - 10.0 {
                (pos2(a.left(), ya), pos2(a.left() - reach, ya), pos2(b.right() + reach, yb), pos2(b.right(), yb))
            } else {
                // Overlapping columns of boxes (or a self reference): loop out on the right.
                let x = a.right().max(b.right());
                (pos2(a.right(), ya), pos2(x + reach, ya), pos2(x + reach, yb), pos2(b.right(), yb))
            };
            let bbox = Rect::from_points(&[p0, p1, p2, p3]);
            if !rect.intersects(bbox) {
                continue;
            }
            let pts: Vec<Pos2> = (0..=20)
                .map(|k| {
                    let t = k as f32 / 20.0;
                    let u = 1.0 - t;
                    (p0.to_vec2() * (u * u * u) + p1.to_vec2() * (3.0 * u * u * t) + p2.to_vec2() * (3.0 * u * t * t) + p3.to_vec2() * (t * t * t)).to_pos2()
                })
                .collect();
            drawn.push((ei, pts));
        }
        if let (Some(p), None) = (pointer, hovered_box) {
            let near = drawn.iter().map(|(ei, pts)| (*ei, pts.windows(2).map(|w| seg_dist(p, w[0], w[1])).fold(f32::MAX, f32::min))).filter(|(_, d)| *d < 6.0);
            hovered_edge = near.min_by(|a, b| a.1.total_cmp(&b.1)).map(|(ei, _)| ei);
        }
        for (ei, pts) in &drawn {
            let e = &self.edges[*ei];
            let lit = hovered_edge == Some(*ei) || hovered_box.is_some_and(|b| b == e.from || b == e.to);
            let base = if e.inferred { theme::ACCENT().gamma_multiply(0.75) } else { theme::MUTED() };
            let stroke = if lit { Stroke::new(wide + 1.0, theme::ACCENT()) } else { Stroke::new(wide, base) };
            if e.inferred {
                painter.extend(Shape::dashed_line(pts, stroke, 7.0 * z.max(0.6), 5.0 * z.max(0.6)));
            } else {
                painter.add(Shape::line(pts.clone(), stroke));
            }
            // Arrowhead at the key it points at.
            let (tip, prev) = (pts[pts.len() - 1], pts[pts.len() - 4]);
            let dir = (tip - prev).normalized();
            let s = (6.0 * z).max(4.0);
            let side = vec2(-dir.y, dir.x) * s * 0.5;
            painter.add(Shape::convex_polygon(vec![tip, tip - dir * s + side, tip - dir * s - side], stroke.color, Stroke::NONE));
        }

        // Markers: which columns carry a link.
        let mut linked: BTreeMap<(usize, usize), bool> = BTreeMap::new();
        for e in self.edges.iter().filter(|e| self.show_inferred || !e.inferred) {
            // Inferred only while no real foreign key starts at the column.
            *linked.entry((e.from, e.from_row)).or_insert(true) &= e.inferred;
        }

        // Boxes.
        for i in (0..visible.len()).filter(|&i| visible[i]) {
            let r = screen(self, i);
            if !rect.intersects(r) {
                continue;
            }
            let t = &self.diagram.tables[i];
            let round = (6.0 * z).clamp(2.0, 8.0) as u8;
            let lit = hovered_box == Some(i);
            painter.rect_filled(r, CornerRadius::same(round), theme::SURFACE());
            let kind_color = if matches!(t.kind, TableKind::Table | TableKind::Partitioned) { theme::ACCENT() } else { theme::VIEW() };
            let head = Rect::from_min_size(r.min, vec2(r.width(), HEAD_H * z));
            painter.rect_filled(head, CornerRadius { nw: round, ne: round, sw: 0, se: 0 }, theme::tint(theme::SURFACE(), kind_color, 0.25));
            painter.rect_stroke(r, CornerRadius::same(round), Stroke::new(1.0_f32, if lit { theme::ACCENT() } else { theme::BORDER() }), StrokeKind::Inside);
            if z < 0.3 {
                continue;
            }
            painter.text(pos2(r.left() + PAD * z, head.center().y), Align2::LEFT_CENTER, &t.name, FontId::proportional(13.0 * z), theme::TEXT());
            let shown = shown_rows(self, i);
            for (ri, c) in t.columns.iter().take(shown).enumerate() {
                let y = r.top() + (HEAD_H + (ri as f32 + 0.5) * ROW_H) * z;
                if c.primary_key {
                    if z >= 0.7 {
                        widgets::paint_icon(&painter, pos2(r.left() + 11.0 * z, y), Icon::Key);
                    } else {
                        painter.circle_filled(pos2(r.left() + 11.0 * z, y), 2.5 * z, theme::WARN());
                    }
                }
                if let Some(&inferred) = linked.get(&(i, ri)) {
                    let at = pos2(r.left() + 24.0 * z, y);
                    if inferred {
                        painter.circle_stroke(at, 2.8 * z, Stroke::new(1.2_f32, theme::ACCENT()));
                    } else {
                        painter.circle_filled(at, 2.8 * z, theme::ACCENT());
                    }
                }
                painter.text(pos2(r.left() + MARK_W * z, y), Align2::LEFT_CENTER, &c.name, FontId::proportional(12.0 * z), theme::TEXT());
                painter.text(pos2(r.right() - PAD * z, y), Align2::RIGHT_CENTER, short_type(&c.data_type), FontId::proportional(11.0 * z), theme::FAINT());
            }
            if t.columns.len() > shown {
                let y = r.top() + (HEAD_H + (shown as f32 + 0.5) * ROW_H) * z;
                painter.text(pos2(r.left() + MARK_W * z, y), Align2::LEFT_CENTER, format!("+{} more", t.columns.len() - shown), FontId::proportional(11.0 * z), theme::FAINT());
            }
        }

        // The hovered line's relation.
        if self.export.is_none()
            && let (Some(ei), Some(p)) = (hovered_edge, pointer)
        {
            let e = &self.edges[ei];
            let l = &self.diagram.links[e.link];
            let cols = l.columns.iter().map(|(a, b)| format!("{}.{a} → {}.{b}", self.diagram.tables[l.from].name, self.diagram.tables[l.to].name)).collect::<Vec<_>>().join(", ");
            let text = if l.inferred { l.name.clone() } else { format!("{}\n{cols}", l.name) };
            let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("diagram-tip"));
            let tip = ui.ctx().layer_painter(layer);
            let galley = tip.layout_no_wrap(text, FontId::proportional(12.5), theme::TEXT());
            let back = Rect::from_min_size(p + vec2(14.0, 14.0), galley.size() + vec2(16.0, 10.0));
            tip.rect(back, 6.0, theme::POPUP(), Stroke::new(1.0_f32, theme::BORDER()), StrokeKind::Inside);
            tip.galley(back.min + vec2(8.0, 5.0), galley, theme::TEXT());
        }

        self.export_step(ui, rect);
        out
    }

    /// PNG export: capture the canvas as drawn (the toolbar is outside it).
    fn export_step(&mut self, ui: &Ui, rect: Rect) {
        match self.export {
            Some(Export::Hide) => {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                self.export = Some(Export::Wait(Instant::now()));
                ui.ctx().request_repaint();
            }
            Some(Export::Wait(since)) => {
                let shot = ui.input(|i| {
                    i.events.iter().find_map(|ev| match ev {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(screen) = shot {
                    let image = mx::crop(&screen, rect, ui.ctx().pixels_per_point());
                    let name = format!("{}_diagram.png", self.schema);
                    self.export = None;
                    if let Some(path) = rfd::FileDialog::new().set_file_name(&name).add_filter("PNG image", &["png"]).save_file() {
                        let message = match mx::png(&image).and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string())) {
                            Ok(()) => format!("Saved {}", path.file_name().map_or(name, |n| n.to_string_lossy().into_owned())),
                            Err(err) => format!("Couldn't save the diagram: {err}"),
                        };
                        self.notice = Some((message, Instant::now()));
                    }
                } else if since.elapsed() > Duration::from_secs(3) {
                    self.export = None;
                    self.notice = Some(("Couldn't capture the diagram".into(), Instant::now()));
                } else {
                    ui.ctx().request_repaint();
                }
            }
            None => {}
        }
        if let Some((text, at)) = &self.notice {
            if at.elapsed() > Duration::from_millis(2500) {
                self.notice = None;
            } else if self.export.is_none() {
                let galley = ui.painter().layout_no_wrap(text.clone(), FontId::proportional(12.5), theme::TEXT());
                let back = Rect::from_center_size(rect.center_bottom() - vec2(0.0, 40.0), galley.size() + vec2(24.0, 12.0));
                ui.painter().rect(back, 8.0, theme::POPUP(), Stroke::new(1.0_f32, theme::BORDER()), StrokeKind::Inside);
                ui.painter().galley(back.center() - galley.size() / 2.0, galley, theme::TEXT());
                ui.ctx().request_repaint_after(Duration::from_millis(200));
            }
        }
    }
}

/// Distance from `p` to the segment `a`-`b`.
fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let t = if len2 == 0.0 { 0.0 } else { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) };
    (a + ab * t - p).length()
}

/// Long type names that crowd a box.
fn short_type(ty: &str) -> String {
    ty.replace("character varying", "varchar").replace("timestamp without time zone", "timestamp").replace("timestamp with time zone", "timestamptz").replace("double precision", "float8")
}
