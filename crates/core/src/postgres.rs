use crate::model::*;
use crate::relations::Origin;
use crate::sql::quote_ident;
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use std::sync::Arc;
use tokio_postgres_rustls::MakeRustlsConnect;
use std::time::{Duration, Instant};
use tokio_postgres::error::ErrorPosition;
use tokio_postgres::{CancelToken, Client, SimpleQueryMessage};

/// Rows shown per result set (and per page); the rest are dropped and
/// `truncated` is set.
pub const PAGE_SIZE: usize = 1000;

pub struct PgConn {
    client: Client,
    cancel: CancelToken,
    tls: MakeRustlsConnect,
    /// Kept alive for as long as the connection.
    tunnel: Option<crate::ssh::Tunnel>,
    /// Held from setting `search_path` to the end of the query, so consoles
    /// sharing this connection can't change each other's schema in between.
    schema_lock: tokio::sync::Mutex<()>,
    /// When [`PgConn::alive`] last vouched for the connection.
    checked: std::sync::Mutex<std::time::Instant>,
}

fn tls_err(what: &str) -> impl Fn(rustls::Error) -> String + '_ {
    move |e| format!("{what}: {e}")
}

fn pem_err(what: &str, path: &str) -> impl Fn(rustls::pki_types::pem::Error) -> String {
    let msg = format!("Cannot read {what} {path}");
    move |e| format!("{msg}: {e}")
}

/// TLS setup with libpq semantics:
/// - disable/prefer/require encrypt without verifying the certificate;
/// - verify-ca checks the chain against the CA (or system roots);
/// - verify-full also checks the host name.
fn tls(cfg: &ConnectionConfig) -> Result<MakeRustlsConnect, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(tls_err("TLS"))?;

    let builder = if cfg.ssl.verifies() {
        let mut roots = RootCertStore::empty();
        let root = cfg.ssl_root_cert.trim();
        if root.is_empty() {
            for cert in rustls_native_certs::load_native_certs().certs {
                let _ = roots.add(cert);
            }
        } else {
            for cert in CertificateDer::pem_file_iter(root).map_err(pem_err("CA certificate", root))? {
                roots.add(cert.map_err(pem_err("CA certificate", root))?).map_err(tls_err("CA certificate"))?;
            }
        }
        let verifier = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
            .build()
            .map_err(|e| format!("CA certificate: {e}"))?;
        if cfg.ssl == SslMode::VerifyFull {
            builder.with_webpki_verifier(verifier)
        } else {
            builder.dangerous().with_custom_certificate_verifier(Arc::new(IgnoreHostname(verifier)))
        }
    } else {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider)))
    };

    let cert_path = cfg.ssl_client_cert.trim();
    let config = if cert_path.is_empty() {
        builder.with_no_client_auth()
    } else {
        let key_path = cfg.ssl_client_key.trim();
        let certs = CertificateDer::pem_file_iter(cert_path)
            .map_err(pem_err("client certificate", cert_path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(pem_err("client certificate", cert_path))?;
        let key = PrivateKeyDer::from_pem_file(key_path).map_err(pem_err("client key", key_path))?;
        builder.with_client_auth_cert(certs, key).map_err(tls_err("Client certificate"))?
    };
    Ok(MakeRustlsConnect::new(config))
}

/// Encrypt without authenticating the server (sslmode=require).
/// Handshake signatures are still checked.
#[derive(Debug)]
struct AcceptAnyCert(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Full chain verification, but any host name is accepted (sslmode=verify-ca).
#[derive(Debug)]
struct IgnoreHostname(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for IgnoreHostname {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.0.verify_server_cert(end_entity, intermediates, server_name, ocsp, now) {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

impl PgConn {
    /// Must be called from within a tokio runtime; the connection task is spawned onto it.
    pub async fn connect(cfg: &ConnectionConfig) -> Result<Self, String> {
        let tunnel = if cfg.ssh.enabled {
            Some(crate::ssh::open(&cfg.ssh, &cfg.host, cfg.port).await?)
        } else {
            None
        };

        let mut pg = tokio_postgres::Config::new();
        // The host name is kept for TLS verification even when tunnelling.
        pg.host(&cfg.host)
            .dbname(&cfg.database)
            .user(&cfg.user)
            .application_name("rgrip")
            .connect_timeout(Duration::from_secs(10))
            // Notice a dead socket (laptop slept, network changed) on its own.
            .keepalives_idle(Duration::from_secs(30))
            .ssl_mode(match cfg.ssl {
                SslMode::Disable => tokio_postgres::config::SslMode::Disable,
                SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
                _ => tokio_postgres::config::SslMode::Require,
            });
        match &tunnel {
            Some(t) => {
                pg.hostaddr(std::net::IpAddr::from([127, 0, 0, 1])).port(t.local_port);
            }
            None => {
                pg.port(cfg.port);
            }
        }
        if !cfg.password.is_empty() {
            pg.password(&cfg.password);
        }

        let tls = tls(cfg)?;
        let (client, connection) = pg.connect(tls.clone()).await.map_err(|e| format_error(&e, None))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        if cfg.read_only {
            // Server-side, so every query on this session is refused, not just ours.
            client.batch_execute("set session characteristics as transaction read only").await.map_err(|e| format_error(&e, None))?;
        }
        let cancel = client.cancel_token();
        Ok(Self { client, cancel, tls, tunnel, schema_lock: Default::default(), checked: std::sync::Mutex::new(std::time::Instant::now()) })
    }

    pub async fn server_info(&self) -> Result<ServerInfo, String> {
        let row = self
            .client
            .query_one(
                "select current_setting('server_version'),
                        (select version from pg_stat_ssl where pid = pg_backend_pid() and ssl)",
                &[],
            )
            .await
            .map_err(|e| format_error(&e, None))?;
        Ok(ServerInfo {
            // "16.4 (Debian 16.4-1.pgdg110+2)" → "16.4"
            version: format!("PostgreSQL {}", row.get::<_, String>(0).split_whitespace().next().unwrap_or_default()),
            tls: row.get(1),
            via_ssh: self.tunnel.is_some(),
        })
    }

    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    /// Open, and after a quiet minute also answering a ping within 5 s: a
    /// socket that died while the laptop slept can still look open.
    pub async fn alive(&self) -> bool {
        const QUIET: Duration = Duration::from_secs(60);
        if self.is_closed() {
            return false;
        }
        {
            let mut checked = self.checked.lock().unwrap();
            if checked.elapsed() < QUIET {
                *checked = std::time::Instant::now();
                return true;
            }
        }
        let ok = matches!(tokio::time::timeout(Duration::from_secs(5), self.client.simple_query("")).await, Ok(Ok(_)));
        if ok {
            *self.checked.lock().unwrap() = std::time::Instant::now();
        }
        ok
    }

    pub async fn schemas(&self) -> Result<Vec<String>, String> {
        let rows = self
            .client
            .query(
                "select nspname::text from pg_namespace
                 where nspname not like 'pg\\_%' and nspname <> 'information_schema'
                 order by nspname = 'public' desc, nspname",
                &[],
            )
            .await
            .map_err(|e| format_error(&e, None))?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    /// Databases of the server a console could be pointed at.
    pub async fn databases(&self) -> Result<Vec<String>, String> {
        let rows = self
            .client
            .query("select datname::text from pg_database where not datistemplate and datallowconn order by 1", &[])
            .await
            .map_err(|e| format_error(&e, None))?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn tables(&self, schema: &str) -> Result<Vec<TableInfo>, String> {
        let rows = self
            .client
            .query(
                "select c.relname::text, c.relkind::text
                 from pg_class c join pg_namespace n on n.oid = c.relnamespace
                 where n.nspname = $1 and c.relkind in ('r','p','v','m','f')
                   and not c.relispartition
                 order by c.relkind in ('v','m'), c.relname",
                &[&schema],
            )
            .await
            .map_err(|e| format_error(&e, None))?;
        Ok(rows
            .iter()
            .map(|r| TableInfo {
                name: r.get(0),
                kind: TableKind::from_relkind(r.get::<_, &str>(1)),
            })
            .collect())
    }

    /// How many rows of `schema.table` match `filter`, counting at most
    /// `cap`. Bounded by a short statement timeout so a guessed relation on
    /// a huge unindexed table can't stall the connection.
    pub async fn count_capped(&self, schema: &str, table: &str, filter: &str, cap: usize) -> Result<i64, String> {
        let sql = format!(
            "set statement_timeout = 2000; select count(*) from (select 1 from {} where {filter} limit {cap}) x; reset statement_timeout",
            qualified(schema, table)
        );
        let result = self.client.simple_query(&sql).await;
        if result.is_err() {
            let _ = self.client.batch_execute("reset statement_timeout").await;
        }
        let messages = result.map_err(|e| format_error(&e, None))?;
        messages
            .iter()
            .find_map(|m| match m {
                SimpleQueryMessage::Row(r) => r.get(0).and_then(|v| v.parse().ok()),
                _ => None,
            })
            .ok_or_else(|| "no count".to_string())
    }

    /// Foreign keys from and to a table (a self-reference appears both ways).
    pub async fn relations(&self, schema: &str, table: &str) -> Result<Vec<Relation>, String> {
        // `cols(rel, keys)`: attribute names of `keys` in `rel`, in key order.
        let rows = self
            .client
            .query(
                "with t as (
                   select c.oid from pg_class c join pg_namespace n on n.oid = c.relnamespace
                   where n.nspname = $1 and c.relname = $2
                 ), fk as (
                   select con.conname, true as outgoing, con.confrelid as other,
                          con.conrelid as here_rel, con.conkey as here, con.confkey as there
                   from pg_constraint con join t on con.conrelid = t.oid where con.contype = 'f'
                   union all
                   select con.conname, false, con.conrelid,
                          con.confrelid, con.confkey, con.conkey
                   from pg_constraint con join t on con.confrelid = t.oid where con.contype = 'f'
                 )
                 select fk.conname::text, fk.outgoing, n.nspname::text, c.relname::text,
                        array(select a.attname::text from unnest(fk.here) with ordinality k(num, i)
                              join pg_attribute a on a.attrelid = fk.here_rel and a.attnum = k.num order by k.i),
                        array(select a.attname::text from unnest(fk.there) with ordinality k(num, i)
                              join pg_attribute a on a.attrelid = fk.other and a.attnum = k.num order by k.i)
                 from fk
                 join pg_class c on c.oid = fk.other
                 join pg_namespace n on n.oid = c.relnamespace
                 order by fk.outgoing desc, c.relname, fk.conname",
                &[&schema, &table],
            )
            .await
            .map_err(|e| format_error(&e, None))?;
        Ok(rows
            .iter()
            .map(|r| {
                let here: Vec<String> = r.get(4);
                let there: Vec<String> = r.get(5);
                Relation {
                    name: r.get(0),
                    outgoing: r.get(1),
                    other_schema: r.get(2),
                    other_table: r.get(3),
                    columns: here.into_iter().zip(there).collect(),
                    inferred: false,
                    alternatives: Vec::new(),
                }
            })
            .collect())
    }

    pub async fn columns(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>, String> {
        let rows = self
            .client
            .query(
                "select a.attname::text,
                        format_type(a.atttypid, a.atttypmod),
                        not a.attnotnull,
                        pg_get_expr(d.adbin, d.adrelid),
                        exists (select 1 from pg_index i
                                where i.indrelid = a.attrelid and i.indisprimary
                                  and a.attnum = any(i.indkey))
                 from pg_attribute a
                 join pg_class c on c.oid = a.attrelid
                 join pg_namespace n on n.oid = c.relnamespace
                 left join pg_attrdef d on d.adrelid = a.attrelid and d.adnum = a.attnum
                 where n.nspname = $1 and c.relname = $2
                   and a.attnum > 0 and not a.attisdropped
                 order by a.attnum",
                &[&schema, &table],
            )
            .await
            .map_err(|e| format_error(&e, None))?;
        Ok(rows
            .iter()
            .map(|r| ColumnInfo {
                name: r.get(0),
                data_type: r.get(1),
                nullable: r.get(2),
                default: r.get(3),
                primary_key: r.get(4),
            })
            .collect())
    }

    /// Every relation with its columns, for autocompletion. Includes
    /// pg_catalog/information_schema so system views complete too.
    pub async fn catalog(&self) -> Result<crate::complete::Catalog, String> {
        let rows = self
            .client
            .query(
                "select n.nspname::text, c.relname::text, c.relkind::text,
                        coalesce(array_agg(a.attname::text order by a.attnum)
                                 filter (where a.attnum is not null), '{}'),
                        coalesce(array_agg(format_type(a.atttypid, a.atttypmod) order by a.attnum)
                                 filter (where a.attnum is not null), '{}'),
                        array(select k.attname::text from pg_index i
                              join pg_attribute k on k.attrelid = i.indrelid and k.attnum = i.indkey[0]
                              where i.indrelid = c.oid and i.indisunique and i.indnatts = 1
                                and i.indexprs is null and i.indpred is null)
                 from pg_class c
                 join pg_namespace n on n.oid = c.relnamespace
                 left join pg_attribute a
                   on a.attrelid = c.oid and a.attnum > 0 and not a.attisdropped
                 where c.relkind in ('r','p','v','m','f') and not c.relispartition
                   and n.nspname not like 'pg\\_toast%' and n.nspname not like 'pg\\_temp%'
                 group by c.oid, 1, 2, 3
                 order by 1, 2",
                &[],
            )
            .await
            .map_err(|e| format_error(&e, None))?;
        Ok(crate::complete::Catalog {
            tables: rows
                .iter()
                .map(|r| {
                    let names: Vec<String> = r.get(3);
                    let types: Vec<String> = r.get(4);
                    crate::complete::CatalogTable {
                        schema: r.get(0),
                        name: r.get(1),
                        kind: TableKind::from_relkind(r.get::<_, &str>(2)),
                        columns: names.into_iter().zip(types).collect(),
                        keys: r.get(5),
                    }
                })
                .collect(),
        })
    }

    /// The tables of `schema` with their columns and foreign keys, plus
    /// guessed links against the whole database's catalog.
    pub async fn schema_diagram(&self, schema: &str) -> Result<crate::diagram::Diagram, String> {
        let rows = self
            .client
            .query(
                "select c.relname::text, c.relkind::text, a.attname::text,
                        format_type(a.atttypid, a.atttypmod), not a.attnotnull,
                        exists (select 1 from pg_index i
                                where i.indrelid = a.attrelid and i.indisprimary
                                  and a.attnum = any(i.indkey))
                 from pg_class c
                 join pg_namespace n on n.oid = c.relnamespace
                 join pg_attribute a on a.attrelid = c.oid and a.attnum > 0 and not a.attisdropped
                 where n.nspname = $1 and c.relkind in ('r','p','v','m','f') and not c.relispartition
                 order by c.relname, a.attnum",
                &[&schema],
            )
            .await
            .map_err(|e| format_error(&e, None))?;
        let mut tables: Vec<crate::diagram::DiagramTable> = Vec::new();
        for r in &rows {
            let name: String = r.get(0);
            if tables.last().is_none_or(|t| t.name != name) {
                tables.push(crate::diagram::DiagramTable { name, kind: TableKind::from_relkind(r.get::<_, &str>(1)), columns: Vec::new() });
            }
            tables.last_mut().unwrap().columns.push(ColumnInfo { name: r.get(2), data_type: r.get(3), nullable: r.get(4), default: None, primary_key: r.get(5) });
        }
        let rows = self
            .client
            .query(
                "select src.relname::text, con.conname::text, n.nspname::text, c.relname::text,
                        array(select a.attname::text from unnest(con.conkey) with ordinality k(num, i)
                              join pg_attribute a on a.attrelid = con.conrelid and a.attnum = k.num order by k.i),
                        array(select a.attname::text from unnest(con.confkey) with ordinality k(num, i)
                              join pg_attribute a on a.attrelid = con.confrelid and a.attnum = k.num order by k.i)
                 from pg_constraint con
                 join pg_class src on src.oid = con.conrelid
                 join pg_namespace sn on sn.oid = src.relnamespace
                 join pg_class c on c.oid = con.confrelid
                 join pg_namespace n on n.oid = c.relnamespace
                 where con.contype = 'f' and sn.nspname = $1
                 order by src.relname, con.conname",
                &[&schema],
            )
            .await
            .map_err(|e| format_error(&e, None))?;
        let fks = rows
            .iter()
            .map(|r| {
                let here: Vec<String> = r.get(4);
                let there: Vec<String> = r.get(5);
                let rel = Relation {
                    name: r.get(1),
                    outgoing: true,
                    other_schema: r.get(2),
                    other_table: r.get(3),
                    columns: here.into_iter().zip(there).collect(),
                    inferred: false,
                    alternatives: Vec::new(),
                };
                (r.get::<_, String>(0), rel)
            })
            .collect();
        let catalog = self.catalog().await?;
        Ok(crate::diagram::assemble(schema, tables, fks, &catalog))
    }

    /// Run one or more statements through the simple query protocol.
    /// All values come back as text, which is exactly what a grid needs.
    /// Result sets are capped at [`PAGE_SIZE`] rows.
    pub async fn execute(&self, sql: &str) -> Result<QueryOutput, String> {
        self.execute_capped(sql, PAGE_SIZE).await
    }

    /// Like [`execute`](Self::execute) with a custom row cap (exports use `usize::MAX`).
    pub async fn execute_capped(&self, sql: &str, cap: usize) -> Result<QueryOutput, String> {
        let started = Instant::now();
        let messages = self
            .client
            .simple_query(sql)
            .await
            .map_err(|e| format_error_shifted(&e, sql, 0))?;
        Ok(QueryOutput {
            statements: collect_results(messages, cap),
            elapsed: started.elapsed(),
        })
    }

    /// Point `search_path` at `schema` (then `public`); `None` restores the
    /// session default. Postgres accepts a missing schema in `search_path`,
    /// so existence is checked in the same round trip.
    /// `schema`: one schema, or several separated by newlines (searched in
    /// that order, then `public`). Checks they all still exist.
    async fn set_schema(&self, schema: Option<&str>) -> Result<(), String> {
        let Some(schema) = schema else {
            return self.client.batch_execute("reset search_path").await.map_err(|e| format_error(&e, None));
        };
        let list: Vec<&str> = schema.split('\n').filter(|s| !s.is_empty()).collect();
        let mut path: Vec<String> = list.iter().map(|s| if *s == "public" { "public".to_string() } else { crate::sql::quote_ident(s) }).collect();
        if !list.contains(&"public") {
            path.push("public".into());
        }
        let literals: Vec<String> = list.iter().map(|s| format!("'{}'", s.replace('\'', "''"))).collect();
        let sql = format!(
            "set search_path to {}; select nspname::text from pg_namespace where nspname in ({})",
            path.join(", "),
            literals.join(", ")
        );
        let messages = self.client.simple_query(&sql).await.map_err(|e| format_error(&e, None))?;
        let found: Vec<String> = messages
            .iter()
            .filter_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(r) => r.get(0).map(str::to_string),
                _ => None,
            })
            .collect();
        let missing: Vec<&str> = list.iter().copied().filter(|s| !found.iter().any(|f| f == s)).collect();
        match missing.as_slice() {
            [] => Ok(()),
            [one] => Err(format!("Schema '{one}' no longer exists. Pick another schema in the toolbar.")),
            many => Err(format!("Schemas {} no longer exist. Pick others in the toolbar.", many.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", "))),
        }
    }

    /// [`execute`](Self::execute) with unqualified names resolved in `schema`.
    pub async fn execute_in(&self, schema: Option<&str>, sql: &str) -> Result<QueryOutput, String> {
        let _guard = self.schema_lock.lock().await;
        self.set_schema(schema).await?;
        self.execute(sql).await
    }

    /// Ask for the plan of one statement (`explain` is an EXPLAIN … FORMAT JSON
    /// statement). With `rollback` it runs inside a transaction that is always
    /// rolled back, for ANALYZE of a statement that writes.
    pub async fn explain_in(&self, schema: Option<&str>, explain: &str, rollback: bool) -> Result<crate::explain::Plan, String> {
        let _guard = self.schema_lock.lock().await;
        self.set_schema(schema).await?;
        let sql = if rollback { crate::explain::rolled_back_sql(explain) } else { explain.to_string() };
        let out = self.execute(&sql).await;
        if rollback && out.is_err() {
            // A failed statement leaves the transaction open (aborted): close it.
            let _ = self.client.batch_execute("rollback").await;
        }
        let out = out?;
        crate::explain::plan_from_output(&out).ok_or_else(|| "Postgres did not return a plan".to_string())
    }

    /// [`execute_page`](Self::execute_page) with unqualified names resolved in `schema`.
    pub async fn execute_page_in(&self, schema: Option<&str>, stmt: &str, offset: usize) -> Result<QueryOutput, String> {
        let _guard = self.schema_lock.lock().await;
        self.set_schema(schema).await?;
        self.execute_page(stmt, offset).await
    }

    /// Where each column of a single read-only `stmt` is read from, found by
    /// preparing it (nothing runs). `None` for computed columns.
    pub async fn column_origins_in(&self, schema: Option<&str>, stmt: &str) -> Result<Vec<Option<Origin>>, String> {
        let prepared = {
            let _guard = self.schema_lock.lock().await;
            self.set_schema(schema).await?;
            self.client.prepare(stmt).await.map_err(|e| format_error(&e, None))?
        };
        let (mut index, mut oids, mut nums) = (Vec::<i32>::new(), Vec::<i64>::new(), Vec::<i16>::new());
        for (i, c) in prepared.columns().iter().enumerate() {
            if let (Some(oid), Some(num)) = (c.table_oid().filter(|o| *o != 0), c.column_id()) {
                index.push(i as i32);
                oids.push(oid as i64);
                nums.push(num);
            }
        }
        let mut out = vec![None; prepared.columns().len()];
        if index.is_empty() {
            return Ok(out);
        }
        let rows = self
            .client
            .query(
                "select k.i, n.nspname::text, c.relname::text, a.attname::text
                 from unnest($1::int4[], $2::int8[], $3::int2[]) k(i, oid, num)
                 join pg_attribute a on a.attrelid = k.oid::oid and a.attnum = k.num
                 join pg_class c on c.oid = a.attrelid
                 join pg_namespace n on n.oid = c.relnamespace",
                &[&index, &oids, &nums],
            )
            .await
            .map_err(|e| format_error(&e, None))?;
        for r in &rows {
            out[r.get::<_, i32>(0) as usize] = Some((r.get(1), r.get(2), r.get(3)));
        }
        Ok(out)
    }

    /// How many rows `stmt` returns. Meant for a connection of its own: it
    /// can run long, so it gives up after a minute.
    pub async fn count_rows_in(&self, schema: Option<&str>, stmt: &str) -> Result<i64, String> {
        let sql = crate::sql::count_query(stmt).ok_or("This statement can't be counted")?;
        self.set_schema(schema).await?;
        self.client.batch_execute("set statement_timeout = 60000").await.map_err(|e| format_error(&e, None))?;
        let row = self.client.query_one(&sql, &[]).await.map_err(|e| format_error(&e, None))?;
        Ok(row.get(0))
    }

    /// One page of a read-only query, fetched server-side with LIMIT/OFFSET.
    /// `truncated` on the result means there is a next page. Statements that
    /// can't be paged run normally (capped).
    pub async fn execute_page(&self, stmt: &str, offset: usize) -> Result<QueryOutput, String> {
        let Some((wrapped, shift)) = crate::sql::paged_query(stmt, PAGE_SIZE + 1, offset) else {
            return self.execute(stmt).await;
        };
        let started = Instant::now();
        let messages = self
            .client
            .simple_query(&wrapped)
            .await
            .map_err(|e| format_error_shifted(&e, stmt, shift))?;
        Ok(QueryOutput {
            statements: collect_results(messages, PAGE_SIZE),
            elapsed: started.elapsed(),
        })
    }

    pub async fn table_page(&self, q: &TableQuery) -> Result<ResultSet, String> {
        let sql = format!("{} limit {} offset {}", table_select_sql(q), q.limit, q.offset);
        let out = self.execute(&sql).await?;
        Ok(out
            .statements
            .into_iter()
            .find_map(|s| s.result)
            .unwrap_or_default())
    }

    /// Run `sql` without a row cap and write its first result set to `path`.
    /// Returns the number of rows written.
    pub async fn export_to_file(
        &self,
        schema: Option<&str>,
        sql: &str,
        format: crate::export::Format,
        table: &str,
        path: &std::path::Path,
    ) -> Result<usize, String> {
        let out = {
            let _guard = self.schema_lock.lock().await;
            self.set_schema(schema).await?;
            self.execute_capped(sql, usize::MAX).await?
        };
        let rs = out.statements.into_iter().find_map(|s| s.result).ok_or("The query returned no rows to export")?;
        let text = crate::export::export(&rs, format, table);
        std::fs::write(path, text).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
        Ok(rs.rows.len())
    }

    pub async fn table_count(&self, q: &TableQuery) -> Result<i64, String> {
        let mut sql = format!("select count(*) from {}", qualified(&q.schema, &q.table));
        push_where(&mut sql, &q.filter);
        let row = self
            .client
            .query_one(&sql, &[])
            .await
            .map_err(|e| format_error(&e, None))?;
        Ok(row.get(0))
    }

    /// Run inline-edit UPDATEs in one transaction. Each must change exactly
    /// one row (the row may have changed or gone since it was loaded);
    /// otherwise nothing is saved. Returns the number of rows updated.
    pub async fn apply_updates(&self, statements: &[String]) -> Result<usize, String> {
        let batch = format!("begin;\n{}", statements.join("\n"));
        let messages = match self.client.simple_query(&batch).await {
            Ok(m) => m,
            Err(e) => {
                let _ = self.client.batch_execute("rollback").await;
                return Err(format_error(&e, None));
            }
        };
        // The first count is BEGIN's.
        let counts: Vec<u64> = messages
            .iter()
            .filter_map(|m| match m {
                SimpleQueryMessage::CommandComplete(n) => Some(*n),
                _ => None,
            })
            .skip(1)
            .collect();
        if let Some(i) = counts.iter().position(|&n| n != 1) {
            let _ = self.client.batch_execute("rollback").await;
            return Err(format!(
                "Nothing saved: change {} matched {} rows instead of 1 (was the row changed or deleted?)",
                i + 1,
                counts[i]
            ));
        }
        self.client.batch_execute("commit").await.map_err(|e| format_error(&e, None))?;
        Ok(counts.len())
    }

    /// Name of a table's primary key constraint, if it has one.
    pub async fn primary_key_name(&self, schema: &str, table: &str) -> Result<Option<String>, String> {
        let rows = self
            .client
            .query(
                "select con.conname::text from pg_constraint con
                 join pg_class c on c.oid = con.conrelid
                 join pg_namespace n on n.oid = c.relnamespace
                 where n.nspname = $1 and c.relname = $2 and con.contype = 'p'",
                &[&schema, &table],
            )
            .await
            .map_err(|e| format_error(&e, None))?;
        Ok(rows.first().map(|r| r.get(0)))
    }

    /// A schema change: unlike `apply_updates`, no row count is expected.
    /// Several statements run as one implicit transaction (all or nothing).
    pub async fn alter(&self, sql: &str) -> Result<(), String> {
        self.client.batch_execute(sql).await.map_err(|e| format_error(&e, None))
    }

    /// Load a file into a table, all or nothing. `progress` gets the rows
    /// sent so far. The caller should use a connection of its own: the
    /// transaction would otherwise swallow other queries on a shared one.
    pub async fn import(&self, req: &crate::import::ImportRequest, mut progress: impl FnMut(usize)) -> Result<usize, String> {
        use futures_util::SinkExt;
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let _reader = crate::import::spawn_encoder(req.clone(), tx);
        self.client.batch_execute("begin").await.map_err(|e| format_error(&e, None))?;
        let load = async {
            let db = |e: tokio_postgres::Error| import_error(&e);
            if req.create {
                self.client.batch_execute(&crate::import::create_table_sql(req)).await.map_err(db)?;
            }
            let sink = self.client.copy_in::<_, bytes::Bytes>(&crate::import::copy_sql(req)).await.map_err(db)?;
            futures_util::pin_mut!(sink);
            let mut sent = 0;
            while let Some(chunk) = rx.recv().await {
                // A file error ends the load here; dropping the sink aborts the COPY.
                let (data, rows) = chunk?;
                sink.send(data).await.map_err(db)?;
                sent += rows;
                progress(sent);
            }
            let copied = sink.finish().await.map_err(db)?;
            for stmt in crate::import::index_sql(req) {
                self.client.batch_execute(&stmt).await.map_err(db)?;
            }
            Ok::<_, String>(copied as usize)
        }
        .await;
        match load {
            Ok(n) => {
                self.client.batch_execute("commit").await.map_err(|e| format_error(&e, None))?;
                Ok(n)
            }
            Err(e) => {
                let _ = self.client.batch_execute("rollback").await;
                Err(e)
            }
        }
    }

    pub async fn cancel(&self) -> Result<(), String> {
        self.cancel.cancel_query(self.tls.clone()).await.map_err(|e| e.to_string())
    }
}

/// `select * from schema.table [where …] [order by …]`, without paging.
pub fn table_select_sql(q: &TableQuery) -> String {
    let mut sql = format!("select * from {}", qualified(&q.schema, &q.table));
    push_where(&mut sql, &q.filter);
    if let Some((col, asc)) = &q.order {
        sql += &format!(" order by {} {}", quote_ident(col), if *asc { "asc" } else { "desc" });
    }
    sql
}

fn qualified(schema: &str, table: &str) -> String {
    format!("{}.{}", quote_ident(schema), quote_ident(table))
}

fn push_where(sql: &mut String, filter: &str) {
    let f = filter.trim();
    if !f.is_empty() {
        *sql += &format!(" where {f}");
    }
}

fn collect_results(messages: Vec<SimpleQueryMessage>, cap: usize) -> Vec<StatementResult> {
    let mut out = Vec::new();
    let mut current: Option<ResultSet> = None;
    for msg in messages {
        match msg {
            SimpleQueryMessage::RowDescription(cols) => {
                current = Some(ResultSet {
                    columns: cols.iter().map(|c| c.name().to_string()).collect(),
                    ..Default::default()
                });
            }
            SimpleQueryMessage::Row(row) => {
                let rs = current.get_or_insert_with(|| ResultSet {
                    columns: row.columns().iter().map(|c| c.name().to_string()).collect(),
                    ..Default::default()
                });
                if rs.rows.len() < cap {
                    rs.rows.push((0..row.len()).map(|i| row.get(i).map(str::to_owned)).collect());
                } else {
                    rs.truncated = true;
                }
            }
            SimpleQueryMessage::CommandComplete(n) => out.push(StatementResult {
                result: current.take(),
                affected: n,
            }),
            _ => {}
        }
    }
    out
}

/// Human-readable error with detail, hint and position when available.
pub fn format_error(e: &tokio_postgres::Error, sql: Option<&str>) -> String {
    match sql {
        Some(sql) => format_error_shifted(e, sql, 0),
        None => format_error_inner(e, None),
    }
}

/// `shift` chars were prepended to `sql` before sending; error positions
/// are mapped back onto the original text.
fn format_error_shifted(e: &tokio_postgres::Error, sql: &str, shift: usize) -> String {
    format_error_inner(e, Some((sql, shift)))
}

fn format_error_inner(e: &tokio_postgres::Error, sql: Option<(&str, usize)>) -> String {
    let Some(db) = e.as_db_error() else {
        let mut s = e.to_string();
        if let Some(src) = std::error::Error::source(e) {
            s = format!("{s}: {src}");
        }
        return s;
    };
    let mut s = if db.code().code() == "25006" {
        "This connection is read-only (25006)".to_string()
    } else {
        format!("[{}] {}", db.code().code(), db.message())
    };
    if let Some(d) = db.detail() {
        s += &format!("\nDetail: {d}");
    }
    if let Some(h) = db.hint() {
        s += &format!("\nHint: {h}");
    }
    if let (Some(ErrorPosition::Original(pos)), Some((sql, shift))) = (db.position(), sql) {
        let (line, col) = crate::sql::line_col(sql, (*pos as usize).saturating_sub(shift).max(1));
        s += &format!("\nPosition: line {line}, column {col}");
    }
    s
}

/// Like [`format_error`], plus where COPY failed ("COPY t, line 5, column x"):
/// its line numbers count data rows, so line N is row N of the file.
fn import_error(e: &tokio_postgres::Error) -> String {
    let mut s = format_error(e, None);
    if let Some(w) = e.as_db_error().and_then(|d| d.where_()) {
        s += &format!("\nWhere: {w} (line numbers count data rows, without the header)");
    }
    s
}
