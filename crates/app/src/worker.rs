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
    TableData { conn: ConnId, tab: TabId, query: TableQuery },
    Cancel(ConnId),
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
    TablePage { tab: TabId, result: Result<ResultSet, String> },
    TableCount { tab: TabId, result: Result<i64, String> },
    /// Path and row count, or an error.
    Exported(Result<(std::path::PathBuf, usize), String>),
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
        Cmd::TableData { conn, tab, query } => {
            let c = match get(&conns, &conn) {
                Ok(c) => c,
                Err(e) => return emit.send(Ev::TablePage { tab, result: Err(e) }),
            };
            let page = c.table_page(&query).await;
            let ok = page.is_ok();
            emit.send(Ev::TablePage { tab, result: page });
            if ok {
                let count = c.table_count(&query).await;
                emit.send(Ev::TableCount { tab, result: count });
            }
        }
        Cmd::ExportAll { conn, sql, format, table, path } => {
            let result = match get(&conns, &conn) {
                Ok(c) => c.export_to_file(&sql, format, &table, &path).await.map(|n| (path, n)),
                Err(e) => Err(e),
            };
            emit.send(Ev::Exported(result));
        }
        Cmd::Cancel(conn) => {
            if let Ok(c) = get(&conns, &conn) {
                let _ = c.cancel().await;
            }
        }
    }
}
