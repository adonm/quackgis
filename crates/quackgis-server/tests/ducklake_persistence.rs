// SPDX-License-Identifier: Apache-2.0
//! DuckLake storage roundtrip + restart persistence (M1 contract).
//!
//! datafusion-ducklake main is used to target the official DuckLake 1.0+
//! format. SQL CTAS/DDL through DataFusion currently no-ops / fails to expose
//! newly-created tables in our integration path, so the M1 storage gate uses
//! the upstream-supported writer API (`DuckLakeTableWriter`). SQL DDL mapping
//! is tracked as follow-up M1/M2 compatibility work.

mod common;

use std::sync::Arc;

use datafusion::arrow::array::{BinaryArray, Int32Array};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion_ducklake::{DuckLakeTableWriter, MetadataWriter, SqliteMetadataWriter};
use object_store::local::LocalFileSystem;
use tokio_postgres::NoTls;

use common::ServerHandle;
use quackgis_server::context::{build_session_context_with_storage, StoragePaths};

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    // OGC WKB, little endian, Point type=1, x, y.
    let mut out = Vec::with_capacity(1 + 4 + 8 + 8);
    out.push(1); // little endian
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&y.to_le_bytes());
    out
}

async fn write_nums(paths: &StoragePaths, table: &str, values: &[i32]) {
    let writer = Arc::new(
        SqliteMetadataWriter::new_with_init(&paths.catalog_conn)
            .await
            .expect("writer"),
    );
    writer
        .set_data_path(&paths.data_path)
        .expect("set data path");
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
    let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(LocalFileSystem::new());
    DuckLakeTableWriter::new(writer, object_store)
        .expect("table writer")
        .write_table("main", table, &[batch])
        .await
        .expect("write table");
}

async fn write_geo(paths: &StoragePaths) {
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
    let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(LocalFileSystem::new());
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
