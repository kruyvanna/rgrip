use super::*;

/// The history record of a finished run: rows are what the batch returned
/// or changed (a page of a long result counts as the page).
fn history_entry(conn: &str, database: Option<String>, sql: String, at: u64, started: Option<Instant>, result: &Result<QueryOutput, String>) -> rgrip_core::history::Entry {
    let (millis, rows, error) = match result {
        Ok(out) => {
            let rows = out.statements.iter().map(|s| s.result.as_ref().map_or(s.affected, |r| r.rows.len() as u64)).sum();
            (out.elapsed.as_millis() as u64, Some(rows), false)
        }
        Err(_) => (started.map_or(0, |t| t.elapsed().as_millis() as u64), None, true),
    };
    rgrip_core::history::Entry { conn: conn.to_string(), database, sql, at, millis, rows, error }
}

impl App {
    // ---- worker events -------------------------------------------------

    pub(super) fn handle_events(&mut self) {
        let events: Vec<Ev> = self.worker.poll().collect();
        for ev in events {
            match ev {
                Ev::Connected { conn, version } => {
                    let Some(ci) = self.conn_index(&conn) else { continue };
                    self.conns[ci].status = Status::Connected(version);
                    self.worker.send(Cmd::Catalog(conn.clone(), None));
                    self.flush_pending(&conn);
                    if !self.tabs.iter().any(|t| t.conn == conn) {
                        self.open_console(ci);
                    }
                }
                Ev::ConnectFailed { conn, error } => {
                    let Some(ci) = self.conn_index(&conn) else { continue };
                    self.conns[ci].status = Status::Failed(error.clone());
                    for tab in self.tabs.iter_mut().filter(|t| t.conn == conn) {
                        match &mut tab.kind {
                            TabKind::Console(c) if c.pending.is_some() => {
                                c.pending = None;
                                c.history_run = None;
                                c.running = None;
                                c.output = Some(Err(error.clone()));
                            }
                            TabKind::Table(t) if t.pending => {
                                t.pending = false;
                                t.loading = false;
                                t.data = Some(Err(error.clone()));
                            }
                            TabKind::Diagram(d) if matches!(d.state, crate::diagram_tab::State::Pending) => {
                                d.state = crate::diagram_tab::State::Failed(error.clone());
                            }
                            TabKind::Activity(a) if a.pending => a.failed(error.clone()),
                            _ => {}
                        }
                    }
                }
                Ev::TestResult(result) => {
                    if let Some(d) = &mut self.dialog {
                        d.testing = false;
                        d.test = Some(result);
                    }
                }
                Ev::Schemas { conn, database: None, result } => {
                    if let Some(ci) = self.conn_index(&conn) {
                        self.conns[ci].schemas = Some(result.into());
                    }
                }
                Ev::Schemas { conn, database: Some(db), result } => {
                    if let Err(e) = &result {
                        self.toast = Some((e.clone(), theme::ERROR(), Instant::now()));
                    }
                    if let Some(ci) = self.conn_index(&conn) {
                        self.conns[ci].dbs.entry(db).or_default().schemas = Some(result.into());
                    }
                }
                Ev::Databases { conn, result } => {
                    if let Some(ci) = self.conn_index(&conn) {
                        self.conns[ci].databases = Some(result.into());
                    }
                }
                Ev::Tables { conn, schema, result } => {
                    if let Some(ci) = self.conn_index(&conn) {
                        self.conns[ci].tables.insert(schema, result.into());
                    }
                }
                Ev::Columns { conn, schema, table, result } => {
                    if let Some(ci) = self.conn_index(&conn) {
                        self.conns[ci].columns.insert((schema, table), result.into());
                    }
                }
                Ev::Catalog { conn, database: Some(db), result } => {
                    if let (Some(ci), Ok(catalog)) = (self.conn_index(&conn), result) {
                        self.conns[ci].dbs.entry(db).or_default().catalog = Arc::new(catalog);
                    }
                }
                Ev::Catalog { conn, database: None, result } => {
                    if let (Some(ci), Ok(catalog)) = (self.conn_index(&conn), result) {
                        let catalog = Arc::new(catalog);
                        self.conns[ci].catalog = catalog.clone();
                        // Pages that arrived before the catalog only had real
                        // foreign keys: add the inferred relations now.
                        for tab in self.tabs.iter_mut().filter(|t| t.conn == conn) {
                            if let TabKind::Table(t) = &mut tab.kind {
                                t.relations = with_inferred(&t.schema, &t.table, &t.columns, &t.fk_relations, &catalog);
                                if t.key.is_empty() && !t.columns.is_empty() {
                                    t.key = edit_key(&t.schema, &t.table, &t.columns, &catalog);
                                }
                                if let Some(rel) = &mut t.related {
                                    rel.relations = with_inferred(&rel.target.schema, &rel.target.table, &rel.columns, &rel.fk_relations, &catalog);
                                }
                            }
                            if let TabKind::Console(c) = &mut tab.kind {
                                if let Some(rel) = &mut c.related {
                                    rel.relations = with_inferred(&rel.target.schema, &rel.target.table, &rel.columns, &rel.fk_relations, &catalog);
                                }
                                // Run on the connection's own database: its catalog.
                                if c.run_database.is_none()
                                    && let Some((origins, tables)) = &c.sources
                                    && let Some(Ok(out)) = &c.output
                                    && let Some(rs) = out.statements.iter().find_map(|s| s.result.as_ref())
                                {
                                    c.relations = result_relations(origins, tables, &rs.columns, &catalog);
                                    c.probe = None;
                                }
                            }
                        }
                    }
                }
                Ev::Diagram { conn, schema, result } => {
                    if let Some(Tab { kind: TabKind::Diagram(d), .. }) = self.tabs.iter_mut().find(|t| t.conn == conn && matches!(&t.kind, TabKind::Diagram(d) if d.schema == schema)) {
                        d.loaded(result);
                    }
                }
                Ev::Activity { tab, result } => {
                    if let Some(Tab { kind: TabKind::Activity(a), .. }) = self.tab_mut(tab) {
                        a.loaded(result);
                    }
                }
                Ev::BackendSignal { tab, pid, terminate, result } => {
                    if let Some(Tab { kind: TabKind::Activity(a), .. }) = self.tab_mut(tab) {
                        a.signalled(pid, terminate, result);
                    }
                }
                Ev::LayerResult { tab, layer, seq, result } => {
                    if let Some(Tab { kind: TabKind::Layout(l), .. }) = self.tab_mut(tab) {
                        l.layer_result(layer, seq, result);
                    }
                }
                Ev::QueryDone { tab, result } => {
                    let mut refresh = None;
                    let mut origins = None;
                    let mut count = None;
                    if let Some(Tab { kind: TabKind::Console(c), conn, .. }) = self.tab_mut(tab) {
                        if let Ok(out) = &result {
                            let truncated = out.statements.iter().any(|s| s.result.as_ref().is_some_and(|r| r.truncated));
                            let args = (conn.clone(), tab, c.run, c.last_sql.clone(), c.run_schema.clone());
                            // The count covers the area filter too.
                            let counted = c.filtered_base().unwrap_or_else(|| c.last_sql.clone());
                            // First page of a single read-only statement on the connection's own database.
                            if c.paged_base.is_some() && c.offset == 0 && c.run_database.is_none() && !c.origins_asked {
                                c.origins_asked = true;
                                origins = Some(args.clone());
                            }
                            // More pages exist: count them in the background (once per run).
                            if c.paged_base.is_some() && truncated && !c.count_asked {
                                c.count_asked = true;
                                let (conn, tab, run, _, schema) = args.clone();
                                count = Some(((conn, tab, run, counted, schema), c.run_database.clone()));
                            }
                        }
                        if std::mem::take(&mut c.ddl) && result.is_ok() {
                            refresh = Some((conn.clone(), c.run_database.clone()));
                        }
                        if let Some((sql, at)) = c.history_run.take() {
                            // Failing to write the history must never get in the way of a run.
                            let _ = rgrip_core::history::record(&history_entry(conn, c.run_database.clone(), sql, at, c.running, &result));
                        }
                        c.running = None;
                        c.grid = GridState::fresh(&c.grid);
                        c.view = match &result {
                            Ok(out) => match (out.statements.iter().filter(|s| s.result.is_some()).count(), c.view) {
                                (0, _) => View::Output,
                                // On the map (an area filter, a page, a sort): stay there.
                                (n, v @ (View::Map(i) | View::Split(i))) if i < n => v,
                                (n, _) => View::Result(n - 1),
                            },
                            Err(_) => View::Output,
                        };
                        // A JSON EXPLAIN you ran yourself gets the plan view too.
                        if let Some(plan) = result.as_ref().ok().and_then(rgrip_core::explain::plan_from_output) {
                            c.plan = Some(crate::plan_view::PlanState::new(plan, None));
                            c.view = View::Plan;
                        }
                        // Edits are keyed by row index: they belong to the old rows.
                        c.changes.clear();
                        c.editor = None;
                        c.output = Some(result);
                    }
                    if let Some((conn, database)) = refresh {
                        self.worker.send(Cmd::Catalog(conn, database));
                    }
                    if let Some((conn, tab, run, sql, schema)) = origins {
                        self.worker.send(Cmd::QueryOrigins { conn, tab, run, sql, schema });
                    }
                    if let Some(((conn, tab, run, sql, schema), database)) = count {
                        self.worker.send(Cmd::CountRows { conn, tab, run, sql, schema, database });
                    }
                }
                Ev::ExplainDone { tab, result } => {
                    if let Some(Tab { kind: TabKind::Console(c), .. }) = self.tab_mut(tab) {
                        let rolled_back = c.explaining.take().unwrap_or_default();
                        c.running = None;
                        match result {
                            Ok(plan) => {
                                let note = rolled_back.then(|| "Ran in a transaction that was rolled back: no rows or tables changed (sequences and other effects outside the transaction still happened)".to_string());
                                c.plan = Some(crate::plan_view::PlanState::new(plan, note));
                                c.view = View::Plan;
                            }
                            Err(e) => {
                                c.output = Some(Err(e));
                                c.view = View::Output;
                            }
                        }
                    }
                }
                Ev::QueryOrigins { tab, run, sql, origins, tables } => {
                    let catalog = self.tab_catalog(tab);
                    if let Some(Tab { kind: TabKind::Console(c), .. }) = self.tab_mut(tab)
                        && c.run == run
                        && c.running.is_none()
                        && c.last_sql == sql
                        && let Some(Ok(out)) = &c.output
                        && let Some(rs) = out.statements.iter().find_map(|s| s.result.as_ref())
                    {
                        c.relations = result_relations(&origins, &tables, &rs.columns, &catalog);
                        c.sources = Some((origins, tables));
                        c.probe = None;
                    }
                }
                Ev::QueryCount { tab, run, sql, result } => {
                    if let Some(Tab { kind: TabKind::Console(c), .. }) = self.tab_mut(tab)
                        && c.run == run
                        && c.filtered_base().unwrap_or_else(|| c.last_sql.clone()) == sql
                    {
                        c.total = result.ok();
                    }
                }
                Ev::TablePage { tab, modal: true, result, columns, relations } => {
                    let catalog = self.tab_catalog(tab);
                    if let Some(tab) = self.tab_mut(tab)
                        && let Some((related, _)) = tab.kind.related_slots()
                        && let Some(rel) = related
                    {
                        rel.relations = with_inferred(&rel.target.schema, &rel.target.table, &columns, &relations, &catalog);
                        (rel.columns, rel.fk_relations) = (columns, relations);
                        rel.grid = GridState::fresh(&rel.grid);
                        rel.data = Some(result);
                    }
                }
                Ev::TablePage { tab, modal: false, result, columns, relations } => {
                    let catalog = self.tab_catalog(tab);
                    if let Some(Tab { kind: TabKind::Table(t), .. }) = self.tab_mut(tab) {
                        t.relations = with_inferred(&t.schema, &t.table, &columns, &relations, &catalog);
                        t.fk_relations = relations;
                        t.loading = false;
                        t.grid = GridState::fresh(&t.grid);
                        t.data = Some(result);
                        t.key = edit_key(&t.schema, &t.table, &columns, &catalog);
                        t.temporal = columns
                            .iter()
                            .filter_map(|c| Some((c.name.clone(), rgrip_core::datetime::kind_of(&c.data_type)?)))
                            .collect();
                        t.columns = columns;
                        // Edits are keyed by row index: they belong to the old page.
                        t.changes.clear();
                        t.added = 0;
                        t.editor = None;
                    }
                }
                Ev::RelationProbe { tab, modal, key, counts } => {
                    if let Some(tab) = self.tab_mut(tab)
                        && let Some((related, probe)) = tab.kind.related_slots()
                    {
                        let probe = if modal { related.as_mut().and_then(|r| r.probe.as_mut()) } else { probe.as_mut() };
                        if let Some(p) = probe.filter(|p| p.rows == key) {
                            p.counts = counts;
                        }
                    }
                }
                Ev::EditsSaved { tab, result } => {
                    let Some(ti) = self.tabs.iter().position(|t| t.id == tab) else { continue };
                    if let TabKind::Table(t) = &mut self.tabs[ti].kind {
                        t.saving = false;
                        let verb = if std::mem::take(&mut t.deleting) { "Deleted" } else { "Saved" };
                        match result {
                            Ok(n) => {
                                t.changes.clear();
                                self.toast = Some((format!("{verb} {}", plural(n, "row")), theme::OK(), Instant::now()));
                                self.tab_action(ti, TabAction::Reload);
                            }
                            Err(e) => self.toast = Some((e, theme::ERROR(), Instant::now())),
                        }
                        continue;
                    }
                    let conn = self.tabs[ti].conn.clone();
                    if let TabKind::Console(c) = &mut self.tabs[ti].kind
                        && std::mem::take(&mut c.saving)
                    {
                        c.running = None;
                        let deleted = std::mem::take(&mut c.deleting);
                        match result {
                            Ok(n) => {
                                let verb = if deleted { "Deleted" } else { "Saved" };
                                self.toast = Some((format!("{verb} {}", plural(n, "row")), theme::OK(), Instant::now()));
                                if deleted {
                                    c.total = c.total.map(|t| t - n as i64);
                                }
                                c.changes.clear();
                                // The same page again, as saved.
                                if let Some(sql) = c.paged.clone() {
                                    c.running = Some(Instant::now());
                                    self.worker.send(Cmd::Query { conn, tab, sql, page: Some(c.offset), schema: c.run_schema.clone(), database: c.run_database.clone() });
                                }
                            }
                            Err(e) => self.toast = Some((e, theme::ERROR(), Instant::now())),
                        }
                    }
                }
                Ev::KeyAdded { tab, result } => {
                    let Some(ti) = self.tabs.iter().position(|t| t.id == tab) else { continue };
                    let conn = self.tabs[ti].conn.clone();
                    let TabKind::Table(t) = &mut self.tabs[ti].kind else { continue };
                    t.saving = false;
                    match result {
                        Ok(()) => {
                            self.toast = Some((format!("Added a primary key to {}.{}", t.schema, t.table), theme::OK(), Instant::now()));
                            // The explorer and the catalog learn the new column and key.
                            let key = (t.schema.clone(), t.table.clone());
                            if let Some(ci) = self.conn_index(&conn) {
                                self.conns[ci].columns.remove(&key);
                                self.worker.send(Cmd::Catalog(conn, None));
                            }
                            self.tab_action(ti, TabAction::Reload);
                        }
                        Err(e) => self.toast = Some((e, theme::ERROR(), Instant::now())),
                    }
                }
                Ev::TableDef { conn, schema, table, result } => {
                    if let Some(ed) = self.table_editor.as_mut().filter(|e| e.conn == conn && e.schema == schema && e.table == table) {
                        ed.loaded(result);
                    }
                }
                Ev::TableAltered { conn, schema, table, new_name, result } => {
                    match result {
                        Ok(()) => {
                            self.table_editor = None;
                            self.toast = Some((format!("Updated {schema}.{new_name}"), theme::OK(), Instant::now()));
                            if let Some(ci) = self.conn_index(&conn) {
                                self.conns[ci].columns.remove(&(schema.clone(), table.clone()));
                                self.conns[ci].columns.remove(&(schema.clone(), new_name.clone()));
                                self.worker.send(Cmd::Tables(conn.clone(), schema.clone()));
                                self.worker.send(Cmd::Catalog(conn.clone(), None));
                            }
                            // Open tabs of the table follow a rename and reload.
                            let open: Vec<usize> = (0..self.tabs.len())
                                .filter(|&i| self.tabs[i].conn == conn && matches!(&self.tabs[i].kind, TabKind::Table(t) if t.schema == schema && t.table == table))
                                .collect();
                            for i in open {
                                if let TabKind::Table(t) = &mut self.tabs[i].kind {
                                    t.table = new_name.clone();
                                    t.changes.clear();
                                    t.editor = None;
                                }
                                self.tab_action(i, TabAction::Reload);
                            }
                        }
                        Err(e) => {
                            if let Some(ed) = self.table_editor.as_mut() {
                                ed.failed(e);
                            }
                        }
                    }
                }
                Ev::ImportProgress { conn, rows } => {
                    if let Some(d) = self.import.as_mut().filter(|d| d.conn == conn) {
                        d.progress(rows);
                    }
                }
                Ev::ImportDone { conn, schema, table, result } => {
                    if let Ok(n) = &result {
                        self.toast = Some((format!("Imported {} into {schema}.{table}", plural(*n, "row")), theme::OK(), Instant::now()));
                        // The table list and the autocomplete catalog know the new table.
                        if let Some(ci) = self.conn_index(&conn) {
                            self.conns[ci].columns.remove(&(schema.clone(), table));
                            self.worker.send(Cmd::Tables(conn.clone(), schema));
                            self.worker.send(Cmd::Catalog(conn.clone(), None));
                        }
                    }
                    if let Some(d) = self.import.as_mut().filter(|d| d.conn == conn) {
                        d.finished(result);
                    }
                }
                Ev::Exported(result) => {
                    self.toast = Some(match result {
                        Ok((path, rows)) => (
                            format!("Exported {} to {}", plural(rows, "row"), path.display()),
                            theme::OK(),
                            Instant::now(),
                        ),
                        Err(e) => (format!("Export failed: {e}"), theme::ERROR(), Instant::now()),
                    });
                }
                Ev::UpdateChecked(result) => {
                    let manual = matches!(self.update, UpdateState::Checking { manual: true });
                    self.update = match result {
                        Ok(Some(release)) => UpdateState::Available(release),
                        Ok(None) => {
                            if manual {
                                self.toast = Some((format!("rgrip {} is up to date", update::CURRENT), theme::OK(), Instant::now()));
                            }
                            UpdateState::Idle
                        }
                        Err(e) => {
                            if manual {
                                self.toast = Some((e, theme::ERROR(), Instant::now()));
                            }
                            UpdateState::Idle
                        }
                    };
                }
                Ev::UpdateInstalled(result) => match result {
                    Ok(app) => match update::relaunch(&app) {
                        Ok(()) => self.ctx_close = true,
                        Err(e) => self.toast = Some((e, theme::ERROR(), Instant::now())),
                    },
                    Err(e) => {
                        if let UpdateState::Installing(r) = std::mem::replace(&mut self.update, UpdateState::Idle) {
                            self.update = UpdateState::Available(r);
                        }
                        self.toast = Some((e, theme::ERROR(), Instant::now()));
                    }
                },
                Ev::TableCount { tab, modal, result } => {
                    match self.tab_mut(tab).map(|t| &mut t.kind) {
                        Some(TabKind::Table(t)) if !modal => t.total = result.ok(),
                        Some(kind) if modal => {
                            if let Some((Some(rel), _)) = kind.related_slots() {
                                rel.total = result.ok();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Send work that was queued while the connection was coming up.
    fn flush_pending(&mut self, conn: &str) {
        for tab in self.tabs.iter_mut().filter(|t| t.conn == conn) {
            match &mut tab.kind {
                TabKind::Console(c) => {
                    if let Some(sql) = c.pending.take() {
                        let page = c.paged.is_some().then_some(c.offset);
                        self.worker.send(Cmd::Query { conn: tab.conn.clone(), tab: tab.id, sql, page, schema: c.run_schema.clone(), database: c.run_database.clone() });
                    }
                }
                TabKind::Table(t) if t.pending => {
                    t.pending = false;
                    self.worker.send(Cmd::TableData {
                        conn: tab.conn.clone(),
                        tab: tab.id,
                        query: t.query(),
                        modal: false,
                    });
                }
                TabKind::Diagram(d) if matches!(d.state, crate::diagram_tab::State::Pending) => {
                    d.state = crate::diagram_tab::State::Loading;
                    self.worker.send(Cmd::Diagram { conn: tab.conn.clone(), schema: d.schema.clone() });
                }
                TabKind::Activity(a) if a.pending => {
                    a.pending = false;
                    self.worker.send(Cmd::Activity { conn: tab.conn.clone(), tab: tab.id });
                }
                _ => {}
            }
        }
    }
}
