use crate::model::*;
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
        let cancel = client.cancel_token();
        Ok(Self { client, cancel, tls, tunnel })
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
                                 filter (where a.attnum is not null), '{}')
                 from pg_class c
                 join pg_namespace n on n.oid = c.relnamespace
                 left join pg_attribute a
                   on a.attrelid = c.oid and a.attnum > 0 and not a.attisdropped
                 where c.relkind in ('r','p','v','m','f') and not c.relispartition
                   and n.nspname not like 'pg\\_toast%' and n.nspname not like 'pg\\_temp%'
                 group by 1, 2, 3
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
                    }
                })
                .collect(),
        })
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
        sql: &str,
        format: crate::export::Format,
        table: &str,
        path: &std::path::Path,
    ) -> Result<usize, String> {
        let out = self.execute_capped(sql, usize::MAX).await?;
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
    let mut s = format!("[{}] {}", db.code().code(), db.message());
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
