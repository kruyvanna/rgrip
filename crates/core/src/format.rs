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

/// `text` re-indented, when it is a JSON object or array; keys keep their
/// order (unlike a parse and print round trip).
pub fn pretty_json(text: &str) -> Option<String> {
    let text = text.trim();
    if !(text.starts_with('{') || text.starts_with('[')) || serde_json::from_str::<serde::de::IgnoredAny>(text).is_err() {
        return None;
    }
    let mut out = String::with_capacity(text.len() * 2);
    let (mut depth, mut in_str, mut escaped) = (0usize, false, false);
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    };
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                // Empty ones stay on one line: `{}`, `[]`.
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
                if matches!(chars.peek(), Some('}' | ']')) {
                    out.push(chars.next().unwrap());
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    Some(out)
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
    #[test]
    fn pretty_json_keeps_order() {
        let p = super::pretty_json(r#"{"name": "Sok \"A\", B", "z": 1, "a": [1, {"k": null}], "e": {}}"#).unwrap();
        assert_eq!(p, "{\n  \"name\": \"Sok \\\"A\\\", B\",\n  \"z\": 1,\n  \"a\": [\n    1,\n    {\n      \"k\": null\n    }\n  ],\n  \"e\": {}\n}");
        assert_eq!(super::pretty_json("[]").unwrap(), "[]");
        assert!(super::pretty_json("{oops").is_none());
        assert!(super::pretty_json("12").is_none());
    }

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
