//! Inline edits to table rows: one UPDATE per changed row, identified by its
//! primary key values as they were loaded.

use crate::sql::quote_ident;

/// `NULL` or a quoted string literal. Postgres coerces an untyped literal
/// to the column's type, so this works for numbers, json, dates, ….
fn literal(v: Option<&str>) -> String {
    match v {
        None => "NULL".into(),
        Some(v) => format!("'{}'", v.replace('\'', "''")),
    }
}

/// `key = value and …`, with `is null` for NULL values.
fn key_cond(key: &[(&str, Option<&str>)]) -> String {
    let cond: Vec<String> = key
        .iter()
        .map(|(c, v)| match v {
            None => format!("{} is null", quote_ident(c)),
            Some(_) => format!("{} = {}", quote_ident(c), literal(*v)),
        })
        .collect();
    cond.join(" and ")
}

/// `update "schema"."table" set "a" = '1', "b" = NULL where "id" = '5';`
/// `key` holds the row's original primary key values.
pub fn update_sql(schema: &str, table: &str, key: &[(&str, Option<&str>)], set: &[(&str, Option<&str>)]) -> String {
    let set: Vec<String> = set.iter().map(|(c, v)| format!("{} = {}", quote_ident(c), literal(*v))).collect();
    format!("update {}.{} set {} where {};", quote_ident(schema), quote_ident(table), set.join(", "), key_cond(key))
}

/// `delete from "schema"."table" where "id" = '5';`
/// `key` holds the row's original primary key values.
pub fn delete_sql(schema: &str, table: &str, key: &[(&str, Option<&str>)]) -> String {
    format!("delete from {}.{} where {};", quote_ident(schema), quote_ident(table), key_cond(key))
}

/// `insert into "schema"."table" ("a", "b") values ('1', NULL);`
/// `values` are the columns to set; leave a column out to let its default
/// apply. With none, `default values`.
pub fn insert_sql(schema: &str, table: &str, values: &[(&str, Option<&str>)]) -> String {
    let target = format!("{}.{}", quote_ident(schema), quote_ident(table));
    if values.is_empty() {
        return format!("insert into {target} default values;");
    }
    let cols: Vec<String> = values.iter().map(|(c, _)| quote_ident(c)).collect();
    let vals: Vec<String> = values.iter().map(|(_, v)| literal(*v)).collect();
    format!("insert into {target} ({}) values ({});", cols.join(", "), vals.join(", "))
}

/// `"col" = 'value'` or `"col" is null`; with `exclude`, `<>` and
/// `is not null`. For the table's quick filter.
pub fn value_filter(column: &str, value: Option<&str>, exclude: bool) -> String {
    match (value, exclude) {
        (None, false) => format!("{} is null", quote_ident(column)),
        (None, true) => format!("{} is not null", quote_ident(column)),
        (Some(_), false) => format!("{} = {}", quote_ident(column), literal(value)),
        (Some(_), true) => format!("{} <> {}", quote_ident(column), literal(value)),
    }
}

/// `existing and extra`, with the existing filter in parentheses so an `or`
/// in it stays grouped. Either may be empty.
pub fn and_filter(existing: &str, extra: &str) -> String {
    let existing = existing.trim().trim_end_matches(';').trim_end();
    if existing.is_empty() {
        return extra.to_string();
    }
    format!("({existing}) and {extra}")
}

/// WHERE clause matching any of `rows`, each a list of (column, value)
/// pairs: `"user_id" = '5'`, `"user_id" in ('5', '7')`, or OR-ed groups
/// for multi-column keys. `None` when no row has a complete (non-NULL) key.
pub fn match_rows(rows: &[Vec<(&str, Option<&str>)>]) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    let rows: Vec<&Vec<(&str, Option<&str>)>> = rows
        .iter()
        .filter(|r| !r.is_empty() && r.iter().all(|(_, v)| v.is_some()))
        .filter(|r| seen.insert(r.iter().map(|(_, v)| *v).collect::<Vec<_>>()))
        .collect();
    let first = rows.first()?;
    if first.len() == 1 {
        let col = quote_ident(first[0].0);
        let values: Vec<String> = rows.iter().map(|r| literal(r[0].1)).collect();
        return Some(match values.as_slice() {
            [one] => format!("{col} = {one}"),
            _ => format!("{col} in ({})", values.join(", ")),
        });
    }
    let groups: Vec<String> = rows
        .iter()
        .map(|r| {
            let conds: Vec<String> = r.iter().map(|(c, v)| format!("{} = {}", quote_ident(c), literal(*v))).collect();
            format!("({})", conds.join(" and "))
        })
        .collect();
    Some(groups.join(" or "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_rows() {
        assert_eq!(match_rows(&[vec![("user_id", Some("5"))]]).unwrap(), r#""user_id" = '5'"#);
        let rows = [vec![("id", Some("5"))], vec![("id", Some("7"))], vec![("id", Some("5"))], vec![("id", None)]];
        assert_eq!(match_rows(&rows).unwrap(), r#""id" in ('5', '7')"#);
        let rows = [vec![("a", Some("1")), ("b", Some("x"))], vec![("a", Some("2")), ("b", Some("y"))]];
        assert_eq!(match_rows(&rows).unwrap(), r#"("a" = '1' and "b" = 'x') or ("a" = '2' and "b" = 'y')"#);
        assert_eq!(match_rows(&[vec![("id", None)]]), None);
        assert_eq!(match_rows(&[]), None);
    }

    #[test]
    fn builds_updates() {
        assert_eq!(
            update_sql("public", "parcels", &[("id", Some("5"))], &[("name", Some("O'Neil")), ("note", None)]),
            r#"update "public"."parcels" set "name" = 'O''Neil', "note" = NULL where "id" = '5';"#
        );
        assert_eq!(
            update_sql("gis", "My Table", &[("a", Some("1")), ("b", None)], &[("x", Some("ផ្លូវ"))]),
            r#"update "gis"."My Table" set "x" = 'ផ្លូវ' where "a" = '1' and "b" is null;"#
        );
    }

    #[test]
    fn builds_inserts() {
        assert_eq!(
            insert_sql("public", "parcels", &[("name", Some("O'Neil")), ("note", None)]),
            r#"insert into "public"."parcels" ("name", "note") values ('O''Neil', NULL);"#
        );
        assert_eq!(insert_sql("gis", "My Table", &[("x", Some("ផ្លូវ"))]), r#"insert into "gis"."My Table" ("x") values ('ផ្លូវ');"#);
        assert_eq!(insert_sql("public", "t", &[]), r#"insert into "public"."t" default values;"#);
    }

    #[test]
    fn builds_value_filters() {
        assert_eq!(value_filter("status", Some("open"), false), r#""status" = 'open'"#);
        assert_eq!(value_filter("My Col", Some("it's"), true), r#""My Col" <> 'it''s'"#);
        assert_eq!(value_filter("note", None, false), r#""note" is null"#);
        assert_eq!(value_filter("note", None, true), r#""note" is not null"#);
    }

    #[test]
    fn ands_filters() {
        assert_eq!(and_filter("", r#""a" = '1'"#), r#""a" = '1'"#);
        assert_eq!(and_filter("  ", r#""a" = '1'"#), r#""a" = '1'"#);
        assert_eq!(and_filter("x = 1 or y = 2;", r#""a" = '1'"#), r#"(x = 1 or y = 2) and "a" = '1'"#);
    }

    #[test]
    fn builds_deletes() {
        assert_eq!(delete_sql("public", "parcels", &[("id", Some("5"))]), r#"delete from "public"."parcels" where "id" = '5';"#);
        assert_eq!(delete_sql("gis", "My Table", &[("a", Some("O'Neil")), ("b", None)]), r#"delete from "gis"."My Table" where "a" = 'O''Neil' and "b" is null;"#);
    }
}
