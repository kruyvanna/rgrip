use super::*;

/// A latitude/longitude grid over the map, labeled along the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Graticule {
    pub visible: bool,
    /// Degrees between lines; `None` picks a round step for the view.
    pub interval: Option<f64>,
    pub labels: bool,
    /// Degrees, minutes, seconds instead of decimal degrees.
    pub dms: bool,
    pub color: [u8; 3],
    pub opacity: f64,
}

impl Default for Graticule {
    fn default() -> Self {
        Self { visible: false, interval: None, labels: true, dms: false, color: [60, 60, 60], opacity: 0.5 }
    }
}

/// Graticule steps, in degrees: 1″ … 30″, 1′ … 30′, 1° … 30°.
pub const GRATICULE_STEPS: [f64; 18] = [
    1.0 / 3600.0,
    2.0 / 3600.0,
    5.0 / 3600.0,
    10.0 / 3600.0,
    15.0 / 3600.0,
    30.0 / 3600.0,
    1.0 / 60.0,
    2.0 / 60.0,
    5.0 / 60.0,
    10.0 / 60.0,
    15.0 / 60.0,
    30.0 / 60.0,
    1.0,
    2.0,
    5.0,
    10.0,
    15.0,
    30.0,
];

/// Steps for decimal-degree labels, so they read 104.95°, not 104.9667°.
pub const GRATICULE_STEPS_DECIMAL: [f64; 16] = [0.0001, 0.0002, 0.0005, 0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0];

/// The steps that suit the label format.
pub fn graticule_steps(dms: bool) -> &'static [f64] {
    if dms { &GRATICULE_STEPS } else { &GRATICULE_STEPS_DECIMAL }
}

/// A graticule step for display: 30″, 5′, 2° (or 0.05° for decimal steps).
pub fn step_label(deg: f64) -> String {
    if GRATICULE_STEPS_DECIMAL.contains(&deg) && !GRATICULE_STEPS.contains(&deg) {
        let s = format!("{deg:.4}");
        return format!("{}°", s.trim_end_matches('0').trim_end_matches('.'));
    }
    let secs = (deg * 3600.0).round();
    if secs < 60.0 {
        format!("{secs}″")
    } else if secs < 3600.0 {
        format!("{}′", secs / 60.0)
    } else {
        format!("{}°", secs / 3600.0)
    }
}

/// A coordinate label: `lon` picks E/W, else N/S.
pub fn coord_label(v: f64, lon: bool, dms: bool) -> String {
    let hemi = match (lon, v < -1e-9) {
        (true, false) => "E",
        (true, true) => "W",
        (false, false) => "N",
        (false, true) => "S",
    };
    let a = v.abs();
    if !dms {
        let s = format!("{a:.4}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        return format!("{s}°{hemi}");
    }
    let total = (a * 3600.0).round() as i64;
    let (d, m, sec) = (total / 3600, total / 60 % 60, total % 60);
    match (m, sec) {
        (0, 0) => format!("{d}°{hemi}"),
        (_, 0) => format!("{d}°{m:02}′{hemi}"),
        _ => format!("{d}°{m:02}′{sec:02}″{hemi}"),
    }
}

impl Layout {
    /// The graticule step in degrees: the chosen one, or the smallest round
    /// step that leaves lines at least ~35 mm apart.
    pub fn graticule_step(&self) -> f64 {
        if let Some(i) = self.graticule.interval {
            return i;
        }
        let deg_per_mm = self.m_per_mm() / (2.0 * std::f64::consts::PI * R / 360.0);
        graticule_steps(self.graticule.dms).iter().copied().find(|s| s / deg_per_mm >= 35.0).unwrap_or(10.0)
    }

    pub(super) fn graticule_svg(&self, s: &mut String) {
        let g = self.graticule;
        let f = self.frame();
        let step = self.graticule_step();
        let [w, south, e, north] = self.view_bbox(0.0);
        // Too many lines (a tiny step at a small scale): skip rather than flood.
        if ((e - w) / step) > 200.0 || ((north - south) / step) > 200.0 {
            return;
        }
        let color = format!("#{:02x}{:02x}{:02x}", g.color[0], g.color[1], g.color[2]);
        let mut d = String::new();
        let mut lons = Vec::new();
        let mut lats = Vec::new();
        let mut v = (w / step).ceil() * step;
        while v <= e {
            let x = self.to_page([v, self.center[1]])[0];
            let _ = write!(d, "M{} {}V{}", n(x), n(f.y), n(f.y + f.h));
            lons.push((v, x));
            v += step;
        }
        let mut v = (south / step).ceil() * step;
        while v <= north {
            let y = self.to_page([self.center[0], v])[1];
            let _ = write!(d, "M{} {}H{}", n(f.x), n(y), n(f.x + f.w));
            lats.push((v, y));
            v += step;
        }
        let _ = write!(s, r#"<path d="{d}" fill="none" stroke="{color}" stroke-opacity="{}" stroke-width="0.18"/>"#, n(g.opacity));
        if !g.labels {
            return;
        }
        // In the margin when it has room, else just inside the frame.
        let [mt, mr, mb, ml] = self.margin;
        let size = 2.3;
        let halo = r##" stroke="#fff" stroke-width="0.5" paint-order="stroke""##;
        let text = |s: &mut String, x: f64, y: f64, anchor: &str, inside: bool, label: &str| {
            let _ = write!(s, r##"<text x="{}" y="{}" font-size="{size}" fill="#333" text-anchor="{anchor}"{}>{}</text>"##, n(x), n(y), if inside { halo } else { "" }, esc(label));
        };
        // Skip labels that would run past the frame's ends (they'd meet the
        // labels of the next side at the corners).
        let fits = |c: f64, lo: f64, len: f64, label: &str| {
            let half = text_w(label, size) / 2.0;
            c - half >= lo + 1.0 && c + half <= lo + len - 1.0
        };
        for (v, x) in &lons {
            let label = coord_label(*v, true, g.dms);
            if !fits(*x, f.x, f.w, &label) {
                continue;
            }
            let (top, bottom) = (mt >= 5.0, mb >= 5.0);
            text(s, *x, if top { f.y - 1.2 } else { f.y + size + 1.0 }, "middle", !top, &label);
            text(s, *x, if bottom { f.y + f.h + size + 1.0 } else { f.y + f.h - 1.2 }, "middle", !bottom, &label);
        }
        // Left and right: turned to run up the frame edge (as printed maps
        // do), so they fit a narrow margin; inside only when there's no room.
        for (v, y) in &lats {
            let label = coord_label(*v, false, g.dms);
            if !fits(*y, f.y, f.h, &label) {
                continue;
            }
            for (left, margin) in [(true, ml), (false, mr)] {
                let outside = margin >= size + 1.8;
                // Rotated -90°, the glyphs sit on the left of the baseline.
                let x = match (left, outside) {
                    (true, true) => f.x - 1.2,
                    (true, false) => f.x + 1.2 + size * 0.8,
                    (false, true) => f.x + f.w + 1.2 + size * 0.8,
                    (false, false) => f.x + f.w - 1.2,
                };
                let _ = write!(
                    s,
                    r##"<text x="{}" y="{}" transform="rotate(-90 {} {})" font-size="{size}" fill="#333" text-anchor="middle"{}>{}</text>"##,
                    n(x),
                    n(*y),
                    n(x),
                    n(*y),
                    if outside { "" } else { halo },
                    esc(&label)
                );
            }
        }
    }
}
