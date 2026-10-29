use super::*;

impl Layout {
    /// The page as SVG (mm). `tile` returns a basemap tile's image bytes
    /// (PNG or JPEG), `None` to leave it out; `dpi` picks the tile zoom.
    pub fn svg(&self, tile: &mut dyn FnMut(u8, u32, u32) -> Option<Vec<u8>>, dpi: f64) -> String {
        self.svg_with(tile, dpi, true)
    }

    /// The page without the title, legend, scale bar and north arrow (the
    /// preview draws those separately, so dragging them needs no re-render).
    pub fn svg_map(&self, tile: &mut dyn FnMut(u8, u32, u32) -> Option<Vec<u8>>, dpi: f64) -> String {
        self.svg_with(tile, dpi, false)
    }

    /// One element alone, as a small SVG, and the page area (mm) it covers
    /// (its box with a little room for strokes and halos). `None` when hidden.
    pub fn element_svg(&self, e: Element) -> Option<(String, Rect)> {
        if !self.item(e).visible || (e == Element::Title && self.title.trim().is_empty()) || (e == Element::Legend && self.layers.is_empty()) {
            return None;
        }
        let b = self.boxes()[e as usize];
        let pad = 1.5;
        let r = Rect { x: b.x - pad, y: b.y - pad, w: b.w + 2.0 * pad, h: b.h + 2.0 * pad };
        let mut s = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}mm" height="{}mm" viewBox="{} {} {} {}" font-family="Helvetica Neue, Helvetica, Arial, sans-serif">"#,
            n(r.w),
            n(r.h),
            n(r.x),
            n(r.y),
            n(r.w),
            n(r.h)
        );
        self.elements_svg(&mut s, Some(e));
        s.push_str("</svg>");
        Some((s, r))
    }

    fn svg_with(&self, tile: &mut dyn FnMut(u8, u32, u32) -> Option<Vec<u8>>, dpi: f64, elements: bool) -> String {
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
        let mut grid_labels = String::new();
        if self.graticule.visible {
            // Lines are clipped to the frame; labels may sit in the margin.
            let mut grid = String::new();
            self.graticule_svg(&mut grid);
            let split = grid.find("<text").unwrap_or(grid.len());
            s.push_str(&grid[..split]);
            grid_labels = grid[split..].to_string();
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
        s.push_str(&grid_labels);
        if elements {
            self.elements_svg(&mut s, None);
        }
        s.push_str("</svg>");
        s
    }

    fn layer_svg(&self, s: &mut String, l: &Layer) {
        match &l.style {
            Style::Single => self.group_svg(s, l, l.shapes.iter(), l.color),
            Style::Categories { classes, .. } => {
                let classes: Vec<&Class> = Style::shown(classes).collect();
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
}

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

/// A number for SVG: two decimals, trailing zeros dropped.
pub(super) fn n(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

pub(super) fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub(super) fn esc(s: &str) -> String {
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
