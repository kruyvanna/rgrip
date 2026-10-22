//! The data behind a schema's ER diagram: its tables with columns, the real
//! foreign keys between them, and guessed links (see [`crate::relations`]) for
//! databases that declare none. Links to other schemas are left out.

use crate::complete::Catalog;
use crate::model::{ColumnInfo, Relation, TableKind};
use crate::relations;

#[derive(Clone, Debug)]
pub struct DiagramTable {
    pub name: String,
    pub kind: TableKind,
    pub columns: Vec<ColumnInfo>,
}

/// `from.columns[..].0` references `to.columns[..].1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// Indexes into [`Diagram::tables`].
    pub from: usize,
    pub to: usize,
    pub name: String,
    pub columns: Vec<(String, String)>,
    pub inferred: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Diagram {
    pub tables: Vec<DiagramTable>,
    pub links: Vec<Link>,
}

/// Join the tables with their real foreign keys (`fks`: the table that owns
/// each, and the outgoing relation) and add the guessed links that no foreign
/// key already covers.
pub fn assemble(schema: &str, tables: Vec<DiagramTable>, fks: Vec<(String, Relation)>, catalog: &Catalog) -> Diagram {
    let index = |name: &str| tables.iter().position(|t| t.name == name);
    let mut links: Vec<Link> = Vec::new();
    for (owner, r) in &fks {
        if r.other_schema != schema {
            continue;
        }
        if let (Some(from), Some(to)) = (index(owner), index(&r.other_table)) {
            links.push(Link { from, to, name: r.name.clone(), columns: r.columns.clone(), inferred: false });
        }
    }
    for (from, t) in tables.iter().enumerate() {
        if !matches!(t.kind, TableKind::Table | TableKind::Partitioned) {
            continue;
        }
        let own: Vec<Relation> = fks.iter().filter(|(o, _)| *o == t.name).map(|(_, r)| r.clone()).collect();
        for r in relations::infer(schema, &t.name, &t.columns, catalog, &own) {
            let Some(to) = index(&r.other_table).filter(|_| r.outgoing && r.other_schema == schema) else { continue };
            // A foreign key on the same column to the same table already says it.
            let col = &r.columns[0].0;
            if links.iter().any(|l| !l.inferred && l.from == from && l.to == to && l.columns.iter().any(|(c, _)| c == col)) {
                continue;
            }
            links.push(Link { from, to, name: r.name, columns: r.columns, inferred: true });
        }
    }
    Diagram { tables, links }
}

/// Gap between boxes, and between components.
const GAP: (f32, f32) = (90.0, 28.0);

/// Top-left positions for boxes of `sizes` (width, height) joined by `edges`.
/// Connected tables go in layers (columns) by breadth-first distance from the
/// component's most-connected table; components stack downwards, and tables
/// without any edge follow in a grid.
pub fn auto_layout(sizes: &[(f32, f32)], edges: &[(usize, usize)]) -> Vec<(f32, f32)> {
    let n = sizes.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in edges {
        if a != b && !adj[a].contains(&b) {
            adj[a].push(b);
            adj[b].push(a);
        }
    }
    let mut roots: Vec<usize> = (0..n).filter(|&i| !adj[i].is_empty()).collect();
    roots.sort_by_key(|&i| std::cmp::Reverse(adj[i].len()));

    let mut pos = vec![(0.0, 0.0); n];
    let mut seen = vec![false; n];
    let mut y0 = 0.0f32;
    let mut width = 0.0f32;
    for root in roots {
        if seen[root] {
            continue;
        }
        let mut layers: Vec<Vec<usize>> = vec![vec![root]];
        seen[root] = true;
        loop {
            let mut next = Vec::new();
            for &t in layers.last().unwrap() {
                for &o in &adj[t] {
                    if !seen[o] {
                        seen[o] = true;
                        next.push(o);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            layers.push(next);
        }
        let mut x = 0.0;
        let mut tallest = 0.0f32;
        for layer in &layers {
            let mut y = y0;
            for &t in layer {
                pos[t] = (x, y);
                y += sizes[t].1 + GAP.1;
            }
            tallest = tallest.max(y - y0);
            x += layer.iter().map(|&t| sizes[t].0).fold(0.0, f32::max) + GAP.0;
        }
        width = width.max(x);
        y0 += tallest + GAP.1;
    }

    // The rest, in rows about as wide as the widest component (at least 4 boxes).
    let mut x = 0.0;
    let mut row_h = 0.0f32;
    let wrap = width.max(4.0 * 240.0);
    for i in (0..n).filter(|&i| !seen[i]) {
        if x > 0.0 && x + sizes[i].0 > wrap {
            y0 += row_h + GAP.1;
            x = 0.0;
            row_h = 0.0;
        }
        pos[i] = (x, y0);
        x += sizes[i].0 + GAP.0 / 2.0;
        row_h = row_h.max(sizes[i].1);
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::complete::CatalogTable;

    fn col(name: &str, ty: &str, pk: bool) -> ColumnInfo {
        ColumnInfo { name: name.into(), data_type: ty.into(), nullable: !pk, default: None, primary_key: pk }
    }

    fn table(name: &str, cols: Vec<ColumnInfo>) -> DiagramTable {
        DiagramTable { name: name.into(), kind: TableKind::Table, columns: cols }
    }

    fn fk(name: &str, schema: &str, other: &str, here: &str) -> Relation {
        Relation { name: name.into(), outgoing: true, other_schema: schema.into(), other_table: other.into(), columns: vec![(here.into(), "id".into())], inferred: false, alternatives: Vec::new() }
    }

    fn catalog(tables: &[DiagramTable]) -> Catalog {
        Catalog {
            tables: tables
                .iter()
                .map(|t| CatalogTable {
                    schema: "public".into(),
                    name: t.name.clone(),
                    kind: t.kind,
                    columns: t.columns.iter().map(|c| (c.name.clone(), c.data_type.clone())).collect(),
                    keys: t.columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn real_and_inferred_links_without_duplicates() {
        let tables = vec![
            table("categories", vec![col("id", "integer", true)]),
            table("posts", vec![col("id", "integer", true), col("user_id", "integer", false), col("category_id", "integer", false)]),
            table("users", vec![col("id", "integer", true)]),
        ];
        let cat = catalog(&tables);
        // posts.user_id is a real foreign key; category_id is only guessed.
        let fks = vec![("posts".to_string(), fk("posts_user_fk", "public", "users", "user_id")), ("posts".to_string(), fk("elsewhere", "other", "users", "x"))];
        let d = assemble("public", tables, fks, &cat);
        let brief: Vec<_> = d.links.iter().map(|l| (d.tables[l.from].name.as_str(), d.tables[l.to].name.as_str(), l.inferred)).collect();
        assert_eq!(brief, [("posts", "users", false), ("posts", "categories", true)]);
    }

    #[test]
    fn layout_layers_and_isolated() {
        let sizes = vec![(100.0, 50.0); 4];
        // 0 - 1 - 2 chain, 3 alone.
        let pos = auto_layout(&sizes, &[(0, 1), (1, 2)]);
        // 1 is the hub: layer 0; 0 and 2 share layer 1.
        assert!(pos[1].0 < pos[0].0 && pos[0].0 == pos[2].0);
        assert!(pos[3].1 > pos[0].1.max(pos[2].1));
        for i in 0..4 {
            for j in i + 1..4 {
                let (a, b) = (pos[i], pos[j]);
                let apart = a.0 + 100.0 <= b.0 || b.0 + 100.0 <= a.0 || a.1 + 50.0 <= b.1 || b.1 + 50.0 <= a.1;
                assert!(apart, "{i} overlaps {j}");
            }
        }
    }
}
