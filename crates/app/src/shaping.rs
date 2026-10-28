//! Shaped rendering for complex scripts (Khmer).
//!
//! egui lays text out glyph-by-codepoint with no OpenType shaping, which
//! breaks Khmer: subscript consonants (coeng) don't stack and pre-base vowels
//! render in the wrong order. For display-only text we shape with rustybuzz
//! (a HarfBuzz port), rasterize the glyphs with ab_glyph and draw the result
//! as a cached texture.

use ab_glyph::{Font, FontRef, GlyphId, PxScale, point};
use eframe::egui::{self, Color32, ColorImage, Painter, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Longest text shaped (cells show one line).
const MAX_CHARS: usize = 300;
/// Widest image produced, in pixels; also capped by the GPU's texture limit.
const MAX_WIDTH_PX: f32 = 4000.0;

pub fn is_complex(text: &str) -> bool {
    text.chars().any(is_khmer)
}

fn is_khmer(c: char) -> bool {
    matches!(c, '\u{1780}'..='\u{17FF}' | '\u{19E0}'..='\u{19FF}')
}

/// Split into maximal Khmer / non-Khmer runs. Spaces, ZWJ/ZWNJ and other
/// script-neutral characters stay with the run they're in.
fn script_runs(text: &str) -> Vec<&str> {
    let mut runs = Vec::new();
    let mut start = 0;
    let mut current: Option<bool> = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() || matches!(c, '\u{200B}'..='\u{200D}') {
            continue;
        }
        let k = is_khmer(c);
        if current.is_some_and(|cur| cur != k) {
            runs.push(&text[start..i]);
            start = i;
        }
        current = Some(k);
    }
    if start < text.len() {
        runs.push(&text[start..]);
    }
    runs
}

fn shape_run(face: &rustybuzz::Face, run: &str) -> rustybuzz::GlyphBuffer {
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(run);
    buffer.guess_segment_properties();
    // Zero-width spaces (common in Khmer) and stray marks would otherwise
    // show up as boxes.
    buffer.set_flags(rustybuzz::BufferFlags::REMOVE_DEFAULT_IGNORABLES | rustybuzz::BufferFlags::DO_NOT_INSERT_DOTTED_CIRCLE);
    rustybuzz::shape(face, &[], buffer)
}

struct Shaper {
    bytes: &'static [u8],
    cache: Mutex<HashMap<(String, u32), Arc<Shaped>>>,
}

pub struct Shaped {
    texture: TextureHandle,
    /// Size in points.
    pub size: Vec2,
}

static SHAPER: OnceLock<Option<Shaper>> = OnceLock::new();

/// Load the Khmer font once. Call at startup with the same font the theme
/// uses as fallback, so shaped and unshaped text look alike.
pub fn init(font: Vec<u8>) {
    let _ = SHAPER.set(Some(Shaper { bytes: Box::leak(font.into_boxed_slice()), cache: Mutex::default() }));
}

fn shaper() -> Option<&'static Shaper> {
    SHAPER.get_or_init(|| None).as_ref()
}

/// Shape and rasterize `text` at `size` points. White glyphs with alpha;
/// tint when drawing. `None` if no Khmer font is available.
pub fn shape(ctx: &egui::Context, text: &str, size: f32) -> Option<Arc<Shaped>> {
    let shaper = shaper()?;
    // Callers show one line; long text would also exceed GPU texture limits.
    let text = match text.char_indices().nth(MAX_CHARS) {
        Some((i, _)) => &text[..i],
        None => text,
    };
    let ppp = ctx.pixels_per_point();
    let px = size * ppp;
    let max_width = MAX_WIDTH_PX.min(ctx.input(|i| i.max_texture_side) as f32 - 4.0 * px);
    let key = (text.to_string(), (px * 10.0) as u32);
    if let Some(hit) = shaper.cache.lock().unwrap().get(&key) {
        return Some(hit.clone());
    }

    let face = rustybuzz::Face::from_slice(shaper.bytes, 0)?;
    let font = FontRef::try_from_slice(shaper.bytes).ok()?;
    let units = face.units_per_em() as f32;
    let scale = px / units;
    let ascent = face.ascender() as f32 * scale;
    let descent = -face.descender() as f32 * scale;
    let pad = 2.0;
    // ab_glyph's PxScale is line height, HarfBuzz works in em units.
    let glyph_scale = PxScale::from(px * font.height_unscaled() / font.units_per_em()?);
    let mut pen = 0.0;
    let mut placed = Vec::new();
    // HarfBuzz shapes one script per buffer, so mixed text is shaped run by run.
    for run in script_runs(text) {
        let glyphs = shape_run(&face, run);
        for (info, pos) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
            let x = pen + pos.x_offset as f32 * scale;
            let y = ascent + pad - pos.y_offset as f32 * scale;
            if pen > max_width {
                break;
            }
            // Glyph 0 is the font's "missing" box.
            if info.glyph_id != 0 {
                placed.push(GlyphId(info.glyph_id as u16).with_scale_and_position(glyph_scale, point(x + pad, y)));
            }
            pen += pos.x_advance as f32 * scale;
        }
    }
    // The last glyph placed may extend up to ~2em past `max_width`.
    let width = (pen.min(max_width + px * 2.0) + 2.0 * pad).ceil().max(1.0) as usize;
    let height = (ascent + descent + 2.0 * pad).ceil().max(1.0) as usize;
    let mut alpha = vec![0.0f32; width * height];
    for glyph in placed {
        let Some(outline) = font.outline_glyph(glyph) else { continue };
        let b = outline.px_bounds();
        outline.draw(|gx, gy, cov| {
            let x = b.min.x as i32 + gx as i32;
            let y = b.min.y as i32 + gy as i32;
            if (0..width as i32).contains(&x) && (0..height as i32).contains(&y) {
                let a = &mut alpha[y as usize * width + x as usize];
                *a = (*a + cov).min(1.0);
            }
        });
    }
    let pixels = alpha
        .iter()
        .map(|a| Color32::from_white_alpha((a * 255.0).round() as u8))
        .collect();
    let image = ColorImage { size: [width, height], pixels, source_size: Vec2::new(width as f32, height as f32) };
    let texture = ctx.load_texture(format!("shaped:{text}"), image, TextureOptions::NEAREST);
    let shaped = Arc::new(Shaped {
        texture,
        size: Vec2::new(width as f32, height as f32) / ppp,
    });

    let mut cache = shaper.cache.lock().unwrap();
    if cache.len() > 4000 {
        cache.clear();
    }
    cache.insert(key, shaped.clone());
    Some(shaped)
}

impl Shaped {
    /// Draw with the left edge at `left`, vertically centred on `center_y`,
    /// clipped to `clip`.
    pub fn paint(&self, painter: &Painter, left: f32, center_y: f32, color: Color32, clip: Rect) {
        // The image is rasterized at device resolution: put it on whole
        // physical pixels, or sampling between them blurs the glyphs.
        let ppp = painter.ctx().pixels_per_point();
        let snap = |v: f32| (v * ppp).round() / ppp;
        let rect = Rect::from_min_size(Pos2::new(snap(left), snap(center_y - self.size.y / 2.0)), self.size);
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.with_clip_rect(clip).image(self.texture.id(), rect, uv, color);
    }
}

/// A label that shapes complex scripts and falls back to a normal label.
pub fn label(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) -> egui::Response {
    match is_complex(text).then(|| shape(ui.ctx(), text, size)).flatten() {
        Some(shaped) => {
            let width = shaped.size.x.min(ui.available_width().max(0.0));
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, shaped.size.y), egui::Sense::hover());
            shaped.paint(ui.painter(), rect.left(), rect.center().y, color, rect);
            resp
        }
        None => ui.add(egui::Label::new(egui::RichText::new(text).size(size).color(color)).truncate().selectable(false)),
    }
}

/// Byte ranges of Khmer text: maximal runs of Khmer characters, with
/// zero-width (non-)joiners and spaces inside a run kept in it.
pub fn khmer_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut run: Option<std::ops::Range<usize>> = None;
    for (i, c) in text.char_indices() {
        let end = i + c.len_utf8();
        if is_khmer(c) {
            match &mut run {
                Some(r) => r.end = end,
                None => run = Some(i..end),
            }
        } else if matches!(c, '\u{200B}'..='\u{200D}') && run.is_some() {
            // Joiners belong to the run; a run never ends on one (below).
        } else if let Some(r) = run.take() {
            out.push(r);
        }
    }
    out.extend(run);
    out
}

/// For an editable text: hide the Khmer runs in `job` (egui would draw them
/// unshaped) and space their glyphs so each run is as wide as its shaped
/// form, so the cursor and the text after it line up. Returns the runs and
/// their colors, for [`paint_runs`] after the text is laid out.
pub fn hide_runs(ui: &egui::Ui, job: &mut egui::text::LayoutJob) -> Vec<(std::ops::Range<usize>, Color32)> {
    let ranges = khmer_ranges(&job.text);
    if ranges.is_empty() || shaper().is_none() {
        return Vec::new();
    }
    let ppp = ui.ctx().pixels_per_point();
    let mut runs = Vec::new();
    let mut spacing = Vec::new();
    for r in &ranges {
        let Some(section) = job.sections.iter().find(|s| s.byte_range.contains(&r.start)) else { continue };
        let (font, color) = (section.format.font_id.clone(), section.format.color);
        let run = &job.text[r.clone()];
        let Some(shaped) = shape(ui.ctx(), run, font.size) else { continue };
        let plain = ui.fonts_mut(|f| f.layout_no_wrap(run.to_string(), font.clone(), color)).size().x;
        let wanted = shaped.size.x - 4.0 / ppp;
        spacing.push((r.clone(), (wanted - plain) / run.chars().count() as f32));
        runs.push((r.clone(), color));
    }
    // Split sections at run edges; the parts inside a run go invisible.
    let mut sections = Vec::new();
    for s in job.sections.drain(..) {
        let mut at = s.byte_range.start;
        while at < s.byte_range.end {
            let inside = spacing.iter().find(|(r, _)| r.contains(&at));
            let end = match inside {
                Some((r, _)) => r.end.min(s.byte_range.end),
                None => spacing.iter().map(|(r, _)| r.start).filter(|&x| x > at).min().unwrap_or(s.byte_range.end).min(s.byte_range.end),
            };
            let mut part = s.clone();
            part.byte_range = at..end;
            if let Some((_, extra)) = inside {
                part.format.color = Color32::TRANSPARENT;
                part.format.extra_letter_spacing = *extra;
            }
            sections.push(part);
            at = end;
        }
    }
    job.sections = sections;
    runs
}

/// Draw the runs [`hide_runs`] hid, shaped, where the galley put them.
pub fn paint_runs(ui: &egui::Ui, galley: &egui::Galley, galley_pos: Pos2, runs: &[(std::ops::Range<usize>, Color32)]) {
    let ppp = ui.ctx().pixels_per_point();
    let text = galley.text();
    for (r, color) in runs {
        let Some(run) = text.get(r.clone()) else { continue };
        let size = galley.job.sections.iter().find(|s| s.byte_range.contains(&r.start)).map_or(14.0, |s| s.format.font_id.size);
        let Some(shaped) = shape(ui.ctx(), run, size) else { continue };
        let index = text[..r.start].chars().count();
        let at = galley.pos_from_cursor(egui::text::CCursor::new(index));
        let left = galley_pos.x + at.min.x - 2.0 / ppp;
        shaped.paint(ui.painter(), left, galley_pos.y + at.center().y, *color, ui.clip_rect());
    }
}

#[cfg(test)]
mod tests {
    use super::{khmer_ranges, script_runs, shape_run};

    #[test]
    fn finds_khmer_runs() {
        let t = "select 1; -- កូនខ្ញុំ\u{200B}ស្រី ok";
        let runs: Vec<&str> = khmer_ranges(t).into_iter().map(|r| &t[r]).collect();
        assert_eq!(runs, ["កូនខ្ញុំ\u{200B}ស្រី"]);
        assert!(khmer_ranges("plain").is_empty());
    }

    #[test]
    fn no_missing_glyph_boxes() {
        // The fonts the app may load (see theme::install_fonts).
        let home = std::env::var("HOME").unwrap_or_default();
        let fonts = [
            format!("{home}/Library/Fonts/KantumruyPro-VariableFont_wght.ttf"),
            "/Library/Fonts/KantumruyPro-VariableFont_wght.ttf".to_string(),
            "/System/Library/Fonts/Supplemental/Khmer Sangam MN.ttf".to_string(),
        ];
        for bytes in fonts.iter().filter_map(|p| std::fs::read(p).ok()) {
            let face = rustybuzz::Face::from_slice(&bytes, 0).unwrap();
            let dotted = face.glyph_index('\u{25CC}').map_or(u32::MAX, |g| g.0 as u32);
            // Zero-width space, a mark with no base, a doubled vowel sign.
            for text in ["ការ៉េ\u{200B}រឹ", "ការ៉េ\u{17B9}រ", "ការ៉េ\u{17C1}\u{17C1}"] {
                for run in script_runs(text) {
                    let glyphs = shape_run(&face, run);
                    assert!(glyphs.glyph_infos().iter().all(|g| g.glyph_id != 0 && g.glyph_id != dotted), "{text:?}");
                }
            }
        }
    }

    #[test]
    fn runs() {
        assert_eq!(script_runs("Mixed ខ្មែរ 123"), ["Mixed ", "ខ្មែរ ", "123"]);
        assert_eq!(script_runs("ភ្នំពេញ"), ["ភ្នំពេញ"]);
        assert_eq!(script_runs("abc"), ["abc"]);
        assert!(script_runs("").is_empty());
    }
}
