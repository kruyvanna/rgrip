//! Runs against a live server. Configure with RGRIP_TEST_HOST / _PORT / _DB / _USER / _PASSWORD;
//! skipped when no server is reachable.

use rgrip_core::*;

fn cfg() -> ConnectionConfig {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    ConnectionConfig {
        host: env("RGRIP_TEST_HOST", "localhost"),
        port: env("RGRIP_TEST_PORT", "5432").parse().unwrap(),
        database: env("RGRIP_TEST_DB", "postgres"),
        user: env("RGRIP_TEST_USER", &std::env::var("USER").unwrap_or_default()),
        password: env("RGRIP_TEST_PASSWORD", ""),
        ssl: SslMode::Disable,
        ..Default::default()
    }
}

#[tokio::test]
async fn end_to_end() {
    let Ok(conn) = PgConn::connect(&cfg()).await else {
        eprintln!("skipping: no postgres reachable");
        return;
    };
    assert!(conn.server_info().await.unwrap().version.starts_with("PostgreSQL"));

    let schema = format!("rgrip_test_{}", std::process::id());
    let setup = format!(
        "create schema {schema};
         create table {schema}.items (id serial primary key, name text not null, tags text[], meta jsonb);
         insert into {schema}.items (name, tags, meta) values
           ('a', '{{x,y}}', '{{\"k\": 1}}'), ('b', null, null), ('c', '{{}}', 'null');
         create view {schema}.v as select id from {schema}.items;"
    );
    conn.execute(&setup).await.unwrap();

    let result = async {
        assert!(conn.schemas().await.unwrap().contains(&schema));

        let tables = conn.tables(&schema).await.unwrap();
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].name, "items");
        assert_eq!(tables[1].kind, TableKind::View);

        let cols = conn.columns(&schema, "items").await.unwrap();
        assert_eq!(cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "name", "tags", "meta"]);
        assert!(cols[0].primary_key && cols[0].default.is_some());
        assert!(!cols[1].nullable);
        assert_eq!(cols[2].data_type, "text[]");

        let out = conn
            .execute(&format!("select * from {schema}.items order by id; update {schema}.items set name = name; select 1 where false"))
            .await
            .unwrap();
        assert_eq!(out.statements.len(), 3);
        let rs = out.statements[0].result.as_ref().unwrap();
        assert_eq!(rs.rows.len(), 3);
        assert_eq!(rs.rows[0][2].as_deref(), Some("{x,y}"));
        assert_eq!(rs.rows[1][3], None);
        assert!(out.statements[1].result.is_none());
        assert_eq!(out.statements[1].affected, 3);
        // Empty result still reports its columns.
        assert_eq!(out.statements[2].result.as_ref().unwrap().columns, ["?column?"]);

        let q = TableQuery {
            schema: schema.clone(),
            table: "items".into(),
            filter: "name <> 'b'".into(),
            order: Some(("id".into(), false)),
            limit: 1,
            offset: 0,
        };
        let page = conn.table_page(&q).await.unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0][1].as_deref(), Some("c"));
        assert_eq!(conn.table_count(&q).await.unwrap(), 2);

        let catalog = conn.catalog().await.unwrap();
        let items = catalog.tables.iter().find(|t| t.schema == schema && t.name == "items").unwrap();
        assert_eq!(items.columns[2], ("tags".to_string(), "text[]".to_string()));
        assert!(catalog.tables.iter().any(|t| t.schema == "pg_catalog" && t.name == "pg_stat_activity"));
        let sql = format!("select i.ta from {schema}.items i");
        let c = rgrip_core::complete::complete(&sql, 11, &catalog, false).unwrap();
        assert_eq!(c.items[0].label, "tags");

        // Server-side paging: 1000 rows per page, `truncated` = more pages.
        let q = "select g from generate_series(1, 2500) g order by g";
        let page = |offset| conn.execute_page(q, offset);
        let first = page(0).await.unwrap();
        let rs = first.statements[0].result.as_ref().unwrap();
        assert_eq!((rs.rows.len(), rs.truncated), (1000, true));
        let last = page(2000).await.unwrap();
        let rs = last.statements[0].result.as_ref().unwrap();
        assert_eq!((rs.rows.len(), rs.truncated, rs.rows[0][0].as_deref()), (500, false, Some("2001")));
        // Errors point at the user's SQL, not the paging wrapper.
        let err = conn.execute_page("select\n  nope from generate_series(1, 3) g", 0).await.unwrap_err();
        assert!(err.contains("line 2, column 3"), "{err}");
        // Export-all writes every row, not just one page.
        let path = std::env::temp_dir().join(format!("rgrip-export-{}.csv", std::process::id()));
        let n = conn.export_to_file(q, rgrip_core::export::Format::Csv, "t", &path).await.unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(n, 2500);
        assert_eq!(text.lines().count(), 2501);
        assert!(text.ends_with("2500\n"));

        // Unpageable statements still run (capped).
        let out = conn.execute_page("select 1; select 2", 0).await.unwrap();
        assert_eq!(out.statements.len(), 2);

        let err = conn.execute("select 1;\nselect nope").await.unwrap_err();
        assert!(err.contains("42703") && err.contains("line 2"), "{err}");
    }
    .await;

    conn.execute(&format!("drop schema {schema} cascade")).await.unwrap();
    result
}
