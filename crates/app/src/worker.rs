//! Background tokio runtime that owns all database connections.
//! The UI sends `Cmd`s and polls `Ev`s every frame, so it never blocks.

use rgrip_core::complete::Catalog;
use rgrip_core::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

pub type ConnId = String;
pub type TabId = u64;

pub enum Cmd {
    Connect(ConnectionConfig),
    Disconnect(ConnId),
    Test(ConnectionConfig),
    Schemas(ConnId),
    Tables(ConnId, String),
    Columns(ConnId, String, String),
    Catalog(ConnId),
    /// `page: Some(offset)` fetches one page of a read-only query.
    Query { conn: ConnId, tab: TabId, sql: String, page: Option<usize> },
    /// `modal`: for the tab's related-rows popup rather than the tab itself.
    TableData { conn: ConnId, tab: TabId, query: TableQuery, modal: bool },
    /// Inline-edit UPDATEs, applied in one transaction.
    SaveEdits { conn: ConnId, tab: TabId, statements: Vec<String> },
    Cancel(ConnId),
    /// Look for a newer release.
    CheckUpdate,
    /// Download, verify and install a release over `app`.
    InstallUpdate { release: rgrip_core::update::Release, app: std::path::PathBuf },
    /// Run `sql` uncapped and write it to `path`.
    ExportAll { conn: ConnId, sql: String, format: rgrip_core::export::Format, table: String, path: std::path::PathBuf },
}

pub enum Ev {
    /// `version` is a summary such as "PostgreSQL 16.4 · TLSv1.3 · SSH".
    Connected { conn: ConnId, version: String },
    ConnectFailed { conn: ConnId, error: String },
    TestResult(Result<String, String>),
    Schemas { conn: ConnId, result: Result<Vec<String>, String> },
    Tables { conn: ConnId, schema: String, result: Result<Vec<TableInfo>, String> },
    Columns { conn: ConnId, schema: String, table: String, result: Result<Vec<ColumnInfo>, String> },
    Catalog { conn: ConnId, result: Result<Catalog, String> },
    QueryDone { tab: TabId, result: Result<QueryOutput, String> },
    /// `columns`: the table's columns (primary key and types, for editing).
    /// `relations`: foreign keys from and to the table.
    TablePage { tab: TabId, modal: bool, result: Result<ResultSet, String>, columns: Vec<ColumnInfo>, relations: Vec<Relation> },
    /// Rows updated, or why nothing was saved.
    EditsSaved { tab: TabId, result: Result<usize, String> },
    TableCount { tab: TabId, modal: bool, result: Result<i64, String> },
    /// Path and row count, or an error.
    Exported(Result<(std::path::PathBuf, usize), String>),
    UpdateChecked(Result<Option<rgrip_core::update::Release>, String>),
    /// The bundle that was updated (ready to relaunch), or an error.
    UpdateInstalled(Result<std::path::PathBuf, String>),
}

pub struct Worker {
    tx: UnboundedSender<Cmd>,
    rx: mpsc::Receiver<Ev>,
}

type Conns = Arc<Mutex<HashMap<ConnId, Arc<PgConn>>>>;

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
    match conns.lock().unwrap().get(id) {
        Some(c) if !c.is_closed() => Ok(c.clone()),
        Some(_) => Err("Connection closed".into()),
        None => Err("Not connected".into()),
    }
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
                    conns.lock().unwrap().insert(cfg.id.clone(), Arc::new(c));
                    emit.send(Ev::Connected { conn: cfg.id, version });
                }
                Err(error) => emit.send(Ev::ConnectFailed { conn: cfg.id, error }),
            }
        }
        Cmd::Disconnect(id) => {
            // Dropping the client closes the connection.
            conns.lock().unwrap().remove(&id);
        }
        Cmd::Test(cfg) => {
            let result = match PgConn::connect(&cfg).await {
                Ok(c) => c.server_info().await.map(|i| i.summary()),
                Err(e) => Err(e),
            };
            emit.send(Ev::TestResult(result));
        }
        Cmd::Schemas(conn) => {
            let result = match get(&conns, &conn) {
                Ok(c) => c.schemas().await,
                Err(e) => Err(e),
            };
            emit.send(Ev::Schemas { conn, result });
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
        Cmd::Catalog(conn) => {
            let result = match get(&conns, &conn) {
                Ok(c) => c.catalog().await,
                Err(e) => Err(e),
            };
            emit.send(Ev::Catalog { conn, result });
        }
        Cmd::Query { conn, tab, sql, page } => {
            let result = match (get(&conns, &conn), page) {
                (Ok(c), Some(offset)) => c.execute_page(&sql, offset).await,
                (Ok(c), None) => c.execute(&sql).await,
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
        Cmd::SaveEdits { conn, tab, statements } => {
            let result = match get(&conns, &conn) {
                Ok(c) => c.apply_updates(&statements).await,
                Err(e) => Err(e),
            };
            emit.send(Ev::EditsSaved { tab, result });
        }
        Cmd::ExportAll { conn, sql, format, table, path } => {
            let result = match get(&conns, &conn) {
                Ok(c) => c.export_to_file(&sql, format, &table, &path).await.map(|n| (path, n)),
                Err(e) => Err(e),
            };
            emit.send(Ev::Exported(result));
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
        Cmd::Cancel(conn) => {
            if let Ok(c) = get(&conns, &conn) {
                let _ = c.cancel().await;
            }
        }
    }
}
