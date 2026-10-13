//! Result set export: CSV, TSV, JSON, GeoJSON, Markdown and SQL INSERTs.
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
}

impl Format {
    pub const ALL: [Format; 6] =
        [Format::Csv, Format::Tsv, Format::Json, Format::GeoJson, Format::Markdown, Format::SqlInsert];

    pub fn label(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Tsv => "TSV",
            Format::Json => "JSON",
            Format::GeoJson => "GeoJSON",
            Format::Markdown => "Markdown",
            Format::SqlInsert => "SQL INSERT",
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
    let geom = geo::geometry_columns(rs);
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
            truncated: false,
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
    fn tsv_escapes() {
        let mut r = rs();
        r.rows[1][1] = Some("a\tb\nc".into());
        assert!(export(&r, Format::Tsv, "t").contains("2\ta\\tb\\nc\t"));
    }
}
