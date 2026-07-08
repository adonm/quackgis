// SPDX-License-Identifier: Apache-2.0
//! LayoutBench sf0: deterministic layout/pruning oracle seed.
//!
//! This is intentionally small enough for the fast local gate. It creates the
//! same table families planned for the larger LayoutBench scales and verifies
//! that bbox-prefiltered query shapes return the same counts as exact SedonaDB
//! predicates. Future hidden layout columns and planner rewrites should preserve
//! these oracle counts while reducing scanned files/row groups.

mod common;

use common::ServerHandle;
use tokio_postgres::NoTls;

#[derive(Debug, Clone, Copy)]
struct Rect {
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64,
}

impl Rect {
    fn intersects(self, other: Rect) -> bool {
        self.minx <= other.maxx
            && self.maxx >= other.minx
            && self.miny <= other.maxy
            && self.maxy >= other.miny
    }
}

#[derive(Debug)]
struct AerialFrame {
    id: i32,
    mission: &'static str,
    strip: i32,
    captured_minute: i32,
    gsd_cm: f64,
    altitude_m: f64,
    footprint: Rect,
}

#[derive(Debug)]
struct CadObject {
    id: i32,
    floor: i32,
    object_type: &'static str,
    source_object_id: String,
    z_min: f64,
    z_max: f64,
    tolerance_mm: f64,
    geom: Rect,
}

#[derive(Debug)]
struct AssetRow {
    id: i32,
    asset_type: &'static str,
    uri: String,
    acquired_minute: i32,
    resolution_cm: f64,
    horizontal_accuracy_cm: f64,
    footprint: Rect,
}

#[derive(Debug)]
struct ControlPoint {
    id: i32,
    point_name: String,
    acquisition_epoch: f64,
    source_x: f64,
    source_y: f64,
    corrected_x: f64,
    corrected_y: f64,
    residual_mm: f64,
}

#[tokio::test(flavor = "multi_thread")]
async fn layoutbench_sf0_oracle_counts_are_stable() {
    let (_server, client) = connect().await;
    let seed = LayoutBenchSf0::generate();
    seed.load(&client).await;
    update_control_point(&client, 1, 500_123.5, 4_100_456.25).await;
    update_control_point_transactional(&client, 2, 500_234.5, 4_100_567.25).await;
    assert_public_layout_columns_are_hidden(&client, "layoutbench_aerial_frames").await;
    assert_projected_bbox_matches_sidecars(
        &client,
        "quackgis.main.layoutbench_aerial_frames",
        seed.aerial_frames.len() as i64,
    )
    .await;
    assert_projected_bbox_matches_sidecars(
        &client,
        "quackgis.main.layoutbench_cad_objects",
        seed.cad_objects.len() as i64,
    )
    .await;
    assert_projected_bbox_matches_sidecars(
        &client,
        "quackgis.main.layoutbench_assets",
        seed.assets.len() as i64,
    )
    .await;
    assert_projected_point_layout(
        &client,
        "quackgis.main.layoutbench_control_points",
        seed.control_points.len() as i64,
    )
    .await;
    assert_public_exact_query_is_rewritten(&client).await;
    assert_alias_and_tile_query_is_rewritten(&client).await;
    assert_derived_tile_query_is_rewritten(&client).await;
    assert_quoted_identifier_query_is_rewritten(&client).await;
    assert_client_supplied_layout_values_are_ignored(&client).await;
    assert_public_wildcard_spatial_query_stays_hidden(&client).await;
    assert_explain_shows_injected_layout_predicate(&client).await;

    let aerial_window = Rect {
        minx: 95.0,
        miny: 95.0,
        maxx: 290.0,
        maxy: 185.0,
    };
    let aerial_expected = seed
        .aerial_frames
        .iter()
        .filter(|row| {
            row.captured_minute >= 40
                && row.captured_minute <= 170
                && row.footprint.intersects(aerial_window)
        })
        .count() as i64;
    assert_exact_and_prefilter_count(
        &client,
        "quackgis.main.layoutbench_aerial_frames",
        "geom",
        aerial_window,
        "captured_minute BETWEEN 40 AND 170",
        aerial_expected,
    )
    .await;

    let cad_viewport = Rect {
        minx: 1_000_020.0,
        miny: 2_000_010.0,
        maxx: 1_000_075.0,
        maxy: 2_000_055.0,
    };
    let cad_expected = seed
        .cad_objects
        .iter()
        .filter(|row| row.floor == 2 && row.geom.intersects(cad_viewport))
        .count() as i64;
    assert_exact_and_prefilter_count(
        &client,
        "quackgis.main.layoutbench_cad_objects",
        "geom",
        cad_viewport,
        "floor = 2",
        cad_expected,
    )
    .await;

    let asset_window = Rect {
        minx: 150.0,
        miny: 90.0,
        maxx: 430.0,
        maxy: 255.0,
    };
    let asset_expected = seed
        .assets
        .iter()
        .filter(|row| {
            row.resolution_cm <= 15.0
                && row.horizontal_accuracy_cm <= 7.0
                && row.footprint.intersects(asset_window)
        })
        .count() as i64;
    assert_exact_and_prefilter_count(
        &client,
        "quackgis.main.layoutbench_assets",
        "footprint",
        asset_window,
        "resolution_cm <= 15.0 AND horizontal_accuracy_cm <= 7.0",
        asset_expected,
    )
    .await;

    let residual_expected = seed
        .control_points
        .iter()
        .filter(|row| row.acquisition_epoch >= 2024.0 && row.residual_mm <= 4.0)
        .count() as i64;
    let residual_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM layoutbench_control_points \
             WHERE acquisition_epoch >= 2024.0 AND residual_mm <= 4.0",
            &[],
        )
        .await
        .expect("control-point residual query")
        .get(0);
    assert_eq!(residual_count, residual_expected);

    let pruning_summary = assert_pruning_metrics_are_stable(
        &client,
        &[
            PruningCase {
                label: "aerial",
                table: "quackgis.main.layoutbench_aerial_frames",
                geom_column: "geom",
                envelope: aerial_window,
                extra_predicate: "captured_minute BETWEEN 40 AND 170",
                expected: PruningMetric {
                    total: seed.aerial_frames.len() as i64,
                    base: 30,
                    candidate: aerial_expected,
                    exact: aerial_expected,
                },
            },
            PruningCase {
                label: "cad",
                table: "quackgis.main.layoutbench_cad_objects",
                geom_column: "geom",
                envelope: cad_viewport,
                extra_predicate: "floor = 2",
                expected: PruningMetric {
                    total: seed.cad_objects.len() as i64,
                    base: 24,
                    candidate: cad_expected,
                    exact: cad_expected,
                },
            },
            PruningCase {
                label: "assets",
                table: "quackgis.main.layoutbench_assets",
                geom_column: "footprint",
                envelope: asset_window,
                extra_predicate: "resolution_cm <= 15.0 AND horizontal_accuracy_cm <= 7.0",
                expected: PruningMetric {
                    total: seed.assets.len() as i64,
                    base: 20,
                    candidate: asset_expected,
                    exact: asset_expected,
                },
            },
            PruningCase {
                label: "false_positive",
                table: "quackgis.main.layoutbench_false_positives",
                geom_column: "geom",
                envelope: Rect {
                    minx: 45.0,
                    miny: 45.0,
                    maxx: 55.0,
                    maxy: 55.0,
                },
                extra_predicate: "id >= 1",
                expected: PruningMetric {
                    total: 3,
                    base: 3,
                    candidate: 2,
                    exact: 1,
                },
            },
        ],
    )
    .await;
    assert_eq!(
        pruning_summary,
        "layoutbench_sf0_pruning aerial=108/30/18/18 cad=96/24/12/12 assets=24/20/18/18 false_positive=3/3/2/1"
    );

    let summary = format!(
        "layoutbench_sf0 aerial={aerial_expected} cad={cad_expected} \
         assets={asset_expected} control={residual_expected}"
    );
    assert_eq!(
        summary,
        "layoutbench_sf0 aerial=18 cad=12 assets=18 control=7"
    );
}

async fn connect() -> (ServerHandle, tokio_postgres::Client) {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    tokio::spawn(connection);
    (server, client)
}

async fn assert_exact_and_prefilter_count(
    client: &tokio_postgres::Client,
    table: &str,
    geom_column: &str,
    envelope: Rect,
    extra_predicate: &str,
    expected: i64,
) {
    let envelope_sql = format!("ST_GeomFromWKB({})", envelope_wkb_sql(envelope));
    let exact_sql = format!(
        "SELECT COUNT(*) FROM {table} \
         WHERE {extra_predicate} AND ST_Intersects(ST_GeomFromWKB({geom_column}), {envelope_sql})"
    );
    let prefilter_sql = format!(
        "SELECT COUNT(*) FROM {table} \
         WHERE {extra_predicate} \
           AND _qg_minx <= {maxx} AND _qg_maxx >= {minx} \
           AND _qg_miny <= {maxy} AND _qg_maxy >= {miny} \
           AND ST_Intersects(ST_GeomFromWKB({geom_column}), {envelope_sql})",
        minx = envelope.minx,
        miny = envelope.miny,
        maxx = envelope.maxx,
        maxy = envelope.maxy,
    );
    let exact: i64 = client
        .query_one(&exact_sql, &[])
        .await
        .expect("exact count")
        .get(0);
    let prefiltered: i64 = client
        .query_one(&prefilter_sql, &[])
        .await
        .expect("prefiltered count")
        .get(0);
    assert_eq!(exact, expected, "exact count for {table}");
    assert_eq!(prefiltered, exact, "prefiltered count for {table}");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PruningMetric {
    /// All rows in the table.
    total: i64,
    /// Rows after non-spatial predicates only.
    base: i64,
    /// Rows after non-spatial predicates plus hidden bbox predicates, before
    /// exact spatial predicate evaluation.
    candidate: i64,
    /// Rows after exact SedonaDB spatial predicate evaluation.
    exact: i64,
}

#[derive(Debug, Clone, Copy)]
struct PruningCase<'a> {
    label: &'a str,
    table: &'a str,
    geom_column: &'a str,
    envelope: Rect,
    extra_predicate: &'a str,
    expected: PruningMetric,
}

async fn assert_pruning_metrics_are_stable(
    client: &tokio_postgres::Client,
    cases: &[PruningCase<'_>],
) -> String {
    let mut parts = vec!["layoutbench_sf0_pruning".to_string()];
    for case in cases {
        let metric = pruning_metric(client, case).await;
        assert_eq!(metric, case.expected, "pruning metric for {}", case.label);
        assert!(
            metric.exact <= metric.candidate,
            "exact rows must be a subset of bbox candidates for {}: {metric:?}",
            case.label
        );
        assert!(
            metric.candidate <= metric.base,
            "bbox candidates must not exceed base predicate rows for {}: {metric:?}",
            case.label
        );
        assert!(
            metric.base <= metric.total,
            "base predicate rows must not exceed table rows for {}: {metric:?}",
            case.label
        );
        assert!(
            metric.candidate < metric.base,
            "sf0 pruning should reduce candidates for {}: {metric:?}",
            case.label
        );
        parts.push(format!(
            "{}={}/{}/{}/{}",
            case.label, metric.total, metric.base, metric.candidate, metric.exact
        ));
    }
    parts.join(" ")
}

async fn pruning_metric(client: &tokio_postgres::Client, case: &PruningCase<'_>) -> PruningMetric {
    let envelope_sql = format!("ST_GeomFromWKB({})", envelope_wkb_sql(case.envelope));
    let total_sql = format!("SELECT COUNT(*) FROM {}", case.table);
    let base_sql = format!(
        "SELECT COUNT(*) FROM {} WHERE {}",
        case.table, case.extra_predicate
    );
    let candidate_sql = format!(
        "SELECT COUNT(*) FROM {table} \
         WHERE {extra_predicate} \
           AND _qg_minx <= {maxx} AND _qg_maxx >= {minx} \
           AND _qg_miny <= {maxy} AND _qg_maxy >= {miny}",
        table = case.table,
        extra_predicate = case.extra_predicate,
        minx = case.envelope.minx,
        miny = case.envelope.miny,
        maxx = case.envelope.maxx,
        maxy = case.envelope.maxy,
    );
    let exact_sql = format!(
        "SELECT COUNT(*) FROM {table} \
         WHERE {extra_predicate} \
           AND ST_Intersects(ST_GeomFromWKB({geom_column}), {envelope_sql})",
        table = case.table,
        extra_predicate = case.extra_predicate,
        geom_column = case.geom_column,
    );

    PruningMetric {
        total: query_count(client, &total_sql).await,
        base: query_count(client, &base_sql).await,
        candidate: query_count(client, &candidate_sql).await,
        exact: query_count(client, &exact_sql).await,
    }
}

async fn query_count(client: &tokio_postgres::Client, sql: &str) -> i64 {
    client
        .query_one(sql, &[])
        .await
        .unwrap_or_else(|err| panic!("count query failed: {sql}\n{err}"))
        .get(0)
}

async fn assert_public_layout_columns_are_hidden(client: &tokio_postgres::Client, table: &str) {
    let messages = client
        .simple_query(&format!("SELECT * FROM {table} LIMIT 1"))
        .await
        .expect("public SELECT *");
    let row = messages
        .iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some(row),
            _ => None,
        })
        .expect("public SELECT * row");
    let column_names = row
        .columns()
        .iter()
        .map(|column| column.name().to_string())
        .collect::<Vec<_>>();
    assert_eq!(row.get("id"), Some("1"));
    assert!(
        !column_names.iter().any(|name| name.starts_with("_qg_")),
        "public SELECT * should hide layout columns, got {column_names:?}"
    );
    let public_count: i64 = client
        .query_one(&format!("SELECT COUNT(*) FROM {table}"), &[])
        .await
        .expect("extended public count")
        .get(0);
    assert!(public_count > 0, "extended public alias should read rows");

    let metadata_count: i64 = client
        .query_one(
            &format!(
                "SELECT COUNT(*) FROM information_schema.columns \
                 WHERE table_schema = 'public' AND table_name = '{}' \
                   AND column_name IN (\
                     '_qg_minx', '_qg_miny', '_qg_maxx', '_qg_maxy', \
                     '_qg_time_bucket', '_qg_space_bucket', '_qg_space_sort')",
                escape_sql(table)
            ),
            &[],
        )
        .await
        .expect("information_schema hidden layout check")
        .get(0);
    assert_eq!(metadata_count, 0, "metadata should hide layout columns");
}

async fn assert_public_exact_query_is_rewritten(client: &tokio_postgres::Client) {
    let public_sql = "SELECT COUNT(*) AS n FROM layoutbench_aerial_frames \
        WHERE captured_minute BETWEEN 40 AND 170 \
          AND ST_Intersects(\
            ST_GeomFromWKB(geom), \
            ST_GeomFromWKB(ST_MakeEnvelope(95, 95, 290, 185, 3857)))";
    let public_exact: i64 = client
        .query_one(public_sql, &[])
        .await
        .expect("extended public exact query with internal pruning rewrite")
        .get(0);
    let simple_public_exact = client
        .simple_query(public_sql)
        .await
        .expect("simple public exact query with internal pruning rewrite")
        .iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => row.get("n"),
            _ => None,
        })
        .expect("simple public exact row")
        .parse::<i64>()
        .expect("simple public exact count");
    let internal_prefiltered: i64 = client
        .query_one(
            "SELECT COUNT(*) AS n FROM quackgis.main.layoutbench_aerial_frames \
             WHERE captured_minute BETWEEN 40 AND 170 \
               AND _qg_minx <= 290 AND _qg_maxx >= 95 \
               AND _qg_miny <= 185 AND _qg_maxy >= 95 \
               AND ST_Intersects(\
                 ST_GeomFromWKB(geom), \
                 ST_GeomFromWKB(ST_MakeEnvelope(95, 95, 290, 185, 3857)))",
            &[],
        )
        .await
        .expect("internal prefiltered query")
        .get(0);
    assert_eq!(public_exact, internal_prefiltered);
    assert_eq!(simple_public_exact, internal_prefiltered);
}

async fn assert_alias_and_tile_query_is_rewritten(client: &tokio_postgres::Client) {
    let public_sql = "SELECT COUNT(*) AS n FROM layoutbench_aerial_frames AS f \
        WHERE f.geom && ST_TileEnvelope(0, 0, 0, ST_MakeEnvelope(0, 0, 500, 500, 3857))";
    let public_count: i64 = client
        .query_one(public_sql, &[])
        .await
        .expect("alias/tile query with internal pruning rewrite")
        .get(0);
    let internal_prefiltered: i64 = client
        .query_one(
            "SELECT COUNT(*) AS n FROM quackgis.main.layoutbench_aerial_frames AS f \
             WHERE f._qg_minx <= 500 AND f._qg_maxx >= 0 \
               AND f._qg_miny <= 500 AND f._qg_maxy >= 0 \
               AND f.geom && ST_TileEnvelope(0, 0, 0, ST_MakeEnvelope(0, 0, 500, 500, 3857))",
            &[],
        )
        .await
        .expect("internal alias/tile prefiltered query")
        .get(0);
    assert_eq!(public_count, internal_prefiltered);
}

async fn assert_derived_tile_query_is_rewritten(client: &tokio_postgres::Client) {
    let public_sql = "SELECT COUNT(*) AS n FROM (\
            SELECT id, captured_minute FROM layoutbench_aerial_frames AS f \
            WHERE f.geom && ST_TileEnvelope(0, 0, 0, ST_MakeEnvelope(0, 0, 500, 500, 3857)) \
            ORDER BY id\
        ) AS q";
    let public_count: i64 = client
        .query_one(public_sql, &[])
        .await
        .expect("derived tile query with internal pruning rewrite")
        .get(0);
    let internal_prefiltered: i64 = client
        .query_one(
            "SELECT COUNT(*) AS n FROM (\
                 SELECT id, captured_minute FROM quackgis.main.layoutbench_aerial_frames AS f \
                 WHERE f._qg_minx <= 500 AND f._qg_maxx >= 0 \
                   AND f._qg_miny <= 500 AND f._qg_maxy >= 0 \
                   AND f.geom && ST_TileEnvelope(0, 0, 0, ST_MakeEnvelope(0, 0, 500, 500, 3857)) \
                 ORDER BY id\
             ) AS q",
            &[],
        )
        .await
        .expect("internal derived tile prefiltered query")
        .get(0);
    assert_eq!(public_count, internal_prefiltered);
}

async fn assert_quoted_identifier_query_is_rewritten(client: &tokio_postgres::Client) {
    let public_count: i64 = client
        .query_one(
            "SELECT COUNT(*) AS n FROM \"layoutbench quoted\" AS q \
             WHERE ST_Intersects(\
               ST_GeomFromWKB(q.\"Geom\"), \
               ST_GeomFromWKB(ST_MakeEnvelope(8, 8, 12, 12, 3857)))",
            &[],
        )
        .await
        .expect("quoted identifier query with pruning rewrite")
        .get(0);
    let internal_prefiltered: i64 = client
        .query_one(
            "SELECT COUNT(*) AS n FROM quackgis.main.\"layoutbench quoted\" AS q \
             WHERE q._qg_minx <= 12 AND q._qg_maxx >= 8 \
               AND q._qg_miny <= 12 AND q._qg_maxy >= 8 \
               AND ST_Intersects(\
                 ST_GeomFromWKB(q.\"Geom\"), \
                 ST_GeomFromWKB(ST_MakeEnvelope(8, 8, 12, 12, 3857)))",
            &[],
        )
        .await
        .expect("quoted identifier internal prefiltered query")
        .get(0);
    assert_eq!(public_count, 1);
    assert_eq!(public_count, internal_prefiltered);
}

async fn assert_client_supplied_layout_values_are_ignored(client: &tokio_postgres::Client) {
    client
        .batch_execute(
            "INSERT INTO quackgis.main.layoutbench_layout_tamper \
                (id, _qg_minx, _qg_miny, _qg_maxx, _qg_maxy, geom, label) \
             VALUES (2, -999, -999, -999, -999, X'010100000000000000000022400000000000002440', 'tampered')",
        )
        .await
        .expect("insert with client-supplied layout columns");
    let row = client
        .query_one(
            "SELECT _qg_minx, _qg_miny, _qg_maxx, _qg_maxy \
             FROM quackgis.main.layoutbench_layout_tamper WHERE id = 2",
            &[],
        )
        .await
        .expect("projected tamper layout");
    let projected: (f64, f64, f64, f64) = (row.get(0), row.get(1), row.get(2), row.get(3));
    assert_eq!(projected, (9.0, 10.0, 9.0, 10.0));
}

async fn assert_public_wildcard_spatial_query_stays_hidden(client: &tokio_postgres::Client) {
    let messages = client
        .simple_query(
            "SELECT * FROM layoutbench_aerial_frames \
             WHERE ST_Intersects(\
               ST_GeomFromWKB(geom), \
               ST_GeomFromWKB(ST_MakeEnvelope(95, 95, 290, 185, 3857))) \
             LIMIT 1",
        )
        .await
        .expect("public wildcard spatial query");
    let row = messages
        .iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some(row),
            _ => None,
        })
        .expect("public wildcard spatial row");
    let column_names = row
        .columns()
        .iter()
        .map(|column| column.name().to_string())
        .collect::<Vec<_>>();
    assert!(
        !column_names.iter().any(|name| name.starts_with("_qg_")),
        "public wildcard spatial query should hide layout columns, got {column_names:?}"
    );
}

async fn assert_explain_shows_injected_layout_predicate(client: &tokio_postgres::Client) {
    let messages = client
        .simple_query(
            "EXPLAIN SELECT COUNT(*) AS n FROM layoutbench_aerial_frames \
             WHERE captured_minute BETWEEN 40 AND 170 \
               AND ST_Intersects(\
                 ST_GeomFromWKB(geom), \
                 ST_GeomFromWKB(ST_MakeEnvelope(95, 95, 290, 185, 3857)))",
        )
        .await
        .expect("EXPLAIN public exact query with internal pruning rewrite");
    let rendered = messages
        .iter()
        .filter_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some(
                row.columns()
                    .iter()
                    .filter_map(|column| row.get(column.name()))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rendered.contains("_qg_minx") && rendered.contains("_qg_maxx"),
        "EXPLAIN should expose injected layout predicate, got:\n{rendered}"
    );
}

async fn assert_projected_bbox_matches_sidecars(
    client: &tokio_postgres::Client,
    table: &str,
    expected_rows: i64,
) {
    let sql = format!(
        "SELECT COUNT(*) FROM {table} \
         WHERE _qg_minx = minx AND _qg_miny = miny \
           AND _qg_maxx = maxx AND _qg_maxy = maxy \
           AND _qg_time_bucket IS NOT NULL \
           AND _qg_space_bucket IS NOT NULL \
           AND _qg_space_sort IS NOT NULL"
    );
    let projected: i64 = client
        .query_one(&sql, &[])
        .await
        .expect("projected bbox sidecar check")
        .get(0);
    assert_eq!(projected, expected_rows, "projected layout for {table}");
}

async fn assert_projected_point_layout(
    client: &tokio_postgres::Client,
    table: &str,
    expected_rows: i64,
) {
    let sql = format!(
        "SELECT COUNT(*) FROM {table} \
         WHERE _qg_minx = corrected_x AND _qg_miny = corrected_y \
           AND _qg_maxx = corrected_x AND _qg_maxy = corrected_y \
           AND _qg_time_bucket IS NOT NULL \
           AND _qg_space_bucket IS NOT NULL \
           AND _qg_space_sort IS NOT NULL"
    );
    let projected: i64 = client
        .query_one(&sql, &[])
        .await
        .expect("projected point layout check")
        .get(0);
    assert_eq!(
        projected, expected_rows,
        "projected point layout for {table}"
    );
}

async fn update_control_point(client: &tokio_postgres::Client, id: i32, x: f64, y: f64) {
    let sql = format!(
        "UPDATE layoutbench_control_points \
         SET corrected_x = {x}, corrected_y = {y}, geom = {} \
         WHERE id = {id}",
        point_wkb_sql(x, y)
    );
    client
        .batch_execute(&sql)
        .await
        .expect("update point layout");
}

async fn update_control_point_transactional(
    client: &tokio_postgres::Client,
    id: i32,
    x: f64,
    y: f64,
) {
    client.batch_execute("BEGIN").await.expect("begin update");
    update_control_point(client, id, x, y).await;
    client.batch_execute("COMMIT").await.expect("commit update");
}

#[derive(Debug)]
struct LayoutBenchSf0 {
    aerial_frames: Vec<AerialFrame>,
    cad_objects: Vec<CadObject>,
    assets: Vec<AssetRow>,
    control_points: Vec<ControlPoint>,
}

impl LayoutBenchSf0 {
    fn generate() -> Self {
        Self {
            aerial_frames: generate_aerial_frames(),
            cad_objects: generate_cad_objects(),
            assets: generate_assets(),
            control_points: generate_control_points(),
        }
    }

    async fn load(&self, client: &tokio_postgres::Client) {
        client
            .batch_execute(
                "CREATE TABLE layoutbench_aerial_frames (\
                     id INT, mission TEXT, strip INT, captured_minute INT, \
                     gsd_cm DOUBLE, altitude_m DOUBLE, geom BINARY, \
                     minx DOUBLE, miny DOUBLE, maxx DOUBLE, maxy DOUBLE);\
                  CREATE TABLE layoutbench_cad_objects (\
                     id INT, floor INT, object_type TEXT, source_object_id TEXT, \
                     z_min DOUBLE, z_max DOUBLE, tolerance_mm DOUBLE, geom BINARY, \
                     minx DOUBLE, miny DOUBLE, maxx DOUBLE, maxy DOUBLE);\
                  CREATE TABLE layoutbench_assets (\
                     id INT, asset_type TEXT, uri TEXT, acquired_minute INT, \
                     resolution_cm DOUBLE, horizontal_accuracy_cm DOUBLE, footprint BINARY, \
                     minx DOUBLE, miny DOUBLE, maxx DOUBLE, maxy DOUBLE);\
                  CREATE TABLE layoutbench_control_points (\
                     id INT, point_name TEXT, acquisition_epoch DOUBLE, \
                     source_x DOUBLE, source_y DOUBLE, corrected_x DOUBLE, corrected_y DOUBLE, \
                     residual_mm DOUBLE, geom BINARY);\
                  CREATE TABLE \"layoutbench quoted\" (\
                     id INT, \"Geom\" BINARY, label TEXT);\
                  CREATE TABLE layoutbench_layout_tamper (\
                     id INT, geom BINARY, label TEXT);\
                  CREATE TABLE layoutbench_false_positives (\
                     id INT, geom BINARY, label TEXT);",
            )
            .await
            .expect("create LayoutBench tables");
        insert_rows(client, "layoutbench_aerial_frames", &self.aerial_values()).await;
        insert_rows(client, "layoutbench_cad_objects", &self.cad_values()).await;
        insert_rows(client, "layoutbench_assets", &self.asset_values()).await;
        insert_rows(
            client,
            "layoutbench_control_points",
            &self.control_point_values(),
        )
        .await;
        insert_rows(
            client,
            "\"layoutbench quoted\" (id, \"Geom\", label)",
            &[
                format!("(1, {}, 'inside')", point_wkb_sql(10.0, 10.0)),
                format!("(2, {}, 'outside')", point_wkb_sql(40.0, 40.0)),
            ],
        )
        .await;
        insert_rows(
            client,
            "layoutbench_layout_tamper (id, geom, label)",
            &[format!("(1, {}, 'baseline')", point_wkb_sql(1.0, 1.0))],
        )
        .await;
        insert_rows(
            client,
            "layoutbench_false_positives (id, geom, label)",
            &[
                format!("(1, {}, 'donut_bbox_only')", donut_wkb_sql()),
                format!(
                    "(2, {}, 'inside_envelope')",
                    rect_wkb_sql(Rect {
                        minx: 48.0,
                        miny: 48.0,
                        maxx: 52.0,
                        maxy: 52.0,
                    })
                ),
                format!(
                    "(3, {}, 'outside_envelope')",
                    rect_wkb_sql(Rect {
                        minx: 120.0,
                        miny: 120.0,
                        maxx: 130.0,
                        maxy: 130.0,
                    })
                ),
            ],
        )
        .await;
    }

    fn aerial_values(&self) -> Vec<String> {
        self.aerial_frames
            .iter()
            .map(|row| {
                format!(
                    "({}, '{}', {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    row.id,
                    escape_sql(row.mission),
                    row.strip,
                    row.captured_minute,
                    row.gsd_cm,
                    row.altitude_m,
                    rect_wkb_sql(row.footprint),
                    row.footprint.minx,
                    row.footprint.miny,
                    row.footprint.maxx,
                    row.footprint.maxy,
                )
            })
            .collect()
    }

    fn cad_values(&self) -> Vec<String> {
        self.cad_objects
            .iter()
            .map(|row| {
                format!(
                    "({}, {}, '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {})",
                    row.id,
                    row.floor,
                    escape_sql(row.object_type),
                    escape_sql(&row.source_object_id),
                    row.z_min,
                    row.z_max,
                    row.tolerance_mm,
                    rect_wkb_sql(row.geom),
                    row.geom.minx,
                    row.geom.miny,
                    row.geom.maxx,
                    row.geom.maxy,
                )
            })
            .collect()
    }

    fn asset_values(&self) -> Vec<String> {
        self.assets
            .iter()
            .map(|row| {
                format!(
                    "({}, '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {})",
                    row.id,
                    escape_sql(row.asset_type),
                    escape_sql(&row.uri),
                    row.acquired_minute,
                    row.resolution_cm,
                    row.horizontal_accuracy_cm,
                    rect_wkb_sql(row.footprint),
                    row.footprint.minx,
                    row.footprint.miny,
                    row.footprint.maxx,
                    row.footprint.maxy,
                )
            })
            .collect()
    }

    fn control_point_values(&self) -> Vec<String> {
        self.control_points
            .iter()
            .map(|row| {
                format!(
                    "({}, '{}', {}, {}, {}, {}, {}, {}, {})",
                    row.id,
                    escape_sql(&row.point_name),
                    row.acquisition_epoch,
                    row.source_x,
                    row.source_y,
                    row.corrected_x,
                    row.corrected_y,
                    row.residual_mm,
                    point_wkb_sql(row.corrected_x, row.corrected_y),
                )
            })
            .collect()
    }
}

async fn insert_rows(client: &tokio_postgres::Client, table: &str, rows: &[String]) {
    let sql = format!("INSERT INTO {table} VALUES {}", rows.join(","));
    client.batch_execute(&sql).await.expect("insert rows");
}

fn generate_aerial_frames() -> Vec<AerialFrame> {
    let mut rows = Vec::new();
    let mut id = 1;
    for mission_idx in 0..3 {
        let mission = match mission_idx {
            0 => "mission_alpha",
            1 => "mission_beta",
            _ => "mission_gamma",
        };
        for strip in 0..3 {
            for frame in 0..12 {
                let center_x = 35.0 + frame as f64 * 27.0 + mission_idx as f64 * 4.0;
                let center_y = 55.0 + strip as f64 * 48.0 + mission_idx as f64 * 3.0;
                let width = 52.0 + (frame % 3) as f64 * 4.0;
                let height = 36.0 + (strip % 2) as f64 * 6.0;
                rows.push(AerialFrame {
                    id,
                    mission,
                    strip,
                    captured_minute: mission_idx * 480 + strip * 40 + frame * 9,
                    gsd_cm: 2.5 + mission_idx as f64 * 0.5,
                    altitude_m: 120.0 + strip as f64 * 8.0,
                    footprint: Rect {
                        minx: center_x - width / 2.0,
                        miny: center_y - height / 2.0,
                        maxx: center_x + width / 2.0,
                        maxy: center_y + height / 2.0,
                    },
                });
                id += 1;
            }
        }
    }
    rows
}

fn generate_cad_objects() -> Vec<CadObject> {
    let mut rows = Vec::new();
    let mut id = 1;
    let origin_x = 1_000_000.0;
    let origin_y = 2_000_000.0;
    for floor in 0..4 {
        for bay_x in 0..6 {
            for bay_y in 0..4 {
                let minx = origin_x + bay_x as f64 * 18.5 + floor as f64 * 0.125;
                let miny = origin_y + bay_y as f64 * 14.25 + floor as f64 * 0.075;
                let object_type = if (bay_x + bay_y + floor) % 5 == 0 {
                    "column"
                } else if bay_y % 2 == 0 {
                    "wall"
                } else {
                    "room"
                };
                rows.push(CadObject {
                    id,
                    floor,
                    object_type,
                    source_object_id: format!("IFC-{:02}-{:02}-{:02}", floor, bay_x, bay_y),
                    z_min: floor as f64 * 3.6,
                    z_max: floor as f64 * 3.6 + 3.4,
                    tolerance_mm: 2.0 + ((bay_x + bay_y) % 3) as f64 * 0.25,
                    geom: Rect {
                        minx,
                        miny,
                        maxx: minx + 12.0 + (bay_x % 2) as f64 * 3.0,
                        maxy: miny + 8.0 + (bay_y % 2) as f64 * 4.0,
                    },
                });
                id += 1;
            }
        }
    }
    rows
}

fn generate_assets() -> Vec<AssetRow> {
    let asset_types = ["copc", "cog", "3dtiles", "ifc", "e57", "citygml"];
    asset_types
        .iter()
        .enumerate()
        .flat_map(|(asset_idx, asset_type)| {
            (0..4).map(move |tile_idx| {
                let id = (asset_idx * 4 + tile_idx + 1) as i32;
                let minx = 40.0 + asset_idx as f64 * 70.0 + tile_idx as f64 * 15.0;
                let miny = 35.0 + tile_idx as f64 * 42.0;
                AssetRow {
                    id,
                    asset_type,
                    uri: format!("s3://layoutbench/sf0/{asset_type}/{tile_idx:02}"),
                    acquired_minute: asset_idx as i32 * 120 + tile_idx as i32 * 15,
                    resolution_cm: 5.0 + asset_idx as f64 * 2.0 + tile_idx as f64,
                    horizontal_accuracy_cm: 2.0 + (asset_idx % 3) as f64 * 2.0,
                    footprint: Rect {
                        minx,
                        miny,
                        maxx: minx + 85.0,
                        maxy: miny + 70.0,
                    },
                }
            })
        })
        .collect()
}

fn generate_control_points() -> Vec<ControlPoint> {
    let mut rows = Vec::new();
    let mut id = 1;
    for station in 0..6 {
        for epoch_idx in 0..4 {
            let acquisition_epoch = 2022.0 + epoch_idx as f64;
            let source_x = 500_000.0 + station as f64 * 125.0;
            let source_y = 4_100_000.0 + station as f64 * 75.0;
            let drift_years = acquisition_epoch - 2020.0;
            let corrected_x = source_x + drift_years * 0.012;
            let corrected_y = source_y - drift_years * 0.008;
            rows.push(ControlPoint {
                id,
                point_name: format!("CP-{station:02}"),
                acquisition_epoch,
                source_x,
                source_y,
                corrected_x,
                corrected_y,
                residual_mm: 1.5 + ((station + epoch_idx) % 5) as f64 * 0.85,
            });
            id += 1;
        }
    }
    rows
}

fn envelope_wkb_sql(rect: Rect) -> String {
    format!(
        "ST_MakeEnvelope({}, {}, {}, {}, 3857)",
        rect.minx, rect.miny, rect.maxx, rect.maxy
    )
}

fn rect_wkb_sql(rect: Rect) -> String {
    format!("X'{}'", hex_encode(&rect_wkb(rect)))
}

fn point_wkb_sql(x: f64, y: f64) -> String {
    format!("X'{}'", hex_encode(&point_wkb(x, y)))
}

fn donut_wkb_sql() -> String {
    let outer = vec![
        (0.0, 0.0),
        (100.0, 0.0),
        (100.0, 100.0),
        (0.0, 100.0),
        (0.0, 0.0),
    ];
    let hole = vec![
        (40.0, 40.0),
        (40.0, 60.0),
        (60.0, 60.0),
        (60.0, 40.0),
        (40.0, 40.0),
    ];
    format!("X'{}'", hex_encode(&polygon_wkb(&[outer, hole])))
}

fn rect_wkb(rect: Rect) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(1 + 4 + 4 + 4 + 5 * 16);
    bytes.push(1);
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&5_u32.to_le_bytes());
    for (x, y) in [
        (rect.minx, rect.miny),
        (rect.maxx, rect.miny),
        (rect.maxx, rect.maxy),
        (rect.minx, rect.maxy),
        (rect.minx, rect.miny),
    ] {
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
    }
    bytes
}

fn polygon_wkb(rings: &[Vec<(f64, f64)>]) -> Vec<u8> {
    let coordinate_count = rings.iter().map(Vec::len).sum::<usize>();
    let mut bytes = Vec::with_capacity(1 + 4 + 4 + rings.len() * 4 + coordinate_count * 16);
    bytes.push(1);
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&(rings.len() as u32).to_le_bytes());
    for ring in rings {
        bytes.extend_from_slice(&(ring.len() as u32).to_le_bytes());
        for (x, y) in ring {
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&y.to_le_bytes());
        }
    }
    bytes
}

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(1 + 4 + 16);
    bytes.push(1);
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn escape_sql(value: &str) -> String {
    value.replace('\'', "''")
}
