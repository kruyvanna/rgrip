//! SSH local port forwarding: listen on 127.0.0.1:<random> and forward every
//! accepted connection through the SSH server to the database host.

use crate::model::{SshAuth, SshConfig};
use russh::client::{self, Handle};
use russh::keys::PrivateKeyWithHashAlg;
use russh::keys::PublicKeyOrCertificate;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub struct Tunnel {
    pub local_port: u16,
    accept_loop: JoinHandle<()>,
    _session: Arc<Handle<HostKeyCheck>>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.accept_loop.abort();
    }
}

/// Host keys are checked against ~/.ssh/known_hosts: a known host with a
/// different key is rejected; unknown hosts are accepted (like OpenSSH's
/// `StrictHostKeyChecking=accept-new`, without writing the file).
pub struct HostKeyCheck {
    host: String,
    port: u16,
}

impl client::Handler for HostKeyCheck {
    type Error = russh::Error;

    async fn check_server_key(&mut self, server: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        // Host certificates are signed by a CA we can't check here; accept them.
        let PublicKeyOrCertificate::PublicKey { key, .. } = server else { return Ok(true) };
        match russh::keys::check_known_hosts(&self.host, self.port, key) {
            Ok(_) => Ok(true),
            Err(russh::keys::Error::KeyChanged { line }) => {
                Err(russh::Error::WrongServerSig).inspect_err(|_| {
                    eprintln!("ssh: host key for {} changed (known_hosts line {line})", self.host)
                })
            }
            // No known_hosts file or unreadable entry: treat as unknown.
            Err(_) => Ok(true),
        }
    }
}

pub async fn open(cfg: &SshConfig, target_host: &str, target_port: u16) -> Result<Tunnel, String> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    let handler = HostKeyCheck { host: cfg.host.clone(), port: cfg.port };
    let connect = client::connect(config, (cfg.host.as_str(), cfg.port), handler);
    let mut session = tokio::time::timeout(Duration::from_secs(10), connect)
        .await
        .map_err(|_| format!("SSH: timed out connecting to {}:{}", cfg.host, cfg.port))?
        .map_err(|e| match e {
            russh::Error::WrongServerSig => format!(
                "SSH: host key for {} does not match ~/.ssh/known_hosts — possible man-in-the-middle",
                cfg.host
            ),
            e => format!("SSH: {e}"),
        })?;

    let auth = match cfg.auth {
        SshAuth::Password => session.authenticate_password(&cfg.user, &cfg.password).await,
        SshAuth::Key => {
            let path = expand_home(&cfg.key_path);
            let passphrase = (!cfg.passphrase.is_empty()).then_some(cfg.passphrase.as_str());
            let key = russh::keys::load_secret_key(&path, passphrase)
                .map_err(|e| format!("SSH: cannot load key {path}: {e}"))?;
            let hash = session.best_supported_rsa_hash().await.map_err(|e| format!("SSH: {e}"))?.flatten();
            session
                .authenticate_publickey(&cfg.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
        }
    }
    .map_err(|e| format!("SSH: {e}"))?;
    if !auth.success() {
        return Err(format!("SSH: authentication failed for {}@{}", cfg.user, cfg.host));
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| format!("SSH: {e}"))?;
    let local_port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let session = Arc::new(session);
    let target_host = target_host.to_string();
    let accept_session = session.clone();
    let accept_loop = tokio::spawn(async move {
        while let Ok((mut socket, peer)) = listener.accept().await {
            let session = accept_session.clone();
            let host = target_host.clone();
            tokio::spawn(async move {
                let channel = session
                    .channel_open_direct_tcpip(host, target_port as u32, peer.ip().to_string(), peer.port() as u32)
                    .await;
                if let Ok(channel) = channel {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                }
            });
        }
    });

    Ok(Tunnel { local_port, accept_loop, _session: session })
}

fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => path.to_string(),
    }
}
