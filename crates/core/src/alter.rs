//! Edit table: turn an edited copy of a table's columns into the `ALTER TABLE`
//! statements that get there. Applied as one implicit transaction, so the
//! order only has to be valid step by step: drop, free the key, rename, alter,
//! add, set the key, rename the table.

use crate::model::ColumnInfo;
use crate::sql::quote_ident;

/// One row of the editor. `original` is the column as loaded (`None`: added).
#[derive(Clone, Debug)]
pub struct EditColumn {
    pub original: Option<ColumnInfo>,
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    /// SQL expression; empty for none.
    pub default: String,
    pub primary_key: bool,
    pub drop: bool,
}

impl EditColumn {
    pub fn from_info(c: &ColumnInfo) -> Self {
        Self {
            original: Some(c.clone()),
            name: c.name.clone(),
            data_type: c.data_type.clone(),
            nullable: c.nullable,
            default: c.default.clone().unwrap_or_default(),
            primary_key: c.primary_key,
            drop: false,
        }
    }

    pub fn new_column() -> Self {
        Self { original: None, name: String::new(), data_type: "text".into(), nullable: true, default: String::new(), primary_key: false, drop: false }
    }

    /// Changed in any way from what was loaded.
    pub fn changed(&self) -> bool {
        match &self.original {
            None => !self.drop,
            Some(o) => {
                self.drop
                    || self.name.trim() != o.name
                    || self.data_type.trim() != o.data_type
                    || self.nullable != o.nullable
                    || self.default.trim() != o.default.as_deref().unwrap_or("").trim()
                    || self.primary_key != o.primary_key
            }
        }
    }
}

/// The table being edited.
pub struct TableEdit<'a> {
    pub schema: &'a str,
    pub table: &'a str,
    pub new_name: &'a str,
    pub columns: &'a [EditColumn],
    /// Name of the current primary key constraint, if any.
    pub pk_constraint: Option<&'a str>,
}

/// Why the edit can't be applied yet.
pub fn problem(e: &TableEdit) -> Option<String> {
    if e.new_name.trim().is_empty() {
        return Some("The table needs a name".into());
    }
    let kept: Vec<&EditColumn> = e.columns.iter().filter(|c| !c.drop).collect();
    if kept.is_empty() {
        return Some("A table needs at least one column".into());
    }
    for (i, c) in kept.iter().enumerate() {
        if c.name.trim().is_empty() {
            return Some("A column has no name".into());
        }
        if c.data_type.trim().is_empty() {
            return Some(format!("Column \"{}\" has no type", c.name.trim()));
        }
        if kept[..i].iter().any(|o| o.name.trim() == c.name.trim()) {
            return Some(format!("Column name \"{}\" is used twice", c.name.trim()));
        }
    }
    None
}

pub fn statements(e: &TableEdit) -> Vec<String> {
    let t = format!("{}.{}", quote_ident(e.schema), quote_ident(e.table));
    let mut out = Vec::new();
    let live = |c: &&EditColumn| !c.drop;

    // Drop.
    for c in e.columns.iter().filter(|c| c.drop) {
        if let Some(o) = &c.original {
            out.push(format!("alter table {t} drop column {}", quote_ident(&o.name)));
        }
    }

    // The key: (current key columns, wanted key columns), by final name.
    let old_key: Vec<&str> = e.columns.iter().filter_map(|c| c.original.as_ref().filter(|o| o.primary_key)).map(|o| o.name.as_str()).collect();
    let key_moved = e
        .columns
        .iter()
        .filter(live)
        .any(|c| c.primary_key != c.original.as_ref().is_some_and(|o| o.primary_key))
        || e.columns.iter().any(|c| c.drop && c.original.as_ref().is_some_and(|o| o.primary_key));
    let new_key: Vec<String> = e.columns.iter().filter(live).filter(|c| c.primary_key).map(|c| c.name.trim().to_string()).collect();
    if key_moved && !old_key.is_empty() {
        // A dropped key column takes the constraint with it.
        let dropped_with_column = e.columns.iter().any(|c| c.drop && c.original.as_ref().is_some_and(|o| o.primary_key));
        if !dropped_with_column {
            match e.pk_constraint {
                Some(name) => out.push(format!("alter table {t} drop constraint {}", quote_ident(name))),
                None => out.push(format!("alter table {t} drop constraint {}", quote_ident(&format!("{}_pkey", e.table)))),
            }
        }
    }

    // Rename. Swaps (a→b, b→a) go through temporary names.
    let renames: Vec<(&str, &str)> = e
        .columns
        .iter()
        .filter(live)
        .filter_map(|c| c.original.as_ref().map(|o| (o.name.as_str(), c.name.trim())))
        .filter(|(from, to)| from != to)
        .collect();
    let clash = renames.iter().any(|(_, to)| renames.iter().any(|(from, _)| from == to));
    if clash {
        for (i, (from, _)) in renames.iter().enumerate() {
            out.push(format!("alter table {t} rename column {} to {}", quote_ident(from), quote_ident(&format!("rgrip_tmp_{i}"))));
        }
        for (i, (_, to)) in renames.iter().enumerate() {
            out.push(format!("alter table {t} rename column {} to {}", quote_ident(&format!("rgrip_tmp_{i}")), quote_ident(to)));
        }
    } else {
        for (from, to) in &renames {
            out.push(format!("alter table {t} rename column {} to {}", quote_ident(from), quote_ident(to)));
        }
    }

    // Type, null, default of kept columns, one statement.
    let mut actions = Vec::new();
    for c in e.columns.iter().filter(live) {
        let Some(o) = &c.original else { continue };
        let col = quote_ident(c.name.trim());
        let ty = c.data_type.trim();
        if ty != o.data_type {
            actions.push(format!("alter column {col} type {ty} using {col}::{ty}"));
        }
        let default = c.default.trim();
        if default != o.default.as_deref().unwrap_or("").trim() {
            actions.push(if default.is_empty() { format!("alter column {col} drop default") } else { format!("alter column {col} set default {default}") });
        }
        // Key columns are not null anyway; leave that to the key.
        if c.nullable != o.nullable && !(c.primary_key && c.nullable) {
            actions.push(format!("alter column {col} {} not null", if c.nullable { "drop" } else { "set" }));
        }
    }
    if !actions.is_empty() {
        out.push(format!("alter table {t} {}", actions.join(", ")));
    }

    // Add.
    for c in e.columns.iter().filter(live).filter(|c| c.original.is_none()) {
        let mut def = format!("alter table {t} add column {} {}", quote_ident(c.name.trim()), c.data_type.trim());
        if !c.default.trim().is_empty() {
            def.push_str(&format!(" default {}", c.default.trim()));
        }
        if !c.nullable && !c.primary_key {
            def.push_str(" not null");
        }
        out.push(def);
    }

    // The new key.
    if key_moved && !new_key.is_empty() {
        let cols: Vec<String> = new_key.iter().map(|n| quote_ident(n)).collect();
        out.push(format!("alter table {t} add primary key ({})", cols.join(", ")));
    }

    if e.new_name.trim() != e.table {
        out.push(format!("alter table {t} rename to {}", quote_ident(e.new_name.trim())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, ty: &str, pk: bool) -> ColumnInfo {
        ColumnInfo { name: name.into(), data_type: ty.into(), nullable: !pk, default: None, primary_key: pk }
    }

    fn edit<'a>(columns: &'a [EditColumn], new_name: &'a str) -> TableEdit<'a> {
        TableEdit { schema: "public", table: "t", new_name, columns, pk_constraint: Some("t_pkey") }
    }

    #[test]
    fn unchanged_is_empty() {
        let cols = [EditColumn::from_info(&info("id", "integer", true)), EditColumn::from_info(&info("name", "text", false))];
        assert!(statements(&edit(&cols, "t")).is_empty());
        assert!(!cols.iter().any(EditColumn::changed));
    }

    #[test]
    fn renames_retypes_adds_and_drops() {
        let mut cols = vec![EditColumn::from_info(&info("id", "integer", true)), EditColumn::from_info(&info("shapeiso", "text", false)), EditColumn::from_info(&info("shapeid", "text", false))];
        cols[1].name = "code".into();
        cols[1].nullable = false;
        cols[0].data_type = "bigint".into();
        cols[2].drop = true;
        let mut added = EditColumn::new_column();
        added.name = "area m2".into();
        added.data_type = "numeric".into();
        added.default = "0".into();
        cols.push(added);
        assert_eq!(
            statements(&edit(&cols, "provinces")),
            [
                "alter table \"public\".\"t\" drop column \"shapeid\"",
                "alter table \"public\".\"t\" rename column \"shapeiso\" to \"code\"",
                "alter table \"public\".\"t\" alter column \"id\" type bigint using \"id\"::bigint, alter column \"code\" set not null",
                "alter table \"public\".\"t\" add column \"area m2\" numeric default 0",
                "alter table \"public\".\"t\" rename to \"provinces\"",
            ]
        );
    }

    #[test]
    fn swaps_names_through_temporaries() {
        let mut cols = vec![EditColumn::from_info(&info("a", "text", false)), EditColumn::from_info(&info("b", "text", false))];
        cols[0].name = "b".into();
        cols[1].name = "a".into();
        let s = statements(&edit(&cols, "t"));
        assert_eq!(s.len(), 4);
        assert!(s[0].ends_with("rename column \"a\" to \"rgrip_tmp_0\""));
        assert!(s[3].ends_with("rename column \"rgrip_tmp_1\" to \"a\""));
    }

    #[test]
    fn moves_the_key() {
        let mut cols = vec![EditColumn::from_info(&info("id", "integer", true)), EditColumn::from_info(&info("code", "text", false))];
        cols[0].primary_key = false;
        cols[1].primary_key = true;
        assert_eq!(
            statements(&edit(&cols, "t")),
            ["alter table \"public\".\"t\" drop constraint \"t_pkey\"", "alter table \"public\".\"t\" add primary key (\"code\")"]
        );
        // No key before: only add it.
        let mut cols = vec![EditColumn::from_info(&info("code", "text", false))];
        cols[0].primary_key = true;
        assert_eq!(statements(&edit(&cols, "t")), ["alter table \"public\".\"t\" add primary key (\"code\")"]);
    }

    #[test]
    fn dropping_the_key_column_drops_its_constraint_too() {
        let mut cols = vec![EditColumn::from_info(&info("id", "integer", true)), EditColumn::from_info(&info("code", "text", false))];
        cols[0].drop = true;
        assert_eq!(statements(&edit(&cols, "t")), ["alter table \"public\".\"t\" drop column \"id\""]);
    }

    #[test]
    fn problems() {
        let mut cols = vec![EditColumn::from_info(&info("a", "text", false)), EditColumn::from_info(&info("b", "text", false))];
        assert_eq!(problem(&edit(&cols, "t")), None);
        cols[1].name = "a".into();
        assert_eq!(problem(&edit(&cols, "t")).as_deref(), Some("Column name \"a\" is used twice"));
        cols[1].drop = true;
        assert_eq!(problem(&edit(&cols, "t")), None);
        assert!(problem(&edit(&cols, " ")).is_some());
    }
}
