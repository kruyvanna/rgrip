use super::*;

/// Estimated width of `text` at `size` mm in a sans-serif face.
pub fn text_w(text: &str, size: f64) -> f64 {
    let mut w = 0.0;
    let mut after_coeng = false;
    for c in text.chars() {
        w += match c {
            // Khmer: vowel signs and marks stack on the consonant, and a
            // consonant after the coeng sign is written below it.
            '\u{17B4}'..='\u{17DD}' | '\u{200B}'..='\u{200D}' => 0.0,
            '\u{1780}'..='\u{17B3}' if after_coeng => 0.0,
            '\u{1780}'..='\u{17B3}' => 0.62,
            c if c.is_ascii_uppercase() || c == 'm' || c == 'w' => 0.68,
            c if " il.,:;|'".contains(c) => 0.3,
            _ => 0.54,
        };
        after_coeng = c == '\u{17D2}';
    }
    w * size
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
