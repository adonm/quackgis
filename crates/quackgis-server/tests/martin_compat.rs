// SPDX-License-Identifier: Apache-2.0
//! Martin tile-server compatibility SQL regression tests.
//!
//! These tests exercise the exact core SQL shape Martin generates for PostGIS
//! table sources. The real Martin binary E2E lives in `martin_real_e2e.rs` and
//! is ignored by default because it needs an external Martin binary.

mod common;

use common::ServerHandle;
use tokio_postgres::NoTls;

fn read_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

fn read_f64(buf: &[u8], off: usize) -> f64 {
    f64::from_le_bytes(buf[off..off + 8].try_into().unwrap())
}

fn rect_bbox(wkb: &[u8]) -> (f64, f64, f64, f64) {
    assert_eq!(
        wkb[0], 1,
        "Sedona WKB helper should write little endian WKB"
    );
    assert_eq!(read_u32(wkb, 1), 3, "expected Polygon WKB");
    assert_eq!(read_u32(wkb, 5), 1, "expected one polygon ring");
    let points = read_u32(wkb, 9) as usize;
    let mut xs = Vec::with_capacity(points);
    let mut ys = Vec::with_capacity(points);
    let mut off = 13;
    for _ in 0..points {
        xs.push(read_f64(wkb, off));
        ys.push(read_f64(wkb, off + 8));
        off += 16;
    }
    (
        xs.iter().copied().fold(f64::INFINITY, f64::min),
        ys.iter().copied().fold(f64::INFINITY, f64::min),
        xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    )
}

fn assert_bbox_approx(actual: (f64, f64, f64, f64), expected: (f64, f64, f64, f64)) {
    let eps = 1e-9;
    assert!((actual.0 - expected.0).abs() < eps, "min_x {actual:?}");
    assert!((actual.1 - expected.1).abs() < eps, "min_y {actual:?}");
    assert!((actual.2 - expected.2).abs() < eps, "max_x {actual:?}");
    assert!((actual.3 - expected.3).abs() < eps, "max_y {actual:?}");
}

async fn setup_martin_test_table(client: &tokio_postgres::Client) {
    client
        .simple_query("CREATE TABLE quackgis.main.points (id INT, geom BINARY, name TEXT)")
        .await
        .expect("CREATE TABLE");

    // POINT(0 0), little-endian WKB. Geometry column naming is intentional:
    // Martin discovers conventional names like `geom` via geometry_columns.
    client
        .simple_query(
            "INSERT INTO quackgis.main.points VALUES \
             (1, X'010100000000000000000000000000000000000000', 'origin')",
        )
        .await
        .expect("INSERT");
}

async fn connect_with_table() -> (ServerHandle, tokio_postgres::Client) {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    tokio::spawn(connection);
    setup_martin_test_table(&client).await;
    (server, client)
}

#[tokio::test(flavor = "multi_thread")]
async fn geometry_columns_survives_insert_refresh() {
    let (_server, client) = connect_with_table().await;

    let rows = client
        .query(
            "SELECT f_table_schema, f_table_name, f_geometry_column, srid, type \
             FROM geometry_columns WHERE f_table_name = 'points'",
            &[],
        )
        .await
        .expect("geometry_columns query");

    assert_eq!(rows.len(), 1, "points.geom should be discoverable");
    assert_eq!(rows[0].get::<_, String>(0), "main");
    assert_eq!(rows[0].get::<_, String>(1), "points");
    assert_eq!(rows[0].get::<_, String>(2), "geom");
    assert_eq!(rows[0].get::<_, i32>(3), 0);
    assert_eq!(rows[0].get::<_, String>(4), "GEOMETRY");
}

#[tokio::test(flavor = "multi_thread")]
async fn martin_bbox_filter_matches_points() {
    let (_server, client) = connect_with_table().await;

    let envelope: Vec<u8> = client
        .query_one("SELECT ST_TileEnvelope(0, 0, 0)", &[])
        .await
        .expect("ST_TileEnvelope")
        .get(0);
    assert!(!envelope.is_empty());

    let count: i64 = client
        .query_one(
            "SELECT count(*) FROM quackgis.main.points \
             WHERE geom && ST_TileEnvelope(0, 0, 0)",
            &[],
        )
        .await
        .expect("bbox overlap")
        .get(0);
    assert_eq!(count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn st_tileenvelope_supports_postgis_bounds_and_margin() {
    let (_server, client) = connect_with_table().await;

    let custom_bounds: Vec<u8> = client
        .query_one(
            "SELECT ST_TileEnvelope(0, 0, 0, ST_MakeEnvelope(0, 0, 100, 100, 3857))",
            &[],
        )
        .await
        .expect("ST_TileEnvelope custom bounds")
        .get(0);
    assert_bbox_approx(rect_bbox(&custom_bounds), (0.0, 0.0, 100.0, 100.0));

    let tile_with_margin: Vec<u8> = client
        .query_one(
            "SELECT ST_TileEnvelope(1, 1, 1, ST_MakeEnvelope(0, 0, 100, 100, 3857), 0.1)",
            &[],
        )
        .await
        .expect("ST_TileEnvelope custom bounds and margin")
        .get(0);
    assert_bbox_approx(rect_bbox(&tile_with_margin), (45.0, -5.0, 105.0, 55.0));

    let margin_only: Vec<u8> = client
        .query_one("SELECT ST_TileEnvelope(0, 0, 0, 0.1)", &[])
        .await
        .expect("ST_TileEnvelope margin convenience overload")
        .get(0);
    assert!(!margin_only.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn martin_mvt_geom_inner_query_executes() {
    let (_server, client) = connect_with_table().await;

    let rows = client
        .query(
            "SELECT ST_AsMVTGeom(\
                ST_Transform(ST_CurveToLine(geom::geometry), 3857),\
                ST_TileEnvelope(0, 0, 0),\
                4096, 64, true\
             ) AS geom \
             FROM quackgis.main.points \
             WHERE geom && ST_TileEnvelope(0, 0, 0)",
            &[],
        )
        .await
        .expect("Martin inner MVT geometry query");

    assert_eq!(rows.len(), 1);
    let geom: Vec<u8> = rows[0].get(0);
    assert!(!geom.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn martin_record_form_st_asmvt_query_executes() {
    let (_server, client) = connect_with_table().await;

    // This is the core shape from Martin's Postgres table source:
    // ST_AsMVT(tile, layer_name, extent, geom_column_name) where `tile` is the
    // derived-table row variable. The datafusion-postgres fork rewrites it to
    // the geometry-only aggregate QuackGIS currently implements and lowers
    // Martin's named ST_TileEnvelope margin argument to a positional overload.
    let tile: Vec<u8> = client
        .query_one(
            "SELECT ST_AsMVT(tile, 'points', 4096, 'geom') FROM (\
                SELECT ST_AsMVTGeom(\
                    ST_Transform(ST_CurveToLine(geom::geometry), 3857),\
                    ST_TileEnvelope(0, 0, 0),\
                    4096, 64, true\
                ) AS geom \
                FROM quackgis.main.points \
                WHERE geom && ST_TileEnvelope(0, 0, 0, margin => 0.015625)\
             ) AS tile",
            &[],
        )
        .await
        .expect("Martin ST_AsMVT record-form query")
        .get(0);

    assert!(!tile.is_empty(), "MVT tile should not be empty");
}
