//! The Map layout tab: a page with a map of several table layers, a title,
//! legend, scale bar and north arrow you drag into place, exported as PNG or
//! PDF. The page is one SVG from `rgrip_core::layout`; a background thread
//! renders it for the preview and for exports, so they match.

use crate::{snapshot, theme};
use eframe::egui::{self, Align2, Color32, ColorImage, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, TextEdit, TextureHandle, Ui, Vec2, pos2, vec2};
use rgrip_core::complete::Catalog;
use rgrip_core::geo::Geometry;
use rgrip_core::layout::{self, Basemap, Element, Layer, Layout, PageSize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Colors for new layers, in order.
const PALETTE: [[u8; 3]; 8] = [[214, 69, 65], [41, 128, 185], [39, 174, 96], [142, 68, 173], [230, 126, 34], [22, 160, 133], [192, 57, 43], [52, 73, 94]];
/// Largest PNG export, in pixels.
const MAX_PIXELS: f64 = 120_000_000.0;

pub enum Action {
    /// Load a layer's geometry (`Cmd::LayerData`).
    Load { layer: u64, sql: String },
}

#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    Png,
    Pdf,
}

enum Job {
    Preview { version: u64, layout: Layout, px_per_mm: f32 },
    Export { layout: Layout, format: Format, dpi: f64, path: PathBuf },
}

enum Done {
    Preview { version: u64, image: ColorImage },
    Exported(Result<PathBuf, String>),
}

enum Drag {
    /// An element, grabbed at an offset from its top-left (mm).
    Element(Element, [f64; 2]),
    /// The map, panned from the last pointer position (mm).
    Pan([f64; 2]),
}

pub struct LayoutTab {
    pub layout: Layout,
    next_id: u64,
    /// Per layer: features loaded, or the error.
    notes: HashMap<u64, (String, bool)>,
    loading: HashSet<u64>,
    open_layer: Option<u64>,
    /// Bumped on every change; the preview catches up.
    version: u64,
    sent: u64,
    texture: Option<(u64, TextureHandle)>,
    jobs: mpsc::Sender<Job>,
    done: mpsc::Receiver<Done>,
    drag: Option<Drag>,
    pub format: Format,
    pub dpi: u32,
    exporting: bool,
    status: Option<(String, bool, Instant)>,
    /// Fit the map to the data once the first layer arrives.
    fit_pending: bool,
    add_search: String,
    /// Elements the user has dragged; the rest stay at their usual spots as
    /// their content changes.
    moved: HashSet<Element>,
}

impl LayoutTab {
    pub fn new(ctx: &egui::Context) -> Self {
        let (jobs, rx) = mpsc::channel::<Job>();
        let (tx, done) = mpsc::channel::<Done>();
        let ctx = ctx.clone();
        std::thread::Builder::new().name("map-layout".into()).spawn(move || render_loop(rx, tx, ctx)).ok();
        Self {
            layout: Layout::default(),
            next_id: 1,
            notes: HashMap::new(),
            loading: HashSet::new(),
            open_layer: None,
            version: 1,
            sent: 0,
            texture: None,
            jobs,
            done,
            drag: None,
            format: Format::Pdf,
            dpi: 300,
            exporting: false,
            status: None,
            fit_pending: true,
            add_search: String::new(),
            moved: HashSet::new(),
        }
    }

    /// Add a table's geometry column as a layer; returns the load to start.
    pub fn add_layer(&mut self, schema: &str, table: &str, column: &str) -> Action {
        let id = self.next_id;
        self.next_id += 1;
        let color = PALETTE[(id as usize - 1) % PALETTE.len()];
        let layer = Layer::new(id, schema, table, column, color);
        let sql = layer.sql(crate::worker::LAYER_LIMIT + 1);
        // New layers go on top.
        self.layout.layers.insert(0, layer);
        self.loading.insert(id);
        self.changed();
        Action::Load { layer: id, sql }
    }

    pub fn layer_loaded(&mut self, id: u64, result: Result<(Vec<Geometry>, usize, bool), String>) {
        self.loading.remove(&id);
        let Some(layer) = self.layout.layers.iter_mut().find(|l| l.id == id) else { return };
        match result {
            Ok((shapes, skipped, truncated)) => {
                let mut note = crate::app::plural(shapes.len(), "feature");
                if truncated {
                    note.push_str(&format!(" (first {})", layout::thousands(crate::worker::LAYER_LIMIT as f64)));
                }
                if skipped > 0 {
                    note.push_str(&format!(", {skipped} in an SRID that can't be drawn"));
                }
                layer.shapes = shapes.into();
                self.notes.insert(id, (note, false));
                // Fit once the first batch of layers is in, to all of them.
                if self.fit_pending && self.loading.is_empty() && self.layout.layers.iter().any(|l| !l.shapes.is_empty()) {
                    self.fit_pending = false;
                    self.layout.fit();
                }
            }
            Err(e) => {
                self.notes.insert(id, (e, true));
            }
        }
        self.changed();
    }

    fn changed(&mut self) {
        self.version += 1;
        for e in Element::ALL {
            if !self.moved.contains(&e) {
                self.layout.place(e);
            }
        }
    }

    fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some((text.into(), error, Instant::now()));
    }

    pub fn ui(&mut self, ui: &mut Ui, catalog: &Catalog) -> Vec<Action> {
        let mut actions = Vec::new();
        while let Ok(done) = self.done.try_recv() {
            match done {
                Done::Preview { version, image } => {
                    let tex = ui.ctx().load_texture("map-layout-preview", image, egui::TextureOptions::LINEAR);
                    self.texture = Some((version, tex));
                }
                Done::Exported(r) => {
                    self.exporting = false;
                    match r {
                        Ok(path) => self.notify(format!("Exported {}", path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())), false),
                        Err(e) => self.notify(format!("Export failed: {e}"), true),
                    }
                }
            }
        }

        egui::SidePanel::left(egui::Id::new(("layout-side", self as *const Self as usize)))
            .resizable(false)
            .exact_width(300.0)
            .frame(egui::Frame::new().fill(theme::SURFACE()).inner_margin(egui::Margin::same(14)))
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    self.side(ui, catalog, &mut actions);
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG())).show_inside(ui, |ui| self.canvas(ui));
        actions
    }

    fn side(&mut self, ui: &mut Ui, catalog: &Catalog, actions: &mut Vec<Action>) {
        let head = |ui: &mut Ui, text: &str| {
            ui.add_space(4.0);
            ui.label(RichText::new(text).size(12.0).strong().color(theme::MUTED()));
            ui.add_space(2.0);
        };
        let before = self.layout.clone();

        head(ui, "PAGE");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("layout-page").width(90.0).selected_text(self.layout.page.label()).show_ui(ui, |ui| {
                for p in PageSize::ALL {
                    let (w, h) = p.mm();
                    ui.selectable_value(&mut self.layout.page, p, format!("{}  {:.0} × {:.0} mm", p.label(), w, h));
                }
            });
            ui.selectable_value(&mut self.layout.landscape, false, "Portrait");
            ui.selectable_value(&mut self.layout.landscape, true, "Landscape");
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Margin").color(theme::MUTED()));
            ui.add(egui::DragValue::new(&mut self.layout.margin).range(0.0..=40.0).speed(0.5).suffix(" mm"));
        });

        ui.add_space(8.0);
        head(ui, "MAP");
        ui.horizontal(|ui| {
            ui.label(RichText::new("Basemap").color(theme::MUTED()));
            egui::ComboBox::from_id_salt("layout-basemap").width(110.0).selected_text(self.layout.basemap.label()).show_ui(ui, |ui| {
                for b in Basemap::ALL {
                    ui.selectable_value(&mut self.layout.basemap, b, b.label());
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Scale 1:").color(theme::MUTED()));
            let speed = self.layout.scale * 0.005;
            ui.add(egui::DragValue::new(&mut self.layout.scale).range(100.0..=50_000_000.0).speed(speed).max_decimals(0));
            if ui.button("Fit").on_hover_text("Fit the map to the layers").clicked() {
                self.layout.fit();
            }
        });
        ui.label(RichText::new("Drag the map to pan, scroll to zoom").size(11.0).color(theme::FAINT()));

        ui.add_space(8.0);
        head(ui, "LAYERS");
        self.add_layer_ui(ui, catalog, actions);
        let mut remove = None;
        let mut swap = None;
        let count = self.layout.layers.len();
        for i in 0..count {
            let id = self.layout.layers[i].id;
            let open = self.open_layer == Some(id);
            let note = self.notes.get(&id).cloned();
            let loading = self.loading.contains(&id);
            let layer = &mut self.layout.layers[i];
            egui::Frame::new().fill(theme::BG()).corner_radius(6).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.checkbox(&mut layer.visible, "");
                    ui.color_edit_button_srgb(&mut layer.color);
                    ui.add(TextEdit::singleline(&mut layer.label).desired_width(120.0).hint_text("Legend label"));
                    if ui.small_button(if open { "▾" } else { "▸" }).on_hover_text("Style and filter").clicked() {
                        self.open_layer = if open { None } else { Some(id) };
                    }
                });
                let sub = format!("{}.{} · {}", layer.schema, layer.table, layer.column);
                ui.label(RichText::new(sub).size(11.0).color(theme::FAINT()));
                if loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Loading…").size(11.5).color(theme::MUTED()));
                    });
                } else if let Some((text, err)) = &note {
                    ui.label(RichText::new(text).size(11.5).color(if *err { theme::ERROR() } else { theme::MUTED() }));
                }
                if open {
                    ui.add_space(4.0);
                    egui::Grid::new(("layer-style", id)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                        ui.label(RichText::new("Line").color(theme::MUTED()));
                        ui.add(egui::Slider::new(&mut layer.width, 0.05..=2.0).suffix(" mm"));
                        ui.end_row();
                        ui.label(RichText::new("Point").color(theme::MUTED()));
                        ui.add(egui::Slider::new(&mut layer.point, 0.3..=5.0).suffix(" mm"));
                        ui.end_row();
                        ui.label(RichText::new("Fill").color(theme::MUTED()));
                        ui.add(egui::Slider::new(&mut layer.fill_opacity, 0.0..=1.0));
                        ui.end_row();
                    });
                    ui.label(RichText::new("Filter (SQL WHERE)").size(11.5).color(theme::MUTED()));
                    let filter = ui.add(TextEdit::singleline(&mut layer.filter).hint_text("land_use = 'garden'").font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
                    ui.horizontal(|ui| {
                        let apply = ui.small_button("Apply filter").clicked() || (filter.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                        if apply {
                            actions.push(Action::Load { layer: id, sql: layer.sql(crate::worker::LAYER_LIMIT + 1) });
                            self.loading.insert(id);
                        }
                        if i > 0 && ui.small_button("Up").clicked() {
                            swap = Some((i, i - 1));
                        }
                        if i + 1 < count && ui.small_button("Down").clicked() {
                            swap = Some((i, i + 1));
                        }
                        if ui.small_button(RichText::new("Remove").color(theme::ERROR())).clicked() {
                            remove = Some(i);
                        }
                    });
                }
            });
            ui.add_space(4.0);
        }
        if let Some((a, b)) = swap {
            self.layout.layers.swap(a, b);
        }
        if let Some(i) = remove {
            let id = self.layout.layers.remove(i).id;
            self.notes.remove(&id);
            self.loading.remove(&id);
        }

        ui.add_space(8.0);
        head(ui, "ON THE PAGE");
        ui.label(RichText::new("Drag them on the page to place them").size(11.0).color(theme::FAINT()));
        let mut title_vis = self.layout.item(Element::Title).visible;
        ui.horizontal(|ui| {
            ui.checkbox(&mut title_vis, "Title");
            ui.add(egui::DragValue::new(&mut self.layout.title_size).range(8.0..=60.0).suffix(" pt"));
        });
        self.layout.item_mut(Element::Title).visible = title_vis;
        ui.add(TextEdit::singleline(&mut self.layout.title).hint_text("Map title").desired_width(f32::INFINITY));
        let mut legend_vis = self.layout.item(Element::Legend).visible;
        ui.checkbox(&mut legend_vis, "Legend");
        self.layout.item_mut(Element::Legend).visible = legend_vis;
        ui.add(TextEdit::singleline(&mut self.layout.legend_title).hint_text("Legend title").desired_width(f32::INFINITY));
        for (e, name) in [(Element::ScaleBar, "Scale bar"), (Element::North, "North arrow")] {
            let mut v = self.layout.item(e).visible;
            ui.checkbox(&mut v, name);
            self.layout.item_mut(e).visible = v;
        }
        if ui.small_button("Reset positions").clicked() {
            self.moved.clear();
            self.layout.place_defaults();
        }

        ui.add_space(10.0);
        head(ui, "EXPORT");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.format, Format::Pdf, "PDF");
            ui.selectable_value(&mut self.format, Format::Png, "PNG");
            if self.format == Format::Png {
                egui::ComboBox::from_id_salt("layout-dpi").width(80.0).selected_text(format!("{} dpi", self.dpi)).show_ui(ui, |ui| {
                    for d in [150, 300, 600] {
                        ui.selectable_value(&mut self.dpi, d, format!("{d} dpi"));
                    }
                });
            }
        });
        let (w, h) = self.layout.page_mm();
        let hint = match self.format {
            Format::Pdf => format!("{} {}, vector shapes and text", self.layout.page.label(), if self.layout.landscape { "landscape" } else { "portrait" }),
            Format::Png => format!("{} × {} px", (w / 25.4 * self.dpi as f64).round(), (h / 25.4 * self.dpi as f64).round()),
        };
        ui.label(RichText::new(hint).size(11.0).color(theme::FAINT()));
        ui.add_space(4.0);
        let busy = self.exporting || !self.loading.is_empty();
        let label = if self.exporting { "Exporting…" } else { "Export…" };
        if ui.add_enabled(!busy, egui::Button::new(RichText::new(label).color(theme::BG())).fill(theme::ACCENT())).clicked() {
            self.export();
        }
        if let Some((text, err, at)) = &self.status {
            if at.elapsed() < Duration::from_secs(6) {
                ui.label(RichText::new(text).size(12.0).color(if *err { theme::ERROR() } else { theme::OK() }));
            }
        }

        if self.layout.page != before.page || self.layout.landscape != before.landscape || self.layout.margin != before.margin {
            self.layout.clamp_items();
        }
        if !same(&before, &self.layout) {
            self.changed();
        }
    }

    fn add_layer_ui(&mut self, ui: &mut Ui, catalog: &Catalog, actions: &mut Vec<Action>) {
        let geo: Vec<(&str, &str, &str)> = catalog
            .tables
            .iter()
            .flat_map(|t| {
                t.columns
                    .iter()
                    .filter(|(_, ty)| ty.starts_with("geometry") || ty.starts_with("geography"))
                    .map(move |(c, _)| (t.schema.as_str(), t.name.as_str(), c.as_str()))
            })
            .collect();
        let mut add = None;
        egui::ComboBox::from_id_salt("layout-add")
            .width(ui.available_width() - 4.0)
            .selected_text(RichText::new("+ Add a table…").color(theme::ACCENT()))
            .show_ui(ui, |ui| {
                if geo.len() > 10 {
                    ui.add(TextEdit::singleline(&mut self.add_search).hint_text("Filter…").desired_width(220.0));
                }
                let needle = self.add_search.to_lowercase();
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    if geo.is_empty() {
                        ui.label(RichText::new("No tables with a geometry column").color(theme::MUTED()));
                    }
                    for (s, t, c) in geo.iter().filter(|(s, t, _)| format!("{s}.{t}").to_lowercase().contains(&needle)) {
                        let label = if geo.iter().filter(|(s2, t2, _)| s2 == s && t2 == t).count() > 1 { format!("{s}.{t} · {c}") } else { format!("{s}.{t}") };
                        if ui.selectable_label(false, label).clicked() {
                            add = Some((s.to_string(), t.to_string(), c.to_string()));
                        }
                    }
                });
            });
        if let Some((s, t, c)) = add {
            actions.push(self.add_layer(&s, &t, &c));
        }
        ui.add_space(4.0);
    }

    /// Debug screenshot scenes: export straight to `path` once every layer
    /// has loaded. Returns true when started.
    #[cfg(debug_assertions)]
    pub fn demo_export(&mut self, path: PathBuf) -> bool {
        if !self.loading.is_empty() || self.layout.layers.is_empty() || self.exporting {
            return false;
        }
        self.format = if path.extension().is_some_and(|e| e == "png") { Format::Png } else { Format::Pdf };
        self.exporting = true;
        let dpi = if self.format == Format::Pdf { 300.0 } else { self.dpi as f64 };
        let _ = self.jobs.send(Job::Export { layout: self.layout.clone(), format: self.format, dpi, path });
        true
    }

    /// Debug: an export has finished.
    #[cfg(debug_assertions)]
    pub fn demo_exported(&self) -> bool {
        !self.exporting && self.status.is_some()
    }

    fn export(&mut self) {
        let (ext, filter) = match self.format {
            Format::Pdf => ("pdf", "PDF"),
            Format::Png => ("png", "PNG image"),
        };
        let base = if self.layout.title.trim().is_empty() { "map".to_string() } else { self.layout.title.trim().replace(['/', ':', '\\'], "-") };
        let Some(path) = rfd::FileDialog::new().set_file_name(format!("{base}.{ext}")).add_filter(filter, &[ext]).save_file() else { return };
        let (w, h) = self.layout.page_mm();
        let px = (w / 25.4 * self.dpi as f64) * (h / 25.4 * self.dpi as f64);
        if self.format == Format::Png && px > MAX_PIXELS {
            self.notify("That's too many pixels: pick a lower dpi or a smaller page", true);
            return;
        }
        self.exporting = true;
        let dpi = if self.format == Format::Pdf { 300.0 } else { self.dpi as f64 };
        let _ = self.jobs.send(Job::Export { layout: self.layout.clone(), format: self.format, dpi, path });
    }

    fn canvas(&mut self, ui: &mut Ui) {
        let area = ui.available_rect_before_wrap().shrink(24.0);
        let (pw, ph) = self.layout.page_mm();
        let scale = (area.width() / pw as f32).min(area.height() / ph as f32).max(0.5);
        let page = Rect::from_center_size(area.center(), vec2(pw as f32 * scale, ph as f32 * scale));
        let to_screen = |p: [f64; 2]| pos2(page.left() + p[0] as f32 * scale, page.top() + p[1] as f32 * scale);
        let to_mm = |p: Pos2| [((p.x - page.left()) / scale) as f64, ((p.y - page.top()) / scale) as f64];
        let painter = ui.painter_at(ui.max_rect());

        // Ask for a fresh preview once the last one is out of date.
        if self.sent != self.version {
            let px_per_mm = scale * ui.ctx().pixels_per_point();
            let _ = self.jobs.send(Job::Preview { version: self.version, layout: self.layout.clone(), px_per_mm });
            self.sent = self.version;
        }

        painter.rect_filled(page.translate(vec2(0.0, 6.0)).expand(2.0), 4.0, Color32::from_black_alpha(90));
        painter.rect_filled(page, 0.0, Color32::WHITE);
        if let Some((_, tex)) = &self.texture {
            painter.image(tex.id(), page, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        let stale = self.texture.as_ref().is_none_or(|(v, _)| *v != self.version);
        if stale {
            painter.text(page.right_bottom() + vec2(-8.0, -8.0), Align2::RIGHT_BOTTOM, "Rendering…", FontId::proportional(11.0), Color32::from_gray(120));
        }

        // Interaction: elements first (on top), then the map frame.
        let response = ui.interact(page, egui::Id::new(("layout-page", self as *const Self as usize)), Sense::click_and_drag());
        let boxes = self.layout.boxes();
        let hover = response.hover_pos().map(to_mm);
        let element_at = |p: [f64; 2], layout: &Layout| {
            Element::ALL.into_iter().rev().find(|e| layout.item(*e).visible && boxes[*e as usize].contains(p) && (*e != Element::Title || !layout.title.trim().is_empty()))
        };
        // Where the press began: egui reports the drag a few pixels later.
        if response.drag_started()
            && let Some(p) = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos()).map(to_mm)
        {
            self.drag = match element_at(p, &self.layout) {
                Some(e) => {
                    let pos = self.layout.item(e).pos;
                    Some(Drag::Element(e, [p[0] - pos[0], p[1] - pos[1]]))
                }
                None if self.layout.frame().contains(p) => Some(Drag::Pan(p)),
                None => None,
            };
        }
        if response.dragged()
            && let Some(p) = response.interact_pointer_pos().map(to_mm)
        {
            match &mut self.drag {
                Some(Drag::Element(e, grab)) => {
                    self.moved.insert(*e);
                    let b = boxes[*e as usize];
                    let mut pos = [p[0] - grab[0], p[1] - grab[1]];
                    pos = snap(pos, [b.w, b.h], &self.layout);
                    self.layout.item_mut(*e).pos = pos;
                    self.layout.clamp_items();
                    self.version += 1;
                }
                Some(Drag::Pan(last)) => {
                    let d = [p[0] - last[0], p[1] - last[1]];
                    *last = p;
                    self.layout.pan(d);
                    self.changed();
                }
                None => {}
            }
        }
        if response.drag_stopped() {
            self.drag = None;
        }
        if let Some(p) = hover
            && self.layout.frame().contains(p)
        {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 {
                self.layout.zoom_at((scroll as f64 / 200.0).exp(), p);
                self.changed();
            }
        }

        // Outlines: the hovered or dragged element, so it reads as movable.
        let active = match &self.drag {
            Some(Drag::Element(e, _)) => Some(*e),
            _ => hover.and_then(|p| element_at(p, &self.layout)),
        };
        if let Some(e) = active {
            let b = self.layout.boxes()[e as usize];
            let r = Rect::from_min_max(to_screen([b.x, b.y]), to_screen([b.x + b.w, b.y + b.h])).expand(2.0);
            painter.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.5, theme::ACCENT()), egui::StrokeKind::Outside);
            ui.ctx().set_cursor_icon(if matches!(self.drag, Some(Drag::Element(..))) { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        } else if hover.is_some_and(|p| self.layout.frame().contains(p)) {
            ui.ctx().set_cursor_icon(if matches!(self.drag, Some(Drag::Pan(_))) { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Move });
        }
        if self.layout.layers.is_empty() {
            let f = self.layout.frame();
            painter.text(
                to_screen([f.x + f.w / 2.0, f.y + f.h / 2.0]),
                Align2::CENTER_CENTER,
                "Add a table with a geometry column (left)",
                FontId::proportional(14.0),
                Color32::from_gray(110),
            );
        }
        let _ = Vec2::ZERO;
    }
}

/// Snap an element's top-left to the frame's edges and center lines when
/// within 2 mm (with a 5 mm inset, the default spacing).
fn snap(pos: [f64; 2], size: [f64; 2], layout: &Layout) -> [f64; 2] {
    let f = layout.frame();
    let inset = 5.0;
    let xs = [f.x + inset, f.x + f.w - inset - size[0], f.x + (f.w - size[0]) / 2.0];
    let ys = [f.y + inset, f.y + f.h - inset - size[1], f.y + (f.h - size[1]) / 2.0];
    let pick = |v: f64, targets: &[f64]| targets.iter().copied().find(|t| (t - v).abs() < 2.0).unwrap_or(v);
    [pick(pos[0], &xs), pick(pos[1], &ys)]
}

/// Layouts compare by everything that changes the picture (shapes by
/// identity: they're replaced, never edited).
fn same(a: &Layout, b: &Layout) -> bool {
    a.page == b.page
        && a.landscape == b.landscape
        && a.margin == b.margin
        && a.center == b.center
        && a.scale == b.scale
        && a.basemap == b.basemap
        && a.title == b.title
        && a.title_size == b.title_size
        && a.legend_title == b.legend_title
        && a.items == b.items
        && a.layers.len() == b.layers.len()
        && a.layers.iter().zip(&b.layers).all(|(x, y)| {
            x.id == y.id
                && x.label == y.label
                && x.color == y.color
                && x.width == y.width
                && x.point == y.point
                && x.fill_opacity == y.fill_opacity
                && x.visible == y.visible
                && std::sync::Arc::ptr_eq(&x.shapes, &y.shapes)
        })
}

/// The SVG for `layout`, with tiles from `cache`, filled from disk and (with
/// `network`) downloaded as needed. Also returns whether tiles are missing.
fn page_svg(layout: &Layout, dpi: f64, cache: &mut HashMap<(Basemap, u8, u32, u32), Vec<u8>>, network: bool) -> (String, bool) {
    let (z, tiles) = layout.tiles(dpi);
    let missing: Vec<(u32, u32)> = tiles.iter().copied().filter(|(x, y)| !cache.contains_key(&(layout.basemap, z, *x, *y))).collect();
    let mut incomplete = false;
    if !missing.is_empty() {
        let got = layout::fetch_tiles(layout.basemap, z, &missing, network);
        incomplete = got.len() < missing.len();
        for ((x, y), bytes) in got {
            cache.insert((layout.basemap, z, x, y), bytes);
        }
    }
    if cache.len() > 2000 {
        cache.clear();
    }
    let basemap = layout.basemap;
    (layout.svg(&mut |z, x, y| cache.get(&(basemap, z, x, y)).cloned(), dpi), incomplete)
}

fn render_loop(jobs: mpsc::Receiver<Job>, done: mpsc::Sender<Done>, ctx: egui::Context) {
    let mut cache = HashMap::new();
    while let Ok(first) = jobs.recv() {
        // Only the newest preview matters; exports all run.
        let mut queue = vec![first];
        while let Ok(more) = jobs.try_recv() {
            queue.push(more);
        }
        let latest_preview = queue.iter().rposition(|j| matches!(j, Job::Preview { .. }));
        for (i, job) in queue.into_iter().enumerate() {
            match job {
                Job::Preview { version, layout, px_per_mm } => {
                    if Some(i) != latest_preview {
                        continue;
                    }
                    let dpi = (px_per_mm as f64 * 25.4).clamp(48.0, 150.0);
                    let render = |svg: &str| {
                        // The SVG is in mm; usvg sizes it at 96 px per inch.
                        if let Some(p) = snapshot::render_at(svg, px_per_mm * 25.4 / 96.0) {
                            let image = ColorImage::from_rgba_premultiplied([p.width() as usize, p.height() as usize], p.data());
                            let _ = done.send(Done::Preview { version, image });
                            ctx.request_repaint();
                        }
                    };
                    // Draw now with the tiles at hand, then again once the
                    // missing ones are downloaded (unless something newer came).
                    let (svg, incomplete) = page_svg(&layout, dpi, &mut cache, false);
                    render(&svg);
                    if incomplete {
                        let (svg, _) = page_svg(&layout, dpi, &mut cache, true);
                        render(&svg);
                    }
                }
                Job::Export { layout, format, dpi, path } => {
                    let (svg, _) = page_svg(&layout, dpi, &mut cache, true);
                    let result = match format {
                        Format::Png => snapshot::render_at(&svg, (dpi / 96.0) as f32).ok_or_else(|| "couldn't render the page".to_string()).and_then(|p| p.encode_png().map_err(|e| e.to_string())),
                        Format::Pdf => pdf(&svg),
                    }
                    .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()))
                    .map(|_| path);
                    let _ = done.send(Done::Exported(result));
                    ctx.request_repaint();
                }
            }
        }
    }
}

/// The page as a PDF: shapes and text stay vector, tiles are images.
fn pdf(svg: &str) -> Result<Vec<u8>, String> {
    use svg2pdf::usvg;
    let mut db = usvg::fontdb::Database::new();
    db.load_system_fonts();
    let options = usvg::Options { fontdb: std::sync::Arc::new(db), ..Default::default() };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|e| e.to_string())?;
    // usvg sizes the mm page at 96 px per inch; dpi 96 keeps it to size.
    let page = svg2pdf::PageOptions { dpi: 96.0 };
    svg2pdf::to_pdf(&tree, svg2pdf::ConversionOptions::default(), page).map_err(|e| format!("{e:?}"))
}
