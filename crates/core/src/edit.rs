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

/// `update "schema"."table" set "a" = '1', "b" = NULL where "id" = '5';`
/// `key` holds the row's original primary key values.
pub fn update_sql(schema: &str, table: &str, key: &[(&str, Option<&str>)], set: &[(&str, Option<&str>)]) -> String {
    let set: Vec<String> = set.iter().map(|(c, v)| format!("{} = {}", quote_ident(c), literal(*v))).collect();
    let cond: Vec<String> = key
        .iter()
        .map(|(c, v)| match v {
            None => format!("{} is null", quote_ident(c)),
            Some(_) => format!("{} = {}", quote_ident(c), literal(*v)),
        })
        .collect();
    format!(
        "update {}.{} set {} where {};",
        quote_ident(schema),
        quote_ident(table),
        set.join(", "),
        cond.join(" and ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
