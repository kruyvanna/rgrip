use super::*;

/// The first `max` chars of `s` on one line (newlines shown as ↵).
pub(super) fn cell_text(s: &str, max: usize) -> String {
    let cut = s.char_indices().nth(max).map_or(s.len(), |(i, _)| i);
    let head = &s[..cut];
    let mut out = if head.contains('\n') { head.replace('\n', "↵ ") } else { head.to_owned() };
    if cut < s.len() {
        out.push('…');
    }
    out
}

/// Column indexes in display order: `order`'s names first, then the rest as queried.
pub(super) fn display_order(columns: &[String], order: &[String]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::with_capacity(columns.len());
    for name in order {
        if let Some(c) = columns.iter().position(|c| c == name)
            && !out.contains(&c)
        {
            out.push(c);
        }
    }
    let rest: Vec<usize> = (0..columns.len()).filter(|c| !out.contains(c)).collect();
    out.extend(rest);
    out
}

/// `list` with `item` moved to slot `to` (counted in `list` without `item`).
pub(super) fn move_to<T: PartialEq + Clone>(list: &[T], item: &T, to: usize) -> Vec<T> {
    let mut out: Vec<T> = list.iter().filter(|x| *x != item).cloned().collect();
    out.insert(to.min(out.len()), item.clone());
    out
}

pub(super) fn column_widths(rs: &ResultSet) -> Vec<f32> {
    rs.columns
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let widest = rs
                .rows
                .iter()
                .take(100)
                // Bounded scan: long values are clamped to the max width anyway.
                .map(|r| r[i].as_deref().map_or(4, |s| s.chars().take(60).take_while(|c| *c != '\n').count()))
                .max()
                .unwrap_or(0)
                .max(name.chars().count() + 2);
            (widest as f32 * CHAR_W + 2.0 * PAD + 8.0).clamp(64.0, 360.0)
        })
        .collect()
}
