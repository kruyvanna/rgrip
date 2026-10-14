//! Slash commands typed in the editor (`/con_4326 parcels`) that expand
//! into SQL. Arguments are a table and/or a geometry field in any order;
//! tables are recognised from the catalog, and the field defaults to the
//! table's geometry column or `shape` (the ArcGIS default).

use crate::complete::{Catalog, CatalogTable, needs_quotes};
use crate::sql::quote_ident;

pub struct Command {
    pub name: &'static str,
    pub args: &'static str,
    pub help: &'static str,
}

pub const COMMANDS: &[Command] = &[
    Command { name: "con_4326", args: "[table] [field]", help: "→ 4326 with ST_Transform" },
    Command {
        name: "con_4326_2",
        args: "[table] [field]",
        help: "→ 4326 via text (ArcGIS): ST_SetSRID(st_astext(…))",
    },
    Command {
        name: "con_merc",
        args: "[table] [field] [srid]",
        help: "Projected (default 32648, UTM 48N) via text → 4326",
    },
    Command { name: "srid", args: "<table> [field]", help: "SRIDs used by a table's geometries" },
    Command { name: "top", args: "<table> [n]", help: "First rows: select * … limit n (100)" },
    Command { name: "count", args: "<table>", help: "Row count" },
];

const DEFAULT_FIELD: &str = "shape";
/// Source SRID for `/con_merc` when none is given.
const DEFAULT_MERC_SRID: u64 = 32648;
/// Placeholder when a reprojection command has no table yet.
const TABLE_PLACEHOLDER: &str = "table_name";

fn ident(s: &str) -> String {
    if needs_quotes(s) { quote_ident(s) } else { s.to_string() }
}

/// `schema.table` or `table` as typed, if it names a catalog table.
fn find_table<'a>(catalog: &'a Catalog, arg: &str, schema: &str) -> Option<&'a CatalogTable> {
    let arg = arg.trim_matches('"');
    match arg.split_once('.') {
        Some((schema, name)) => catalog
            .tables
            .iter()
            .find(|t| t.schema.eq_ignore_ascii_case(schema) && t.name.eq_ignore_ascii_case(name)),
        None => {
            let matches: Vec<_> = catalog.tables.iter().filter(|t| t.name.eq_ignore_ascii_case(arg)).collect();
            // Prefer an exact-case match, then the console's schema, then `public`.
            let exact = matches.iter().filter(|t| t.name == arg);
            exact.clone().find(|t| t.schema == schema).or(exact.clone().find(|t| t.schema == "public")).or(exact.clone().next())
                .or(matches.iter().find(|t| t.schema == schema)).or(matches.iter().find(|t| t.schema == "public")).or(matches.first()).copied()
        }
    }
}

/// The table's geometry column: PostGIS geometry/geography, ArcGIS
/// st_geometry, or a column with a conventional name.
fn geometry_field(t: &CatalogTable) -> Option<&str> {
    let typed = t.columns.iter().find(|(_, ty)| {
        crate::geo::column_srid(ty).is_some() || ty.to_ascii_lowercase().ends_with("st_geometry")
    });
    let named = || {
        ["shape", "geom", "the_geom", "wkb_geometry", "geometry"]
            .iter()
            .find_map(|n| t.columns.iter().find(|(c, _)| c.eq_ignore_ascii_case(n)))
    };
    typed.or_else(named).map(|(c, _)| c.as_str())
}

/// SQL for a table reference as typed (keeps schema qualification).
fn table_sql(t: &CatalogTable, typed: &str, schema: &str) -> String {
    if typed.contains('.') || (t.schema != "public" && t.schema != schema) {
        format!("{}.{}", ident(&t.schema), ident(&t.name))
    } else {
        ident(&t.name)
    }
}

/// Expand a command line such as `/con_4326 parcels geom`.
pub fn expand(line: &str, catalog: &Catalog, schema: &str) -> Result<String, String> {
    let line = line.trim();
    let body = line.strip_prefix('/').ok_or("Commands start with /")?;
    let words = split_args(body);
    let mut words = words.iter().map(String::as_str);
    let name = words.next().unwrap_or_default().to_ascii_lowercase();
    let command = COMMANDS.iter().find(|c| c.name == name).ok_or_else(|| format!("Unknown command /{name}"))?;

    let mut table: Option<(&CatalogTable, &str)> = None;
    let mut other: Vec<&str> = Vec::new();
    let mut number: Option<u64> = None;
    for w in words {
        if let Ok(n) = w.parse::<u64>() {
            number = Some(n);
        } else if table.is_none() && let Some(t) = find_table(catalog, w, schema) {
            table = Some((t, w));
        } else {
            other.push(w);
        }
    }
    let usage = || format!("Usage: /{} {}", command.name, command.args);
    // `/cmd <table> <field>` with a table the catalog doesn't know (not
    // loaded yet, or a typo Postgres will report): use it as typed.
    let (from, field) = match (table, other.as_slice()) {
        (Some((t, typed)), [] | [_]) => (Some(table_sql(t, typed, schema)), other.first().copied()),
        // A lone unknown word is the table for table-only commands, the
        // field otherwise.
        (None, [t]) if matches!(command.name, "top" | "count" | "srid") => (Some(t.to_string()), None),
        (None, [f]) => (None, Some(*f)),
        (None, []) => (None, None),
        (None, [t, f]) => (Some(t.to_string()), Some(*f)),
        (_, [.., w]) => return Err(format!("Unexpected argument {w}. {}", usage())),
    };
    let field = field.map(|f| f.trim_matches('"').to_string()).unwrap_or_else(|| {
        table.and_then(|(t, _)| geometry_field(t)).unwrap_or(DEFAULT_FIELD).to_string()
    });

    let reproject = |f: &str| -> Option<String> {
        Some(match command.name {
            "con_4326" => format!("ST_Transform({f}, 4326)"),
            "con_4326_2" => format!("ST_SetSRID(st_astext({f})::text, 4326)"),
            "con_merc" => format!(
                "ST_Transform(ST_SetSRID(st_astext({f})::text::geometry, {}), 4326)",
                number.unwrap_or(DEFAULT_MERC_SRID)
            ),
            _ => return None,
        })
    };

    // Reprojections always expand to a complete query.
    let from = from.or_else(|| reproject("").map(|_| TABLE_PLACEHOLDER.to_string()));
    match (command.name, from) {
        (_, Some(from)) if reproject("").is_some() => {
            let expr = reproject(&format!("t.{}", ident(&field))).unwrap();
            Ok(format!("select t.*, {expr} as {}\nfrom {from} t;", ident(&format!("{field}_4326"))))
        }
        ("srid", Some(from)) => Ok(format!(
            "select ST_SRID({f}) as srid, count(*)\nfrom {from}\ngroup by 1\norder by 2 desc;",
            f = ident(&field)
        )),
        ("top", Some(from)) => Ok(format!("select *\nfrom {from}\nlimit {};", number.unwrap_or(100))),
        ("count", Some(from)) => Ok(format!("select count(*)\nfrom {from};")),
        _ => Err(usage()),
    }
}

/// Split on whitespace, keeping `"quoted names"` (quotes included) together.
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The slash command on the cursor's line, as (byte range of the line, text).
pub fn command_line_at(sql: &str, cursor: usize) -> Option<(std::ops::Range<usize>, &str)> {
    let cursor = cursor.min(sql.len());
    let start = sql[..cursor].rfind('\n').map_or(0, |i| i + 1);
    let end = sql[cursor..].find('\n').map_or(sql.len(), |i| cursor + i);
    let line = &sql[start..end];
    is_command_line(line).then_some((start..end, line))
}

/// `/word…` at the start of the line (not a `/* comment`).
pub fn is_command_line(line: &str) -> bool {
    line.trim_start().strip_prefix('/').is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_alphabetic()))
}

/// Expand every command line in `sql` (for Run all / selection), keeping
/// the other lines as they are.
pub fn expand_all(sql: &str, catalog: &Catalog, schema: &str) -> Result<String, String> {
    let mut out = Vec::new();
    for line in sql.split('\n') {
        if is_command_line(line) {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            let expanded = expand(line, catalog, schema)?;
            out.push(format!("{indent}{expanded}"));
        } else {
            out.push(line.to_string());
        }
    }
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TableKind;

    fn catalog() -> Catalog {
        let t = |schema: &str, name: &str, cols: &[(&str, &str)]| CatalogTable {
            schema: schema.into(),
            name: name.into(),
            kind: TableKind::Table,
            columns: cols.iter().map(|(c, ty)| (c.to_string(), ty.to_string())).collect(),
        };
        Catalog {
            tables: vec![
                t("public", "parcels", &[("objectid", "integer"), ("shape", "sde.st_geometry")]),
                t("public", "roads", &[("id", "integer"), ("geom", "geometry(LineString,32648)")]),
                t("gis", "zones", &[("id", "integer"), ("the_geom", "geometry")]),
                t("public", "notes", &[("id", "integer"), ("body", "text")]),
            ],
        }
    }

    fn x(line: &str) -> String {
        expand(line, &catalog(), "public").unwrap()
    }

    #[test]
    fn console_schema_is_preferred() {
        let mut c = catalog();
        c.tables.push(CatalogTable { schema: "gis".into(), name: "roads".into(), kind: TableKind::Table, columns: vec![("gid".into(), "integer".into())] });
        // Unqualified `roads` is public's by default, gis's in a gis console.
        assert_eq!(expand("/top roads", &c, "public").unwrap(), "select *\nfrom roads\nlimit 100;");
        assert_eq!(expand("/top roads", &c, "gis").unwrap(), "select *\nfrom roads\nlimit 100;");
        assert_eq!(find_table(&c, "roads", "gis").unwrap().schema, "gis");
        assert_eq!(find_table(&c, "roads", "public").unwrap().schema, "public");
        // Tables only in the console schema stay unqualified.
        assert!(expand("/top zones", &c, "gis").unwrap().contains("from zones\n"));
        assert!(expand("/top zones", &c, "public").unwrap().contains("from gis.zones\n"));
    }

    #[test]
    fn without_table_uses_a_placeholder() {
        assert_eq!(x("/con_4326"), "select t.*, ST_Transform(t.shape, 4326) as shape_4326\nfrom table_name t;");
        assert!(x("/con_4326 geom").starts_with("select t.*, ST_Transform(t.geom, 4326) as geom_4326"));
        assert!(x("/con_4326_2").contains("ST_SetSRID(st_astext(t.shape)::text, 4326)"));
        assert!(x("/con_4326 \"Shape Field\"").contains("ST_Transform(t.\"Shape Field\", 4326) as \"Shape Field_4326\""));
    }

    #[test]
    fn merc_srid_defaults_to_32648() {
        assert_eq!(
            x("/con_merc"),
            "select t.*, ST_Transform(ST_SetSRID(st_astext(t.shape)::text::geometry, 32648), 4326) as shape_4326\nfrom table_name t;"
        );
        assert!(x("/con_merc parcels 3857").contains("::geometry, 3857), 4326)"));
    }

    #[test]
    fn with_table_writes_the_query() {
        assert_eq!(x("/con_4326_2 parcels"), "select t.*, ST_SetSRID(st_astext(t.shape)::text, 4326) as shape_4326\nfrom parcels t;");
        // The table's geometry column is used, and args can come in any order.
        assert_eq!(x("/con_4326 roads"), "select t.*, ST_Transform(t.geom, 4326) as geom_4326\nfrom roads t;");
        assert_eq!(x("/con_4326 geom roads"), x("/con_4326 roads geom"));
        assert_eq!(x("/con_merc gis.zones"), "select t.*, ST_Transform(ST_SetSRID(st_astext(t.the_geom)::text::geometry, 32648), 4326) as the_geom_4326\nfrom gis.zones t;");
        assert!(x("/con_4326 zones").contains("from gis.zones t"));
    }

    #[test]
    fn helpers_and_errors() {
        assert_eq!(x("/top roads"), "select *\nfrom roads\nlimit 100;");
        assert_eq!(x("/top roads 20"), "select *\nfrom roads\nlimit 20;");
        assert_eq!(x("/count notes"), "select count(*)\nfrom notes;");
        assert_eq!(x("/srid parcels"), "select ST_SRID(shape) as srid, count(*)\nfrom parcels\ngroup by 1\norder by 2 desc;");
        assert!(expand("/top", &catalog(), "public").unwrap_err().contains("Usage: /top <table>"));
        assert!(expand("/nope", &catalog(), "public").unwrap_err().contains("Unknown command"));
        assert!(expand("/con_4326 a b c", &catalog(), "public").unwrap_err().contains("Unexpected argument"));
    }

    #[test]
    fn table_and_field_args() {
        assert_eq!(
            x("/con_4326_2 PARCELS objectid"),
            "select t.*, ST_SetSRID(st_astext(t.objectid)::text, 4326) as objectid_4326\nfrom parcels t;"
        );
        // Unknown table with an explicit field: used as typed.
        assert_eq!(
            x("/con_4326_2 parcel shape"),
            "select t.*, ST_SetSRID(st_astext(t.shape)::text, 4326) as shape_4326\nfrom parcel t;"
        );
        assert_eq!(x("/count gis.ZONES"), "select count(*)\nfrom gis.zones;");
        // Table-only commands take an unknown word as the table.
        assert_eq!(x("/count parcel"), "select count(*)\nfrom parcel;");
        assert_eq!(x("/top parcel 5"), "select *\nfrom parcel\nlimit 5;");
    }

    #[test]
    fn expands_every_command_line() {
        let sql = "/con_4326_2 parcels\n/* note */\nselect 1;\n  /count notes";
        assert_eq!(
            expand_all(sql, &catalog(), "public").unwrap(),
            "select t.*, ST_SetSRID(st_astext(t.shape)::text, 4326) as shape_4326\nfrom parcels t;\n/* note */\nselect 1;\n  select count(*)\nfrom notes;"
        );
        assert_eq!(expand_all("select 4 / 2;", &catalog(), "public").unwrap(), "select 4 / 2;");
        assert!(expand_all("/top", &catalog(), "public").unwrap_err().contains("Usage"));
        assert!(!is_command_line("/* comment */"));
    }

    #[test]
    fn finds_the_command_line() {
        let sql = "select 1;\n  /con_4326 roads\nselect 2;";
        let cursor = sql.find("roads").unwrap();
        let (range, line) = command_line_at(sql, cursor).unwrap();
        assert_eq!(line, "  /con_4326 roads");
        assert_eq!(&sql[range], "  /con_4326 roads");
        assert!(command_line_at(sql, 3).is_none());
    }
}
