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
    /// `database`: `None` = the connection's own database.
    /// `page: Some(offset)` fetches one page of a read-only query.
    Query { conn: ConnId, tab: TabId, sql: String, page: Option<usize>, schema: Option<String>, database: Option<String> },
    /// `modal`: for the tab's related-rows popup rather than the tab itself.
    TableData { conn: ConnId, tab: TabId, query: TableQuery, modal: bool },
    /// Count matches of related-row filters (schema, table, filter) for the
    /// Related menu; `key` comes back with the counts.
    ProbeRelations { conn: ConnId, tab: TabId, modal: bool, key: Vec<usize>, probes: Vec<(String, String, String)> },
    /// Inline-edit UPDATEs, applied in one transaction.
    SaveEdits { conn: ConnId, tab: TabId, statements: Vec<String> },
    Cancel(ConnId, Option<String>),
    /// Look for a newer release.
    CheckUpdate,
    /// Download, verify and install a release over `app`.
    InstallUpdate { release: rgrip_core::update::Release, app: std::path::PathBuf },
    /// Load a file into a table (own connection, one transaction).
    Import { conn: ConnId, request: rgrip_core::import::ImportRequest },
    /// Run `sql` uncapped and write it to `path`.
    ExportAll { conn: ConnId, database: Option<String>, schema: Option<String>, sql: String, format: rgrip_core::export::Format, table: String, path: std::path::PathBuf },
}

pub enum Ev {
    /// `version` is a summary such as "PostgreSQL 16.4 · TLSv1.3 · SSH".
    Connected { conn: ConnId, version: String },
    ConnectFailed { conn: ConnId, error: String },
    TestResult(Result<String, String>),
    Schemas { conn: ConnId, database: Option<String>, result: Result<Vec<String>, String> },
    Databases { conn: ConnId, result: Result<Vec<String>, String> },
    Tables { conn: ConnId, schema: String, result: Result<Vec<TableInfo>, String> },
    Columns { conn: ConnId, schema: String, table: String, result: Result<Vec<ColumnInfo>, String> },
    Catalog { conn: ConnId, database: Option<String>, result: Result<Catalog, String> },
    QueryDone { tab: TabId, result: Result<QueryOutput, String> },
    /// `columns`: the table's columns (primary key and types, for editing).
    /// `relations`: foreign keys from and to the table.
    TablePage { tab: TabId, modal: bool, result: Result<ResultSet, String>, columns: Vec<ColumnInfo>, relations: Vec<Relation> },
    /// Match counts per probe (`None`: failed or timed out).
    RelationProbe { tab: TabId, modal: bool, key: Vec<usize>, counts: Vec<Option<i64>> },
    /// Rows updated, or why nothing was saved.
    EditsSaved { tab: TabId, result: Result<usize, String> },
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

fn get(conns: &Conns, id: &str) -> Result<Arc<PgConn>, String> {
    match conns.main.lock().unwrap().get(id) {
        Some(c) if !c.is_closed() => Ok(c.clone()),
        Some(_) => Err("Connection closed".into()),
        None => Err("Not connected".into()),
    }
}

/// The connection serving `database` of `id`, connecting to it if needed.
async fn get_db(conns: &Conns, id: &str, database: Option<&str>) -> Result<Arc<PgConn>, String> {
    let main = get(conns, id)?;
    let Some(cfg) = conns.cfgs.lock().unwrap().get(id).cloned() else { return Ok(main) };
    let Some(key) = extra_key(id, database, &cfg.database) else { return Ok(main) };
    if let Some(c) = conns.extra.lock().unwrap().get(&key)
        && !c.is_closed()
    {
        return Ok(c.clone());
    }
    let cfg = ConnectionConfig { database: key.1.clone(), ..cfg };
    let c = Arc::new(PgConn::connect(&cfg).await.map_err(|e| format!("Could not connect to database {}: {e}", key.1))?);
    let mut extra = conns.extra.lock().unwrap();
    // A concurrent run may have connected meanwhile: share that one.
    match extra.get(&key) {
        Some(other) if !other.is_closed() => Ok(other.clone()),
        _ => {
            extra.insert(key, c.clone());
            Ok(c)
        }
    }
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
            let result = match get(&conns, &conn) {
                Ok(c) => c.databases().await,
                Err(e) => Err(e),
            };
            emit.send(Ev::Databases { conn, result });
        }
        Cmd::Tables(conn, schema) => {
            let result = match get(&conns, &conn) {
                Ok(c) => c.tables(&schema).await,
                Err(e) => Err(e),
            };
            emit.send(Ev::Tables { conn, schema, result });
        }
        Cmd::Columns(conn, schema, table) => {
            let result = match get(&conns, &conn) {
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
        Cmd::Query { conn, tab, sql, page, schema, database } => {
            let result = match (get_db(&conns, &conn, database.as_deref()).await, page) {
                (Ok(c), Some(offset)) => c.execute_page_in(schema.as_deref(), &sql, offset).await,
                (Ok(c), None) => c.execute_in(schema.as_deref(), &sql).await,
                (Err(e), _) => Err(e),
            };
            emit.send(Ev::QueryDone { tab, result });
        }
        Cmd::TableData { conn, tab, query, modal } => {
            let c = match get(&conns, &conn) {
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
            let Ok(c) = get(&conns, &conn) else { return };
            let mut counts = Vec::with_capacity(probes.len());
            for (schema, table, filter) in &probes {
                counts.push(c.count_capped(schema, table, filter, PROBE_CAP).await.ok());
            }
            emit.send(Ev::RelationProbe { tab, modal, key, counts });
        }
        Cmd::SaveEdits { conn, tab, statements } => {
            let result = match get(&conns, &conn) {
                Ok(c) => c.apply_updates(&statements).await,
                Err(e) => Err(e),
            };
            emit.send(Ev::EditsSaved { tab, result });
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
                None => get(&conns, &conn).ok(),
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
