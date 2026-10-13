//! Result set export: CSV, TSV, JSON, GeoJSON, Markdown, SQL INSERTs and
//! an SVG picture of the table.
//!
//! Values arrive as text, so each column's kind is inferred from its data to
//! emit real numbers/booleans/objects in JSON and unquoted literals in SQL.

use crate::geo;
use crate::model::ResultSet;
use crate::sql::quote_ident;
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Csv,
    Tsv,
    Json,
    GeoJson,
    Markdown,
    SqlInsert,
    Svg,
}

impl Format {
    pub const ALL: [Format; 7] = [
        Format::Csv,
        Format::Tsv,
        Format::Json,
        Format::GeoJson,
        Format::Markdown,
        Format::SqlInsert,
        Format::Svg,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Tsv => "TSV",
            Format::Json => "JSON",
            Format::GeoJson => "GeoJSON",
            Format::Markdown => "Markdown",
            Format::SqlInsert => "SQL INSERT",
            Format::Svg => "SVG table",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Tsv => "tsv",
            Format::Json => "json",
            Format::GeoJson => "geojson",
            Format::Markdown => "md",
            Format::SqlInsert => "sql",
            Format::Svg => "svg",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Number,
    Bool,
    Json,
    Geometry,
    Text,
}

/// Infer each column's kind from all its non-null values.
pub fn infer_kinds(rs: &ResultSet) -> Vec<Kind> {
    let geom = rs.geometry_columns();
    (0..rs.columns.len())
        .map(|i| {
            if geom.contains(&i) {
                return Kind::Geometry;
            }
            let values: Vec<&str> = rs.rows.iter().filter_map(|r| r[i].as_deref()).collect();
            if values.is_empty() {
                Kind::Text
            } else if values.iter().all(|v| is_json_number(v)) {
                Kind::Number
            } else if values.iter().all(|v| *v == "t" || *v == "f") {
                Kind::Bool
            } else if values.iter().all(|v| {
                let t = v.trim_start();
                (t.starts_with('{') || t.starts_with('[')) && serde_json::from_str::<Value>(v).is_ok()
            }) {
                Kind::Json
            } else {
                Kind::Text
            }
        })
        .collect()
}

/// JSON number grammar: rejects `007` (keeps zip codes as text), `NaN`, `Infinity`.
fn is_json_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if b.first() == Some(&b'-') {
        i += 1;
    }
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == b.len()
}

/// `table` names the target in SQL INSERT output.
pub fn export(rs: &ResultSet, format: Format, table: &str) -> String {
    let kinds = infer_kinds(rs);
    match format {
        Format::Csv => delimited(rs, &kinds, ',', true),
        Format::Tsv => delimited(rs, &kinds, '\t', false),
        Format::Json => json(rs, &kinds),
        Format::GeoJson => geojson(rs, &kinds),
        Format::Markdown => markdown(rs, &kinds),
        Format::SqlInsert => sql_insert(rs, &kinds, table),
        Format::Svg => svg(rs, &kinds),
    }
}

/// Text form used by the flat formats: geometries become WKT.
fn display(v: &str, kind: Kind) -> String {
    match kind {
        Kind::Geometry => geo::parse_hex(v).map_or_else(|| v.to_string(), |g| g.geometry.to_wkt()),
        _ => v.to_string(),
    }
}

fn delimited(rs: &ResultSet, kinds: &[Kind], sep: char, bom: bool) -> String {
    let field = |s: &str| -> String {
        if sep == ',' {
            // RFC 4180 quoting.
            if s.contains([',', '"', '\n', '\r']) || s.starts_with(' ') || s.ends_with(' ') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_string()
            }
        } else {
            s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "\\r")
        }
    };
    let sep = sep.to_string();
    // A BOM lets Excel detect UTF-8 (Khmer and other non-Latin text).
    let mut out = if bom { "\u{FEFF}".to_string() } else { String::new() };
    out += &rs.columns.iter().map(|c| field(c)).collect::<Vec<_>>().join(&sep);
    out.push('\n');
    for row in &rs.rows {
        let cells: Vec<String> = row
            .iter()
            .zip(kinds)
            .map(|(v, k)| v.as_deref().map_or(String::new(), |v| field(&display(v, *k))))
            .collect();
        out += &cells.join(&sep);
        out.push('\n');
    }
    out
}

fn json_value(v: Option<&str>, kind: Kind) -> Value {
    let Some(v) = v else { return Value::Null };
    match kind {
        Kind::Number => serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.into())),
        Kind::Bool => Value::Bool(v == "t"),
        Kind::Json => serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.into())),
        Kind::Geometry => geo::parse_hex(v)
            .map(|g| g.to_wgs84().unwrap_or(g.geometry).to_geojson())
            .unwrap_or_else(|| Value::String(v.into())),
        Kind::Text => Value::String(v.into()),
    }
}

fn row_object(rs: &ResultSet, kinds: &[Kind], row: &[Option<String>], skip: Option<usize>) -> Map<String, Value> {
    let mut obj = Map::new();
    for (i, (name, v)) in rs.columns.iter().zip(row).enumerate() {
        if Some(i) != skip {
            obj.insert(name.clone(), json_value(v.as_deref(), kinds[i]));
        }
    }
    obj
}

fn json(rs: &ResultSet, kinds: &[Kind]) -> String {
    let rows: Vec<Value> = rs.rows.iter().map(|r| Value::Object(row_object(rs, kinds, r, None))).collect();
    serde_json::to_string_pretty(&rows).unwrap_or_default()
}

/// FeatureCollection from the first geometry column, converted to WGS84
/// when the SRID is supported (as GeoJSON requires); other columns become
/// properties.
fn geojson(rs: &ResultSet, kinds: &[Kind]) -> String {
    let geom_col = kinds.iter().position(|k| *k == Kind::Geometry);
    let features: Vec<Value> = rs
        .rows
        .iter()
        .map(|row| {
            let geometry = geom_col
                .and_then(|i| row[i].as_deref())
                .and_then(geo::parse_hex)
                .map(|g| g.to_wgs84().unwrap_or(g.geometry).to_geojson())
                .unwrap_or(Value::Null);
            serde_json::json!({
                "type": "Feature",
                "geometry": geometry,
                "properties": row_object(rs, kinds, row, geom_col),
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({"type": "FeatureCollection", "features": features}))
        .unwrap_or_default()
}

fn markdown(rs: &ResultSet, kinds: &[Kind]) -> String {
    let cell = |s: &str| s.replace('|', "\\|").replace('\n', "<br>");
    let mut out = format!("| {} |\n", rs.columns.iter().map(|c| cell(c)).collect::<Vec<_>>().join(" | "));
    out += &format!("|{}\n", rs.columns.iter().zip(kinds).map(|(_, k)| if *k == Kind::Number { " ---: |" } else { " --- |" }).collect::<String>());
    for row in &rs.rows {
        let cells: Vec<String> = row
            .iter()
            .zip(kinds)
            .map(|(v, k)| v.as_deref().map_or("NULL".to_string(), |v| cell(&display(v, *k))))
            .collect();
        out += &format!("| {} |\n", cells.join(" | "));
    }
    out
}

fn sql_literal(v: Option<&str>, kind: Kind) -> String {
    match (v, kind) {
        (None, _) => "NULL".into(),
        (Some(v), Kind::Number) => v.into(),
        (Some(v), Kind::Bool) => if v == "t" { "true" } else { "false" }.into(),
        // Hex EWKB is accepted directly by geometry/geography columns.
        (Some(v), _) => format!("'{}'", v.replace('\'', "''")),
    }
}

fn sql_insert(rs: &ResultSet, kinds: &[Kind], table: &str) -> String {
    let target = table.split('.').map(quote_ident).collect::<Vec<_>>().join(".");
    let cols = rs.columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
    let mut out = String::new();
    for chunk in rs.rows.chunks(500) {
        out += &format!("insert into {target} ({cols}) values\n");
        let values: Vec<String> = chunk
            .iter()
            .map(|row| {
                let lits: Vec<String> = row.iter().zip(kinds).map(|(v, k)| sql_literal(v.as_deref(), *k)).collect();
                format!("  ({})", lits.join(", "))
            })
            .collect();
        out += &values.join(",\n");
        out += ";\n";
    }
    out
}

/// A light, readable picture of the table: bold header, zebra rows,
/// right-aligned numbers, greyed NULLs. Long values are shortened and very
/// long results are cut off with a note, so the image stays legible.
fn svg(rs: &ResultSet, kinds: &[Kind]) -> String {
    const MAX_ROWS: usize = 500;
    const MAX_CHARS: usize = 48;
    const FONT: f32 = 13.0;
    const PAD_X: f32 = 12.0;
    const ROW_H: f32 = 28.0;
    const FONTS: &str = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, 'Noto Sans Khmer', 'Khmer Sangam MN', sans-serif";

    let shorten = |s: &str| -> String {
        let s: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        if s.chars().count() > MAX_CHARS {
            s.chars().take(MAX_CHARS - 1).collect::<String>() + "…"
        } else {
            s
        }
    };
    let shown = &rs.rows[..rs.rows.len().min(MAX_ROWS)];
    let cells: Vec<Vec<Option<String>>> = shown
        .iter()
        .map(|row| row.iter().zip(kinds).map(|(v, k)| v.as_deref().map(|v| shorten(&display(v, *k)))).collect())
        .collect();
    let widths: Vec<f32> = rs
        .columns
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let body = cells.iter().map(|r| r[i].as_deref().map_or(text_width("NULL", FONT), |v| text_width(v, FONT)));
            body.fold(text_width(name, FONT) * 1.08, f32::max) + 2.0 * PAD_X
        })
        .collect();
    let total_w: f32 = widths.iter().sum::<f32>().max(160.0);
    let more = rs.rows.len() - shown.len();
    let footer = if more > 0 { ROW_H } else { 0.0 };
    let total_h = ROW_H * (shown.len() + 1) as f32 + footer;
    let (w, h) = (total_w + 2.0, total_h + 2.0);

    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" font-family=\"{FONTS}\" font-size=\"{FONT}\">\n"
    );
    out += &format!("<rect x=\"1\" y=\"1\" width=\"{total_w}\" height=\"{total_h}\" rx=\"8\" fill=\"#ffffff\" stroke=\"#d0d7de\"/>\n");
    out += &format!("<path d=\"M1 {ROW_H} h{total_w}\" stroke=\"#d0d7de\"/>\n");
    for i in 0..shown.len() {
        if i % 2 == 1 {
            out += &format!("<rect x=\"1\" y=\"{}\" width=\"{total_w}\" height=\"{ROW_H}\" fill=\"#f6f8fa\"/>\n", 1.0 + ROW_H * (i + 1) as f32);
        }
    }
    let baseline = |row: usize| 1.0 + ROW_H * row as f32 + ROW_H / 2.0 + FONT * 0.35;
    let mut x = 1.0;
    for (i, name) in rs.columns.iter().enumerate() {
        let right = kinds[i] == Kind::Number;
        let tx = if right { x + widths[i] - PAD_X } else { x + PAD_X };
        let anchor = if right { " text-anchor=\"end\"" } else { "" };
        out += &format!(
            "<text x=\"{tx}\" y=\"{}\" font-weight=\"600\" fill=\"#1f2328\"{anchor}>{}</text>\n",
            baseline(0),
            xml_escape(name)
        );
        for (r, row) in cells.iter().enumerate() {
            out += &match &row[i] {
                Some(v) => format!("<text x=\"{tx}\" y=\"{}\" fill=\"#1f2328\"{anchor}>{}</text>\n", baseline(r + 1), xml_escape(v)),
                None => format!("<text x=\"{tx}\" y=\"{}\" fill=\"#8c959f\" font-style=\"italic\"{anchor}>NULL</text>\n", baseline(r + 1)),
            };
        }
        x += widths[i];
        if i + 1 < rs.columns.len() {
            out += &format!("<path d=\"M{x} 1 v{}\" stroke=\"#eaeef2\"/>\n", ROW_H * (shown.len() + 1) as f32);
        }
    }
    if more > 0 {
        out += &format!(
            "<text x=\"{}\" y=\"{}\" fill=\"#8c959f\" font-style=\"italic\">… {more} more rows not shown</text>\n",
            1.0 + PAD_X,
            baseline(shown.len() + 1)
        );
    }
    out += "</svg>\n";
    out
}

/// Rough rendered width; there are no font metrics here, so err wide.
fn text_width(s: &str, font: f32) -> f32 {
    s.chars()
        .map(|c| match c as u32 {
            // Khmer dependent vowels and signs stack on the consonant.
            0x17B4..=0x17DD => 0.0,
            0x1780..=0x17B3 => 0.75,
            0x2E80..=0xFFEF => 1.0,
            _ if c.is_ascii_uppercase() || "mwMW@%".contains(c) => 0.72,
            _ if c.is_ascii() => 0.56,
            _ => 0.65,
        })
        .sum::<f32>()
        * font
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Just the given rows (in order).
pub fn subset(rs: &ResultSet, rows: &[usize]) -> ResultSet {
    ResultSet {
        columns: rs.columns.clone(),
        rows: rows.iter().filter_map(|&i| rs.rows.get(i).cloned()).collect(),
        ..Default::default()
    }
}

/// The column that identifies rows: `id`, `objectid`, `gid`, `fid`, else the first.
fn key_column(rs: &ResultSet) -> usize {
    ["id", "objectid", "gid", "fid", "ogc_fid"]
        .iter()
        .find_map(|k| rs.columns.iter().position(|c| c.eq_ignore_ascii_case(k)))
        .unwrap_or(0)
}

/// `select * from <table> where <key> in (…);` for re-fetching these rows.
pub fn where_in(rs: &ResultSet, table: &str) -> String {
    let kinds = infer_kinds(rs);
    let key = key_column(rs);
    let target = table.split('.').map(quote_ident).collect::<Vec<_>>().join(".");
    let values: Vec<String> = rs.rows.iter().map(|r| sql_literal(r[key].as_deref(), kinds[key])).collect();
    format!("select *\nfrom {target}\nwhere {} in ({});\n", quote_ident(&rs.columns[key]), values.join(", "))
}

/// Markdown for pasting into an AI agent: the query, column types and rows
/// (long values shortened, geometries as WKT).
pub fn for_agent(rs: &ResultSet, table: &str, sql: Option<&str>) -> String {
    const MAX_CELL: usize = 300;
    let kinds = infer_kinds(rs);
    let mut out = format!("Data from `{table}` ({} rows)\n\n", rs.rows.len());
    if let Some(sql) = sql.map(str::trim).filter(|s| !s.is_empty()) {
        out += &format!("Query:\n```sql\n{sql}\n```\n\n");
    }
    let types: Vec<String> = rs
        .columns
        .iter()
        .zip(&kinds)
        .map(|(c, k)| {
            let k = match k {
                Kind::Number => "number",
                Kind::Bool => "boolean",
                Kind::Json => "json",
                Kind::Geometry => "geometry (WKT)",
                Kind::Text => "text",
            };
            format!("`{c}` {k}")
        })
        .collect();
    out += &format!("Columns: {}\n\n", types.join(", "));
    let short = ResultSet {
        columns: rs.columns.clone(),
        rows: rs
            .rows
            .iter()
            .map(|r| {
                r.iter()
                    .zip(&kinds)
                    .map(|(v, k)| {
                        v.as_deref().map(|v| {
                            let v = display(v, *k);
                            match v.char_indices().nth(MAX_CELL) {
                                Some((i, _)) => format!("{}…", &v[..i]),
                                None => v,
                            }
                        })
                    })
                    .collect()
            })
            .collect(),
        ..Default::default()
    };
    // Already converted; markdown() must not re-parse geometries.
    out += &markdown(&short, &vec![Kind::Text; kinds.len()]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rs() -> ResultSet {
        let s = |v: &str| Some(v.to_string());
        ResultSet {
            columns: vec!["id".into(), "name".into(), "zip".into(), "ok".into(), "meta".into(), "geom".into()],
            rows: vec![
                vec![
                    s("1"),
                    s("Royal, \"Palace\""),
                    s("012"),
                    s("t"),
                    s(r#"{"floors": 2}"#),
                    s("0101000020E61000008F537424973B5A40AA8251499D202740"),
                ],
                vec![s("2"), s("ផ្សារធំថ្មី"), None, s("f"), None, None],
            ],
            ..Default::default()
        }
    }

    #[test]
    fn svg_table() {
        let out = export(&rs(), Format::Svg, "t");
        assert!(out.starts_with("<svg xmlns="), "{out}");
        assert!(out.ends_with("</svg>\n"));
        // Escaped, numbers right-aligned, NULLs greyed, geometry as WKT.
        assert!(out.contains("Royal, &quot;Palace&quot;"));
        assert!(out.contains("text-anchor=\"end\">1</text>"));
        assert!(out.contains(">NULL</text>"));
        assert!(out.contains("POINT"));
        assert!(out.contains("ផ្សារធំថ្មី"));
        if let Ok(path) = std::env::var("RGRIP_SVG_SAMPLE") {
            std::fs::write(path, &out).unwrap();
        }
    }

    #[test]
    fn infers_kinds() {
        assert_eq!(
            infer_kinds(&rs()),
            [Kind::Number, Kind::Text, Kind::Text, Kind::Bool, Kind::Json, Kind::Geometry]
        );
        for (s, ok) in [("0", true), ("-1.5e3", true), ("007", false), ("1.", false), ("NaN", false), ("", false)] {
            assert_eq!(is_json_number(s), ok, "{s}");
        }
    }

    #[test]
    fn csv() {
        let out = export(&rs(), Format::Csv, "t");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "\u{FEFF}id,name,zip,ok,meta,geom");
        assert_eq!(lines[1], r#"1,"Royal, ""Palace""",012,t,"{""floors"": 2}",POINT (104.9311 11.5637)"#);
        assert_eq!(lines[2], "2,ផ្សារធំថ្មី,,f,,");
    }

    #[test]
    fn json_types() {
        let v: Value = serde_json::from_str(&export(&rs(), Format::Json, "t")).unwrap();
        assert_eq!(v[0]["id"], 1);
        assert_eq!(v[0]["zip"], "012");
        assert_eq!(v[0]["ok"], true);
        assert_eq!(v[0]["meta"]["floors"], 2);
        assert_eq!(v[0]["geom"]["type"], "Point");
        assert_eq!(v[1]["name"], "ផ្សារធំថ្មី");
        assert_eq!(v[1]["meta"], Value::Null);
    }

    #[test]
    fn geojson() {
        let v: Value = serde_json::from_str(&export(&rs(), Format::GeoJson, "t")).unwrap();
        assert_eq!(v["type"], "FeatureCollection");
        let f = &v["features"][0];
        assert_eq!(f["geometry"]["coordinates"][0], 104.9311);
        assert_eq!(f["properties"]["name"], "Royal, \"Palace\"");
        assert!(f["properties"].get("geom").is_none());
        assert_eq!(v["features"][1]["geometry"], Value::Null);
    }

    #[test]
    fn sql_and_markdown() {
        let sql = export(&rs(), Format::SqlInsert, "public.places");
        assert!(sql.starts_with("insert into \"public\".\"places\" (\"id\", \"name\", \"zip\", \"ok\", \"meta\", \"geom\") values\n"));
        assert!(sql.contains("(1, 'Royal, \"Palace\"', '012', true, '{\"floors\": 2}', '0101000020E6"));
        assert!(sql.contains("(2, 'ផ្សារធំថ្មី', NULL, false, NULL, NULL);"));
        let md = export(&rs(), Format::Markdown, "t");
        assert!(md.starts_with("| id | name | zip | ok | meta | geom |\n| ---: | --- |"));
        assert!(md.contains("| 2 | ផ្សារធំថ្មី | NULL |"));
    }

    #[test]
    fn subsets_where_in_and_agent() {
        let r = subset(&rs(), &[1]);
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][1].as_deref(), Some("ផ្សារធំថ្មី"));
        assert_eq!(where_in(&subset(&rs(), &[0, 1]), "public.places"), "select *\nfrom \"public\".\"places\"\nwhere \"id\" in (1, 2);\n");

        let md = for_agent(&rs(), "places", Some("select * from places"));
        assert!(md.starts_with("Data from `places` (2 rows)\n\nQuery:\n```sql\nselect * from places\n```"));
        assert!(md.contains("`id` number, `name` text, `zip` text, `ok` boolean, `meta` json, `geom` geometry (WKT)"));
        assert!(md.contains("POINT (104.9311 11.5637)"));
        assert!(md.contains("| 2 | ផ្សារធំថ្មី | NULL |"));
    }

    #[test]
    fn tsv_escapes() {
        let mut r = rs();
        r.rows[1][1] = Some("a\tb\nc".into());
        assert!(export(&r, Format::Tsv, "t").contains("2\ta\\tb\\nc\t"));
    }
}
