//! Map layouts: a page with a map frame, layers from tables, and a title,
//! legend, scale bar and north arrow placed on the page. Rendered to one SVG
//! (in millimetres); the app turns that SVG into the preview, a PNG or a PDF,
//! so all three match.

use crate::geo::{Coord, Geometry};
use std::fmt::Write;

/// Millimetres per typographic point.
pub const PT: f64 = 25.4 / 72.0;
/// Web Mercator sphere radius.
const R: f64 = 6_378_137.0;
/// Tiles fetched for one render at most; past that the tile zoom drops (the
/// basemap gets softer instead of the render getting slow or abusive).
const MAX_TILES: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageSize {
    A5,
    A4,
    A3,
    A2,
    Letter,
    Legal,
    Tabloid,
}

impl PageSize {
    pub const ALL: [PageSize; 7] = [PageSize::A5, PageSize::A4, PageSize::A3, PageSize::A2, PageSize::Letter, PageSize::Legal, PageSize::Tabloid];

    pub fn label(self) -> &'static str {
        match self {
            PageSize::A5 => "A5",
            PageSize::A4 => "A4",
            PageSize::A3 => "A3",
            PageSize::A2 => "A2",
            PageSize::Letter => "Letter",
            PageSize::Legal => "Legal",
            PageSize::Tabloid => "Tabloid",
        }
    }

    /// Portrait width and height in mm.
    pub fn mm(self) -> (f64, f64) {
        match self {
            PageSize::A5 => (148.0, 210.0),
            PageSize::A4 => (210.0, 297.0),
            PageSize::A3 => (297.0, 420.0),
            PageSize::A2 => (420.0, 594.0),
            PageSize::Letter => (215.9, 279.4),
            PageSize::Legal => (215.9, 355.6),
            PageSize::Tabloid => (279.4, 431.8),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Basemap {
    None,
    Light,
    Dark,
    Streets,
    Satellite,
}

impl Basemap {
    pub const ALL: [Basemap; 5] = [Basemap::Light, Basemap::Dark, Basemap::Streets, Basemap::Satellite, Basemap::None];

    pub fn label(self) -> &'static str {
        match self {
            Basemap::None => "None",
            Basemap::Light => "Light",
            Basemap::Dark => "Dark",
            Basemap::Streets => "Streets",
            Basemap::Satellite => "Satellite",
        }
    }

    pub fn tile_url(self, z: u8, x: u32, y: u32) -> Option<String> {
        Some(match self {
            Basemap::None => return None,
            Basemap::Light => format!("https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Light_Gray_Base/MapServer/tile/{z}/{y}/{x}"),
            Basemap::Dark => format!("https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Base/MapServer/tile/{z}/{y}/{x}"),
            Basemap::Streets => format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png"),
            Basemap::Satellite => format!("https://mt1.google.com/vt/lyrs=s&x={x}&y={y}&z={z}"),
        })
    }

    pub fn max_zoom(self) -> u8 {
        match self {
            Basemap::Light | Basemap::Dark => 16,
            Basemap::Streets => 19,
            Basemap::Satellite => 20,
            Basemap::None => 0,
        }
    }

    pub fn attribution(self) -> &'static str {
        match self {
            Basemap::None => "",
            Basemap::Light | Basemap::Dark => "Esri, HERE, Garmin, © OpenStreetMap contributors",
            Basemap::Streets => "© OpenStreetMap contributors",
            Basemap::Satellite => "Imagery © Google",
        }
    }
}

/// What a layer draws, for its legend swatch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Point,
    Line,
    Polygon,
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub id: u64,
    pub schema: String,
    pub table: String,
    pub column: String,
    /// SQL WHERE text; empty for all rows.
    pub filter: String,
    /// Legend label.
    pub label: String,
    pub color: [u8; 3],
    /// Line width and polygon outline, mm.
    pub width: f64,
    /// Point radius, mm.
    pub point: f64,
    pub fill_opacity: f64,
    pub visible: bool,
    pub style: Style,
    /// Loaded geometries, WGS84 (shared: layouts are cloned for each render).
    pub shapes: std::sync::Arc<Vec<Geometry>>,
    /// The category column's value for each shape (empty without categories).
    pub values: std::sync::Arc<Vec<Option<String>>>,
    /// The whole layer's extent in lon/lat (for Fit).
    pub extent: Option<(Coord, Coord)>,
    /// The area (lon/lat bbox) and scale the shapes were loaded for.
    pub loaded: Option<([f64; 4], f64)>,
}

/// How a layer is colored.
#[derive(Clone, Debug, PartialEq)]
pub enum Style {
    /// One color for every feature.
    Single,
    /// A color per value of `column`; `more` when there were more values than classes.
    Categories { column: String, classes: Vec<Class>, more: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    pub value: Option<String>,
    /// Legend label (starts as the value).
    pub label: String,
    pub color: [u8; 3],
    pub visible: bool,
    pub count: i64,
}

/// One row of the legend: a swatch (layer kind and color) or a heading.
pub struct LegendRow {
    pub label: String,
    pub swatch: Option<(Kind, [u8; 3])>,
    pub indent: bool,
}

impl Layer {
    pub fn new(id: u64, schema: &str, table: &str, column: &str, color: [u8; 3]) -> Self {
        Self {
            id,
            schema: schema.into(),
            table: table.into(),
            column: column.into(),
            filter: String::new(),
            label: table.into(),
            color,
            width: 0.35,
            point: 1.2,
            fill_opacity: 0.35,
            visible: true,
            style: Style::Single,
            shapes: Default::default(),
            values: Default::default(),
            extent: None,
            loaded: None,
        }
    }

    fn table_ref(&self) -> String {
        format!("{}.{}", crate::sql::quote_ident(&self.schema), crate::sql::quote_ident(&self.table))
    }

    fn geom(&self) -> String {
        format!("{}::geometry", crate::sql::quote_ident(&self.column))
    }

    fn filter_and(&self) -> String {
        let f = self.filter.trim();
        if f.is_empty() { String::new() } else { format!(" and ({f})") }
    }

    pub fn category_column(&self) -> Option<&str> {
        match &self.style {
            Style::Categories { column, .. } => Some(column),
            Style::Single => None,
        }
    }

    /// The features inside `bbox` (lon/lat), in WGS84 and simplified to
    /// about 0.15 mm at `scale`, plus the category column's value. PostGIS
    /// does the reprojection, so any SRID works; the bbox uses the index.
    pub fn data_sql(&self, bbox: [f64; 4], scale: f64, limit: usize) -> String {
        let (t, g) = (self.table_ref(), self.geom());
        let tol = scale / 1000.0 * 0.15 / 111_320.0;
        let cat = self.category_column().map_or(String::new(), |c| format!(", ({})::text", crate::sql::quote_ident(c)));
        let [x0, y0, x1, y1] = [bbox[0].max(-180.0), bbox[1].max(-85.0), bbox[2].min(180.0), bbox[3].min(85.0)];
        format!(
            "with s as (select ST_SRID({g}) as srid from {t} where {g} is not null limit 1) \
             select ST_SimplifyPreserveTopology(case when ST_SRID({g}) = 0 then ST_SetSRID({g}, 4326) else ST_Transform({g}, 4326) end, {tol:e}){cat} \
             from {t} where {g} is not null \
             and ((select srid from s) = 0 or {g} && ST_Transform(ST_MakeEnvelope({x0}, {y0}, {x1}, {y1}, 4326), (select srid from s))){} limit {limit}",
            self.filter_and()
        )
    }

    /// The whole layer's extent, as a WGS84 geometry.
    pub fn extent_sql(&self) -> String {
        let (t, g) = (self.table_ref(), self.geom());
        let f = self.filter.trim();
        let filter = if f.is_empty() { String::new() } else { format!(" where {f}") };
        format!(
            "with s as (select coalesce(nullif(ST_SRID({g}), 0), 4326) as srid from {t} where {g} is not null limit 1) \
             select ST_Transform(ST_SetSRID(ST_Extent({g})::geometry, (select srid from s)), 4326) from {t}{filter}"
        )
    }

    /// The most common values of `column`, with counts.
    pub fn classes_sql(&self, column: &str, limit: usize) -> String {
        let f = self.filter.trim();
        let filter = if f.is_empty() { String::new() } else { format!(" where {f}") };
        format!(
            "select ({})::text, count(*) from {}{filter} group by 1 order by 2 desc, 1 limit {limit}",
            crate::sql::quote_ident(column),
            self.table_ref()
        )
    }

    /// Whether the loaded shapes no longer cover `view` (lon/lat) or were
    /// simplified for a very different scale.
    pub fn needs_reload(&self, view: [f64; 4], scale: f64) -> bool {
        match self.loaded {
            None => true,
            Some((b, s)) => view[0] < b[0] || view[1] < b[1] || view[2] > b[2] || view[3] > b[3] || scale < s / 2.0 || scale > s * 2.0,
        }
    }

    /// Legend rows for this layer.
    fn legend_rows(&self, out: &mut Vec<LegendRow>) {
        match &self.style {
            Style::Single => out.push(LegendRow { label: self.label.clone(), swatch: Some((self.kind(), self.color)), indent: false }),
            Style::Categories { classes, more, .. } => {
                out.push(LegendRow { label: self.label.clone(), swatch: None, indent: false });
                for c in classes.iter().filter(|c| c.visible) {
                    out.push(LegendRow { label: c.label.clone(), swatch: Some((self.kind(), c.color)), indent: true });
                }
                if *more {
                    out.push(LegendRow { label: "Other".into(), swatch: Some((self.kind(), self.color)), indent: true });
                }
            }
        }
    }

    pub fn kind(&self) -> Kind {
        fn of(g: &Geometry) -> Kind {
            match g {
                Geometry::Point(_) | Geometry::MultiPoint(_) => Kind::Point,
                Geometry::LineString(_) | Geometry::MultiLineString(_) => Kind::Line,
                Geometry::Polygon(_) | Geometry::MultiPolygon(_) => Kind::Polygon,
                Geometry::Collection(c) => c.first().map_or(Kind::Point, of),
            }
        }
        self.shapes.first().map_or(Kind::Polygon, of)
    }
}

/// A rectangle on the page, mm, from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, p: [f64; 2]) -> bool {
        p[0] >= self.x && p[0] <= self.x + self.w && p[1] >= self.y && p[1] <= self.y + self.h
    }
}

/// An element placed on the page: shown or not, and its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Item {
    pub visible: bool,
    pub pos: [f64; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Element {
    Title,
    Legend,
    ScaleBar,
    North,
}

impl Element {
    pub const ALL: [Element; 4] = [Element::Title, Element::Legend, Element::ScaleBar, Element::North];
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub page: PageSize,
    pub landscape: bool,
    /// Page margins around the map frame, mm: top, right, bottom, left.
    pub margin: [f64; 4],
    /// Map center, lon/lat.
    pub center: Coord,
    /// Map scale 1:N, true at the center's latitude.
    pub scale: f64,
    pub basemap: Basemap,
    pub layers: Vec<Layer>,
    pub title: String,
    /// Title size, points.
    pub title_size: f64,
    pub legend_title: String,
    pub items: [Item; 4],
}

impl Default for Layout {
    fn default() -> Self {
        let mut l = Self {
            page: PageSize::A4,
            landscape: true,
            margin: [10.0; 4],
            center: [104.92, 11.56],
            scale: 50_000.0,
            basemap: Basemap::Light,
            layers: Vec::new(),
            title: String::new(),
            title_size: 20.0,
            legend_title: "Legend".into(),
            items: [Item { visible: true, pos: [0.0, 0.0] }; 4],
        };
        l.place_defaults();
        l
    }
}

impl Layout {
    pub fn item(&self, e: Element) -> &Item {
        &self.items[e as usize]
    }

    pub fn item_mut(&mut self, e: Element) -> &mut Item {
        &mut self.items[e as usize]
    }

    pub fn page_mm(&self) -> (f64, f64) {
        let (w, h) = self.page.mm();
        if self.landscape { (h, w) } else { (w, h) }
    }

    pub fn frame(&self) -> Rect {
        let (w, h) = self.page_mm();
        let [t, r, b, l] = self.margin;
        Rect { x: l, y: t, w: (w - l - r).max(10.0), h: (h - t - b).max(10.0) }
    }

    /// The map frame's extent in lon/lat (min lon, min lat, max lon, max
    /// lat), grown by `pad` × its size on every side.
    pub fn view_bbox(&self, pad: f64) -> [f64; 4] {
        let f = self.frame();
        let (dx, dy) = (f.w * pad, f.h * pad);
        let nw = self.from_page([f.x - dx, f.y - dy]);
        let se = self.from_page([f.x + f.w + dx, f.y + f.h + dy]);
        [nw[0], se[1], se[0], nw[1]]
    }

    pub fn legend_rows(&self) -> Vec<LegendRow> {
        let mut rows = Vec::new();
        for l in self.layers.iter().filter(|l| l.visible) {
            l.legend_rows(&mut rows);
        }
        rows
    }

    /// The usual spots: title top center, north arrow top right, legend
    /// bottom left, scale bar bottom right (all inside the frame).
    pub fn place_defaults(&mut self) {
        for e in Element::ALL {
            self.place(e);
        }
    }

    /// Put one element at its usual spot (sized for its current content, so
    /// a growing legend grows upward from the bottom).
    pub fn place(&mut self, e: Element) {
        let f = self.frame();
        let pad = 5.0;
        let b = self.boxes()[e as usize];
        self.item_mut(e).pos = match e {
            Element::Title => [f.x + (f.w - b.w) / 2.0, f.y + pad],
            Element::North => [f.x + f.w - b.w - pad, f.y + pad],
            Element::Legend => [f.x + pad, f.y + f.h - b.h - pad],
            Element::ScaleBar => [f.x + f.w - b.w - pad, f.y + f.h - b.h - pad - 4.0],
        };
    }

    /// Keep every element on the page (after a page size change).
    pub fn clamp_items(&mut self) {
        let (w, h) = self.page_mm();
        let boxes = self.boxes();
        for e in Element::ALL {
            let b = boxes[e as usize];
            let pos = &mut self.items[e as usize].pos;
            pos[0] = pos[0].clamp(0.0, (w - b.w).max(0.0));
            pos[1] = pos[1].clamp(0.0, (h - b.h).max(0.0));
        }
    }

    /// Mercator meters per page millimetre.
    pub fn m_per_mm(&self) -> f64 {
        self.scale / 1000.0 / self.center[1].to_radians().cos().max(0.01)
    }

    fn center_merc(&self) -> [f64; 2] {
        merc(self.center)
    }

    /// Lon/lat to page mm.
    pub fn to_page(&self, c: Coord) -> [f64; 2] {
        let f = self.frame();
        let (m, k) = (merc(c), self.m_per_mm());
        let cm = self.center_merc();
        [f.x + f.w / 2.0 + (m[0] - cm[0]) / k, f.y + f.h / 2.0 - (m[1] - cm[1]) / k]
    }

    /// Page mm to lon/lat.
    pub fn from_page(&self, p: [f64; 2]) -> Coord {
        let f = self.frame();
        let (k, cm) = (self.m_per_mm(), self.center_merc());
        unmerc([cm[0] + (p[0] - f.x - f.w / 2.0) * k, cm[1] - (p[1] - f.y - f.h / 2.0) * k])
    }

    /// Move the map by `d` page mm (a drag).
    pub fn pan(&mut self, d: [f64; 2]) {
        let f = self.frame();
        let mid = [f.x + f.w / 2.0, f.y + f.h / 2.0];
        self.center = self.from_page([mid[0] - d[0], mid[1] - d[1]]);
    }

    /// Zoom by `factor` (> 1 zooms in) keeping the point at `at` (page mm) still.
    pub fn zoom_at(&mut self, factor: f64, at: [f64; 2]) {
        let before = self.from_page(at);
        self.scale = (self.scale / factor).clamp(100.0, 50_000_000.0);
        // Panning moves the center's latitude, which nudges the projection
        // scale: a few passes settle the point under the cursor.
        for _ in 0..4 {
            let after = self.to_page(before);
            self.pan([at[0] - after[0], at[1] - after[1]]);
        }
    }

    /// Center and scale to the visible layers, with some room around them.
    pub fn fit(&mut self) {
        let mut lo = [f64::MAX; 2];
        let mut hi = [f64::MIN; 2];
        let mut add = |c: Coord| {
            if c[0].is_finite() && c[1].is_finite() {
                let m = merc(c);
                lo = [lo[0].min(m[0]), lo[1].min(m[1])];
                hi = [hi[0].max(m[0]), hi[1].max(m[1])];
            }
        };
        for l in self.layers.iter().filter(|l| l.visible) {
            match l.extent {
                Some((a, b)) => {
                    add(a);
                    add(b);
                }
                None => l.shapes.iter().for_each(|g| g.for_each_coord(&mut add)),
            }
        }
        if lo[0] > hi[0] {
            return;
        }
        let mid = unmerc([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]);
        let f = self.frame();
        let span = ((hi[0] - lo[0]) / f.w).max((hi[1] - lo[1]) / f.h).max(1e-9) * 1.12;
        self.center = mid;
        // m_per_mm = scale / 1000 / cos(lat): solve for the scale.
        self.scale = round_scale(span * 1000.0 * mid[1].to_radians().cos()).clamp(100.0, 50_000_000.0);
    }

    /// Element boxes on the page, in [`Element`] order. Text widths are
    /// estimated (the same estimate the renderer sizes its boxes with).
    pub fn boxes(&self) -> [Rect; 4] {
        let at = |e: Element| self.item(e).pos;
        let ts = self.title_size * PT;
        let title = if self.title.trim().is_empty() { "Map title" } else { self.title.trim() };
        let title = Rect { x: at(Element::Title)[0], y: at(Element::Title)[1], w: text_w(title, ts) + 4.0, h: ts * 1.5 };
        let legend_rows = self.legend_rows();
        let rows = legend_rows.len().max(1);
        let label_w = legend_rows.iter().map(|r| text_w(&r.label, LEGEND_TEXT) + if r.indent { 3.0 } else { 0.0 }).fold(0.0, f64::max);
        let head_w = text_w(&self.legend_title, LEGEND_TITLE);
        let legend = Rect {
            x: at(Element::Legend)[0],
            y: at(Element::Legend)[1],
            w: (label_w + 13.0).max(head_w + 8.0).max(28.0),
            h: 9.0 + rows as f64 * 6.0 + 2.0,
        };
        let (bar, label) = self.scale_bar();
        let scale = Rect {
            x: at(Element::ScaleBar)[0],
            y: at(Element::ScaleBar)[1],
            w: bar.max(text_w(&label, 2.8)) + 8.0,
            h: 13.0,
        };
        let north = Rect { x: at(Element::North)[0], y: at(Element::North)[1], w: 11.0, h: 15.0 };
        [title, legend, scale, north]
    }

    /// Scale bar length in mm and its label (a round ground distance).
    pub fn scale_bar(&self) -> (f64, String) {
        let ground_per_mm = self.scale / 1000.0;
        let meters = nice_length(ground_per_mm * 40.0);
        (meters / ground_per_mm, distance_label(meters))
    }

    /// Basemap tile zoom for a render at `dpi`, and the tiles it needs.
    pub fn tiles(&self, dpi: f64) -> (u8, Vec<(u32, u32)>) {
        if self.basemap == Basemap::None {
            return (0, Vec::new());
        }
        let f = self.frame();
        let k = self.m_per_mm();
        let cm = self.center_merc();
        let (left, right) = (cm[0] - f.w / 2.0 * k, cm[0] + f.w / 2.0 * k);
        let (top, bottom) = (cm[1] + f.h / 2.0 * k, cm[1] - f.h / 2.0 * k);
        let px_per_mm = dpi / 25.4;
        let world = 2.0 * std::f64::consts::PI * R;
        let mut z = ((world / (256.0 * k / px_per_mm)).log2().round() as i32).clamp(0, self.basemap.max_zoom() as i32) as u8;
        loop {
            let n = 1u32 << z;
            let tile = world / n as f64;
            let col = |m: f64| (((m + world / 2.0) / tile).floor().max(0.0) as u32).min(n - 1);
            let row = |m: f64| (((world / 2.0 - m) / tile).floor().max(0.0) as u32).min(n - 1);
            let (x0, x1, y0, y1) = (col(left), col(right), row(top), row(bottom));
            let count = (x1 - x0 + 1) as usize * (y1 - y0 + 1) as usize;
            if count <= MAX_TILES || z == 0 {
                let tiles = (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))).collect();
                return (z, tiles);
            }
            z -= 1;
        }
    }

    /// The page as SVG (mm). `tile` returns a basemap tile's image bytes
    /// (PNG or JPEG), `None` to leave it out; `dpi` picks the tile zoom.
    pub fn svg(&self, tile: &mut dyn FnMut(u8, u32, u32) -> Option<Vec<u8>>, dpi: f64) -> String {
        let (pw, ph) = self.page_mm();
        let f = self.frame();
        let mut s = String::new();
        let _ = write!(
            s,
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{pw}mm" height="{ph}mm" viewBox="0 0 {pw} {ph}" font-family="Helvetica Neue, Helvetica, Arial, sans-serif">"#
        );
        let _ = write!(s, r##"<rect width="{pw}" height="{ph}" fill="#fff"/>"##);
        let _ = write!(s, r#"<defs><clipPath id="frame"><rect x="{}" y="{}" width="{}" height="{}"/></clipPath></defs>"#, n(f.x), n(f.y), n(f.w), n(f.h));
        s.push_str(r#"<g clip-path="url(#frame)">"#);
        // Basemap.
        let (z, tiles) = self.tiles(dpi);
        let world = 2.0 * std::f64::consts::PI * R;
        for (x, y) in tiles {
            let Some(bytes) = tile(z, x, y) else { continue };
            let size = world / (1u32 << z) as f64;
            let nw = unmerc([-world / 2.0 + x as f64 * size, world / 2.0 - y as f64 * size]);
            let se = unmerc([-world / 2.0 + (x + 1) as f64 * size, world / 2.0 - (y + 1) as f64 * size]);
            let (a, b) = (self.to_page(nw), self.to_page(se));
            let mime = if bytes.starts_with(&[0xff, 0xd8]) { "image/jpeg" } else { "image/png" };
            // A hair of overlap hides seams between tiles.
            let _ = write!(
                s,
                r#"<image x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="none" xlink:href="data:{mime};base64,{}"/>"#,
                n(a[0]),
                n(a[1]),
                n(b[0] - a[0] + 0.05),
                n(b[1] - a[1] + 0.05),
                base64(&bytes)
            );
        }
        if self.basemap == Basemap::None {
            let _ = write!(s, r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#f4f5f2"/>"##, n(f.x), n(f.y), n(f.w), n(f.h));
        }
        // Layers: the first in the list draws on top.
        for l in self.layers.iter().rev().filter(|l| l.visible) {
            self.layer_svg(&mut s, l);
        }
        if self.basemap != Basemap::None {
            let text = self.basemap.attribution();
            let _ = write!(
                s,
                r##"<text x="{}" y="{}" font-size="{}" text-anchor="end" fill="#555" stroke="#fff" stroke-width="0.6" paint-order="stroke">{}</text>"##,
                n(f.x + f.w - 1.5),
                n(f.y + f.h - 1.5),
                n(5.5 * PT),
                esc(text)
            );
        }
        s.push_str("</g>");
        let _ = write!(s, r##"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="#222" stroke-width="0.3"/>"##, n(f.x), n(f.y), n(f.w), n(f.h));
        self.elements_svg(&mut s);
        s.push_str("</svg>");
        s
    }

    fn layer_svg(&self, s: &mut String, l: &Layer) {
        match &l.style {
            Style::Single => self.group_svg(s, l, l.shapes.iter(), l.color),
            Style::Categories { classes, .. } => {
                let index: std::collections::HashMap<Option<&str>, usize> = classes.iter().enumerate().map(|(i, c)| (c.value.as_deref(), i)).collect();
                let mut groups: Vec<Vec<&Geometry>> = vec![Vec::new(); classes.len() + 1];
                for (i, g) in l.shapes.iter().enumerate() {
                    let v = l.values.get(i).and_then(|v| v.as_deref());
                    groups[index.get(&v).copied().unwrap_or(classes.len())].push(g);
                }
                // Values without a class ("Other") go underneath, in the layer color.
                self.group_svg(s, l, groups[classes.len()].iter().copied(), l.color);
                for (c, g) in classes.iter().zip(&groups) {
                    if c.visible {
                        self.group_svg(s, l, g.iter().copied(), c.color);
                    }
                }
            }
        }
    }

    fn group_svg<'a>(&self, s: &mut String, l: &Layer, shapes: impl Iterator<Item = &'a Geometry>, color: [u8; 3]) {
        let f = self.frame();
        let color = format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]);
        // Skip what's well outside the frame, and points closer than 0.05 mm.
        let (lo, hi) = ([f.x - 20.0, f.y - 20.0], [f.x + f.w + 20.0, f.y + f.h + 20.0]);
        let mut fills = String::new();
        let mut lines = String::new();
        let mut dots = String::new();
        let path = |out: &mut String, ring: &[Coord], close: bool| {
            let pts: Vec<[f64; 2]> = ring.iter().map(|c| self.to_page(*c)).collect();
            if pts.iter().all(|p| p[0] < lo[0]) || pts.iter().all(|p| p[0] > hi[0]) || pts.iter().all(|p| p[1] < lo[1]) || pts.iter().all(|p| p[1] > hi[1]) {
                return;
            }
            let mut last: Option<[f64; 2]> = None;
            for p in pts {
                if last.is_some_and(|q| (q[0] - p[0]).abs() < 0.05 && (q[1] - p[1]).abs() < 0.05) {
                    continue;
                }
                let _ = write!(out, "{}{} {}", if last.is_none() { "M" } else { "L" }, n(p[0]), n(p[1]));
                last = Some(p);
            }
            if close {
                out.push('Z');
            }
        };
        let add = |g: &Geometry, fills: &mut String, lines: &mut String, dots: &mut String| {
            let mut stack = vec![g];
            while let Some(g) = stack.pop() {
                match g {
                    Geometry::Point(c) => dot(dots, self.to_page(*c), l.point, lo, hi),
                    Geometry::MultiPoint(ps) => ps.iter().for_each(|c| dot(dots, self.to_page(*c), l.point, lo, hi)),
                    Geometry::LineString(line) => path(lines, line, false),
                    Geometry::MultiLineString(ls) => ls.iter().for_each(|line| path(lines, line, false)),
                    // One path per polygon: even-odd makes holes of its own
                    // rings, but would also cut where separate polygons overlap.
                    Geometry::Polygon(rings) => polygon(fills, rings, &path),
                    Geometry::MultiPolygon(ps) => ps.iter().for_each(|rings| polygon(fills, rings, &path)),
                    Geometry::Collection(c) => stack.extend(c.iter()),
                }
            }
        };
        for g in shapes {
            add(g, &mut fills, &mut lines, &mut dots);
        }
        let w = n(l.width);
        if !fills.is_empty() {
            let _ = write!(
                s,
                r#"<g fill="{color}" fill-opacity="{}" fill-rule="evenodd" stroke="{color}" stroke-width="{w}" stroke-linejoin="round">{fills}</g>"#,
                n(l.fill_opacity)
            );
        }
        if !lines.is_empty() {
            let _ = write!(s, r#"<path d="{lines}" fill="none" stroke="{color}" stroke-width="{w}" stroke-linejoin="round" stroke-linecap="round"/>"#);
        }
        if !dots.is_empty() {
            let _ = write!(s, r##"<g fill="{color}" stroke="#fff" stroke-width="{}">{dots}</g>"##, n(l.point * 0.3));
        }
    }

    fn elements_svg(&self, s: &mut String) {
        let boxes = self.boxes();
        let card = r##"fill="#fff" fill-opacity="0.92" stroke="#222" stroke-opacity="0.25" stroke-width="0.25""##;
        if self.item(Element::Title).visible && !self.title.trim().is_empty() {
            let b = boxes[Element::Title as usize];
            let ts = self.title_size * PT;
            let _ = write!(
                s,
                r##"<text x="{}" y="{}" font-size="{}" font-weight="600" text-anchor="middle" fill="#111" stroke="#fff" stroke-width="{}" stroke-linejoin="round" paint-order="stroke">{}</text>"##,
                n(b.x + b.w / 2.0),
                n(b.y + ts * 1.1),
                n(ts),
                n(ts * 0.18),
                esc(self.title.trim())
            );
        }
        if self.item(Element::Legend).visible && !self.layers.is_empty() {
            let b = boxes[Element::Legend as usize];
            let _ = write!(s, r#"<rect x="{}" y="{}" width="{}" height="{}" rx="1" {card}/>"#, n(b.x), n(b.y), n(b.w), n(b.h));
            let _ = write!(s, r##"<text x="{}" y="{}" font-size="{}" font-weight="600" fill="#111">{}</text>"##, n(b.x + 4.0), n(b.y + 6.0), n(LEGEND_TITLE), esc(&self.legend_title));
            // Style per row comes from its layer (line width, fill, point size).
            let mut layer_of_row = Vec::new();
            for l in self.layers.iter().filter(|l| l.visible) {
                let mut rows = Vec::new();
                l.legend_rows(&mut rows);
                layer_of_row.extend(std::iter::repeat_n(l, rows.len()));
            }
            for (i, (row, l)) in self.legend_rows().iter().zip(layer_of_row).enumerate() {
                let y = b.y + 9.0 + i as f64 * 6.0;
                let (sx, sy) = (b.x + 4.0 + if row.indent { 3.0 } else { 0.0 }, y + 1.0);
                let Some((kind, c)) = row.swatch else {
                    let _ = write!(s, r##"<text x="{}" y="{}" font-size="{}" font-weight="600" fill="#333">{}</text>"##, n(sx), n(sy + 3.0), n(LEGEND_TEXT), esc(&row.label));
                    continue;
                };
                let color = format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
                match kind {
                    Kind::Polygon => {
                        let _ = write!(s, r#"<rect x="{}" y="{}" width="5" height="3.6" fill="{color}" fill-opacity="{}" stroke="{color}" stroke-width="{}"/>"#, n(sx), n(sy), n(l.fill_opacity), n(l.width));
                    }
                    Kind::Line => {
                        let _ = write!(s, r#"<path d="M{} {}L{} {}" stroke="{color}" stroke-width="{}" stroke-linecap="round"/>"#, n(sx), n(sy + 1.8), n(sx + 5.0), n(sy + 1.8), n(l.width.max(0.4)));
                    }
                    Kind::Point => {
                        let _ = write!(s, r##"<circle cx="{}" cy="{}" r="{}" fill="{color}" stroke="#fff" stroke-width="0.3"/>"##, n(sx + 2.5), n(sy + 1.8), n(l.point.min(2.0)));
                    }
                }
                let _ = write!(s, r##"<text x="{}" y="{}" font-size="{}" fill="#222">{}</text>"##, n(sx + 8.0), n(sy + 3.0), n(LEGEND_TEXT), esc(&row.label));
            }
        }
        if self.item(Element::ScaleBar).visible {
            let b = boxes[Element::ScaleBar as usize];
            let (len, label) = self.scale_bar();
            let _ = write!(s, r#"<rect x="{}" y="{}" width="{}" height="{}" rx="1" {card}/>"#, n(b.x), n(b.y), n(b.w), n(b.h));
            let (x0, y) = (b.x + 4.0, b.y + 6.0);
            for i in 0..4 {
                let fill = if i % 2 == 0 { "#222" } else { "#fff" };
                let _ = write!(s, r#"<rect x="{}" y="{}" width="{}" height="1.6" fill="{fill}"/>"#, n(x0 + len / 4.0 * i as f64), n(y), n(len / 4.0));
            }
            let _ = write!(s, r##"<rect x="{}" y="{}" width="{}" height="1.6" fill="none" stroke="#222" stroke-width="0.2"/>"##, n(x0), n(y), n(len));
            let _ = write!(s, r##"<text x="{}" y="{}" font-size="2.6" fill="#444">0</text>"##, n(x0), n(y - 1.2));
            let _ = write!(s, r##"<text x="{}" y="{}" font-size="2.6" fill="#222" text-anchor="end">{}</text>"##, n(x0 + len), n(y - 1.2), esc(&label));
            let _ = write!(s, r##"<text x="{}" y="{}" font-size="2.4" fill="#555">1:{}</text>"##, n(x0), n(y + 5.0), thousands(self.scale));
        }
        if self.item(Element::North).visible {
            let b = boxes[Element::North as usize];
            let (cx, top) = (b.x + b.w / 2.0, b.y + 1.0);
            let _ = write!(
                s,
                r##"<path d="M{cx} {top}L{} {}L{cx} {}Z" fill="#222"/><path d="M{cx} {top}L{} {}L{cx} {}Z" fill="#888"/><text x="{cx}" y="{}" font-size="3.4" font-weight="600" text-anchor="middle" fill="#222" stroke="#fff" stroke-width="0.5" paint-order="stroke">N</text>"##,
                n(cx - 3.5),
                n(top + 10.0),
                n(top + 7.5),
                n(cx + 3.5),
                n(top + 10.0),
                n(top + 7.5),
                n(top + 13.8)
            );
        }
    }
}

const LEGEND_TITLE: f64 = 3.4;
const LEGEND_TEXT: f64 = 3.0;

/// One polygon as its own `<path>` (outer ring and holes together).
fn polygon(out: &mut String, rings: &[Vec<Coord>], path: &dyn Fn(&mut String, &[Coord], bool)) {
    let mut d = String::new();
    for r in rings {
        path(&mut d, r, true);
    }
    if !d.is_empty() {
        let _ = write!(out, r#"<path d="{d}"/>"#);
    }
}

fn dot(out: &mut String, p: [f64; 2], r: f64, lo: [f64; 2], hi: [f64; 2]) {
    if p[0] >= lo[0] && p[0] <= hi[0] && p[1] >= lo[1] && p[1] <= hi[1] {
        let _ = write!(out, r#"<circle cx="{}" cy="{}" r="{}"/>"#, n(p[0]), n(p[1]), n(r));
    }
}

/// Estimated width of `text` at `size` mm in a sans-serif face.
pub fn text_w(text: &str, size: f64) -> f64 {
    text.chars().map(|c| if c.is_ascii_uppercase() || c == 'm' || c == 'w' { 0.68 } else if " il.,:;|'".contains(c) { 0.3 } else { 0.54 }).sum::<f64>() * size
}

pub fn merc(c: Coord) -> [f64; 2] {
    let lat = c[1].clamp(-85.05, 85.05).to_radians();
    [c[0].to_radians() * R, (std::f64::consts::FRAC_PI_4 + lat / 2.0).tan().ln() * R]
}

pub fn unmerc(m: [f64; 2]) -> Coord {
    [(m[0] / R).to_degrees(), (2.0 * (m[1] / R).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees()]
}

/// The largest 1, 2 or 5 × 10ⁿ at most `max`.
pub fn nice_length(max: f64) -> f64 {
    if !(max.is_finite() && max > 0.0) {
        return 1.0;
    }
    let unit = 10f64.powf(max.log10().floor());
    [5.0, 2.0, 1.0].into_iter().map(|m| m * unit).find(|v| *v <= max).unwrap_or(unit)
}

/// A map scale rounded to two significant figures (1:23,456 → 1:23,000).
pub fn round_scale(s: f64) -> f64 {
    if s <= 0.0 || !s.is_finite() {
        return 1.0;
    }
    let unit = 10f64.powf(s.log10().floor() - 1.0);
    (s / unit).round() * unit
}

pub fn distance_label(meters: f64) -> String {
    let trim = |v: f64| {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    if meters >= 1000.0 { format!("{} km", trim(meters / 1000.0)) } else { format!("{} m", trim(meters)) }
}

pub fn thousands(v: f64) -> String {
    let digits = format!("{:.0}", v.round());
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A number for SVG: two decimals, trailing zeros dropped.
fn n(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let v = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(v >> 18) as usize & 63] as char);
        out.push(T[(v >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(v >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[v as usize & 63] as char } else { '=' });
    }
    out
}

/// Fetch basemap tiles (blocking; call off the UI thread). Tiles are cached
/// on disk, so re-renders and re-exports don't fetch them again; a tile that
/// fails is simply left out.
/// With `network` off, only what's on disk (instant, for a first preview).
pub fn fetch_tiles(basemap: Basemap, z: u8, tiles: &[(u32, u32)], network: bool) -> std::collections::HashMap<(u32, u32), Vec<u8>> {
    let dir = std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join("Library/Caches/rgrip/layout-tiles").join(basemap.label()).join(z.to_string()));
    let mut out = std::collections::HashMap::new();
    let mut missing = Vec::new();
    for &(x, y) in tiles {
        match dir.as_ref().and_then(|d| std::fs::read(d.join(format!("{x}_{y}"))).ok()) {
            Some(bytes) => {
                out.insert((x, y), bytes);
            }
            None => missing.push((x, y)),
        }
    }
    if missing.is_empty() || !network {
        return out;
    }
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return out };
    let Ok(client) = reqwest::Client::builder()
        .user_agent(concat!("rgrip/", env!("CARGO_PKG_VERSION"), " (desktop PostgreSQL client; map layout)"))
        .timeout(std::time::Duration::from_secs(20))
        .build()
    else {
        return out;
    };
    let fetched = rt.block_on(async {
        use futures_util::StreamExt;
        futures_util::stream::iter(missing.into_iter().filter_map(|(x, y)| basemap.tile_url(z, x, y).map(|u| (x, y, u))))
            .map(|(x, y, url)| {
                let client = client.clone();
                async move {
                    let resp = client.get(url).send().await.ok()?;
                    if !resp.status().is_success() {
                        return None;
                    }
                    Some(((x, y), resp.bytes().await.ok()?.to_vec()))
                }
            })
            // A few at a time: polite to the tile servers.
            .buffer_unordered(6)
            .collect::<Vec<_>>()
            .await
    });
    for ((x, y), bytes) in fetched.into_iter().flatten() {
        if let Some(d) = &dir
            && std::fs::create_dir_all(d).is_ok()
        {
            let _ = std::fs::write(d.join(format!("{x}_{y}")), &bytes);
        }
        out.insert((x, y), bytes);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_with(shapes: Vec<Geometry>) -> Layout {
        let mut l = Layout::default();
        let mut layer = Layer::new(1, "demo", "parcels", "geom", [200, 60, 40]);
        layer.shapes = shapes.into();
        l.layers.push(layer);
        l.basemap = Basemap::None;
        l
    }

    #[test]
    fn page_and_frame() {
        let mut l = Layout::default();
        assert_eq!(l.page_mm(), (297.0, 210.0));
        l.landscape = false;
        assert_eq!(l.frame(), Rect { x: 10.0, y: 10.0, w: 190.0, h: 277.0 });
        // Each side on its own: top, right, bottom, left.
        l.margin = [20.0, 5.0, 30.0, 15.0];
        assert_eq!(l.frame(), Rect { x: 15.0, y: 20.0, w: 190.0, h: 247.0 });
    }

    #[test]
    fn page_and_map_coordinates_round_trip() {
        let l = Layout::default();
        let f = l.frame();
        let mid = l.to_page(l.center);
        assert!((mid[0] - (f.x + f.w / 2.0)).abs() < 1e-6 && (mid[1] - (f.y + f.h / 2.0)).abs() < 1e-6);
        let c = l.from_page([30.0, 40.0]);
        let p = l.to_page(c);
        assert!((p[0] - 30.0).abs() < 1e-6 && (p[1] - 40.0).abs() < 1e-6);
        // 1:50,000: 1 mm on the page is 50 m on the ground.
        let east = l.from_page([mid[0] + 1.0, mid[1]]);
        let ground = crate::layout::tests::haversine(l.center, east);
        assert!((ground - 50.0).abs() < 0.5, "{ground}");
    }

    fn haversine(a: Coord, b: Coord) -> f64 {
        let (la1, la2) = (a[1].to_radians(), b[1].to_radians());
        let h = ((la2 - la1) / 2.0).sin().powi(2) + la1.cos() * la2.cos() * ((b[0] - a[0]).to_radians() / 2.0).sin().powi(2);
        2.0 * 6_371_008.8 * h.sqrt().asin()
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let mut l = Layout::default();
        let at = [60.0, 50.0];
        let before = l.from_page(at);
        l.zoom_at(2.0, at);
        assert!((l.scale - 25_000.0).abs() < 1e-6);
        let after = l.to_page(before);
        assert!((after[0] - at[0]).abs() < 1e-6 && (after[1] - at[1]).abs() < 1e-6);
    }

    #[test]
    fn fit_frames_the_layers() {
        let mut l = layout_with(vec![Geometry::LineString(vec![[104.85, 11.50], [104.95, 11.60]])]);
        l.fit();
        let f = l.frame();
        for c in [[104.85, 11.50], [104.95, 11.60]] {
            let p = l.to_page(c);
            assert!(f.contains(p), "{p:?} outside {f:?}");
        }
        assert_eq!(l.scale, round_scale(l.scale));
    }

    #[test]
    fn scale_bar_and_numbers() {
        let mut l = Layout::default();
        l.scale = 50_000.0;
        // 40 mm at 1:50,000 is 2 km: the bar is exactly that.
        let (len, label) = l.scale_bar();
        assert_eq!(label, "2 km");
        assert!((len - 40.0).abs() < 1e-9);
        l.scale = 2_500.0;
        assert_eq!(l.scale_bar().1, "100 m");
        assert_eq!(thousands(1_234_567.0), "1,234,567");
        assert_eq!(round_scale(23_456.0), 23_000.0);
        assert_eq!(nice_length(7.0), 5.0);
    }

    #[test]
    fn tile_count_is_capped() {
        let mut l = Layout::default();
        l.basemap = Basemap::Streets;
        l.page = PageSize::A2;
        l.scale = 5_000.0;
        let (z, tiles) = l.tiles(300.0);
        assert!(tiles.len() <= MAX_TILES && !tiles.is_empty(), "{}", tiles.len());
        let (z_low, _) = l.tiles(72.0);
        assert!(z_low <= z);
    }

    #[test]
    fn svg_has_layers_legend_and_elements() {
        let mut l = layout_with(vec![
            Geometry::Polygon(vec![vec![[104.90, 11.55], [104.93, 11.55], [104.93, 11.57], [104.90, 11.55]]]),
            Geometry::Point([104.91, 11.56]),
        ]);
        l.title = "Parcels & roads".into();
        l.layers[0].label = "Land parcels".into();
        let svg = l.svg(&mut |_, _, _| None, 150.0);
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert!(svg.contains("fill-rule=\"evenodd\""));
        assert!(svg.contains("<circle"));
        assert!(svg.contains("Parcels &amp; roads"));
        assert!(svg.contains("Land parcels"));
        assert!(svg.contains("1:50,000"));
        // Hidden elements and layers leave no trace.
        l.item_mut(Element::Legend).visible = false;
        l.layers[0].visible = false;
        let svg = l.svg(&mut |_, _, _| None, 150.0);
        assert!(!svg.contains("Land parcels") && !svg.contains("evenodd"));
    }

    #[test]
    fn categories_color_and_label_features() {
        let mut l = layout_with(vec![Geometry::Point([104.91, 11.56]), Geometry::Point([104.92, 11.56]), Geometry::Point([104.93, 11.56])]);
        l.layers[0].values = vec![Some("garden".to_string()), Some("public".to_string()), None].into();
        let class = |v: &str, label: &str, color| Class { value: Some(v.into()), label: label.into(), color, visible: true, count: 1 };
        l.layers[0].style = Style::Categories { column: "land_use".into(), classes: vec![class("garden", "Gardens", [0, 200, 0]), class("public", "Public land", [0, 0, 200])], more: true };
        let svg = l.svg(&mut |_, _, _| None, 150.0);
        assert!(svg.contains("#00c800") && svg.contains("#0000c8"));
        assert!(svg.contains("Gardens") && svg.contains("Public land") && svg.contains(">Other<"));
        // A hidden class is neither drawn nor listed.
        if let Style::Categories { classes, .. } = &mut l.layers[0].style {
            classes[1].visible = false;
        }
        let svg = l.svg(&mut |_, _, _| None, 150.0);
        assert!(!svg.contains("#0000c8") && !svg.contains("Public land"));
    }

    #[test]
    fn overlapping_polygons_dont_cut_each_other() {
        let square = |x: f64| Geometry::Polygon(vec![vec![[x, 11.55], [x + 0.02, 11.55], [x + 0.02, 11.57], [x, 11.57], [x, 11.55]]]);
        let l = layout_with(vec![square(104.90), square(104.91)]);
        let svg = l.svg(&mut |_, _, _| None, 150.0);
        // Each polygon is its own path, so even-odd only applies within it.
        let group = svg.split("fill-rule=\"evenodd\"").nth(1).and_then(|g| g.split("</g>").next()).unwrap();
        assert_eq!(group.matches("<path ").count(), 2, "{group}");
    }

    #[test]
    fn tiles_are_embedded() {
        let mut l = Layout::default();
        l.page = PageSize::A5;
        l.scale = 2_000_000.0;
        let png = vec![0x89, b'P', b'N', b'G'];
        let svg = l.svg(&mut |_, _, _| Some(png.clone()), 72.0);
        assert!(svg.contains("data:image/png;base64,iVBORw=="));
        assert!(svg.contains(Basemap::Light.attribution().split(',').next().unwrap()));
    }

    #[test]
    fn layer_sql_and_base64() {
        let mut layer = Layer::new(1, "demo", "parcels", "geom", [0, 0, 0]);
        let sql = layer.data_sql([104.8, 11.5, 105.0, 11.7], 50_000.0, 100);
        assert!(sql.contains("ST_MakeEnvelope(104.8, 11.5, 105, 11.7, 4326)"), "{sql}");
        assert!(sql.contains("ST_Transform(\"geom\"::geometry, 4326)") && sql.ends_with("limit 100"), "{sql}");
        layer.filter = "land_use = 'garden'".into();
        layer.style = Style::Categories { column: "land_use".into(), classes: Vec::new(), more: false };
        let sql = layer.data_sql([0.0, 0.0, 1.0, 1.0], 1000.0, 5);
        assert!(sql.contains(", (\"land_use\")::text from") && sql.contains(" and (land_use = 'garden') limit 5"), "{sql}");
        assert!(layer.extent_sql().contains("ST_Extent(\"geom\"::geometry)"));
        assert_eq!(layer.classes_sql("land_use", 20), "select (\"land_use\")::text, count(*) from \"demo\".\"parcels\" where land_use = 'garden' group by 1 order by 2 desc, 1 limit 20");
        // Reload when the view leaves what's loaded, or the scale moves a lot.
        layer.loaded = Some(([0.0, 0.0, 10.0, 10.0], 10_000.0));
        assert!(!layer.needs_reload([1.0, 1.0, 9.0, 9.0], 12_000.0));
        assert!(layer.needs_reload([1.0, 1.0, 11.0, 9.0], 12_000.0));
        assert!(layer.needs_reload([1.0, 1.0, 9.0, 9.0], 4_000.0));
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn items_stay_on_the_page() {
        let mut l = Layout::default();
        l.item_mut(Element::North).pos = [500.0, -20.0];
        l.clamp_items();
        let b = l.boxes()[Element::North as usize];
        let (w, _) = l.page_mm();
        assert!(b.x + b.w <= w && b.y >= 0.0);
    }
}
