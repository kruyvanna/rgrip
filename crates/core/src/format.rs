//! SQL formatting for the editor. Slash command lines are kept as they are
//! (they aren't SQL); everything between them is formatted.

use sqlformat::{Dialect, FormatOptions, Indent, QueryParams};

pub fn format_sql(sql: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut chunk: Vec<&str> = Vec::new();
    for line in sql.split('\n') {
        if crate::commands::is_command_line(line) {
            flush(&mut chunk, &mut out);
            out.push(line.trim_end().to_string());
        } else {
            chunk.push(line);
        }
    }
    flush(&mut chunk, &mut out);
    let mut text = out.join("\n");
    if sql.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// ⌘/: comment the lines the selection touches with `-- `, or uncomment them
/// when every non-blank one already is. `sel` is a byte range; returns the new
/// text and where the selection ends up.
pub fn toggle_comment(text: &str, sel: std::ops::Range<usize>) -> (String, std::ops::Range<usize>) {
    let start = text[..sel.start].rfind('\n').map_or(0, |i| i + 1);
    // A selection ending at the start of a line leaves that line alone.
    let last = if sel.end > sel.start && text[..sel.end].ends_with('\n') { sel.end - 1 } else { sel.end };
    let end = text[last..].find('\n').map_or(text.len(), |i| last + i);
    let lines: Vec<(usize, &str)> = text[start..end]
        .split('\n')
        .scan(start, |at, l| {
            let here = *at;
            *at += l.len() + 1;
            Some((here, l))
        })
        .collect();
    let filled: Vec<&(usize, &str)> = lines.iter().filter(|(_, l)| !l.trim().is_empty()).collect();
    let uncomment = !filled.is_empty() && filled.iter().all(|(_, l)| l.trim_start().starts_with("--"));
    // (where, bytes removed, text inserted), in order.
    let mut edits: Vec<(usize, usize, &str)> = Vec::new();
    if uncomment {
        for (at, l) in &filled {
            let indent = l.len() - l.trim_start().len();
            let rest = &l[indent + 2..];
            edits.push((at + indent, if rest.starts_with(' ') { 3 } else { 2 }, ""));
        }
    } else if filled.is_empty() {
        edits.push((lines[0].0 + lines[0].1.len(), 0, "-- "));
    } else {
        // At the shallowest indent, so the comment markers line up.
        let indent = filled.iter().map(|(_, l)| l.len() - l.trim_start().len()).min().unwrap_or(0);
        for (at, _) in &filled {
            edits.push((at + indent, 0, "-- "));
        }
    }
    let mut out = String::with_capacity(text.len() + edits.len() * 3);
    let mut from = 0;
    for (at, removed, inserted) in &edits {
        out.push_str(&text[from..*at]);
        out.push_str(inserted);
        from = at + removed;
    }
    out.push_str(&text[from..]);
    // A selection's start stays put at an insertion; a lone cursor moves past it.
    let map = |p: usize, before: bool| {
        let mut shift = 0isize;
        for (at, removed, inserted) in &edits {
            let grow = inserted.len() as isize - *removed as isize;
            if p > at + removed || (p == *at && *removed == 0 && !before) || (p == at + removed && *removed > 0) {
                shift += grow;
            } else if p > *at {
                shift += *at as isize - p as isize;
            }
        }
        (p as isize + shift) as usize
    };
    let empty = sel.start == sel.end;
    (out, map(sel.start, !empty)..map(sel.end, false))
}

fn flush(chunk: &mut Vec<&str>, out: &mut Vec<String>) {
    let text = chunk.join("\n");
    chunk.clear();
    if text.trim().is_empty() {
        return;
    }
    let options = FormatOptions {
        indent: Indent::Spaces(2),
        lines_between_queries: 2,
        max_inline_arguments: Some(60),
        max_inline_top_level: Some(60),
        joins_as_top_level: true,
        dialect: Dialect::PostgreSql,
        ..FormatOptions::default()
    };
    out.push(sqlformat::format(&text, &QueryParams::None, &options));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sql_and_keeps_commands() {
        let out = format_sql("select a, b from t where a = 1;\n/count parcels\nselect 1 -- note");
        assert!(out.starts_with("select a, b\nfrom t\nwhere a = 1;\n/count parcels\n"), "{out}");
        assert!(out.contains("-- note"), "{out}");
        assert_eq!(format_sql("  \n"), "\n");
        let geo = format_sql("select t.*, ST_Transform(t.shape, 4326) as g from gis.parcels t join roads r on r.id = t.road_id where t.a::int > 2 and t.tags @> array['x'] order by 1 limit 10;");
        assert_eq!(
            geo,
            "select t.*, ST_Transform(t.shape, 4326) as g\nfrom gis.parcels t\njoin roads r on r.id = t.road_id\nwhere t.a::int > 2 and t.tags @> array['x']\norder by 1\nlimit 10;"
        );
    }

    #[test]
    fn toggles_comments() {
        // The cursor's line, and back.
        let (t, sel) = toggle_comment("select 1;\nselect 2;", 12..12);
        assert_eq!(t, "select 1;\n-- select 2;");
        assert_eq!(sel, 15..15);
        let (t, sel) = toggle_comment(&t, 15..15);
        assert_eq!(t, "select 1;\nselect 2;");
        assert_eq!(sel, 12..12);
        // Lines a selection touches, at the shallowest indent, blank lines left alone.
        let text = "select a\n  from t\n\n  where b\nlimit 1";
        let (t, sel) = toggle_comment(text, 3..text.find("where").unwrap());
        assert_eq!(t, "-- select a\n--   from t\n\n--   where b\nlimit 1");
        assert_eq!(&t[sel.clone()], "ect a\n--   from t\n\n--   ");
        // One commented, one not: comment both.
        assert_eq!(toggle_comment("-- a\nb", 0..6).0, "-- -- a\n-- b");
        // All commented: uncomment, with or without the space.
        assert_eq!(toggle_comment("  -- a\n  --b", 0..8).0, "  a\n  b");
        // A selection ending at a line start doesn't take that line.
        assert_eq!(toggle_comment("a\nb\nc", 0..2).0, "-- a\nb\nc");
        assert_eq!(toggle_comment("", 0..0).0, "-- ");
    }

}
