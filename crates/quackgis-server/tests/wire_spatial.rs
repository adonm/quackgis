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
