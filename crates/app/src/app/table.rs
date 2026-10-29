use super::*;

/// Columns that identify a row for inline edits: the primary key, else a
/// single-column unique key (ArcGIS tables have no primary key but unique
/// `objectid` / `id` / `globalid`). Empty: read-only. Saves stay safe either
/// way: each UPDATE must change exactly one row.
pub(super) fn edit_key(schema: &str, table: &str, columns: &[ColumnInfo], catalog: &Catalog) -> Vec<String> {
    edit_keys(schema, table, columns, catalog).into_iter().next().unwrap_or_default()
}

/// Every key [`edit_key`] could pick, best first: the primary key alone if
/// there is one, else each single-column unique key, NOT NULL ones first
/// (a row with a null key can't be matched).
fn edit_keys(schema: &str, table: &str, columns: &[ColumnInfo], catalog: &Catalog) -> Vec<Vec<String>> {
    let pk: Vec<String> = columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect();
    if !pk.is_empty() {
        return vec![pk];
    }
    let Some(t) = catalog.tables.iter().find(|t| t.schema == schema && t.name == table) else { return Vec::new() };
    let rank = |k: &String| {
        let nullable = columns.iter().find(|c| c.name == *k).is_none_or(|c| c.nullable);
        (nullable, ["objectid", "id", "globalid", "gid", "fid"].iter().position(|p| k.eq_ignore_ascii_case(p)).unwrap_or(usize::MAX))
    };
    let mut keys = t.keys.clone();
    keys.sort_by_key(rank);
    keys.into_iter().map(|k| vec![k]).collect()
}

/// Real foreign keys plus relations guessed from column names.
pub(super) fn with_inferred(schema: &str, table: &str, columns: &[ColumnInfo], fks: &[Relation], catalog: &Catalog) -> Vec<Relation> {
    let mut all = fks.to_vec();
    all.extend(rgrip_core::relations::infer(schema, table, columns, catalog, fks));
    all
}

pub(super) fn area_action(e: crate::map::AreaEvent) -> TabAction {
    match e {
        crate::map::AreaEvent::Set(sql) => TabAction::ResultArea(Some(sql)),
        crate::map::AreaEvent::Clear => TabAction::ResultArea(None),
    }
}

/// Relations of a query result: those of each source table, real and
/// inferred, renamed to the result's columns.
pub(super) fn result_relations(origins: &[Option<rgrip_core::relations::Origin>], tables: &[SourceTable], names: &[String], catalog: &Catalog) -> Vec<Relation> {
    tables
        .iter()
        .flat_map(|(schema, table, columns, fks)| {
            with_inferred(schema, table, columns, fks, catalog)
                .iter()
                .filter_map(|r| rgrip_core::relations::on_result(r, schema, table, origins, names))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Show the rows related to `rows` through `r` in the popup.
/// `only`: the variants known to match (empty: try them all).
pub(super) fn follow(r: &Relation, rs: &ResultSet, rows: &[usize], only: &[usize]) -> TabAction {
    match related_filter_only(r, rs, rows, only) {
        Some(filter) => {
            let (arrow, cols): (&str, Vec<&str>) = if r.outgoing {
                ("→", r.columns.iter().map(|(here, _)| here.as_str()).collect())
            } else {
                ("←", r.columns.iter().map(|(_, there)| there.as_str()).collect())
            };
            let other = if r.other_schema == "public" { r.other_table.clone() } else { format!("{}.{}", r.other_schema, r.other_table) };
            TabAction::ShowRelated(RelatedTarget {
                schema: r.other_schema.clone(),
                table: r.other_table.clone(),
                filter,
                label: format!("{arrow} {other} ({}){}", cols.join(", "), if r.inferred { "  ~ inferred" } else { "" }),
            })
        }
        None => TabAction::Notify("No related rows: the key is NULL".into()),
    }
}

/// Show the whole table `r` points at in the popup (the lookup icon).
pub(super) fn lookup(r: &Relation) -> TabAction {
    let other = if r.other_schema == "public" { r.other_table.clone() } else { format!("{}.{}", r.other_schema, r.other_table) };
    TabAction::ShowRelated(RelatedTarget { schema: r.other_schema.clone(), table: r.other_table.clone(), filter: String::new(), label: other })
}

/// The column pairs a relation may join on: the main pairs, then each
/// alternative key on its own.
fn relation_variants(r: &Relation) -> Vec<Vec<(String, String)>> {
    std::iter::once(r.columns.clone()).chain(r.alternatives.iter().map(|p| vec![p.clone()])).collect()
}

/// WHERE on the other side for one variant, matching the given rows' values.
fn variant_filter(pairs: &[(String, String)], rs: &ResultSet, rows: &[usize]) -> Option<String> {
    let pairs: Vec<(usize, &str)> = pairs
        .iter()
        .map(|(here, there)| Some((rs.columns.iter().position(|c| c == here)?, there.as_str())))
        .collect::<Option<_>>()?;
    let keys: Vec<Vec<(&str, Option<&str>)>> = rows
        .iter()
        .filter_map(|&row| rs.rows.get(row))
        .map(|row| pairs.iter().map(|&(c, there)| (there, row[c].as_deref())).collect())
        .collect();
    rgrip_core::edit::match_rows(&keys)
}

/// WHERE for the other side of `r`, matching the given rows' key values:
/// any of its variants (only those in `only`, when given). `None` when
/// every key is NULL (nothing related) or a column is missing.
fn related_filter_only(r: &Relation, rs: &ResultSet, rows: &[usize], only: &[usize]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for (v, pairs) in relation_variants(r).iter().enumerate() {
        if !only.is_empty() && !only.contains(&v) {
            continue;
        }
        if let Some(f) = variant_filter(pairs, rs, rows).filter(|f| !parts.contains(f)) {
            parts.push(f);
        }
    }
    match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(parts.iter().map(|p| format!("({p})")).collect::<Vec<_>>().join(" or ")),
    }
}

#[cfg(test)]
fn related_filter(r: &Relation, rs: &ResultSet, rows: &[usize]) -> Option<String> {
    related_filter_only(r, rs, rows, &[])
}

/// One count per relation variant for `rows` (what the row menu shows).
pub(super) fn probe_action(relations: &[Relation], rs: &ResultSet, rows: Vec<usize>, modal: bool) -> Option<TabAction> {
    let (mut probes, mut map) = (Vec::new(), Vec::new());
    for (ri, r) in relations.iter().enumerate() {
        for (vi, pairs) in relation_variants(r).iter().enumerate() {
            if let Some(f) = variant_filter(pairs, rs, &rows) {
                probes.push((r.other_schema.clone(), r.other_table.clone(), f));
                map.push((ri, vi));
            }
        }
    }
    (!probes.is_empty()).then_some(TabAction::Probe { modal, rows, probes, map })
}

/// One UPDATE per edited row, matched on the primary key values as loaded,
/// then one INSERT per pending new row.
pub(super) fn edit_statements(t: &TableView, rs: &ResultSet) -> Result<Vec<String>, String> {
    let first_new = rs.rows.len().saturating_sub(t.added);
    let mut rows: std::collections::BTreeMap<usize, Vec<(&str, Option<&str>)>> = Default::default();
    for ((row, col), value) in t.changes.range(..(first_new, 0)) {
        rows.entry(*row).or_default().push((rs.columns[*col].as_str(), value.as_deref()));
    }
    // Rows are found by key only when an existing row was edited.
    let key_cols: Vec<usize> = if rows.is_empty() {
        Vec::new()
    } else {
        t.key
            .iter()
            .map(|k| rs.columns.iter().position(|c| c == k).ok_or(format!("Primary key column {k} isn't in the result")))
            .collect::<Result<_, _>>()?
    };
    if !rows.is_empty() && key_cols.is_empty() {
        return Err("This table has no key to find rows by".into());
    }
    let mut out: Vec<String> = rows
        .into_iter()
        .map(|(row, set)| {
            let key: Vec<(&str, Option<&str>)> =
                key_cols.iter().map(|&c| (rs.columns[c].as_str(), rs.rows[row][c].as_deref())).collect();
            rgrip_core::edit::update_sql(&t.schema, &t.table, &key, &set)
        })
        .collect();
    for row in first_new..rs.rows.len() {
        // Cells still NULL are left out, so their defaults apply.
        let values: Vec<(&str, Option<&str>)> = (0..rs.columns.len())
            .filter_map(|c| {
                let v = t.changes.get(&(row, c)).map_or(rs.rows[row][c].as_deref(), |v| v.as_deref());
                v.map(|v| (rs.columns[c].as_str(), Some(v)))
            })
            .collect();
        out.push(rgrip_core::edit::insert_sql(&t.schema, &t.table, &values));
    }
    Ok(out)
}

/// Keep an edited cell as a pending change, or drop it when it's back to the
/// value as loaded.
pub(super) fn record_change(changes: &mut std::collections::BTreeMap<(usize, usize), Option<String>>, rs: &ResultSet, row: usize, col: usize, value: Option<String>) {
    if rs.rows[row][col] == value {
        changes.remove(&(row, col));
    } else {
        changes.insert((row, col), value);
    }
}

/// A primary key in a result: (result column, key column in the table) pairs.
type ResultKey = Vec<(usize, String)>;

/// The primary key of a table tab in its page.
pub(super) fn table_key(t: &TableView, rs: &ResultSet) -> Result<ResultKey, String> {
    t.key
        .iter()
        .map(|k| rs.columns.iter().position(|c| c == k).map(|i| (i, k.clone())).ok_or(format!("Primary key column {k} isn't in the result")))
        .collect()
}

/// One DELETE per row, matched on the primary key values as loaded.
pub(super) fn delete_statements(schema: &str, table: &str, key: &[(usize, String)], rs: &ResultSet, rows: &[usize]) -> Vec<String> {
    rows.iter()
        .filter(|&&row| row < rs.rows.len())
        .map(|&row| {
            let values: Vec<(&str, Option<&str>)> = key.iter().map(|(c, name)| (name.as_str(), rs.rows[row][*c].as_deref())).collect();
            rgrip_core::edit::delete_sql(schema, table, &values)
        })
        .collect()
}

/// The table a console result's rows can be edited and deleted in: schema,
/// table and the key rows are matched on ([`edit_keys`]: the primary key, else
/// a unique column). Only when every column read from a table comes from that
/// one table, and a whole key is there.
pub(super) fn console_edit_target(c: &Console, catalog: &Catalog) -> Option<(String, String, ResultKey)> {
    let (origins, tables) = c.sources.as_ref()?;
    let (schema, table, _) = origins.iter().flatten().next()?;
    if origins.iter().flatten().any(|(s, t, _)| s != schema || t != table) {
        return None;
    }
    let (_, _, columns, _) = tables.iter().find(|(s, t, _, _)| s == schema && t == table)?;
    let key = edit_keys(schema, table, columns, catalog).into_iter().find_map(|key| {
        key.into_iter()
            .map(|k| origins.iter().position(|o| o.as_ref().is_some_and(|(_, _, col)| *col == k)).map(|i| (i, k)))
            .collect::<Option<Vec<_>>>()
    })?;
    Some((schema.clone(), table.clone(), key))
}

/// One UPDATE per edited row of a console result, matched on the key values
/// as loaded. Columns are named as in the table (`origins`), not the result;
/// edits to columns that aren't read from the table are skipped.
pub(super) fn console_edit_statements(
    schema: &str,
    table: &str,
    key: &[(usize, String)],
    origins: &[Option<rgrip_core::relations::Origin>],
    rs: &ResultSet,
    changes: &std::collections::BTreeMap<(usize, usize), Option<String>>,
) -> Vec<String> {
    let mut rows: std::collections::BTreeMap<usize, Vec<(&str, Option<&str>)>> = Default::default();
    for ((row, col), value) in changes {
        if let (true, Some(Some((_, _, name)))) = (*row < rs.rows.len(), origins.get(*col)) {
            rows.entry(*row).or_default().push((name.as_str(), value.as_deref()));
        }
    }
    rows.into_iter()
        .map(|(row, set)| {
            let values: Vec<(&str, Option<&str>)> = key.iter().map(|(c, name)| (name.as_str(), rs.rows[row][*c].as_deref())).collect();
            rgrip_core::edit::update_sql(schema, table, &values, &set)
        })
        .collect()
}

/// A small lock and "read-only" for connections whose session rejects writes.
pub(super) fn read_only_badge(ui: &mut Ui) {
    let resp = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
            crate::icons::paint(ui, Svg::Lock, r, theme::WARN());
            ui.label(RichText::new("read-only").size(11.5).color(theme::WARN()));
        })
        .response;
    resp.on_hover_text("Postgres rejects writes on this connection");
}

pub(super) fn table_ui(ui: &mut Ui, tab_id: TabId, t: &mut TableView, read_only: bool) -> Option<TabAction> {
    let mut action = None;
    if t.dirty() && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
        action = Some(TabAction::SaveEdits);
    }
    // Like the ↻ button: not while edits are pending (a reload drops them).
    if !t.dirty() && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::R)) {
        action = Some(TabAction::Reload);
    }
    toolbar(ui, |ui| {
        ui.label(RichText::new(format!("{}.{}", t.schema, t.table)).strong());
        ui.add_space(8.0);
        let filter = ui.add(
            TextEdit::singleline(&mut t.filter)
                .hint_text(hint("WHERE …   (enter to apply)"))
                .font(TextStyle::Monospace)
                .desired_width(320.0)
                .margin(Margin::symmetric(8, 4)),
        );
        // Reloading drops pending edits, so it waits for Save / Discard.
        let locked = t.dirty();
        let rows_len = match &t.data {
            Some(Ok(rs)) => rs.rows.len(),
            _ => 0,
        };
        let can_insert = !read_only && !t.loading && !t.saving && matches!(&t.data, Some(Ok(_)));
        // Escape empties the field and drops the filter.
        let cleared = filter.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape));
        if cleared {
            t.filter.clear();
        }
        if cleared || (filter.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
            if locked {
                action = Some(TabAction::Notify("Save or discard your changes first".into()));
            } else if t.applied_filter != t.filter || !cleared {
                t.applied_filter = t.filter.clone();
                t.page = 0;
                action = Some(TabAction::Reload);
            }
        }
        if ui.add_enabled_ui(!locked, |ui| ghost_button(ui, "↻").on_hover_text("Reload  ⌘R")).inner.clicked() {
            action = Some(TabAction::Reload);
        }
        if ui.add_enabled_ui(can_insert, |ui| ghost_button(ui, "+ Add row")).inner.on_hover_text("Add a blank row, saved as an INSERT with Save").clicked() {
            t.add_row(None);
        }
        if let Some(Ok(rs)) = &t.data {
            if !rs.geometry_columns().is_empty() {
                ui.add_space(6.0);
                if segment(ui, t.pane == Pane::Grid, "Grid") {
                    t.pane = Pane::Grid;
                }
                if segment(ui, t.pane == Pane::Map, "Map") {
                    if t.pane == Pane::Grid {
                        t.map.opened(!t.grid.selection.is_empty());
                    }
                    t.pane = Pane::Map;
                }
                if segment(ui, t.pane == Pane::Split, "Split") {
                    if t.pane == Pane::Grid {
                        t.map.opened(!t.grid.selection.is_empty());
                    }
                    t.pane = Pane::Split;
                }
            }
        }
        if t.loading || t.saving {
            ui.add(egui::Spinner::new().size(12.0));
        }
        if locked {
            ui.add_space(8.0);
            let cells = t.changes.iter().filter(|((r, _), _)| *r < rows_len.saturating_sub(t.added)).count();
            let mut parts = Vec::new();
            if cells > 0 {
                parts.push(plural(cells, "change"));
            }
            if t.added > 0 {
                parts.push(plural(t.added, "new row"));
            }
            ui.label(RichText::new(parts.join(", ")).small().color(theme::ACCENT()));
            let save = Button::new(RichText::new("Save").color(theme::BG())).fill(theme::ACCENT());
            if ui.add_enabled(!t.saving, save).on_hover_text("Write the changes in one transaction  ⌘S").clicked() {
                action = Some(TabAction::SaveEdits);
            }
            if ui.add_enabled(!t.saving, Button::new("Discard")).clicked() {
                t.discard();
            }
        } else if read_only {
            ui.add_space(8.0);
            ui.label(RichText::new("read-only").small().color(theme::FAINT())).on_hover_text(READ_ONLY_HINT);
        } else if matches!(&t.data, Some(Ok(_))) && t.key.is_empty() && !t.loading {
            ui.add_space(8.0);
            ui.label(RichText::new("read-only").small().color(theme::FAINT()))
                .on_hover_text("No primary key or unique column: rows can't be identified for editing");
            if ui.add_enabled(!t.saving, Button::new(RichText::new("Add primary key").small())).on_hover_text("Add an id column numbered 1, 2, 3… as the primary key, so rows can be edited").clicked() {
                action = Some(TabAction::AddKey);
            }
        } else if !t.key.is_empty() && !t.columns.iter().any(|c| c.primary_key) && !locked {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("keyed on {}", t.key.join(", "))).small().color(theme::FAINT()))
                .on_hover_text("No primary key: edits find each row by this unique column");
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(Ok(rs)) = &t.data {
                if let Some(a) = export_menu(ui, rs, &format!("{}.{}", t.schema, t.table), true) {
                    action = Some(a);
                }
                ui.add_space(6.0);
            }
            let rows = match &t.data {
                Some(Ok(rs)) => rs.rows.len() as i64,
                _ => 0,
            };
            let last_page = match t.total {
                Some(total) => (t.page + 1) * PAGE_SIZE >= total,
                None => rows < PAGE_SIZE,
            };
            if ui.add_enabled(!last_page && !t.loading && !locked, Button::new("›")).clicked() {
                t.page += 1;
                action = Some(TabAction::Reload);
            }
            if ui.add_enabled(t.page > 0 && !t.loading && !locked, Button::new("‹")).clicked() {
                t.page -= 1;
                action = Some(TabAction::Reload);
            }
            let from = if rows == 0 { 0 } else { t.page * PAGE_SIZE + 1 };
            let range = format!("{from}–{}", t.page * PAGE_SIZE + rows);
            let text = match t.total {
                Some(total) => format!("{range} of {total}"),
                None => range,
            };
            ui.label(RichText::new(text).small().color(theme::MUTED()));
        });
    });

    match &t.data {
        None => centered(ui, |ui| muted(ui, "loading…")),
        Some(Err(e)) => {
            Frame::new().inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
                ui.label(RichText::new(e.as_str()).monospace().color(theme::ERROR()));
            });
        }
        Some(Ok(rs)) => {
            let cols = rs.geometry_columns().to_vec();
            let pane = if cols.is_empty() { Pane::Grid } else { t.pane };
            let (table_name, table_sql) = (format!("{}.{}", t.schema, t.table), rgrip_core::postgres::table_select_sql(&t.query()));
            let sort = t.order.as_ref().map(|(c, asc)| Sort { column: c, ascending: *asc });
            let offset = (t.page * PAGE_SIZE) as usize;
            let kinds: Vec<_> = rs.columns.iter().map(|c| t.temporal.get(c).copied()).collect();
            let menu_rows: Vec<usize> = t.grid.selection.iter().copied().collect();
            let counts = t.probe.as_ref().map(|p| p.best(&menu_rows, t.relations.len())).unwrap_or_default();
            // Without a key only new rows can be edited; existing ones can't be found again.
            let has_key = !t.key.is_empty();
            let added_from = if t.added > 0 { rs.rows.len().saturating_sub(t.added) } else { usize::MAX };
            let editing = (!t.saving && !read_only).then_some(grid::Editing {
                changes: &t.changes,
                editor: &mut t.editor,
                kinds: &kinds,
                locked: &[],
                added_from,
                existing_locked: !has_key,
                insertable: true,
            });
            // Both views take the grid state as an argument (they share its selection).
            let draw_grid = |ui: &mut Ui, g: &mut GridState| {
                grid::show(
                    ui,
                    Id::new(("table-grid", tab_id)),
                    rs,
                    g,
                    sort,
                    offset,
                    &grid::Source { table: &table_name, sql: Some(&table_sql), relations: &t.relations, counts: &counts, deletable: editing.is_some() && has_key, filterable: true },
                    editing,
                )
            };
            let applied = t.applied_filter.as_str();
            let map = &mut t.map;
            let mut draw_map = |ui: &mut Ui, g: &mut GridState| {
                let link = crate::map::Link { selection: &mut g.selection, reveal: &mut g.reveal, layer: Some(table_name.as_str()), area_filter: Some(applied) };
                map.show(ui, rs, &cols, false, link)
            };
            let mut event = None;
            let mut row_op: Option<(bool, Vec<usize>)> = None;
            let mut map_out = crate::map::MapOutput::default();
            match pane {
                Pane::Grid => event = draw_grid(ui, &mut t.grid),
                Pane::Map => map_out = draw_map(ui, &mut t.grid),
                Pane::Split => {
                    let (mut e, mut m) = (None, None);
                    split_view(ui, Id::new(("split", tab_id)), &mut t.grid, |ui, g| m = Some(draw_map(ui, g)), |ui, g| e = draw_grid(ui, g));
                    event = e;
                    map_out = m.unwrap_or_default();
                }
            }
            match map_out.area {
                // Reloading drops pending edits, so it waits for Save / Discard.
                Some(_) if t.dirty() => {
                    action = Some(TabAction::Notify("Save or discard your changes first".into()));
                }
                Some(crate::map::AreaEvent::Set(sql)) => {
                    t.filter = sql.clone();
                    t.applied_filter = sql;
                    t.page = 0;
                    action = Some(TabAction::Reload);
                }
                Some(crate::map::AreaEvent::Clear) => {
                    t.filter.clear();
                    t.applied_filter.clear();
                    t.page = 0;
                    action = Some(TabAction::Reload);
                }
                None => {}
            }
            match event {
                Some(GridEvent::Change { row, col, value }) => record_change(&mut t.changes, rs, row, col, value),
                Some(GridEvent::Revert { row, col }) => {
                    t.changes.remove(&(row, col));
                }
                Some(GridEvent::Follow { relation, rows }) => {
                    let only = t.probe.as_ref().map(|p| p.matched(&rows, relation)).unwrap_or_default();
                    action = Some(follow(&t.relations[relation], rs, &rows, &only));
                }
                Some(GridEvent::RowsMenu { rows }) => {
                    action = probe_action(&t.relations, rs, rows, false);
                }
                Some(GridEvent::Lookup { relation }) => action = Some(lookup(&t.relations[relation])),
                Some(GridEvent::Delete { rows }) => action = Some(TabAction::DeleteRows(rows)),
                Some(GridEvent::FilterValue { .. }) if t.dirty() => {
                    action = Some(TabAction::Notify("Save or discard your changes first".into()));
                }
                Some(GridEvent::FilterValue { row, col, exclude }) => {
                    let cond = rgrip_core::edit::value_filter(&rs.columns[col], rs.rows[row][col].as_deref(), exclude);
                    t.filter = rgrip_core::edit::and_filter(&t.applied_filter, &cond);
                    t.applied_filter = t.filter.clone();
                    t.page = 0;
                    action = Some(TabAction::Reload);
                }
                // These change the page's rows, which `rs` borrows: after the match.
                Some(GridEvent::Duplicate { rows }) => row_op = Some((true, rows)),
                Some(GridEvent::RemoveNew { rows }) => row_op = Some((false, rows)),
                // Sorting reloads the page, which would drop pending edits.
                Some(GridEvent::HeaderClicked(_)) if t.dirty() => {
                    action = Some(TabAction::Notify("Save or discard your changes first".into()));
                }
                Some(GridEvent::HeaderClicked(i)) => {
                    let col = rs.columns[i].clone();
                    // Cycle: ascending → descending → unsorted.
                    t.order = match t.order.take() {
                        Some((c, true)) if c == col => Some((c, false)),
                        Some((c, false)) if c == col => None,
                        _ => Some((col, true)),
                    };
                    t.page = 0;
                    action = Some(TabAction::Reload);
                }
                None => {}
            }
            match row_op {
                Some((true, rows)) => {
                    for r in rows {
                        t.add_row(Some(r));
                    }
                }
                Some((false, rows)) => t.remove_added(&rows),
                None => {}
            }
        }
    }
    if let Some(a) = related_ui(ui, tab_id, &mut t.related) {
        action = Some(a);
    }
    action
}

/// The related-rows popup: a read-only grid of the rows on the other side
/// of a foreign key, which can follow further relations.
pub(super) fn related_ui(ui: &mut Ui, tab_id: TabId, related: &mut Option<Related>) -> Option<TabAction> {
    let rel = related.as_mut()?;
    let mut action = None;
    let mut close = false;
    let screen = ui.ctx().content_rect();
    let modal = egui::Modal::new(Id::new(("related", tab_id))).show(ui.ctx(), |ui| {
        ui.set_width((screen.width() * 0.85).min(1400.0));
        ui.horizontal(|ui| {
            if !rel.back.is_empty() && ghost_button(ui, "‹ Back").clicked() {
                action = Some(TabAction::RelatedBack);
            }
            ui.label(RichText::new(&rel.target.label).strong());
            ui.label(RichText::new(&rel.target.filter).monospace().small().color(theme::MUTED()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if crate::widgets::icon_button(ui, Icon::Close, "Close  Esc").clicked() {
                    close = true;
                }
                if ghost_button(ui, "Open as tab").on_hover_text("Open the table filtered to these rows").clicked() {
                    action = Some(TabAction::OpenFiltered {
                        schema: rel.target.schema.clone(),
                        table: rel.target.table.clone(),
                        filter: rel.target.filter.clone(),
                    });
                    close = true;
                }
                if let Some(Ok(rs)) = &rel.data {
                    let n = rs.rows.len() as i64;
                    let text = match rel.total {
                        Some(total) if total > n => format!("first {n} of {total}"),
                        _ => plural(n as usize, "row"),
                    };
                    ui.label(RichText::new(text).small().color(theme::MUTED()));
                }
            });
        });
        ui.add_space(6.0);
        let height = (screen.height() * 0.7).max(200.0);
        ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
            ui.set_min_height(height);
            match &rel.data {
                None => centered(ui, |ui| muted(ui, "loading…")),
                Some(Err(e)) => {
                    ui.label(RichText::new(e.as_str()).monospace().color(theme::ERROR()));
                }
                Some(Ok(rs)) if rs.rows.is_empty() => centered(ui, |ui| muted(ui, if rel.target.filter.is_empty() { "no rows" } else { "no related rows" })),
                Some(Ok(rs)) => {
                    let name = format!("{}.{}", rel.target.schema, rel.target.table);
                    let menu_rows: Vec<usize> = rel.grid.selection.iter().copied().collect();
                    let counts = rel.probe.as_ref().map(|p| p.best(&menu_rows, rel.relations.len())).unwrap_or_default();
                    let source = grid::Source { table: &name, sql: None, relations: &rel.relations, counts: &counts, deletable: false, filterable: false };
                    let event = grid::show(ui, Id::new(("related-grid", tab_id)), rs, &mut rel.grid, None, 0, &source, None);
                    match event {
                        Some(GridEvent::Follow { relation, rows }) => {
                            let only = rel.probe.as_ref().map(|p| p.matched(&rows, relation)).unwrap_or_default();
                            action = Some(follow(&rel.relations[relation], rs, &rows, &only));
                        }
                        Some(GridEvent::RowsMenu { rows }) => action = probe_action(&rel.relations, rs, rows, true),
                        Some(GridEvent::Lookup { relation }) => action = Some(lookup(&rel.relations[relation])),
                        _ => {}
                    }
                }
            }
        });
    });
    if close || modal.should_close() {
        *related = None;
    }
    action
}


#[cfg(test)]
mod tests {
    use super::*;
    use rgrip_core::complete::CatalogTable;

    #[test]
    fn related_popup() {
        let s = |v: &str| Some(v.to_string());
        let rows = |n: usize| ResultSet {
            columns: vec!["id".into(), "user_id".into()],
            rows: (0..n).map(|i| vec![Some(i.to_string()), s("5")]).collect(),
            ..Default::default()
        };
        let mut t = TableView::new("public".into(), "users".into(), String::new());
        t.data = Some(Ok(rows(1)));
        let target = |label: &str| RelatedTarget {
            schema: "public".into(),
            table: "posts".into(),
            filter: r#""user_id" = '5'"#.into(),
            label: label.into(),
        };
        t.related = Some(Related {
            target: target("← posts (user_id)"),
            back: vec![target("previous")],
            data: Some(Ok(rows(3))),
            total: Some(3),
            relations: Vec::new(),
            columns: Vec::new(),
            fk_relations: Vec::new(),
            grid: GridState::default(),
            probe: None,
        });
        let ctx = egui::Context::default();
        let mut frame = |t: &mut TableView, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events,
                ..Default::default()
            };
            let mut action = None;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| action = table_ui(ui, 1, t, false));
            });
            action
        };
        frame(&mut t, vec![]);
        frame(&mut t, vec![]);
        // The popup is up, over the tab, with its rows.
        let modal = ctx.memory(|m| m.area_rect(Id::new(("related", 1u64)))).expect("modal shown");
        assert!(modal.width() > 900.0 && modal.height() > 400.0, "{modal:?}");
        // Esc closes it.
        let esc = egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        frame(&mut t, vec![esc]);
        assert!(t.related.is_none());
    }

    #[test]
    fn new_rows_are_saved_as_inserts() {
        let s = |v: &str| Some(v.to_string());
        let col = |name: &str, pk: bool| ColumnInfo { name: name.into(), data_type: "text".into(), nullable: !pk, default: None, primary_key: pk };
        let mut t = TableView::new("public".into(), "users".into(), String::new());
        t.data = Some(Ok(ResultSet { columns: vec!["id".into(), "name".into(), "note".into()], rows: vec![vec![s("1"), s("Ann"), None]], ..Default::default() }));
        t.columns = vec![col("id", true), col("name", false), col("note", false)];
        t.key = vec!["id".into()];
        assert!(!t.dirty());
        t.add_row(None);
        t.add_row(Some(0));
        assert!(t.dirty());
        // Empty cells are left out; the copy has no key.
        t.changes.insert((1, 1), s("O'Neil"));
        let Some(Ok(rs)) = &t.data else { panic!() };
        let sql = edit_statements(&t, rs).unwrap();
        assert_eq!(
            sql,
            [r#"insert into "public"."users" ("name") values ('O''Neil');"#, r#"insert into "public"."users" ("name") values ('Ann');"#]
        );
        // Edits to existing rows come first, matched on the key.
        t.changes.insert((0, 1), s("Anna"));
        let Some(Ok(rs)) = &t.data else { panic!() };
        assert_eq!(edit_statements(&t, rs).unwrap()[0], r#"update "public"."users" set "name" = 'Anna' where "id" = '1';"#);
        // Removing the first new row moves the copy (and its edits) up.
        t.changes.insert((2, 2), s("hi"));
        t.remove_added(&[1]);
        assert_eq!(t.added, 1);
        let Some(Ok(rs)) = &t.data else { panic!() };
        assert_eq!(edit_statements(&t, rs).unwrap()[1], r#"insert into "public"."users" ("name", "note") values ('Ann', 'hi');"#);
        t.discard();
        let Some(Ok(rs)) = &t.data else { panic!() };
        assert_eq!((rs.rows.len(), t.dirty()), (1, false));
    }

    #[test]
    fn console_delete_needs_one_table_and_its_key() {
        let col = |name: &str, pk: bool| ColumnInfo { name: name.into(), data_type: "integer".into(), nullable: !pk, default: None, primary_key: pk };
        let origin = |t: &str, c: &str| Some(("public".to_string(), t.to_string(), c.to_string()));
        let users = ("public".to_string(), "users".to_string(), vec![col("id", true), col("name", false)], Vec::new());
        let orders = ("public".to_string(), "orders".to_string(), vec![col("id", true)], Vec::new());
        let catalog = Catalog { tables: Vec::new() };
        let mut c = Console::new();
        assert!(console_edit_target(&c, &catalog).is_none(), "no origins yet");
        // `select name, id as user_id, 1 from users`: the key found by origin.
        c.sources = Some((vec![origin("users", "name"), origin("users", "id"), None], vec![users.clone()]));
        assert_eq!(console_edit_target(&c, &catalog), Some(("public".into(), "users".into(), vec![(1, "id".into())])));
        c.sources = Some((vec![origin("users", "name")], vec![users.clone()]));
        assert!(console_edit_target(&c, &catalog).is_none(), "key column not selected");
        c.sources = Some((vec![origin("users", "id"), origin("orders", "id")], vec![users, orders]));
        assert!(console_edit_target(&c, &catalog).is_none(), "a join");
    }

    #[test]
    fn console_edits_fall_back_to_a_unique_key() {
        let col = |name: &str, nullable: bool| ColumnInfo { name: name.into(), data_type: "integer".into(), nullable, default: None, primary_key: false };
        let origin = |c: &str| Some(("mlmupc".to_string(), "parcel".to_string(), c.to_string()));
        let parcel = ("mlmupc".to_string(), "parcel".to_string(), vec![col("id", true), col("objectid", false), col("globalid", false), col("name", true)], Vec::new());
        let catalog = Catalog {
            tables: vec![CatalogTable {
                schema: "mlmupc".into(),
                name: "parcel".into(),
                kind: TableKind::Table,
                columns: Vec::new(),
                keys: vec!["id".into(), "globalid".into(), "objectid".into()],
            }],
        };
        let mut c = Console::new();
        // `select * from mlmupc.parcel`: no primary key, but a unique NOT NULL objectid.
        c.sources = Some((vec![origin("id"), origin("objectid"), origin("globalid"), origin("name")], vec![parcel.clone()]));
        assert_eq!(console_edit_target(&c, &catalog), Some(("mlmupc".into(), "parcel".into(), vec![(1, "objectid".into())])));
        // Without objectid: the next NOT NULL unique column, ahead of the nullable id.
        c.sources = Some((vec![origin("id"), origin("globalid")], vec![parcel.clone()]));
        assert_eq!(console_edit_target(&c, &catalog), Some(("mlmupc".into(), "parcel".into(), vec![(1, "globalid".into())])));
        c.sources = Some((vec![origin("name")], vec![parcel]));
        assert!(console_edit_target(&c, &catalog).is_none(), "no key column selected");
    }

    #[test]
    fn console_edits_use_table_column_names() {
        // `select id as user_id, name as who, 1 as one from users`
        let origin = |c: &str| Some(("public".to_string(), "users".to_string(), c.to_string()));
        let origins = vec![origin("id"), origin("name"), None];
        let rs = ResultSet { columns: vec!["user_id".into(), "who".into(), "one".into()], rows: vec![vec![Some("5".into()), Some("Ann".into()), Some("1".into())]], ..Default::default() };
        let changes = [((0, 1), Some("Bo".to_string())), ((0, 2), Some("2".to_string())), ((3, 1), None)].into_iter().collect();
        let statements = console_edit_statements("public", "users", &[(0, "id".into())], &origins, &rs, &changes);
        assert_eq!(statements, [r#"update "public"."users" set "name" = 'Bo' where "id" = '5';"#], "alias mapped, expression and missing row skipped");
    }

    #[test]
    fn edit_key_falls_back_to_unique_columns() {
        let col = |name: &str, pk: bool| ColumnInfo { name: name.into(), data_type: "integer".into(), nullable: !pk, default: None, primary_key: pk };
        let catalog = Catalog {
            tables: vec![CatalogTable {
                schema: "mlmupc".into(),
                name: "parcel".into(),
                kind: TableKind::Table,
                columns: Vec::new(),
                keys: vec!["globalid".into(), "objectid".into(), "id".into()],
            }],
        };
        assert_eq!(edit_key("mlmupc", "parcel", &[col("objectid", false)], &catalog), ["objectid"]);
        assert_eq!(edit_key("mlmupc", "parcel", &[col("code", true)], &catalog), ["code"], "a primary key wins");
        assert!(edit_key("mlmupc", "other", &[col("x", false)], &catalog).is_empty());
        let named = ConnectionConfig { name: "CIS Prod".into(), database: "cis".into(), host: "db".into(), ..Default::default() };
        let unnamed = ConnectionConfig { database: "cis_next".into(), host: "cis-db2".into(), ..Default::default() };
        assert_eq!(conn_db_title(&named, "cis"), "CIS Prod@cis");
        assert_eq!(conn_db_title(&unnamed, "cis_next"), "cis_next@cis-db2");
        assert_eq!(console_title("cis_next@cis-db2 (2)", &unnamed, "other").as_deref(), Some("other@cis-db2 (2)"));
    }

    #[test]
    fn related_rows_filter() {
        let s = |v: &str| Some(v.to_string());
        let users = ResultSet {
            columns: vec!["id".into(), "name".into()],
            rows: vec![vec![s("5"), s("a")], vec![s("7"), s("b")]],
            ..Default::default()
        };
        let rel = |outgoing, table: &str, cols: &[(&str, &str)]| Relation {
            name: "fk".into(),
            outgoing,
            other_schema: "public".into(),
            other_table: table.into(),
            columns: cols.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            inferred: false,
            alternatives: Vec::new(),
        };
        // users ← posts.user_id: posts whose user_id is one of these ids.
        let posts = rel(false, "posts", &[("id", "user_id")]);
        assert_eq!(related_filter(&posts, &users, &[0]).as_deref(), Some(r#""user_id" = '5'"#));
        assert_eq!(related_filter(&posts, &users, &[0, 1]).as_deref(), Some(r#""user_id" in ('5', '7')"#));
        // posts.user_id → users, with a NULL user_id: nothing to open.
        let post_rows = ResultSet {
            columns: vec!["id".into(), "user_id".into()],
            rows: vec![vec![s("1"), None], vec![s("2"), s("7")]],
            ..Default::default()
        };
        let user = rel(true, "users", &[("user_id", "id")]);
        assert_eq!(related_filter(&user, &post_rows, &[0]), None);
        assert_eq!(related_filter(&user, &post_rows, &[0, 1]).as_deref(), Some(r#""id" = '7'"#));
        // A key column that isn't in the result.
        assert_eq!(related_filter(&rel(true, "x", &[("nope", "id")]), &users, &[0]), None);
        // An inferred link that may use either key matches both.
        let parcels = ResultSet {
            columns: vec!["id".into(), "land_right".into()],
            rows: vec![vec![s("p1"), s("{6726-AB}")]],
            ..Default::default()
        };
        let mut lr = rel(true, "land_right", &[("land_right", "id")]);
        lr.alternatives = vec![("land_right".into(), "globalid".into())];
        assert_eq!(related_filter(&lr, &parcels, &[0]).as_deref(), Some(r#"("id" = '{6726-AB}') or ("globalid" = '{6726-AB}')"#));
        // Counting: one probe per variant; following keeps the variants that matched.
        let Some(TabAction::Probe { probes, map, .. }) = probe_action(std::slice::from_ref(&lr), &parcels, vec![0], false) else { panic!("probe") };
        assert_eq!(map, [(0, 0), (0, 1)]);
        assert_eq!(probes[1], ("public".into(), "land_right".into(), r#""globalid" = '{6726-AB}'"#.into()));
        let probe = Probe { rows: vec![0], map, counts: vec![Some(0), Some(1)] };
        assert_eq!(probe.best(&[0], 1), [Some(1)]);
        assert_eq!(probe.matched(&[0], 0), [1]);
        assert!(probe.best(&[5], 1).is_empty(), "counts are for other rows");
        assert_eq!(related_filter_only(&lr, &parcels, &[0], &probe.matched(&[0], 0)).as_deref(), Some(r#""globalid" = '{6726-AB}'"#));
    }
}
