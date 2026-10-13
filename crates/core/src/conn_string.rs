//! Connection strings: `postgres://` URLs, libpq `key=value` strings and
//! DataGrip/JDBC `jdbc:postgresql://` URLs.

use crate::model::{ConnectionConfig, SslMode};

/// Fill `cfg` from a connection string. Fields the string doesn't mention
/// are left as they are.
pub fn apply(cfg: &mut ConnectionConfig, s: &str) -> Result<(), String> {
    let s = s.trim();
    let s = s.strip_prefix("jdbc:").unwrap_or(s);
    if let Some(rest) = s.strip_prefix("postgresql://").or_else(|| s.strip_prefix("postgres://")) {
        apply_url(cfg, rest)
    } else if s.contains('=') {
        apply_pairs(cfg, s)
    } else {
        Err("Not a connection string (expected postgres://… or host=… dbname=…)".into())
    }
}

fn apply_url(cfg: &mut ConnectionConfig, rest: &str) -> Result<(), String> {
    let (main, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = main.split_once('/').unwrap_or((main, ""));
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    if let Some(u) = userinfo {
        let (user, password) = u.split_once(':').unwrap_or((u, ""));
        if !user.is_empty() {
            cfg.user = decode(user)?;
        }
        if !password.is_empty() {
            cfg.password = decode(password)?;
        }
    }
    // Multiple hosts (failover) aren't supported: use the first.
    let hostport = hostport.split(',').next().unwrap_or_default();
    let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
        let (h, after) = v6.split_once(']').ok_or("Unclosed [ in IPv6 host")?;
        (h.to_string(), after.strip_prefix(':'))
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (hostport.to_string(), None),
        }
    };
    if !host.is_empty() {
        cfg.host = decode(&host)?;
    }
    if let Some(p) = port.filter(|p| !p.is_empty()) {
        cfg.port = p.parse().map_err(|_| format!("Invalid port: {p}"))?;
    }
    if !path.is_empty() {
        cfg.database = decode(path)?;
    }
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        set_param(cfg, &decode(k)?, &decode(v)?)?;
    }
    Ok(())
}

/// libpq `key=value` pairs; values may be single-quoted with `\` escapes.
fn apply_pairs(cfg: &mut ConnectionConfig, s: &str) -> Result<(), String> {
    let mut chars = s.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        if chars.peek().is_none() {
            return Ok(());
        }
        let key: String = std::iter::from_fn(|| chars.next_if(|c| *c != '=' && !c.is_whitespace())).collect();
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        if chars.next() != Some('=') {
            return Err(format!("Expected = after {key}"));
        }
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let mut value = String::new();
        if chars.next_if_eq(&'\'').is_some() {
            loop {
                match chars.next() {
                    Some('\\') => value.extend(chars.next()),
                    Some('\'') => break,
                    Some(c) => value.push(c),
                    None => return Err(format!("Unterminated quote in {key}")),
                }
            }
        } else {
            value.extend(std::iter::from_fn(|| chars.next_if(|c| !c.is_whitespace())));
        }
        set_param(cfg, &key, &value)?;
    }
}

fn set_param(cfg: &mut ConnectionConfig, key: &str, value: &str) -> Result<(), String> {
    match key {
        "host" | "hostaddr" => cfg.host = value.split(',').next().unwrap_or_default().to_string(),
        "port" => cfg.port = value.split(',').next().unwrap_or_default().parse().map_err(|_| format!("Invalid port: {value}"))?,
        "dbname" | "database" => cfg.database = value.to_string(),
        "user" => cfg.user = value.to_string(),
        "password" => cfg.password = value.to_string(),
        "sslmode" | "ssl" => {
            cfg.ssl = match value {
                "disable" | "false" => SslMode::Disable,
                "allow" | "prefer" => SslMode::Prefer,
                "require" | "true" => SslMode::Require,
                "verify-ca" => SslMode::VerifyCa,
                "verify-full" => SslMode::VerifyFull,
                other => return Err(format!("Unknown sslmode: {other}")),
            }
        }
        "sslrootcert" => cfg.ssl_root_cert = value.to_string(),
        "sslcert" => cfg.ssl_client_cert = value.to_string(),
        "sslkey" => cfg.ssl_client_key = value.to_string(),
        // Harmless libpq/JDBC options we don't use.
        _ => {}
    }
    Ok(())
}

fn decode(s: &str) -> Result<String, String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3).ok_or("Bad % escape")?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| format!("Bad % escape: %{hex}"))?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "Connection string is not valid UTF-8".into())
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `postgres://user[:password]@host:port/db?sslmode=…`
pub fn to_url(cfg: &ConnectionConfig, with_password: bool) -> String {
    let mut auth = encode(&cfg.user);
    if with_password && !cfg.password.is_empty() {
        auth += &format!(":{}", encode(&cfg.password));
    }
    let host = if cfg.host.contains(':') { format!("[{}]", cfg.host) } else { cfg.host.clone() };
    let mut url = format!("postgres://{auth}@{host}:{}/{}", cfg.port, encode(&cfg.database));
    if cfg.ssl != SslMode::Prefer {
        url += &format!("?sslmode={}", cfg.ssl.label());
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> ConnectionConfig {
        let mut cfg = ConnectionConfig::default();
        apply(&mut cfg, s).unwrap();
        cfg
    }

    #[test]
    fn urls() {
        let c = parse("postgres://lucy:p%40ss%3Aw0rd@db.example.com:6543/gis%20data?sslmode=verify-full&sslrootcert=/tmp/ca.pem&application_name=x");
        assert_eq!((c.user.as_str(), c.password.as_str()), ("lucy", "p@ss:w0rd"));
        assert_eq!((c.host.as_str(), c.port, c.database.as_str()), ("db.example.com", 6543, "gis data"));
        assert_eq!((c.ssl, c.ssl_root_cert.as_str()), (SslMode::VerifyFull, "/tmp/ca.pem"));

        let c = parse("postgresql://[::1]:5433/app");
        assert_eq!((c.host.as_str(), c.port, c.database.as_str()), ("::1", 5433, "app"));

        // Only what's given changes.
        let c = parse("postgres://onlyhost");
        assert_eq!((c.host.as_str(), c.port), ("onlyhost", 5432));
    }

    #[test]
    fn jdbc_and_pairs() {
        let c = parse("jdbc:postgresql://10.0.0.5:5432/urban?user=gis&password=secret&ssl=true");
        assert_eq!((c.host.as_str(), c.user.as_str(), c.password.as_str(), c.ssl), ("10.0.0.5", "gis", "secret", SslMode::Require));

        let c = parse("host=localhost port=5544 dbname=mydb user=u password='it\\'s a secret' sslmode=disable");
        assert_eq!((c.host.as_str(), c.port, c.database.as_str()), ("localhost", 5544, "mydb"));
        assert_eq!((c.password.as_str(), c.ssl), ("it's a secret", SslMode::Disable));
    }

    #[test]
    fn errors() {
        let mut cfg = ConnectionConfig::default();
        assert!(apply(&mut cfg, "mysql://x").is_err());
        assert!(apply(&mut cfg, "postgres://h:notaport/db").unwrap_err().contains("Invalid port"));
        assert!(apply(&mut cfg, "host=x password='open").unwrap_err().contains("Unterminated"));
        assert!(apply(&mut cfg, "postgres://h/db?sslmode=maybe").unwrap_err().contains("sslmode"));
    }

    #[test]
    fn round_trip() {
        let c = parse("postgres://u%20x:p%2Fw@h:5433/d?sslmode=require");
        assert_eq!(to_url(&c, true), "postgres://u%20x:p%2Fw@h:5433/d?sslmode=require");
        assert_eq!(to_url(&c, false), "postgres://u%20x@h:5433/d?sslmode=require");
        assert_eq!(parse(&to_url(&c, true)).password, "p/w");
    }
}
