use super::*;

impl Layout {
    /// The page elements (or just `only`).
    pub(super) fn elements_svg(&self, s: &mut String, only: Option<Element>) {
        let want = |e: Element| only.is_none_or(|o| o == e);
        let boxes = self.boxes();
        let card = r##"fill="#fff" fill-opacity="0.92" stroke="#222" stroke-opacity="0.25" stroke-width="0.25""##;
        if want(Element::Title) && self.item(Element::Title).visible && !self.title.trim().is_empty() {
            let b = boxes[Element::Title as usize];
            let ts = self.title_size * PT;
            let halo = if self.title_halo { format!(r##" stroke="#fff" stroke-width="{}" stroke-linejoin="round" paint-order="stroke""##, n(ts * 0.18)) } else { String::new() };
            let _ = write!(
                s,
                r##"<text x="{}" y="{}" font-size="{}" font-weight="{}" text-anchor="middle" fill="{}"{halo}>{}</text>"##,
                n(b.x + b.w / 2.0),
                n(b.y + ts * 1.1),
                n(ts),
                if self.title_bold { 600 } else { 400 },
                hex(self.title_color),
                esc(self.title.trim())
            );
        }
        if want(Element::Legend) && self.item(Element::Legend).visible && !self.layers.is_empty() {
            // Laid out at the default size, then scaled about its corner.
            let k = self.legend_scale();
            let b0 = boxes[Element::Legend as usize];
            let b = Rect { w: b0.w / k, h: b0.h / k, ..b0 };
            let ink = hex(self.legend_color);
            let _ = write!(s, r#"<g transform="translate({} {}) scale({}) translate({} {})">"#, n(b.x), n(b.y), k, n(-b.x), n(-b.y));
            if self.legend_box {
                let _ = write!(s, r#"<rect x="{}" y="{}" width="{}" height="{}" rx="1" {card}/>"#, n(b.x), n(b.y), n(b.w), n(b.h));
            }
            let _ = write!(s, r##"<text x="{}" y="{}" font-size="{}" font-weight="600" fill="{ink}">{}</text>"##, n(b.x + 4.0), n(b.y + 6.0), n(LEGEND_TITLE), esc(&self.legend_title));
            // Style per row comes from its layer (line width, fill, point size).
            let mut layer_of_row = Vec::new();
            for l in self.layers.iter().filter(|l| l.visible) {
                let mut rows = Vec::new();
                l.legend_rows(&mut rows);
                layer_of_row.extend(std::iter::repeat_n(l, rows.len()));
            }
            let mut y = b.y + 9.0;
            for (row, l) in self.legend_rows().iter().zip(layer_of_row) {
                let row_y = y;
                y += row_height(&row.label);
                let y = row_y;
                let (sx, sy) = (b.x + 4.0 + if row.indent { 3.0 } else { 0.0 }, y + 1.0);
                let Some((kind, c)) = row.swatch else {
                    let _ = write!(s, r##"<text x="{}" y="{}" font-size="{}" font-weight="600" fill="{ink}">{}</text>"##, n(sx), n(sy + 3.0), n(LEGEND_TEXT), lines(&row.label, sx));
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
                let _ = write!(s, r##"<text x="{}" y="{}" font-size="{}" fill="{ink}">{}</text>"##, n(sx + 8.0), n(sy + 3.0), n(LEGEND_TEXT), lines(&row.label, sx + 8.0));
            }
            s.push_str("</g>");
        }
        if want(Element::ScaleBar) && self.item(Element::ScaleBar).visible {
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
        if want(Element::North) && self.item(Element::North).visible {
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

pub(super) const LEGEND_TITLE: f64 = 3.4;
pub(super) const LEGEND_TEXT: f64 = 3.0;
/// Extra height of each further line of a legend label.
pub(super) const LEGEND_LINE: f64 = 3.8;

/// A legend row's height: one line, plus each line break in its label.
pub(super) fn row_height(label: &str) -> f64 {
    6.0 + label.trim_end().lines().count().saturating_sub(1) as f64 * LEGEND_LINE
}

/// Label text for SVG, one `tspan` per line (labels may hold line breaks).
fn lines(label: &str, x: f64) -> String {
    let mut out = String::new();
    for (i, line) in label.trim_end().lines().enumerate() {
        if i == 0 {
            out.push_str(&esc(line));
        } else {
            let _ = write!(out, r#"<tspan x="{}" dy="{}">{}</tspan>"#, n(x), n(LEGEND_LINE), esc(line));
        }
    }
    out
}
