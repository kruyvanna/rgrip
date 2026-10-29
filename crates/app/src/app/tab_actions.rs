use super::*;

impl App {
    pub(super) fn tab_action(&mut self, ti: usize, action: TabAction) {
        let Some(ci) = self.conn_index(&self.tabs[ti].conn) else { return };
        if let TabAction::OpenTable { schema, table } = action {
            return self.open_table(ci, schema, table);
        }
        if let TabAction::ReloadDiagram = action {
            let conn = self.tabs[ti].conn.clone();
            let TabKind::Diagram(d) = &mut self.tabs[ti].kind else { return };
            let schema = d.schema.clone();
            if matches!(self.conns[ci].status, Status::Connected(_)) {
                d.state = crate::diagram_tab::State::Loading;
                self.worker.send(Cmd::Diagram { conn, schema });
            } else {
                d.state = crate::diagram_tab::State::Pending;
                self.ensure_connected(ci);
            }
            return;
        }
        if let TabAction::RefreshActivity = action {
            let (conn, tab) = (self.tabs[ti].conn.clone(), self.tabs[ti].id);
            let TabKind::Activity(a) = &mut self.tabs[ti].kind else { return };
            a.requested();
            if matches!(self.conns[ci].status, Status::Connected(_)) {
                self.worker.send(Cmd::Activity { conn, tab });
            } else {
                a.pending = true;
                self.ensure_connected(ci);
            }
            return;
        }
        if let TabAction::ConfirmBackend(pid, terminate) = action {
            if self.confirm.is_some() {
                return;
            }
            if self.conns[ci].cfg.read_only {
                self.toast = Some((READ_ONLY_HINT.into(), theme::WARN(), Instant::now()));
                return;
            }
            let tab = self.tabs[ti].id;
            let TabKind::Activity(a) = &self.tabs[ti].kind else { return };
            let (user, query) = a.describe(pid).unwrap_or(("", ""));
            let who = if user.is_empty() { String::new() } else { format!(" ({user})") };
            let (title, why) = if terminate {
                (format!("Terminate session {pid}{who}?"), "Disconnects the session and rolls back its open transaction")
            } else {
                (format!("Cancel the query of session {pid}{who}?"), "Stops the running query; the session stays connected")
            };
            let query = if query.trim().is_empty() { "(no query text)".to_string() } else { query.to_string() };
            self.confirm = Some(Confirm { tab, kind: ConfirmKind::Backend(pid, terminate), title, items: vec![(query, why.to_string(), terminate)], focused: false });
            return;
        }
        if let TabAction::SignalBackend(pid, terminate) = action {
            if self.conns[ci].cfg.read_only {
                return;
            }
            let (conn, tab) = (self.tabs[ti].conn.clone(), self.tabs[ti].id);
            self.worker.send(Cmd::BackendSignal { conn, tab, pid, terminate });
            return;
        }
        if let TabAction::OpenFiltered { schema, table, filter } = action {
            return self.open_table_filtered(ci, schema, table, filter);
        }
        if let TabAction::LoadLayers(loads) = action {
            let (conn, tab) = (self.tabs[ti].conn.clone(), self.tabs[ti].id);
            self.ensure_connected(ci);
            for a in loads {
                self.worker.send(Cmd::LayerQuery { conn: conn.clone(), tab, layer: a.layer, seq: a.seq, kind: a.kind, sql: a.sql });
            }
            return;
        }
        // Pending result edits are keyed by row index: anything that replaces
        // or reorders the rows waits for Save / Discard.
        if let TabKind::Console(c) = &self.tabs[ti].kind
            && !c.changes.is_empty()
            && matches!(action, TabAction::Run { .. } | TabAction::Explain { .. } | TabAction::SortResult(..) | TabAction::Page(_) | TabAction::ResultArea(_) | TabAction::DeleteRows(_))
        {
            self.toast = Some(("Save or discard your changes first".into(), theme::WARN(), Instant::now()));
            return;
        }
        let connected = matches!(self.conns[ci].status, Status::Connected(_));
        let mut needs_connect = false;
        let database = match &self.tabs[ti].kind {
            TabKind::Console(c) => c.database.clone(),
            TabKind::Table(_) | TabKind::Layout(_) | TabKind::Diagram(_) | TabKind::Activity(_) => None,
        };
        let catalog = self.conns[ci].catalog_in(database.as_deref());
        let schemas = user_schemas(&self.conns[ci], database.as_deref());
        let tab = &mut self.tabs[ti];
        match (action, &mut tab.kind) {
            (TabAction::Run { all }, TabKind::Console(c)) => {
                let confirmed = std::mem::take(&mut self.skip_confirm);
                if c.running.is_some() || self.confirm.is_some() {
                    return;
                }
                let at = sql::char_to_byte(&c.sql, c.cursor.start);
                let text = if all {
                    c.sql.clone()
                } else if c.cursor.start != c.cursor.end {
                    let b = sql::char_to_byte(&c.sql, c.cursor.end);
                    c.sql[at..b].to_string()
                } else {
                    run_range(&c.sql, at).map(|r| c.sql[r].to_string()).unwrap_or_default()
                };
                // Slash commands are ours, not SQL: expand before sending.
                let text = match rgrip_core::commands::expand_all(&text, &catalog, c.schema_or_default(&schemas)) {
                    Ok(t) => t,
                    Err(e) => {
                        self.toast = Some((e, theme::WARN(), Instant::now()));
                        return;
                    }
                };
                if text.trim().is_empty() {
                    if !all && !c.sql.trim().is_empty() {
                        self.toast = Some(("Put the cursor on the statement to run".into(), theme::WARN(), Instant::now()));
                    }
                    return;
                }
                let cfg = &self.conns[ci].cfg;
                if !confirmed {
                    let items = risky_items(&text, cfg.color == ConnColor::Red);
                    if !items.is_empty() {
                        self.confirm = Some(Confirm { tab: tab.id, kind: ConfirmKind::Run { all }, title: format!("Run on {}?", cfg.display_name()), items, focused: false });
                        return;
                    }
                }
                c.running = Some(Instant::now());
                c.popup = None;
                let lower = text.to_lowercase();
                c.ddl = ["create", "alter", "drop", "truncate", "comment"]
                    .iter()
                    .any(|k| lower.split(|ch: char| !ch.is_alphanumeric()).any(|w| w == *k));
                c.offset = 0;
                c.last_sql = text.clone();
                c.run_schema = c.schema_for_run(&schemas);
                c.run_database = c.database.clone();
                c.paged = sql::paged_query(&text, 1, 0).map(|_| text.clone());
                c.paged_base = c.paged.clone();
                c.area.clear();
                c.order = None;
                c.unsorted = None;
                (c.relations, c.sources, c.probe, c.related, c.total) = (Vec::new(), None, None, None, None);
                (c.count_asked, c.origins_asked) = (false, false);
                c.run += 1;
                c.history_run = Some((text.clone(), rgrip_core::history::now()));
                let page = c.paged.is_some().then_some(0);
                if connected {
                    self.worker.send(Cmd::Query { conn: tab.conn.clone(), tab: tab.id, sql: text, page, schema: c.run_schema.clone(), database: c.run_database.clone() });
                } else {
                    c.pending = Some(text);
                    needs_connect = true;
                }
            }
            (TabAction::Explain { analyze }, TabKind::Console(c)) => {
                let confirmed = std::mem::take(&mut self.skip_confirm);
                if c.running.is_some() || self.confirm.is_some() {
                    return;
                }
                let at = sql::char_to_byte(&c.sql, c.cursor.start);
                // The statement Run would run.
                let text = if c.cursor.start != c.cursor.end {
                    let b = sql::char_to_byte(&c.sql, c.cursor.end);
                    c.sql[at..b].to_string()
                } else {
                    run_range(&c.sql, at).map(|r| c.sql[r].to_string()).unwrap_or_default()
                };
                let text = match rgrip_core::commands::expand_all(&text, &catalog, c.schema_or_default(&schemas)) {
                    Ok(t) => t,
                    Err(e) => {
                        self.toast = Some((e, theme::WARN(), Instant::now()));
                        return;
                    }
                };
                let note = if text.trim().is_empty() {
                    Some("Put the cursor on the statement to explain")
                } else if !rgrip_core::explain::is_single_statement(&text) {
                    Some("Explain works on one statement: put the cursor on it or select just that one")
                } else if text.trim_start().to_lowercase().starts_with("explain") {
                    Some("This is already an EXPLAIN: run it with ⌘↩")
                } else if !connected {
                    Some("Open the connection first")
                } else {
                    None
                };
                if let Some(n) = note {
                    if !c.sql.trim().is_empty() {
                        self.toast = Some((n.into(), theme::WARN(), Instant::now()));
                    }
                    return;
                }
                let cfg = &self.conns[ci].cfg;
                // ANALYZE really executes the statement, so a risky one asks first, like Run.
                if analyze && !confirmed {
                    let items = risky_items(&text, cfg.color == ConnColor::Red);
                    if !items.is_empty() {
                        self.confirm = Some(Confirm { tab: tab.id, kind: ConfirmKind::Explain { analyze }, title: format!("Explain analyze on {}?", cfg.display_name()), items, focused: false });
                        return;
                    }
                }
                // A statement that writes runs in a transaction that is rolled back.
                let rollback = analyze && !rgrip_core::safety::classify(&text).is_empty();
                let sql = rgrip_core::explain::explain_sql(&text, analyze);
                c.running = Some(Instant::now());
                c.popup = None;
                c.explaining = Some(rollback);
                c.run_schema = c.schema_for_run(&schemas);
                c.run_database = c.database.clone();
                self.worker.send(Cmd::Explain { conn: tab.conn.clone(), tab: tab.id, sql, rollback, schema: c.run_schema.clone(), database: c.run_database.clone() });
            }
            (TabAction::SortResult(n, col), TabKind::Console(c)) => {
                if c.running.is_some() {
                    return;
                }
                let base = c.filtered_base();
                let Some(Ok(out)) = &mut c.output else { return };
                let Some(rs) = out.statements.iter_mut().filter_map(|s| s.result.as_mut()).nth(n) else { return };
                let Some(name) = rs.columns.get(col).cloned() else { return };
                // Cycle: ascending → descending → as returned.
                c.order = match c.order.take() {
                    Some((m, c2, true)) if m == n && c2 == name => Some((n, name.clone(), false)),
                    Some((m, c2, false)) if m == n && c2 == name => None,
                    _ => Some((n, name.clone(), true)),
                };
                match (base, connected) {
                    // Paged: sort on the server, so every page is in order.
                    (Some(base), true) => {
                        c.paged = c.sorted(base);
                        c.offset = 0;
                        c.running = Some(Instant::now());
                        let sql = c.paged.clone().unwrap_or_default();
                        self.worker.send(Cmd::Query { conn: tab.conn.clone(), tab: tab.id, sql, page: Some(0), schema: c.run_schema.clone(), database: c.run_database.clone() });
                    }
                    (Some(_), false) => {}
                    // Everything is loaded: sort in memory (keeping the original order to go back to).
                    (None, _) => {
                        if c.unsorted.as_ref().is_none_or(|(m, _)| *m != n) {
                            c.unsorted = Some((n, rs.rows.clone()));
                        }
                        match &c.order {
                            Some((_, _, asc)) => rs.sort_by_column(col, *asc),
                            None => {
                                if let Some((_, rows)) = c.unsorted.take() {
                                    rs.rows = rows;
                                }
                            }
                        }
                        c.grid = GridState::fresh(&c.grid);
                    }
                }
            }
            (TabAction::ResultArea(area), TabKind::Console(c)) => {
                if c.running.is_some() || !connected {
                    return;
                }
                c.area = area.unwrap_or_default();
                let Some(base) = c.filtered_base() else { return };
                c.paged = c.sorted(base);
                c.offset = 0;
                (c.total, c.count_asked) = (None, false);
                c.running = Some(Instant::now());
                let sql = c.paged.clone().unwrap_or_default();
                self.worker.send(Cmd::Query { conn: tab.conn.clone(), tab: tab.id, sql, page: Some(0), schema: c.run_schema.clone(), database: c.run_database.clone() });
            }
            (TabAction::Page(forward), TabKind::Console(c)) => {
                let Some(sql) = c.paged.clone().filter(|_| c.running.is_none() && connected) else { return };
                let size = PAGE_SIZE as usize;
                c.offset = if forward { c.offset + size } else { c.offset.saturating_sub(size) };
                c.running = Some(Instant::now());
                self.worker.send(Cmd::Query { conn: tab.conn.clone(), tab: tab.id, sql, page: Some(c.offset), schema: c.run_schema.clone(), database: c.run_database.clone() });
            }
            (TabAction::Save, TabKind::Console(c)) => {
                let existing = c.saved_id.as_ref().and_then(|id| self.saved.iter_mut().find(|q| &q.id == id));
                match existing {
                    Some(q) => {
                        q.sql = c.sql.clone();
                        q.connection = Some(tab.conn.clone());
                        (q.database, q.schemas) = (c.database.clone(), c.picked_schemas());
                        let (name, result) = (q.name.clone(), config::update_query(q));
                        if result.is_ok() {
                            self.toast = Some((format!("Saved “{name}”"), theme::OK(), Instant::now()));
                        }
                        self.report(result);
                    }
                    None => {
                        let name = default_query_name(&c.sql, self.saved.len() + 1);
                        self.name_prompt = Some(NamePrompt { name, target: NameTarget::NewFrom(tab.id) });
                    }
                }
            }
            (TabAction::ExportAll(format, path), kind) => {
                let (database, schema, sql, table) = match kind {
                    TabKind::Console(c) => match &c.paged {
                        Some(sql) => (c.run_database.clone(), c.run_schema.clone(), sql.clone(), "query_result".to_string()),
                        None => return,
                    },
                    TabKind::Table(t) => (
                        None,
                        None,
                        rgrip_core::postgres::table_select_sql(&t.query()),
                        format!("{}.{}", t.schema, t.table),
                    ),
                    TabKind::Layout(_) | TabKind::Diagram(_) | TabKind::Activity(_) => return,
                };
                self.toast = Some((format!("Exporting to {}…", path.display()), theme::MUTED(), Instant::now()));
                self.worker.send(Cmd::ExportAll { conn: tab.conn.clone(), database, schema, sql, format, table, path });
            }
            (TabAction::Reproject(col), TabKind::Console(c)) => {
                let Some(query) = c.paged.clone() else { return };
                let alias = format!("{col}_4326");
                c.sql = reproject_sql(&query, &col, &alias);
                c.map.prefer = Some(alias);
                c.view = View::Map(0);
                c.cursor = 0..0;
                return self.tab_action(ti, TabAction::Run { all: true });
            }
            (TabAction::LoadSchemas, _) => {
                let c = &mut self.conns[ci];
                let id = c.cfg.id.clone();
                if c.databases.is_none() {
                    c.databases = Some(Load::Loading);
                    self.worker.send(Cmd::Databases(id.clone()));
                }
                match c.other_db(database.as_deref()).map(str::to_string) {
                    None if c.schemas.is_none() => {
                        c.schemas = Some(Load::Loading);
                        self.worker.send(Cmd::Schemas(id, None));
                    }
                    Some(db) if c.schemas_in(Some(&db)).is_none() => {
                        c.dbs.entry(db.clone()).or_default().schemas = Some(Load::Loading);
                        self.worker.send(Cmd::Schemas(id.clone(), Some(db.clone())));
                        self.worker.send(Cmd::Catalog(id, Some(db)));
                    }
                    _ => {}
                }
            }
            (TabAction::Notify(text), _) => self.toast = Some((text, theme::WARN(), Instant::now())),
            (TabAction::NewCommand(name), _) => self.apply(Action::NewCommand(name)),
            (TabAction::History, _) => self.apply(Action::ShowHistory(ci)),
            (TabAction::Cancel, TabKind::Console(c)) => {
                if c.pending.take().is_some() {
                    c.history_run = None;
                    c.running = None;
                } else if c.running.is_some() {
                    self.worker.send(Cmd::Cancel(tab.conn.clone(), c.run_database.clone()));
                }
            }
            (TabAction::SaveEdits, TabKind::Table(t)) => {
                let confirmed = std::mem::take(&mut self.skip_confirm);
                if self.conns[ci].cfg.read_only {
                    self.toast = Some((READ_ONLY_HINT.into(), theme::WARN(), Instant::now()));
                    return;
                }
                let Some(Ok(rs)) = &t.data else { return };
                if !t.dirty() || t.saving || !connected {
                    return;
                }
                match edit_statements(t, rs) {
                    Ok(statements) => {
                        let cfg = &self.conns[ci].cfg;
                        if cfg.color == ConnColor::Red && !confirmed {
                            let items = statements
                                .iter()
                                .map(|s| (s.clone(), if s.starts_with("insert") { "Adds a row on a production connection" } else { "Updates a row on a production connection" }.to_string(), false))
                                .collect();
                            self.confirm = Some(Confirm { tab: tab.id, kind: ConfirmKind::Save, title: format!("Save {} to {}?", plural(if t.added == 0 { t.changes.len() } else { statements.len() }, "change"), cfg.display_name()), items, focused: false });
                            return;
                        }
                        t.saving = true;
                        t.editor = None;
                        self.worker.send(Cmd::SaveEdits { conn: tab.conn.clone(), tab: tab.id, statements });
                    }
                    Err(e) => self.toast = Some((e, theme::ERROR(), Instant::now())),
                }
            }
            (TabAction::SaveEdits, TabKind::Console(c)) => {
                let confirmed = std::mem::take(&mut self.skip_confirm);
                if self.conns[ci].cfg.read_only {
                    self.toast = Some((READ_ONLY_HINT.into(), theme::WARN(), Instant::now()));
                    return;
                }
                if c.changes.is_empty() || c.running.is_some() || !connected {
                    return;
                }
                let Some((schema, table, key)) = console_edit_target(c, &self.conns[ci].catalog) else { return };
                let Some((origins, _)) = &c.sources else { return };
                let Some(Ok(out)) = &c.output else { return };
                let Some(rs) = out.statements.iter().find_map(|s| s.result.as_ref()) else { return };
                let statements = console_edit_statements(&schema, &table, &key, origins, rs, &c.changes);
                let cfg = &self.conns[ci].cfg;
                if cfg.color == ConnColor::Red && !confirmed {
                    let items = statements.iter().map(|s| (s.clone(), "Updates a row on a production connection".to_string(), false)).collect();
                    self.confirm = Some(Confirm { tab: tab.id, kind: ConfirmKind::Save, title: format!("Save {} to {}?", plural(c.changes.len(), "change"), cfg.display_name()), items, focused: false });
                    return;
                }
                c.running = Some(Instant::now());
                c.saving = true;
                c.editor = None;
                self.worker.send(Cmd::SaveEdits { conn: tab.conn.clone(), tab: tab.id, statements });
            }
            (TabAction::DeleteRows(rows), TabKind::Table(t)) => {
                let confirmed = std::mem::take(&mut self.skip_confirm);
                if self.conns[ci].cfg.read_only {
                    self.toast = Some((READ_ONLY_HINT.into(), theme::WARN(), Instant::now()));
                    return;
                }
                let Some(Ok(rs)) = &t.data else { return };
                // Pending edits are keyed by row index, and the reload after
                // deleting would drop them.
                if t.dirty() {
                    self.toast = Some(("Save or discard your changes first".into(), theme::WARN(), Instant::now()));
                    return;
                }
                if t.saving || !connected || rows.is_empty() {
                    return;
                }
                match table_key(t, rs).map(|key| delete_statements(&t.schema, &t.table, &key, rs, &rows)) {
                    Ok(statements) => {
                        // Deleting can't be undone: always show the statements.
                        if !confirmed {
                            let cfg = &self.conns[ci].cfg;
                            let why = if cfg.color == ConnColor::Red { "Deletes a row on a production connection" } else { "Deletes a row" };
                            let items = statements.iter().map(|s| (s.clone(), why.to_string(), true)).collect();
                            let title = format!("Delete {} from {}.{}?", plural(rows.len(), "row"), t.schema, t.table);
                            self.confirm = Some(Confirm { tab: tab.id, kind: ConfirmKind::Delete(rows), title, items, focused: false });
                            return;
                        }
                        t.saving = true;
                        t.deleting = true;
                        t.editor = None;
                        t.grid.selection.clear();
                        self.worker.send(Cmd::SaveEdits { conn: tab.conn.clone(), tab: tab.id, statements });
                    }
                    Err(e) => self.toast = Some((e, theme::ERROR(), Instant::now())),
                }
            }
            (TabAction::DeleteRows(rows), TabKind::Console(c)) => {
                let confirmed = std::mem::take(&mut self.skip_confirm);
                if self.conns[ci].cfg.read_only {
                    self.toast = Some((READ_ONLY_HINT.into(), theme::WARN(), Instant::now()));
                    return;
                }
                if c.running.is_some() || !connected || rows.is_empty() {
                    return;
                }
                let Some((schema, table, key)) = console_edit_target(c, &self.conns[ci].catalog) else { return };
                let Some(Ok(out)) = &c.output else { return };
                let Some(rs) = out.statements.iter().find_map(|s| s.result.as_ref()) else { return };
                let statements = delete_statements(&schema, &table, &key, rs, &rows);
                // Deleting can't be undone: always show the statements.
                if !confirmed {
                    let cfg = &self.conns[ci].cfg;
                    let why = if cfg.color == ConnColor::Red { "Deletes a row on a production connection" } else { "Deletes a row" };
                    let items = statements.iter().map(|s| (s.clone(), why.to_string(), true)).collect();
                    let title = format!("Delete {} from {schema}.{table}?", plural(rows.len(), "row"));
                    self.confirm = Some(Confirm { tab: tab.id, kind: ConfirmKind::Delete(rows), title, items, focused: false });
                    return;
                }
                c.running = Some(Instant::now());
                (c.saving, c.deleting) = (true, true);
                c.editor = None;
                c.grid.selection.clear();
                self.worker.send(Cmd::SaveEdits { conn: tab.conn.clone(), tab: tab.id, statements });
            }
            (TabAction::AddKey, TabKind::Table(t)) => {
                let confirmed = std::mem::take(&mut self.skip_confirm);
                if self.conns[ci].cfg.read_only {
                    self.toast = Some((READ_ONLY_HINT.into(), theme::WARN(), Instant::now()));
                    return;
                }
                if t.saving || !connected || t.dirty() {
                    return;
                }
                let name = ["id", "gid", "fid", "row_id"]
                    .into_iter()
                    .find(|n| !t.columns.iter().any(|c| c.name == *n))
                    .unwrap_or("rgrip_id");
                let sql = format!(
                    "alter table {}.{} add column {} bigint generated always as identity primary key",
                    rgrip_core::sql::quote_ident(&t.schema),
                    rgrip_core::sql::quote_ident(&t.table),
                    rgrip_core::sql::quote_ident(name)
                );
                // Changes the table for everyone: always show the statement.
                if !confirmed {
                    let title = format!("Add a primary key to {}.{}?", t.schema, t.table);
                    let why = format!("Adds column {name}, numbered 1, 2, 3… in the current row order; rewrites the table");
                    self.confirm = Some(Confirm { tab: tab.id, kind: ConfirmKind::AddKey, title, items: vec![(sql, why, false)], focused: false });
                    return;
                }
                t.saving = true;
                self.worker.send(Cmd::AddKey { conn: tab.conn.clone(), tab: tab.id, sql });
            }
            (TabAction::ShowRelated(target), kind) => {
                let Some((related, _)) = kind.related_slots() else { return };
                match related {
                    Some(rel) => {
                        let prev = std::mem::replace(&mut rel.target, target);
                        rel.back.push(prev);
                        rel.data = None;
                        rel.total = None;
                    }
                    None => {
                        *related = Some(Related {
                            target,
                            back: Vec::new(),
                            data: None,
                            total: None,
                            relations: Vec::new(),
                            columns: Vec::new(),
                            fk_relations: Vec::new(),
                            grid: GridState::default(),
                            probe: None,
                        })
                    }
                }
                let query = related.as_ref().unwrap().query();
                if connected {
                    self.worker.send(Cmd::TableData { conn: tab.conn.clone(), tab: tab.id, query, modal: true });
                }
            }
            (TabAction::Probe { modal, rows, probes, map }, kind) => {
                let Some((related, slot)) = kind.related_slots() else { return };
                if !connected || probes.is_empty() {
                    return;
                }
                let probe = Probe { rows: rows.clone(), map, counts: Vec::new() };
                match (modal, related) {
                    (true, Some(rel)) => rel.probe = Some(probe),
                    (true, None) => return,
                    (false, _) => *slot = Some(probe),
                }
                self.worker.send(Cmd::ProbeRelations { conn: tab.conn.clone(), tab: tab.id, modal, key: rows, probes });
            }
            (TabAction::RelatedBack, kind) => {
                let Some((Some(rel), _)) = kind.related_slots() else { return };
                let Some(prev) = rel.back.pop() else { return };
                rel.target = prev;
                rel.data = None;
                rel.total = None;
                let query = rel.query();
                if connected {
                    self.worker.send(Cmd::TableData { conn: tab.conn.clone(), tab: tab.id, query, modal: true });
                }
            }
            (TabAction::Reload, TabKind::Table(t)) => {
                t.loading = true;
                t.total = None;
                if connected {
                    self.worker.send(Cmd::TableData {
                        conn: tab.conn.clone(),
                        tab: tab.id,
                        query: t.query(),
                        modal: false,
                    });
                } else {
                    t.pending = true;
                    needs_connect = true;
                }
            }
            _ => {}
        }
        if needs_connect {
            self.ensure_connected(ci);
        }
    }
}
