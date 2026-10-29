use super::*;

impl App {
    pub(super) fn apply(&mut self, action: Action) {
        match action {
            Action::NewConnection => {
                self.dialog = Some(Dialog::new(ConnectionConfig::default(), true));
            }
            Action::NewConnectionIn(folder) => {
                let cfg = ConnectionConfig { folder: Some(folder), ..ConnectionConfig::default() };
                self.dialog = Some(Dialog::new(cfg, true));
            }
            Action::Connect(ci) => {
                let c = &mut self.conns[ci];
                c.status = Status::Connecting;
                c.clear_cache();
                self.worker.send(Cmd::Connect(c.cfg.clone()));
            }
            Action::Disconnect(ci) => {
                let c = &mut self.conns[ci];
                self.worker.send(Cmd::Disconnect(c.cfg.id.clone()));
                c.status = Status::Disconnected;
                c.clear_cache();
            }
            Action::CopyUrlWithPassword(ci) => {
                self.egui_ctx.copy_text(rgrip_core::conn_string::to_url(&self.conns[ci].cfg, true));
            }
            Action::SetColor(ci, color) => {
                self.conns[ci].cfg.color = color;
                self.save_config();
            }
            Action::Edit(ci) => {
                self.dialog = Some(Dialog::new(self.conns[ci].cfg.clone(), false));
            }
            Action::Delete(ci) => {
                let id = self.conns[ci].cfg.id.clone();
                self.worker.send(Cmd::Disconnect(id.clone()));
                while let Some(i) = self.tabs.iter().position(|t| t.conn == id) {
                    self.close_tab(i);
                }
                self.conns.remove(ci);
                self.save_config();
                let _ = rgrip_core::history::clear(&id);
                let key = format!("c/{id}");
                if self.settings.pinned.contains(&key) {
                    self.settings.pinned.retain(|k| *k != key);
                    self.save_settings();
                }
            }
            Action::Refresh(ci) => {
                self.conns[ci].clear_cache();
                self.worker.send(Cmd::Catalog(self.conns[ci].cfg.id.clone(), None));
            }
            Action::NewConsole(ci) => self.open_console(ci),
            Action::ShowHistory(ci) => {
                let cfg = &self.conns[ci].cfg;
                self.history = Some(crate::history_view::HistoryView::new(cfg.id.clone(), cfg.display_name()));
            }
            Action::AddJump(i) => self.add_jump(i),
            Action::OpenJumpList => self.jump_view = Some(crate::jump_view::JumpView::new()),
            Action::OpenDiagram(ci, schema) => self.open_diagram(ci, schema),
            Action::OpenActivity(ci) => self.open_activity(ci),
            Action::OpenTable(ci, s, t) => self.open_table(ci, s, t),
            Action::OpenInConsole(ci, schema, table) => {
                self.open_console(ci);
                if let Some(TabKind::Console(c)) = self.tabs.last_mut().map(|t| &mut t.kind) {
                    c.sql = format!("SELECT *\nFROM {}.{}\nLIMIT 100;", sql::quote_ident(&schema), sql::quote_ident(&table));
                    c.schema = Some(schema);
                }
            }
            Action::Import(ci, schema, table) => {
                if !matches!(self.conns[ci].tables.get(&schema), Some(Load::Ready(_) | Load::Loading)) {
                    self.apply(Action::LoadTables(ci, schema.clone()));
                }
                self.import = Some(crate::import::ImportDialog::new(self.conns[ci].cfg.id.clone(), schema, table));
            }
            Action::EditTable(ci, schema, table) => {
                let cfg = &self.conns[ci].cfg;
                if cfg.read_only {
                    self.toast = Some((READ_ONLY_HINT.into(), theme::WARN(), Instant::now()));
                    return;
                }
                let id = cfg.id.clone();
                self.table_editor = Some(crate::table_edit::TableEditor::new(id.clone(), schema.clone(), table.clone(), cfg.color == ConnColor::Red));
                self.worker.send(Cmd::TableDef { conn: id, schema, table });
            }
            Action::NewLayout(ci) => {
                self.open_layout(ci, true);
            }
            Action::AddToLayout(ci, schema, table, column) => {
                let ti = self.open_layout(ci, false);
                if let TabKind::Layout(l) = &mut self.tabs[ti].kind {
                    let loads = l.add_layer(&schema, &table, &column);
                    self.tab_action(ti, TabAction::LoadLayers(loads));
                }
            }
            Action::LoadSchemas(ci) => {
                let c = &mut self.conns[ci];
                c.schemas = Some(Load::Loading);
                self.worker.send(Cmd::Schemas(c.cfg.id.clone(), None));
            }
            Action::LoadTables(ci, schema) => {
                let c = &mut self.conns[ci];
                c.tables.insert(schema.clone(), Load::Loading);
                self.worker.send(Cmd::Tables(c.cfg.id.clone(), schema));
            }
            Action::LoadColumns(ci, schema, table) => {
                let c = &mut self.conns[ci];
                c.columns.insert((schema.clone(), table.clone()), Load::Loading);
                self.worker.send(Cmd::Columns(c.cfg.id.clone(), schema, table));
            }
            Action::Toggle(key) if key.starts_with("fc/") || key.starts_with("fq/") => {
                let collapsed = &mut self.settings.collapsed_folders;
                match collapsed.iter().position(|k| *k == key) {
                    Some(i) => _ = collapsed.remove(i),
                    None => collapsed.push(key),
                }
                self.save_settings();
            }
            Action::TogglePin(key) => {
                let pinned = &mut self.settings.pinned;
                match pinned.iter().position(|k| *k == key) {
                    Some(i) => _ = pinned.remove(i),
                    None => pinned.push(key),
                }
                self.save_settings();
            }
            Action::Toggle(key) => {
                if !self.expanded.remove(&key) {
                    self.expanded.insert(key);
                }
            }
            Action::Select(key) => {
                self.selected = Some(key);
                self.explorer_active = true;
            }
            Action::ActivateTab(i) => self.active = i,
            Action::Close(which) => close_tabs(&mut self.tabs, &mut self.active, which),
            Action::OpenSaved(i) => self.open_saved(i),
            Action::SetTheme(choice) => {
                self.settings.theme = choice;
                theme::set_choice(&self.egui_ctx, choice);
                self.save_settings();
            }
            Action::SetPalette(dark, name) => {
                if dark {
                    self.settings.dark_theme = name.into();
                } else {
                    self.settings.light_theme = name.into();
                }
                theme::set_themes(&self.egui_ctx, &self.settings.dark_theme, &self.settings.light_theme);
                // Show the chosen theme right away.
                let wanted = if dark { ThemeChoice::Dark } else { ThemeChoice::Light };
                if self.settings.theme != ThemeChoice::System || theme::is_dark() != dark {
                    self.settings.theme = wanted;
                    theme::set_choice(&self.egui_ctx, wanted);
                }
                self.save_settings();
            }
            Action::SetAutoUpdate(on) => {
                self.settings.check_updates = on;
                self.save_settings();
            }
            Action::EditCommands => {
                self.commands_editor = Some(CommandsEditor { list: rgrip_core::commands::active().to_vec(), selected: 0, error: None, preview_args: String::new() });
            }
            Action::NewCommand(name) => {
                let mut list = rgrip_core::commands::active().to_vec();
                let name = if name.is_empty() { format!("cmd{}", list.len() + 1) } else { name.to_ascii_lowercase() };
                list.push(rgrip_core::commands::SlashCommand { name, help: String::new(), template: "select *\nfrom {table}\nwhere id in {ids};".into() });
                let selected = list.len() - 1;
                self.commands_editor = Some(CommandsEditor { list, selected, error: None, preview_args: String::new() });
            }
            Action::CheckUpdate => {
                if !matches!(self.update, UpdateState::Checking { .. } | UpdateState::Installing(_)) {
                    self.update = UpdateState::Checking { manual: true };
                    self.worker.send(Cmd::CheckUpdate);
                }
            }
            Action::InstallUpdate => {
                if let UpdateState::Available(release) = &self.update {
                    match update::current_bundle() {
                        Some(app) => {
                            let release = release.clone();
                            self.worker.send(Cmd::InstallUpdate { release: release.clone(), app });
                            self.update = UpdateState::Installing(release);
                        }
                        // Running from source: send the user to the release page.
                        None => self.egui_ctx.open_url(egui::OpenUrl::new_tab(&release.page_url)),
                    }
                }
            }
            Action::MoveConn(id, folder) => {
                if let Some(ci) = self.conn_index(&id) {
                    self.conns[ci].cfg.folder = folder;
                    self.save_config();
                }
            }
            Action::MoveSaved(id, folder) => {
                if let Some(q) = self.saved.iter().find(|q| q.id == id).cloned() {
                    let result = config::move_query(&q, folder.as_deref()).map(|r| self.relink(&q.id, Some((&r.id, &r.name))));
                    self.report(result);
                    self.reload_queries();
                }
            }
            Action::NewFolder(kind, item) => {
                self.name_prompt = Some(NamePrompt { name: String::new(), target: NameTarget::NewFolder(kind, item) });
            }
            Action::RenameFolder(kind, name) => {
                self.name_prompt = Some(NamePrompt { name: name.clone(), target: NameTarget::RenameFolder(kind, name) });
            }
            Action::DeleteFolder(kind, name) => {
                self.forget_folder(kind, &name);
                match kind {
                    FolderKind::Conns => {
                        for c in self.conns.iter_mut().filter(|c| c.cfg.folder.as_deref() == Some(name.as_str())) {
                            c.cfg.folder = None;
                        }
                        self.conn_folders.retain(|f| f != &name);
                        self.save_config();
                    }
                    FolderKind::Queries => {
                        let result = config::delete_query_folder(&name).map(|moved| {
                            for (old, q) in moved {
                                self.relink(&old, Some((&q.id, &q.name)));
                            }
                        });
                        self.report(result);
                        self.reload_queries();
                    }
                }
            }
            Action::RevealQueryFolder(name) => {
                if let Some(dir) = config::queries_dir() {
                    let result = config::reveal_in_finder(&dir.join(name));
                    self.report(result);
                }
            }
            Action::RenameSaved(i) => {
                if let Some(q) = self.saved.get(i) {
                    self.name_prompt = Some(NamePrompt { name: q.name.clone(), target: NameTarget::Rename(q.id.clone()) });
                }
            }
            Action::DeleteSaved(i) => {
                if let Some(q) = self.saved.get(i).cloned() {
                    let result = config::delete_query(&q);
                    if result.is_ok() {
                        self.relink(&q.id, None);
                        self.toast = Some((format!("Moved “{}” to the Trash", q.name), theme::MUTED(), Instant::now()));
                    }
                    self.report(result);
                    self.reload_queries();
                }
            }
            Action::RevealSaved(i) => {
                if let Some(path) = self.saved.get(i).map(|q| q.path.clone()) {
                    let result = config::reveal_in_finder(&path);
                    self.report(result);
                }
            }
            Action::OpenQueriesFolder => {
                if let Some(dir) = config::queries_dir() {
                    let _ = std::fs::create_dir_all(&dir);
                    self.egui_ctx.open_url(egui::OpenUrl::same_tab(format!("file://{}", dir.display())));
                }
            }
        }
    }
}
