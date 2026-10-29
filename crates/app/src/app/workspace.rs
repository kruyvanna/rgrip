use super::*;

impl App {
    pub(super) fn open_console(&mut self, ci: usize) {
        let conn = self.conns[ci].cfg.id.clone();
        let n = self
            .tabs
            .iter()
            .filter(|t| t.conn == conn && matches!(t.kind, TabKind::Console(_)))
            .count();
        let cfg = &self.conns[ci].cfg;
        let name = conn_db_title(cfg, &cfg.database);
        let title = if n == 0 { name } else { format!("{name} ({})", n + 1) };
        self.push_tab(
            conn,
            title,
            TabKind::Console(Console::new()),
        );
        self.ensure_connected(ci);
    }

    pub(super) fn open_table(&mut self, ci: usize, schema: String, table: String) {
        self.open_table_with(ci, schema, table, None);
    }

    /// Open (or switch to) a table tab showing only the rows matching `filter`.
    pub(super) fn open_table_filtered(&mut self, ci: usize, schema: String, table: String, filter: String) {
        self.open_table_with(ci, schema, table, Some(filter));
    }

    fn open_table_with(&mut self, ci: usize, schema: String, table: String, filter: Option<String>) {
        let conn = self.conns[ci].cfg.id.clone();
        if let Some(i) = self.tabs.iter().position(|t| {
            t.conn == conn && matches!(&t.kind, TabKind::Table(v) if v.schema == schema && v.table == table)
        }) {
            self.active = i;
            if let (Some(filter), TabKind::Table(t)) = (filter, &mut self.tabs[i].kind) {
                if t.dirty() {
                    self.toast = Some(("Save or discard your changes in this tab first".into(), theme::WARN(), Instant::now()));
                    return;
                }
                t.filter = filter.clone();
                t.applied_filter = filter;
                t.page = 0;
                self.tab_action(i, TabAction::Reload);
            }
            return;
        }
        let filter = filter.unwrap_or_default();
        let title = if schema == "public" { table.clone() } else { format!("{schema}.{table}") };
        self.push_tab(
            conn,
            title,
            TabKind::Table(TableView::new(schema, table, filter)),
        );
        self.tab_action(self.active, TabAction::Reload);
    }

    /// Switch to the connection's map layout tab (the last one), or open a
    /// new one when there's none or `new` is set. Returns its index.
    pub(super) fn open_layout(&mut self, ci: usize, new: bool) -> usize {
        let conn = self.conns[ci].cfg.id.clone();
        if !new
            && let Some(i) = self.tabs.iter().rposition(|t| t.conn == conn && matches!(t.kind, TabKind::Layout(_)))
        {
            self.active = i;
            return i;
        }
        let n = self.tabs.iter().filter(|t| t.conn == conn && matches!(t.kind, TabKind::Layout(_))).count();
        let title = if n == 0 { "Map layout".to_string() } else { format!("Map layout ({})", n + 1) };
        self.push_tab(conn, title, TabKind::Layout(Box::new(crate::layout_tab::LayoutTab::new(&self.egui_ctx))));
        self.ensure_connected(ci);
        self.tabs.len() - 1
    }

    /// Switch to the diagram of `schema`, or open it and ask for its data.
    pub(super) fn open_diagram(&mut self, ci: usize, schema: String) {
        let conn = self.conns[ci].cfg.id.clone();
        if let Some(i) = self.tabs.iter().position(|t| t.conn == conn && matches!(&t.kind, TabKind::Diagram(d) if d.schema == schema)) {
            self.active = i;
            return;
        }
        let cfg = &self.conns[ci].cfg;
        let key = format!("{}/{}/{schema}", cfg.id, cfg.database);
        let mut tab = crate::diagram_tab::DiagramTab::new(&schema, key);
        if self.ensure_connected(ci) {
            tab.state = crate::diagram_tab::State::Loading;
            self.worker.send(Cmd::Diagram { conn: conn.clone(), schema: schema.clone() });
        }
        self.push_tab(conn, format!("Diagram · {schema}"), TabKind::Diagram(Box::new(tab)));
    }

    /// Switch to the connection's Activity tab, or open it (it loads itself).
    pub(super) fn open_activity(&mut self, ci: usize) {
        let conn = self.conns[ci].cfg.id.clone();
        if let Some(i) = self.tabs.iter().position(|t| t.conn == conn && matches!(t.kind, TabKind::Activity(_))) {
            self.active = i;
            return;
        }
        let title = format!("Activity · {}", self.conns[ci].cfg.display_name());
        self.push_tab(conn, title, TabKind::Activity(Box::new(crate::activity_tab::ActivityTab::new())));
        self.ensure_connected(ci);
    }

    fn push_tab(&mut self, conn: ConnId, title: String, kind: TabKind) {
        self.tabs.push(Tab { id: self.next_tab, conn, title, kind });
        self.next_tab += 1;
        self.active = self.tabs.len() - 1;
    }

    pub(super) fn close_tab(&mut self, i: usize) {
        close_tabs(&mut self.tabs, &mut self.active, CloseWhich::One(i));
    }

    /// Point consoles linked to saved query `old` at `new` (None = unlink).
    pub(super) fn relink(&mut self, old: &str, new: Option<(&str, &str)>) {
        for tab in &mut self.tabs {
            if let TabKind::Console(c) = &mut tab.kind {
                if c.saved_id.as_deref() == Some(old) {
                    c.saved_id = new.map(|(id, _)| id.to_string());
                    if let Some((_, name)) = new {
                        tab.title = name.to_string();
                    }
                }
            }
        }
    }

    /// Rename a folder, keeping it open or closed and consoles linked.
    pub(super) fn rename_folder(&mut self, kind: FolderKind, old: &str, new: &str) -> Result<String, String> {
        let new = match kind {
            FolderKind::Conns => {
                for c in self.conns.iter_mut().filter(|c| c.cfg.folder.as_deref() == Some(old)) {
                    c.cfg.folder = Some(new.to_string());
                }
                self.conn_folders.retain(|f| f != old);
                self.conn_folders.push(new.to_string());
                self.save_config();
                new.to_string()
            }
            FolderKind::Queries => {
                let new = config::rename_query_folder(old, new)?;
                for q in self.saved.clone().iter().filter(|q| q.folder.as_deref() == Some(old)) {
                    let id = format!("{new}/{}", q.name);
                    self.relink(&q.id, Some((&id, &q.name)));
                }
                new
            }
        };
        let key = kind.key(&new);
        let (collapsed, pinned) = self.forget_folder(kind, old);
        for (had, list) in [(collapsed, &mut self.settings.collapsed_folders), (pinned, &mut self.settings.pinned)] {
            if had && !list.contains(&key) {
                list.push(key.clone());
            }
        }
        if collapsed || pinned {
            self.save_settings();
        }
        Ok(new)
    }

    /// Drop a folder's collapsed and pinned state (deleted or renamed
    /// away); returns whether it was (collapsed, pinned).
    pub(super) fn forget_folder(&mut self, kind: FolderKind, name: &str) -> (bool, bool) {
        let key = kind.key(name);
        let drop = |list: &mut Vec<String>| {
            let before = list.len();
            list.retain(|k| *k != key);
            list.len() != before
        };
        let was = (drop(&mut self.settings.collapsed_folders), drop(&mut self.settings.pinned));
        if was.0 || was.1 {
            self.save_settings();
        }
        was
    }

    pub(super) fn open_saved(&mut self, i: usize) {
        let Some(q) = self.saved.get(i).cloned() else { return };
        let linked = self.tabs.iter().position(|t| matches!(&t.kind, TabKind::Console(c) if c.saved_id.as_ref() == Some(&q.id)));
        if let Some(t) = linked {
            self.active = t;
            return;
        }
        let current = self.tabs.get(self.active).map(|t| t.conn.clone());
        let ci = q
            .connection
            .as_deref()
            .and_then(|id| self.conn_index(id))
            .or_else(|| current.and_then(|id| self.conn_index(&id)))
            .unwrap_or(0);
        if self.conns.is_empty() {
            return;
        }
        self.open_console(ci);
        if let Some(tab) = self.tabs.last_mut() {
            tab.title = q.name.clone();
            if let TabKind::Console(c) = &mut tab.kind {
                c.sql = q.sql.clone();
                c.saved_id = Some(q.id.clone());
                c.database = q.database.clone();
                let mut schemas = q.schemas.clone();
                c.schema = (!schemas.is_empty()).then(|| schemas.remove(0));
                c.more_schemas = schemas;
            }
        }
    }
}
