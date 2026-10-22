//! Background tokio runtime that owns all database connections.
//! The UI sends `Cmd`s and polls `Ev`s every frame, so it never blocks.

use rgrip_core::complete::Catalog;
use rgrip_core::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

pub type ConnId = String;
pub type TabId = u64;

/// Related-row counts stop here ("1000+").
pub const PROBE_CAP: usize = 1000;

/// Most features a map layout layer loads.
pub const LAYER_LIMIT: usize = 50_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerQuery {
    /// Geometry (WGS84 hex) and, optionally, a category value per row.
    Data,
    /// One WGS84 geometry: the layer's extent.
    Extent,
}

pub enum LayerResult {
    Data { shapes: Vec<rgrip_core::geo::Geometry>, values: Vec<Option<String>>, truncated: bool },
    /// The extent, and the row count.
    Extent(Option<(rgrip_core::geo::Coord, rgrip_core::geo::Coord)>, Option<i64>),
}

/// A table a query result reads from: schema, name, columns, foreign keys.
pub type SourceTable = (String, String, Vec<ColumnInfo>, Vec<Relation>);

pub enum Cmd {
    Connect(ConnectionConfig),
    Disconnect(ConnId),
    Test(ConnectionConfig),
    /// `Some(db)`: another database of the same server.
    Schemas(ConnId, Option<String>),
    Databases(ConnId),
    Tables(ConnId, String),
    Columns(ConnId, String, String),
    Catalog(ConnId, Option<String>),
    /// Tables, columns and links of a schema, for its diagram tab.
    Diagram { conn: ConnId, schema: String },
    /// `database`: `None` = the connection's own database.
    /// `page: Some(offset)` fetches one page of a read-only query.
    Query { conn: ConnId, tab: TabId, sql: String, page: Option<usize>, schema: Option<String>, database: Option<String> },
    /// Where the columns of a console statement come from, with their tables'
    /// columns and foreign keys (for relations in the result grid).
    QueryOrigins { conn: ConnId, tab: TabId, run: u64, sql: String, schema: Option<String> },
    /// Count the rows of a paged console statement (own connection, so paging stays quick).
    CountRows { conn: ConnId, tab: TabId, run: u64, sql: String, schema: Option<String>, database: Option<String> },
    /// `modal`: for the tab's related-rows popup rather than the tab itself.
    TableData { conn: ConnId, tab: TabId, query: TableQuery, modal: bool },
    /// Count matches of related-row filters (schema, table, filter) for the
    /// Related menu; `key` comes back with the counts.
    ProbeRelations { conn: ConnId, tab: TabId, modal: bool, key: Vec<usize>, probes: Vec<(String, String, String)> },
    /// Inline-edit UPDATEs, applied in one transaction.
    SaveEdits { conn: ConnId, tab: TabId, statements: Vec<String> },
    /// Columns and primary key constraint name, for Edit table.
    TableDef { conn: ConnId, schema: String, table: String },
    /// Edit table: the statements, all or nothing.
    AlterTable { conn: ConnId, schema: String, table: String, new_name: String, statements: Vec<String> },
    /// Add a primary key to a table tab's table.
    AddKey { conn: ConnId, tab: TabId, sql: String },
    Cancel(ConnId, Option<String>),
    /// Look for a newer release.
    CheckUpdate,
    /// Download, verify and install a release over `app`.
    InstallUpdate { release: rgrip_core::update::Release, app: std::path::PathBuf },
    /// Load a file into a table (own connection, one transaction).
    Import { conn: ConnId, request: rgrip_core::import::ImportRequest },
    /// Run `sql` uncapped and write it to `path`.
    ExportAll { conn: ConnId, database: Option<String>, schema: Option<String>, sql: String, format: rgrip_core::export::Format, table: String, path: std::path::PathBuf },
    /// A map layout layer query; `seq` comes back so stale answers can be dropped.
    LayerQuery { conn: ConnId, tab: TabId, layer: u64, seq: u64, kind: LayerQuery, sql: String },
}

pub enum Ev {
    LayerResult { tab: TabId, layer: u64, seq: u64, result: Result<LayerResult, String> },
    /// `version` is a summary such as "PostgreSQL 16.4 · TLSv1.3 · SSH".
    Connected { conn: ConnId, version: String },
    ConnectFailed { conn: ConnId, error: String },
    TestResult(Result<String, String>),
    Schemas { conn: ConnId, database: Option<String>, result: Result<Vec<String>, String> },
    Databases { conn: ConnId, result: Result<Vec<String>, String> },
    Tables { conn: ConnId, schema: String, result: Result<Vec<TableInfo>, String> },
    Columns { conn: ConnId, schema: String, table: String, result: Result<Vec<ColumnInfo>, String> },
    Catalog { conn: ConnId, database: Option<String>, result: Result<Catalog, String> },
    Diagram { conn: ConnId, schema: String, result: Result<rgrip_core::diagram::Diagram, String> },
    QueryDone { tab: TabId, result: Result<QueryOutput, String> },
    /// Per result column of `sql`, its source column; then per source table
    /// (schema, table) its columns and foreign keys.
    QueryOrigins { tab: TabId, run: u64, sql: String, origins: Vec<Option<rgrip_core::relations::Origin>>, tables: Vec<SourceTable> },
    /// Total rows of `sql`, or why it couldn't be counted.
    QueryCount { tab: TabId, run: u64, sql: String, result: Result<i64, String> },
    /// `columns`: the table's columns (primary key and types, for editing).
    /// `relations`: foreign keys from and to the table.
    TablePage { tab: TabId, modal: bool, result: Result<ResultSet, String>, columns: Vec<ColumnInfo>, relations: Vec<Relation> },
    /// Match counts per probe (`None`: failed or timed out).
    RelationProbe { tab: TabId, modal: bool, key: Vec<usize>, counts: Vec<Option<i64>> },
    /// Rows updated, or why nothing was saved.
    EditsSaved { tab: TabId, result: Result<usize, String> },
    KeyAdded { tab: TabId, result: Result<(), String> },
    TableDef { conn: ConnId, schema: String, table: String, result: Result<(Vec<ColumnInfo>, Option<String>), String> },
    TableAltered { conn: ConnId, schema: String, table: String, new_name: String, result: Result<(), String> },
    TableCount { tab: TabId, modal: bool, result: Result<i64, String> },
    /// Path and row count, or an error.
    Exported(Result<(std::path::PathBuf, usize), String>),
    /// Rows sent to the server so far by the running import.
    ImportProgress { conn: ConnId, rows: usize },
    /// Rows imported, or why nothing was (the import is rolled back).
    ImportDone { conn: ConnId, schema: String, table: String, result: Result<usize, String> },
    UpdateChecked(Result<Option<rgrip_core::update::Release>, String>),
    /// The bundle that was updated (ready to relaunch), or an error.
    UpdateInstalled(Result<std::path::PathBuf, String>),
}

pub struct Worker {
    tx: UnboundedSender<Cmd>,
    rx: mpsc::Receiver<Ev>,
}

/// The main connection of each `ConnId`, plus connections to other databases
/// of the same server, opened on demand for consoles that picked one.
#[derive(Clone, Default)]
struct Conns {
    main: Arc<Mutex<HashMap<ConnId, Arc<PgConn>>>>,
    extra: Arc<Mutex<HashMap<(ConnId, String), Arc<PgConn>>>>,
    /// Configs of connected connections, to open the extra ones from.
    cfgs: Arc<Mutex<HashMap<ConnId, ConnectionConfig>>>,
}

/// Key of the extra connection for `database`; `None` when the main
/// connection serves it (no database picked, or the configured one).
fn extra_key(conn: &str, database: Option<&str>, default: &str) -> Option<(ConnId, String)> {
    database.filter(|d| *d != default).map(|d| (conn.to_string(), d.to_string()))
}

#[derive(Clone)]
struct Emitter {
    tx: mpsc::Sender<Ev>,
    ctx: eframe::egui::Context,
}

impl Emitter {
    fn send(&self, ev: Ev) {
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }
}

impl Worker {
    pub fn spawn(ctx: eframe::egui::Context) -> Self {
        let (tx, mut cmd_rx) = unbounded_channel::<Cmd>();
        let (ev_tx, rx) = mpsc::channel();
        let emit = Emitter { tx: ev_tx, ctx };

        std::thread::Builder::new()
            .name("db-worker".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                rt.block_on(async move {
                    let conns: Conns = Default::default();
                    while let Some(cmd) = cmd_rx.recv().await {
                        tokio::spawn(handle(cmd, conns.clone(), emit.clone()));
                    }
                });
            })
            .expect("spawn worker thread");

        Self { tx, rx }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn poll(&self) -> impl Iterator<Item = Ev> + '_ {
        self.rx.try_iter()
    }
}

/// The main connection of `id`, reconnected if it died (e.g. the laptop
/// slept): refreshing then just works, no disconnect/connect needed.
async fn get(conns: &Conns, id: &str) -> Result<Arc<PgConn>, String> {
    let current = conns.main.lock().unwrap().get(id).cloned();
    let Some(c) = current else { return Err("Not connected".into()) };
    if c.alive().await {
        return Ok(c);
    }
    let Some(cfg) = conns.cfgs.lock().unwrap().get(id).cloned() else { return Err("Connection closed".into()) };
    let fresh = Arc::new(PgConn::connect(&cfg).await.map_err(|e| format!("Connection lost, reconnecting failed: {e}"))?);
    // Still ours (not disconnected meanwhile): swap it in; its other databases' connections died with it.
    if conns.cfgs.lock().unwrap().contains_key(id) {
        conns.main.lock().unwrap().insert(id.to_string(), fresh.clone());
        conns.extra.lock().unwrap().retain(|(c, _), _| c != id);
    }
    Ok(fresh)
}

/// The main connection as is, without checking or reconnecting.
fn current(conns: &Conns, id: &str) -> Option<Arc<PgConn>> {
    conns.main.lock().unwrap().get(id).filter(|c| !c.is_closed()).cloned()
}

/// The connection serving `database` of `id`, connecting to it if needed.
async fn get_db(conns: &Conns, id: &str, database: Option<&str>) -> Result<Arc<PgConn>, String> {
    let main = get(conns, id).await?;
    let Some(cfg) = conns.cfgs.lock().unwrap().get(id).cloned() else { return Ok(main) };
    let Some(key) = extra_key(id, database, &cfg.database) else { return Ok(main) };
    let existing = conns.extra.lock().unwrap().get(&key).cloned();
    if let Some(c) = existing
        && c.alive().await
    {
        return Ok(c);
    }
    let cfg = ConnectionConfig { database: key.1.clone(), ..cfg };
    let c = Arc::new(PgConn::connect(&cfg).await.map_err(|e| format!("Could not connect to database {}: {e}", key.1))?);
    conns.extra.lock().unwrap().insert(key, c.clone());
    Ok(c)
}

/// Drop every connection of `id`; dropping a client closes it.
fn close_all(conns: &Conns, id: &str) {
    conns.main.lock().unwrap().remove(id);
    conns.cfgs.lock().unwrap().remove(id);
    conns.extra.lock().unwrap().retain(|(c, _), _| c != id);
}

async fn handle(cmd: Cmd, conns: Conns, emit: Emitter) {
    match cmd {
        Cmd::Connect(cfg) => {
            let result = match PgConn::connect(&cfg).await {
                Ok(c) => c.server_info().await.map(|i| (c, i.summary())),
                Err(e) => Err(e),
            };
            match result {
                Ok((c, version)) => {
                    close_all(&conns, &cfg.id);
                    conns.cfgs.lock().unwrap().insert(cfg.id.clone(), cfg.clone());
                    conns.main.lock().unwrap().insert(cfg.id.clone(), Arc::new(c));
                    emit.send(Ev::Connected { conn: cfg.id, version });
                }
                Err(error) => emit.send(Ev::ConnectFailed { conn: cfg.id, error }),
            }
        }
        Cmd::Disconnect(id) => {
            close_all(&conns, &id);
        }
        Cmd::Test(cfg) => {
            let result = match PgConn::connect(&cfg).await {
                Ok(c) => c.server_info().await.map(|i| i.summary()),
                Err(e) => Err(e),
            };
            emit.send(Ev::TestResult(result));
        }
        Cmd::Schemas(conn, database) => {
            let result = match get_db(&conns, &conn, database.as_deref()).await {
                Ok(c) => c.schemas().await,
                Err(e) => Err(e),
            };
            emit.send(Ev::Schemas { conn, database, result });
        }
        Cmd::Databases(conn) => {
            let result = match get(&conns, &conn).await {
                Ok(c) => c.databases().await,
                Err(e) => Err(e),
            };
            emit.send(Ev::Databases { conn, result });
        }
        Cmd::Tables(conn, schema) => {
            let result = match get(&conns, &conn).await {
                Ok(c) => c.tables(&schema).await,
                Err(e) => Err(e),
            };
            emit.send(Ev::Tables { conn, schema, result });
        }
        Cmd::Columns(conn, schema, table) => {
            let result = match get(&conns, &conn).await {
                Ok(c) => c.columns(&schema, &table).await,
                Err(e) => Err(e),
            };
            emit.send(Ev::Columns { conn, schema, table, result });
        }
        Cmd::Catalog(conn, database) => {
            let result = match get_db(&conns, &conn, database.as_deref()).await {
                Ok(c) => c.catalog().await,
                Err(e) => Err(e),
            };
            emit.send(Ev::Catalog { conn, database, result });
        }
        Cmd::Diagram { conn, schema } => {
            let result = match get(&conns, &conn).await {
                Ok(c) => c.schema_diagram(&schema).await,
                Err(e) => Err(e),
            };
            emit.send(Ev::Diagram { conn, schema, result });
        }
        Cmd::Query { conn, tab, sql, page, schema, database } => {
            let result = match (get_db(&conns, &conn, database.as_deref()).await, page) {
                (Ok(c), Some(offset)) => c.execute_page_in(schema.as_deref(), &sql, offset).await,
                (Ok(c), None) => c.execute_in(schema.as_deref(), &sql).await,
                (Err(e), _) => Err(e),
            };
            emit.send(Ev::QueryDone { tab, result });
        }
        Cmd::QueryOrigins { conn, tab, run, sql, schema } => {
            // Quiet on any failure: the grid just has no relations.
            let Ok(c) = get(&conns, &conn).await else { return };
            let Ok(origins) = c.column_origins_in(schema.as_deref(), &sql).await else { return };
            let mut names: Vec<(String, String)> = Vec::new();
            for (s, t, _) in origins.iter().flatten() {
                if !names.iter().any(|(a, b)| a == s && b == t) {
                    names.push((s.clone(), t.clone()));
                }
            }
            let mut tables = Vec::new();
            // Schema-qualified, so the run's search_path doesn't matter here.
            for (s, t) in names {
                let columns = c.columns(&s, &t).await.unwrap_or_default();
                let relations = c.relations(&s, &t).await.unwrap_or_default();
                tables.push((s, t, columns, relations));
            }
            emit.send(Ev::QueryOrigins { tab, run, sql, origins, tables });
        }
        Cmd::CountRows { conn, tab, run, sql, schema, database } => {
            let cfg = conns.cfgs.lock().unwrap().get(&conn).cloned();
            let result = match cfg {
                None => Err("Not connected".to_string()),
                Some(mut cfg) => {
                    if let Some(db) = database {
                        cfg.database = db;
                    }
                    match PgConn::connect(&cfg).await {
                        Ok(c) => c.count_rows_in(schema.as_deref(), &sql).await,
                        Err(e) => Err(e),
                    }
                }
            };
            emit.send(Ev::QueryCount { tab, run, sql, result });
        }
        Cmd::TableData { conn, tab, query, modal } => {
            let c = match get(&conns, &conn).await {
                Ok(c) => c,
                Err(e) => return emit.send(Ev::TablePage { tab, modal, result: Err(e), columns: Vec::new(), relations: Vec::new() }),
            };
            let page = c.table_page(&query).await;
            let ok = page.is_ok();
            let columns = c.columns(&query.schema, &query.table).await.unwrap_or_default();
            let relations = c.relations(&query.schema, &query.table).await.unwrap_or_default();
            emit.send(Ev::TablePage { tab, modal, result: page, columns, relations });
            if ok {
                let count = c.table_count(&query).await;
                emit.send(Ev::TableCount { tab, modal, result: count });
            }
        }
        Cmd::ProbeRelations { conn, tab, modal, key, probes } => {
            let Ok(c) = get(&conns, &conn).await else { return };
            let mut counts = Vec::with_capacity(probes.len());
            for (schema, table, filter) in &probes {
                counts.push(c.count_capped(schema, table, filter, PROBE_CAP).await.ok());
            }
            emit.send(Ev::RelationProbe { tab, modal, key, counts });
        }
        Cmd::TableDef { conn, schema, table } => {
            let result = match get(&conns, &conn).await {
                Ok(c) => match c.columns(&schema, &table).await {
                    Ok(cols) => c.primary_key_name(&schema, &table).await.map(|pk| (cols, pk)),
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            };
            emit.send(Ev::TableDef { conn, schema, table, result });
        }
        Cmd::AlterTable { conn, schema, table, new_name, statements } => {
            let result = match get(&conns, &conn).await {
                Ok(c) => c.alter(&statements.join(";\n")).await,
                Err(e) => Err(e),
            };
            emit.send(Ev::TableAltered { conn, schema, table, new_name, result });
        }
        Cmd::AddKey { conn, tab, sql } => {
            let result = match get(&conns, &conn).await {
                Ok(c) => c.alter(&sql).await,
                Err(e) => Err(e),
            };
            emit.send(Ev::KeyAdded { tab, result });
        }
        Cmd::SaveEdits { conn, tab, statements } => {
            let result = match get(&conns, &conn).await {
                Ok(c) => c.apply_updates(&statements).await,
                Err(e) => Err(e),
            };
            emit.send(Ev::EditsSaved { tab, result });
        }
        Cmd::LayerQuery { conn, tab, layer, seq, kind, sql } => {
            let result = match get(&conns, &conn).await {
                Ok(c) => c.execute_capped(&sql, LAYER_LIMIT).await.map(|out| {
                    let rows: Vec<_> = out.statements.into_iter().filter_map(|s| s.result).flat_map(|rs| {
                        let truncated = rs.truncated;
                        rs.rows.into_iter().map(move |r| (r, truncated))
                    }).collect();
                    let truncated = rows.iter().any(|(_, t)| *t);
                    let geom = |v: Option<&Option<String>>| v.and_then(|v| v.as_deref()).and_then(rgrip_core::geo::parse_hex).and_then(|g| g.to_wgs84());
                    match kind {
                        LayerQuery::Data => {
                            let (mut shapes, mut values) = (Vec::with_capacity(rows.len()), Vec::with_capacity(rows.len()));
                            for (row, _) in &rows {
                                if let Some(g) = geom(row.first()) {
                                    shapes.push(g);
                                    values.push(row.get(1).cloned().flatten());
                                }
                            }
                            LayerResult::Data { shapes, values, truncated }
                        }
                        LayerQuery::Extent => {
                            let mut lo = [f64::MAX; 2];
                            let mut hi = [f64::MIN; 2];
                            if let Some(g) = rows.first().and_then(|(r, _)| geom(r.first())) {
                                g.for_each_coord(&mut |c| {
                                    lo = [lo[0].min(c[0]), lo[1].min(c[1])];
                                    hi = [hi[0].max(c[0]), hi[1].max(c[1])];
                                });
                            }
                            let count = rows.first().and_then(|(r, _)| r.get(1).cloned().flatten()).and_then(|n| n.parse().ok());
                            LayerResult::Extent((lo[0] <= hi[0]).then_some((lo, hi)), count)
                        }
                    }
                }),
                Err(e) => Err(e),
            };
            emit.send(Ev::LayerResult { tab, layer, seq, result });
        }
        Cmd::ExportAll { conn, database, schema, sql, format, table, path } => {
            let result = match get_db(&conns, &conn, database.as_deref()).await {
                Ok(c) => c.export_to_file(schema.as_deref(), &sql, format, &table, &path).await.map(|n| (path, n)),
                Err(e) => Err(e),
            };
            emit.send(Ev::Exported(result));
        }
        Cmd::Import { conn, request } => {
            // A connection of its own: the import's transaction would
            // otherwise swallow whatever else runs on the shared one.
            let cfg = conns.cfgs.lock().unwrap().get(&conn).cloned();
            let result = match cfg {
                None => Err("Not connected".to_string()),
                Some(cfg) => match PgConn::connect(&cfg).await {
                    Ok(c) => c.import(&request, |rows| emit.send(Ev::ImportProgress { conn: conn.clone(), rows })).await,
                    Err(e) => Err(e),
                },
            };
            emit.send(Ev::ImportDone { conn, schema: request.schema, table: request.table, result });
        }
        Cmd::CheckUpdate => emit.send(Ev::UpdateChecked(rgrip_core::update::check().await)),
        Cmd::InstallUpdate { release, app } => {
            let dir = std::env::temp_dir();
            let result = match rgrip_core::update::download(&release, &dir).await {
                Ok(zip) => {
                    let installed = tokio::task::spawn_blocking(move || {
                        let r = rgrip_core::update::install(&zip, &app).map(|_| app);
                        let _ = std::fs::remove_file(&zip);
                        r
                    })
                    .await;
                    installed.unwrap_or_else(|e| Err(e.to_string()))
                }
                Err(e) => Err(e),
            };
            emit.send(Ev::UpdateInstalled(result));
        }
        Cmd::Cancel(conn, database) => {
            // Never opens a connection: a query only runs on an existing one.
            let default = conns.cfgs.lock().unwrap().get(&conn).map(|c| c.database.clone()).unwrap_or_default();
            let target = match extra_key(&conn, database.as_deref(), &default) {
                Some(k) => conns.extra.lock().unwrap().get(&k).cloned(),
                None => current(&conns, &conn),
            };
            if let Some(c) = target {
                let _ = c.cancel().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::extra_key;

    #[test]
    fn main_connection_serves_the_default_database() {
        assert_eq!(extra_key("c", None, "app"), None);
        assert_eq!(extra_key("c", Some("app"), "app"), None);
        assert_eq!(extra_key("c", Some("other"), "app"), Some(("c".into(), "other".into())));
    }
}
