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
        args: "[table] [field]",
        help: "Web Mercator (3857/102100) via text → 4326",
    },
    Command { name: "srid", args: "<table> [field]", help: "SRIDs used by a table's geometries" },
    Command { name: "top", args: "<table> [n]", help: "First rows: select * … limit n (100)" },
    Command { name: "count", args: "<table>", help: "Row count" },
];

const DEFAULT_FIELD: &str = "shape";

fn ident(s: &str) -> String {
    if needs_quotes(s) { quote_ident(s) } else { s.to_string() }
}

/// `schema.table` or `table` as typed, if it names a catalog table.
fn find_table<'a>(catalog: &'a Catalog, arg: &str) -> Option<&'a CatalogTable> {
    let arg = arg.trim_matches('"');
    match arg.split_once('.') {
        Some((schema, name)) => catalog.tables.iter().find(|t| t.schema == schema && t.name == name),
        None => {
            let matches: Vec<_> = catalog.tables.iter().filter(|t| t.name == arg).collect();
            matches.iter().find(|t| t.schema == "public").or(matches.first()).copied()
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
fn table_sql(t: &CatalogTable, typed: &str) -> String {
    if typed.contains('.') || t.schema != "public" {
        format!("{}.{}", ident(&t.schema), ident(&t.name))
    } else {
        ident(&t.name)
    }
}

/// Expand a command line such as `/con_4326 parcels geom`.
pub fn expand(line: &str, catalog: &Catalog) -> Result<String, String> {
    let line = line.trim();
    let body = line.strip_prefix('/').ok_or("Commands start with /")?;
    let words = split_args(body);
    let mut words = words.iter().map(String::as_str);
    let name = words.next().unwrap_or_default().to_ascii_lowercase();
    let command = COMMANDS.iter().find(|c| c.name == name).ok_or_else(|| format!("Unknown command /{name}"))?;

    let mut table: Option<(&CatalogTable, &str)> = None;
    let mut field: Option<&str> = None;
    let mut number: Option<u64> = None;
    for w in words {
        if let Ok(n) = w.parse::<u64>() {
            number = Some(n);
        } else if table.is_none() && let Some(t) = find_table(catalog, w) {
            table = Some((t, w));
        } else if field.is_none() {
            field = Some(w);
        } else {
            return Err(format!("Unexpected argument {w}. Usage: /{} {}", command.name, command.args));
        }
    }
    let usage = || format!("Usage: /{} {}", command.name, command.args);
    let field = field.map(|f| f.trim_matches('"').to_string()).unwrap_or_else(|| {
        table.and_then(|(t, _)| geometry_field(t)).unwrap_or(DEFAULT_FIELD).to_string()
    });

    let reproject = |f: &str| -> Option<String> {
        Some(match command.name {
            "con_4326" => format!("ST_Transform({f}, 4326)"),
            "con_4326_2" => format!("ST_SetSRID(st_astext({f})::text, 4326)"),
            "con_merc" => format!("ST_Transform(ST_SetSRID(st_astext({f})::text::geometry, 3857), 4326)"),
            _ => return None,
        })
    };

    match (command.name, table) {
        (_, None) if reproject("").is_some() => Ok(reproject(&ident(&field)).unwrap()),
        (_, Some((t, typed))) if reproject("").is_some() => {
            let expr = reproject(&format!("t.{}", ident(&field))).unwrap();
            Ok(format!(
                "select t.*, {expr} as {}\nfrom {} t;",
                ident(&format!("{field}_4326")),
                table_sql(t, typed)
            ))
        }
        ("srid", Some((t, typed))) => Ok(format!(
            "select ST_SRID({f}) as srid, count(*)\nfrom {}\ngroup by 1\norder by 2 desc;",
            table_sql(t, typed),
            f = ident(&field)
        )),
        ("top", Some((t, typed))) => Ok(format!("select *\nfrom {}\nlimit {};", table_sql(t, typed), number.unwrap_or(100))),
        ("count", Some((t, typed))) => Ok(format!("select count(*)\nfrom {};", table_sql(t, typed))),
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
    line.trim_start().starts_with('/').then_some((start..end, line))
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
        expand(line, &catalog()).unwrap()
    }

    #[test]
    fn expressions_default_to_shape() {
        assert_eq!(x("/con_4326"), "ST_Transform(shape, 4326)");
        assert_eq!(x("/con_4326 geom"), "ST_Transform(geom, 4326)");
        assert_eq!(x("/con_4326_2"), "ST_SetSRID(st_astext(shape)::text, 4326)");
        assert_eq!(x("/con_merc"), "ST_Transform(ST_SetSRID(st_astext(shape)::text::geometry, 3857), 4326)");
        assert_eq!(x("/con_4326 \"Shape Field\""), "ST_Transform(\"Shape Field\", 4326)");
    }

    #[test]
    fn with_table_writes_the_query() {
        assert_eq!(x("/con_4326_2 parcels"), "select t.*, ST_SetSRID(st_astext(t.shape)::text, 4326) as shape_4326\nfrom parcels t;");
        // The table's geometry column is used, and args can come in any order.
        assert_eq!(x("/con_4326 roads"), "select t.*, ST_Transform(t.geom, 4326) as geom_4326\nfrom roads t;");
        assert_eq!(x("/con_4326 geom roads"), x("/con_4326 roads geom"));
        assert_eq!(x("/con_merc gis.zones"), "select t.*, ST_Transform(ST_SetSRID(st_astext(t.the_geom)::text::geometry, 3857), 4326) as the_geom_4326\nfrom gis.zones t;");
        assert!(x("/con_4326 zones").contains("from gis.zones t"));
    }

    #[test]
    fn helpers_and_errors() {
        assert_eq!(x("/top roads"), "select *\nfrom roads\nlimit 100;");
        assert_eq!(x("/top roads 20"), "select *\nfrom roads\nlimit 20;");
        assert_eq!(x("/count notes"), "select count(*)\nfrom notes;");
        assert_eq!(x("/srid parcels"), "select ST_SRID(shape) as srid, count(*)\nfrom parcels\ngroup by 1\norder by 2 desc;");
        assert!(expand("/top", &catalog()).unwrap_err().contains("Usage: /top <table>"));
        assert!(expand("/nope", &catalog()).unwrap_err().contains("Unknown command"));
        assert!(expand("/con_4326 a b c", &catalog()).unwrap_err().contains("Unexpected argument"));
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
