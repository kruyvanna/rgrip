//! Edit table against a live server (same environment variables as
//! import.rs). Skipped when no server is reachable.

use rgrip_core::alter::{self, EditColumn, TableEdit};
use rgrip_core::*;

fn cfg() -> ConnectionConfig {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    ConnectionConfig {
        host: env("RGRIP_TEST_HOST", "localhost"),
        port: env("RGRIP_TEST_PORT", "5432").parse().unwrap(),
        database: env("RGRIP_TEST_DB", "postgres"),
        user: env("RGRIP_TEST_USER", &std::env::var("USER").unwrap_or_default()),
        password: env("RGRIP_TEST_PASSWORD", ""),
        ssl: if env("RGRIP_TEST_SSL", "") == "require" { SslMode::Require } else { SslMode::Disable },
        ..Default::default()
    }
}

#[tokio::test]
async fn edits_a_table() {
    let Ok(conn) = PgConn::connect(&cfg()).await else {
        eprintln!("no server: skipped");
        return;
    };
    let (s, t) = ("public", format!("rgrip_alter_{}", std::process::id()));
    let renamed = format!("{t}_2");
    conn.alter(&format!("create table {s}.{t} (shapeiso text, shapename text, shapeid text, n text); insert into {s}.{t} values ('KH-1', 'A', 'x', '1'), ('KH-2', 'B', 'y', '2')")).await.unwrap();
    let infos = conn.columns(s, &t).await.unwrap();
    let pk = conn.primary_key_name(s, &t).await.unwrap();
    let mut cols: Vec<EditColumn> = infos.iter().map(EditColumn::from_info).collect();
    cols[0].name = "code".into();
    cols[0].primary_key = true;
    cols[1].name = "name".into();
    cols[1].nullable = false;
    cols[2].drop = true;
    cols[3].data_type = "integer".into();
    let mut added = EditColumn::new_column();
    added.name = "verified".into();
    added.data_type = "boolean".into();
    added.default = "false".into();
    added.nullable = false;
    cols.push(added);
    let edit = TableEdit { schema: s, table: &t, new_name: &renamed, columns: &cols, pk_constraint: pk.as_deref() };
    assert_eq!(alter::problem(&edit), None);
    let result = conn.alter(&alter::statements(&edit).join(";\n")).await;
    let after = conn.columns(s, &renamed).await;
    let _ = conn.alter(&format!("drop table if exists {s}.{t}; drop table if exists {s}.{renamed}")).await;
    result.unwrap();
    let after = after.unwrap();
    let names: Vec<(&str, &str, bool, bool)> = after.iter().map(|c| (c.name.as_str(), c.data_type.as_str(), c.nullable, c.primary_key)).collect();
    assert_eq!(names, [("code", "text", false, true), ("name", "text", false, false), ("n", "integer", true, false), ("verified", "boolean", false, false)]);
}

/// A failing statement (not null over NULLs) leaves the earlier ones undone.
#[tokio::test]
async fn all_or_nothing() {
    let Ok(conn) = PgConn::connect(&cfg()).await else {
        eprintln!("no server: skipped");
        return;
    };
    let t = format!("rgrip_alter_fail_{}", std::process::id());
    conn.alter(&format!("create table public.{t} (a text, b text); insert into public.{t} values ('x', null)")).await.unwrap();
    let mut cols: Vec<EditColumn> = conn.columns("public", &t).await.unwrap().iter().map(EditColumn::from_info).collect();
    cols[0].name = "renamed".into();
    cols[1].nullable = false;
    let edit = TableEdit { schema: "public", table: &t, new_name: &t, columns: &cols, pk_constraint: None };
    let result = conn.alter(&alter::statements(&edit).join(";\n")).await;
    let after = conn.columns("public", &t).await.unwrap();
    conn.alter(&format!("drop table public.{t}")).await.unwrap();
    assert!(result.is_err());
    assert_eq!(after[0].name, "a");
}
