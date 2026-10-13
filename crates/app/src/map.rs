//! Built-in map for PostGIS results: slippy-map basemap (walkers) with the
//! result's geometries drawn on top.

use crate::{shaping, theme};
use eframe::egui::{
    self, Align2, Color32, FontId, Frame, Margin, Mesh, Pos2, Rect, RichText, Shape, Stroke, Ui, Vec2,
    vec2,
};
use rgrip_core::ResultSet;
use rgrip_core::geo::{self, Coord, Geometry};
use std::collections::HashMap;
use walkers::sources::{Attribution, OpenStreetMap, TileSource};
use walkers::{HttpOptions, HttpTiles, MapMemory, Plugin, Projector, TileId, lon_lat};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Basemap {
    Dark,
    Light,
    Satellite,
}

impl Basemap {
    const ALL: [Basemap; 3] = [Basemap::Dark, Basemap::Light, Basemap::Satellite];

    fn label(self) -> &'static str {
        match self {
            Basemap::Dark => "Dark",
            Basemap::Light => "Streets",
            Basemap::Satellite => "Satellite",
        }
    }
}

/// Esri Dark Gray Canvas: a quiet basemap that suits the dark UI (no API key).
struct EsriDarkGray;

impl TileSource for EsriDarkGray {
    fn tile_url(&self, t: TileId) -> String {
        format!(
            "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Base/MapServer/tile/{}/{}/{}",
            t.zoom, t.y, t.x
        )
    }
    fn attribution(&self) -> Attribution {
        Attribution { text: "Esri, HERE, Garmin, © OpenStreetMap contributors", url: "https://www.esri.com", logo_light: None, logo_dark: None }
    }
    fn max_zoom(&self) -> u8 {
        16
    }
}

/// Google satellite. `scale=2` returns 512 px images for the same 256 px
/// tile grid, so tiles render at double density (sharp on Retina).
struct GoogleSatellite;

impl TileSource for GoogleSatellite {
    fn tile_url(&self, t: TileId) -> String {
        format!("https://mt1.google.com/vt/lyrs=s&x={}&y={}&z={}&scale=2", t.x, t.y, t.zoom)
    }
    fn attribution(&self) -> Attribution {
        Attribution { text: "Imagery © Google", url: "https://www.google.com/maps", logo_light: None, logo_dark: None }
    }
    fn max_zoom(&self) -> u8 {
        20
    }
}

fn attribution(b: Basemap) -> &'static str {
    match b {
        Basemap::Dark => EsriDarkGray.attribution().text,
        Basemap::Light => "© OpenStreetMap contributors",
        Basemap::Satellite => GoogleSatellite.attribution().text,
    }
}

/// One drawable feature (a result row), in WGS84.
struct Feature {
    row: usize,
    points: Vec<Coord>,
    lines: Vec<Vec<Coord>>,
    /// Triangulated polygons: vertices and triangle indices, plus rings for outlines.
    fills: Vec<(Vec<Coord>, Vec<usize>)>,
    rings: Vec<Vec<Coord>>,
}

pub struct MapData {
    features: Vec<Feature>,
    bounds: Option<(Coord, Coord)>,
    /// Rows whose SRID we can't convert, with the SRIDs seen.
    skipped: usize,
    srids: Vec<i32>,
}

impl MapData {
    pub fn build(rs: &ResultSet, col: usize) -> Self {
        let mut data = MapData { features: Vec::new(), bounds: None, skipped: 0, srids: Vec::new() };
        for (row, values) in rs.rows.iter().enumerate() {
            let Some(g) = values[col].as_deref().and_then(geo::parse_hex) else { continue };
            let Some(wgs) = g.to_wgs84() else {
                data.skipped += 1;
                if let Some(s) = g.srid.filter(|s| !data.srids.contains(s)) {
                    data.srids.push(s);
                }
                continue;
            };
            let mut f = Feature { row, points: Vec::new(), lines: Vec::new(), fills: Vec::new(), rings: Vec::new() };
            add_geometry(&mut f, &wgs);
            wgs.for_each_coord(&mut |c| {
                if c[0].is_finite() && c[1].is_finite() {
                    data.bounds = Some(match data.bounds {
                        None => (c, c),
                        Some((lo, hi)) => ([lo[0].min(c[0]), lo[1].min(c[1])], [hi[0].max(c[0]), hi[1].max(c[1])]),
                    });
                }
            });
            data.features.push(f);
        }
        data
    }
}

fn add_geometry(f: &mut Feature, g: &Geometry) {
    match g {
        Geometry::Point(c) if c[0].is_finite() => f.points.push(*c),
        Geometry::Point(_) => {}
        Geometry::MultiPoint(ps) => f.points.extend(ps),
        Geometry::LineString(l) => f.lines.push(l.clone()),
        Geometry::MultiLineString(ls) => f.lines.extend(ls.iter().cloned()),
        Geometry::Polygon(p) => add_polygon(f, p),
        Geometry::MultiPolygon(mp) => mp.iter().for_each(|p| add_polygon(f, p)),
        Geometry::Collection(gs) => gs.iter().for_each(|g| add_geometry(f, g)),
    }
}

fn add_polygon(f: &mut Feature, rings: &[Vec<Coord>]) {
    let mut flat = Vec::new();
    let mut holes = Vec::new();
    let mut verts = Vec::new();
    for (i, ring) in rings.iter().enumerate() {
        if i > 0 {
            holes.push(verts.len());
        }
        for c in ring {
            flat.extend([c[0], c[1]]);
            verts.push(*c);
        }
    }
    if let Ok(tris) = earcutr::earcut(&flat, &holes, 2) {
        f.fills.push((verts, tris));
    }
    f.rings.extend(rings.iter().cloned());
}

pub struct MapView {
    tiles: HashMap<Basemap, HttpTiles>,
    memory: MapMemory,
    basemap: Basemap,
    /// Identity of the data the view was last fitted to.
    fitted: Option<usize>,
    pub selected: Option<usize>,
    column: usize,
    cache: Option<(usize, MapData)>,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            tiles: HashMap::new(),
            memory: MapMemory::default(),
            // Debug builds: RGRIP_DEMO_BASEMAP=satellite|streets for screenshots.
            basemap: match std::env::var("RGRIP_DEMO_BASEMAP").as_deref() {
                Ok("satellite") if cfg!(debug_assertions) => Basemap::Satellite,
                Ok("streets") if cfg!(debug_assertions) => Basemap::Light,
                _ => Basemap::Dark,
            },
            fitted: None,
            selected: None,
            column: 0,
            cache: None,
        }
    }
}

fn http_options() -> HttpOptions {
    let cache = std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join("Library/Caches/rgrip/tiles"));
    HttpOptions {
        cache,
        user_agent: Some(walkers::HeaderValue::from_static("rgrip/0.1 (desktop PostgreSQL client)")),
        ..Default::default()
    }
}

impl MapView {
    fn tiles(&mut self, ctx: &egui::Context) -> &mut HttpTiles {
        let basemap = self.basemap;
        self.tiles.entry(basemap).or_insert_with(|| match basemap {
            Basemap::Dark => HttpTiles::with_options(EsriDarkGray, http_options(), ctx.clone()),
            Basemap::Light => HttpTiles::with_options(OpenStreetMap, http_options(), ctx.clone()),
            Basemap::Satellite => HttpTiles::with_options(GoogleSatellite, http_options(), ctx.clone()),
        })
    }

    fn fit(&mut self, data: &MapData, size: Vec2) {
        let Some((lo, hi)) = data.bounds else { return };
        let center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        // World width at zoom z is 256·2^z px; fit the larger span with margin.
        let span_x = (hi[0] - lo[0]).max(1e-6) / 360.0;
        let merc = |lat: f64| (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln();
        let span_y = ((merc(hi[1]) - merc(lo[1])).abs() / (2.0 * std::f64::consts::PI)).max(1e-6);
        let zx = (size.x as f64 * 0.8 / 256.0 / span_x).log2();
        let zy = (size.y as f64 * 0.8 / 256.0 / span_y).log2();
        let zoom = zx.min(zy).clamp(1.0, 17.0);
        self.memory.center_at(lon_lat(center[0], center[1]));
        let _ = self.memory.set_zoom(zoom);
    }

    /// Show the map for `rs`; `geom_cols` are its geometry columns.
    pub fn show(&mut self, ui: &mut Ui, rs: &ResultSet, geom_cols: &[usize]) {
        if !geom_cols.contains(&self.column) {
            self.column = geom_cols[0];
        }
        // Re-decode only when the result or column changes.
        let key = rs as *const ResultSet as usize ^ rs.rows.len().rotate_left(17) ^ self.column.rotate_left(41);
        if self.cache.as_ref().is_none_or(|(k, _)| *k != key) {
            self.cache = Some((key, MapData::build(rs, self.column)));
            self.selected = None;
        }
        let (_, data) = self.cache.take().unwrap();

        let rect = ui.available_rect_before_wrap();
        if self.fitted != Some(key) {
            self.fit(&data, rect.size());
            self.fitted = Some(key);
        }

        let mut hovered = None;
        let mut clicked = None;
        let ctx = ui.ctx().clone();
        let selected = self.selected;
        let mut memory = std::mem::take(&mut self.memory);
        {
            let tiles = self.tiles(&ctx);
            let map = walkers::Map::new(Some(tiles), &mut memory, lon_lat(0.0, 0.0))
                .double_click_to_zoom(true)
                // Wheel/trackpad scroll zooms (drag pans) instead of scrolling the map.
                .zoom_with_ctrl(false)
                .with_plugin(Features { data: &data, selected, hovered: &mut hovered, clicked: &mut clicked });
            ui.put(rect, map);
        }
        self.memory = memory;
        if let Some(c) = clicked {
            self.selected = c;
        }

        self.overlay(ui, rect, &data, rs, geom_cols);
        if let Some(i) = hovered.filter(|_| self.selected.is_none() || hovered != self.selected) {
            let f = &data.features[i];
            egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::new("map-hover"), egui::PopupAnchor::Pointer)
                .show(|ui| feature_title(ui, rs, f.row, self.column));
        }
        if let Some(i) = self.selected.filter(|i| *i < data.features.len()) {
            self.details(ui, rect, rs, data.features[i].row);
        }
        self.cache = Some((key, data));
    }

    fn overlay(&mut self, ui: &mut Ui, rect: Rect, data: &MapData, rs: &ResultSet, geom_cols: &[usize]) {
        let panel = |ui: &mut Ui, pos: Pos2, align: Align2, id: &str, add: &mut dyn FnMut(&mut Ui)| {
            egui::Area::new(egui::Id::new(id))
                .order(egui::Order::Foreground)
                .pivot(align)
                .fixed_pos(pos)
                .show(ui.ctx(), |ui| {
                    Frame::new()
                        .fill(theme::POPUP.gamma_multiply(0.94))
                        .stroke(Stroke::new(1.0, theme::BORDER))
                        .corner_radius(8)
                        .inner_margin(Margin::symmetric(8, 6))
                        .show(ui, |ui| ui.horizontal(|ui| add(ui)));
                });
        };

        let mut fit = false;
        panel(ui, rect.left_top() + vec2(12.0, 12.0), Align2::LEFT_TOP, "map-controls", &mut |ui| {
            for b in Basemap::ALL {
                if ui.selectable_label(self.basemap == b, b.label()).clicked() {
                    self.basemap = b;
                }
            }
            ui.separator();
            if geom_cols.len() > 1 {
                egui::ComboBox::from_id_salt("map-col")
                    .selected_text(&rs.columns[self.column])
                    .show_ui(ui, |ui| {
                        for &c in geom_cols {
                            ui.selectable_value(&mut self.column, c, &rs.columns[c]);
                        }
                    });
            }
            if ui.button("Fit").on_hover_text("Zoom to all features").clicked() {
                fit = true;
            }
            ui.label(RichText::new(crate::app::plural(data.features.len(), "feature")).size(11.5).color(theme::MUTED));
            if data.skipped > 0 {
                let srids = data.srids.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(", ");
                ui.label(RichText::new(format!("{} not shown (SRID {srids})", data.skipped)).size(11.5).color(theme::WARN))
                    .on_hover_text("Only WGS84, Web Mercator and UTM can be drawn.\nUse ST_Transform(geom, 4326) in the query.");
            }
        });
        if fit {
            self.fit(data, rect.size());
        }

        let text = attribution(self.basemap);
        ui.painter().text(
            rect.right_bottom() - vec2(8.0, 6.0),
            Align2::RIGHT_BOTTOM,
            text,
            FontId::proportional(10.0),
            Color32::from_white_alpha(150),
        );
    }

    fn details(&mut self, ui: &mut Ui, rect: Rect, rs: &ResultSet, row: usize) {
        let mut close = false;
        egui::Area::new(egui::Id::new("map-details"))
            .order(egui::Order::Foreground)
            .pivot(Align2::RIGHT_TOP)
            .fixed_pos(rect.right_top() + vec2(-12.0, 12.0))
            .show(ui.ctx(), |ui| {
                Frame::new()
                    .fill(theme::POPUP)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .corner_radius(10)
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(300.0);
                        ui.horizontal(|ui| {
                            feature_title(ui, rs, row, self.column);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new("×").frame(false)).clicked() {
                                    close = true;
                                }
                            });
                        });
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical().max_height(rect.height() - 120.0).show(ui, |ui| {
                            egui::Grid::new("map-attrs").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                                for (i, name) in rs.columns.iter().enumerate() {
                                    ui.label(RichText::new(name).size(12.0).color(theme::MUTED));
                                    match rs.rows[row][i].as_deref() {
                                        None => ui.label(RichText::new("NULL").italics().color(theme::FAINT)),
                                        Some(v) => match geo::parse_hex(v) {
                                            Some(g) => ui.label(RichText::new(g.summary()).size(12.0).color(theme::GEO)),
                                            None => shaping::label(ui, v, 12.5, theme::TEXT),
                                        },
                                    };
                                    ui.end_row();
                                }
                            });
                        });
                    });
            });
        if close {
            self.selected = None;
        }
    }
}

/// "Row 12" or the first text-like column's value.
fn feature_title(ui: &mut Ui, rs: &ResultSet, row: usize, geom_col: usize) {
    let name = ["name", "title", "label", "name_en", "name_km"]
        .iter()
        .find_map(|n| rs.columns.iter().position(|c| c.eq_ignore_ascii_case(n)))
        .filter(|&c| c != geom_col)
        .and_then(|c| rs.rows[row][c].clone());
    match name {
        Some(n) => {
            shaping::label(ui, &n, 13.5, theme::TEXT);
        }
        None => {
            ui.label(RichText::new(format!("Row {}", row + 1)).color(theme::TEXT));
        }
    }
}

struct Features<'a> {
    data: &'a MapData,
    selected: Option<usize>,
    hovered: &'a mut Option<usize>,
    clicked: &'a mut Option<Option<usize>>,
}

impl Plugin for Features<'_> {
    fn run(self: Box<Self>, ui: &mut Ui, response: &egui::Response, projector: &Projector, _: &MapMemory) {
        let painter = ui.painter_at(response.rect);
        let screen = |c: &Coord| -> Pos2 { projector.project(lon_lat(c[0], c[1])).to_pos2() };
        let pointer = response.hover_pos();

        // Hit test, topmost first: points, then lines, then polygons.
        let mut hit = None;
        if let Some(p) = pointer {
            for (i, f) in self.data.features.iter().enumerate().rev() {
                if f.points.iter().any(|c| screen(c).distance(p) < 8.0) {
                    hit = Some(i);
                    break;
                }
            }
            if hit.is_none() {
                hit = self.data.features.iter().enumerate().rev().find_map(|(i, f)| {
                    f.lines.iter().any(|l| near_line(&l.iter().map(screen).collect::<Vec<_>>(), p, 6.0)).then_some(i)
                });
            }
            if hit.is_none() {
                hit = self.data.features.iter().enumerate().rev().find_map(|(i, f)| {
                    f.fills.iter().any(|(v, t)| {
                        let pts: Vec<Pos2> = v.iter().map(screen).collect();
                        t.chunks(3).any(|tri| in_triangle(p, pts[tri[0]], pts[tri[1]], pts[tri[2]]))
                    })
                    .then_some(i)
                });
            }
        }
        *self.hovered = hit;
        if response.clicked() {
            *self.clicked = Some(hit);
        }
        if hit.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let style = |i: usize| -> Color32 {
            if Some(i) == self.selected {
                theme::WARN
            } else if Some(i) == hit {
                Color32::WHITE
            } else {
                theme::GEO
            }
        };
        let mut shapes = Vec::new();
        for pass in 0..3 {
            for (i, f) in self.data.features.iter().enumerate() {
                let color = style(i);
                match pass {
                    0 => {
                        for (verts, tris) in &f.fills {
                            let mut mesh = Mesh::default();
                            for v in verts {
                                mesh.colored_vertex(screen(v), color.gamma_multiply(0.22));
                            }
                            mesh.indices = tris.iter().map(|&t| t as u32).collect();
                            shapes.push(Shape::mesh(mesh));
                        }
                        for ring in &f.rings {
                            shapes.push(Shape::closed_line(ring.iter().map(screen).collect(), Stroke::new(1.5, color)));
                        }
                    }
                    1 => {
                        for l in &f.lines {
                            shapes.push(Shape::line(l.iter().map(screen).collect(), Stroke::new(2.5, color)));
                        }
                    }
                    _ => {
                        for c in &f.points {
                            let p = screen(c);
                            shapes.push(Shape::circle_filled(p, 6.0, Color32::from_black_alpha(160)));
                            shapes.push(Shape::circle_filled(p, 4.5, color));
                        }
                    }
                }
            }
        }
        painter.extend(shapes);
    }
}

fn near_line(points: &[Pos2], p: Pos2, tolerance: f32) -> bool {
    points.windows(2).any(|w| {
        let (a, b) = (w[0], w[1]);
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
        (a + ab * t).distance(p) < tolerance
    })
}

fn in_triangle(p: Pos2, a: Pos2, b: Pos2, c: Pos2) -> bool {
    let sign = |p1: Pos2, p2: Pos2, p3: Pos2| (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y);
    let (d1, d2, d3) = (sign(p, a, b), sign(p, b, c), sign(p, c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}
