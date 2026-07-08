// SPDX-License-Identifier: Apache-2.0
//! DuckLake storage roundtrip + restart persistence (M1 contract).
//!
//! datafusion-ducklake main is used to target the official DuckLake 1.0+
//! format. SQL CTAS/DDL through DataFusion currently no-ops / fails to expose
//! newly-created tables in our integration path, so the M1 storage gate uses
//! the upstream-supported writer API (`DuckLakeTableWriter`). SQL DDL mapping
//! is tracked as follow-up M1/M2 compatibility work.

mod common;

use std::io::Cursor;
use std::pin::pin;
use std::sync::Arc;

use datafusion::arrow::array::{BinaryArray, Int32Array};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion_ducklake::DuckLakeTableWriter;
use futures::SinkExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_postgres::NoTls;

use common::ServerHandle;
use quackgis_server::context::{StoragePaths, build_session_context_with_storage};

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    // OGC WKB, little endian, Point type=1, x, y.
    let mut out = Vec::with_capacity(1 + 4 + 8 + 8);
    out.push(1); // little endian
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&y.to_le_bytes());
    out
}

fn pg_copy_bytea(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &byte in bytes {
        if !(40..=126).contains(&byte) || byte == b'\\' {
            out.push_str(&format!("\\\\{byte:03o}"));
        } else {
            out.push(byte as char);
        }
    }
    out
}

async fn write_nums(paths: &StoragePaths, table: &str, values: &[i32]) {
    let writer = paths.metadata_writer().await.expect("writer");
    // Ensure an initial snapshot + main schema exist for a fresh catalog.
    let snapshot = writer.create_snapshot().expect("snapshot");
    writer
        .get_or_create_schema("main", None, snapshot)
        .expect("main schema");

    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)])),
        vec![Arc::new(Int32Array::from(values.to_vec()))],
    )
    .expect("batch");
    let object_store = paths.object_store().expect("object store");
    DuckLakeTableWriter::new(writer, object_store)
        .expect("table writer")
        .write_table("main", table, &[batch])
        .await
        .expect("write table");
}

async fn write_geo(paths: &StoragePaths) {
    let writer = paths.metadata_writer().await.expect("writer");
    let snapshot = writer.create_snapshot().expect("snapshot");
    writer
        .get_or_create_schema("main", None, snapshot)
        .expect("main schema");

    let wkbs = [
        point_wkb(1.0, 2.0),
        point_wkb(2.0, 4.0),
        point_wkb(3.0, 6.0),
    ];
    let geom_values: Vec<Option<&[u8]>> = wkbs.iter().map(|v| Some(v.as_slice())).collect();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("geom", DataType::Binary, true),
        ])),
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3])),
            Arc::new(BinaryArray::from(geom_values)),
        ],
    )
    .expect("batch");
    let object_store = paths.object_store().expect("object store");
    DuckLakeTableWriter::new(writer, object_store)
        .expect("table writer")
        .write_table("main", "geo", &[batch])
        .await
        .expect("write geo table");
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_sql_ctas_and_insert_route_to_writer() {
    let server = ServerHandle::start().await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE src (id INT);
             INSERT INTO src VALUES (1), (2), (3);
             CREATE TABLE quackgis.main.nums AS SELECT id FROM src;
             INSERT INTO quackgis.main.nums SELECT CAST(4 AS INT) AS id;",
        )
        .await
        .expect("SQL CTAS + INSERT routed into DuckLake writer");

    let rows = client
        .query("SELECT id FROM quackgis.main.nums ORDER BY id", &[])
        .await
        .expect("SELECT after SQL writes");
    let ids: Vec<i32> = rows.into_iter().map(|r| r.get::<_, i32>(0)).collect();
    assert_eq!(ids, vec![1, 2, 3, 4]);
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_copy_from_stdin_appends_gdal_style_text_bytea() {
    let server = ServerHandle::start().await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute("CREATE TABLE quackgis.main.copy_points (id INT, name TEXT, geom BINARY)")
        .await
        .expect("create COPY target");

    let row1 = point_wkb(1.0, 2.0);
    let row2 = point_wkb(3.0, 4.0);
    let copy_data = format!(
        "1\talpha\t{}\n2\t\\N\t{}\n",
        pg_copy_bytea(&row1),
        pg_copy_bytea(&row2)
    );
    let sink = client
        .copy_in::<_, Cursor<Vec<u8>>>("COPY quackgis.main.copy_points (id, name, geom) FROM STDIN")
        .await
        .expect("enter COPY IN");
    let mut sink = pin!(sink);
    sink.as_mut()
        .send(Cursor::new(copy_data.into_bytes()))
        .await
        .expect("send COPY data");
    let copied = sink.as_mut().finish().await.expect("finish COPY");
    assert_eq!(copied, 2);

    let rows = client
        .query(
            "SELECT id, name, ST_AsText(ST_GeomFromWKB(geom)) \
             FROM quackgis.main.copy_points ORDER BY id",
            &[],
        )
        .await
        .expect("read copied rows");
    let got: Vec<(i32, Option<String>, String)> = rows
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    assert_eq!(
        got,
        vec![
            (1, Some("alpha".to_string()), "POINT(1 2)".to_string()),
            (2, None, "POINT(3 4)".to_string()),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_simple_protocol_copy_from_stdin_matches_gdal_path() {
    let server = ServerHandle::start().await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute("CREATE TABLE public.simple_copy_points (id INT, name TEXT, geom BINARY)")
        .await
        .expect("create simple COPY target");

    let row1 = point_wkb(5.0, 6.0);
    let row2 = point_wkb(7.0, 8.0);
    let copy_data = format!(
        "1\talpha\t{}\n2\tbeta\t{}\n",
        pg_copy_bytea(&row1),
        pg_copy_bytea(&row2)
    );
    let split = copy_data.len() / 2;
    let command = raw_simple_copy_from_stdin(
        &server.conn_str(),
        "COPY public.simple_copy_points (id, name, geom) FROM STDIN;",
        &[
            &copy_data.as_bytes()[..split],
            &copy_data.as_bytes()[split..],
        ],
    )
    .await;
    assert_eq!(command, "COPY 2");

    let rows = client
        .query(
            "SELECT id, name, ST_AsText(ST_GeomFromWKB(geom)) \
             FROM public.simple_copy_points ORDER BY id",
            &[],
        )
        .await
        .expect("read simple COPY rows");
    let got: Vec<(i32, String, String)> = rows
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    assert_eq!(
        got,
        vec![
            (1, "alpha".to_string(), "POINT(5 6)".to_string()),
            (2, "beta".to_string(), "POINT(7 8)".to_string()),
        ]
    );
}

async fn raw_simple_copy_from_stdin(conn_str: &str, copy_sql: &str, chunks: &[&[u8]]) -> String {
    let (host, port) = parse_conn_addr(conn_str);
    let mut stream = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .expect("raw pgwire connect");
    send_startup(&mut stream).await;
    read_until_ready(&mut stream).await;

    send_frontend_message(&mut stream, b'Q', cstring_body(copy_sql).as_slice()).await;
    let (typ, body) = read_backend_message(&mut stream).await;
    match typ {
        b'G' => {}
        b'E' => panic!("COPY query failed: {}", error_message(&body)),
        other => panic!("expected CopyInResponse, got backend message {other:?}"),
    }

    for chunk in chunks {
        send_frontend_message(&mut stream, b'd', chunk).await;
    }
    send_frontend_message(&mut stream, b'c', &[]).await;

    let mut command = None;
    loop {
        let (typ, body) = read_backend_message(&mut stream).await;
        match typ {
            b'C' => command = Some(cstring_from_body(&body)),
            b'Z' => break,
            b'E' => panic!("COPY data failed: {}", error_message(&body)),
            _ => {}
        }
    }
    command.expect("CommandComplete after COPY")
}

fn parse_conn_addr(conn_str: &str) -> (String, u16) {
    let mut host = "127.0.0.1".to_string();
    let mut port = 5432_u16;
    for part in conn_str.split_whitespace() {
        if let Some(value) = part.strip_prefix("host=") {
            host = value.to_string();
        } else if let Some(value) = part.strip_prefix("port=") {
            port = value.parse().expect("port in connection string");
        }
    }
    (host, port)
}

async fn send_startup(stream: &mut tokio::net::TcpStream) {
    let mut body = Vec::new();
    body.extend_from_slice(&196_608_i32.to_be_bytes());
    body.extend_from_slice(b"user\0postgres\0database\0quackgis\0client_encoding\0UTF8\0\0");
    let len = (body.len() + 4) as i32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .expect("write startup length");
    stream.write_all(&body).await.expect("write startup body");
}

async fn send_frontend_message(stream: &mut tokio::net::TcpStream, typ: u8, body: &[u8]) {
    stream.write_all(&[typ]).await.expect("write message type");
    let len = (body.len() + 4) as i32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .expect("write message length");
    stream.write_all(body).await.expect("write message body");
}

async fn read_until_ready(stream: &mut tokio::net::TcpStream) {
    loop {
        let (typ, body) = read_backend_message(stream).await;
        match typ {
            b'Z' => break,
            b'E' => panic!("startup failed: {}", error_message(&body)),
            _ => {}
        }
    }
}

async fn read_backend_message(stream: &mut tokio::net::TcpStream) -> (u8, Vec<u8>) {
    let mut typ = [0_u8; 1];
    stream
        .read_exact(&mut typ)
        .await
        .expect("read backend message type");
    let mut len = [0_u8; 4];
    stream
        .read_exact(&mut len)
        .await
        .expect("read backend message length");
    let len = i32::from_be_bytes(len);
    assert!(len >= 4, "invalid backend message length {len}");
    let mut body = vec![0_u8; len as usize - 4];
    stream
        .read_exact(&mut body)
        .await
        .expect("read backend message body");
    (typ[0], body)
}

fn cstring_body(value: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(value.len() + 1);
    body.extend_from_slice(value.as_bytes());
    body.push(0);
    body
}

fn cstring_from_body(body: &[u8]) -> String {
    let end = body.iter().position(|&b| b == 0).unwrap_or(body.len());
    String::from_utf8_lossy(&body[..end]).into_owned()
}

fn error_message(body: &[u8]) -> String {
    let mut message = String::new();
    let mut i = 0;
    while i < body.len() {
        let field = body[i];
        if field == 0 {
            break;
        }
        i += 1;
        let start = i;
        while i < body.len() && body[i] != 0 {
            i += 1;
        }
        let value = String::from_utf8_lossy(&body[start..i]);
        if field == b'M' {
            message = value.into_owned();
        }
        i += 1;
    }
    if message.is_empty() {
        format!("unparsed ErrorResponse: {body:?}")
    } else {
        message
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_compact_table_rewrites_without_changing_results() {
    let server = ServerHandle::start().await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE public.compact_points (id INT, geom BINARY, name TEXT);
             INSERT INTO public.compact_points (id, geom, name) VALUES
                (2, X'0101000000000000000000f03f0000000000000040', 'b');
             INSERT INTO public.compact_points (id, geom, name) VALUES
                (1, X'010100000000000000000000000000000000000000', 'a');",
        )
        .await
        .expect("seed compact target");

    let before: Vec<(i32, String)> = client
        .query(
            "SELECT id, ST_AsText(ST_GeomFromWKB(geom)) FROM public.compact_points ORDER BY id",
            &[],
        )
        .await
        .expect("select before compact")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();

    client
        .batch_execute("CALL quackgis_compact_table('public.compact_points')")
        .await
        .expect("compact table");

    let after: Vec<(i32, String)> = client
        .query(
            "SELECT id, ST_AsText(ST_GeomFromWKB(geom)) FROM public.compact_points ORDER BY id",
            &[],
        )
        .await
        .expect("select after compact")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();

    assert_eq!(after, before);
    assert_eq!(
        after,
        vec![(1, "POINT(0 0)".to_string()), (2, "POINT(1 2)".to_string()),]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_writer_api_roundtrip_through_wire() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let paths = StoragePaths::new(
        tmp.path().join("quackgis.db").to_str().unwrap(),
        tmp.path().join("data").to_str().unwrap(),
    )
    .expect("paths");
    write_nums(&paths, "nums", &[1, 2, 3]).await;
    let server = ServerHandle::start_with_tempdir(tmp).await;

    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    let rows = client
        .query("SELECT id FROM quackgis.main.nums ORDER BY id", &[])
        .await
        .expect("SELECT");
    let ids: Vec<i32> = rows.into_iter().map(|r| r.get::<_, i32>(0)).collect();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_data_survives_process_restart() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let catalog = tmp.path().join("quackgis.db");
    let data = tmp.path().join("data");
    let paths =
        StoragePaths::new(catalog.to_str().unwrap(), data.to_str().unwrap()).expect("paths");

    write_nums(&paths, "nums", &[1, 2]).await;

    let ctx = build_session_context_with_storage(paths)
        .await
        .expect("read-side context");
    let out = ctx
        .sql("SELECT id FROM quackgis.main.nums ORDER BY id")
        .await
        .expect("SELECT")
        .collect()
        .await
        .expect("collect");
    let rendered = datafusion::arrow::util::pretty::pretty_format_batches(&out)
        .expect("render")
        .to_string();
    assert!(rendered.contains("1"), "expected 1 in output:\n{rendered}");
    assert!(rendered.contains("2"), "expected 2 in output:\n{rendered}");
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_spatial_geometry_roundtrips_as_wkb() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let paths = StoragePaths::new(
        tmp.path().join("quackgis.db").to_str().unwrap(),
        tmp.path().join("data").to_str().unwrap(),
    )
    .expect("paths");
    write_geo(&paths).await;
    let server = ServerHandle::start_with_tempdir(tmp).await;

    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    let rows = client
        .query(
            "SELECT id, ST_AsText(ST_GeomFromWKB(geom)) FROM quackgis.main.geo ORDER BY id",
            &[],
        )
        .await
        .expect("SELECT with ST_AsText");
    let points: Vec<(i32, String)> = rows
        .into_iter()
        .map(|r| (r.get::<_, i32>(0), r.get::<_, String>(1)))
        .collect();
    assert_eq!(
        points,
        vec![
            (1, "POINT(1 2)".to_string()),
            (2, "POINT(2 4)".to_string()),
            (3, "POINT(3 6)".to_string()),
        ],
        "geometry WKB round-trip through DuckLake should be lossless"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_bare_create_insert_values_update_delete() {
    let server = ServerHandle::start().await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE quackgis.main.items (id INT, qty INT, note VARCHAR);\n             INSERT INTO quackgis.main.items (id, qty, note) VALUES\n                 (1, 10, 'a'), (2, 20, 'b'), (3, 30, 'c');\n             UPDATE quackgis.main.items SET qty = qty + 1, note = 'z' WHERE id = 2;\n             DELETE FROM quackgis.main.items WHERE id = 1;",
        )
        .await
        .expect("CREATE + INSERT VALUES + UPDATE + DELETE");

    let rows = client
        .query(
            "SELECT id, qty, note FROM quackgis.main.items ORDER BY id",
            &[],
        )
        .await
        .expect("SELECT");
    let got: Vec<(i32, i32, String)> = rows
        .into_iter()
        .map(|r| {
            (
                r.get::<_, i32>(0),
                r.get::<_, i32>(1),
                r.get::<_, String>(2),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![(2, 21, "z".to_string()), (3, 30, "c".to_string())]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_filter_pushdown_path_accepts_predicates() {
    // datafusion-ducklake declares filter pushdown as Inexact so Parquet can
    // use row-group/page stats while DataFusion reapplies filters for
    // correctness. This test guards the public query path with predicates.
    let server = ServerHandle::start().await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE quackgis.main.nums AS SELECT n::INT AS id FROM generate_series(1, 100) AS t(n);",
        )
        .await
        .expect("CTAS");

    let count: i64 = client
        .query_one(
            "SELECT count(*) FROM quackgis.main.nums WHERE id BETWEEN 10 AND 20",
            &[],
        )
        .await
        .expect("filtered count")
        .get(0);
    assert_eq!(count, 11);
}
