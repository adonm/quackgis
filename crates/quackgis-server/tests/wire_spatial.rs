// SPDX-License-Identifier: Apache-2.0
//! End-to-end smoke: spin up the actual pgwire server on an ephemeral port and
//! drive it with `tokio-postgres`. Verifies the full wire stack (not just the
//! in-process SedonaDB call) without needing psql on the host.

mod common;

use std::{collections::BTreeMap, sync::Arc};

use datafusion::arrow::array::{BinaryArray, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion_ducklake::DuckLakeTableWriter;
use quackgis_server::auth::{AuthConfig, parse_write_allowlist};
use quackgis_server::context::StoragePaths;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_postgres::NoTls;
use tokio_postgres::types::Type;

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

async fn startup_auth_code(port: u16, user: &str) -> i32 {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("raw startup TCP connect");

    let mut payload = Vec::new();
    payload.extend_from_slice(&196_608_i32.to_be_bytes()); // protocol 3.0
    for (key, value) in [("user", user), ("database", "quackgis")] {
        payload.extend_from_slice(key.as_bytes());
        payload.push(0);
        payload.extend_from_slice(value.as_bytes());
        payload.push(0);
    }
    payload.push(0);
    let len = (payload.len() + 4) as i32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .expect("write startup length");
    stream
        .write_all(&payload)
        .await
        .expect("write startup payload");

    let mut tag = [0_u8; 1];
    stream
        .read_exact(&mut tag)
        .await
        .expect("read startup response tag");
    assert_eq!(
        tag[0], b'R',
        "first startup response should be Authentication"
    );

    let mut len_bytes = [0_u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .await
        .expect("read auth response length");
    let body_len = i32::from_be_bytes(len_bytes) as usize - 4;
    let mut body = vec![0_u8; body_len];
    stream
        .read_exact(&mut body)
        .await
        .expect("read auth response body");
    i32::from_be_bytes(body[0..4].try_into().expect("auth code bytes"))
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
    let writer = paths.metadata_writer().await.expect("writer");
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
    let object_store = paths.object_store().expect("object store");
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
async fn password_auth_and_readonly_role_fail_closed() {
    let auth = AuthConfig::password(
        "postgres",
        "readwrite-secret",
        Some(("quackgis_readonly", "readonly-secret")),
    )
    .expect("auth config");
    let server = ServerHandle::start_with_auth(auth).await;

    assert_eq!(
        startup_auth_code(server.port(), "postgres").await,
        10,
        "password auth must negotiate PostgreSQL SASL/SCRAM, not cleartext-password"
    );

    let bad_login =
        tokio_postgres::connect(&format!("{} password=wrong", server.conn_str()), NoTls).await;
    assert!(bad_login.is_err(), "bad password unexpectedly connected");

    let unknown_login = tokio_postgres::connect(
        &format!(
            "host=127.0.0.1 port={} user=unknown dbname=quackgis password=readwrite-secret",
            server.port()
        ),
        NoTls,
    )
    .await;
    assert!(
        unknown_login.is_err(),
        "unknown user unexpectedly connected"
    );

    let bad_readonly_login = tokio_postgres::connect(
        &format!(
            "host=127.0.0.1 port={} user=quackgis_readonly dbname=quackgis password=wrong",
            server.port()
        ),
        NoTls,
    )
    .await;
    assert!(
        bad_readonly_login.is_err(),
        "readonly user with bad password unexpectedly connected"
    );

    let concurrent_rw = format!("{} password=readwrite-secret", server.conn_str());
    let concurrent_ro = format!(
        "host=127.0.0.1 port={} user=quackgis_readonly dbname=quackgis password=readonly-secret",
        server.port()
    );
    let (concurrent_rw, concurrent_ro) = tokio::join!(
        tokio_postgres::connect(&concurrent_rw, NoTls),
        tokio_postgres::connect(&concurrent_ro, NoTls),
    );
    let (_concurrent_rw, concurrent_rw_connection) = concurrent_rw.expect("concurrent rw login");
    let (_concurrent_ro, concurrent_ro_connection) = concurrent_ro.expect("concurrent ro login");
    let _concurrent_rw_conn = tokio::spawn(concurrent_rw_connection);
    let _concurrent_ro_conn = tokio::spawn(concurrent_ro_connection);

    let (writer, writer_connection) = tokio_postgres::connect(
        &format!("{} password=readwrite-secret", server.conn_str()),
        NoTls,
    )
    .await
    .expect("readwrite login");
    assert_eq!(writer_connection.parameter("is_superuser"), Some("off"));
    assert_eq!(
        writer_connection.parameter("default_transaction_read_only"),
        Some("off")
    );
    let _writer_conn = tokio::spawn(writer_connection);
    writer
        .simple_query("CREATE TABLE public.auth_rw_points (id INT, geom BINARY)")
        .await
        .expect("readwrite role can create DuckLake tables");
    writer
        .simple_query(
            "INSERT INTO public.auth_rw_points VALUES \
             (1, X'010100000000000000000000000000000000000000')",
        )
        .await
        .expect("readwrite role can seed DuckLake tables");

    let roles = writer
        .query(
            "SELECT rolname, rolsuper, rolcanlogin, rolcreatedb, rolcreaterole, rolreplication \
             FROM pg_catalog.pg_roles \
             WHERE rolname IN ('postgres', 'quackgis_readonly')",
            &[],
        )
        .await
        .expect("pg_roles reflects configured login roles");
    let role_flags: BTreeMap<String, (bool, bool, bool, bool, bool)> = roles
        .into_iter()
        .map(|row| {
            (
                row.get::<_, String>(0),
                (
                    row.get::<_, bool>(1),
                    row.get::<_, bool>(2),
                    row.get::<_, bool>(3),
                    row.get::<_, bool>(4),
                    row.get::<_, bool>(5),
                ),
            )
        })
        .collect();
    assert_eq!(
        role_flags.get("postgres"),
        Some(&(false, true, false, false, false))
    );
    assert_eq!(
        role_flags.get("quackgis_readonly"),
        Some(&(false, true, false, false, false))
    );

    let privilege_metadata = writer
        .query_one(
            "SELECT \
               has_database_privilege('quackgis_readonly', 'quackgis', 'CONNECT'), \
               has_schema_privilege('quackgis_readonly', 'public', 'USAGE'), \
               has_table_privilege('quackgis_readonly', 'public.auth_rw_points', 'SELECT'), \
               has_table_privilege('quackgis_readonly', 'public.auth_rw_points', 'UPDATE'), \
               has_column_privilege('quackgis_readonly', 'public.auth_rw_points', 'geom', 'UPDATE'), \
               has_table_privilege('postgres', 'public.auth_rw_points', 'UPDATE'), \
               has_table_privilege('missing_user', 'public.auth_rw_points', 'SELECT')",
            &[],
        )
        .await
        .expect("explicit-user privilege metadata reflects configured roles");
    assert!(privilege_metadata.get::<_, bool>(0));
    assert!(privilege_metadata.get::<_, bool>(1));
    assert!(privilege_metadata.get::<_, bool>(2));
    assert!(!privilege_metadata.get::<_, bool>(3));
    assert!(!privilege_metadata.get::<_, bool>(4));
    assert!(privilege_metadata.get::<_, bool>(5));
    assert!(!privilege_metadata.get::<_, bool>(6));

    let readonly_conn = format!(
        "host=127.0.0.1 port={} user=quackgis_readonly dbname=quackgis password=readonly-secret",
        server.port()
    );
    let (readonly, readonly_connection) = tokio_postgres::connect(&readonly_conn, NoTls)
        .await
        .expect("readonly login");
    assert_eq!(readonly_connection.parameter("is_superuser"), Some("off"));
    assert_eq!(
        readonly_connection.parameter("default_transaction_read_only"),
        Some("on")
    );
    let _readonly_conn = tokio::spawn(readonly_connection);
    let count: i64 = readonly
        .query_one("SELECT COUNT(*) FROM public.auth_rw_points", &[])
        .await
        .expect("readonly role can query")
        .get(0);
    assert_eq!(count, 1);
    readonly
        .batch_execute(
            "SET application_name = 'quackgis-readonly-probe';
             SHOW application_name;
             BEGIN;
             EXPLAIN SELECT * FROM public.auth_rw_points;
             ROLLBACK;",
        )
        .await
        .expect("read-only role can use safe session, transaction, and explain surfaces");

    let denied_before = quackgis_server::ducklake_sql::metrics_snapshot().writes_denied_total;
    let denied_writes = [
        (
            "create table",
            "CREATE TABLE public.auth_ro_denied (id INT)",
        ),
        (
            "insert",
            "INSERT INTO public.auth_rw_points VALUES \
             (2, X'0101000000000000000000F03F000000000000F03F')",
        ),
        (
            "update",
            "UPDATE public.auth_rw_points SET id = 3 WHERE id = 1",
        ),
        ("delete", "DELETE FROM public.auth_rw_points WHERE id = 1"),
        ("drop", "DROP TABLE public.auth_rw_points"),
        ("truncate", "TRUNCATE TABLE public.auth_rw_points"),
        (
            "alter table",
            "ALTER TABLE public.auth_rw_points ADD COLUMN denied TEXT",
        ),
        (
            "create view",
            "CREATE VIEW public.auth_ro_view AS SELECT * FROM public.auth_rw_points",
        ),
        ("create schema", "CREATE SCHEMA auth_ro_schema"),
        ("analyze", "ANALYZE public.auth_rw_points"),
        (
            "explain delete",
            "EXPLAIN DELETE FROM public.auth_rw_points WHERE id = 1",
        ),
        (
            "unknown call",
            "CALL future_admin_operation('public.auth_rw_points')",
        ),
        (
            "compact",
            "CALL quackgis_compact_table('public.auth_rw_points')",
        ),
    ];
    for (label, sql) in denied_writes {
        let denied = readonly.simple_query(sql).await.expect_err(&format!(
            "readonly role unexpectedly executed {label}: {sql}"
        ));
        assert_eq!(
            denied.code(),
            Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
            "{label} must be denied by QuackGIS authorization, not an incidental planner error: {denied}"
        );
    }

    let copy_denied = readonly
        .simple_query("COPY public.auth_rw_points FROM STDIN")
        .await;
    let copy_denied = copy_denied.expect_err("readonly role unexpectedly entered COPY FROM STDIN");
    assert_eq!(
        copy_denied.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
    let denied_after = quackgis_server::ducklake_sql::metrics_snapshot().writes_denied_total;
    assert!(
        denied_after > denied_before + denied_writes.len() as u64,
        "read-only denied writes should increment metrics: before={denied_before} after={denied_after} denied_sql={}",
        denied_writes.len()
    );

    let count_after_denials: i64 = writer
        .query_one("SELECT COUNT(*) FROM public.auth_rw_points", &[])
        .await
        .expect("readwrite role can verify readonly denials")
        .get(0);
    assert_eq!(count_after_denials, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn password_auth_write_allowlist_limits_readwrite_targets() {
    let auth = AuthConfig::password("postgres", "readwrite-secret", None::<(String, String)>)
        .expect("auth config")
        .with_readwrite_allowlist(
            parse_write_allowlist("public.auth_write_allowed").expect("allowlist parses"),
        );
    let server = ServerHandle::start_with_auth(auth).await;
    let (writer, writer_connection) = tokio_postgres::connect(
        &format!("{} password=readwrite-secret", server.conn_str()),
        NoTls,
    )
    .await
    .expect("readwrite login");
    let _writer_conn = tokio::spawn(writer_connection);

    writer
        .simple_query("CREATE TABLE public.auth_write_allowed (id INT, geom BINARY)")
        .await
        .expect("allowlisted table can be created");
    writer
        .simple_query(
            "INSERT INTO auth_write_allowed VALUES \
             (1, X'010100000000000000000000000000000000000000')",
        )
        .await
        .expect("allowlisted bare target can be inserted");
    writer
        .simple_query(
            "UPDATE quackgis.main.auth_write_allowed SET id = 2 WHERE id = 1; \
             DELETE FROM main.auth_write_allowed WHERE id = 2",
        )
        .await
        .expect("equivalent normalized target names stay allowed");

    let privilege_metadata = writer
        .query_one(
            "SELECT \
               has_table_privilege('postgres', 'public.auth_write_allowed', 'UPDATE'), \
               has_table_privilege('postgres', 'public.auth_write_denied', 'UPDATE'), \
               has_table_privilege('postgres', 'public.auth_write_denied', 'SELECT'), \
               has_column_privilege('postgres', 'public.auth_write_denied', 'geom', 'UPDATE')",
            &[],
        )
        .await
        .expect("privilege metadata reflects write allowlist");
    assert!(privilege_metadata.get::<_, bool>(0));
    assert!(!privilege_metadata.get::<_, bool>(1));
    assert!(privilege_metadata.get::<_, bool>(2));
    assert!(!privilege_metadata.get::<_, bool>(3));

    let denied_before = quackgis_server::ducklake_sql::metrics_snapshot().writes_denied_total;
    let denied_writes = [
        (
            "create denied table",
            "CREATE TABLE public.auth_write_denied (id INT)",
        ),
        (
            "insert denied table",
            "INSERT INTO public.auth_write_denied VALUES (1)",
        ),
        (
            "update denied table",
            "UPDATE public.auth_write_denied SET id = 2 WHERE id = 1",
        ),
        (
            "delete denied table",
            "DELETE FROM public.auth_write_denied WHERE id = 1",
        ),
        (
            "alter denied table",
            "ALTER TABLE public.auth_write_denied ADD COLUMN note TEXT",
        ),
        (
            "indeterminate schema write",
            "CREATE SCHEMA auth_write_other_schema",
        ),
    ];
    for (label, sql) in denied_writes {
        let denied = writer
            .simple_query(sql)
            .await
            .expect_err(&format!("allowlist unexpectedly permitted {label}: {sql}"));
        assert_eq!(
            denied.code(),
            Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
            "{label} must be denied by QuackGIS authorization: {denied}"
        );
    }

    let copy_denied = writer
        .simple_query("COPY public.auth_write_denied FROM STDIN")
        .await
        .expect_err("allowlist unexpectedly entered COPY for denied target");
    assert_eq!(
        copy_denied.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
    let denied_after = quackgis_server::ducklake_sql::metrics_snapshot().writes_denied_total;
    assert!(
        denied_after > denied_before + denied_writes.len() as u64,
        "write allowlist denials should increment metrics: before={denied_before} after={denied_after} denied_sql={}",
        denied_writes.len() + 1
    );
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
async fn ducklake_transaction_rollback_discards_insert() {
    let (_server, client, _conn) = connect().await;

    client
        .batch_execute(
            "CREATE TABLE tx_rollback (id INT, label VARCHAR);\
             INSERT INTO tx_rollback VALUES (1, 'a');",
        )
        .await
        .expect("seed table");

    client
        .batch_execute(
            "BEGIN;\
             INSERT INTO tx_rollback VALUES (2, 'b');\
             ROLLBACK;",
        )
        .await
        .expect("rollback staged insert");

    let count: i64 = client
        .query_one("SELECT COUNT(*) FROM tx_rollback", &[])
        .await
        .expect("count after rollback")
        .get(0);
    assert_eq!(count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_transaction_commit_publishes_grouped_inserts() {
    let (_server, client, _conn) = connect().await;

    client
        .batch_execute(
            "CREATE TABLE tx_commit (id INT, label VARCHAR);\
             INSERT INTO tx_commit VALUES (1, 'a');\
             BEGIN;\
             INSERT INTO tx_commit VALUES (2, 'b');\
             INSERT INTO tx_commit VALUES (3, 'c');\
             COMMIT;",
        )
        .await
        .expect("commit staged inserts");

    let rows = client
        .query("SELECT id, label FROM tx_commit ORDER BY id", &[])
        .await
        .expect("select after commit");
    let values: Vec<(i32, String)> = rows
        .into_iter()
        .map(|r| (r.get::<_, i32>(0), r.get::<_, String>(1)))
        .collect();
    assert_eq!(
        values,
        vec![(1, "a".into()), (2, "b".into()), (3, "c".into())]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_mojibaked_utf8_insert_literal_is_repaired() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    client
        .batch_execute("CREATE TABLE public.ogr_mojibake (name TEXT)")
        .await
        .expect("create OGR mojibake table");

    let double_mojibaked_name = "Quai des Ã\u{83}Â\u{89}tats-Unis";
    let insert_sql = format!(
        "INSERT INTO public.ogr_mojibake (name) VALUES ('{}')",
        double_mojibaked_name.replace('\'', "''")
    );
    client
        .execute(&insert_sql, &[])
        .await
        .expect("OGR-shaped INSERT repairs mojibaked UTF-8 literal");

    let double_mojibaked_name = "La PÃ\u{83}Âªcherie U Luvassu";
    let insert_sql = format!(
        "INSERT INTO public.ogr_mojibake (name) VALUES ('{}')",
        double_mojibaked_name.replace('\'', "''")
    );
    client
        .execute(&insert_sql, &[])
        .await
        .expect("OGR-shaped INSERT repairs second mojibaked UTF-8 literal");

    let rows = client
        .query("SELECT name FROM public.ogr_mojibake ORDER BY name", &[])
        .await
        .expect("read repaired OGR names");
    let names: Vec<String> = rows.into_iter().map(|row| row.get(0)).collect();
    assert_eq!(
        names,
        vec![
            "La Pêcherie U Luvassu".to_string(),
            "Quai des États-Unis".to_string(),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_transaction_commit_rewrites_staged_table_once() {
    let (_server, client, _conn) = connect().await;

    client
        .batch_execute(
            "CREATE TABLE tx_rewrite (id INT, label VARCHAR);\
             INSERT INTO tx_rewrite VALUES (1, 'a'), (2, 'b'), (3, 'c');\
             BEGIN;\
             UPDATE tx_rewrite SET label = 'bb' WHERE id = 2;\
             DELETE FROM tx_rewrite WHERE id = 1;\
             INSERT INTO tx_rewrite VALUES (4, 'd');\
             COMMIT;",
        )
        .await
        .expect("commit staged rewrite");

    let rows = client
        .query("SELECT id, label FROM tx_rewrite ORDER BY id", &[])
        .await
        .expect("select after rewrite commit");
    let values: Vec<(i32, String)> = rows
        .into_iter()
        .map(|r| (r.get::<_, i32>(0), r.get::<_, String>(1)))
        .collect();
    assert_eq!(
        values,
        vec![(2, "bb".into()), (3, "c".into()), (4, "d".into())]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_transaction_stages_alter_add_column_for_ogr_append() {
    let (_server, client, _conn) = connect().await;

    client
        .batch_execute(
            "CREATE TABLE tx_alter (id INT, name VARCHAR);\
             BEGIN;\
             ALTER TABLE tx_alter ADD COLUMN category VARCHAR;\
             INSERT INTO tx_alter (id, name, category) VALUES (1, 'a', 'client');\
             COMMIT;",
        )
        .await
        .expect("commit staged alter + insert");

    let row = client
        .query_one("SELECT id, name, category FROM tx_alter", &[])
        .await
        .expect("select altered table");
    assert_eq!(row.get::<_, i32>(0), 1);
    assert_eq!(row.get::<_, String>(1), "a");
    assert_eq!(row.get::<_, String>(2), "client");
}

#[tokio::test(flavor = "multi_thread")]
async fn qgis_deallocate_statement_name_does_not_abort_staged_transaction() {
    let (_server, client, _conn) = connect().await;

    client
        .batch_execute(
            "CREATE TABLE tx_deallocate (id INT, label VARCHAR);\
             BEGIN;\
             INSERT INTO tx_deallocate VALUES (1, 'a');\
             DEALLOCATE addfeatures;\
             COMMIT;",
        )
        .await
        .expect("QGIS-style DEALLOCATE should not abort transaction");

    let label: String = client
        .query_one("SELECT label FROM tx_deallocate", &[])
        .await
        .expect("select after deallocate + commit")
        .get(0);
    assert_eq!(label, "a");
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_transaction_commit_detects_concurrent_replace_conflict() {
    let server = ServerHandle::start().await;
    let (client1, connection1) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect first client");
    let conn1 = tokio::spawn(async move {
        let _ = connection1.await;
    });
    let (client2, connection2) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect second client");
    let conn2 = tokio::spawn(async move {
        let _ = connection2.await;
    });

    client1
        .batch_execute(
            "CREATE TABLE tx_conflict (id INT, label VARCHAR);\
             INSERT INTO tx_conflict VALUES (1, 'a');",
        )
        .await
        .expect("seed conflict table");

    client1
        .batch_execute(
            "BEGIN;\
             UPDATE tx_conflict SET label = 'one' WHERE id = 1;",
        )
        .await
        .expect("stage first-client rewrite");
    client2
        .batch_execute("INSERT INTO tx_conflict VALUES (2, 'two');")
        .await
        .expect("concurrent autocommit insert");

    client1
        .batch_execute("COMMIT;")
        .await
        .expect_err("commit should detect stale base snapshot");

    let rows = client2
        .query("SELECT id, label FROM tx_conflict ORDER BY id", &[])
        .await
        .expect("select after conflict");
    let values: Vec<(i32, String)> = rows
        .into_iter()
        .map(|r| (r.get::<_, i32>(0), r.get::<_, String>(1)))
        .collect();
    assert_eq!(values, vec![(1, "a".into()), (2, "two".into())]);

    drop((conn1, conn2));
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

    let driver_ver: String = client
        .query_one("SELECT postgis_version()", &[])
        .await
        .expect("postgis_version")
        .get(0);
    assert!(driver_ver.starts_with("3.4."), "got {driver_ver}");

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
async fn privilege_metadata_surface_allows_common_text_helpers() {
    let (_server, client, _conn) = connect().await;

    let row = client
        .query_one(
            "SELECT \
               has_database_privilege('quackgis', 'CONNECT'), \
               has_schema_privilege('public', 'USAGE'), \
               has_table_privilege('public.points', 'SELECT'), \
               has_column_privilege('public.points', 'geom', 'UPDATE'), \
               pg_has_role(0::int4, 'USAGE')",
            &[],
        )
        .await
        .expect("PostgreSQL privilege metadata helpers");

    for idx in 0..5 {
        assert!(
            row.get::<_, bool>(idx),
            "privilege helper {idx} returned false"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_misc_surface_shims_return_expected_shapes() {
    let (_server, client, _conn) = connect().await;

    let instance_rows = client
        .simple_query("SELECT quackgis_instance_id() AS quackgis_instance_id")
        .await
        .expect("instance id surface");
    let instance_id = instance_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("quackgis_instance_id"),
        _ => None,
    });
    assert!(
        instance_id.is_some_and(|value| !value.is_empty()),
        "instance id should be present"
    );

    let geom_typename_rows = client
        .simple_query(
            "SELECT t.typname FROM pg_attribute a JOIN pg_type t ON a.atttypid = t.oid \
             WHERE a.attname = 'geom'",
        )
        .await
        .expect("geometry type-name surface");
    let typname = geom_typename_rows.iter().find_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => row.get("typname"),
        _ => None,
    });
    assert_eq!(
        typname, None,
        "a bare geom name must not synthesize geometry without a table field"
    );
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
async fn geometry_columns_discovers_asset_footprint_columns() {
    let (_server, client, _conn) = connect().await;
    client
        .simple_query(
            "CREATE TABLE public.asset_footprints (
                id INT,
                asset_uri TEXT,
                captured_minute INT,
                footprint BINARY
            )",
        )
        .await
        .expect("create asset footprint table");

    let rows = client
        .query(
            "SELECT f_table_schema, f_table_name, f_geometry_column, type
             FROM geometry_columns
             WHERE f_table_name = 'asset_footprints'",
            &[],
        )
        .await
        .expect("geometry_columns asset footprint query");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, String>(0), "public");
    assert_eq!(rows[0].get::<_, String>(1), "asset_footprints");
    assert_eq!(rows[0].get::<_, String>(2), "footprint");
    assert_eq!(rows[0].get::<_, String>(3), "GEOMETRY");
}

#[tokio::test(flavor = "multi_thread")]
async fn qgis_pg_type_lookup_resolves_custom_geometry_oid() {
    let (_server, client, _conn) = connect().await;

    let messages = client
        .simple_query(
            "SELECT oid,typname,typtype,typelem,typlen FROM pg_type WHERE oid in (20,23,90001,25)",
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
            .any(|(oid, typname, _, _, _)| oid == "20" && typname == "int8"),
        "lookup should preserve int8 row for synthetic rowid field discovery: {rows:?}"
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
async fn geoserver_pgjdbc_table_metadata_uses_jdbc_column_labels() {
    let (_server, client, _conn) = connect().await;

    client
        .simple_query("CREATE TABLE public.geoserver_meta (id INT, geom BINARY, name TEXT)")
        .await
        .expect("create GeoServer metadata table");

    let messages = client
        .simple_query(
            "SELECT current_database() AS \"TABLE_CAT\", n.nspname AS \"TABLE_SCHEM\", \
                    c.relname AS \"TABLE_NAME\", CASE WHEN n.nspname = 'pg_catalog' OR \
                    n.nspname = 'pg_toast' THEN CASE WHEN c.relkind = 'r' THEN 'SYSTEM TABLE' \
                    WHEN c.relkind = 'v' THEN 'SYSTEM VIEW' WHEN c.relkind = 'i' THEN 'SYSTEM INDEX' \
                    ELSE NULL END WHEN c.relkind = 'r' THEN 'TABLE' WHEN c.relkind = 'p' THEN 'PARTITIONED TABLE' \
                    WHEN c.relkind = 'v' THEN 'VIEW' WHEN c.relkind = 'i' THEN 'INDEX' \
                    WHEN c.relkind = 'f' THEN 'FOREIGN TABLE' WHEN c.relkind = 'm' THEN 'MATERIALIZED VIEW' \
                    ELSE NULL END AS \"TABLE_TYPE\", d.description AS \"REMARKS\", \
                    '' AS \"TYPE_CAT\", '' AS \"TYPE_SCHEM\", '' AS \"TYPE_NAME\", \
                    '' AS \"SELF_REFERENCING_COL_NAME\", '' AS \"REF_GENERATION\" \
             FROM pg_catalog.pg_namespace n, pg_catalog.pg_class c \
             LEFT JOIN pg_catalog.pg_description d ON (c.oid = d.objoid AND d.objsubid = 0) \
             WHERE c.relnamespace = n.oid AND c.relkind IN ('r','p','v','f','m') \
             ORDER BY \"TABLE_TYPE\", \"TABLE_SCHEM\", \"TABLE_NAME\"",
        )
        .await
        .expect("pgjdbc getTables shape");

    let rows: Vec<_> = messages
        .iter()
        .filter_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some((
                row.get("TABLE_CAT").unwrap_or_default().to_string(),
                row.get("TABLE_SCHEM").unwrap_or_default().to_string(),
                row.get("TABLE_NAME").unwrap_or_default().to_string(),
                row.get("TABLE_TYPE").unwrap_or_default().to_string(),
            )),
            _ => None,
        })
        .collect();

    assert!(
        rows.contains(&(
            "quackgis".to_string(),
            "public".to_string(),
            "geoserver_meta".to_string(),
            "TABLE".to_string()
        )),
        "GeoServer/pgjdbc getTables must expose non-null JDBC table labels: {rows:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn geoserver_pgjdbc_primary_key_metadata_handles_expandarray_query() {
    let (_server, client, _conn) = connect().await;

    client
        .simple_query("CREATE TABLE public.geoserver_pk (id INT, geom BINARY, name TEXT)")
        .await
        .expect("create GeoServer primary-key table");

    let rows = client
        .query(
            "SELECT result.TABLE_CAT AS \"TABLE_CAT\", result.TABLE_SCHEM AS \"TABLE_SCHEM\", \
                    result.TABLE_NAME AS \"TABLE_NAME\", result.COLUMN_NAME AS \"COLUMN_NAME\", \
                    result.KEY_SEQ AS \"KEY_SEQ\", result.PK_NAME AS \"PK_NAME\" \
             FROM (SELECT current_database() AS TABLE_CAT, n.nspname AS TABLE_SCHEM, \
                          ct.relname AS TABLE_NAME, a.attname AS COLUMN_NAME, \
                          (information_schema._pg_expandarray(i.indkey)).n AS KEY_SEQ, \
                          ci.relname AS PK_NAME, information_schema._pg_expandarray(i.indkey) AS KEYS, \
                          a.attnum AS A_ATTNUM, i.indnkeyatts AS KEY_COUNT \
                   FROM pg_catalog.pg_class ct \
                   JOIN pg_catalog.pg_attribute a ON (ct.oid = a.attrelid) \
                   JOIN pg_catalog.pg_namespace n ON (ct.relnamespace = n.oid) \
                   JOIN pg_catalog.pg_index i ON (a.attrelid = i.indrelid) \
                   JOIN pg_catalog.pg_class ci ON (ci.oid = i.indexrelid) \
                   WHERE true AND n.nspname = $1 AND ct.relname = $2 AND i.indisprimary) result \
             WHERE result.A_ATTNUM = (result.KEYS).x AND result.KEY_SEQ <= KEY_COUNT \
             ORDER BY result.table_name, result.pk_name, result.key_seq",
            &[&"public", &"geoserver_pk"],
        )
        .await
        .expect("pgjdbc getPrimaryKeys shape");

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    let table_cat: String = row.get("TABLE_CAT");
    let table_schema: String = row.get("TABLE_SCHEM");
    let table_name: String = row.get("TABLE_NAME");
    let column_name: String = row.get("COLUMN_NAME");
    let key_seq: i32 = row.get("KEY_SEQ");
    let pk_name: String = row.get("PK_NAME");

    assert_eq!(table_cat, "quackgis");
    assert_eq!(table_schema, "public");
    assert_eq!(table_name, "geoserver_pk");
    assert_eq!(column_name, "id");
    assert_eq!(key_seq, 1);
    assert_eq!(pk_name, "geoserver_pk_pkey");
}

#[tokio::test(flavor = "multi_thread")]
async fn geoserver_pgjdbc_generated_key_probe_handles_pg_get_serial_sequence() {
    let (_server, client, _conn) = connect().await;

    client
        .simple_query("CREATE TABLE public.geoserver_serial (id INT, geom BINARY, name TEXT)")
        .await
        .expect("create GeoServer serial metadata table");

    let row = client
        .query_one(
            "SELECT pg_get_serial_sequence('\"public\".\"geoserver_serial\"', 'id')",
            &[],
        )
        .await
        .expect("GeoServer/pgjdbc generated-key sequence probe");
    let sequence: Option<String> = row.get(0);
    assert_eq!(sequence, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn geoserver_pgjdbc_type_info_probe_handles_current_schemas_array_upper() {
    let (_server, client, _conn) = connect().await;

    let upper: i64 = client
        .query_one("SELECT array_upper(current_schemas(false), 1)", &[])
        .await
        .expect("array_upper(current_schemas(false), 1)")
        .get(0);
    assert_eq!(upper, 1);

    let _rows = client
        .query(
            "SELECT typinput='pg_catalog.array_in'::regproc as is_array, typtype, typname, pg_type.oid \
             FROM pg_catalog.pg_type \
             LEFT JOIN (select ns.oid as nspoid, ns.nspname, r.r \
                         from pg_namespace as ns \
                         join ( select s.r, (current_schemas(false))[s.r] as nspname \
                                  from generate_series(1, array_upper(current_schemas(false), 1)) as s(r) ) as r \
                        using ( nspname ) \
                      ) as sp \
                   ON sp.nspoid = typnamespace \
             WHERE pg_type.oid = 23 \
             ORDER BY sp.r, pg_type.oid DESC",
            &[],
        )
        .await
        .expect("pgjdbc TypeInfoCache getSQLType search-path probe");

    let rows = client
        .query(
            "SELECT typinput='pg_catalog.array_in'::regproc as is_array, typtype, typname, pg_type.oid \
             FROM pg_catalog.pg_type \
             LEFT JOIN (select ns.oid as nspoid, ns.nspname, r.r \
                         from pg_namespace as ns \
                         join ( select s.r, (current_schemas(false))[s.r] as nspname \
                                  from generate_series(1, array_upper(current_schemas(false), 1)) as s(r) ) as r \
                        using ( nspname ) \
                      ) as sp \
                   ON sp.nspoid = typnamespace \
             WHERE pg_type.oid = $1 \
             ORDER BY sp.r, pg_type.oid DESC",
            &[&90_001_i32],
        )
        .await
        .expect("pgjdbc TypeInfoCache getSQLType custom geometry oid probe");
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].get::<_, bool>("is_array"));
    assert_eq!(rows[0].get::<_, i8>("typtype") as u8, b'b');
    assert_eq!(rows[0].get::<_, String>("typname"), "geometry");
    assert_eq!(rows[0].get::<_, u32>("oid"), 90_001);

    let row = client
        .query_one(
            "SELECT n.nspname = ANY(current_schemas(true)), n.nspname, t.typname \
             FROM pg_catalog.pg_type t \
             JOIN pg_catalog.pg_namespace n ON t.typnamespace = n.oid \
             WHERE t.oid = $1",
            &[&90_001_u32],
        )
        .await
        .expect("pgjdbc TypeInfoCache getPGType custom geometry oid probe");
    assert!(row.get::<_, bool>(0));
    assert_eq!(row.get::<_, String>("nspname"), "public");
    assert_eq!(row.get::<_, String>("typname"), "geometry");
}

#[tokio::test(flavor = "multi_thread")]
async fn geoserver_pgjdbc_column_metadata_marks_geom_as_geometry_oid() {
    let (_server, client, _conn) = connect().await;

    client
        .simple_query("CREATE TABLE public.geoserver_columns (id INT, geom BINARY, name TEXT)")
        .await
        .expect("create GeoServer column metadata table");

    let rows = client
        .query(
            "SELECT * FROM (SELECT current_database() AS current_database, n.nspname,c.relname,\
                    a.attname,a.atttypid,a.attnotnull OR (t.typtype = 'd' AND t.typnotnull) AS attnotnull,\
                    a.atttypmod,a.attlen,t.typtypmod,row_number() OVER (PARTITION BY a.attrelid ORDER BY a.attnum) AS attnum,\
                    nullif(a.attidentity, '') AS attidentity,nullif(a.attgenerated, '') AS attgenerated,\
                    pg_catalog.pg_get_expr(def.adbin, def.adrelid) AS adsrc,dsc.description,t.typbasetype,t.typtype \
             FROM pg_catalog.pg_namespace n \
             JOIN pg_catalog.pg_class c ON (c.relnamespace = n.oid) \
             JOIN pg_catalog.pg_attribute a ON (a.attrelid=c.oid) \
             JOIN pg_catalog.pg_type t ON (a.atttypid = t.oid) \
             LEFT JOIN pg_catalog.pg_attrdef def ON (a.attrelid=def.adrelid AND a.attnum = def.adnum) \
             LEFT JOIN pg_catalog.pg_description dsc ON (c.oid=dsc.objoid AND a.attnum = dsc.objsubid) \
             WHERE c.relkind in ('r','p','v','f','m') AND a.attnum > 0 AND NOT a.attisdropped \
               AND n.nspname LIKE $1 AND c.relname LIKE $2) c \
             WHERE true AND attname LIKE $3 ORDER BY nspname,c.relname,attnum",
            &[&"public", &"geoserver_columns", &"%"],
        )
        .await
        .expect("pgjdbc getColumns shape");

    let columns: Vec<(String, u32)> = rows
        .iter()
        .map(|row| (row.get("attname"), row.get("atttypid")))
        .collect();

    assert_eq!(
        columns,
        vec![
            ("id".to_string(), 23),
            ("geom".to_string(), 90_001),
            ("name".to_string(), 25),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn geoserver_pgjdbc_column_metadata_accepts_escaped_name_patterns() {
    let (_server, client, _conn) = connect().await;

    client
        .simple_query(
            "CREATE TABLE public.geoserver_columns_escaped (id INT, geom BINARY, name TEXT)",
        )
        .await
        .expect("create GeoServer column metadata table with underscores");

    let rows = client
        .query(
            "SELECT * FROM (SELECT current_database() AS current_database, n.nspname,c.relname,\
                    a.attname,a.atttypid,a.attnotnull OR (t.typtype = 'd' AND t.typnotnull) AS attnotnull,\
                    a.atttypmod,a.attlen,t.typtypmod,row_number() OVER (PARTITION BY a.attrelid ORDER BY a.attnum) AS attnum,\
                    nullif(a.attidentity, '') AS attidentity,nullif(a.attgenerated, '') AS attgenerated,\
                    pg_catalog.pg_get_expr(def.adbin, def.adrelid) AS adsrc,dsc.description,t.typbasetype,t.typtype \
             FROM pg_catalog.pg_namespace n \
             JOIN pg_catalog.pg_class c ON (c.relnamespace = n.oid) \
             JOIN pg_catalog.pg_attribute a ON (a.attrelid=c.oid) \
             JOIN pg_catalog.pg_type t ON (a.atttypid = t.oid) \
             LEFT JOIN pg_catalog.pg_attrdef def ON (a.attrelid=def.adrelid AND a.attnum = def.adnum) \
             LEFT JOIN pg_catalog.pg_description dsc ON (c.oid=dsc.objoid AND a.attnum = dsc.objsubid) \
             WHERE c.relkind in ('r','p','v','f','m') AND a.attnum > 0 AND NOT a.attisdropped \
               AND n.nspname LIKE $1 AND c.relname LIKE $2) c \
             WHERE true AND attname LIKE $3 ORDER BY nspname,c.relname,attnum",
            &[&"public", &"geoserver\\_columns\\_escaped", &"id"],
        )
        .await
        .expect("pgjdbc getColumns shape with escaped exact-name pattern");

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    let table_name: String = row.get("relname");
    let column_name: String = row.get("attname");
    let type_oid: u32 = row.get("atttypid");

    assert_eq!(table_name, "geoserver_columns_escaped");
    assert_eq!(column_name, "id");
    assert_eq!(type_oid, 23);
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
async fn keyless_rowid_survives_edit_delete_compaction_and_next_insert() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    client
        .batch_execute(
            "CREATE TABLE public.keyless_compact_edit (name TEXT, geom BINARY);
             INSERT INTO public.keyless_compact_edit (name, geom) VALUES
               ('seed', X'010100000000000000000000000000000000000000'),
               ('edit-me', X'0101000000000000000000f03f000000000000f03f');
             UPDATE public.keyless_compact_edit
             SET name = 'updated'
             WHERE \"_quackgis_rowid\" = 2;
             DELETE FROM public.keyless_compact_edit
             WHERE \"_quackgis_rowid\" = 1;",
        )
        .await
        .expect("seed and edit keyless compact table");

    let before = client
        .query(
            "SELECT \"_quackgis_rowid\", name, ST_AsText(ST_GeomFromWKB(geom))
             FROM public.keyless_compact_edit
             ORDER BY \"_quackgis_rowid\"",
            &[],
        )
        .await
        .expect("select keyless rows before compact");
    let before: Vec<(i64, String, String)> = before
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    assert_eq!(
        before,
        vec![(2, "updated".to_string(), "POINT(1 1)".to_string())]
    );

    client
        .batch_execute("CALL quackgis_compact_table('public.keyless_compact_edit')")
        .await
        .expect("compact keyless table after edit/delete");

    let after = client
        .query(
            "SELECT \"_quackgis_rowid\", name, ST_AsText(ST_GeomFromWKB(geom))
             FROM public.keyless_compact_edit
             ORDER BY \"_quackgis_rowid\"",
            &[],
        )
        .await
        .expect("select keyless rows after compact");
    let after: Vec<(i64, String, String)> = after
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    assert_eq!(after, before);

    let inserted = client
        .query(
            "INSERT INTO public.keyless_compact_edit (name, geom) VALUES
               ('after-compact', X'010100000000000000000000400000000000000040')
             RETURNING \"_quackgis_rowid\", name",
            &[],
        )
        .await
        .expect("insert after keyless compaction");
    assert_eq!(inserted.len(), 1);
    assert_eq!(inserted[0].get::<_, i64>(0), 3);
    assert_eq!(inserted[0].get::<_, String>(1), "after-compact");
}

#[tokio::test(flavor = "multi_thread")]
async fn qgis_edit_save_parameterized_insert_generates_rowid() {
    let (_server, client, _conn) = connect().await;

    client
        .batch_execute("CREATE TABLE public.qgis_edit_params (name TEXT, geom BINARY)")
        .await
        .expect("create parameterized edit table");

    let rowid_param: Option<i64> = None;
    let statement = client
        .prepare_typed(
            "INSERT INTO public.qgis_edit_params (geom, \"_quackgis_rowid\", name)
             VALUES (ST_GeomFromWKB($1::bytea,0), $2, 'inserted')
             RETURNING \"_quackgis_rowid\", name, ST_AsText(ST_GeomFromWKB(geom))",
            &[Type::BYTEA, Type::INT8],
        )
        .await
        .expect("prepare QGIS-style parameterized insert");
    let rows = client
        .query(&statement, &[&point_wkb(1.0, 1.0), &rowid_param])
        .await
        .expect("parameterized QGIS-style insert returning rowid");

    assert_eq!(rows.len(), 1);
    let rowid: i64 = rows[0].get(0);
    let name: String = rows[0].get(1);
    let wkt: String = rows[0].get(2);
    assert_eq!(rowid, 1);
    assert_eq!(name, "inserted");
    assert_eq!(wkt, "POINT(1 1)");
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
async fn geoserver_wfs_st_asewkb_binary_passthrough_returns_wkb() {
    let (_server, client, _conn) = connect().await;

    client
        .simple_query("CREATE TABLE public.geoserver_wfs_read (id INT, geom BINARY, name TEXT)")
        .await
        .expect("create GeoServer WFS read table");
    client
        .simple_query(
            "INSERT INTO public.geoserver_wfs_read VALUES \
             (1, X'010100000000000000000000000000000000000000', 'origin')",
        )
        .await
        .expect("insert GeoServer WFS read row");

    let ewkb: Vec<u8> = client
        .query_one(
            "SELECT ST_AsEWKB(geom) AS geom FROM public.geoserver_wfs_read WHERE id = 1",
            &[],
        )
        .await
        .expect("GeoTools ST_AsEWKB projection")
        .get(0);

    assert_eq!(ewkb, point_wkb(0.0, 0.0));

    let as_binary = client
        .prepare("SELECT ST_AsBinary(geom) AS geom FROM public.geoserver_wfs_read WHERE id = 1")
        .await
        .expect("prepare GeoTools ST_AsBinary projection");
    let row = client
        .query_one(&as_binary, &[])
        .await
        .expect("GeoTools ST_AsBinary projection keeps bytea RowDescription");

    assert_eq!(*row.columns()[0].type_(), Type::BYTEA);
    assert_eq!(row.get::<_, Vec<u8>>(0), point_wkb(0.0, 0.0));
}

#[tokio::test(flavor = "multi_thread")]
async fn geoserver_wfs_bounds_st_envelope_accepts_st_extent_box_text() {
    let (_server, client, _conn) = connect().await;

    client
        .simple_query("CREATE TABLE public.geoserver_bounds (id INT, geom BINARY, name TEXT)")
        .await
        .expect("create GeoServer bounds table");
    client
        .simple_query(
            "INSERT INTO public.geoserver_bounds VALUES \
             (1, X'010100000000000000000000000000000000000000', 'origin'), \
             (2, X'010100000000000000000000400000000000000840', 'far')",
        )
        .await
        .expect("insert GeoServer bounds rows");

    let roundtrip_extent: String = client
        .query_one(
            "SELECT ST_Extent(bounds) \
             FROM (SELECT ST_Envelope(ST_Extent(ST_Force2D(geom))) AS bounds \
                   FROM public.geoserver_bounds) q",
            &[],
        )
        .await
        .expect("GeoTools ST_Envelope(ST_Extent(ST_Force2D)) bounds query")
        .get(0);

    assert_eq!(roundtrip_extent, "BOX(0 0,2 3)");

    let bounds_wkt: String = client
        .query_one(
            "SELECT ST_AsText(ST_Envelope(ST_Extent(ST_Force2D(geom)))) \
             FROM public.geoserver_bounds",
            &[],
        )
        .await
        .expect("GeoTools ST_AsText bounds query")
        .get(0);

    assert!(
        bounds_wkt.starts_with("POLYGON(("),
        "unexpected bounds WKT: {bounds_wkt}"
    );

    let has_arc: bool = client
        .query_one(
            "SELECT ST_HasArc(geom) FROM public.geoserver_bounds WHERE id = 1",
            &[],
        )
        .await
        .expect("GeoTools ST_HasArc reader guard")
        .get(0);

    assert!(!has_arc);

    let simplified: Vec<u8> = client
        .query_one(
            "SELECT ST_Simplify(geom, 0.0, true) FROM public.geoserver_bounds WHERE id = 1",
            &[],
        )
        .await
        .expect("GeoTools ST_Simplify reader projection")
        .get(0);

    assert_eq!(simplified, point_wkb(0.0, 0.0));

    let overlaps_literal_bbox: bool = client
        .query_one(
            "SELECT st_overlaps_bbox(geom, \
                    X'010100000000000000000000000000000000000000') \
             FROM public.geoserver_bounds WHERE id = 1",
            &[],
        )
        .await
        .expect("GeoTools bbox filter with BinaryView literal")
        .get(0);

    assert!(overlaps_literal_bbox);

    let wms_projection = client
        .prepare_typed(
            "SELECT ST_Simplify(ST_Force2D(geom), $1, true) AS geom \
             FROM public.geoserver_bounds WHERE id = 1",
            &[Type::FLOAT8],
        )
        .await
        .expect("prepare GeoTools WMS projection");
    let row = client
        .query_one(&wms_projection, &[&0.0_f64])
        .await
        .expect("GeoTools WMS parameterized binary geometry projection");

    assert_eq!(*row.columns()[0].type_(), Type::BYTEA);
    assert_eq!(row.get::<_, Vec<u8>>(0), point_wkb(0.0, 0.0));

    let wms_case_projection = client
        .prepare_typed(
            "SELECT ST_AsBinary(
                 CASE
                   WHEN ST_HasArc(ST_Force2D(geom))
                   THEN ST_CurveToLine(ST_Force2D(geom))
                   ELSE ST_Simplify(ST_Force2D(geom), $1, true)
                 END) AS geom
             FROM public.geoserver_bounds WHERE id = 1",
            &[Type::FLOAT8],
        )
        .await
        .expect("prepare GeoTools WMS CASE binary geometry projection");
    let row = client
        .query_one(&wms_case_projection, &[&0.0_f64])
        .await
        .expect("GeoTools WMS CASE projection keeps WKB as bytea");

    assert_eq!(*row.columns()[0].type_(), Type::BYTEA);
    assert_eq!(row.get::<_, Vec<u8>>(0), point_wkb(0.0, 0.0));
}

#[tokio::test(flavor = "multi_thread")]
async fn geoserver_wfst_trace_dml_shapes_roundtrip() {
    let (_server, client, _conn) = connect().await;

    client
        .batch_execute("CREATE TABLE public.geoserver_wfst (id INT, geom BINARY, name TEXT)")
        .await
        .expect("create GeoServer WFS-T table");

    let insert = client
        .prepare_typed(
            "INSERT INTO public.geoserver_wfst (id, geom, name)
             VALUES ($1, ST_GeomFromWKB($2::bytea, 4326), $3)
             RETURNING id, name, ST_AsText(ST_GeomFromWKB(geom))",
            &[Type::INT4, Type::BYTEA, Type::TEXT],
        )
        .await
        .expect("prepare GeoServer WFS-T INSERT trace shape");
    let insert_wkb = point_wkb(4.0, 5.0);
    let insert_name = "inserted";
    let inserted = client
        .query(&insert, &[&7_i32, &insert_wkb, &insert_name])
        .await
        .expect("GeoServer WFS-T INSERT trace shape");
    assert_eq!(inserted.len(), 1);
    assert_eq!(inserted[0].get::<_, i32>(0), 7);
    assert_eq!(inserted[0].get::<_, String>(1), "inserted");
    assert_eq!(inserted[0].get::<_, String>(2), "POINT(4 5)");

    let update = client
        .prepare_typed(
            "UPDATE public.geoserver_wfst
             SET geom = ST_GeomFromWKB($1::bytea, 4326), name = $2
             WHERE id = $3
             RETURNING id, name, ST_AsText(ST_GeomFromWKB(geom))",
            &[Type::BYTEA, Type::TEXT, Type::INT4],
        )
        .await
        .expect("prepare GeoServer WFS-T UPDATE trace shape");
    let update_wkb = point_wkb(6.0, 7.0);
    let update_name = "updated";
    let updated = client
        .query(&update, &[&update_wkb, &update_name, &7_i32])
        .await
        .expect("GeoServer WFS-T UPDATE trace shape");
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].get::<_, i32>(0), 7);
    assert_eq!(updated[0].get::<_, String>(1), "updated");
    assert_eq!(updated[0].get::<_, String>(2), "POINT(6 7)");

    let deleted = client
        .query(
            "DELETE FROM public.geoserver_wfst WHERE id = 7 RETURNING id, name",
            &[],
        )
        .await
        .expect("GeoServer WFS-T DELETE trace shape");
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].get::<_, i32>(0), 7);
    assert_eq!(deleted[0].get::<_, String>(1), "updated");
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
async fn ogr_postgis_type_probe_returns_oid_typname_shape() {
    let (_server, client, _conn) = connect().await;

    let messages = client
        .simple_query(
            "SELECT oid, typname FROM pg_type \
             WHERE typname IN ('geometry', 'geography') AND typtype='b'",
        )
        .await
        .expect("OGR PostGIS type probe should return oid/typname shape");

    let typnames: Vec<String> = messages
        .iter()
        .filter_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => row.get("typname").map(str::to_string),
            _ => None,
        })
        .collect();
    assert_eq!(
        typnames,
        vec!["geometry".to_string(), "geography".to_string()]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_normalized_bytea_type_probe_resolves_spatial_oids() {
    let (_server, client, _conn) = connect().await;

    let messages = client
        .simple_query(
            "SELECT oid, typname FROM pg_catalog.pg_type \
             WHERE typname IN ('bytea', 'bytea') AND typtype='b'",
        )
        .await
        .expect("OGR normalized bytea type probe should resolve spatial oids");

    let got: Vec<(u32, String)> = messages
        .iter()
        .filter_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some((
                row.get("oid")?.parse::<u32>().ok()?,
                row.get("typname")?.to_string(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (90_001, "geometry".to_string()),
            (90_002, "geography".to_string()),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_system_metadata_table_probe_is_not_synthesized() {
    let (_server, client, _conn) = connect().await;

    let rows = client
        .query(
            "SELECT c.oid FROM pg_class c \
             JOIN pg_namespace n ON c.relnamespace=n.oid \
             WHERE c.relname = 'metadata' AND n.nspname = 'ogr_system_tables'",
            &[],
        )
        .await
        .expect("OGR optional metadata table existence probe should not error");

    assert!(
        rows.is_empty(),
        "QuackGIS should not synthesize an OID for absent OGR metadata tables"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_system_metadata_privilege_probes_fail_closed() {
    let (_server, client, _conn) = connect().await;

    let row = client
        .query_one(
            "SELECT usesuper FROM pg_user WHERE usename = CURRENT_USER",
            &[],
        )
        .await
        .expect("OGR superuser probe should not fall through to catalog planning");
    assert!(!row.get::<_, bool>("usesuper"));

    let event_rows = client
        .query(
            "SELECT 1 FROM pg_event_trigger \
             WHERE evtname = 'ogr_system_tables_event_trigger_for_metadata'",
            &[],
        )
        .await
        .expect("OGR metadata event-trigger probe should not error");
    assert!(event_rows.is_empty());

    let row = client
        .query_one(
            "SELECT has_table_privilege('ogr_system_tables.metadata', 'SELECT')",
            &[],
        )
        .await
        .expect("OGR metadata privilege probe should not error");
    assert!(!row.get::<_, bool>("has_table_privilege"));
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_system_metadata_read_and_bootstrap_are_noops() {
    let (_server, client, _conn) = connect().await;

    let rows = client
        .query(
            "SELECT metadata FROM ogr_system_tables.metadata \
             WHERE schema_name = 'public' AND table_name = 'points'",
            &[],
        )
        .await
        .expect("OGR optional metadata read should return an empty result");
    assert!(rows.is_empty());

    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS ogr_system_tables.metadata( \
             id SERIAL, schema_name TEXT NOT NULL, table_name TEXT NOT NULL, \
             metadata TEXT, UNIQUE(schema_name, table_name))",
        )
        .await
        .expect("OGR optional metadata bootstrap should not trip SERIAL parsing");

    let deleted = client
        .execute(
            "DELETE FROM ogr_system_tables.metadata \
             WHERE schema_name = 'public' AND table_name = 'points'",
            &[],
        )
        .await
        .expect("OGR optional metadata delete should be a no-op");
    assert_eq!(deleted, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_table_description_probe_returns_description_shape() {
    let (_server, client, _conn) = connect().await;

    client
        .batch_execute("CREATE TABLE public.ogr_desc_visible (id INT)")
        .await
        .expect("create OGR description-visible table");

    let rows = client
        .query(
            "SELECT d.description FROM pg_class c \
             JOIN pg_namespace n ON c.relnamespace=n.oid \
             JOIN pg_description d \
               ON d.objoid = c.oid \
              AND d.classoid = 'pg_class'::regclass::oid \
              AND d.objsubid = 0 \
             WHERE c.relname = 'ogr_desc_visible' \
               AND n.nspname = 'public' \
               AND c.relkind in ('r', 'v')",
            &[],
        )
        .await
        .expect("OGR table-description probe should return description-shaped rows");

    assert!(rows.is_empty());
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
async fn ogr_column_listing_query_accepts_attgenerated_shape() {
    let (_server, client, _conn) = connect().await;

    let rows = client
        .query(
            "SELECT a.attname, t.typname, a.attlen, \
                    format_type(a.atttypid,a.atttypmod), a.attnotnull, \
                    def.def, i.indisunique, descr.description, a.attgenerated \
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
        .expect("OGR PostgreSQL-12+ column listing should include attgenerated shape");

    let cols: Vec<(String, String)> = rows
        .into_iter()
        .map(|row| (row.get("attname"), row.get("attgenerated")))
        .collect();
    assert_eq!(
        cols,
        vec![
            ("id".to_string(), "".to_string()),
            ("wkb_geometry".to_string(), "".to_string()),
            ("name".to_string(), "".to_string()),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_column_listing_query_is_schema_derived_for_appended_fields() {
    let (_server, client, _conn) = connect().await;

    client
        .batch_execute(
            "CREATE TABLE public.ogr_osm_fields \
             (id INT, wkb_geometry BINARY, name TEXT, osm_id TEXT)",
        )
        .await
        .expect("create OGR OSM-shaped table");

    let oid_messages = client
        .simple_query(
            "SELECT c.oid, c.relname FROM pg_catalog.pg_class c \
             WHERE c.relname ~ '^(ogr_osm_fields)$' \
             ORDER BY 2, 3",
        )
        .await
        .expect("OGR pg_class oid lookup");
    let (table_oid, relname) = oid_messages
        .iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some((
                row.get("oid")?.parse::<u32>().ok()?,
                row.get("relname")?.to_string(),
            )),
            _ => None,
        })
        .expect("synthetic OGR table oid row");
    assert_eq!(relname, "ogr_osm_fields");

    let rows = client
        .query(
            &format!(
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
                 WHERE a.attnum > 0 AND a.attrelid = {table_oid} \
                 ORDER BY a.attnum"
            ),
            &[],
        )
        .await
        .expect("OGR column listing should reflect table schema");

    let cols: Vec<(String, String)> = rows
        .into_iter()
        .map(|row| (row.get("attname"), row.get("typname")))
        .collect();
    assert_eq!(
        cols,
        vec![
            ("id".to_string(), "int4".to_string()),
            ("wkb_geometry".to_string(), "geometry".to_string()),
            ("name".to_string(), "text".to_string()),
            ("osm_id".to_string(), "text".to_string()),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ogr_column_listing_resolves_relname_equality_and_regclass_attrelid() {
    let (_server, client, _conn) = connect().await;

    client
        .batch_execute(
            "CREATE TABLE public.ogr_osm_fields_eq \
             (id INT, wkb_geometry BINARY, name TEXT, osm_id TEXT, category TEXT)",
        )
        .await
        .expect("create OGR equality lookup table");

    let oid_messages = client
        .simple_query(
            "SELECT c.oid, n.nspname, c.relname \
             FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relname = 'ogr_osm_fields_eq' AND n.nspname = 'public'",
        )
        .await
        .expect("OGR equality pg_class oid lookup");
    let (table_oid, nspname, relname) = oid_messages
        .iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some((
                row.get("oid")?.parse::<u32>().ok()?,
                row.get("nspname")?.to_string(),
                row.get("relname")?.to_string(),
            )),
            _ => None,
        })
        .expect("synthetic OGR equality table oid row");
    assert_eq!(nspname, "public");
    assert_eq!(relname, "ogr_osm_fields_eq");

    let rows = client
        .query(
            &format!(
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
                 WHERE a.attnum > 0 AND a.attrelid = {table_oid} \
                 ORDER BY a.attnum"
            ),
            &[],
        )
        .await
        .expect("OGR equality column listing should reflect table schema");
    let cols: Vec<String> = rows.into_iter().map(|row| row.get("attname")).collect();
    assert_eq!(
        cols,
        vec!["id", "wkb_geometry", "name", "osm_id", "category"]
    );

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
             WHERE a.attnum > 0 AND a.attrelid = '\"public\".\"ogr_osm_fields_eq\"'::regclass::oid \
             ORDER BY a.attnum",
            &[],
        )
        .await
        .expect("OGR regclass column listing should reflect table schema");
    let cols: Vec<String> = rows.into_iter().map(|row| row.get("attname")).collect();
    assert_eq!(
        cols,
        vec!["id", "wkb_geometry", "name", "osm_id", "category"]
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
            "CREATE TABLE quackgis.main.ogr_cursor \
             (id INT, wkb_geometry BINARY, name TEXT, category TEXT)",
        )
        .await
        .expect("create OGR cursor table");
    client
        .simple_query(
            "INSERT INTO quackgis.main.ogr_cursor VALUES \
             (1, X'010100000000000000000000000000000000000000', 'origin', 'client')",
        )
        .await
        .expect("insert OGR cursor row");

    client.query("BEGIN", &[]).await.expect("begin");
    client
        .query(
            "DECLARE OGRPGLayerReader0xabc CURSOR FOR \
             SELECT \"wkb_geometry\", \"id\", \"name\", \"category\" FROM \"ogr_cursor\"",
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
    let columns: Vec<&str> = rows[0]
        .columns()
        .iter()
        .map(|column| column.name())
        .collect();
    assert_eq!(columns, vec!["wkb_geometry", "id", "name", "category"]);
    assert_eq!(rows[0].get::<_, String>("category"), "client");
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
    client
        .execute(
            r#"INSERT INTO "ogr_write_load" ("wkb_geometry" , "name", "category") VALUES (E'\\001\\001\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000@\\000\\000\\000\\000\\000\\000\\000@', E'Quai des \\303\\211tats-Unis', 'client')"#,
            &[],
        )
        .await
        .expect("OGR extended INSERT preserves UTF-8 text octal escapes");

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
                "Quai des États-Unis".to_string(),
                "POINT(2 2)".to_string(),
                Some("client".to_string())
            ),
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
