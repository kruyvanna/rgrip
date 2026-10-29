use super::*;

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
    pub title_color: [u8; 3],
    pub title_bold: bool,
    /// A white outline around the title letters (reads on any map).
    pub title_halo: bool,
    pub legend_title: String,
    /// Legend item text size, points (the rest of the legend scales with it).
    pub legend_size: f64,
    pub legend_color: [u8; 3],
    /// The legend's white box.
    pub legend_box: bool,
    pub items: [Item; 4],
    pub graticule: Graticule,
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
            title_color: [17, 17, 17],
            title_bold: true,
            title_halo: true,
            legend_title: "Legend".into(),
            legend_size: LEGEND_TEXT / PT,
            legend_color: [34, 34, 34],
            legend_box: true,
            items: [Item { visible: true, pos: [0.0, 0.0] }; 4],
            graticule: Graticule::default(),
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
    /// Center and scale to a lon/lat box (at least `min_scale`).
    pub fn fit_bounds(&mut self, lo: Coord, hi: Coord, min_scale: f64) {
        let (a, b) = (merc(lo), merc(hi));
        let mid = unmerc([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
        let f = self.frame();
        let span = ((b[0] - a[0]).abs() / f.w).max((b[1] - a[1]).abs() / f.h).max(1e-9) * 1.12;
        self.center = mid;
        self.scale = round_scale(span * 1000.0 * mid[1].to_radians().cos()).clamp(min_scale.max(100.0), 50_000_000.0);
    }

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
        let rows_h = legend_rows.iter().map(|r| row_height(&r.label)).sum::<f64>().max(6.0);
        let label_w = legend_rows
            .iter()
            .map(|r| r.label.lines().map(|l| text_w(l, LEGEND_TEXT)).fold(0.0, f64::max) + if r.indent { 3.0 } else { 0.0 })
            .fold(0.0, f64::max);
        let head_w = text_w(&self.legend_title, LEGEND_TITLE);
        let k = self.legend_scale();
        let legend = Rect {
            x: at(Element::Legend)[0],
            y: at(Element::Legend)[1],
            w: (label_w + 13.0).max(head_w + 8.0).max(28.0) * k,
            h: (9.0 + rows_h + 2.0) * k,
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

    /// The legend's size relative to its default (from the text size).
    pub fn legend_scale(&self) -> f64 {
        (self.legend_size * PT / LEGEND_TEXT).clamp(0.4, 4.0)
    }
}
