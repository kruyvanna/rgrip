//! Relations guessed from naming conventions, for databases without foreign
//! key constraints (ArcGIS, legacy schemas). Conservative on purpose: a missed
//! relation costs nothing, a wrong one sends people to the wrong rows.
//!
//! A table's *keys* are its single-column primary key and unique indexes
//! (ArcGIS tables often have no primary key but unique `id` / `globalid`).
//!
//! Rules, each in both directions (this table → other, other → this):
//! - `X_id` → `X.id` (`X` may be plural: `Xs`, `Xes`, `Xies`). The target
//!   needs an `id` column; it doesn't have to be declared unique.
//! - A column named after a table (`building.parcel`) → that table's unique
//!   `id`.
//! - `[prefix_]X_<key>` (`parcel_globalid`, `original_parcel_globalid`,
//!   `new_parcel_id`) → `X.<key>` when `<key>` is a key of `X`.
//! - Shared key name: a distinctively named key of this table (`parcel_code`,
//!   not `id`/`objectid`/`globalid`…) also appearing in other tables.
//! - Same schema wins; a match in another schema is used only if it is the
//!   only one. Ambiguous matches are dropped.
//! - Both sides must have compatible types (integers and numeric together,
//!   text types together, uuid with uuid, otherwise identical).
//! - System schemas (and PostGIS helpers `tiger`, `tiger_data`, `topology`)
//!   are skipped, as are views as the other side, and anything a real
//!   foreign key already covers.

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

fn has_key(t: &CatalogTable, name: &str) -> bool {
    t.keys.iter().any(|k| k.eq_ignore_ascii_case(name))
}

/// Does `stem` name table `t` (`parcel` / `parcels`, case-insensitive)?
fn names_table(stem: &str, t: &CatalogTable) -> bool {
    plurals(stem).iter().any(|n| t.name.eq_ignore_ascii_case(n))
}

/// Which column of `t` a column named `col` (in another table) refers to,
/// by the naming rules in the module docs.
fn target(col: &str, t: &CatalogTable) -> Option<String> {
    let c = col.to_lowercase();
    // `X_id` → X.id
    if let Some(stem) = c.strip_suffix("_id").filter(|s| !s.is_empty())
        && names_table(stem, t)
        && let Some((id, _)) = column(t, "id")
    {
        return Some(id.clone());
    }
    // `parcel` → parcel.id (only a unique id: a bare name is a weaker hint)
    if names_table(&c, t) && has_key(t, "id") {
        return column(t, "id").map(|(id, _)| id.clone());
    }
    // `[prefix_]parcel_globalid` → parcel.globalid
    for key in &t.keys {
        let Some(head) = c.strip_suffix(&format!("_{}", key.to_lowercase())).filter(|h| !h.is_empty()) else { continue };
        let stem = head.rsplit('_').next().unwrap_or(head);
        // `original_parcel` → `parcel`; also multi-word table names.
        let named = names_table(stem, t) || plurals(head).iter().any(|n| t.name.eq_ignore_ascii_case(n))
            || plurals(&t.name.to_lowercase()).iter().chain([singular(&t.name.to_lowercase())].iter()).any(|n| head.ends_with(&format!("_{n}")) || head == n);
        if named {
            return Some(key.clone());
        }
    }
    None
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
    let this = catalog.tables.iter().find(|t| t.schema == schema && t.name == table);
    // Keys of this table: primary key plus unique indexes from the catalog.
    let mut keys: Vec<&str> = columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect();
    for k in this.map(|t| t.keys.as_slice()).unwrap_or_default() {
        if !keys.iter().any(|x| x.eq_ignore_ascii_case(k)) {
            keys.push(k);
        }
    }
    let is_key = |name: &str| keys.iter().any(|k| k.eq_ignore_ascii_case(name));
    // This table as a catalog entry, with the merged keys (for `target`).
    let me = CatalogTable {
        schema: schema.to_string(),
        name: table.to_string(),
        kind: TableKind::Table,
        columns: columns.iter().map(|c| (c.name.clone(), c.data_type.clone())).collect(),
        keys: keys.iter().map(|k| k.to_string()).collect(),
    };

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
    let ty_of = |t: &CatalogTable, c: &str| column(t, c).map(|(_, ty)| ty.clone());

    // Outgoing: this.col → other.key
    for col in columns.iter().filter(|c| !is_key(&c.name)) {
        let matches: Vec<(&CatalogTable, String)> = candidates
            .iter()
            .filter(|t| !(t.schema == schema && t.name == table) || !col.name.eq_ignore_ascii_case("id"))
            .filter_map(|t| {
                let there = target(&col.name, t)?;
                ty_of(t, &there).filter(|ty| compatible(&col.data_type, ty)).map(|_| (*t, there))
            })
            .collect();
        let same: Vec<_> = matches.iter().filter(|(t, _)| t.schema == schema).collect();
        let pick = if same.len() == 1 { Some(same[0]) } else if same.is_empty() && matches.len() == 1 { Some(&matches[0]) } else { None };
        if let Some((t, there)) = pick {
            add(true, t, &col.name, there);
        }
    }

    // Incoming: other.col → this.key
    for t in &candidates {
        if t.schema == schema && t.name == table {
            continue;
        }
        for (c, ty) in &t.columns {
            if has_key(t, c) && !c.eq_ignore_ascii_case("id") {
                // The other table's own distinctive key isn't a reference.
                if !(keys.iter().any(|k| k.eq_ignore_ascii_case(c))) {
                    continue;
                }
            }
            let here = target(c, &me).or_else(|| {
                // Shared distinctive key name.
                keys.iter()
                    .find(|k| k.eq_ignore_ascii_case(c) && k.len() >= 3 && !GENERIC_KEYS.contains(&k.to_lowercase().as_str()))
                    .map(|k| k.to_string())
            });
            if let Some(here) = here
                && let Some(here_ty) = ty_of(&me, &here)
                && compatible(&here_ty, ty)
            {
                add(false, t, &here, c);
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
            keys: Vec::new(),
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

    /// Shaped like an ArcGIS geodatabase: no primary keys or foreign keys,
    /// unique indexes on `id` / `objectid` / `globalid`, references by
    /// table name and `[prefix_]table_key`.
    #[test]
    fn arcgis_style() {
        let keyed = |name: &str, cols: &[(&str, &str)], keys: &[&str]| CatalogTable {
            keys: keys.iter().map(|k| k.to_string()).collect(),
            ..table("mlmupc", name, cols)
        };
        let cat = Catalog {
            tables: vec![
                keyed("parcel", &[("objectid", "integer"), ("id", "character varying(38)"), ("globalid", "character varying(38)"), ("uprn", "character varying"), ("access_id", "character varying")], &["objectid", "id", "globalid"]),
                keyed("building", &[("objectid", "integer"), ("parcel", "character varying"), ("access_id", "character varying")], &["objectid"]),
                keyed("application_parcel", &[("objectid", "integer"), ("parcel", "character varying(38)")], &["objectid"]),
                keyed("infra_change_item", &[("objectid", "integer"), ("original_parcel_globalid", "character varying"), ("original_parcel_oid", "integer")], &["objectid"]),
                keyed("move_parcels_to_village_detail", &[("objectid", "integer"), ("new_parcel_id", "character varying")], &["objectid"]),
                keyed("audit_log", &[("objectid", "integer"), ("parcel", "integer")], &["objectid"]),
            ],
        };
        let cols = |t: &CatalogTable| t.columns.iter().map(|(n, ty)| col(n, ty, false)).collect::<Vec<_>>();
        let brief = |rs: Vec<Relation>| rs.iter().map(|r| format!("{} {}.{}={}", if r.outgoing { "→" } else { "←" }, r.other_table, r.columns[0].1, r.columns[0].0)).collect::<Vec<_>>();

        let parcel = &cat.tables[0];
        let got = brief(infer("mlmupc", "parcel", &cols(parcel), &cat, &[]));
        assert_eq!(
            got,
            [
                "← application_parcel.parcel=id",
                "← building.parcel=id",
                "← infra_change_item.original_parcel_globalid=globalid",
                "← move_parcels_to_village_detail.new_parcel_id=id",
            ],
            "audit_log.parcel is an integer: no match; objectid/access_id aren't references"
        );
        let building = &cat.tables[1];
        assert_eq!(brief(infer("mlmupc", "building", &cols(building), &cat, &[])), ["→ parcel.id=parcel"]);
    }
}
