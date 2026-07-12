// SPDX-License-Identifier: Apache-2.0
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adbc_core::options::IngestMode;
use arrow_array::{Array, Int32Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use bytes::Bytes;
use futures::{SinkExt, StreamExt, stream};
use quackgis_server::duckdb_adbc_storage::{DuckDbAdbcConfig, DuckDbAdbcStorage, ExtensionPolicy};
use quackgis_server::pgwire_server::ServerOptions;
use serde::Deserialize;
use serde_json::json;

mod support;
use support::evidence::{EvidenceEnvelope, EvidenceLevel, EvidenceProfile, ExecutionEnvironment};

#[derive(Debug, Eq, PartialEq)]
struct GeometryBytes(Vec<u8>);

impl<'a> tokio_postgres::types::FromSql<'a> for GeometryBytes {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(raw.to_vec()))
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        ty.oid() == 90_001
    }
}

struct ChildGuard(std::process::Child);

fn first_i64(batch: &RecordBatch, column: usize) -> i64 {
    batch
        .column(column)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("Int64 result")
        .value(0)
}

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut bytes = vec![1, 1, 0, 0, 0];
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes
}

fn flushed_copy_rows(count: usize) -> Bytes {
    let mut rows = String::new();
    for id in 0..count {
        rows.push_str(&format!("{id}\trow-{id}\n"));
    }
    Bytes::from(rows)
}

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
    expected: Option<String>,
    unsupported: Option<UnsupportedSpatialExpectation>,
}

#[derive(Deserialize)]
struct UnsupportedSpatialExpectation {
    sqlstate: String,
    message: String,
}

fn executable_spatial_cases() -> Vec<(String, String, String)> {
    let case_pattern = regex::Regex::new(
        r#"(?s)Case\s*\{\s*name:\s*"(?P<name>[^"]+)",\s*sql:\s*"(?P<sql>[^"]+)",\s*expected:\s*"(?P<expected>[^"]+)""#,
    )
    .expect("spatial case regex");
    let regress = case_pattern
        .captures_iter(include_str!(
            "../../../tests/fixtures/postgis_curated_cases.rs"
        ))
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
                source_sql.clone(),
                case.expected.unwrap_or_else(|| source_expected.clone()),
            )
        })
        .collect()
}

fn unsupported_spatial_cases() -> Vec<(String, String, UnsupportedSpatialExpectation)> {
    let case_pattern = regex::Regex::new(
        r#"(?s)Case\s*\{\s*name:\s*"(?P<name>[^"]+)",\s*sql:\s*"(?P<sql>[^"]+)""#,
    )
    .expect("spatial case regex");
    let regress = case_pattern
        .captures_iter(include_str!(
            "../../../tests/fixtures/postgis_curated_cases.rs"
        ))
        .map(|captures| (captures["name"].to_owned(), captures["sql"].to_owned()))
        .collect::<HashMap<_, _>>();
    let ledger: SpatialLedger =
        serde_json::from_str(include_str!("../../../tests/duckdb_spatial_compat.json"))
            .expect("DuckDB spatial compatibility ledger");
    ledger
        .cases
        .into_iter()
        .filter_map(|case| {
            case.unsupported.map(|expectation| {
                let sql = regress
                    .get(&case.name)
                    .unwrap_or_else(|| panic!("missing maintained spatial case {}", case.name));
                (case.name, sql.clone(), expectation)
            })
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

async fn prove_native_admission_limit(port: u16) {
    const CLIENTS: usize = 32;
    const LIMIT: usize = 8;
    let barrier = Arc::new(tokio::sync::Barrier::new(CLIENTS + 1));
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::channel(CLIENTS);
    let mut releases = Vec::with_capacity(CLIENTS);
    let mut tasks = Vec::with_capacity(CLIENTS);

    for id in 0..CLIENTS {
        let barrier = Arc::clone(&barrier);
        let entered_tx = entered_tx.clone();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        releases.push(Some(release_tx));
        tasks.push(tokio::spawn(async move {
            let (mut client, connection) = tokio_postgres::connect(
                &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
                tokio_postgres::NoTls,
            )
            .await
            .expect("native admission client connect");
            let connection_task = tokio::spawn(connection);
            let transaction = client
                .transaction()
                .await
                .expect("native admission transaction");
            let statement = transaction
                .prepare("SELECT i::BIGINT FROM range(100000) AS rows(i)")
                .await
                .expect("native admission statement");
            let portal = transaction
                .bind(&statement, &[])
                .await
                .expect("native admission portal");
            barrier.wait().await;
            let rows = transaction
                .query_portal(&portal, 1)
                .await
                .expect("native admission first row");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].get::<_, i64>(0), 0);
            entered_tx.send(id).await.expect("report admitted client");
            release_rx.await.expect("release admitted client");
            drop(portal);
            drop(transaction);
            connection_task.abort();
        }));
    }
    drop(entered_tx);
    barrier.wait().await;

    for wave in 0..(CLIENTS / LIMIT) {
        let mut admitted = Vec::with_capacity(LIMIT);
        for _ in 0..LIMIT {
            admitted.push(
                tokio::time::timeout(Duration::from_secs(5), entered_rx.recv())
                    .await
                    .unwrap_or_else(|_| panic!("native admission wave {wave} stalled"))
                    .expect("admitted client channel"),
            );
        }
        if wave == 0 {
            assert!(
                tokio::time::timeout(Duration::from_millis(200), entered_rx.recv())
                    .await
                    .is_err(),
                "a ninth native reader entered while eight portals retained permits"
            );
        }
        for id in admitted {
            releases[id]
                .take()
                .expect("one release per client")
                .send(())
                .expect("release native admission client");
        }
    }
    for task in tasks {
        task.await.expect("native admission task");
    }
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
            .arg("--shutdown-timeout-ms=500")
            .spawn()
            .expect("start DuckDB CLI backend"),
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
                connected = Some((client, tokio::spawn(connection)));
                break;
            }
            Err(error) if server.0.try_wait().expect("server status").is_none() => {
                let _ = error;
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            Err(error) => panic!("DuckDB CLI backend exited before accepting connections: {error}"),
        }
    }
    let (client, writer_task) = connected.unwrap_or_else(|| {
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
    let denied_maintenance = client
        .batch_execute(
            "CALL quackgis_merge_adjacent_files('main', 'cli_points', 8, 16777216, NULL)",
        )
        .await
        .expect_err("maintenance is disabled without an explicit identity");
    assert_eq!(
        denied_maintenance.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
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

    drop(statement);
    drop(reader);
    reader_task.abort();
    client
        .batch_execute("BEGIN")
        .await
        .expect("begin transaction before graceful shutdown");
    client
        .batch_execute("INSERT INTO quackgis.main.cli_points VALUES (3, 'drained')")
        .await
        .expect("write before graceful shutdown");
    let signal_result = unsafe { libc::kill(server.0.id() as i32, libc::SIGTERM) };
    assert_eq!(signal_result, 0, "send SIGTERM to CLI backend");
    tokio::time::sleep(Duration::from_millis(100)).await;
    client
        .batch_execute("COMMIT")
        .await
        .expect("active transaction commits during drain");
    let rejected_begin = client
        .batch_execute("BEGIN")
        .await
        .expect_err("draining server rejects a new transaction");
    assert!(
        rejected_begin
            .as_db_error()
            .is_some_and(|error| error.message().contains("draining"))
    );
    drop(client);
    writer_task.abort();
    let status = server
        .0
        .wait()
        .expect("reap gracefully stopped CLI backend");
    assert!(
        status.success(),
        "CLI backend did not stop cleanly: {status}"
    );
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
    storage
        .execute_update(
            "CREATE TABLE quackgis.main.stream_rows AS \
             SELECT i::INTEGER AS id FROM range(100000) AS rows(i)",
        )
        .expect("seed streaming rows");

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("ephemeral listener");
    let port = listener.local_addr().expect("address").port();
    let options = ServerOptions::new()
        .with_host("127.0.0.1".to_owned())
        .with_port(port)
        .with_statement_timeout(Duration::from_secs(2))
        .with_copy_batch_rows(64)
        .with_copy_batch_bytes(65_536)
        .with_copy_max_row_bytes(16_384);
    let server_storage = Arc::clone(&storage);
    let task = tokio::spawn(async move {
        let _ = quackgis_server::pgwire_server::serve_duckdb_on_listener(
            server_storage,
            listener,
            &options,
            quackgis_server::auth::AuthConfig::trust()
                .with_maintenance_user("postgres")
                .expect("maintenance identity"),
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

    prove_native_admission_limit(port).await;

    let simple = client
        .simple_query("SELECT count(*) AS rows FROM quackgis.main.wire_points")
        .await
        .expect("simple DuckDB SELECT");
    assert!(simple.iter().any(|message| matches!(
        message,
        tokio_postgres::SimpleQueryMessage::Row(row) if row.get(0) == Some("3")
    )));
    client
        .batch_execute("SET standard_conforming_strings = ON")
        .await
        .expect("standard strings setting");
    client
        .batch_execute("SET client_min_messages = error")
        .await
        .expect("client message setting");
    let search_path = client
        .simple_query("SHOW search_path")
        .await
        .expect("SHOW search_path");
    assert!(search_path.iter().any(|message| matches!(
        message,
        tokio_postgres::SimpleQueryMessage::Row(row) if row.get(0) == Some("public")
    )));
    let public_count = client
        .query_one("SELECT count(*)::BIGINT FROM public.wire_points", &[])
        .await
        .expect("public schema mapping");
    assert_eq!(public_count.get::<_, i64>(0), 3);
    client
        .batch_execute("CREATE TABLE quackgis.main.quoted_copy(id INTEGER, name VARCHAR)")
        .await
        .expect("create quoted COPY target");
    let quoted_copy = client
        .copy_in("COPY \"public\".\"quoted_copy\" (\"id\", \"name\") FROM STDIN")
        .await
        .expect("start quoted two-part COPY");
    let mut quoted_copy = std::pin::pin!(quoted_copy);
    quoted_copy
        .send(Bytes::from_static(b"1\tquoted\n"))
        .await
        .expect("send quoted COPY row");
    assert_eq!(quoted_copy.finish().await.expect("finish quoted COPY"), 1);
    let quoted_count = client
        .query_one("SELECT count(*)::BIGINT FROM public.quoted_copy", &[])
        .await
        .expect("query quoted COPY through public schema");
    assert_eq!(quoted_count.get::<_, i64>(0), 1);
    client
        .batch_execute(
            "CREATE TABLE quackgis.main.layout_copy(\
             id INTEGER, geom_wkb BLOB, _qg_minx DOUBLE, _qg_miny DOUBLE, \
             _qg_maxx DOUBLE, _qg_maxy DOUBLE)",
        )
        .await
        .expect("create maintained bbox target");
    let layout_copy = client
        .copy_in("COPY public.layout_copy (id, geom_wkb) FROM STDIN")
        .await
        .expect("start maintained bbox COPY");
    let mut layout_copy = std::pin::pin!(layout_copy);
    layout_copy
        .send(Bytes::from_static(
            b"1\t\\x0101000000000000000000F03F0000000000000040\n2\t\\N\n",
        ))
        .await
        .expect("send maintained bbox rows");
    assert_eq!(layout_copy.finish().await.expect("finish bbox COPY"), 2);
    let bbox = client
        .query_one(
            "SELECT _qg_minx, _qg_miny, _qg_maxx, _qg_maxy \
             FROM public.layout_copy WHERE id = 1",
            &[],
        )
        .await
        .expect("query maintained bbox");
    assert_eq!(bbox.get::<_, f64>(0), 1.0);
    assert_eq!(bbox.get::<_, f64>(1), 2.0);
    assert_eq!(bbox.get::<_, f64>(2), 1.0);
    assert_eq!(bbox.get::<_, f64>(3), 2.0);
    let exact_bbox = client
        .query_one(
            "SELECT count(*)::BIGINT FROM public.layout_copy \
             WHERE _qg_minx <= 1.5 AND _qg_maxx >= 0.5 \
               AND _qg_miny <= 2.5 AND _qg_maxy >= 1.5 \
               AND ST_Intersects(ST_GeomFromWKB(geom_wkb), \
                   ST_MakeEnvelope(0.5, 1.5, 1.5, 2.5))",
            &[],
        )
        .await
        .expect("bbox candidate plus exact recheck");
    assert_eq!(exact_bbox.get::<_, i64>(0), 1);
    let supplied_bbox = client
        .copy_in::<str, Bytes>(
            "COPY public.layout_copy \
             (id, geom_wkb, _qg_minx, _qg_miny, _qg_maxx, _qg_maxy) FROM STDIN",
        )
        .await
        .expect("open rejected bbox COPY protocol stream");
    let mut supplied_bbox = std::pin::pin!(supplied_bbox);
    let supplied_bbox = match supplied_bbox
        .send(Bytes::from_static(
            b"3\t\\x010100000000000000000008400000000000001040\t3\t4\t3\t4\n",
        ))
        .await
    {
        Err(error) => error,
        Ok(()) => match supplied_bbox.finish().await {
            Ok(_) => panic!("caller-supplied bbox columns must fail closed"),
            Err(error) => error,
        },
    };
    assert_eq!(
        supplied_bbox.code(),
        Some(&tokio_postgres::error::SqlState::FEATURE_NOT_SUPPORTED)
    );
    let layout_count = client
        .query_one("SELECT count(*)::BIGINT FROM public.layout_copy", &[])
        .await
        .expect("layout session remains reusable after rejected COPY");
    assert_eq!(layout_count.get::<_, i64>(0), 2);
    for sql in [
        "INSERT INTO public.layout_copy (id, geom_wkb) VALUES (3, NULL)",
        "UPDATE public.layout_copy SET _qg_minx = 0 WHERE id = 1",
        "UPDATE public.layout_copy SET geom_wkb = ST_AsWKB(ST_Point(3, 4)) WHERE id = 1",
    ] {
        let error = client
            .execute(sql, &[])
            .await
            .expect_err("direct maintained bbox mutation must fail closed");
        assert_eq!(
            error.code(),
            Some(&tokio_postgres::error::SqlState::FEATURE_NOT_SUPPORTED),
            "{sql}"
        );
    }
    let safe_layout_update = client
        .prepare_typed(
            "UPDATE public.layout_copy SET id = $1::INTEGER WHERE id = $2::INTEGER",
            &[
                tokio_postgres::types::Type::INT4,
                tokio_postgres::types::Type::INT4,
            ],
        )
        .await
        .expect("prepare non-spatial maintained-layout update");
    assert_eq!(
        client
            .execute(&safe_layout_update, &[&10_i32, &1_i32])
            .await
            .expect("non-spatial maintained-layout update"),
        1
    );
    let unchanged_layout = client
        .query_one(
            "SELECT hex(geom_wkb), _qg_minx, _qg_miny, _qg_maxx, _qg_maxy \
             FROM public.layout_copy WHERE id = 10",
            &[],
        )
        .await
        .expect("non-spatial update preserves maintained layout");
    assert_eq!(
        unchanged_layout.get::<_, String>(0),
        "0101000000000000000000F03F0000000000000040"
    );
    for column in 1..=4 {
        assert_eq!(
            unchanged_layout.get::<_, f64>(column),
            if column == 2 || column == 4 { 2.0 } else { 1.0 }
        );
    }
    let geometry_layout_update = client
        .prepare_typed(
            "UPDATE public.layout_copy SET geom_wkb = $1::BLOB WHERE id = $2::INTEGER",
            &[
                tokio_postgres::types::Type::BYTEA,
                tokio_postgres::types::Type::INT4,
            ],
        )
        .await
        .expect("prepare maintained geometry update");
    let point_7_8 = point_wkb(7.0, 8.0);
    assert_eq!(
        client
            .execute(&geometry_layout_update, &[&&point_7_8[..], &10_i32])
            .await
            .expect("atomically update geometry and bbox"),
        1
    );
    let refreshed_layout = client
        .query_one(
            "SELECT hex(geom_wkb), _qg_minx, _qg_miny, _qg_maxx, _qg_maxy \
             FROM public.layout_copy WHERE id = 10",
            &[],
        )
        .await
        .expect("query refreshed maintained layout");
    assert_eq!(
        refreshed_layout.get::<_, String>(0),
        "01010000000000000000001C400000000000002040"
    );
    for (column, expected) in [7.0, 8.0, 7.0, 8.0].into_iter().enumerate() {
        assert_eq!(refreshed_layout.get::<_, f64>(column + 1), expected);
    }
    let malformed_geometry = [1_u8, 2, 3];
    client
        .execute(
            &geometry_layout_update,
            &[&&malformed_geometry[..], &10_i32],
        )
        .await
        .expect_err("malformed geometry must abort the atomic update");
    let after_malformed = client
        .query_one(
            "SELECT hex(geom_wkb), _qg_minx, _qg_miny FROM public.layout_copy WHERE id = 10",
            &[],
        )
        .await
        .expect("session and row remain usable after malformed geometry");
    assert_eq!(
        after_malformed.get::<_, String>(0),
        "01010000000000000000001C400000000000002040"
    );
    assert_eq!(after_malformed.get::<_, f64>(1), 7.0);
    assert_eq!(after_malformed.get::<_, f64>(2), 8.0);
    client
        .batch_execute("BEGIN")
        .await
        .expect("begin bbox rollback");
    let point_9_10 = point_wkb(9.0, 10.0);
    client
        .execute(&geometry_layout_update, &[&&point_9_10[..], &10_i32])
        .await
        .expect("update maintained geometry in transaction");
    client
        .batch_execute("ROLLBACK")
        .await
        .expect("rollback maintained geometry update");
    let after_layout_rollback = client
        .query_one(
            "SELECT _qg_minx, _qg_miny FROM public.layout_copy WHERE id = 10",
            &[],
        )
        .await
        .expect("query layout after rollback");
    assert_eq!(after_layout_rollback.get::<_, f64>(0), 7.0);
    assert_eq!(after_layout_rollback.get::<_, f64>(1), 8.0);
    client
        .execute(
            "UPDATE public.layout_copy SET geom_wkb = NULL WHERE id = 2",
            &[],
        )
        .await
        .expect("NULL geometry writes NULL bounds");
    let null_layout = client
        .query_one(
            "SELECT geom_wkb IS NULL, _qg_minx IS NULL, _qg_miny IS NULL, \
             _qg_maxx IS NULL, _qg_maxy IS NULL FROM public.layout_copy WHERE id = 2",
            &[],
        )
        .await
        .expect("query NULL maintained layout");
    for column in 0..5 {
        assert!(null_layout.get::<_, bool>(column));
    }
    let layout_count = client
        .query_one("SELECT count(*)::BIGINT FROM public.layout_copy", &[])
        .await
        .expect("layout session remains reusable after rejected mutations");
    assert_eq!(layout_count.get::<_, i64>(0), 2);

    let stream_started = Instant::now();
    let row_stream = client
        .query_raw(
            "SELECT id FROM quackgis.main.stream_rows ORDER BY id",
            std::iter::empty::<&i32>(),
        )
        .await
        .expect("open pgwire row stream");
    futures::pin_mut!(row_stream);
    let first = tokio::time::timeout(Duration::from_secs(2), row_stream.next())
        .await
        .expect("first row deadline")
        .expect("first row")
        .expect("first row result");
    let first_row_elapsed = stream_started.elapsed();
    assert_eq!(first.get::<_, i32>(0), 0);
    let mut streamed_rows = 1_usize;
    while let Some(row) = row_stream.next().await {
        let row = row.expect("streamed row");
        assert_eq!(row.get::<_, i32>(0), streamed_rows as i32);
        streamed_rows += 1;
    }
    assert_eq!(streamed_rows, 100_000);
    assert!(
        first_row_elapsed < stream_started.elapsed(),
        "first row must arrive before the complete result"
    );

    let spatial_cases = executable_spatial_cases();
    assert_eq!(spatial_cases.len(), 42, "executable spatial ledger count");
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
    let unsupported_spatial = unsupported_spatial_cases();
    assert_eq!(unsupported_spatial.len(), 15, "unsupported spatial cases");
    for (name, sql, expected) in unsupported_spatial {
        for error in [
            client
                .query_one(&sql, &[])
                .await
                .expect_err("extended unsupported spatial query"),
            client
                .simple_query(&sql)
                .await
                .expect_err("simple unsupported spatial query"),
        ] {
            let database = error
                .as_db_error()
                .unwrap_or_else(|| panic!("DuckDB pgwire spatial case {name}: {error}"));
            assert_eq!(database.code().code(), expected.sqlstate, "{name}");
            assert_eq!(database.message(), expected.message, "{name}");
        }
        assert_eq!(
            client
                .query_one("SELECT 1::INTEGER", &[])
                .await
                .expect("session remains reusable")
                .get::<_, i32>(0),
            1
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
    assert!(
        paging_transaction
            .query_portal(&portal, 1)
            .await
            .expect("observe paged DuckDB EOF")
            .is_empty()
    );
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
    let geometry_statement = client
        .prepare("SELECT geom_wkb FROM quackgis.main.wire_copy ORDER BY id")
        .await
        .expect("describe maintained geometry column");
    assert_eq!(geometry_statement.columns().len(), 1);
    assert_eq!(geometry_statement.columns()[0].type_().oid(), 90_001);
    assert_eq!(geometry_statement.columns()[0].type_().name(), "geometry");
    let geometry_rows = client
        .query(&geometry_statement, &[])
        .await
        .expect("query binary geometry values");
    assert_eq!(geometry_rows[0].get::<_, GeometryBytes>(0).0.len(), 21);
    assert_eq!(
        geometry_rows[1].get::<_, GeometryBytes>(0).0,
        vec![
            1_u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f,
        ]
    );
    let null_geometry = client
        .query_one("SELECT NULL::BLOB AS geom_wkb", &[])
        .await
        .expect("query NULL geometry value");
    assert_eq!(null_geometry.get::<_, Option<GeometryBytes>>(0), None);
    let geometry_text = client
        .simple_query("SELECT geom_wkb FROM quackgis.main.wire_copy WHERE id = 1")
        .await
        .expect("query text geometry value")
        .into_iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
            _ => None,
        })
        .expect("text geometry row");
    assert_eq!(
        geometry_text,
        "\\x010100000000000000000000000000000000000000"
    );
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
            b"7\tt\t1.25\t2026-07-11\t2026-07-11 12:34:56.123456\t12.34\n\\N\t\\N\t\\N\t\\N\t\\N\t\\N\n",
        ))
        .await
        .expect("send scalar COPY row");
    assert_eq!(scalar_copy.finish().await.expect("finish scalar COPY"), 2);
    let scalar_copy_row = client
        .query_one(
            "SELECT small_id, enabled, ratio, CAST(observed_on AS VARCHAR), \
             CAST(observed_at AS VARCHAR), CAST(amount AS VARCHAR) \
             FROM quackgis.main.wire_copy_scalars WHERE small_id = 7",
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

    const LARGE_COPY_ROWS: usize = 220_000;
    client
        .batch_execute("CREATE TABLE quackgis.main.large_copy(id INTEGER, name VARCHAR)")
        .await
        .expect("create large COPY target");
    let large_copy = client
        .copy_in("COPY public.large_copy (id, name) FROM STDIN")
        .await
        .expect("start large COPY");
    let mut large_copy = std::pin::pin!(large_copy);
    let payload = "x".repeat(96);
    let mut chunk = Vec::with_capacity(60 * 1024);
    let mut wire_bytes = 0_usize;
    for id in 0..LARGE_COPY_ROWS {
        let row = format!("{id}\t{payload}\n");
        if chunk.len() + row.len() > 60 * 1024 {
            wire_bytes += chunk.len();
            large_copy
                .send(Bytes::from(std::mem::take(&mut chunk)))
                .await
                .expect("send bounded large COPY chunk");
            chunk = Vec::with_capacity(60 * 1024);
        }
        chunk.extend_from_slice(row.as_bytes());
    }
    if !chunk.is_empty() {
        wire_bytes += chunk.len();
        large_copy
            .send(Bytes::from(chunk))
            .await
            .expect("send final large COPY chunk");
    }
    assert!(wire_bytes > 20 * 1024 * 1024);
    assert_eq!(
        large_copy.finish().await.expect("finish large COPY"),
        LARGE_COPY_ROWS as u64
    );
    let large_count = client
        .query_one("SELECT count(*)::BIGINT FROM public.large_copy", &[])
        .await
        .expect("large COPY count");
    assert_eq!(large_count.get::<_, i64>(0), LARGE_COPY_ROWS as i64);

    client
        .batch_execute("CREATE TABLE quackgis.main.fragmented_copy(id INTEGER, name VARCHAR)")
        .await
        .expect("create fragmented COPY target");
    for id in 0..8 {
        let fragmented_copy = client
            .copy_in("COPY quackgis.main.fragmented_copy (id, name) FROM STDIN")
            .await
            .expect("start fragmented COPY");
        let mut fragmented_copy = std::pin::pin!(fragmented_copy);
        fragmented_copy
            .send(Bytes::from(format!("{id}\tfragment-{id}\n")))
            .await
            .expect("send fragmented COPY row");
        assert_eq!(
            fragmented_copy
                .finish()
                .await
                .expect("finish fragmented COPY"),
            1
        );
    }
    let fragment_files_before = client
        .query_one(
            "SELECT count(*)::BIGINT FROM \
             ducklake_list_files('quackgis', 'fragmented_copy', schema => 'main')",
            &[],
        )
        .await
        .expect("fragmented files before pgwire maintenance")
        .get::<_, i64>(0);
    assert!(fragment_files_before >= 8);
    client
        .batch_execute(
            "CALL quackgis_merge_adjacent_files('public', 'fragmented_copy', 8, 16777216, NULL)",
        )
        .await
        .expect("run bounded pgwire maintenance");
    let fragment_files_after = client
        .query_one(
            "SELECT count(*)::BIGINT FROM \
             ducklake_list_files('quackgis', 'fragmented_copy', schema => 'main')",
            &[],
        )
        .await
        .expect("fragmented files after pgwire maintenance")
        .get::<_, i64>(0);
    assert!(fragment_files_after * 2 <= fragment_files_before);
    client
        .batch_execute("BEGIN")
        .await
        .expect("begin maintenance rejection transaction");
    let transactional_maintenance = client
        .batch_execute(
            "CALL quackgis_merge_adjacent_files('main', 'fragmented_copy', 8, 16777216, NULL)",
        )
        .await
        .expect_err("maintenance inside a transaction is rejected");
    assert_eq!(
        transactional_maintenance.code(),
        Some(&tokio_postgres::error::SqlState::ACTIVE_SQL_TRANSACTION)
    );
    client
        .batch_execute("ROLLBACK")
        .await
        .expect("rollback after maintenance rejection");

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
        .batch_execute("CREATE TABLE quackgis.main.failed_copy(id INTEGER, name VARCHAR)")
        .await
        .expect("create atomic COPY failure target");
    let mut failed_copy = Box::pin(
        client
            .copy_in("COPY quackgis.main.failed_copy (id, name) FROM STDIN")
            .await
            .expect("start malformed COPY"),
    );
    failed_copy
        .send(flushed_copy_rows(64))
        .await
        .expect("send flushed COPY batch");
    let malformed_send = failed_copy.send(Bytes::from_static(b"3\n")).await;
    let malformed_failed = if malformed_send.is_err() {
        true
    } else {
        failed_copy.as_mut().finish().await.is_err()
    };
    assert!(malformed_failed, "malformed COPY must fail");
    let failed_rows = observer
        .query_one(
            "SELECT count(*)::BIGINT FROM quackgis.main.failed_copy",
            &[],
        )
        .await
        .expect("observe malformed COPY rollback");
    assert_eq!(failed_rows.get::<_, i64>(0), 0);

    client
        .batch_execute("CREATE TABLE quackgis.main.final_row_copy(id INTEGER, name VARCHAR)")
        .await
        .expect("create incomplete final-row COPY target");
    let mut final_row_copy = Box::pin(
        client
            .copy_in("COPY quackgis.main.final_row_copy (id, name) FROM STDIN")
            .await
            .expect("start incomplete final-row COPY"),
    );
    final_row_copy
        .send(flushed_copy_rows(64))
        .await
        .expect("send complete batches before incomplete final row");
    final_row_copy
        .send(Bytes::from_static(b"999"))
        .await
        .expect("send incomplete final row");
    let final_row_error = final_row_copy
        .as_mut()
        .finish()
        .await
        .expect_err("incomplete final COPY row must fail");
    assert_eq!(
        final_row_error.code(),
        Some(&tokio_postgres::error::SqlState::BAD_COPY_FILE_FORMAT)
    );
    let final_row_rows = observer
        .query_one(
            "SELECT count(*)::BIGINT FROM quackgis.main.final_row_copy",
            &[],
        )
        .await
        .expect("incomplete final-row cleanup is synchronous");
    assert_eq!(final_row_rows.get::<_, i64>(0), 0);

    client
        .batch_execute("CREATE TABLE quackgis.main.oversized_chunk_copy(id INTEGER, name VARCHAR)")
        .await
        .expect("create oversized chunk COPY target");
    let mut oversized_chunk_copy = Box::pin(
        client
            .copy_in("COPY quackgis.main.oversized_chunk_copy (id, name) FROM STDIN")
            .await
            .expect("start oversized chunk COPY"),
    );
    let oversized_chunk = Bytes::from(vec![b'\n'; 65_537]);
    let oversized_send = oversized_chunk_copy.send(oversized_chunk).await;
    let oversized_error = match oversized_send {
        Err(error) => error,
        Ok(()) => oversized_chunk_copy
            .as_mut()
            .finish()
            .await
            .expect_err("oversized COPY chunk must fail"),
    };
    assert_eq!(
        oversized_error.code(),
        Some(&tokio_postgres::error::SqlState::PROGRAM_LIMIT_EXCEEDED)
    );
    let oversized_rows = observer
        .query_one(
            "SELECT count(*)::BIGINT FROM quackgis.main.oversized_chunk_copy",
            &[],
        )
        .await
        .expect("oversized COPY chunk cleanup is synchronous");
    assert_eq!(oversized_rows.get::<_, i64>(0), 0);

    observer
        .batch_execute("CREATE TABLE quackgis.main.disconnected_copy(id INTEGER, name VARCHAR)")
        .await
        .expect("create disconnected COPY target");
    let (abandoned, abandoned_connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
        tokio_postgres::NoTls,
    )
    .await
    .expect("abandoned COPY client connect");
    let abandoned_task = tokio::spawn(abandoned_connection);
    let mut abandoned_copy = Box::pin(
        abandoned
            .copy_in("COPY quackgis.main.disconnected_copy (id, name) FROM STDIN")
            .await
            .expect("start abandoned COPY"),
    );
    abandoned_copy
        .send(flushed_copy_rows(64))
        .await
        .expect("send abandoned flushed batch");
    drop(abandoned_copy);
    drop(abandoned);
    abandoned_task.abort();
    let _ = abandoned_task.await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let disconnected_rows = observer
        .query_one(
            "SELECT count(*)::BIGINT FROM quackgis.main.disconnected_copy",
            &[],
        )
        .await
        .expect("observe disconnected COPY rollback");
    assert_eq!(disconnected_rows.get::<_, i64>(0), 0);

    observer
        .batch_execute("CREATE TABLE quackgis.main.cancelled_copy(id INTEGER, name VARCHAR)")
        .await
        .expect("create cancelled COPY target");
    let (cancelled, cancelled_connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
        tokio_postgres::NoTls,
    )
    .await
    .expect("cancelled COPY client connect");
    let cancel_token = cancelled.cancel_token();
    let cancelled_task = tokio::spawn(cancelled_connection);
    let mut cancelled_copy = Box::pin(
        cancelled
            .copy_in("COPY quackgis.main.cancelled_copy (id, name) FROM STDIN")
            .await
            .expect("start cancelled COPY"),
    );
    cancelled_copy
        .send(flushed_copy_rows(64))
        .await
        .expect("send cancelled flushed batch");
    cancel_token
        .cancel_query(tokio_postgres::NoTls)
        .await
        .expect("cancel COPY");
    assert!(
        cancelled_copy.as_mut().finish().await.is_err(),
        "cancelled COPY must fail"
    );
    drop(cancelled_copy);
    drop(cancelled);
    cancelled_task.abort();
    let _ = cancelled_task.await;
    let cancelled_rows = observer
        .query_one(
            "SELECT count(*)::BIGINT FROM quackgis.main.cancelled_copy",
            &[],
        )
        .await
        .expect("observe cancelled COPY rollback");
    assert_eq!(cancelled_rows.get::<_, i64>(0), 0);

    observer
        .batch_execute("CREATE TABLE quackgis.main.timed_out_copy(id INTEGER, name VARCHAR)")
        .await
        .expect("create timed-out COPY target");
    let (timed_out, timed_out_connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
        tokio_postgres::NoTls,
    )
    .await
    .expect("timed-out COPY client connect");
    let timed_out_task = tokio::spawn(timed_out_connection);
    let mut timed_out_copy = Box::pin(
        timed_out
            .copy_in("COPY public.timed_out_copy (id, name) FROM STDIN")
            .await
            .expect("start timed-out COPY"),
    );
    timed_out_copy
        .send(flushed_copy_rows(64))
        .await
        .expect("send timed-out flushed batch");
    tokio::time::sleep(Duration::from_millis(2_200)).await;
    let timeout_error = timed_out_copy
        .as_mut()
        .finish()
        .await
        .expect_err("timed-out COPY must fail on the next frame");
    assert_eq!(
        timeout_error.code(),
        Some(&tokio_postgres::error::SqlState::QUERY_CANCELED)
    );
    drop(timed_out_copy);
    drop(timed_out);
    timed_out_task.abort();
    let _ = timed_out_task.await;
    let timed_out_rows = observer
        .query_one("SELECT count(*)::BIGINT FROM public.timed_out_copy", &[])
        .await
        .expect("observe timed-out COPY rollback");
    assert_eq!(timed_out_rows.get::<_, i64>(0), 0);

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

    client
        .batch_execute("BEGIN")
        .await
        .expect("begin failed-transaction oracle");
    client
        .batch_execute("INSERT INTO quackgis.main.wire_mutations VALUES (9, 'must_rollback')")
        .await
        .expect("write before transaction error");
    let transaction_error = client
        .batch_execute("TRUNCATE quackgis.main.wire_mutations")
        .await
        .expect_err("unsupported statement fails the transaction");
    assert_eq!(
        transaction_error.code(),
        Some(&tokio_postgres::error::SqlState::FEATURE_NOT_SUPPORTED)
    );
    let aborted_query = client
        .query_one(
            "SELECT count(*)::BIGINT FROM quackgis.main.wire_mutations",
            &[],
        )
        .await
        .expect_err("failed transaction rejects extended queries");
    assert_eq!(
        aborted_query.code(),
        Some(&tokio_postgres::error::SqlState::IN_FAILED_SQL_TRANSACTION)
    );
    client
        .batch_execute("COMMIT")
        .await
        .expect("COMMIT rolls back a failed transaction");
    let after_failed_transaction = observer
        .query_one(
            "SELECT count(*)::BIGINT FROM quackgis.main.wire_mutations",
            &[],
        )
        .await
        .expect("observer after failed transaction");
    assert_eq!(after_failed_transaction.get::<_, i64>(0), 1);
    client
        .query_one(
            "SELECT count(*)::BIGINT FROM quackgis.main.wire_mutations",
            &[],
        )
        .await
        .expect("session reusable after failed transaction rollback");

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

    let cancel_token = client.cancel_token();
    let cancel_rows = client
        .query_raw(
            "SELECT i::BIGINT FROM range(1000000000) AS cancel_rows(i)",
            std::iter::empty::<&i32>(),
        )
        .await
        .expect("open cancellable query");
    futures::pin_mut!(cancel_rows);
    cancel_rows
        .next()
        .await
        .expect("cancellable first row")
        .expect("cancellable first row result");
    cancel_token
        .cancel_query(tokio_postgres::NoTls)
        .await
        .expect("send native cancel request");
    let cancellation_error = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match cancel_rows.next().await {
                Some(Ok(_)) => continue,
                Some(Err(error)) => break error,
                None => panic!("cancellable query completed without cancellation"),
            }
        }
    })
    .await
    .expect("native cancellation deadline");
    assert_eq!(
        cancellation_error.code(),
        Some(&tokio_postgres::error::SqlState::QUERY_CANCELED)
    );
    let quarantined_error = client
        .query_one("SELECT 1", &[])
        .await
        .expect_err("cancelled streaming session must be explicitly quarantined");
    assert_eq!(
        quarantined_error.code(),
        Some(&tokio_postgres::error::SqlState::INTERNAL_ERROR)
    );
    observer
        .query_one("SELECT 1", &[])
        .await
        .expect("a fresh session remains usable after cancellation quarantine");

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
    let copied_wkb = reopened
        .query(
            "SELECT string_agg(hex(geom_wkb), ',' ORDER BY id) \
             FROM quackgis.main.wire_copy",
        )
        .expect("all COPY WKB after restart");
    assert_eq!(
        copied_wkb[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("ordered WKB hex")
            .value(0),
        "010100000000000000000000000000000000000000,0101000000000000000000F03F000000000000F03F"
    );
    let bbox_after_restart = reopened
        .query(
            "SELECT _qg_minx, _qg_miny, _qg_maxx, _qg_maxy, \
             count(*) OVER ()::BIGINT FROM quackgis.main.layout_copy \
             WHERE id = 10",
        )
        .expect("maintained bbox after restart");
    for (column, expected) in [7.0, 8.0, 7.0, 8.0].into_iter().enumerate() {
        let values = bbox_after_restart[0]
            .column(column)
            .as_any()
            .downcast_ref::<arrow_array::Float64Array>()
            .expect("bbox Float64");
        assert_eq!(values.value(0), expected);
    }
    assert_eq!(first_i64(&bbox_after_restart[0], 4), 1);
    let scalar_after_restart = reopened
        .query(
            "SELECT count(*)::BIGINT, \
             count(*) FILTER (WHERE small_id IS NULL AND enabled IS NULL AND ratio IS NULL \
               AND observed_on IS NULL AND observed_at IS NULL AND amount IS NULL)::BIGINT, \
             max(CAST(small_id AS VARCHAR)), max(CAST(enabled AS VARCHAR)), \
             max(CAST(ratio AS VARCHAR)), max(CAST(observed_on AS VARCHAR)), \
             max(CAST(observed_at AS VARCHAR)), max(CAST(amount AS VARCHAR)) \
             FROM quackgis.main.wire_copy_scalars",
        )
        .expect("COPY scalar and NULL values after restart");
    assert_eq!(first_i64(&scalar_after_restart[0], 0), 2);
    assert_eq!(first_i64(&scalar_after_restart[0], 1), 1);
    let expected = [
        "7",
        "true",
        "1.25",
        "2026-07-11",
        "2026-07-11 12:34:56.123456",
        "12.34",
    ];
    for (column, expected) in expected.into_iter().enumerate() {
        let values = scalar_after_restart[0]
            .column(column + 2)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("scalar text result");
        assert!(!values.is_null(0));
        assert_eq!(values.value(0), expected);
    }

    let files_after_restart = reopened
        .query(
            "SELECT count(*)::BIGINT FROM \
             ducklake_list_files('quackgis', 'fragmented_copy', schema => 'main')",
        )
        .expect("compacted files after restart");
    let files_after_restart = first_i64(&files_after_restart[0], 0);
    assert!(
        files_after_restart * 2 <= fragment_files_before,
        "pgwire compaction must survive restart: before={fragment_files_before}, after={files_after_restart}"
    );
    let canonical = reopened
        .query("SELECT count(*)::BIGINT, sum(id)::BIGINT FROM quackgis.main.fragmented_copy")
        .expect("fragmented canonical result after pgwire compaction and restart");
    assert_eq!(first_i64(&canonical[0], 0), 8);
    assert_eq!(first_i64(&canonical[0], 1), 28);
}

const DEFAULT_BENCHMARK_ROWS: i64 = 100_000;
const BENCHMARK_QUERY: &str = "SELECT count(*)::BIGINT, sum(id)::BIGINT, \
    count(*) FILTER (WHERE grp = 7)::BIGINT, \
    count(*) FILTER (WHERE x BETWEEN 100 AND 199 AND y BETWEEN 100 AND 199)::BIGINT, \
    count(*) FILTER (WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), \
        ST_MakeEnvelope(100, 100, 199, 199)))::BIGINT, \
    sum(length(name))::BIGINT, sum(octet_length(geom_wkb))::BIGINT \
    FROM quackgis.main.benchmark_points";

#[tokio::test]
#[ignore = "requires the pinned DuckDB CLI and ADBC runtime"]
async fn current_duckdb_transport_profile() {
    let started = Instant::now();
    let evidence_level = EvidenceLevel::parse(
        &std::env::var("QUACKGIS_EVIDENCE_LEVEL").unwrap_or_else(|_| "smoke".to_owned()),
    )
    .expect("valid QUACKGIS_EVIDENCE_LEVEL");
    assert_ne!(
        evidence_level,
        EvidenceLevel::External,
        "the local transport scenario cannot emit external evidence"
    );
    let execution_environment = ExecutionEnvironment::parse(
        &std::env::var("QUACKGIS_EXECUTION_ENVIRONMENT")
            .unwrap_or_else(|_| "host_process".to_owned()),
    )
    .expect("valid QUACKGIS_EXECUTION_ENVIRONMENT");
    let benchmark_rows = std::env::var("QUACKGIS_BENCHMARK_ROWS")
        .map(|value| value.parse::<i64>().expect("integer benchmark rows"))
        .unwrap_or(DEFAULT_BENCHMARK_ROWS);
    assert!(
        (1..=100_000_000).contains(&benchmark_rows),
        "benchmark rows must be between 1 and 100M"
    );
    let driver_path = std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER").expect("set ADBC driver");
    let duckdb_bin = std::env::var_os("DUCKDB_BIN").expect("set DUCKDB_BIN");
    let output_path = std::env::var_os("QUACKGIS_BENCHMARK_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ".tmp/duckdb-current-benchmark/manifest.json".into());
    let temp = tempfile::tempdir().expect("benchmark tempdir");
    let catalog_path = temp.path().join("catalog.ducklake");
    let data_path = temp.path().join("data");
    std::fs::create_dir(&data_path).expect("benchmark data path");

    let create_sql = format!(
        "LOAD spatial; LOAD ducklake; SET threads=1; \
         SET ducklake_default_data_inlining_row_limit=0; \
         ATTACH 'ducklake:{}' AS quackgis (DATA_PATH '{}', DATA_INLINING_ROW_LIMIT 0); \
         CREATE TABLE quackgis.main.benchmark_points AS \
         SELECT i::INTEGER AS id, \
           'point-' || lpad(i::VARCHAR, greatest(6, length(i::VARCHAR)), '0') AS name, \
           (i % 32)::SMALLINT AS grp, ((i * 17) % 1000)::DOUBLE AS x, \
           ((i * 31) % 1000)::DOUBLE AS y, \
           ST_AsWKB(ST_Point(((i * 17) % 1000)::DOUBLE, ((i * 31) % 1000)::DOUBLE)) AS geom_wkb \
         FROM range({benchmark_rows}) AS r(i)",
        sql_literal_path(&catalog_path),
        sql_literal_path(&data_path),
    );
    let load_started = Instant::now();
    run_duckdb(&duckdb_bin, &create_sql);
    let load_ms = load_started.elapsed().as_secs_f64() * 1000.0;

    let expected = benchmark_expected(benchmark_rows);
    let direct_sql = format!(
        "LOAD spatial; LOAD ducklake; ATTACH 'ducklake:{}' AS quackgis (DATA_PATH '{}'); {BENCHMARK_QUERY}",
        sql_literal_path(&catalog_path),
        sql_literal_path(&data_path),
    );
    let mut direct_samples = Vec::new();
    for _ in 0..3 {
        let sample_started = Instant::now();
        let output = run_duckdb(&duckdb_bin, &direct_sql);
        direct_samples.push(sample_started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(parse_canonical_csv(&output), expected);
    }

    let config = DuckDbAdbcConfig {
        driver_path: driver_path.into(),
        database_uri: ":memory:".to_owned(),
        ducklake_uri: format!("ducklake:{}", catalog_path.display()),
        catalog_name: "quackgis".to_owned(),
        data_path: data_path.display().to_string(),
        extension_policy: ExtensionPolicy::LoadOnly,
    };
    let open_started = Instant::now();
    let storage = Arc::new(DuckDbAdbcStorage::open(config).expect("benchmark ADBC storage"));
    let adbc_open_ms = open_started.elapsed().as_secs_f64() * 1000.0;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("benchmark listener");
    let port = listener.local_addr().expect("benchmark address").port();
    let options = ServerOptions::new().with_max_connections(4);
    let server_storage = Arc::clone(&storage);
    let server_task = tokio::spawn(async move {
        quackgis_server::pgwire_server::serve_duckdb_on_listener(
            server_storage,
            listener,
            &options,
            quackgis_server::auth::AuthConfig::trust(),
        )
        .await
    });
    let handshake_started = Instant::now();
    let (client, connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
        tokio_postgres::NoTls,
    )
    .await
    .expect("benchmark pgwire connect");
    let handshake_ms = handshake_started.elapsed().as_secs_f64() * 1000.0;
    let connection_task = tokio::spawn(connection);
    let statement = client
        .prepare(BENCHMARK_QUERY)
        .await
        .expect("prepare benchmark");
    assert_eq!(
        canonical_batches(
            &storage
                .query(BENCHMARK_QUERY)
                .expect("warm ADBC benchmark query")
        ),
        expected
    );
    let warm_row = client
        .query_one(&statement, &[])
        .await
        .expect("warm pgwire benchmark query");
    assert_eq!(
        (0..7)
            .map(|index| warm_row.get(index))
            .collect::<Vec<i64>>(),
        expected
    );
    let mut adbc_samples = Vec::new();
    let mut pgwire_samples = Vec::new();
    let mut sample_order = Vec::new();
    for iteration in 0..5 {
        if iteration % 2 == 0 {
            sample_order.extend(["adbc", "pgwire"]);
            adbc_samples.push(run_adbc_benchmark(&storage, &expected));
            pgwire_samples.push(run_pgwire_benchmark(&client, &statement, &expected).await);
        } else {
            sample_order.extend(["pgwire", "adbc"]);
            pgwire_samples.push(run_pgwire_benchmark(&client, &statement, &expected).await);
            adbc_samples.push(run_adbc_benchmark(&storage, &expected));
        }
    }
    drop(client);
    connection_task.abort();
    server_task.abort();

    let (load_budget, handshake_budget, direct_budget, adbc_budget, pgwire_budget, total_budget) =
        match evidence_level {
            EvidenceLevel::Smoke => (30_000.0, 5_000.0, 15_000.0, 10_000.0, 10_000.0, 90),
            EvidenceLevel::Local => (120_000.0, 5_000.0, 60_000.0, 60_000.0, 60_000.0, 300),
            EvidenceLevel::Reference => (300_000.0, 5_000.0, 120_000.0, 120_000.0, 120_000.0, 900),
            EvidenceLevel::External => unreachable!("external level rejected above"),
        };
    assert!(
        load_ms < load_budget,
        "{} load exceeded {load_budget} ms",
        evidence_level.as_str()
    );
    assert!(
        handshake_ms < handshake_budget,
        "pgwire handshake exceeded {handshake_budget} ms"
    );
    for (path, samples, budget) in [
        ("direct", &direct_samples, direct_budget),
        ("adbc", &adbc_samples, adbc_budget),
        ("pgwire", &pgwire_samples, pgwire_budget),
    ] {
        assert!(
            samples.iter().all(|sample| *sample < budget),
            "{path} smoke sample exceeded {budget} ms: {samples:?}"
        );
    }
    let adbc_p50_ms = sample_p50(&adbc_samples);
    let pgwire_p50_ms = sample_p50(&pgwire_samples);
    let pgwire_overhead_ratio = pgwire_p50_ms / adbc_p50_ms;
    let overhead_budget_eligible = adbc_p50_ms >= 1_000.0;
    if evidence_level == EvidenceLevel::Reference {
        assert!(
            overhead_budget_eligible,
            "reference ADBC p50 {adbc_p50_ms:.3} ms is shorter than the required one-second scan"
        );
        assert!(
            pgwire_overhead_ratio <= 1.15,
            "reference pgwire/ADBC p50 ratio {pgwire_overhead_ratio:.3} exceeds 1.15"
        );
    }
    assert!(started.elapsed() < Duration::from_secs(total_budget));

    let profile_id =
        if evidence_level == EvidenceLevel::Smoke && benchmark_rows == DEFAULT_BENCHMARK_ROWS {
            "duckdb-current-smoke-r100k-v2".to_owned()
        } else {
            format!(
                "duckdb-transport-{}-r{}-v2",
                evidence_level.as_str(),
                benchmark_rows
            )
        };
    let manifest = EvidenceEnvelope::collect(
        EvidenceProfile::new(
            profile_id,
            evidence_level,
            execution_environment,
            "single-client warm scalar full-scan transport profile; not a streaming-result or selective-scan claim",
        ),
        json!({
            "rows": benchmark_rows,
            "logical_bytes": null,
            "files": null,
            "row_groups": null,
        }),
        json!({
            "canonical_result": expected,
            "bbox_equals_exact": expected[3] == expected[4],
            "wkb_bytes": expected[6],
            "all_transport_results_exact": true,
        }),
        json!({
            "load_ms": load_ms,
            "adbc_open_ms": adbc_open_ms,
            "pgwire_handshake_ms": handshake_ms,
            "paths": {
                "direct_duckdb_cli": sample_summary(&direct_samples),
                "adbc": sample_summary(&adbc_samples),
                "pgwire": sample_summary(&pgwire_samples),
            },
            "interleaved_sample_order": sample_order,
            "adbc_p50_ms": adbc_p50_ms,
            "pgwire_p50_ms": pgwire_p50_ms,
            "pgwire_to_adbc_p50_ratio": pgwire_overhead_ratio,
            "overhead_budget_eligible": overhead_budget_eligible,
        }),
        json!({
            "load_max_ms": load_budget,
            "handshake_max_ms": handshake_budget,
            "direct_sample_max_ms": direct_budget,
            "adbc_sample_max_ms": adbc_budget,
            "pgwire_sample_max_ms": pgwire_budget,
            "reference_adbc_p50_min_ms": 1000.0,
            "reference_pgwire_to_adbc_p50_ratio_max": 1.15,
            "total_max_seconds": total_budget,
        }),
    )
    .expect("collect benchmark evidence");
    manifest
        .write(&output_path)
        .expect("write benchmark manifest");
    println!("duckdb_current_benchmark_ok out={}", output_path.display());
}

fn sql_literal_path(path: &Path) -> String {
    path.display().to_string().replace('\'', "''")
}

fn run_duckdb(binary: &std::ffi::OsStr, sql: &str) -> String {
    let output = std::process::Command::new(binary)
        .args(["-csv", "-noheader", ":memory:", "-c", sql])
        .output()
        .expect("run DuckDB CLI");
    assert!(
        output.status.success(),
        "DuckDB CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("DuckDB UTF-8 output")
}

fn parse_canonical_csv(output: &str) -> Vec<i64> {
    output
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .expect("DuckDB result line")
        .split(',')
        .map(|value| value.parse().expect("integer benchmark result"))
        .collect()
}

fn canonical_batches(batches: &[RecordBatch]) -> Vec<i64> {
    assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 1);
    let batch = batches
        .iter()
        .find(|batch| batch.num_rows() == 1)
        .expect("result batch");
    (0..7)
        .map(|index| {
            batch
                .column(index)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("Int64 benchmark result")
                .value(0)
        })
        .collect()
}

fn run_adbc_benchmark(storage: &DuckDbAdbcStorage, expected: &[i64]) -> f64 {
    let sample_started = Instant::now();
    let batches = storage
        .query(BENCHMARK_QUERY)
        .expect("ADBC benchmark query");
    let elapsed_ms = sample_started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(canonical_batches(&batches), expected);
    elapsed_ms
}

async fn run_pgwire_benchmark(
    client: &tokio_postgres::Client,
    statement: &tokio_postgres::Statement,
    expected: &[i64],
) -> f64 {
    let sample_started = Instant::now();
    let row = client
        .query_one(statement, &[])
        .await
        .expect("pgwire benchmark query");
    let elapsed_ms = sample_started.elapsed().as_secs_f64() * 1000.0;
    let actual = (0..7).map(|index| row.get(index)).collect::<Vec<i64>>();
    assert_eq!(actual, expected);
    elapsed_ms
}

fn benchmark_expected(rows: i64) -> Vec<i64> {
    let mut group = 0_i64;
    let mut bbox = 0_i64;
    let mut text_bytes = 0_i64;
    for id in 0..rows {
        if id % 32 == 7 {
            group += 1;
        }
        let x = (id * 17) % 1000;
        let y = (id * 31) % 1000;
        if (100..=199).contains(&x) && (100..=199).contains(&y) {
            bbox += 1;
        }
        text_bytes += 6 + id.to_string().len().max(6) as i64;
    }
    vec![
        rows,
        rows * (rows - 1) / 2,
        group,
        bbox,
        bbox,
        text_bytes,
        rows * 21,
    ]
}

#[test]
fn benchmark_oracle_handles_rows_beyond_six_digits() {
    assert_eq!(benchmark_expected(1_000_000)[5], 12_000_000);
    assert_eq!(benchmark_expected(1_000_001)[5], 12_000_013);
    assert_eq!(benchmark_expected(10_000_000)[5], 129_000_000);
}

fn sample_summary(samples: &[f64]) -> serde_json::Value {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({
        "samples_ms": samples,
        "min_ms": sorted[0],
        "p50_ms": sorted[sorted.len() / 2],
        "max_ms": sorted[sorted.len() - 1],
        "mean_ms": samples.iter().sum::<f64>() / samples.len() as f64,
    })
}

fn sample_p50(samples: &[f64]) -> f64 {
    assert!(!samples.is_empty(), "benchmark samples must not be empty");
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

#[test]
fn benchmark_p50_uses_middle_sample() {
    assert_eq!(sample_p50(&[5.0, 1.0, 3.0, 2.0, 4.0]), 3.0);
}
