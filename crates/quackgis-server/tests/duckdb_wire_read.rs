// SPDX-License-Identifier: Apache-2.0
#![cfg(feature = "duckdb-adbc")]

use std::collections::HashMap;
use std::sync::Arc;

use adbc_core::options::IngestMode;
use arrow_array::{Int32Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use bytes::Bytes;
use datafusion_postgres::ServerOptions;
use futures::{SinkExt, stream};
use quackgis_server::duckdb_adbc_storage::{DuckDbAdbcConfig, DuckDbAdbcStorage, ExtensionPolicy};
use serde::Deserialize;

struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Deserialize)]
struct SpatialLedger {
    cases: Vec<SpatialLedgerCase>,
}

#[derive(Deserialize)]
struct SpatialLedgerCase {
    name: String,
    disposition: String,
    duckdb_sql: Option<String>,
    expected: Option<String>,
}

fn executable_spatial_cases() -> Vec<(String, String, String)> {
    let case_pattern = regex::Regex::new(
        r#"(?s)Case\s*\{\s*name:\s*"(?P<name>[^"]+)",\s*sql:\s*"(?P<sql>[^"]+)",\s*expected:\s*"(?P<expected>[^"]+)""#,
    )
    .expect("spatial case regex");
    let regress = case_pattern
        .captures_iter(include_str!("postgis_regress.rs"))
        .map(|captures| {
            (
                captures["name"].to_owned(),
                (captures["sql"].to_owned(), captures["expected"].to_owned()),
            )
        })
        .collect::<HashMap<_, _>>();
    let ledger: SpatialLedger =
        serde_json::from_str(include_str!("../../../tests/duckdb_spatial_compat.json"))
            .expect("DuckDB spatial compatibility ledger");
    ledger
        .cases
        .into_iter()
        .filter(|case| {
            matches!(
                case.disposition.as_str(),
                "native_duckdb" | "sql_rewrite" | "quackgis_macro"
            )
        })
        .map(|case| {
            let (source_sql, source_expected) = regress
                .get(&case.name)
                .unwrap_or_else(|| panic!("missing maintained spatial case {}", case.name));
            (
                case.name,
                case.duckdb_sql.unwrap_or_else(|| source_sql.clone()),
                case.expected.unwrap_or_else(|| source_expected.clone()),
            )
        })
        .collect()
}

fn normalize_spatial_scalar(value: &str) -> String {
    let trimmed = value.trim();
    let upper = trimmed.to_ascii_uppercase();
    if [
        "POINT",
        "LINESTRING",
        "POLYGON",
        "MULTI",
        "GEOMETRYCOLLECTION",
        "SRID=",
    ]
    .iter()
    .any(|prefix| upper.starts_with(prefix))
    {
        return upper
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
    }
    if matches!(upper.as_str(), "TRUE" | "FALSE") {
        return upper.to_ascii_lowercase();
    }
    trimmed.to_owned()
}

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn cli_duckdb_backend_serves_an_official_local_catalog() {
    let driver_path =
        std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER").expect("set QUACKGIS_DUCKDB_ADBC_DRIVER");
    let temp = tempfile::tempdir().expect("tempdir");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("ephemeral listener");
    let port = listener.local_addr().expect("address").port();
    drop(listener);

    let mut server = ChildGuard(
        std::process::Command::new(env!("CARGO_BIN_EXE_quackgis-server"))
            .arg("--engine-backend=duckdb")
            .arg("--duckdb-driver")
            .arg(&driver_path)
            .arg("--catalog-path")
            .arg(temp.path().join("catalog.ducklake"))
            .arg("--data-path")
            .arg(temp.path().join("data"))
            .arg("--host=127.0.0.1")
            .arg(format!("--port={port}"))
            .arg("--auth-mode=password")
            .arg("--readwrite-user=writer")
            .arg("--readwrite-password=writer-secret")
            .arg("--readonly-user=reader")
            .arg("--readonly-password=reader-secret")
            .arg("--write-allowlist=cli_points,private_points")
            .arg("--read-allowlist=cli_points")
            .spawn()
            .expect("start feature-gated DuckDB CLI backend"),
    );

    let mut connected = None;
    for _ in 0..200 {
        let mut config = tokio_postgres::Config::new();
        config
            .host("127.0.0.1")
            .port(port)
            .user("writer")
            .password("writer-secret")
            .dbname("quackgis");
        match config.connect(tokio_postgres::NoTls).await {
            Ok((client, connection)) => {
                tokio::spawn(connection);
                connected = Some(client);
                break;
            }
            Err(error) if server.0.try_wait().expect("server status").is_none() => {
                let _ = error;
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            Err(error) => panic!("DuckDB CLI backend exited before accepting connections: {error}"),
        }
    }
    let client = connected.unwrap_or_else(|| {
        let _ = server.0.kill();
        let _ = server.0.wait();
        panic!("DuckDB CLI backend did not accept connections before the timeout")
    });

    client
        .batch_execute(
            "CREATE TABLE quackgis.main.cli_points(id INTEGER, name VARCHAR); \
             INSERT INTO quackgis.main.cli_points VALUES (1, 'one')",
        )
        .await
        .expect_err("bounded backend rejects multiple statements");
    client
        .batch_execute("CREATE TABLE quackgis.main.cli_points(id INTEGER, name VARCHAR)")
        .await
        .expect("CLI CREATE");
    client
        .batch_execute("INSERT INTO quackgis.main.cli_points VALUES (1, 'one')")
        .await
        .expect("CLI INSERT");
    client
        .batch_execute("CREATE TABLE quackgis.main.private_points(id INTEGER)")
        .await
        .expect("writer can create second allowlisted table");
    let statement = client
        .prepare_typed(
            "SELECT name FROM quackgis.main.cli_points WHERE id = $1::INTEGER",
            &[tokio_postgres::types::Type::INT4],
        )
        .await
        .expect("CLI typed prepare");
    let row = client
        .query_one(&statement, &[&1_i32])
        .await
        .expect("CLI parameterized SELECT");
    assert_eq!(row.get::<_, String>(0), "one");

    let mut reader_config = tokio_postgres::Config::new();
    reader_config
        .host("127.0.0.1")
        .port(port)
        .user("reader")
        .password("reader-secret")
        .dbname("quackgis");
    let (reader, reader_connection) = reader_config
        .connect(tokio_postgres::NoTls)
        .await
        .expect("SCRAM reader connect");
    let reader_task = tokio::spawn(reader_connection);
    assert_eq!(
        reader
            .query_one("SELECT count(*)::BIGINT FROM quackgis.main.cli_points", &[])
            .await
            .expect("allowlisted reader query")
            .get::<_, i64>(0),
        1
    );
    let denied_write = reader
        .batch_execute("INSERT INTO quackgis.main.cli_points VALUES (2, 'denied')")
        .await
        .expect_err("read-only user cannot write through DuckDB");
    assert_eq!(
        denied_write.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
    let denied_copy = match reader
        .copy_in::<_, Bytes>("COPY quackgis.main.cli_points (id, name) FROM STDIN")
        .await
    {
        Ok(_) => panic!("read-only user started DuckDB COPY"),
        Err(error) => error,
    };
    assert_eq!(
        denied_copy.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
    let denied_read = reader
        .query("SELECT id FROM quackgis.main.private_points", &[])
        .await
        .expect_err("read allowlist applies before DuckDB planning");
    assert_eq!(
        denied_read.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
    let denied_metadata = reader
        .query("SELECT * FROM ducklake_snapshots('quackgis')", &[])
        .await
        .expect_err("restricted reader cannot inspect unfiltered DuckLake metadata");
    assert_eq!(
        denied_metadata.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );

    drop(client);
    drop(reader);
    reader_task.abort();
    server.0.kill().expect("stop CLI backend");
    server.0.wait().expect("reap CLI backend");
}

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn pgwire_reads_writes_and_isolates_duckdb_sessions() {
    let driver_path =
        std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER").expect("set QUACKGIS_DUCKDB_ADBC_DRIVER");
    let temp = tempfile::tempdir().expect("tempdir");
    let data_path = temp.path().join("data");
    std::fs::create_dir(&data_path).expect("data path");
    let config = DuckDbAdbcConfig {
        driver_path: driver_path.into(),
        database_uri: ":memory:".to_owned(),
        ducklake_uri: format!(
            "ducklake:{}",
            temp.path().join("catalog.ducklake").display()
        ),
        catalog_name: "quackgis".to_owned(),
        data_path: data_path.display().to_string(),
        extension_policy: ExtensionPolicy::LoadOnly,
    };
    let storage = Arc::new(DuckDbAdbcStorage::open(config.clone()).expect("DuckDB storage"));
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, false),
    ]));
    storage
        .ingest(
            "main",
            "wire_points",
            vec![
                RecordBatch::try_new(
                    schema,
                    vec![
                        Arc::new(Int32Array::from(vec![1, 2, 3])),
                        Arc::new(StringArray::from(vec!["one", "two", "three"])),
                    ],
                )
                .expect("batch"),
            ],
            IngestMode::Create,
        )
        .expect("seed official DuckLake");

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("ephemeral listener");
    let port = listener.local_addr().expect("address").port();
    drop(listener);
    let options = ServerOptions::new()
        .with_host("127.0.0.1".to_owned())
        .with_port(port);
    let server_storage = Arc::clone(&storage);
    let task = tokio::spawn(async move {
        let _ = quackgis_server::pgwire_server::serve_duckdb(
            server_storage,
            &options,
            quackgis_server::auth::AuthConfig::trust(),
        )
        .await;
    });
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let (mut client, connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
        tokio_postgres::NoTls,
    )
    .await
    .expect("pgwire connect");
    let connection_task = tokio::spawn(connection);

    let simple = client
        .simple_query("SELECT count(*) AS rows FROM quackgis.main.wire_points")
        .await
        .expect("simple DuckDB SELECT");
    assert!(simple.iter().any(|message| matches!(
        message,
        tokio_postgres::SimpleQueryMessage::Row(row) if row.get(0) == Some("3")
    )));

    let spatial_cases = executable_spatial_cases();
    assert_eq!(spatial_cases.len(), 40, "executable spatial ledger count");
    for (name, sql, expected) in spatial_cases {
        let row = client
            .query_one(&sql, &[])
            .await
            .unwrap_or_else(|error| panic!("DuckDB pgwire spatial case {name} failed: {error}"));
        let actual = row
            .try_get::<_, String>(0)
            .unwrap_or_else(|error| panic!("DuckDB pgwire spatial case {name} result: {error}"));
        assert_eq!(
            normalize_spatial_scalar(&actual),
            normalize_spatial_scalar(&expected),
            "DuckDB pgwire spatial case {name}"
        );
    }

    let statement = client
        .prepare_typed(
            "SELECT name FROM quackgis.main.wire_points WHERE id = $1::INTEGER",
            &[tokio_postgres::types::Type::INT4],
        )
        .await
        .expect("extended Parse/Describe");
    assert_eq!(statement.params(), &[tokio_postgres::types::Type::INT4]);
    assert_eq!(statement.columns()[0].name(), "name");
    let rows = client
        .query(&statement, &[&2_i32])
        .await
        .expect("bound query");
    assert_eq!(rows[0].get::<_, String>(0), "two");

    let empty = client
        .prepare("SELECT id, name FROM quackgis.main.wire_points WHERE false")
        .await
        .expect("empty-result Describe");
    assert_eq!(empty.columns().len(), 2);
    assert!(
        client
            .query(&empty, &[])
            .await
            .expect("empty result")
            .is_empty()
    );

    let scalar_statement = client
        .prepare(
            "SELECT true::BOOLEAN AS enabled, 7::BIGINT AS big_id, \
             1.5::DOUBLE AS ratio, 12.34::DECIMAL(10,2) AS amount, \
             DATE '2026-07-11' AS observed_on, \
             TIMESTAMP '2026-07-11 12:34:56' AS observed_at, \
             NULL::INTEGER AS optional_id",
        )
        .await
        .expect("describe DuckDB scalar result types");
    let scalar_types = scalar_statement
        .columns()
        .iter()
        .map(|column| column.type_().clone())
        .collect::<Vec<_>>();
    assert_eq!(
        scalar_types,
        vec![
            tokio_postgres::types::Type::BOOL,
            tokio_postgres::types::Type::INT8,
            tokio_postgres::types::Type::FLOAT8,
            tokio_postgres::types::Type::NUMERIC,
            tokio_postgres::types::Type::DATE,
            tokio_postgres::types::Type::TIMESTAMP,
            tokio_postgres::types::Type::INT4,
        ]
    );
    let scalar_row = client
        .query_one(&scalar_statement, &[])
        .await
        .expect("encode DuckDB scalar result row");
    assert!(scalar_row.get::<_, bool>(0));
    assert_eq!(scalar_row.get::<_, i64>(1), 7);
    assert_eq!(scalar_row.get::<_, f64>(2), 1.5);
    assert_eq!(scalar_row.get::<_, Option<i32>>(6), None);

    let paging_transaction = client.transaction().await.expect("paging transaction");
    let paging_statement = paging_transaction
        .prepare("SELECT id FROM quackgis.main.wire_points ORDER BY id")
        .await
        .expect("prepare paged DuckDB query");
    let portal = paging_transaction
        .bind(&paging_statement, &[])
        .await
        .expect("bind paged DuckDB portal");
    let first_page = paging_transaction
        .query_portal(&portal, 1)
        .await
        .expect("first portal page");
    let second_page = paging_transaction
        .query_portal(&portal, 1)
        .await
        .expect("second portal page");
    let final_page = paging_transaction
        .query_portal(&portal, 1)
        .await
        .expect("final portal page");
    assert_eq!(first_page[0].get::<_, i32>(0), 1);
    assert_eq!(second_page[0].get::<_, i32>(0), 2);
    assert_eq!(final_page[0].get::<_, i32>(0), 3);
    paging_transaction
        .commit()
        .await
        .expect("commit paging transaction");

    client
        .batch_execute("CREATE TABLE quackgis.main.wire_mutations(id INTEGER, name VARCHAR)")
        .await
        .expect("pgwire CREATE TABLE through DuckDB");
    client
        .batch_execute("INSERT INTO quackgis.main.wire_mutations VALUES (1, 'one'), (2, 'two')")
        .await
        .expect("pgwire INSERT through DuckDB");
    client
        .batch_execute("UPDATE quackgis.main.wire_mutations SET name = 'uno' WHERE id = 1")
        .await
        .expect("pgwire UPDATE through DuckDB");
    client
        .batch_execute("DELETE FROM quackgis.main.wire_mutations WHERE id = 2")
        .await
        .expect("pgwire DELETE through DuckDB");

    let insert = client
        .prepare_typed(
            "INSERT INTO quackgis.main.wire_mutations VALUES ($1::INTEGER, $2::VARCHAR)",
            &[
                tokio_postgres::types::Type::INT4,
                tokio_postgres::types::Type::TEXT,
            ],
        )
        .await
        .expect("prepare parameterized INSERT");
    assert_eq!(
        client
            .execute(&insert, &[&3_i32, &"three"])
            .await
            .expect("execute parameterized INSERT"),
        1
    );
    let update = client
        .prepare_typed(
            "UPDATE quackgis.main.wire_mutations SET name = $1::VARCHAR WHERE id = $2::INTEGER",
            &[
                tokio_postgres::types::Type::TEXT,
                tokio_postgres::types::Type::INT4,
            ],
        )
        .await
        .expect("prepare parameterized UPDATE");
    assert_eq!(
        client
            .execute(&update, &[&"tres", &3_i32])
            .await
            .expect("execute parameterized UPDATE"),
        1
    );
    let delete = client
        .prepare_typed(
            "DELETE FROM quackgis.main.wire_mutations WHERE id = $1::INTEGER",
            &[tokio_postgres::types::Type::INT4],
        )
        .await
        .expect("prepare parameterized DELETE");
    assert_eq!(
        client
            .execute(&delete, &[&3_i32])
            .await
            .expect("execute parameterized DELETE"),
        1
    );
    let mutated = client
        .query_one(
            "SELECT count(*)::BIGINT, min(name) FROM quackgis.main.wire_mutations",
            &[],
        )
        .await
        .expect("query pgwire mutations");
    assert_eq!(mutated.get::<_, i64>(0), 1);
    assert_eq!(mutated.get::<_, String>(1), "uno");

    client
        .batch_execute(
            "CREATE TABLE quackgis.main.wire_copy(\
             id INTEGER, name VARCHAR, geom_wkb BLOB)",
        )
        .await
        .expect("create COPY target");
    let mut copy_rows = stream::iter(
        [
            Bytes::from_static(b"1\torigin\t\\x010100000000000000000000000000000000000000\n"),
            Bytes::from_static(b"2\tone\t\\x0101000000000000000000F03F000000000000F03F\n"),
        ]
        .into_iter()
        .map(Ok::<_, tokio_postgres::Error>),
    );
    let copy_sink = client
        .copy_in("COPY quackgis.main.wire_copy (id, name, geom_wkb) FROM STDIN")
        .await
        .expect("start bounded COPY");
    let mut copy_sink = std::pin::pin!(copy_sink);
    copy_sink
        .send_all(&mut copy_rows)
        .await
        .expect("stream COPY rows");
    assert_eq!(copy_sink.finish().await.expect("finish COPY"), 2);
    let copied = client
        .query_one(
            "SELECT count(*)::BIGINT, \
             count(*) FILTER (WHERE ST_Intersects(\
             ST_GeomFromWKB(geom_wkb), ST_MakeEnvelope(-1, -1, 2, 2)))::BIGINT \
             FROM quackgis.main.wire_copy",
            &[],
        )
        .await
        .expect("query COPY WKB rows");
    assert_eq!(copied.get::<_, i64>(0), 2);
    assert_eq!(copied.get::<_, i64>(1), 2);
    let spatial_statement = client
        .prepare_typed(
            "SELECT count(*)::BIGINT FROM quackgis.main.wire_copy \
             WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromWKB($1::BLOB))",
            &[tokio_postgres::types::Type::BYTEA],
        )
        .await
        .expect("prepare binary WKB spatial query");
    let point_one = [
        1_u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f,
    ];
    let spatial = client
        .query_one(&spatial_statement, &[&&point_one[..]])
        .await
        .expect("execute binary WKB spatial query");
    assert_eq!(spatial.get::<_, i64>(0), 1);

    client
        .batch_execute(
            "CREATE TABLE quackgis.main.wire_copy_scalars(\
             small_id SMALLINT, enabled BOOLEAN, ratio REAL, observed_on DATE, \
             observed_at TIMESTAMP, amount DECIMAL(10,2))",
        )
        .await
        .expect("create scalar COPY target");
    let scalar_copy = client
        .copy_in::<_, Bytes>(
            "COPY quackgis.main.wire_copy_scalars \
             (small_id, enabled, ratio, observed_on, observed_at, amount) FROM STDIN",
        )
        .await
        .expect("start scalar COPY");
    let mut scalar_copy = std::pin::pin!(scalar_copy);
    scalar_copy
        .send(Bytes::from_static(
            b"7\tt\t1.25\t2026-07-11\t2026-07-11 12:34:56.123456\t12.34\n",
        ))
        .await
        .expect("send scalar COPY row");
    assert_eq!(scalar_copy.finish().await.expect("finish scalar COPY"), 1);
    let scalar_copy_row = client
        .query_one(
            "SELECT small_id, enabled, ratio, CAST(observed_on AS VARCHAR), \
             CAST(observed_at AS VARCHAR), CAST(amount AS VARCHAR) \
             FROM quackgis.main.wire_copy_scalars",
            &[],
        )
        .await
        .expect("query scalar COPY row");
    assert_eq!(scalar_copy_row.get::<_, i16>(0), 7);
    assert!(scalar_copy_row.get::<_, bool>(1));
    assert_eq!(scalar_copy_row.get::<_, f32>(2), 1.25);
    assert_eq!(scalar_copy_row.get::<_, String>(3), "2026-07-11");
    assert_eq!(
        scalar_copy_row.get::<_, String>(4),
        "2026-07-11 12:34:56.123456"
    );
    assert_eq!(scalar_copy_row.get::<_, String>(5), "12.34");

    client
        .batch_execute("BEGIN")
        .await
        .expect("begin COPY rollback transaction");
    let rollback_copy = client
        .copy_in("COPY quackgis.main.wire_copy (id, name, geom_wkb) FROM STDIN")
        .await
        .expect("start transactional COPY");
    let mut rollback_copy = std::pin::pin!(rollback_copy);
    rollback_copy
        .send(Bytes::from_static(
            b"3\trollback\t\\x010100000000000000000008400000000000000840\n",
        ))
        .await
        .expect("send transactional COPY row");
    assert_eq!(
        rollback_copy
            .finish()
            .await
            .expect("finish transactional COPY"),
        1
    );
    client
        .batch_execute("ROLLBACK")
        .await
        .expect("rollback COPY transaction");
    let after_copy_rollback = client
        .query_one("SELECT count(*)::BIGINT FROM quackgis.main.wire_copy", &[])
        .await
        .expect("COPY rollback count");
    assert_eq!(after_copy_rollback.get::<_, i64>(0), 2);

    let (observer, observer_connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
        tokio_postgres::NoTls,
    )
    .await
    .expect("observer pgwire connect");
    let observer_task = tokio::spawn(observer_connection);

    client
        .batch_execute("BEGIN")
        .await
        .expect("begin transaction");
    client
        .batch_execute(
            "UPDATE quackgis.main.wire_mutations SET name = 'must_rollback' WHERE id = 1",
        )
        .await
        .expect("transactional update");
    let isolated = observer
        .query_one(
            "SELECT name FROM quackgis.main.wire_mutations WHERE id = 1",
            &[],
        )
        .await
        .expect("observer isolation query");
    assert_eq!(isolated.get::<_, String>(0), "uno");
    client
        .batch_execute("ROLLBACK")
        .await
        .expect("rollback transaction");

    client
        .batch_execute("BEGIN")
        .await
        .expect("second transaction");
    client
        .batch_execute("UPDATE quackgis.main.wire_mutations SET name = 'committed' WHERE id = 1")
        .await
        .expect("committed update");
    let before_commit = observer
        .query_one(
            "SELECT name FROM quackgis.main.wire_mutations WHERE id = 1",
            &[],
        )
        .await
        .expect("observer before commit");
    assert_eq!(before_commit.get::<_, String>(0), "uno");
    client
        .batch_execute("COMMIT")
        .await
        .expect("commit transaction");
    let after_commit = observer
        .query_one(
            "SELECT name FROM quackgis.main.wire_mutations WHERE id = 1",
            &[],
        )
        .await
        .expect("observer after commit");
    assert_eq!(after_commit.get::<_, String>(0), "committed");

    let unsupported = client
        .batch_execute("TRUNCATE quackgis.main.wire_mutations")
        .await
        .expect_err("unsupported writes must fail closed");
    assert_eq!(
        unsupported.code(),
        Some(&tokio_postgres::error::SqlState::FEATURE_NOT_SUPPORTED)
    );

    let (abandoned, abandoned_connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
        tokio_postgres::NoTls,
    )
    .await
    .expect("abandoned pgwire connect");
    let abandoned_task = tokio::spawn(abandoned_connection);
    abandoned
        .batch_execute("BEGIN")
        .await
        .expect("abandoned begin");
    abandoned
        .batch_execute("INSERT INTO quackgis.main.wire_mutations VALUES (2, 'abandoned')")
        .await
        .expect("abandoned insert");
    drop(abandoned);
    tokio::time::timeout(std::time::Duration::from_secs(2), abandoned_task)
        .await
        .expect("abandoned connection closes")
        .expect("abandoned connection task")
        .expect("abandoned connection protocol");
    let after_disconnect = observer
        .query_one(
            "SELECT count(*)::BIGINT FROM quackgis.main.wire_mutations",
            &[],
        )
        .await
        .expect("observer after abandoned transaction");
    assert_eq!(after_disconnect.get::<_, i64>(0), 1);
    let snapshots = observer
        .query_one(
            "SELECT count(*)::BIGINT FROM ducklake_snapshots('quackgis')",
            &[],
        )
        .await
        .expect("official snapshot inspection through pgwire");
    assert!(snapshots.get::<_, i64>(0) > 0);

    drop(client);
    drop(observer);
    task.abort();
    connection_task.abort();
    observer_task.abort();
    let _ = task.await;
    let _ = connection_task.await;
    let _ = observer_task.await;
    drop(storage);

    let reopened = DuckDbAdbcStorage::open(config).expect("reopen pgwire-authored catalog");
    let rows = reopened
        .query("SELECT count(*) FROM quackgis.main.wire_mutations")
        .expect("query pgwire mutations after restart");
    let count = rows[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count Int64")
        .value(0);
    assert_eq!(count, 1);
    let name = reopened
        .query("SELECT name FROM quackgis.main.wire_mutations WHERE id = 1")
        .expect("committed value after restart");
    assert_eq!(
        name[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("name Utf8")
            .value(0),
        "committed"
    );
    let copied_after_restart = reopened
        .query(
            "SELECT count(*), min(hex(geom_wkb)) \
             FROM quackgis.main.wire_copy",
        )
        .expect("COPY WKB after restart");
    assert_eq!(
        copied_after_restart[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("COPY count Int64")
            .value(0),
        2
    );
    assert_eq!(
        copied_after_restart[0]
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("COPY hex Utf8")
            .value(0),
        "010100000000000000000000000000000000000000"
    );
}
