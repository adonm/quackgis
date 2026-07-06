// SPDX-License-Identifier: Apache-2.0
//! End-to-end smoke: spin up the actual pgwire server on an ephemeral port and
//! drive it with `tokio-postgres`. Verifies the full wire stack (not just the
//! in-process SedonaDB call) without needing psql on the host.

mod common;

use std::sync::Arc;

use datafusion::arrow::array::{BinaryArray, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion_ducklake::{DuckLakeTableWriter, MetadataWriter, SqliteMetadataWriter};
use object_store::local::LocalFileSystem;
use quackgis_server::context::StoragePaths;
use tokio_postgres::NoTls;

use common::ServerHandle;

async fn connect() -> (
    ServerHandle,
    tokio_postgres::Client,
    tokio::task::JoinHandle<()>,
) {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect to quackgis-server");
    let conn_task = tokio::spawn(async move {
        let _ = connection.await;
    });
    (server, client, conn_task)
}

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut out = Vec::with_capacity(21);
    out.push(1);
    out.extend_from_slice(&1_u32.to_le_bytes());
    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&y.to_le_bytes());
    out
}

async fn write_keyless_geo(paths: &StoragePaths, table: &str) {
    let writer = Arc::new(
        SqliteMetadataWriter::new_with_init(&paths.catalog_conn)
            .await
            .expect("writer"),
    );
    writer
        .set_data_path(&paths.data_path)
        .expect("set data path");
    let snapshot = writer.create_snapshot().expect("snapshot");
    writer
        .get_or_create_schema("main", None, snapshot)
        .expect("main schema");

    let wkbs = [point_wkb(0.0, 0.0), point_wkb(1.0, 1.0)];
    let geom_values: Vec<Option<&[u8]>> = wkbs.iter().map(|v| Some(v.as_slice())).collect();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, true),
            Field::new("geom", DataType::Binary, true),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["a", "b"])),
            Arc::new(BinaryArray::from(geom_values)),
        ],
    )
    .expect("batch");
    let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(LocalFileSystem::new());
    DuckLakeTableWriter::new(writer, object_store)
        .expect("table writer")
        .write_table("main", table, &[batch])
        .await
        .expect("write keyless geo table");
}

#[tokio::test(flavor = "multi_thread")]
async fn wire_spatial_queries_execute() {
    let (_server, client, _conn) = connect().await;

    // Spatial function execution through the wire.
    let point: String = client
        .query_one("SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))", &[])
        .await
        .expect("SELECT ST_AsText")
        .get(0);
    assert_eq!(point, "POINT(1 2)");

    let area: f64 = client
        .query_one(
            "SELECT ST_Area(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'))",
            &[],
        )
        .await
        .expect("SELECT ST_Area")
        .get(0);
    assert!((area - 16.0).abs() < 1e-9, "got {area}");

    let intersects: bool = client
        .query_one(
            "SELECT ST_Intersects(\
             ST_GeomFromText('POINT(0 0)'), \
             ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'))",
            &[],
        )
        .await
        .expect("SELECT ST_Intersects")
        .get(0);
    assert!(intersects);

    // Extended-protocol parameter binding through the wire.
    let bound: String = client
        .query_one(
            "SELECT ST_AsText(ST_GeomFromText($1))",
            &[&"POINT(3 4)".to_string()],
        )
        .await
        .expect("SELECT with $1")
        .get(0);
    assert_eq!(bound, "POINT(3 4)");

    // Basic transaction control passes through (no-op transaction hook).
    client.simple_query("BEGIN").await.expect("BEGIN");
    client.simple_query("COMMIT").await.expect("COMMIT");
}

#[tokio::test(flavor = "multi_thread")]
async fn ddl_in_memory_table_roundtrip() {
    // Unqualified public tables should round-trip within a single
    // SessionContext. Clients may use these names for scratch or default-schema
    // writes, and QuackGIS routes them to the DuckLake-backed public alias.
    let (_server, client, _conn) = connect().await;

    client
        .batch_execute(
            "CREATE TABLE points (id INT, label VARCHAR, geom VARCHAR);\
             INSERT INTO points VALUES \
             (1, 'a', 'POINT(0 0)'),\
             (2, 'b', 'POINT(1 1)'),\
             (3, 'c', 'POINT(2 2)');",
        )
        .await
        .expect("CREATE + INSERT");

    let rows = client
        .query("SELECT id, label FROM points ORDER BY id", &[])
        .await
        .expect("SELECT");
    let labels: Vec<String> = rows.into_iter().map(|r| r.get::<_, String>(1)).collect();
    assert_eq!(labels, vec!["a", "b", "c"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn wire_cursors_declare_fetch_close() {
    // G3 readiness: cursor machinery works on master's default hook list for
    // the simple-query / libpq-style path (psql, QGIS). FETCH via the extended
    // protocol (pgjdbc, tokio-postgres) currently fails upstream with
    // "DataRow field count does not match the number of columns" — the
    // CursorStatementHook stores a StoredStatement with no schema and FETCH
    // doesn't emit a matching RowDescription. Tracked as a G3 follow-up.
    let (_server, client, _conn) = connect().await;

    let declare_rows = client
        .simple_query("DECLARE c CURSOR FOR SELECT n FROM generate_series(1, 5) AS t(n)")
        .await
        .expect("DECLARE");
    // simple_query returns a Vec<SimpleQueryMessage>; for a DDL/command tag
    // the response is exactly one CommandComplete entry, no Row variants.
    assert_eq!(
        declare_rows.len(),
        1,
        "DECLARE returns a single CommandComplete (got {declare_rows:?})"
    );

    let page1 = client
        .simple_query("FETCH FORWARD 2 FROM c")
        .await
        .expect("FETCH 1");
    // FETCH returns RowDescription + N Row messages + a CommandComplete trailer.
    assert_eq!(
        page1.len(),
        4,
        "first page = rowdesc + 2 rows + command-complete (got {} msgs)",
        page1.len()
    );

    let page2 = client
        .simple_query("FETCH FORWARD 10 FROM c")
        .await
        .expect("FETCH 2");
    // 5 total - 2 already fetched = 3 remaining + rowdesc + command-complete.
    assert_eq!(
        page2.len(),
        5,
        "second page = rowdesc + 3 rows + command-complete (got {} msgs)",
        page2.len()
    );

    let drained = client
        .simple_query("FETCH FORWARD 1 FROM c")
        .await
        .expect("FETCH 3");
    // No rows left; FETCH still emits a RowDescription + CommandComplete.
    assert_eq!(
        drained.len(),
        2,
        "cursor exhausted: rowdesc + command-complete only (got {} msgs)",
        drained.len()
    );

    client.simple_query("CLOSE c").await.expect("CLOSE");
}

#[tokio::test(flavor = "multi_thread")]
async fn wire_extended_protocol_describe() {
    // Some clients (psycopg3, pgjdbc) rely on Describe to learn the parameter
    // types of a prepared statement before binding. This test exercises that
    // path through tokio-postgres' prepared-statement caching.
    let (_server, client, _conn) = connect().await;

    let stmt = client
        .prepare("SELECT ST_AsText(ST_GeomFromText($1))")
        .await
        .expect("prepare");
    // tokio-postgres' prepare() implicitly describes; if the server can't
    // describe (e.g., DF 53 changed parameter inference), this fails above.

    let row = client
        .query_one(&stmt, &[&"POINT(9 9)".to_string()])
        .await
        .expect("execute prepared");
    let v: String = row.get(0);
    assert_eq!(v, "POINT(9 9)");
}

#[tokio::test(flavor = "multi_thread")]
async fn pg_roles_does_not_crash() {
    // Regression guard for the G2 finding (commit 99e3a7d): the blanket
    // `impl PgCatalogContextProvider for Arc<T>` in datafusion-pg-catalog
    // self-recursed (`self.roles()` resolved to the Arc impl rather than
    // the inner T), causing stack overflow on any access to pg_roles. The
    // fix lives in adonm/datafusion-postgres@quackgis/fixes (commit 2c43dc6).
    // If this test starts failing again, the upstream regression returned.
    let (_server, client, _conn) = connect().await;

    let rows = client
        .query(
            "SELECT rolname FROM pg_catalog.pg_roles ORDER BY rolname",
            &[],
        )
        .await
        .expect("SELECT pg_roles");
    let names: Vec<String> = rows.into_iter().map(|r| r.get::<_, String>(0)).collect();
    assert_eq!(
        names,
        vec!["postgres".to_string()],
        "AuthManager default has exactly the postgres role"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn binary_cursor_returns_raw_bytes() {
    // G3-BINARY regression: DECLARE <name> BINARY CURSOR must return rows in
    // the binary wire format, not hex-text bytea. QGIS/GDAL use BINARY
    // cursors specifically to skip hex decoding of WKB. Fix landed in
    // adonm/datafusion-postgres@quackgis/fixes (commit 98b3865).
    //
    // We verify via tokio-postgres' simple_query (the path that already works
    // for cursors): the text cursor returns ASCII digits, the binary cursor
    // returns raw big-endian bytes.
    let (_server, client, _conn) = connect().await;

    client
        .simple_query("DECLARE bc CURSOR FOR SELECT 42")
        .await
        .expect("DECLARE text");
    let text_rows = client
        .simple_query("FETCH FORWARD 1 FROM bc")
        .await
        .expect("FETCH text");
    client.simple_query("CLOSE bc").await.expect("CLOSE text");

    client
        .simple_query("DECLARE bcb BINARY CURSOR FOR SELECT 42")
        .await
        .expect("DECLARE binary");
    let bin_rows = client
        .simple_query("FETCH FORWARD 1 FROM bcb")
        .await
        .expect("FETCH binary");
    client
        .simple_query("CLOSE bcb")
        .await
        .expect("CLOSE binary");

    // Each FETCH = RowDescription + 1 Row + CommandComplete = 3 messages.
    assert_eq!(text_rows.len(), 3);
    assert_eq!(bin_rows.len(), 3);

    use tokio_postgres::SimpleQueryMessage::*;
    let text_value = match &text_rows[1] {
        Row(r) => r.get(0).unwrap_or("").to_string(),
        _ => String::new(),
    };
    let bin_str_repr = match &bin_rows[1] {
        Row(r) => r.get(0).unwrap_or("").to_string(),
        _ => String::new(),
    };

    // Text cursor: ASCII "42" (2 bytes).
    assert_eq!(text_value, "42", "text cursor returns ASCII '42'");

    // Binary cursor: 8 raw bytes encoding i64 = 42 big-endian
    // (0x00 00 00 00 00 00 00 2a). DataFusion's `SELECT 42` infers Int64.
    // simple_query surfaces bytes lossily as &str; we just check the byte
    // pattern is exactly the i64 BE encoding, not ASCII digits.
    let bin_bytes = bin_str_repr.as_bytes();
    assert_eq!(
        bin_bytes.len(),
        8,
        "binary cursor should return 8 raw bytes (i64 BE), got {:?} (str lossy {:?})",
        bin_bytes,
        bin_str_repr
    );
    let mut buf = [0u8; 8];
    buf.copy_from_slice(bin_bytes);
    assert_eq!(
        i64::from_be_bytes(buf),
        42,
        "binary cursor bytes should decode to i64=42, got {:?}",
        bin_bytes
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn postgis_metadata_surface_works() {
    let (_server, client, _conn) = connect().await;

    // PostGIS_Lib_Version() — Martin startup gate
    let ver: String = client
        .query_one("SELECT PostGIS_Lib_Version()", &[])
        .await
        .expect("PostGIS_Lib_Version")
        .get(0);
    assert!(ver.starts_with("3.4."), "got {ver}");

    // current_setting — Martin startup queries server_version
    let sv: String = client
        .query_one("SELECT current_setting('server_version')", &[])
        .await
        .expect("current_setting")
        .get(0);
    assert!(sv.contains("QuackGIS"), "got {sv}");

    // geometry_columns exists and is queryable — empty on fresh server
    let count: i64 = client
        .query_one("SELECT count(*) FROM geometry_columns", &[])
        .await
        .expect("geometry_columns count")
        .get(0);
    assert_eq!(count, 0);

    // spatial_ref_sys has at least EPSG:4326 and 3857
    let srid_count: i64 = client
        .query_one(
            "SELECT count(*) FROM spatial_ref_sys WHERE srid IN (4326, 3857)",
            &[],
        )
        .await
        .expect("spatial_ref_sys")
        .get(0);
    assert_eq!(srid_count, 2);

    let found_srid: i32 = client
        .query_one("SELECT Find_SRID('public', 'points', 'geom')", &[])
        .await
        .expect("Find_SRID metadata lookup")
        .get(0);
    assert_eq!(found_srid, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn postgis_extent_surface_returns_box2d_bounds() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    client
        .batch_execute(
            "CREATE TABLE quackgis.main.extent_points (id INT, geom BINARY);
             INSERT INTO quackgis.main.extent_points VALUES
               (1, X'010100000000000000000000000000000000000000'),
               (2, X'010100000000000000000000400000000000000840'),
               (3, NULL);",
        )
        .await
        .expect("create extent fixtures");

    let exact: String = client
        .query_one("SELECT ST_Extent(geom) FROM public.extent_points", &[])
        .await
        .expect("exact PostGIS extent")
        .get(0);
    assert_eq!(exact, "BOX(0 0,2 3)");

    let exact_cast: String = client
        .query_one(
            "SELECT ST_Extent(geom::geometry) FROM public.extent_points",
            &[],
        )
        .await
        .expect("exact PostGIS extent with geometry cast")
        .get(0);
    assert_eq!(exact_cast, "BOX(0 0,2 3)");

    // Without PostgreSQL statistics, PostGIS may return NULL here; clients
    // such as QGIS/GDAL then fall back to exact ST_Extent for layer bounds.
    let estimated: Option<String> = client
        .query_one(
            "SELECT ST_EstimatedExtent('public', 'extent_points', 'geom')",
            &[],
        )
        .await
        .expect("estimated PostGIS extent")
        .get(0);
    assert_eq!(estimated, None);

    let estimated_parent_only: Option<String> = client
        .query_one(
            "SELECT ST_EstimatedExtent('public', 'extent_points', 'geom', true)",
            &[],
        )
        .await
        .expect("estimated PostGIS extent parent-only overload")
        .get(0);
    assert_eq!(estimated_parent_only, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn postgis_srid_metadata_functions_roundtrip_ewkb_tags() {
    let (_server, client, _conn) = connect().await;

    let row = client
        .query_one(
            "SELECT \
               ST_SRID(ST_SetSRID(ST_GeomFromText('POINT(1 2)'), 4326)), \
               ST_SRID(ST_GeomFromEWKT('SRID=27700;POINT(1 2)')), \
               ST_SRID(ST_SetSRID(ST_GeomFromEWKT('SRID=27700;POINT(1 2)'), 0)), \
               ST_SRID(ST_MakeEnvelope(0.0, 0.0, 1.0, 1.0, 3857)), \
               ST_SRID(ST_Transform(ST_SetSRID(ST_GeomFromText('POINT(0 0)'), 4326), 3857))",
            &[],
        )
        .await
        .expect("SRID metadata function roundtrip");

    assert_eq!(row.get::<_, i32>(0), 4326);
    assert_eq!(row.get::<_, i32>(1), 27700);
    assert_eq!(row.get::<_, i32>(2), 0);
    assert_eq!(row.get::<_, i32>(3), 3857);
    assert_eq!(row.get::<_, i32>(4), 3857);
}

#[tokio::test(flavor = "multi_thread")]
async fn geometry_columns_discovers_tables_with_geom_column() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    // Create a table with a conventional geometry column name
    client
        .simple_query("CREATE TABLE quackgis.main.roads (id INT, geom BINARY, name TEXT)")
        .await
        .expect("CREATE TABLE");

    // geometry_columns should now show the 'geom' column
    let rows = client
        .query(
            "SELECT f_table_schema, f_table_name, f_geometry_column, type \
             FROM geometry_columns \
             WHERE f_table_name = 'roads'",
            &[],
        )
        .await
        .expect("SELECT geometry_columns");

    let rows_len = rows.len();
    assert_eq!(rows_len, 1, "expected 1 geometry column, got {rows_len}");
    let schema: String = rows[0].get(0);
    let table: String = rows[0].get(1);
    let col: String = rows[0].get(2);
    let geom_type: String = rows[0].get(3);
    assert_eq!(schema, "public");
    assert_eq!(table, "roads");
    assert_eq!(col, "geom");
    assert_eq!(geom_type, "GEOMETRY");

    // Non-geometry Binary columns should NOT appear
    client
        .simple_query("CREATE TABLE quackgis.main.blobs (id INT, payload BINARY, geom BINARY)")
        .await
        .expect("CREATE TABLE blobs");

    let blob_rows = client
        .query(
            "SELECT f_geometry_column FROM geometry_columns WHERE f_table_name = 'blobs'",
            &[],
        )
        .await
        .expect("SELECT blobs geometry_columns");
    let blob_len = blob_rows.len();
    assert_eq!(blob_len, 1, "only 'geom' should appear, not 'payload'");
    let only_col: String = blob_rows[0].get(0);
    assert_eq!(only_col, "geom");
}

#[tokio::test(flavor = "multi_thread")]
async fn qgis_pg_type_lookup_resolves_custom_geometry_oid() {
    let (_server, client, _conn) = connect().await;

    let messages = client
        .simple_query(
            "SELECT oid,typname,typtype,typelem,typlen FROM pg_type WHERE oid in (23,90001,25)",
        )
        .await
        .expect("QGIS pg_type lookup");

    let rows: Vec<_> = messages
        .iter()
        .filter_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some((
                row.get("oid").unwrap_or_default().to_string(),
                row.get("typname").unwrap_or_default().to_string(),
                row.get("typtype").unwrap_or_default().to_string(),
                row.get("typelem").unwrap_or_default().to_string(),
                row.get("typlen").unwrap_or_default().to_string(),
            )),
            _ => None,
        })
        .collect();

    assert!(
        rows.contains(&(
            "90001".to_string(),
            "geometry".to_string(),
            "b".to_string(),
            "0".to_string(),
            "-1".to_string()
        )),
        "custom geometry oid must be present in {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|(oid, typname, _, _, _)| oid == "23" && typname == "int4"),
        "lookup should preserve int4 row for mixed QGIS field discovery: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|(oid, typname, _, _, _)| oid == "25" && typname == "text"),
        "lookup should preserve text row for mixed QGIS field discovery: {rows:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn qgis_primary_key_catalog_shim_exposes_synthetic_index() {
    let (_server, client, _conn) = connect().await;

    let index_rows = client
        .simple_query(
            "SELECT indexrelid FROM pg_index WHERE indrelid='\"public\".\"points\"'::regclass \
             AND (indisprimary OR indisunique) ORDER BY CASE WHEN indisprimary THEN 1 ELSE 2 END LIMIT 1",
        )
        .await
        .expect("QGIS indexrelid lookup");
    let index_oid = index_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("indexrelid"),
        _ => None,
    });
    assert_eq!(index_oid, Some("90101"));

    let indkey_rows = client
        .simple_query("SELECT indkey FROM pg_index WHERE indexrelid=90101")
        .await
        .expect("QGIS indkey lookup");
    let indkey = indkey_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("indkey"),
        _ => None,
    });
    assert_eq!(indkey, Some("1"));

    let key_column_rows = client
        .simple_query(
            "SELECT attname,attnotnull FROM pg_index,pg_attribute WHERE indexrelid=90101 \
             AND indrelid=attrelid AND pg_attribute.attnum=any(pg_index.indkey)",
        )
        .await
        .expect("QGIS primary-key column lookup");
    let key_column = key_column_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => Some((
            row.get("attname").unwrap_or_default(),
            row.get("attnotnull").unwrap_or_default(),
        )),
        _ => None,
    });
    assert_eq!(key_column, Some(("id", "t")));

    let def_rows = client
        .simple_query("SELECT pg_get_indexdef(90101)")
        .await
        .expect("QGIS pg_get_indexdef lookup");
    let index_def = def_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("pg_get_indexdef"),
        _ => None,
    });
    assert_eq!(
        index_def,
        Some("CREATE UNIQUE INDEX points_pkey ON public.points (id)")
    );

    let layer_styles_rows = client
        .simple_query(
            "SELECT EXISTS ( SELECT oid FROM pg_catalog.pg_class WHERE relname='layer_styles')",
        )
        .await
        .expect("QGIS layer_styles existence lookup");
    let layer_styles_exists = layer_styles_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("exists"),
        _ => None,
    });
    assert_eq!(layer_styles_exists, Some("f"));
}

#[tokio::test(flavor = "multi_thread")]
async fn qgis_primary_key_catalog_shim_uses_table_schema() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    client
        .simple_query("CREATE TABLE quackgis.main.roads_keyed (name TEXT, id INT, geom BINARY)")
        .await
        .expect("create keyed table");

    let index_rows = client
        .simple_query(
            "SELECT indexrelid FROM pg_index WHERE indrelid='\"public\".\"roads_keyed\"'::regclass \
             AND (indisprimary OR indisunique) ORDER BY CASE WHEN indisprimary THEN 1 ELSE 2 END LIMIT 1",
        )
        .await
        .expect("schema-derived indexrelid lookup");
    let index_oid = index_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("indexrelid"),
        _ => None,
    });
    let index_oid = index_oid.expect("synthetic index oid for roads_keyed");
    assert_ne!(
        index_oid, "90101",
        "non-points tables should not reuse the legacy points synthetic OID"
    );

    let indkey_rows = client
        .simple_query(&format!(
            "SELECT indkey FROM pg_index WHERE indexrelid={index_oid}"
        ))
        .await
        .expect("schema-derived indkey lookup");
    let indkey = indkey_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("indkey"),
        _ => None,
    });
    assert_eq!(indkey, Some("2"), "id is the second column");

    let key_column_rows = client
        .simple_query(&format!(
            "SELECT attname,attnotnull FROM pg_index,pg_attribute WHERE indexrelid={index_oid} \
             AND indrelid=attrelid AND pg_attribute.attnum=any(pg_index.indkey)"
        ))
        .await
        .expect("schema-derived primary-key column lookup");
    let key_column = key_column_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => Some((
            row.get("attname").unwrap_or_default(),
            row.get("attnotnull").unwrap_or_default(),
        )),
        _ => None,
    });
    assert_eq!(key_column, Some(("id", "t")));

    let def_rows = client
        .simple_query(&format!("SELECT pg_get_indexdef({index_oid})"))
        .await
        .expect("schema-derived pg_get_indexdef lookup");
    let index_def = def_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("pg_get_indexdef"),
        _ => None,
    });
    assert_eq!(
        index_def,
        Some("CREATE UNIQUE INDEX roads_keyed_pkey ON public.roads_keyed (id)")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn qgis_keyless_layer_gets_synthetic_rowid_metadata_and_projection() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    client
        .batch_execute(
            "CREATE TABLE quackgis.main.keyless_qgis (name TEXT, geom BINARY);
             INSERT INTO quackgis.main.keyless_qgis (name, geom) VALUES
               ('a', X'010100000000000000000000000000000000000000'),
               ('b', X'0101000000000000000000f03f000000000000f03f');",
        )
        .await
        .expect("create keyless spatial table");

    let rows = client
        .query(
            "SELECT \"_quackgis_rowid\", name FROM public.keyless_qgis ORDER BY \"_quackgis_rowid\"",
            &[],
        )
        .await
        .expect("QGIS synthetic rowid feature projection");
    let got: Vec<(i64, String)> = rows
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(got, vec![(1, "a".to_string()), (2, "b".to_string())]);

    let index_rows = client
        .simple_query(
            "SELECT indexrelid FROM pg_index WHERE indrelid='\"public\".\"keyless_qgis\"'::regclass \
             AND (indisprimary OR indisunique) ORDER BY CASE WHEN indisprimary THEN 1 ELSE 2 END LIMIT 1",
        )
        .await
        .expect("QGIS keyless indexrelid lookup");
    let index_oid = index_rows
        .iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => row.get("indexrelid"),
            _ => None,
        })
        .expect("synthetic rowid index oid");

    let indkey_rows = client
        .simple_query(&format!(
            "SELECT indkey FROM pg_index WHERE indexrelid={index_oid}"
        ))
        .await
        .expect("QGIS keyless indkey lookup");
    let indkey = indkey_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("indkey"),
        _ => None,
    });
    assert_eq!(indkey, Some("1"));

    let key_column_rows = client
        .simple_query(&format!(
            "SELECT attname,attnotnull FROM pg_index,pg_attribute WHERE indexrelid={index_oid} \
             AND indrelid=attrelid AND pg_attribute.attnum=any(pg_index.indkey)"
        ))
        .await
        .expect("QGIS keyless primary-key column lookup");
    let key_column = key_column_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => Some((
            row.get("attname").unwrap_or_default(),
            row.get("attnotnull").unwrap_or_default(),
        )),
        _ => None,
    });
    assert_eq!(key_column, Some(("_quackgis_rowid", "t")));

    let def_rows = client
        .simple_query(&format!("SELECT pg_get_indexdef({index_oid})"))
        .await
        .expect("QGIS keyless pg_get_indexdef lookup");
    let index_def = def_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("pg_get_indexdef"),
        _ => None,
    });
    assert_eq!(
        index_def,
        Some("CREATE UNIQUE INDEX keyless_qgis_pkey ON public.keyless_qgis (_quackgis_rowid)")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn qgis_keyless_writer_table_gets_virtual_rowid_projection() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let paths = StoragePaths::new(
        tmp.path().join("quackgis.db").to_str().unwrap(),
        tmp.path().join("data").to_str().unwrap(),
    )
    .expect("storage paths");
    write_keyless_geo(&paths, "writer_keyless_qgis").await;

    let server = ServerHandle::start_with_tempdir(tmp).await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    let rows = client
        .query(
            "SELECT \"_quackgis_rowid\", name FROM public.writer_keyless_qgis ORDER BY \"_quackgis_rowid\"",
            &[],
        )
        .await
        .expect("virtual rowid projection for writer-backed keyless table");
    let got: Vec<(i64, String)> = rows
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(got, vec![(1, "a".to_string()), (2, "b".to_string())]);

    let index_rows = client
        .simple_query(
            "SELECT indexrelid FROM pg_index WHERE indrelid='\"public\".\"writer_keyless_qgis\"'::regclass \
             AND (indisprimary OR indisunique) ORDER BY CASE WHEN indisprimary THEN 1 ELSE 2 END LIMIT 1",
        )
        .await
        .expect("QGIS writer-backed keyless indexrelid lookup");
    let index_oid = index_rows
        .iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => row.get("indexrelid"),
            _ => None,
        })
        .expect("virtual synthetic rowid index oid");

    let indkey_rows = client
        .simple_query(&format!(
            "SELECT indkey FROM pg_index WHERE indexrelid={index_oid}"
        ))
        .await
        .expect("QGIS writer-backed keyless indkey lookup");
    let indkey = indkey_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("indkey"),
        _ => None,
    });
    assert_eq!(indkey, Some("1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn qgis_edit_save_shape_dml_returning_roundtrips_rowid() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    client
        .batch_execute("CREATE TABLE public.qgis_edit_returning (name TEXT, geom BINARY)")
        .await
        .expect("create keyless edit table");

    let inserted = client
        .query(
            "INSERT INTO public.qgis_edit_returning (name, geom) VALUES
              ('draft', X'010100000000000000000000000000000000000000')
             RETURNING \"_quackgis_rowid\", name, ST_AsText(ST_GeomFromWKB(geom))",
            &[],
        )
        .await
        .expect("QGIS-shaped INSERT RETURNING");
    assert_eq!(inserted.len(), 1);
    let rowid: i64 = inserted[0].get(0);
    let name: String = inserted[0].get(1);
    let wkt: String = inserted[0].get(2);
    assert_eq!(rowid, 1);
    assert_eq!(name, "draft");
    assert_eq!(wkt, "POINT(0 0)");

    let updated = client
        .query(
            "UPDATE public.qgis_edit_returning
             SET name = 'saved'
             WHERE \"_quackgis_rowid\" = 1
             RETURNING \"_quackgis_rowid\", name",
            &[],
        )
        .await
        .expect("QGIS-shaped UPDATE RETURNING");
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].get::<_, i64>(0), 1);
    assert_eq!(updated[0].get::<_, String>(1), "saved");

    let deleted = client
        .query(
            "DELETE FROM public.qgis_edit_returning
             WHERE \"_quackgis_rowid\" = 1
             RETURNING \"_quackgis_rowid\", name",
            &[],
        )
        .await
        .expect("QGIS-shaped DELETE RETURNING");
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].get::<_, i64>(0), 1);
    assert_eq!(deleted[0].get::<_, String>(1), "saved");

    let count: i64 = client
        .query_one("SELECT count(*) FROM public.qgis_edit_returning", &[])
        .await
        .expect("post-delete count")
        .get(0);
    assert_eq!(count, 0);

    let simple_insert = client
        .simple_query(
            "INSERT INTO public.qgis_edit_returning (name, geom) VALUES
              ('simple', X'0101000000000000000000f03f000000000000f03f')
             RETURNING \"_quackgis_rowid\", name",
        )
        .await
        .expect("simple-query INSERT RETURNING");
    let simple_row = simple_insert.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => Some((
            row.get("_quackgis_rowid").unwrap_or_default().to_string(),
            row.get("name").unwrap_or_default().to_string(),
        )),
        _ => None,
    });
    assert_eq!(simple_row, Some(("1".to_string(), "simple".to_string())));
}

#[tokio::test(flavor = "multi_thread")]
async fn qgis_st_asbinary_binary_endian_overload_returns_wkb() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    client
        .simple_query("CREATE TABLE quackgis.main.points (id INT, geom BINARY, name TEXT)")
        .await
        .expect("create points");
    client
        .simple_query(
            "INSERT INTO quackgis.main.points VALUES \
             (1, X'010100000000000000000000000000000000000000', 'origin')",
        )
        .await
        .expect("insert point");

    let wkb: Vec<u8> = client
        .query_one(
            "SELECT st_asbinary(\"geom\", 'NDR') FROM \"public\".\"points\" WHERE id = 1",
            &[],
        )
        .await
        .expect("QGIS ST_AsBinary overload")
        .get(0);

    assert_eq!(
        wkb,
        vec![
            1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_table_listing_query_does_not_cast_pg_class_literal() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    client
        .simple_query("CREATE TABLE quackgis.main.ogr_visible (id INT, wkb_geometry BINARY)")
        .await
        .expect("create OGR-visible table");

    let rows = client
        .query(
            "SELECT c.relname, n.nspname, d.description \
             FROM pg_class c \
             JOIN pg_namespace n ON c.relnamespace=n.oid \
             LEFT JOIN pg_description d \
               ON d.objoid = c.oid \
              AND d.classoid = 'pg_class'::regclass::oid \
              AND d.objsubid = 0 \
             WHERE (c.relkind in ('r','v','m','f') AND c.relname !~ '^pg_')",
            &[],
        )
        .await
        .expect("OGR table listing catalog query should not trip oid coercion");

    let names: Vec<(String, String)> = rows
        .into_iter()
        .map(|row| (row.get("relname"), row.get("nspname")))
        .collect();
    assert!(
        names.contains(&("ogr_visible".to_string(), "public".to_string())),
        "OGR table listing should expose DuckLake main as public, got {names:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_postgis_type_probe_returns_empty_oid_typname_shape() {
    let (_server, client, _conn) = connect().await;

    let messages = client
        .simple_query(
            "SELECT oid, typname FROM pg_type \
             WHERE typname IN ('geometry', 'geography') AND typtype='b'",
        )
        .await
        .expect("OGR PostGIS type probe should return oid/typname shape");

    let rows = messages
        .iter()
        .filter(|message| matches!(message, tokio_postgres::SimpleQueryMessage::Row(_)))
        .count();
    assert_eq!(rows, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_column_listing_query_returns_matching_field_count() {
    let (_server, client, _conn) = connect().await;

    let rows = client
        .query(
            "SELECT a.attname, t.typname, a.attlen, \
                    format_type(a.atttypid,a.atttypmod), a.attnotnull, \
                    def.def, i.indisunique, descr.description \
             FROM pg_attribute a \
             JOIN pg_type t ON t.oid = a.atttypid \
             LEFT JOIN (SELECT adrelid, adnum, pg_get_expr(adbin, adrelid) AS def FROM pg_attrdef) def \
                    ON def.adrelid = a.attrelid AND def.adnum = a.attnum \
             LEFT JOIN (SELECT DISTINCT indrelid, indkey, indisunique FROM pg_index WHERE indisunique) i \
                    ON i.indrelid = a.attrelid AND i.indkey[0] = a.attnum AND i.indkey[1] IS NULL \
             LEFT JOIN pg_description descr \
                    ON descr.objoid = a.attrelid \
                   AND descr.classoid = 'pg_class'::regclass::oid \
                   AND descr.objsubid = a.attnum \
             WHERE a.attnum > 0 AND a.attrelid = 16479 \
             ORDER BY a.attnum",
            &[],
        )
        .await
        .expect("OGR column listing should return rows matching RowDescription");

    let cols: Vec<(String, String)> = rows
        .into_iter()
        .map(|row| (row.get("attname"), row.get("typname")))
        .collect();
    assert_eq!(
        cols,
        vec![
            ("id".to_string(), "int4".to_string()),
            ("wkb_geometry".to_string(), "bytea".to_string()),
            ("name".to_string(), "text".to_string())
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_primary_key_probe_returns_matching_field_count() {
    let (_server, client, _conn) = connect().await;

    let rows = client
        .query(
            "SELECT a.attname, a.attnum, t.typname, \
                    t.typname = ANY(ARRAY['int2','int4','int8','serial','bigserial']) AS isfid \
             FROM pg_attribute a \
             JOIN pg_type t ON t.oid = a.atttypid \
             JOIN pg_index i ON i.indrelid = a.attrelid \
             WHERE a.attnum > 0 \
               AND a.attrelid = 16395 \
               AND i.indisprimary = 't' \
               AND t.typname !~ '^geom' \
               AND a.attnum = ANY(i.indkey) \
             ORDER BY a.attnum",
            &[],
        )
        .await
        .expect("OGR primary-key probe should return a 4-column empty result");

    assert!(rows.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_geography_columns_probe_returns_matching_field_count() {
    let (_server, client, _conn) = connect().await;

    let rows = client
        .query(
            "SELECT type, coord_dimension, srid \
             FROM geography_columns \
             WHERE f_table_name = 'ogr_visible' \
               AND f_geography_column='wkb_geometry' \
               AND f_table_schema = 'public'",
            &[],
        )
        .await
        .expect("OGR geography_columns probe should return a 3-column empty result");

    assert!(rows.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_inheritance_parent_probe_returns_relname_shape() {
    let (_server, client, _conn) = connect().await;

    let rows = client
        .query(
            "SELECT pg_class.relname FROM pg_class \
             WHERE oid = (SELECT pg_inherits.inhparent FROM pg_inherits \
                          WHERE inhrelid = (SELECT c.oid FROM pg_class c, pg_namespace n \
                                            WHERE c.relname = 'ogr_visible' \
                                              AND c.relnamespace=n.oid \
                                              AND n.nspname = 'public'))",
            &[],
        )
        .await
        .expect("OGR inheritance parent probe should return relname-shaped rows");

    assert!(rows.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_extended_cursor_fetch_returns_matching_feature_rows() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    client
        .simple_query(
            "CREATE TABLE quackgis.main.ogr_cursor (id INT, wkb_geometry BINARY, name TEXT)",
        )
        .await
        .expect("create OGR cursor table");
    client
        .simple_query(
            "INSERT INTO quackgis.main.ogr_cursor VALUES \
             (1, X'010100000000000000000000000000000000000000', 'origin')",
        )
        .await
        .expect("insert OGR cursor row");

    client.query("BEGIN", &[]).await.expect("begin");
    client
        .query(
            "DECLARE OGRPGLayerReader0xabc CURSOR FOR \
             SELECT \"wkb_geometry\", \"id\", \"name\" FROM \"ogr_cursor\"",
            &[],
        )
        .await
        .expect("declare OGR cursor");
    let rows = client
        .query("FETCH 500 IN OGRPGLayerReader0xabc", &[])
        .await
        .expect("fetch OGR cursor via extended protocol");
    client.query("COMMIT", &[]).await.expect("commit");

    assert_eq!(rows.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_write_load_shape_alter_add_column_and_insert_roundtrips() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    // OGR append-with-addfields writes through the PostgreSQL `public` schema,
    // first creating/inspecting a spatial table and then adding source fields
    // before INSERT. The actual Kind probe keeps this test name as provenance;
    // this Rust regression pins the DuckLake write-routing pieces directly.
    client
        .batch_execute(
            "CREATE TABLE public.ogr_write_load (id INT, wkb_geometry GEOMETRY(Point,4326));
             ALTER TABLE public.ogr_write_load ADD COLUMN name VARCHAR;",
        )
        .await
        .expect("OGR-shaped CREATE/ALTER");

    client
        .execute(
            "ALTER TABLE \"ogr_write_load\" ADD COLUMN \"category\" VARCHAR",
            &[],
        )
        .await
        .expect("OGR extended ALTER ADD COLUMN");
    client
        .execute(
            r#"INSERT INTO "ogr_write_load" ("wkb_geometry" , "name", "category") VALUES (E'\\001\\001\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000@\\000\\000\\000\\000\\000\\000\\000@', 'load-a', 'client')"#,
            &[],
        )
        .await
        .expect("OGR extended INSERT first feature");
    client
        .execute(
            r#"INSERT INTO "ogr_write_load" ("wkb_geometry" , "name", "category") VALUES (E'\\001\\001\\000\\000\\000\\000\\000\\000\\000\\000\\000\\010@\\000\\000\\000\\000\\000\\000\\010@', 'load-b', 'client')"#,
            &[],
        )
        .await
        .expect("OGR extended INSERT second feature");

    let rows = client
        .query(
            "SELECT name, ST_AsText(ST_GeomFromWKB(wkb_geometry)), category
             FROM public.ogr_write_load
             ORDER BY name",
            &[],
        )
        .await
        .expect("read back OGR-loaded rows");

    let got: Vec<(String, String, Option<String>)> = rows
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    assert_eq!(
        got,
        vec![
            (
                "load-a".to_string(),
                "POINT(2 2)".to_string(),
                Some("client".to_string())
            ),
            (
                "load-b".to_string(),
                "POINT(3 3)".to_string(),
                Some("client".to_string())
            ),
        ]
    );
}
