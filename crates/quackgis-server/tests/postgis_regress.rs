// SPDX-License-Identifier: Apache-2.0
//! Starter curated PostGIS regress subset.
//!
//! This is intentionally small and explicit: it pins the PostGIS-compatible
//! functions QuackGIS already claims, prints a pass-rate line for scheduled trend
//! artifacts, and gives future upstream-regress imports a stable harness.

mod common;

use common::ServerHandle;
use tokio_postgres::NoTls;

struct Case {
    name: &'static str,
    sql: &'static str,
    expected: &'static str,
}

const CASES: &[Case] = &[
    Case {
        name: "postgis_lib_version",
        sql: "SELECT postgis_lib_version()",
        expected: "3.4.0",
    },
    Case {
        name: "postgis_version_marker",
        sql: "SELECT postgis_version()",
        expected: "3.4.0 QUACKGIS",
    },
    Case {
        name: "geomfromtext_astext_point",
        sql: "SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))",
        expected: "POINT(1 2)",
    },
    Case {
        name: "geomfromewkt_astext_point",
        sql: "SELECT ST_AsText(ST_GeomFromEWKT('SRID=4326;POINT(1 2)'))",
        expected: "POINT(1 2)",
    },
    Case {
        name: "setsrid_srid",
        sql: "SELECT CAST(ST_SRID(ST_SetSRID(ST_GeomFromText('POINT(1 2)'), 4326)) AS TEXT)",
        expected: "4326",
    },
    Case {
        name: "transform_sets_target_srid",
        sql: "SELECT CAST(ST_SRID(ST_Transform(ST_SetSRID(ST_GeomFromText('POINT(0 0)'), 4326), 3857)) AS TEXT)",
        expected: "3857",
    },
    Case {
        name: "makeenvelope_srid",
        sql: "SELECT CAST(ST_SRID(ST_MakeEnvelope(0.0, 0.0, 1.0, 1.0, 3857)) AS TEXT)",
        expected: "3857",
    },
    Case {
        name: "intersects_point_in_polygon",
        sql: "SELECT CAST(ST_Intersects(ST_GeomFromText('POINT(1 1)'), ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))')) AS TEXT)",
        expected: "true",
    },
    Case {
        name: "area_square",
        sql: "SELECT CAST(ST_Area(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))')) AS TEXT)",
        expected: "16.0",
    },
    Case {
        name: "intersects_disjoint_points",
        sql: "SELECT CAST(ST_Intersects(ST_GeomFromText('POINT(0 0)'), ST_GeomFromText('POINT(2 2)')) AS TEXT)",
        expected: "false",
    },
    Case {
        name: "extent_points",
        sql: "SELECT ST_Extent(geom) FROM public.postgis_regress_points",
        expected: "BOX(0 0,2 3)",
    },
    Case {
        name: "find_srid_metadata",
        sql: "SELECT CAST(Find_SRID('public', 'postgis_regress_points', 'geom') AS TEXT)",
        expected: "0",
    },
];

#[tokio::test(flavor = "multi_thread")]
async fn curated_postgis_regress_subset_passes() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _conn = tokio::spawn(connection);

    client
        .batch_execute(
            "CREATE TABLE public.postgis_regress_points (id INT, geom BINARY, name TEXT);
             INSERT INTO public.postgis_regress_points VALUES
               (1, X'010100000000000000000000000000000000000000', 'origin'),
               (2, X'010100000000000000000000400000000000000840', 'far');",
        )
        .await
        .expect("seed regress points");

    let mut failures = Vec::new();
    for case in CASES {
        match client.query_one(case.sql, &[]).await {
            Ok(row) => {
                let got: String = row.get(0);
                if got != case.expected {
                    failures.push(format!(
                        "{} expected {:?}, got {:?}",
                        case.name, case.expected, got
                    ));
                }
            }
            Err(err) => failures.push(format!("{} failed: {err}", case.name)),
        }
    }

    let total = CASES.len();
    let passed = total - failures.len();
    println!(
        "postgis_regress_subset passed={passed} total={total} pass_rate={:.3}",
        passed as f64 / total as f64
    );

    if !failures.is_empty() {
        panic!("PostGIS regress subset failures:\n{}", failures.join("\n"));
    }
}
