use super::*;

/// Copy one column of `rows`: every value (one per line, NULL as `NULL`),
/// the distinct values, or the distinct values as an SQL list.
pub(super) fn copy_column_items(ui: &mut Ui, rs: &ResultSet, rows: &[usize], col: usize, label: &str) {
    if crate::icons::menu_item(ui, crate::icons::Svg::Text, label, true).clicked() {
        ui.ctx().copy_text(column_lines(rs, rows, col));
        ui.close();
    }
    if crate::icons::menu_item(ui, crate::icons::Svg::Copy, "Copy distinct values", true).clicked() {
        ui.ctx().copy_text(column_distinct(rs, rows, col).join("\n"));
        ui.close();
    }
    if crate::icons::menu_item(ui, crate::icons::Svg::Table, "Copy as IN ('a', 'b')", true)
        .on_hover_text("Distinct values, quoted, for  where col in ('a', 'b')")
        .clicked()
    {
        ui.ctx().copy_text(column_sql_list(rs, rows, col));
        ui.close();
    }
}

pub(super) fn column_lines(rs: &ResultSet, rows: &[usize], col: usize) -> String {
    rows.iter().filter_map(|&r| rs.rows.get(r)).map(|row| row[col].as_deref().unwrap_or("NULL")).collect::<Vec<_>>().join("\n")
}

/// Distinct non-NULL values, in first-seen order.
pub(super) fn column_distinct<'a>(rs: &'a ResultSet, rows: &[usize], col: usize) -> Vec<&'a str> {
    let mut seen = std::collections::HashSet::new();
    rows.iter().filter_map(|&r| rs.rows.get(r)?[col].as_deref()).filter(|v| seen.insert(*v)).collect()
}

/// `('a', 'b')`: always quoted, since Postgres reads a quoted literal as
/// the column's type, so it works for text and number columns alike.
pub(super) fn column_sql_list(rs: &ResultSet, rows: &[usize], col: usize) -> String {
    let items: Vec<String> = column_distinct(rs, rows, col).iter().map(|v| format!("'{}'", v.replace('\'', "''"))).collect();
    format!("({})", items.join(", "))
}

/// Copy / export actions for the given rows.
pub(super) fn rows_menu(ui: &mut Ui, rs: &ResultSet, rows: &[usize], source: &Source<'_>, geom: bool) -> Option<GridEvent> {
    use rgrip_core::export::{self, Format};
    let n = rows.len();
    let subset = || export::subset(rs, rows);
    let noun = if n == 1 { "row".to_string() } else { format!("{n} rows") };
    let mut event = None;
    if !source.relations.is_empty() {
        crate::icons::submenu(ui, crate::icons::Svg::GitBranch, "Related", |ui| {
            for (i, r) in source.relations.iter().enumerate() {
                let cols: Vec<&str> = r.columns.iter().map(|(here, there)| if r.outgoing { here.as_str() } else { there.as_str() }).collect();
                let other = if r.other_schema == "public" { r.other_table.clone() } else { format!("{}.{}", r.other_schema, r.other_table) };
                let (arrow, hover) = if r.outgoing { ("→", "Rows these rows reference") } else { ("←", "Rows that reference these rows") };
                let mut label = format!("{arrow} {other}  ({})", cols.join(", "));
                let mut hover = format!("{hover}\n{}", r.name);
                if r.inferred {
                    label.push_str("  ~ inferred");
                    hover.push_str("\nGuessed from column names (no foreign key constraint)");
                }
                // Counted matches for these rows; a guess with none is dimmed.
                let count = source.counts.get(i).copied().flatten();
                match count {
                    Some(n) if n >= crate::worker::PROBE_CAP as i64 => label.push_str(&format!("  · {}+", crate::worker::PROBE_CAP)),
                    Some(n) => label.push_str(&format!("  · {n}")),
                    None => {}
                }
                let text = if count == Some(0) { RichText::new(label).color(theme::FAINT()) } else { RichText::new(label) };
                if ui.button(text).on_hover_text(hover).clicked() {
                    event = Some(GridEvent::Follow { relation: i, rows: rows.to_vec() });
                    ui.close();
                }
            }
        });
        ui.separator();
    }
    if crate::icons::menu_item(ui, crate::icons::Svg::Copy, format!("Copy {noun}"), true).on_hover_text("Tab-separated  ⌘C").clicked() {
        ui.ctx().copy_text(rows_tsv(rs, rows, false));
        ui.close();
    }
    if crate::icons::menu_item(ui, crate::icons::Svg::Text, "Copy with header", true).clicked() {
        ui.ctx().copy_text(rows_tsv(rs, rows, true));
        ui.close();
    }
    if crate::icons::menu_item(ui, crate::icons::Svg::Image, "Copy as image", true).on_hover_text("PNG of the table: paste into Telegram, Slack…").clicked() {
        crate::snapshot::copy_table(ui.ctx(), &subset(), source.table);
        ui.close();
    }
    crate::icons::submenu(ui, crate::icons::Svg::Copy, "Copy as", |ui| {
        let item = |ui: &mut Ui, label: &str, text: &dyn Fn() -> String| {
            if ui.button(label).clicked() {
                ui.ctx().copy_text(text());
                ui.close();
            }
        };
        item(ui, "CSV", &|| export::export(&subset(), Format::Csv, source.table).trim_start_matches('\u{FEFF}').to_string());
        item(ui, "JSON", &|| export::export(&subset(), Format::Json, source.table));
        item(ui, "Markdown", &|| export::export(&subset(), Format::Markdown, source.table));
        if geom {
            item(ui, "GeoJSON", &|| export::export(&subset(), Format::GeoJson, source.table));
        }
        ui.separator();
        item(ui, "SQL INSERT", &|| export::export(&subset(), Format::SqlInsert, source.table));
        item(ui, "SQL  select … where id in (…)", &|| export::where_in(&subset(), source.table));
    });
    if crate::icons::menu_item(ui, crate::icons::Svg::Bot, "Copy for AI agent", true)
        .on_hover_text("Markdown with the query, column types and rows: paste into an agent")
        .clicked()
    {
        ui.ctx().copy_text(export::for_agent(&subset(), source.table, source.sql));
        ui.close();
    }
    ui.separator();
    crate::icons::submenu(ui, crate::icons::Svg::Download, &format!("Save {noun} as"), |ui| {
        for f in Format::ALL.into_iter().filter(|f| *f != Format::GeoJson || geom) {
            if ui.button(format!("{}…", f.label())).clicked() {
                ui.close();
                let name = source.table.rsplit('.').next().unwrap_or("rows");
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name(format!("{name}.{}", f.extension()))
                    .add_filter(f.label(), &[f.extension()])
                    .save_file()
                {
                    let _ = std::fs::write(path, export::export(&subset(), f, source.table));
                }
            }
        }
    });
    event
}

pub(super) fn rows_tsv(rs: &ResultSet, rows: &[usize], header: bool) -> String {
    let mut lines: Vec<String> = Vec::new();
    if header {
        lines.push(rs.columns.join("\t"));
    }
    lines.extend(rows.iter().filter_map(|&i| rs.rows.get(i)).map(|r| row_tsv(r)));
    lines.join("\n")
}
