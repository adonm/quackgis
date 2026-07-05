// SPDX-License-Identifier: Apache-2.0
//! End-to-end smoke: spin up the actual pgwire server on an ephemeral port and
//! drive it with `tokio-postgres`. Verifies the full wire stack (not just the
//! in-process SedonaDB call) without needing psql on the host.

mod common;

use tokio_postgres::NoTls;

use common::ServerHandle;

async fn connect() -> (tokio_postgres::Client, tokio::task::JoinHandle<()>) {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect to quackgis-server");
    let conn_task = tokio::spawn(async move {
        let _ = connection.await;
    });
    (client, conn_task)
}

#[tokio::test(flavor = "multi_thread")]
async fn wire_spatial_queries_execute() {
    let (client, _conn) = connect().await;

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
    // M0 has no persistence — but in-process DataFusion tables should round-trip
    // within a single SessionContext. This pins the contract for M1 (DuckLake):
    // whatever CREATE TABLE semantics land at M1 must keep this test green.
    let (client, _conn) = connect().await;

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
    let (client, _conn) = connect().await;

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
    let (client, _conn) = connect().await;

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
    let (client, _conn) = connect().await;

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
    let (client, _conn) = connect().await;

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
    let (client, _conn) = connect().await;

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
    assert_eq!(schema, "main");
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
