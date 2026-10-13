//! SSL and SSH tests against the dev environment (dev/docker-compose.yml).
//! Skipped when it isn't running.

use rgrip_core::*;

const TLS: &str = "TLSv1.3";

fn dev(path: &str) -> String {
    format!("{}/../../dev/{path}", env!("CARGO_MANIFEST_DIR"))
}

fn base(ssl: SslMode) -> ConnectionConfig {
    ConnectionConfig {
        host: "localhost".into(),
        port: 55432,
        database: "rgrip".into(),
        user: "rgrip".into(),
        password: "rgrip".into(),
        ssl,
        ..Default::default()
    }
}

fn via_ssh(mut cfg: ConnectionConfig, auth: SshAuth) -> ConnectionConfig {
    // From the bastion, Postgres is `pg:5432`.
    cfg.host = "pg".into();
    cfg.port = 5432;
    cfg.ssh = SshConfig {
        enabled: true,
        // 127.0.0.1 rather than localhost: the developer's known_hosts may
        // hold a key for some other [localhost]:2222 server.
        host: "127.0.0.1".into(),
        port: 2222,
        user: "rgrip".into(),
        auth,
        password: "rgrip".into(),
        key_path: dev("ssh/id_ed25519"),
        passphrase: String::new(),
    };
    cfg
}

async fn info(cfg: &ConnectionConfig) -> Result<ServerInfo, String> {
    PgConn::connect(cfg).await?.server_info().await
}

async fn available() -> bool {
    let up = tokio::net::TcpStream::connect("127.0.0.1:55432").await.is_ok();
    if !up {
        eprintln!("skipping: dev environment not running");
    }
    up
}

#[tokio::test]
async fn ssl_modes() {
    if !available().await {
        return;
    }
    assert_eq!(info(&base(SslMode::Disable)).await.unwrap().tls, None);
    assert_eq!(info(&base(SslMode::Prefer)).await.unwrap().tls.as_deref(), Some(TLS));
    assert_eq!(info(&base(SslMode::Require)).await.unwrap().tls.as_deref(), Some(TLS));

    // Self-signed CA is not trusted unless configured.
    let err = info(&base(SslMode::VerifyFull)).await.unwrap_err();
    assert!(err.to_lowercase().contains("certificate") || err.contains("tls") || err.contains("TLS"), "{err}");

    let mut cfg = base(SslMode::VerifyFull);
    cfg.ssl_root_cert = dev("certs/ca.crt");
    let i = info(&cfg).await.unwrap();
    assert_eq!(i.tls.as_deref(), Some(TLS));
    assert_eq!(i.summary(), format!("{} · {TLS}", i.version));

    cfg.ssl = SslMode::VerifyCa;
    assert!(info(&cfg).await.is_ok());

    // verify-ca ignores the host name; verify-full doesn't.
    let mut by_ip = cfg.clone();
    // The dev certificate covers localhost, pg and 127.0.0.1, but not ::1.
    by_ip.host = "::1".into();
    if tokio::net::TcpStream::connect("[::1]:55432").await.is_ok() {
        assert!(info(&by_ip).await.is_ok());
        by_ip.ssl = SslMode::VerifyFull;
        let err = info(&by_ip).await.unwrap_err();
        assert!(err.contains("NotValidForName") || err.contains("not valid for"), "{err}");
    }

    cfg.ssl_root_cert = dev("certs/missing.crt");
    assert!(info(&cfg).await.unwrap_err().contains("Cannot read CA certificate"));
}

#[tokio::test]
async fn client_certificate_is_presented() {
    if !available().await {
        return;
    }
    let mut cfg = base(SslMode::VerifyFull);
    cfg.ssl_root_cert = dev("certs/ca.crt");
    cfg.ssl_client_cert = dev("certs/client.crt");
    cfg.ssl_client_key = dev("certs/client.key");
    let conn = PgConn::connect(&cfg).await.unwrap();
    let out = conn
        .execute("select client_dn from pg_stat_ssl where pid = pg_backend_pid()")
        .await
        .unwrap();
    let dn = out.statements[0].result.as_ref().unwrap().rows[0][0].clone();
    assert_eq!(dn.as_deref(), Some("/CN=rgrip"));
}

#[tokio::test]
async fn ssh_tunnel() {
    if !available().await {
        return;
    }
    for auth in [SshAuth::Password, SshAuth::Key] {
        let i = info(&via_ssh(base(SslMode::Disable), auth)).await.unwrap();
        assert!(i.via_ssh && i.tls.is_none(), "{auth:?}");
    }

    // TLS inside the tunnel, verified against the remote host name `pg`.
    let mut cfg = via_ssh(base(SslMode::VerifyFull), SshAuth::Key);
    cfg.ssl_root_cert = dev("certs/ca.crt");
    let i = info(&cfg).await.unwrap();
    assert_eq!(i.tls.as_deref(), Some(TLS));
    assert!(i.summary().ends_with(&format!("{TLS} · SSH")));

    // The tunnel carries real queries, several at once.
    let conn = PgConn::connect(&cfg).await.unwrap();
    let (a, b) = tokio::join!(conn.execute("select count(*) from places"), conn.catalog());
    assert_eq!(a.unwrap().statements[0].result.as_ref().unwrap().rows[0][0].as_deref(), Some("5"));
    assert!(b.unwrap().tables.iter().any(|t| t.name == "places"));

    let mut bad = via_ssh(base(SslMode::Disable), SshAuth::Password);
    bad.ssh.password = "wrong".into();
    assert!(info(&bad).await.unwrap_err().contains("authentication failed"));

    let mut bad = via_ssh(base(SslMode::Disable), SshAuth::Key);
    bad.ssh.key_path = dev("ssh/nope");
    assert!(info(&bad).await.unwrap_err().contains("cannot load key"));
}
