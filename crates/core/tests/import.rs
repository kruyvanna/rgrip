//! Imports files into a live server (same environment variables as pg.rs, plus
//! RGRIP_TEST_SSL=require for the dev PostGIS). Skipped when no server is
//! reachable; the geometry parts are skipped when PostGIS can't be enabled.

use rgrip_core::import::{self, Format, ImportRequest, PlanColumn, Src};
use rgrip_core::*;
use std::path::PathBuf;

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

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// A request that takes every column of the file as inferred, plus extras.
fn request(path: PathBuf, schema: &str, table: &str, srid: i32, extra: Vec<PlanColumn>) -> ImportRequest {
    let o = import::open(&path).unwrap();
    let names = import::unique_names(&o.columns);
    let mut columns: Vec<PlanColumn> = (0..o.columns.len())
        .map(|i| {
            let ty = if o.geometry == Some(i) { "geometry" } else { import::infer_type(o.sample.iter().filter_map(|r| r[i].as_deref())) };
            PlanColumn { name: names[i].clone(), ty: ty.into(), src: Src::Field(i) }
        })
        .collect();
    columns.extend(extra);
    ImportRequest { path, format: o.format, delimiter: o.delimiter, schema: schema.into(), table: table.into(), create: true, columns, srid, spatial_index: true, key: Some("row_key".into()) }
}

async fn rows(conn: &PgConn, sql: &str) -> Vec<Vec<Option<String>>> {
    conn.execute(sql).await.unwrap().statements[0].result.as_ref().unwrap().rows.clone()
}

#[tokio::test]
async fn imports_files() {
    let Ok(conn) = PgConn::connect(&cfg()).await else {
        eprintln!("skipping: no postgres reachable");
        return;
    };
    let schema = format!("rgrip_import_{}", std::process::id());
    conn.execute(&format!("create schema {schema}")).await.unwrap();
    // PostGIS lives in the schema, so dropping it cleans up the extension too.
    let postgis = conn.execute(&format!("create extension if not exists postgis schema {schema}")).await.is_ok();
    if postgis {
        conn.execute(&format!("set search_path to {schema}, public")).await.unwrap();
    }
    let result = async {
        // CSV: BOM, semicolons, quotes with tabs/commas, a literal \n, an empty row of NULLs.
        let o = import::open(&fixture("places.csv")).unwrap();
        assert_eq!(o.delimiter, b';');
        assert_eq!(o.columns[0], "id");
        let mut req = request(fixture("places.csv"), &schema, "places", 0, vec![]);
        let types: Vec<&str> = req.columns.iter().map(|c| c.ty.as_str()).collect();
        assert_eq!(types, ["integer", "text", "text", "numeric", "numeric", "date"]);
        let mut progress = Vec::new();
        assert_eq!(conn.import(&req, |n| progress.push(n)).await.unwrap(), 3);
        assert_eq!(progress.last(), Some(&3));
        let got = rows(&conn, &format!("select id, name, note, lon, seen from {schema}.places order by id")).await;
        assert_eq!(got[0][2].as_deref(), Some("Reclining\tBuddha, \"big\""));
        assert_eq!(got[1][2].as_deref(), Some("line one\\nstill"));
        assert_eq!(got[2][1], None);
        assert_eq!(got[0][4].as_deref(), Some("2024-01-05"));

        // Appending adds to the same table; a duplicate create fails and changes nothing.
        req.create = false;
        assert_eq!(conn.import(&req, |_| {}).await.unwrap(), 3);
        assert_eq!(rows(&conn, &format!("select count(*) from {schema}.places")).await[0][0].as_deref(), Some("6"));
        req.create = true;
        assert!(conn.import(&req, |_| {}).await.unwrap_err().contains("already exists"));
        assert_eq!(rows(&conn, &format!("select count(*) from {schema}.places")).await[0][0].as_deref(), Some("6"));

        // A bad value rolls everything back, and the error names the row.
        let bad = std::env::temp_dir().join(format!("rgrip-bad-{}.csv", std::process::id()));
        std::fs::write(&bad, "n\n1\n2\nthree\n").unwrap();
        let mut req = request(bad.clone(), &schema, "bad", 0, vec![]);
        req.columns[0].ty = "integer".into();
        let err = conn.import(&req, |_| {}).await.unwrap_err();
        assert!(err.contains("line 3"), "{err}");
        assert!(rows(&conn, &format!("select to_regclass('{schema}.bad')::text")).await[0][0].is_none());
        std::fs::remove_file(bad).unwrap();

        if !postgis {
            eprintln!("skipping geometry imports: PostGIS is not available");
            return;
        }

        // CSV with a point built from lon/lat.
        let point = PlanColumn { name: "geom".into(), ty: "geometry".into(), src: Src::Point { lon: 3, lat: 4 } };
        let req = request(fixture("places.csv"), &schema, "places_geo", 4326, vec![point]);
        assert_eq!(conn.import(&req, |_| {}).await.unwrap(), 3);
        let got = rows(&conn, &format!("select st_srid(geom), st_x(geom), st_y(geom) from {schema}.places_geo order by id")).await;
        assert_eq!(got[0], [Some("4326".into()), Some("100.493".into()), Some("13.7465".into())]);
        assert_eq!(got[2][0], None);
        // The spatial index came with it.
        let idx = format!("select count(*) from pg_indexes where schemaname = '{schema}' and tablename = 'places_geo' and indexdef like '%gist%'");
        assert_eq!(rows(&conn, &idx).await[0][0].as_deref(), Some("1"));

        // GeoJSON: properties become columns; geometries are Point, Polygon and NULL.
        let req = request(fixture("places.geojson"), &schema, "shapes", 4326, vec![]);
        assert_eq!(req.format, Format::GeoJson);
        assert_eq!(conn.import(&req, |_| {}).await.unwrap(), 3);
        let got = rows(&conn, &format!("select name, height, open, st_astext(geom), st_srid(geom) from {schema}.shapes order by name")).await;
        assert_eq!(got[0][0].as_deref(), Some("Nowhere"));
        assert_eq!(got[0][3], None);
        assert_eq!(got[1][0].as_deref(), Some("Pin"));
        assert_eq!((got[1][1].as_deref(), got[1][2].as_deref(), got[1][3].as_deref()), (Some("12"), Some("t"), Some("POINT(100.5 13.75)")));
        assert_eq!(got[2][0].as_deref(), Some("Plot\tA"));
        assert_eq!(got[2][4].as_deref(), Some("4326"));

        // A shapefile written on the fly, with a .prj for the SRID.
        use shapefile::{Point, Polyline, Writer, dbase};
        let dir = std::env::temp_dir().join(format!("rgrip-live-shp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shp = dir.join("roads.shp");
        let table = dbase::TableWriterBuilder::new().add_character_field("name".try_into().unwrap(), 20);
        let mut w = Writer::from_path(&shp, table).unwrap();
        let mut rec = dbase::Record::default();
        rec.insert("name".to_string(), dbase::FieldValue::Character(Some("Rama IV".into())));
        w.write_shape_and_record(&Polyline::new(vec![Point::new(100.0, 13.0), Point::new(100.1, 13.1)]), &rec).unwrap();
        drop(w);
        std::fs::write(dir.join("roads.prj"), r#"PROJCS["WGS_1984_UTM_Zone_47N",GEOGCS["GCS_WGS_1984"]]"#).unwrap();
        let o = import::open(&shp).unwrap();
        let req = request(shp, &schema, "roads", o.srid.unwrap(), vec![]);
        assert_eq!(conn.import(&req, |_| {}).await.unwrap(), 1);
        let got = rows(&conn, &format!("select name, st_srid(geom), st_geometrytype(geom) from {schema}.roads")).await;
        assert_eq!(got[0], [Some("Rama IV".into()), Some("32647".into()), Some("ST_MultiLineString".into())]);
        std::fs::remove_dir_all(dir).unwrap();
    }
    .await;

    let _ = conn.execute(&format!("drop schema {schema} cascade")).await;
    result
}

/// Add primary key on a table without one: existing rows get numbered.
#[tokio::test]
async fn adds_a_primary_key() {
    let Ok(conn) = PgConn::connect(&cfg()).await else {
        eprintln!("no server: skipped");
        return;
    };
    conn.alter("drop table if exists public.rgrip_test_nokey; create table public.rgrip_test_nokey as select v from (values ('a'), ('b'), ('c')) t(v)").await.unwrap();
    conn.alter("alter table \"public\".\"rgrip_test_nokey\" add column \"id\" bigint generated always as identity primary key").await.unwrap();
    let got = rows(&conn, "select id::text, v from public.rgrip_test_nokey order by id").await;
    conn.alter("drop table public.rgrip_test_nokey").await.unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[0][0].as_deref(), Some("1"));
}
