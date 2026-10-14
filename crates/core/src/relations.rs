//! Relations guessed from naming conventions, for databases without foreign
//! key constraints (ArcGIS, legacy schemas). Conservative on purpose: a missed
//! relation costs nothing, a wrong one sends people to the wrong rows.
//!
//! Rules:
//! - Outgoing: a non-PK column `X_id` points to the table named `X` (or `Xs`,
//!   `Xes`, `Xies` for `Xy`) through its `id` column. Same schema wins; a
//!   candidate in another schema is used only if it is the single match.
//!   Ambiguous matches are dropped.
//! - Incoming: another table's column `<this>_id` or `<singular(this)>_id`
//!   references this table's `id` (only if it has one).
//! - Incoming by key name: when this table's primary key is one column with a
//!   distinctive name (`parcel_code`, not `id`/`gid`/`objectid`...), other
//!   tables with a column of exactly that name reference it. Outgoing by key
//!   name is not guessed: the catalog doesn't know other tables' primary keys.
//! - Both sides must have compatible types (integers and numeric together,
//!   text types together, uuid with uuid, otherwise identical).
//! - System schemas (and PostGIS helpers `tiger`, `tiger_data`, `topology`)
//!   are skipped, as are views as the other side.
//! - Anything already covered by a real foreign key is skipped.

use crate::complete::{Catalog, CatalogTable};
use crate::model::{ColumnInfo, Relation, TableKind};

/// More than this and the menu stops being useful.
const MAX: usize = 25;

/// Key names shared by whole families of tables; matching them by name
/// alone would relate unrelated tables.
const GENERIC_KEYS: &[&str] = &["id", "gid", "fid", "oid", "objectid", "ogc_fid", "rowid", "row_id", "uuid", "pk", "key", "globalid", "guid"];

fn skipped_schema(s: &str) -> bool {
    matches!(s, "pg_catalog" | "information_schema" | "pg_toast" | "tiger" | "tiger_data" | "topology") || s.starts_with("pg_temp") || s.starts_with("pg_toast_temp")
}

#[derive(PartialEq, Eq)]
enum Family {
    Int,
    Numeric,
    Text,
    Uuid,
    Other(String),
}

fn family(ty: &str) -> Family {
    let t = ty.trim().to_lowercase();
    let base = t.split('(').next().unwrap_or("").trim();
    match base {
        "smallint" | "integer" | "bigint" | "int2" | "int4" | "int8" | "int" => Family::Int,
        "numeric" | "decimal" => Family::Numeric,
        "text" | "character varying" | "varchar" | "character" | "char" | "bpchar" | "citext" => Family::Text,
        "uuid" => Family::Uuid,
        _ => Family::Other(t),
    }
}

fn compatible(a: &str, b: &str) -> bool {
    let (a, b) = (family(a), family(b));
    a == b || matches!((&a, &b), (Family::Int, Family::Numeric) | (Family::Numeric, Family::Int))
}

/// `users` → `user`, `categories` → `category`, `boxes` → `box`.
fn singular(name: &str) -> String {
    if let Some(s) = name.strip_suffix("ies").filter(|s| !s.is_empty()) {
        format!("{s}y")
    } else if let Some(s) = ["ses", "xes", "ches", "shes"].iter().find_map(|suf| name.strip_suffix(suf).map(|s| format!("{s}{}", &suf[..suf.len() - 2]))) {
        s
    } else if let Some(s) = name.strip_suffix('s').filter(|s| !s.is_empty() && !s.ends_with('s')) {
        s.to_string()
    } else {
        name.to_string()
    }
}

/// Table names that `X_id` may refer to.
fn plurals(stem: &str) -> Vec<String> {
    let mut v = vec![stem.to_string(), format!("{stem}s"), format!("{stem}es")];
    if let Some(s) = stem.strip_suffix('y') {
        v.push(format!("{s}ies"));
    }
    v
}

fn column<'a>(t: &'a CatalogTable, name: &str) -> Option<&'a (String, String)> {
    t.columns.iter().find(|(c, _)| c.eq_ignore_ascii_case(name))
}

fn covered(existing: &[Relation], r: &Relation) -> bool {
    existing.iter().any(|e| e.outgoing == r.outgoing && e.other_schema == r.other_schema && e.other_table == r.other_table && e.columns == r.columns)
}

/// Guess relations of `schema.table` that no foreign key declares.
/// `existing` are the real foreign keys of the table.
pub fn infer(schema: &str, table: &str, columns: &[ColumnInfo], catalog: &Catalog, existing: &[Relation]) -> Vec<Relation> {
    if skipped_schema(schema) {
        return Vec::new();
    }
    let candidates: Vec<&CatalogTable> = catalog
        .tables
        .iter()
        .filter(|t| !skipped_schema(&t.schema) && matches!(t.kind, TableKind::Table | TableKind::Partitioned))
        .collect();
    let mut out: Vec<Relation> = Vec::new();
    let mut add = |outgoing: bool, other: &CatalogTable, here: &str, there: &str| {
        let r = Relation {
            name: if outgoing { format!("inferred: {table}.{here} → {}.{there}", other.name) } else { format!("inferred: {}.{there} → {table}.{here}", other.name) },
            outgoing,
            other_schema: other.schema.clone(),
            other_table: other.name.clone(),
            columns: vec![(here.to_string(), there.to_string())],
            inferred: true,
        };
        if !covered(existing, &r) && !out.iter().any(|o| (o.outgoing, &o.other_schema, &o.other_table, &o.columns) == (r.outgoing, &r.other_schema, &r.other_table, &r.columns)) {
            out.push(r);
        }
    };

    // Outgoing: `X_id` → X.id
    for col in columns.iter().filter(|c| !c.primary_key) {
        let lower = col.name.to_lowercase();
        let Some(stem) = lower.strip_suffix("_id").filter(|s| !s.is_empty()) else { continue };
        let names = plurals(stem);
        let matches: Vec<(&CatalogTable, &str)> = candidates
            .iter()
            .filter(|t| names.iter().any(|n| t.name.eq_ignore_ascii_case(n)))
            .filter_map(|t| column(t, "id").filter(|(_, ty)| compatible(&col.data_type, ty)).map(|(id, _)| (*t, id.as_str())))
            .collect();
        let same: Vec<_> = matches.iter().filter(|(t, _)| t.schema == schema).collect();
        let pick = if same.len() == 1 { Some(*same[0]) } else if same.is_empty() && matches.len() == 1 { Some(matches[0]) } else { None };
        if let Some((t, id)) = pick {
            add(true, t, &col.name, id);
        }
    }

    // Incoming: other.<this>_id → this.id
    if let Some(id) = columns.iter().find(|c| c.name.eq_ignore_ascii_case("id")) {
        let lower = table.to_lowercase();
        let wanted = [format!("{}_id", singular(&lower)), format!("{lower}_id")];
        for t in &candidates {
            for (c, ty) in &t.columns {
                if wanted.iter().any(|w| c.eq_ignore_ascii_case(w)) && compatible(&id.data_type, ty) {
                    add(false, t, &id.name, c);
                }
            }
        }
    }

    // Incoming: a distinctively named single-column primary key
    let keys: Vec<&ColumnInfo> = columns.iter().filter(|c| c.primary_key).collect();
    if let [key] = keys[..]
        && key.name.len() >= 3
        && !GENERIC_KEYS.contains(&key.name.to_lowercase().as_str())
    {
        for t in &candidates {
            if t.schema == schema && t.name == table {
                continue;
            }
            if let Some((c, ty)) = column(t, &key.name)
                && compatible(&key.data_type, ty)
            {
                add(false, t, &key.name, c);
            }
        }
    }

    let elsewhere = |r: &Relation| r.other_schema != schema;
    out.sort_by(|a, b| (!a.outgoing, elsewhere(a), &a.other_table, &a.other_schema, &a.columns).cmp(&(!b.outgoing, elsewhere(b), &b.other_table, &b.other_schema, &b.columns)));
    out.truncate(MAX);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str, pk: bool) -> ColumnInfo {
        ColumnInfo { name: name.into(), data_type: ty.into(), nullable: !pk, default: None, primary_key: pk }
    }

    fn table(schema: &str, name: &str, cols: &[(&str, &str)]) -> CatalogTable {
        CatalogTable {
            schema: schema.into(),
            name: name.into(),
            kind: TableKind::Table,
            columns: cols.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        }
    }

    fn catalog() -> Catalog {
        Catalog {
            tables: vec![
                table("public", "users", &[("id", "integer"), ("name", "text")]),
                table("public", "posts", &[("id", "integer"), ("user_id", "integer"), ("category_id", "bigint")]),
                table("public", "categories", &[("id", "bigint")]),
                table("public", "parcels", &[("parcel_code", "character varying(20)")]),
                table("public", "owners", &[("owner_id", "integer"), ("parcel_code", "text")]),
                table("pg_catalog", "pg_user", &[("id", "integer"), ("user_id", "integer")]),
            ],
        }
    }

    fn summary(r: &[Relation]) -> Vec<String> {
        r.iter().map(|r| format!("{}{}{}", if r.outgoing { "→" } else { "←" }, r.other_table, r.columns.iter().map(|(a, b)| format!(" {a}={b}")).collect::<String>())).collect()
    }

    #[test]
    fn outgoing_and_plural() {
        let cols = [col("id", "integer", true), col("user_id", "integer", false), col("category_id", "bigint", false)];
        let r = infer("public", "posts", &cols, &catalog(), &[]);
        assert_eq!(summary(&r), ["→categories category_id=id", "→users user_id=id"]);
        assert!(r.iter().all(|r| r.inferred));
        assert_eq!(r[1].name, "inferred: posts.user_id → users.id");
    }

    #[test]
    fn incoming() {
        let cols = [col("id", "integer", true), col("name", "text", false)];
        let r = infer("public", "users", &cols, &catalog(), &[]);
        assert_eq!(summary(&r), ["←posts id=user_id"]);
    }

    #[test]
    fn same_name_primary_key() {
        let cols = [col("parcel_code", "character varying(20)", true)];
        let r = infer("public", "parcels", &cols, &catalog(), &[]);
        assert_eq!(summary(&r), ["←owners parcel_code=parcel_code"]);
        // generic key names never match by name
        let cols = [col("objectid", "integer", true)];
        let mut cat = catalog();
        cat.tables.push(table("public", "a", &[("objectid", "integer")]));
        assert!(infer("public", "b", &cols, &cat, &[]).is_empty());
    }

    #[test]
    fn type_mismatch_rejected() {
        let cols = [col("id", "integer", true), col("user_id", "text", false)];
        assert!(infer("public", "posts", &cols, &catalog(), &[]).is_empty());
    }

    #[test]
    fn existing_fk_not_duplicated() {
        let cols = [col("id", "integer", true), col("user_id", "integer", false)];
        let fk = Relation {
            name: "posts_user_fk".into(),
            outgoing: true,
            other_schema: "public".into(),
            other_table: "users".into(),
            columns: vec![("user_id".into(), "id".into())],
            inferred: false,
        };
        assert!(infer("public", "posts", &cols, &catalog(), &[fk]).is_empty());
    }

    #[test]
    fn system_schemas_skipped() {
        let cols = [col("id", "integer", true), col("user_id", "integer", false)];
        let cat = Catalog { tables: vec![table("information_schema", "users", &[("id", "integer")])] };
        assert!(infer("public", "posts", &cols, &cat, &[]).is_empty());
        // pg_catalog.pg_user has user_id, but must not count as referencing public.user
        let cols = [col("id", "integer", true)];
        let cat = Catalog { tables: vec![table("pg_catalog", "pg_user", &[("user_id", "integer")])] };
        assert!(infer("public", "user", &cols, &cat, &[]).is_empty());
    }

    #[test]
    fn prefers_same_schema() {
        let cols = [col("id", "integer", true), col("user_id", "integer", false)];
        let mut cat = catalog();
        cat.tables.push(table("other", "users", &[("id", "integer")]));
        let r = infer("public", "posts", &cols, &cat, &[]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].other_schema, "public");
    }
}
