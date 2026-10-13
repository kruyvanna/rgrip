use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
    /// Require TLS and check the server certificate against the CA.
    VerifyCa,
    /// Like `VerifyCa`, and the certificate must match the host name.
    VerifyFull,
}

impl SslMode {
    pub const ALL: [SslMode; 5] =
        [SslMode::Disable, SslMode::Prefer, SslMode::Require, SslMode::VerifyCa, SslMode::VerifyFull];

    pub fn label(self) -> &'static str {
        match self {
            SslMode::Disable => "disable",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify-ca",
            SslMode::VerifyFull => "verify-full",
        }
    }

    pub fn verifies(self) -> bool {
        matches!(self, SslMode::VerifyCa | SslMode::VerifyFull)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshAuth {
    #[default]
    Password,
    Key,
}

/// Optional SSH tunnel: the database host/port are reached from the SSH server.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SshConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    pub password: String,
    /// Private key file (OpenSSH or PEM format).
    pub key_path: String,
    pub passphrase: String,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: 22,
            user: std::env::var("USER").unwrap_or_default(),
            auth: SshAuth::Key,
            password: String::new(),
            key_path: std::env::var("HOME").map(|h| format!("{h}/.ssh/id_ed25519")).unwrap_or_default(),
            passphrase: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub ssl: SslMode,
    /// CA certificate (PEM) used by verify-ca / verify-full.
    #[serde(default)]
    pub ssl_root_cert: String,
    /// Client certificate and PKCS#8 key (PEM) for mutual TLS.
    #[serde(default)]
    pub ssl_client_cert: String,
    #[serde(default)]
    pub ssl_client_key: String,
    #[serde(default)]
    pub ssh: SshConfig,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        Self {
            id: format!("{nanos:x}"),
            name: String::new(),
            host: "localhost".into(),
            port: 5432,
            database: "postgres".into(),
            user: std::env::var("USER").unwrap_or_else(|_| "postgres".into()),
            password: String::new(),
            ssl: SslMode::Prefer,
            ssl_root_cert: String::new(),
            ssl_client_cert: String::new(),
            ssl_client_key: String::new(),
            ssh: SshConfig::default(),
        }
    }
}

impl ConnectionConfig {
    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() {
            format!("{}@{}", self.database, self.host)
        } else {
            self.name.clone()
        }
    }
}

/// What we learned about the server when connecting.
#[derive(Clone, Debug)]
pub struct ServerInfo {
    pub version: String,
    /// TLS protocol in use (e.g. "TLSv1.3"), `None` when unencrypted.
    pub tls: Option<String>,
    pub via_ssh: bool,
}

impl ServerInfo {
    /// e.g. "PostgreSQL 16.4 · TLSv1.3 · SSH"
    pub fn summary(&self) -> String {
        let mut s = self.version.clone();
        s += match &self.tls {
            Some(v) => format!(" · {v}"),
            None => " · no TLS".into(),
        }
        .as_str();
        if self.via_ssh {
            s += " · SSH";
        }
        s
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableKind {
    Table,
    Partitioned,
    View,
    MaterializedView,
    Foreign,
}

impl TableKind {
    pub fn from_relkind(k: &str) -> Self {
        match k {
            "p" => TableKind::Partitioned,
            "v" => TableKind::View,
            "m" => TableKind::MaterializedView,
            "f" => TableKind::Foreign,
            _ => TableKind::Table,
        }
    }

    /// Short tag shown next to non-plain tables in the explorer.
    pub fn tag(self) -> Option<&'static str> {
        match self {
            TableKind::Table => None,
            TableKind::Partitioned => Some("partitioned"),
            TableKind::View => Some("view"),
            TableKind::MaterializedView => Some("matview"),
            TableKind::Foreign => Some("foreign"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TableInfo {
    pub name: String,
    pub kind: TableKind,
}

#[derive(Clone, Debug)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
}

/// A foreign key touching a table, seen from that table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub name: String,
    /// `true`: this table references `other` (posts.user_id → users).
    /// `false`: `other` references this table (users ← posts.user_id).
    pub outgoing: bool,
    pub other_schema: String,
    pub other_table: String,
    /// Column pairs (this table's column, other table's column).
    pub columns: Vec<(String, String)>,
}

/// Rows are kept as text (Postgres text protocol), `None` is SQL NULL.
#[derive(Clone, Debug, Default)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// More rows were returned than we keep in memory.
    pub truncated: bool,
    /// Lazily computed geometry column indexes (see [`ResultSet::geometry_columns`]).
    pub geometry_cache: std::sync::OnceLock<Vec<usize>>,
}

impl ResultSet {
    /// Columns holding PostGIS geometries; computed once per result set.
    pub fn geometry_columns(&self) -> &[usize] {
        self.geometry_cache.get_or_init(|| crate::geo::geometry_columns(self))
    }
}

#[derive(Clone, Debug)]
pub struct StatementResult {
    /// Present for statements that return rows (SELECT, RETURNING, SHOW…).
    pub result: Option<ResultSet>,
    /// Row count reported by CommandComplete.
    pub affected: u64,
}

#[derive(Clone, Debug)]
pub struct QueryOutput {
    pub statements: Vec<StatementResult>,
    pub elapsed: Duration,
}

#[derive(Clone, Debug)]
pub struct TableQuery {
    pub schema: String,
    pub table: String,
    /// Raw SQL placed after WHERE.
    pub filter: String,
    /// Column name and ascending flag.
    pub order: Option<(String, bool)>,
    pub limit: i64,
    pub offset: i64,
}
