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
}
