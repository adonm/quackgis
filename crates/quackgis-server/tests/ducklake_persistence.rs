// SPDX-License-Identifier: Apache-2.0
//! DuckLake storage roundtrip + restart persistence (M1 contract).
//!
//! datafusion-ducklake main is used to target the official DuckLake 1.0+
//! format. SQL CTAS/DDL through DataFusion currently no-ops / fails to expose
//! newly-created tables in our integration path, so the M1 storage gate uses
//! the upstream-supported writer API (`DuckLakeTableWriter`). SQL DDL mapping
//! is tracked as follow-up M1/M2 compatibility work.

mod common;

use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::pin::pin;
use std::sync::Arc;

use datafusion::arrow::array::{BinaryArray, Int32Array};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion_ducklake::DuckLakeTableWriter;
use futures::SinkExt;
use sqlx::Row;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_postgres::NoTls;

use common::ServerHandle;
use quackgis_server::context::{StoragePaths, build_session_context_with_storage};
use quackgis_server::ducklake_sql;

static NATIVE_MUTATION_FAILPOINT_TEST_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

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

fn copy_dir_all(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create destination directory");
    for entry in fs::read_dir(src).expect("read source directory") {
        let entry = entry.expect("source directory entry");
        let ty = entry.file_type().expect("source entry type");
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy backup file");
        }
    }
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

async fn live_column_types(server: &ServerHandle, table: &str) -> Vec<(String, String)> {
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite:{}",
        server.tmp_dir().join("quackgis.db").display()
    ))
    .await
    .expect("open DuckLake catalog");
    let rows = sqlx::query(
        "SELECT c.column_name, c.column_type
         FROM ducklake_column c
         JOIN ducklake_table t ON t.table_id = c.table_id
         WHERE t.table_name = ? AND t.end_snapshot IS NULL AND c.end_snapshot IS NULL
         ORDER BY c.column_order",
    )
    .bind(table)
    .fetch_all(&pool)
    .await
    .expect("read live DuckLake column types");
    rows.into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect()
}

async fn assert_durable_spatial_identity(client: &tokio_postgres::Client, conn_str: &str) {
    let geometry = client
        .query(
            "SELECT f_geometry_column, coord_dimension, srid, type
             FROM geometry_columns WHERE f_table_name = 'durable_identity'",
            &[],
        )
        .await
        .expect("geometry_columns identity row");
    assert_eq!(geometry.len(), 1);
    assert_eq!(geometry[0].get::<_, String>(0), "location");
    assert_eq!(geometry[0].get::<_, i32>(1), 2);
    assert_eq!(geometry[0].get::<_, i32>(2), 0);
    assert_eq!(geometry[0].get::<_, String>(3), "GEOMETRY");

    let geography = client
        .query(
            "SELECT f_geography_column, coord_dimension, srid, type
             FROM geography_columns WHERE f_table_name = 'durable_identity'",
            &[],
        )
        .await
        .expect("geography_columns identity row");
    assert_eq!(geography.len(), 1);
    assert_eq!(geography[0].get::<_, String>(0), "earth");
    assert_eq!(geography[0].get::<_, i32>(1), 2);
    assert_eq!(geography[0].get::<_, i32>(2), 0);
    assert_eq!(geography[0].get::<_, String>(3), "GEOGRAPHY");

    let oids = raw_row_description_oids(
        conn_str,
        "SELECT location, earth, payload, geom FROM public.durable_identity LIMIT 1",
    )
    .await;
    assert_eq!(oids, vec![90_001, 90_002, 17, 25]);

    let bytes = raw_simple_query_first_row(
        conn_str,
        "SELECT location, earth, payload FROM public.durable_identity LIMIT 1",
    )
    .await;
    assert_eq!(
        bytes[0].as_deref(),
        Some(bytea_hex_text(&point_wkb(3.0, 4.0)).as_slice()),
        "updated geometry remains WKB bytes"
    );
    assert_eq!(
        bytes[1].as_deref(),
        Some(bytea_hex_text(&point_wkb(5.0, 6.0)).as_slice())
    );
    assert_eq!(bytes[2].as_deref(), Some(b"\\xdeadbeef".as_slice()));
}

#[tokio::test(flavor = "multi_thread")]
async fn explicit_spatial_family_identity_survives_rewrites_and_restart() {
    let mut server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let connection = tokio::spawn(connection);

    client
        .batch_execute(
            "CREATE TABLE public.durable_identity (
                 location GEOMETRY(Point,4326),
                 earth GEOGRAPHY(Point,4326),
                 payload BINARY,
                 geom TEXT
             );
             INSERT INTO public.durable_identity VALUES (
                 X'010100000000000000000000000000000000000000',
                 X'010100000000000000000014400000000000001840',
                 X'DEADBEEF',
                 'not spatial'
             );
             UPDATE public.durable_identity
                SET location = X'010100000000000000000008400000000000001040'
              WHERE geom = 'not spatial';
             CALL quackgis_compact_table('public.durable_identity');",
        )
        .await
        .expect("create, mutate, and compact durable spatial families");

    let types = live_column_types(&server, "durable_identity").await;
    for expected in [
        ("location", "geometry"),
        ("earth", "geography"),
        ("payload", "blob"),
        ("geom", "varchar"),
    ] {
        assert!(
            types
                .iter()
                .any(|(name, ty)| name == expected.0 && ty == expected.1),
            "missing persisted family/type {expected:?} in {types:?}"
        );
    }
    assert_durable_spatial_identity(&client, &server.conn_str()).await;

    drop(client);
    connection.abort();
    server.restart().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect after restart");
    let _connection = tokio::spawn(connection);

    assert_durable_spatial_identity(&client, &server.conn_str()).await;
    let reopened_types = live_column_types(&server, "durable_identity").await;
    assert_eq!(reopened_types, types);
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

async fn raw_row_description_oids(conn_str: &str, sql: &str) -> Vec<u32> {
    let (host, port) = parse_conn_addr(conn_str);
    let mut stream = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .expect("raw pgwire RowDescription connect");
    send_startup(&mut stream).await;
    read_until_ready(&mut stream).await;
    send_frontend_message(&mut stream, b'Q', cstring_body(sql).as_slice()).await;

    loop {
        let (typ, body) = read_backend_message(&mut stream).await;
        match typ {
            b'T' => return parse_row_description_oids(&body),
            b'E' => panic!("RowDescription query failed: {}", error_message(&body)),
            b'Z' => panic!("RowDescription query returned no columns"),
            _ => {}
        }
    }
}

async fn raw_simple_query_first_row(conn_str: &str, sql: &str) -> Vec<Option<Vec<u8>>> {
    let (host, port) = parse_conn_addr(conn_str);
    let mut stream = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .expect("raw pgwire data row connect");
    send_startup(&mut stream).await;
    read_until_ready(&mut stream).await;
    send_frontend_message(&mut stream, b'Q', cstring_body(sql).as_slice()).await;

    loop {
        let (typ, body) = read_backend_message(&mut stream).await;
        match typ {
            b'D' => return parse_data_row(&body),
            b'E' => panic!("data row query failed: {}", error_message(&body)),
            b'Z' => panic!("data row query returned no rows"),
            _ => {}
        }
    }
}

fn parse_data_row(body: &[u8]) -> Vec<Option<Vec<u8>>> {
    assert!(body.len() >= 2, "short DataRow body");
    let count = u16::from_be_bytes(body[0..2].try_into().unwrap()) as usize;
    let mut offset = 2;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        assert!(offset + 4 <= body.len(), "short DataRow field length");
        let len = i32::from_be_bytes(body[offset..offset + 4].try_into().unwrap());
        offset += 4;
        if len < 0 {
            values.push(None);
            continue;
        }
        let len = len as usize;
        assert!(offset + len <= body.len(), "short DataRow field value");
        values.push(Some(body[offset..offset + len].to_vec()));
        offset += len;
    }
    values
}

fn bytea_hex_text(bytes: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = Vec::with_capacity(2 + bytes.len() * 2);
    text.extend_from_slice(b"\\x");
    for &byte in bytes {
        text.push(HEX[(byte >> 4) as usize]);
        text.push(HEX[(byte & 0x0f) as usize]);
    }
    text
}

fn parse_row_description_oids(body: &[u8]) -> Vec<u32> {
    assert!(body.len() >= 2, "short RowDescription body");
    let count = u16::from_be_bytes(body[0..2].try_into().unwrap()) as usize;
    let mut offset = 2;
    let mut oids = Vec::with_capacity(count);
    for _ in 0..count {
        let name_len = body[offset..]
            .iter()
            .position(|&byte| byte == 0)
            .expect("RowDescription field name terminator");
        offset += name_len + 1;
        assert!(offset + 18 <= body.len(), "short RowDescription field");
        offset += 6; // table OID + attribute number
        oids.push(u32::from_be_bytes(
            body[offset..offset + 4].try_into().unwrap(),
        ));
        offset += 12; // type OID + type size + type modifier + format
    }
    oids
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
async fn ducklake_compact_table_accepts_layout_bucket_scope() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let catalog_path = tmp.path().join("quackgis.db");
    let server = ServerHandle::start_with_tempdir(tmp).await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE public.compact_bucket_points (
                 id INT,
                 captured_minute INT,
                 geom BINARY,
                 name TEXT
             );
             INSERT INTO public.compact_bucket_points (id, captured_minute, geom, name) VALUES
                (1, 10, X'010100000000000000000000000000000000000000', 'a');
              INSERT INTO public.compact_bucket_points (id, captured_minute, geom, name) VALUES
                (2, 80, X'010100000000000000000000400000000000000040', 'b');
              INSERT INTO public.compact_bucket_points (id, captured_minute, geom, name) VALUES
                (3, 10, X'010100000000000000000010400000000000001040', 'c');",
        )
        .await
        .expect("seed compact bucket target");

    let bucket = client
        .query_one(
            "SELECT _qg_time_bucket, _qg_space_bucket
             FROM quackgis.main.compact_bucket_points
             WHERE id = 1",
            &[],
        )
        .await
        .expect("read layout bucket");
    let time_bucket: i64 = bucket.get(0);
    let space_bucket: i64 = bucket.get(1);

    let before: Vec<(i32, i32, String)> = client
        .query(
            "SELECT id, captured_minute, ST_AsText(ST_GeomFromWKB(geom))
             FROM public.compact_bucket_points
             ORDER BY id",
            &[],
        )
        .await
        .expect("select before bucket compact")
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();

    client
        .batch_execute(&format!(
            "CALL quackgis_compact_table('public.compact_bucket_points', {time_bucket}, {space_bucket})"
        ))
        .await
        .expect("compact layout bucket");

    let after: Vec<(i32, i32, String)> = client
        .query(
            "SELECT id, captured_minute, ST_AsText(ST_GeomFromWKB(geom))
             FROM public.compact_bucket_points
             ORDER BY id",
            &[],
        )
        .await
        .expect("select after bucket compact")
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();

    assert_eq!(after, before);
    assert_eq!(
        after,
        vec![
            (1, 10, "POINT(0 0)".to_string()),
            (2, 80, "POINT(2 2)".to_string()),
            (3, 10, "POINT(4 4)".to_string()),
        ]
    );

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", catalog_path.display()))
        .await
        .expect("catalog pool");
    let metadata = sqlx::query(
        "WITH target_table AS (
             SELECT t.table_id
             FROM ducklake_table t
             JOIN ducklake_schema s ON s.schema_id = t.schema_id
             WHERE s.schema_name = 'main'
               AND t.table_name = 'compact_bucket_points'
               AND t.end_snapshot IS NULL
         ), delete_snapshot AS (
             SELECT DISTINCT df.begin_snapshot AS snapshot_id
             FROM ducklake_delete_file df
             JOIN target_table tt ON tt.table_id = df.table_id
         )
         SELECT
             (SELECT COUNT(*) FROM ducklake_delete_file df
              JOIN target_table tt ON tt.table_id = df.table_id) AS delete_files,
             (SELECT COUNT(DISTINCT df.data_file_id) FROM ducklake_delete_file df
              JOIN target_table tt ON tt.table_id = df.table_id) AS affected_data_files,
             (SELECT COUNT(*) FROM delete_snapshot) AS delete_snapshots,
             (SELECT COUNT(*) FROM ducklake_data_file data
              JOIN target_table tt ON tt.table_id = data.table_id
              WHERE data.begin_snapshot IN (SELECT snapshot_id FROM delete_snapshot)) AS appended_files,
             (SELECT COUNT(*) FROM ducklake_data_file data
              JOIN target_table tt ON tt.table_id = data.table_id
              WHERE data.end_snapshot IN (SELECT snapshot_id FROM delete_snapshot)) AS retired_files",
    )
    .fetch_one(&pool)
    .await
    .expect("bucket compaction metadata");
    let delete_files: i64 = metadata.try_get("delete_files").unwrap();
    let affected_data_files: i64 = metadata.try_get("affected_data_files").unwrap();
    let delete_snapshots: i64 = metadata.try_get("delete_snapshots").unwrap();
    let appended_files: i64 = metadata.try_get("appended_files").unwrap();
    let retired_files: i64 = metadata.try_get("retired_files").unwrap();
    assert_eq!(delete_files, 2, "one delete file per bucket source file");
    assert_eq!(affected_data_files, 2, "bucket rows span two data files");
    assert_eq!(delete_snapshots, 1, "bucket masks share one snapshot");
    assert_eq!(
        appended_files, 1,
        "compacted bucket is one replacement file"
    );
    assert_eq!(
        retired_files, 0,
        "bucket compaction must not retire whole data files"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_native_compact_failpoint_before_commit_leaves_catalog_unchanged() {
    let _serial = NATIVE_MUTATION_FAILPOINT_TEST_LOCK.lock().await;
    let _failpoint = NativeMutationFailpointGuard::set(
        "compact:before_commit:main.native_compact_failpoint_points",
    );
    let aborts_before = ducklake_sql::metrics_snapshot().native_mutation_aborts_total;
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let catalog_path = tmp.path().join("quackgis.db");
    let server = ServerHandle::start_with_tempdir(tmp).await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE public.native_compact_failpoint_points (
                 id INT,
                 captured_minute INT,
                 geom BINARY,
                 name TEXT
             );
             INSERT INTO public.native_compact_failpoint_points (id, captured_minute, geom, name) VALUES
                (1, 10, X'010100000000000000000000000000000000000000', 'a');
             INSERT INTO public.native_compact_failpoint_points (id, captured_minute, geom, name) VALUES
                (2, 10, X'010100000000000000000000000000000000000000', 'b');
             INSERT INTO public.native_compact_failpoint_points (id, captured_minute, geom, name) VALUES
                (3, 80, X'010100000000000000000010400000000000001040', 'c');",
        )
        .await
        .expect("seed native compact failpoint target");

    let bucket = client
        .query_one(
            "SELECT _qg_time_bucket, _qg_space_bucket
             FROM quackgis.main.native_compact_failpoint_points
             WHERE id = 1",
            &[],
        )
        .await
        .expect("read layout bucket");
    let time_bucket: i64 = bucket.get(0);
    let space_bucket: i64 = bucket.get(1);

    let before_rows: Vec<(i32, i32, String)> = client
        .query(
            "SELECT id, captured_minute, name
             FROM public.native_compact_failpoint_points
             ORDER BY id",
            &[],
        )
        .await
        .expect("select before failed compact")
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", catalog_path.display()))
        .await
        .expect("catalog pool");
    let before_counts = catalog_mutation_counts(&pool, "native_compact_failpoint_points").await;

    let failed = client
        .batch_execute(&format!(
            "CALL quackgis_compact_table('public.native_compact_failpoint_points', {time_bucket}, {space_bucket})"
        ))
        .await;
    assert!(
        failed.is_err(),
        "failpoint should abort before commit_table_mutation"
    );
    assert_eq!(
        ducklake_sql::metrics_snapshot().native_mutation_aborts_total,
        aborts_before + 1,
        "native mutation abort counter should increment for the failed compaction"
    );

    let after_rows: Vec<(i32, i32, String)> = client
        .query(
            "SELECT id, captured_minute, name
             FROM public.native_compact_failpoint_points
             ORDER BY id",
            &[],
        )
        .await
        .expect("select after failed compact")
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    assert_eq!(
        after_rows, before_rows,
        "failed native compaction must leave visible rows unchanged"
    );
    let after_counts = catalog_mutation_counts(&pool, "native_compact_failpoint_points").await;
    assert_eq!(
        after_counts, before_counts,
        "pending compacted data/delete objects must not become catalog-visible after failed compaction"
    );
    let info = table_info(&client, "native_compact_failpoint_points").await;
    assert_eq!(info.delete_file_count, 0);

    client
        .batch_execute(&format!(
            "CALL quackgis_compact_table('public.native_compact_failpoint_points', {time_bucket}, {space_bucket})"
        ))
        .await
        .expect("retry native compaction after one-shot failpoint");
    let retry_rows: Vec<(i32, i32, String)> = client
        .query(
            "SELECT id, captured_minute, name
             FROM public.native_compact_failpoint_points
             ORDER BY id",
            &[],
        )
        .await
        .expect("select after retried compact")
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    assert_eq!(
        retry_rows, before_rows,
        "retried native compaction must preserve visible rows"
    );
    let retry_counts = catalog_mutation_counts(&pool, "native_compact_failpoint_points").await;
    assert!(
        retry_counts.delete_files > before_counts.delete_files,
        "retried compaction should publish delete-file metadata: before={before_counts:?} retry={retry_counts:?}"
    );
    assert!(
        retry_counts.data_files > before_counts.data_files,
        "retried compaction should publish replacement data-file metadata: before={before_counts:?} retry={retry_counts:?}"
    );
    assert_eq!(
        ducklake_sql::metrics_snapshot().native_mutation_aborts_total,
        aborts_before + 1,
        "retry after one-shot compact failpoint must not increment abort counter again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_metadata_table_functions_roundtrip_through_wire() {
    let server = ServerHandle::start().await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE public.metadata_points (id INT, geom BINARY, name TEXT);
             INSERT INTO public.metadata_points (id, geom, name) VALUES
                (1, X'010100000000000000000000000000000000000000', 'kept'),
                (2, X'0101000000000000000000F03F000000000000F03F', 'deleted');
             DELETE FROM public.metadata_points WHERE id = 2;",
        )
        .await
        .expect("seed metadata table-function target");

    let max_snapshot: i64 = client
        .query_one("SELECT MAX(snapshot_id) FROM ducklake_snapshots()", &[])
        .await
        .expect("ducklake_snapshots")
        .get(0);
    assert!(max_snapshot >= 3, "unexpected snapshot id {max_snapshot}");

    let info = client
        .query_one(
            "SELECT file_count, delete_file_count
             FROM ducklake_table_info()
             WHERE schema_name = 'main' AND table_name = 'metadata_points'",
            &[],
        )
        .await
        .expect("ducklake_table_info");
    let file_count: i64 = info.get(0);
    let delete_file_count: i64 = info.get(1);
    assert!(
        file_count >= 1,
        "expected visible data files, got {file_count}"
    );
    assert_eq!(delete_file_count, 1, "native delete file is visible");

    let files_with_deletes: i64 = client
        .query_one(
            "SELECT COUNT(*)
             FROM ducklake_list_files()
             WHERE schema_name = 'main'
               AND table_name = 'metadata_points'
               AND has_delete_file",
            &[],
        )
        .await
        .expect("ducklake_list_files")
        .get(0);
    assert_eq!(files_with_deletes, 1);

    let cdc_unregistered = client
        .simple_query(&format!(
            "SELECT * FROM ducklake_table_deletions('main.metadata_points', 0, {max_snapshot})"
        ))
        .await;
    assert!(
        cdc_unregistered.is_err(),
        "CDC row table functions stay disabled until pgwire projection is safe"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_snapshot_selector_reads_pinned_table() {
    let server = ServerHandle::start().await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);
    let metrics_before = ducklake_sql::metrics_snapshot();

    client
        .batch_execute(
            "CREATE TABLE public.snapshot_points (id INT, geom BINARY, name TEXT);
             INSERT INTO public.snapshot_points (id, geom, name) VALUES
                (1, X'010100000000000000000000000000000000000000', 'first');",
        )
        .await
        .expect("seed first snapshot row");
    let snapshot_id: i64 = client
        .query_one("SELECT MAX(snapshot_id) FROM ducklake_snapshots()", &[])
        .await
        .expect("snapshot after first insert")
        .get(0);

    client
        .batch_execute(
            "INSERT INTO public.snapshot_points (id, geom, name) VALUES
                (2, X'0101000000000000000000F03F000000000000F03F', 'second');",
        )
        .await
        .expect("seed second snapshot row");

    let current_ids: Vec<i32> = client
        .query("SELECT id FROM public.snapshot_points ORDER BY id", &[])
        .await
        .expect("current snapshot query")
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(current_ids, vec![1, 2]);

    let asof_ids: Vec<i32> = client
        .query(
            &format!(
                "SELECT id FROM public.snapshot_points(snapshot => {snapshot_id}) ORDER BY id"
            ),
            &[],
        )
        .await
        .expect("snapshot-pinned table query")
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(asof_ids, vec![1]);

    let snapshot_id_alias_ids: Vec<i32> = client
        .query(
            &format!(
                "SELECT id FROM public.snapshot_points(snapshot_id => {snapshot_id}) ORDER BY id"
            ),
            &[],
        )
        .await
        .expect("snapshot-pinned snapshot_id alias table query")
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(snapshot_id_alias_ids, vec![1]);

    let as_of_snapshot_ids: Vec<i32> = client
        .query(
            &format!(
                "SELECT id FROM public.snapshot_points AS OF SNAPSHOT {snapshot_id} ORDER BY id"
            ),
            &[],
        )
        .await
        .expect("AS OF SNAPSHOT table query")
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(as_of_snapshot_ids, vec![1]);

    let snapshot_count_extent = client
        .query_one(
            &format!(
                "SELECT COUNT(*), ST_Extent(geom) FROM public.snapshot_points(snapshot => {snapshot_id})"
            ),
            &[],
        )
        .await
        .expect("snapshot-pinned count and extent query");
    assert_eq!(snapshot_count_extent.get::<_, i64>(0), 1);
    assert_eq!(snapshot_count_extent.get::<_, String>(1), "BOX(0 0,0 0)");

    let current_after_snapshot_ids: Vec<i32> = client
        .query("SELECT id FROM public.snapshot_points ORDER BY id", &[])
        .await
        .expect("current table remains current after snapshot read")
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(current_after_snapshot_ids, vec![1, 2]);

    let missing = client
        .simple_query("SELECT id FROM public.snapshot_points(snapshot => 1)")
        .await;
    assert!(
        missing.is_err(),
        "snapshot read must fail closed when the table is absent at the snapshot"
    );
    let future = client
        .simple_query("SELECT id FROM public.snapshot_points(snapshot => 9223372036854775807)")
        .await;
    assert!(
        future.is_err(),
        "snapshot read must reject unknown future ids even when the table visibility interval is open"
    );

    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite:{}",
        server.tmp_dir().join("quackgis.db").display()
    ))
    .await
    .expect("open snapshot catalog");
    sqlx::query(
        "UPDATE ducklake_snapshot
         SET snapshot_time = CASE
             WHEN snapshot_id <= ? THEN '2026-07-09 12:00:00.000000'
             ELSE '2026-07-09 12:00:01.000000'
         END",
    )
    .bind(snapshot_id)
    .execute(&pool)
    .await
    .expect("set deterministic snapshot timestamps");
    pool.close().await;

    let timestamp_ids: Vec<i32> = client
        .query(
            "SELECT id FROM public.snapshot_points
             AS OF TIMESTAMP '2026-07-09T12:00:00.500000Z'
             ORDER BY id",
            &[],
        )
        .await
        .expect("AS OF TIMESTAMP table query")
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(timestamp_ids, vec![1]);

    let named_timestamp_ids: Vec<i32> = client
        .query(
            "SELECT id FROM public.snapshot_points(
                 snapshot_at => '2026-07-09T12:00:00.500000+00:00'
             ) ORDER BY id",
            &[],
        )
        .await
        .expect("named timestamp selector")
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(named_timestamp_ids, vec![1]);

    assert!(
        client
            .simple_query(
                "SELECT id FROM public.snapshot_points
                 AS OF TIMESTAMP '2026-07-09T11:59:59Z'"
            )
            .await
            .is_err(),
        "timestamp before catalog history must fail closed"
    );
    let metrics_after = ducklake_sql::metrics_snapshot();
    assert!(
        metrics_after.snapshot_reads_total >= metrics_before.snapshot_reads_total + 4,
        "successful snapshot reads should increment metrics: before={metrics_before:?} after={metrics_after:?}"
    );
    assert!(
        metrics_after.snapshot_read_errors_total > metrics_before.snapshot_read_errors_total,
        "failed snapshot reads should increment metrics: before={metrics_before:?} after={metrics_after:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_bucket_compaction_reports_fragmented_file_delta() {
    let server = ServerHandle::start().await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE public.compact_fragment_points (
                 id INT,
                 captured_minute INT,
                 geom BINARY,
                 name TEXT
             );
             INSERT INTO public.compact_fragment_points (id, captured_minute, geom, name) VALUES
                (1, 10, X'010100000000000000000000000000000000000000', 'a');
             INSERT INTO public.compact_fragment_points (id, captured_minute, geom, name) VALUES
                (2, 10, X'010100000000000000000000000000000000000000', 'b');
             INSERT INTO public.compact_fragment_points (id, captured_minute, geom, name) VALUES
                (3, 10, X'010100000000000000000000000000000000000000', 'c');
             INSERT INTO public.compact_fragment_points (id, captured_minute, geom, name) VALUES
                (4, 10, X'010100000000000000000000000000000000000000', 'd');",
        )
        .await
        .expect("seed fragmented compaction target");

    let before = table_info(&client, "compact_fragment_points").await;
    assert!(
        before.file_count >= 4,
        "autocommit inserts fragment files: {before:?}"
    );
    assert_eq!(before.delete_file_count, 0);
    assert!(before.file_size_bytes > 0);

    let bucket = client
        .query_one(
            "SELECT _qg_time_bucket, _qg_space_bucket
             FROM quackgis.main.compact_fragment_points
             WHERE id = 1",
            &[],
        )
        .await
        .expect("read fragmented layout bucket");
    let time_bucket: i64 = bucket.get(0);
    let space_bucket: i64 = bucket.get(1);

    client
        .batch_execute(&format!(
            "CALL quackgis_compact_table('public.compact_fragment_points', {time_bucket}, {space_bucket})"
        ))
        .await
        .expect("compact fragmented layout bucket");

    let rows: Vec<i32> = client
        .query(
            "SELECT id FROM public.compact_fragment_points ORDER BY id",
            &[],
        )
        .await
        .expect("select after fragmented compact")
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(rows, vec![1, 2, 3, 4]);

    let after = table_info(&client, "compact_fragment_points").await;
    assert_eq!(
        after.delete_file_count, 4,
        "one delete file masks each source fragment"
    );
    assert_eq!(
        after.file_count,
        before.file_count + 1,
        "partial bucket compaction appends one replacement while source files stay visible"
    );
    assert!(
        after.file_size_bytes >= before.file_size_bytes,
        "catalog bytes should include replacement file"
    );

    let summary = format!(
        "bucket_compaction_fragmented file_groups={}->{} delete_files={} bytes={}->{}",
        before.file_count,
        after.file_count,
        after.delete_file_count,
        before.file_size_bytes,
        after.file_size_bytes,
    );
    assert!(
        summary.contains(&format!(
            "file_groups={}->{}",
            before.file_count, after.file_count
        )),
        "{summary}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_full_compaction_reports_scan_metric_improvement() {
    let server = ServerHandle::start().await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE public.compact_scan_points (
                 id INT,
                 captured_minute INT,
                 geom BINARY,
                 name TEXT
             );
             INSERT INTO public.compact_scan_points (id, captured_minute, geom, name) VALUES
                (1, 10, X'010100000000000000000000000000000000000000', 'a');
             INSERT INTO public.compact_scan_points (id, captured_minute, geom, name) VALUES
                (2, 20, X'0101000000000000000000F03F000000000000F03F', 'b');
             INSERT INTO public.compact_scan_points (id, captured_minute, geom, name) VALUES
                (3, 30, X'010100000000000000000000400000000000000040', 'c');
             INSERT INTO public.compact_scan_points (id, captured_minute, geom, name) VALUES
                (4, 40, X'010100000000000000000008400000000000000840', 'd');
             INSERT INTO public.compact_scan_points (id, captured_minute, geom, name) VALUES
                (5, 50, X'010100000000000000000010400000000000001040', 'e');
             INSERT INTO public.compact_scan_points (id, captured_minute, geom, name) VALUES
                (6, 60, X'010100000000000000000014400000000000001440', 'f');",
        )
        .await
        .expect("seed fragmented scan target");

    let scan_sql = "SELECT COUNT(*) AS n
         FROM quackgis.main.compact_scan_points
         WHERE _qg_minx <= 6.0 AND _qg_maxx >= -1.0
           AND _qg_miny <= 6.0 AND _qg_maxy >= -1.0
           AND ST_Intersects(
             ST_GeomFromWKB(geom),
             ST_GeomFromWKB(ST_MakeEnvelope(-1.0, -1.0, 6.0, 6.0, 3857)))";

    let before_count: i64 = client
        .query_one(scan_sql, &[])
        .await
        .expect("count before scan compact")
        .get(0);
    assert_eq!(before_count, 6);

    let before_info = table_info(&client, "compact_scan_points").await;
    assert!(
        before_info.file_count >= 6,
        "autocommit inserts should fragment scan target: {before_info:?}"
    );
    let before_scan = explain_scan_metric(&server.conn_str(), scan_sql).await;
    assert!(
        before_scan.file_groups.is_some(),
        "expected scan file-group evidence before compaction: {before_scan:?}"
    );
    assert!(
        before_scan.row_groups_total.unwrap_or(0) >= 6,
        "expected fragmented row-group evidence before compaction: {before_scan:?}"
    );
    assert!(
        before_scan.bytes_scanned.is_some(),
        "expected scan-byte evidence before compaction: {before_scan:?}"
    );

    client
        .batch_execute("CALL quackgis_compact_table('public.compact_scan_points')")
        .await
        .expect("whole-table compact scan target");

    let after_count: i64 = client
        .query_one(scan_sql, &[])
        .await
        .expect("count after scan compact")
        .get(0);
    assert_eq!(after_count, before_count);

    let after_info = table_info(&client, "compact_scan_points").await;
    assert!(
        after_info.file_count < before_info.file_count,
        "whole-table compaction should reduce visible file count: before={before_info:?} after={after_info:?}"
    );
    let after_scan = explain_scan_metric(&server.conn_str(), scan_sql).await;
    assert!(
        after_scan.file_groups <= before_scan.file_groups,
        "compaction should not increase scan file groups: before={before_scan:?} after={after_scan:?}"
    );
    assert!(
        after_scan.bytes_scanned.is_some(),
        "expected scan-byte evidence after compaction: {after_scan:?}"
    );
    assert!(
        after_scan.row_groups_total <= before_scan.row_groups_total,
        "compaction should not increase scanned row groups: before={before_scan:?} after={after_scan:?}"
    );

    println!(
        "compaction_scan file_groups={}->{} bytes_scanned={}->{} row_groups={}->{} visible_files={}->{}",
        fmt_scan(before_scan.file_groups),
        fmt_scan(after_scan.file_groups),
        fmt_scan(before_scan.bytes_scanned),
        fmt_scan(after_scan.bytes_scanned),
        before_scan.row_group_summary(),
        after_scan.row_group_summary(),
        before_info.file_count,
        after_info.file_count,
    );
}

#[derive(Debug, Clone, Copy)]
struct DuckLakeTableInfo {
    file_count: i64,
    file_size_bytes: i64,
    delete_file_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CatalogMutationCounts {
    data_files: i64,
    delete_files: i64,
}

#[derive(Debug, Clone, Copy, Default)]
struct ScanMetric {
    bytes_scanned: Option<u64>,
    file_groups: Option<u64>,
    row_groups_total: Option<u64>,
    row_groups_matched: Option<u64>,
}

struct NativeMutationFailpointGuard;

impl NativeMutationFailpointGuard {
    fn set(spec: &str) -> Self {
        quackgis_server::ducklake_sql::set_native_mutation_failpoint_for_tests(Some(spec))
            .expect("install native mutation failpoint");
        Self
    }
}

impl Drop for NativeMutationFailpointGuard {
    fn drop(&mut self) {
        let _ = quackgis_server::ducklake_sql::set_native_mutation_failpoint_for_tests(None);
    }
}

impl ScanMetric {
    fn row_group_summary(self) -> String {
        match (self.row_groups_total, self.row_groups_matched) {
            (Some(total), Some(matched)) => format!("{matched}/{total}"),
            _ => "NA".to_string(),
        }
    }
}

async fn table_info(client: &tokio_postgres::Client, table_name: &str) -> DuckLakeTableInfo {
    let row = client
        .query_one(
            "SELECT file_count, file_size_bytes, delete_file_count
             FROM ducklake_table_info()
             WHERE schema_name = 'main' AND table_name = $1",
            &[&table_name],
        )
        .await
        .expect("ducklake_table_info row");
    DuckLakeTableInfo {
        file_count: row.get(0),
        file_size_bytes: row.get(1),
        delete_file_count: row.get(2),
    }
}

async fn catalog_mutation_counts(
    pool: &sqlx::SqlitePool,
    table_name: &str,
) -> CatalogMutationCounts {
    let row = sqlx::query(
        "WITH target_table AS (
             SELECT t.table_id
             FROM ducklake_table t
             JOIN ducklake_schema s ON s.schema_id = t.schema_id
             WHERE s.schema_name = 'main'
               AND t.table_name = ?
               AND t.end_snapshot IS NULL
         )
         SELECT
             (SELECT COUNT(*) FROM ducklake_data_file data
              JOIN target_table tt ON tt.table_id = data.table_id) AS data_files,
             (SELECT COUNT(*) FROM ducklake_delete_file df
              JOIN target_table tt ON tt.table_id = df.table_id) AS delete_files",
    )
    .bind(table_name)
    .fetch_one(pool)
    .await
    .expect("catalog mutation counts");
    CatalogMutationCounts {
        data_files: row.try_get("data_files").unwrap(),
        delete_files: row.try_get("delete_files").unwrap(),
    }
}

async fn explain_scan_metric(conn_str: &str, sql: &str) -> ScanMetric {
    let plan = explain_analyze_plan(conn_str, sql).await;
    let metric = scan_metric_from_plan(&plan);
    assert!(
        plan.contains("DataSourceExec"),
        "EXPLAIN ANALYZE did not include a DataSourceExec plan:\n{plan}"
    );
    metric
}

async fn explain_analyze_plan(conn_str: &str, sql: &str) -> String {
    raw_simple_query_text(conn_str, &format!("EXPLAIN ANALYZE {sql}"))
        .await
        .join("\n")
}

async fn raw_simple_query_text(conn_str: &str, sql: &str) -> Vec<String> {
    let (host, port) = parse_conn_addr(conn_str);
    let mut stream = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .expect("raw pgwire connect for simple query");
    send_startup(&mut stream).await;
    read_until_ready(&mut stream).await;

    send_frontend_message(&mut stream, b'Q', cstring_body(sql).as_slice()).await;
    let mut values = Vec::new();
    loop {
        let (typ, body) = read_backend_message(&mut stream).await;
        match typ {
            b'D' => values.extend(parse_text_data_row(&body).into_iter().flatten()),
            b'Z' => break,
            b'E' => panic!("simple query failed: {}", error_message(&body)),
            _ => {}
        }
    }
    values
}

fn parse_text_data_row(body: &[u8]) -> Vec<Option<String>> {
    assert!(body.len() >= 2, "DataRow missing column count");
    let mut pos = 2;
    let columns = i16::from_be_bytes([body[0], body[1]]) as usize;
    let mut values = Vec::with_capacity(columns);
    for _ in 0..columns {
        assert!(pos + 4 <= body.len(), "DataRow missing column length");
        let len = i32::from_be_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]);
        pos += 4;
        if len < 0 {
            values.push(None);
            continue;
        }
        let len = len as usize;
        assert!(
            pos + len <= body.len(),
            "DataRow column length exceeds body"
        );
        values.push(Some(
            String::from_utf8_lossy(&body[pos..pos + len]).into_owned(),
        ));
        pos += len;
    }
    values
}

fn scan_metric_from_plan(plan: &str) -> ScanMetric {
    let mut metric = ScanMetric {
        bytes_scanned: metric_value(plan, "bytes_scanned"),
        file_groups: file_group_count(plan),
        ..ScanMetric::default()
    };
    if let Some((total, matched)) = pruning_pair(plan, "row_groups_pruned_statistics") {
        metric.row_groups_total = Some(total);
        metric.row_groups_matched = Some(matched);
    }
    metric
}

fn metric_value(plan: &str, metric_name: &str) -> Option<u64> {
    let needle = format!("{metric_name}=");
    let start = plan.find(&needle)? + needle.len();
    parse_u64_prefix(&plan[start..])
}

fn file_group_count(plan: &str) -> Option<u64> {
    let start = plan.find("file_groups={")? + "file_groups={".len();
    parse_u64_prefix(&plan[start..])
}

fn pruning_pair(plan: &str, metric_name: &str) -> Option<(u64, u64)> {
    let needle = format!("{metric_name}=");
    let start = plan.find(&needle)? + needle.len();
    let rest = &plan[start..];
    let total = parse_u64_prefix(rest)?;
    let matched_end = rest.find(" matched")?;
    let matched = parse_last_u64(&rest[..matched_end])?;
    Some((total, matched))
}

fn parse_u64_prefix(value: &str) -> Option<u64> {
    let digits = value
        .chars()
        .skip_while(|ch| ch.is_whitespace())
        .take_while(|ch| ch.is_ascii_digit() || *ch == ',')
        .filter(|ch| *ch != ',')
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn parse_last_u64(value: &str) -> Option<u64> {
    let mut end = None;
    for (idx, ch) in value.char_indices().rev() {
        if ch.is_ascii_digit() || ch == ',' {
            end = Some(idx + ch.len_utf8());
            break;
        }
    }
    let end = end?;
    let start = value[..end]
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_ascii_digit() && *ch != ',')
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    value[start..end].replace(',', "").parse().ok()
}

fn fmt_scan(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NA".to_string())
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
async fn ducklake_data_survives_context_reopen_without_advancing_history() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let catalog = tmp.path().join("quackgis.db");
    let data = tmp.path().join("data");
    let paths =
        StoragePaths::new(catalog.to_str().unwrap(), data.to_str().unwrap()).expect("paths");

    write_nums(&paths, "nums", &[1, 2]).await;

    let snapshots_before = paths
        .metadata_provider()
        .await
        .expect("provider before reopen")
        .list_snapshots()
        .expect("snapshots before reopen")
        .len();

    let ctx = build_session_context_with_storage(paths.clone())
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
    drop(ctx);

    let reopened = build_session_context_with_storage(paths.clone())
        .await
        .expect("second read-side context");
    drop(reopened);
    let snapshots_after = paths
        .metadata_provider()
        .await
        .expect("provider after reopen")
        .list_snapshots()
        .expect("snapshots after reopen")
        .len();
    assert_eq!(
        snapshots_after, snapshots_before,
        "opening an initialized catalog must not create bare snapshots"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_local_rollback_to_matched_backup_restores_prior_head() {
    let source = tempfile::TempDir::new().expect("source tempdir");
    let source_catalog = source.path().join("quackgis.db");
    let source_data = source.path().join("data");
    let source_paths = StoragePaths::new(
        source_catalog.to_str().unwrap(),
        source_data.to_str().unwrap(),
    )
    .expect("source paths");
    write_nums(&source_paths, "backup_nums", &[10, 20, 30]).await;
    let release_snapshot = source_paths
        .metadata_provider()
        .await
        .expect("release provider")
        .get_current_snapshot()
        .expect("release snapshot");

    let restored = tempfile::TempDir::new().expect("restored tempdir");
    fs::copy(&source_catalog, restored.path().join("quackgis.db")).expect("copy catalog");
    copy_dir_all(&source_data, &restored.path().join("data"));

    // Advance the source after cutting the matched backup. The restored pair must
    // remain pinned to the recorded release head rather than following this newer
    // source state.
    write_nums(&source_paths, "backup_nums", &[999]).await;
    let newer_snapshot = source_paths
        .metadata_provider()
        .await
        .expect("newer source provider")
        .get_current_snapshot()
        .expect("newer source snapshot");
    assert!(
        newer_snapshot > release_snapshot,
        "source head must advance after the release backup"
    );

    let server = ServerHandle::start_with_tempdir(restored).await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect restored backup");
    let _conn = tokio::spawn(conn);

    let restored_sum: i64 = client
        .query_one("SELECT SUM(id) FROM quackgis.main.backup_nums", &[])
        .await
        .expect("query restored backup")
        .get(0);
    assert_eq!(restored_sum, 60);

    let restored_head: i64 = client
        .query_one("SELECT MAX(snapshot_id) FROM ducklake_snapshots()", &[])
        .await
        .expect("restored snapshot head")
        .get(0);
    assert_eq!(restored_head, release_snapshot);

    let newer_snapshot_count: i64 = client
        .query_one(
            &format!(
                "SELECT COUNT(*) FROM ducklake_snapshots() WHERE snapshot_id = {newer_snapshot}"
            ),
            &[],
        )
        .await
        .expect("newer snapshot must be absent from restore")
        .get(0);
    assert_eq!(newer_snapshot_count, 0);

    let as_of_ids = client
        .simple_query(&format!(
            "SELECT id FROM public.backup_nums AS OF SNAPSHOT {release_snapshot} ORDER BY id"
        ))
        .await
        .expect("simple-protocol AS OF validation on restored release")
        .into_iter()
        .filter_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => row
                .get(0)
                .map(|value| value.parse::<i32>().expect("integer id")),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(as_of_ids, vec![10, 20, 30]);

    let visible_files: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM ducklake_list_files()
             WHERE schema_name = 'main' AND table_name = 'backup_nums'",
            &[],
        )
        .await
        .expect("restored file inventory")
        .get(0);
    assert!(
        visible_files > 0,
        "restored release must reference data files"
    );
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
async fn ducklake_delete_uses_atomic_native_delete_files_across_data_files() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let catalog_path = tmp.path().join("quackgis.db");
    let server = ServerHandle::start_with_tempdir(tmp).await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE public.native_delete_points (id INT, geom BINARY, name TEXT);
             INSERT INTO public.native_delete_points (id, geom, name) VALUES
                (1, X'010100000000000000000000000000000000000000', 'a'),
                (2, X'0101000000000000000000F03F000000000000F03F', 'b');
             INSERT INTO public.native_delete_points (id, geom, name) VALUES
                (3, X'010100000000000000000000400000000000000040', 'c'),
                (4, X'010100000000000000000008400000000000000840', 'd');",
        )
        .await
        .expect("seed native delete target");

    let deleted = client
        .execute(
            "DELETE FROM public.native_delete_points WHERE id = 2 OR id = 4",
            &[],
        )
        .await
        .expect("native DELETE");
    assert_eq!(deleted, 2);

    let ids: Vec<i32> = client
        .query(
            "SELECT id FROM public.native_delete_points ORDER BY id",
            &[],
        )
        .await
        .expect("select survivors")
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(ids, vec![1, 3]);

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", catalog_path.display()))
        .await
        .expect("catalog pool");
    let metadata = sqlx::query(
        "SELECT COUNT(*) AS delete_files,
                COUNT(DISTINCT begin_snapshot) AS delete_snapshots
         FROM ducklake_delete_file",
    )
    .fetch_one(&pool)
    .await
    .expect("delete metadata");
    let delete_files: i64 = metadata.try_get("delete_files").unwrap();
    let delete_snapshots: i64 = metadata.try_get("delete_snapshots").unwrap();
    assert_eq!(delete_files, 2, "one delete file per affected data file");
    assert_eq!(
        delete_snapshots, 1,
        "both delete files must be committed under one snapshot"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_native_delete_failpoint_before_commit_leaves_catalog_unchanged() {
    let _serial = NATIVE_MUTATION_FAILPOINT_TEST_LOCK.lock().await;
    let _failpoint = NativeMutationFailpointGuard::set(
        "delete:before_commit:main.native_delete_failpoint_points",
    );
    let aborts_before = ducklake_sql::metrics_snapshot().native_mutation_aborts_total;
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let catalog_path = tmp.path().join("quackgis.db");
    let server = ServerHandle::start_with_tempdir(tmp).await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE public.native_delete_failpoint_points (id INT, geom BINARY, name TEXT);
             INSERT INTO public.native_delete_failpoint_points (id, geom, name) VALUES
                (1, X'010100000000000000000000000000000000000000', 'a'),
                (2, X'0101000000000000000000F03F000000000000F03F', 'b');
             INSERT INTO public.native_delete_failpoint_points (id, geom, name) VALUES
                (3, X'010100000000000000000000400000000000000040', 'c'),
                (4, X'010100000000000000000008400000000000000840', 'd');",
        )
        .await
        .expect("seed native delete failpoint target");

    let failed = client
        .execute(
            "DELETE FROM public.native_delete_failpoint_points WHERE id = 2 OR id = 4",
            &[],
        )
        .await;
    assert!(
        failed.is_err(),
        "failpoint should abort before commit_table_mutation"
    );
    assert_eq!(
        ducklake_sql::metrics_snapshot().native_mutation_aborts_total,
        aborts_before + 1,
        "native mutation abort counter should increment for the failed delete"
    );

    let rows: Vec<(i32, String)> = client
        .query(
            "SELECT id, name FROM public.native_delete_failpoint_points ORDER BY id",
            &[],
        )
        .await
        .expect("select rows after failed native delete")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        rows,
        vec![
            (1, "a".to_string()),
            (2, "b".to_string()),
            (3, "c".to_string()),
            (4, "d".to_string()),
        ],
        "failed native delete must leave the visible table unchanged"
    );

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", catalog_path.display()))
        .await
        .expect("catalog pool");
    let delete_files: i64 = sqlx::query(
        "SELECT COUNT(*) AS delete_files
         FROM ducklake_delete_file df
         JOIN ducklake_table t ON t.table_id = df.table_id
         JOIN ducklake_schema s ON s.schema_id = t.schema_id
         WHERE s.schema_name = 'main'
           AND t.table_name = 'native_delete_failpoint_points'",
    )
    .fetch_one(&pool)
    .await
    .expect("delete metadata after failed native delete")
    .try_get("delete_files")
    .unwrap();
    assert_eq!(
        delete_files, 0,
        "prewritten delete objects must not become visible catalog delete-file rows"
    );

    let info = table_info(&client, "native_delete_failpoint_points").await;
    assert_eq!(info.delete_file_count, 0);

    let deleted = client
        .execute(
            "DELETE FROM public.native_delete_failpoint_points WHERE id = 2 OR id = 4",
            &[],
        )
        .await
        .expect("retry native DELETE after one-shot failpoint");
    assert_eq!(deleted, 2);
    let retry_rows: Vec<(i32, String)> = client
        .query(
            "SELECT id, name FROM public.native_delete_failpoint_points ORDER BY id",
            &[],
        )
        .await
        .expect("select rows after retried native delete")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        retry_rows,
        vec![(1, "a".to_string()), (3, "c".to_string())],
        "retried native delete should publish exactly the intended mutation"
    );
    let retry_info = table_info(&client, "native_delete_failpoint_points").await;
    assert!(
        retry_info.delete_file_count > 0,
        "retried native delete should publish delete-file metadata: {retry_info:?}"
    );
    assert_eq!(
        ducklake_sql::metrics_snapshot().native_mutation_aborts_total,
        aborts_before + 1,
        "retry after one-shot delete failpoint must not increment abort counter again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_native_update_failpoint_before_commit_leaves_catalog_unchanged() {
    let _serial = NATIVE_MUTATION_FAILPOINT_TEST_LOCK.lock().await;
    let _failpoint = NativeMutationFailpointGuard::set(
        "update:before_commit:main.native_update_failpoint_points",
    );
    let aborts_before = ducklake_sql::metrics_snapshot().native_mutation_aborts_total;
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let catalog_path = tmp.path().join("quackgis.db");
    let server = ServerHandle::start_with_tempdir(tmp).await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE public.native_update_failpoint_points (id INT, geom BINARY, name TEXT);
             INSERT INTO public.native_update_failpoint_points (id, geom, name) VALUES
                (1, X'010100000000000000000000000000000000000000', 'a'),
                (2, X'0101000000000000000000F03F000000000000F03F', 'b');
             INSERT INTO public.native_update_failpoint_points (id, geom, name) VALUES
                (3, X'010100000000000000000000400000000000000040', 'c'),
                (4, X'010100000000000000000008400000000000000840', 'd');",
        )
        .await
        .expect("seed native update failpoint target");

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", catalog_path.display()))
        .await
        .expect("catalog pool");
    let before_counts = catalog_mutation_counts(&pool, "native_update_failpoint_points").await;

    let failed = client
        .execute(
            "UPDATE public.native_update_failpoint_points SET name = 'updated' WHERE id = 2 OR id = 4",
            &[],
        )
        .await;
    assert!(
        failed.is_err(),
        "failpoint should abort before commit_table_mutation"
    );
    assert_eq!(
        ducklake_sql::metrics_snapshot().native_mutation_aborts_total,
        aborts_before + 1,
        "native mutation abort counter should increment for the failed update"
    );

    let rows: Vec<(i32, String)> = client
        .query(
            "SELECT id, name FROM public.native_update_failpoint_points ORDER BY id",
            &[],
        )
        .await
        .expect("select rows after failed native update")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        rows,
        vec![
            (1, "a".to_string()),
            (2, "b".to_string()),
            (3, "c".to_string()),
            (4, "d".to_string()),
        ],
        "failed native update must leave the visible table unchanged"
    );
    let after_counts = catalog_mutation_counts(&pool, "native_update_failpoint_points").await;
    assert_eq!(
        after_counts, before_counts,
        "pending replacement/delete objects must not become catalog-visible after failed update"
    );
    let info = table_info(&client, "native_update_failpoint_points").await;
    assert_eq!(info.delete_file_count, 0);

    let updated = client
        .execute(
            "UPDATE public.native_update_failpoint_points SET name = 'updated' WHERE id = 2 OR id = 4",
            &[],
        )
        .await
        .expect("retry native UPDATE after one-shot failpoint");
    assert_eq!(updated, 2);
    let retry_rows: Vec<(i32, String)> = client
        .query(
            "SELECT id, name FROM public.native_update_failpoint_points ORDER BY id",
            &[],
        )
        .await
        .expect("select rows after retried native update")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        retry_rows,
        vec![
            (1, "a".to_string()),
            (2, "updated".to_string()),
            (3, "c".to_string()),
            (4, "updated".to_string()),
        ],
        "retried native update should publish exactly the intended mutation"
    );
    let retry_counts = catalog_mutation_counts(&pool, "native_update_failpoint_points").await;
    assert!(
        retry_counts.delete_files > before_counts.delete_files,
        "retried update should publish delete-file metadata: before={before_counts:?} retry={retry_counts:?}"
    );
    assert!(
        retry_counts.data_files > before_counts.data_files,
        "retried update should publish replacement data-file metadata: before={before_counts:?} retry={retry_counts:?}"
    );
    assert_eq!(
        ducklake_sql::metrics_snapshot().native_mutation_aborts_total,
        aborts_before + 1,
        "retry after one-shot update failpoint must not increment abort counter again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ducklake_update_uses_atomic_native_delete_and_append() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let catalog_path = tmp.path().join("quackgis.db");
    let server = ServerHandle::start_with_tempdir(tmp).await;
    let (client, conn) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(conn);

    client
        .batch_execute(
            "CREATE TABLE public.native_update_points (id INT, geom BINARY, name TEXT);
             INSERT INTO public.native_update_points (id, geom, name) VALUES
                (1, X'010100000000000000000000000000000000000000', 'a'),
                (2, X'0101000000000000000000F03F000000000000F03F', 'b');
             INSERT INTO public.native_update_points (id, geom, name) VALUES
                (3, X'010100000000000000000000400000000000000040', 'c'),
                (4, X'010100000000000000000008400000000000000840', 'd');",
        )
        .await
        .expect("seed native update target");

    let updated = client
        .execute(
            "UPDATE public.native_update_points SET name = 'updated' WHERE id = 2 OR id = 4",
            &[],
        )
        .await
        .expect("native UPDATE");
    assert_eq!(updated, 2);

    let rows: Vec<(i32, String)> = client
        .query(
            "SELECT id, name FROM public.native_update_points ORDER BY id",
            &[],
        )
        .await
        .expect("select updated rows")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        rows,
        vec![
            (1, "a".to_string()),
            (2, "updated".to_string()),
            (3, "c".to_string()),
            (4, "updated".to_string()),
        ]
    );

    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", catalog_path.display()))
        .await
        .expect("catalog pool");
    let metadata = sqlx::query(
        "WITH delete_snapshot AS (
             SELECT begin_snapshot AS snapshot_id FROM ducklake_delete_file GROUP BY begin_snapshot
         )
         SELECT
             (SELECT COUNT(*) FROM ducklake_delete_file) AS delete_files,
             (SELECT COUNT(*) FROM delete_snapshot) AS delete_snapshots,
             (SELECT COUNT(*) FROM ducklake_data_file
              WHERE begin_snapshot = (SELECT snapshot_id FROM delete_snapshot)) AS appended_files",
    )
    .fetch_one(&pool)
    .await
    .expect("update metadata");
    let delete_files: i64 = metadata.try_get("delete_files").unwrap();
    let delete_snapshots: i64 = metadata.try_get("delete_snapshots").unwrap();
    let appended_files: i64 = metadata.try_get("appended_files").unwrap();
    assert_eq!(
        delete_files, 2,
        "one delete file per affected old data file"
    );
    assert_eq!(delete_snapshots, 1, "old-row masks share one snapshot");
    assert_eq!(
        appended_files, 1,
        "replacement rows are appended in the same mutation snapshot"
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
