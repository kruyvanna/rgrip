use super::*;

// ---- saved query name prompt -------------------------------------------

/// `query` wrapped to add `col` reprojected to WGS84 as `alias`.
pub(super) fn reproject_sql(query: &str, col: &str, alias: &str) -> String {
    let id = |s: &str| if complete::needs_quotes(s) { sql::quote_ident(s) } else { s.to_string() };
    let body = query.trim().trim_end_matches(';').trim_end();
    format!("select q.*, ST_Transform(q.{}, 4326) as {}\nfrom (\n{body}\n) q;", id(col), id(alias))
}

/// First meaningful line of SQL, shortened, as a default query name.
pub(super) fn default_query_name(sql: &str, n: usize) -> String {
    let line = sql
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("--"))
        .unwrap_or_default();
    let line: String = line.chars().take(40).collect();
    if line.is_empty() { format!("Query {n}") } else { line }
}

impl App {
    /// The import dialog, fed the schema's tables and, when appending, the
    /// target's columns.
    pub(super) fn import_ui(&mut self, ctx: &egui::Context) {
        use crate::import::Outcome;
        let Some(d) = &mut self.import else { return };
        let Some(ci) = self.conns.iter().position(|c| c.cfg.id == d.conn) else {
            self.import = None;
            return;
        };
        let conn = &self.conns[ci];
        let tables = match conn.tables.get(&d.schema) {
            Some(Load::Ready(t)) => t.iter().filter(|t| matches!(t.kind, TableKind::Table | TableKind::Partitioned)).map(|t| t.name.clone()).collect(),
            _ => Vec::new(),
        };
        let target = match conn.columns.get(&(d.schema.clone(), d.table().to_string())) {
            Some(Load::Ready(cols)) => Some(cols.as_slice()),
            _ => None,
        };
        let (id, schema) = (d.conn.clone(), d.schema.clone());
        match d.ui(ctx, tables, target) {
            Outcome::Keep => {}
            Outcome::Close => self.import = None,
            Outcome::Start(request) => self.worker.send(Cmd::Import { conn: id, request }),
            Outcome::NeedColumns(table) => self.apply(Action::LoadColumns(ci, schema, table)),
            Outcome::OpenTable(table) => {
                self.import = None;
                self.open_table(ci, schema, table);
            }
        }
    }

    pub(super) fn history_ui(&mut self, ctx: &egui::Context) {
        use crate::history_view::Outcome;
        let Some(h) = &mut self.history else { return };
        match h.ui(ctx) {
            Outcome::Keep => {}
            Outcome::Close => self.history = None,
            Outcome::Clear => {
                let r = rgrip_core::history::clear(&h.conn);
                h.cleared();
                self.report(r);
            }
            Outcome::Open(e) => {
                self.history = None;
                if let Some(ci) = self.conn_index(&e.conn) {
                    self.open_console(ci);
                    if let Some(TabKind::Console(c)) = self.tabs.last_mut().map(|t| &mut t.kind) {
                        c.sql = e.sql;
                        c.database = e.database;
                    }
                }
            }
        }
    }

    pub(super) fn table_editor_ui(&mut self, ctx: &egui::Context) {
        use crate::table_edit::Outcome;
        let Some(ed) = &mut self.table_editor else { return };
        match ed.ui(ctx) {
            Outcome::Keep => {}
            Outcome::Close => self.table_editor = None,
            Outcome::Apply { statements, new_name } => {
                let (conn, schema, table) = (ed.conn.clone(), ed.schema.clone(), ed.table.clone());
                self.worker.send(Cmd::AlterTable { conn, schema, table, new_name, statements });
            }
        }
    }

    /// Settings → Slash commands: list on the left, the selected command's
    /// name, description and SQL template on the right, with a live preview.
    pub(super) fn commands_editor_ui(&mut self, ctx: &egui::Context) {
        use rgrip_core::commands::{self, SlashCommand};
        const LIST_W: f32 = 190.0;
        const GAP: f32 = 10.0;
        /// Height of the template and preview panes.
        const PANE_H: f32 = 380.0;
        let field_label = |ui: &mut Ui, text: &str| {
            ui.label(RichText::new(text).size(12.0).color(theme::MUTED()));
            ui.add_space(2.0);
        };
        let Some(ed) = &mut self.commands_editor else { return };
        // Preview against the active tab's connection, if it has a catalog.
        let catalog = self.tabs.get(self.active).and_then(|t| self.conns.iter().find(|c| c.cfg.id == t.conn)).map(|c| c.catalog.clone()).unwrap_or_default();
        let (mut save, mut close, mut reset) = (false, false, false);
        let modal = egui::Modal::new(Id::new("commands-editor"))
            .frame(Frame::new().fill(theme::SURFACE()).stroke(Stroke::new(1.0_f32, theme::BORDER())).corner_radius(12).inner_margin(Margin::same(20)))
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 80.0).clamp(760.0, 1180.0));
                ui.label(RichText::new("Slash commands").heading().color(theme::TEXT()));
                ui.label(RichText::new("Type /name in the editor, then Tab to expand (or run it directly).").size(12.5).color(theme::MUTED()));
                ui.add_space(12.0);
                ui.horizontal_top(|ui| {
                    // List: a heading level with "Name", left-aligned rows.
                    ui.vertical(|ui| {
                        ui.set_width(LIST_W);
                        field_label(ui, "Commands");
                        egui::ScrollArea::vertical().id_salt("cmd-list").max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (i, c) in ed.list.iter().enumerate() {
                                let label = if c.name.is_empty() { "(unnamed)".to_string() } else { format!("/{}", c.name) };
                                if list_row(ui, &label, i == ed.selected, LIST_W).clicked() {
                                    ed.selected = i;
                                }
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("+ Add").clicked() {
                                ed.list.push(SlashCommand { name: format!("cmd{}", ed.list.len() + 1), help: String::new(), template: "select *\nfrom {table}\nlimit {n=50};".into() });
                                ed.selected = ed.list.len() - 1;
                            }
                            if ui.add_enabled(!ed.list.is_empty(), Button::new("Delete")).clicked() {
                                ed.list.remove(ed.selected);
                                ed.selected = ed.selected.min(ed.list.len().saturating_sub(1));
                            }
                        });
                    });
                    // Not a separator: in a horizontal row it stretches to
                    // the full available height and blows up the modal.
                    ui.add_space(24.0);
                    // Details: name and description on one row, then the
                    // template and its preview side by side.
                    ui.vertical(|ui| {
                        let width = ui.available_width();
                        let Some(c) = ed.list.get_mut(ed.selected) else {
                            muted(ui, "No commands. Add one, or reset to the defaults.");
                            return;
                        };
                        let half = (width - 2.0 * GAP) / 2.0;
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0 * GAP;
                            ui.vertical(|ui| {
                                ui.set_width(half);
                                field_label(ui, "Name");
                                ui.add(TextEdit::singleline(&mut c.name).font(TextStyle::Monospace).desired_width(half).margin(Margin::symmetric(8, 6)));
                            });
                            ui.vertical(|ui| {
                                ui.set_width(half);
                                field_label(ui, "Description");
                                ui.add(TextEdit::singleline(&mut c.help).desired_width(half).margin(Margin::symmetric(8, 6)));
                            });
                        });
                        ui.add_space(GAP);
                        let line = format!("/{} {}", c.name, ed.preview_args);
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0 * GAP;
                            // Left: the template.
                            ui.vertical(|ui| {
                                ui.set_width(half);
                                field_label(ui, "SQL template");
                                let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                                    let mut job = highlight::sql(buf.as_str(), TextStyle::Monospace.resolve(ui.style()));
                                    job.wrap.max_width = wrap;
                                    ui.fonts_mut(|f| f.layout_job(job))
                                };
                                egui::ScrollArea::vertical().id_salt("cmd-template").max_height(PANE_H).auto_shrink([false, true]).show(ui, |ui| {
                                    ui.add(
                                        TextEdit::multiline(&mut c.template)
                                            .code_editor()
                                            .desired_rows(18)
                                            .desired_width(half)
                                            .margin(Margin::symmetric(8, 6))
                                            .layouter(&mut layouter),
                                    );
                                });
                                ui.add_space(6.0);
                                // Placeholders as chips that wrap cleanly.
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                                    for (token, what) in [
                                        ("{name}", "Your own argument, any name: pasted in as typed. Arguments go in the order the placeholders appear; the last one takes the rest of the line"),
                                        ("{name=x}", "Your own argument with a default"),
                                        ("{table}", "The table argument (schema-qualified if needed)"),
                                        ("{field}", "Geometry column: the argument, else the table's geometry column, else shape"),
                                        ("{field+_4326}", "The field name plus a suffix, quoted as needed (for aliases)"),
                                        ("{n}", "A number argument (required)"),
                                        ("{n=100}", "A number argument with a default"),
                                    ] {
                                        Frame::new().fill(theme::HOVER()).corner_radius(5).inner_margin(Margin::symmetric(6, 2)).show(ui, |ui| {
                                            ui.label(RichText::new(token).monospace().size(11.5).color(theme::MUTED()));
                                        })
                                        .response
                                        .on_hover_text(what);
                                    }
                                });
                            });
                            // Right: try it with arguments, see the SQL.
                            ui.vertical(|ui| {
                                ui.set_width(half);
                                field_label(ui, "Preview");
                                Frame::new().fill(theme::BG()).stroke(Stroke::new(1.0_f32, theme::BORDER())).corner_radius(8).inner_margin(Margin::symmetric(8, 5)).show(ui, |ui| {
                                    ui.set_width(half - 18.0);
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        ui.label(RichText::new(format!("/{} ", c.name)).monospace().color(theme::MUTED()));
                                        ui.add(
                                            TextEdit::singleline(&mut ed.preview_args)
                                                .hint_text(hint(&c.args()))
                                                .font(TextStyle::Monospace)
                                                .frame(false)
                                                .margin(Margin::ZERO)
                                                .desired_width(f32::INFINITY),
                                        );
                                    });
                                });
                                let preview = match c.problem() {
                                    Some(p) => Err(p),
                                    None => commands::preview_with(std::slice::from_ref(c), &line, &catalog, "public"),
                                };
                                ui.add_space(6.0);
                                Frame::new().fill(theme::BG()).corner_radius(8).inner_margin(Margin::same(10)).show(ui, |ui| {
                                    ui.set_width(half - 20.0);
                                    egui::ScrollArea::vertical().id_salt("cmd-preview").max_height(PANE_H - 44.0).auto_shrink([false, false]).show(ui, |ui| {
                                        match &preview {
                                            Ok(sql) => {
                                                let mut job = highlight::sql(sql, TextStyle::Monospace.resolve(ui.style()));
                                                job.wrap.max_width = ui.available_width();
                                                ui.add(egui::Label::new(job).selectable(true));
                                            }
                                            Err(e) => {
                                                ui.label(RichText::new(e).monospace().size(12.5).color(theme::WARN()));
                                            }
                                        }
                                    });
                                });
                                // Free placeholders still waiting for a value.
                                if let Ok(sql) = &preview {
                                    let missing: Vec<String> = c.args().split_whitespace().filter(|a| a.starts_with('<')).map(|a| a.trim_matches(['<', '>']).to_string()).filter(|n| sql.contains(&format!("{{{n}}}"))).collect();
                                    if !missing.is_empty() {
                                        ui.add_space(4.0);
                                        ui.label(RichText::new(format!("Waiting for: {}", missing.join(", "))).size(12.0).color(theme::WARN()));
                                    }
                                }
                            });
                        });
                    });
                });
                if let Some(e) = &ed.error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(e).color(theme::ERROR()));
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset to defaults").on_hover_text("Restore the built-in commands").clicked() {
                        reset = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(Button::new(RichText::new("Save").color(theme::BG())).fill(theme::ACCENT())).clicked() {
                            save = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            });
        if reset {
            ed.list = commands::defaults();
            ed.selected = 0;
            ed.error = None;
        }
        if save {
            let mut seen = HashSet::new();
            let problem = ed.list.iter().find_map(|c| {
                c.problem().or_else(|| (!seen.insert(c.name.clone())).then(|| format!("/{} is defined twice", c.name)))
            });
            ed.error = problem;
            if ed.error.is_none() {
                // Saving exactly the defaults forgets the file, so later
                // versions' new built-ins still show up.
                let result = if ed.list == commands::defaults() { config::reset_commands() } else { config::save_commands(&ed.list) };
                match result {
                    Ok(()) => {
                        commands::set_active(ed.list.clone());
                        self.toast = Some(("Slash commands saved".into(), theme::OK(), Instant::now()));
                        close = true;
                    }
                    Err(e) => ed.error = Some(format!("Could not save: {e}")),
                }
            }
        }
        if close || modal.should_close() {
            self.commands_editor = None;
        }
    }

    /// Run or save what the confirmation was asking about.
    pub(super) fn accept_confirm(&mut self) {
        let Some(c) = self.confirm.take() else { return };
        let Some(ti) = self.tabs.iter().position(|t| t.id == c.tab) else { return };
        self.skip_confirm = true;
        self.tab_action(ti, match c.kind {
            ConfirmKind::Run { all } => TabAction::Run { all },
            ConfirmKind::Explain { analyze } => TabAction::Explain { analyze },
            ConfirmKind::Save => TabAction::SaveEdits,
            ConfirmKind::AddKey => TabAction::AddKey,
            ConfirmKind::Delete(rows) => TabAction::DeleteRows(rows),
            ConfirmKind::Backend(pid, terminate) => TabAction::SignalBackend(pid, terminate),
        });
        self.skip_confirm = false;
    }

    pub(super) fn confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(c) = &mut self.confirm else { return };
        match confirm_modal(ctx, c) {
            Some(true) => self.accept_confirm(),
            Some(false) => self.confirm = None,
            None => {}
        }
    }
}

/// "Run on <connection>?" with the risky statements and why. `Some(true)`:
/// run anyway; `Some(false)`: cancelled. Cancel has the focus, so a stray
/// Enter doesn't run anything.
pub(super) fn confirm_modal(ctx: &egui::Context, c: &mut Confirm) -> Option<bool> {
    {
        let (mut go, mut cancel) = (false, false);
        let modal = egui::Modal::new(Id::new("risk-confirm"))
            .frame(
                Frame::new()
                    .fill(theme::SURFACE())
                    .stroke(Stroke::new(1.0_f32, theme::BORDER()))
                    .corner_radius(12)
                    .inner_margin(Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.set_width(480.0);
                ui.label(RichText::new(&c.title).heading().color(theme::TEXT()));
                ui.add_space(10.0);
                egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (sql, why, danger) in &c.items {
                        ui.label(RichText::new(why).size(12.0).strong().color(if *danger { theme::ERROR() } else { theme::WARN() }));
                        // Long statements are cut: the reason is what matters here.
                        let shown: String = sql.lines().take(4).collect::<Vec<_>>().join("\n");
                        let cut = shown.chars().count() > 300 || sql.lines().count() > 4;
                        let shown = if cut { format!("{}…", shown.chars().take(300).collect::<String>()) } else { shown };
                        ui.add(egui::Label::new(RichText::new(shown).monospace().size(12.0).color(theme::MUTED())).wrap());
                        ui.add_space(8.0);
                    }
                });
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let danger = c.items.iter().any(|i| i.2);
                    let fill = if danger { theme::ERROR() } else { theme::ACCENT() };
                    let label = match c.kind {
                        ConfirmKind::Delete(_) => "Delete  ⌘↩",
                        ConfirmKind::Backend(_, true) => "Terminate  ⌘↩",
                        ConfirmKind::Backend(_, false) => "Cancel query  ⌘↩",
                        _ => "Run anyway  ⌘↩",
                    };
                    if ui.add(Button::new(RichText::new(label).color(theme::BG())).fill(fill)).clicked() {
                        go = true;
                    }
                    let cancel_button = ui.button("Cancel");
                    if !c.focused {
                        cancel_button.request_focus();
                        c.focused = true;
                    }
                    if cancel_button.clicked() {
                        cancel = true;
                    }
                });
            });
        if go {
            Some(true)
        } else if cancel || modal.should_close() {
            Some(false)
        } else {
            None
        }
    }
}

impl App {
    pub(super) fn name_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &mut self.name_prompt else { return };
        let title = match prompt.target {
            NameTarget::NewFrom(_) => "Save query",
            NameTarget::Rename(_) => "Rename query",
            NameTarget::NewFolder(..) => "New folder",
            NameTarget::RenameFolder(..) => "Rename folder",
        };
        let mut done = false;
        let mut cancel = false;
        let modal = egui::Modal::new(Id::new("name-prompt"))
            .frame(
                Frame::new()
                    .fill(theme::SURFACE())
                    .stroke(Stroke::new(1.0_f32, theme::BORDER()))
                    .corner_radius(12)
                    .inner_margin(Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(RichText::new(title).heading().color(theme::TEXT()));
                ui.add_space(12.0);
                let edit = ui.add(
                    TextEdit::singleline(&mut prompt.name)
                        .hint_text(hint("Name"))
                        .desired_width(340.0 - 16.0)
                        .margin(Margin::symmetric(8, 6)),
                );
                if !edit.has_focus() && !edit.lost_focus() {
                    edit.request_focus();
                }
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    done = true;
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(Button::new(RichText::new("Save").color(theme::BG())).fill(theme::ACCENT())).clicked() {
                            done = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            });

        let name = prompt.name.trim().to_string();
        if done && !name.is_empty() {
            let Some(prompt) = self.name_prompt.take() else { return };
            let result = match prompt.target {
                NameTarget::NewFrom(tab_id) => {
                    let tab = self.tabs.iter_mut().find(|t| t.id == tab_id);
                    match tab.and_then(|t| match &mut t.kind {
                        TabKind::Console(c) => Some((&mut t.title, c, t.conn.clone())),
                        TabKind::Table(_) | TabKind::Layout(_) | TabKind::Diagram(_) | TabKind::Activity(_) => None,
                    }) {
                        Some((title, c, conn)) => config::create_query(&name, &c.sql, Some(conn), c.database.clone(), c.picked_schemas()).map(|q| {
                            c.saved_id = Some(q.id.clone());
                            *title = q.name.clone();
                            format!("Saved “{}”", q.name)
                        }),
                        None => Err("The console was closed".into()),
                    }
                }
                NameTarget::Rename(id) => match self.saved.iter().find(|q| q.id == id).cloned() {
                    Some(q) => config::rename_query(&q, &name).map(|r| {
                        self.relink(&id, Some((&r.id, &r.name)));
                        format!("Saved “{}”", r.name)
                    }),
                    None => Err("The query no longer exists".into()),
                },
                NameTarget::NewFolder(FolderKind::Conns, item) => {
                    self.conn_folders.push(name.clone());
                    match item {
                        Some(id) => {
                            self.apply(Action::MoveConn(id, Some(name.clone())));
                            Ok(format!("Moved to “{name}”"))
                        }
                        None => {
                            self.save_config();
                            Ok(format!("Created “{name}”"))
                        }
                    }
                }
                NameTarget::NewFolder(FolderKind::Queries, item) => config::create_query_folder(&name).map(|folder| {
                    match item {
                        Some(id) => {
                            self.apply(Action::MoveSaved(id, Some(folder.clone())));
                            format!("Moved to “{folder}”")
                        }
                        None => format!("Created “{folder}”"),
                    }
                }),
                NameTarget::RenameFolder(kind, old) => self.rename_folder(kind, &old, &name).map(|new| format!("Renamed to “{new}”")),
            };
            match result {
                Ok(message) => self.toast = Some((message, theme::OK(), Instant::now())),
                Err(e) => self.toast = Some((e, theme::ERROR(), Instant::now())),
            }
            self.reload_queries();
        } else if cancel || modal.should_close() {
            self.name_prompt = None;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn confirm_frame(ctx: &egui::Context, c: &mut Confirm, events: Vec<egui::Event>) -> Option<bool> {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), events, ..Default::default() };
        let mut out = None;
        let _ = ctx.run(input, |ctx| out = confirm_modal(ctx, c));
        out
    }

    #[test]
    fn confirm_modal_cancels_with_escape() {
        let ctx = egui::Context::default();
        let items = risky_items("delete from t", false);
        let mut c = Confirm { tab: 1, kind: ConfirmKind::Run { all: false }, title: "Run on prod?".into(), items, focused: false };
        assert_eq!(confirm_frame(&ctx, &mut c, vec![]), None);
        assert_eq!(confirm_frame(&ctx, &mut c, vec![]), None, "waits for a choice");
        let esc = egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        assert_eq!(confirm_frame(&ctx, &mut c, vec![esc]), Some(false));
    }

    #[test]
    fn reprojection_query() {
        assert_eq!(
            reproject_sql("select * from parcels where id < 10;\n", "geom", "geom_4326"),
            "select q.*, ST_Transform(q.geom, 4326) as geom_4326\nfrom (\nselect * from parcels where id < 10\n) q;"
        );
        assert!(reproject_sql("select 1", "Geom Col", "Geom Col_4326").contains("q.\"Geom Col\""));
    }

    #[test]
    fn query_names() {
        assert_eq!(default_query_name("-- note\n\n  select * from users where id = 1 and name like 'x%'", 1), "select * from users where id = 1 and nam");
        assert_eq!(default_query_name("  \n-- only a comment", 3), "Query 3");
    }
}
