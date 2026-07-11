// SPDX-License-Identifier: Apache-2.0
use std::sync::Arc;

use adbc_core::options::IngestMode;
use arrow_array::{
    Array, BinaryArray, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use quackgis_server::duckdb_adbc_storage::{DuckDbAdbcConfig, DuckDbAdbcStorage, ExtensionPolicy};
use quackgis_server::engine_api::{EngineMaintenanceRequest, EngineStorageKernel, EngineTableRef};

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut wkb = Vec::with_capacity(21);
    wkb.push(1); // little-endian
    wkb.extend_from_slice(&1_u32.to_le_bytes()); // Point
    wkb.extend_from_slice(&x.to_le_bytes());
    wkb.extend_from_slice(&y.to_le_bytes());
    wkb
}

fn first_i64(batch: &RecordBatch, column: usize) -> i64 {
    batch
        .column(column)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("Int64 query result")
        .value(0)
}

fn first_string(batch: &RecordBatch, column: usize) -> &str {
    batch
        .column(column)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("Utf8 query result")
        .value(0)
}

fn point_batch(ids: Vec<i32>, names: Vec<&str>, geometries: Vec<&[u8]>) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("geom_wkb", DataType::Binary, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(ids)),
            Arc::new(StringArray::from(names)),
            Arc::new(BinaryArray::from_vec(geometries)),
        ],
    )
    .expect("Arrow input batch")
}

fn layout_point_batch(points: &[(i32, f64, f64)]) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("geom_wkb", DataType::Binary, false),
        Field::new("_qg_minx", DataType::Float64, false),
        Field::new("_qg_miny", DataType::Float64, false),
        Field::new("_qg_maxx", DataType::Float64, false),
        Field::new("_qg_maxy", DataType::Float64, false),
    ]));
    let geometries: Vec<Vec<u8>> = points.iter().map(|(_, x, y)| point_wkb(*x, *y)).collect();
    let xs: Vec<f64> = points.iter().map(|(_, x, _)| *x).collect();
    let ys: Vec<f64> = points.iter().map(|(_, _, y)| *y).collect();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(
                points.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
            )),
            Arc::new(BinaryArray::from_iter_values(
                geometries.iter().map(Vec::as_slice),
            )),
            Arc::new(Float64Array::from(xs.clone())),
            Arc::new(Float64Array::from(ys.clone())),
            Arc::new(Float64Array::from(xs)),
            Arc::new(Float64Array::from(ys)),
        ],
    )
    .expect("layout point batch")
}

#[test]
#[ignore = "requires QUACKGIS_DUCKDB_ADBC_DRIVER pointing to libduckdb"]
fn official_ducklake_roundtrips_arrow_and_one_snapshot_transaction() {
    let driver_path = std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER")
        .expect("set QUACKGIS_DUCKDB_ADBC_DRIVER to an absolute libduckdb path");
    let temp = tempfile::tempdir().expect("temporary DuckLake root");
    let data_path = temp.path().join("data");
    std::fs::create_dir(&data_path).expect("DuckLake data directory");
    let catalog_path = temp.path().join("catalog.ducklake");

    let config = DuckDbAdbcConfig {
        driver_path: driver_path.into(),
        database_uri: ":memory:".to_owned(),
        ducklake_uri: format!("ducklake:{}", catalog_path.display()),
        catalog_name: "quackgis".to_owned(),
        data_path: data_path.display().to_string(),
        extension_policy: ExtensionPolicy::LoadOnly,
    };
    let storage = DuckDbAdbcStorage::open(config.clone()).expect("open DuckDB ADBC storage");

    let geometries = [
        point_wkb(0.0, 0.0),
        point_wkb(1.0, 1.0),
        point_wkb(2.0, 2.0),
    ];
    let batch = point_batch(
        vec![1, 2, 3],
        vec!["origin", "one", "two"],
        geometries.iter().map(Vec::as_slice).collect(),
    );

    storage
        .ingest("main", "points", vec![batch], IngestMode::Create)
        .expect("ADBC Arrow ingestion into DuckLake");

    let spatial = storage
        .query(
            "SELECT count(*) FROM quackgis.main.points \
             WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromText('POINT (1 1)'))",
        )
        .expect("exact DuckDB spatial predicate over valid WKB");
    assert_eq!(first_i64(&spatial[0], 0), 1);

    let nested_call = storage.transaction::<()>(|_| {
        storage.query("SELECT 1")?;
        Ok(())
    });
    assert!(
        nested_call
            .expect_err("reentrant storage access must fail rather than deadlock")
            .to_string()
            .contains("busy or quarantined")
    );

    let snapshots_before = storage
        .query("SELECT count(*) AS snapshots FROM ducklake_snapshots('quackgis')")
        .expect("snapshot count before transaction");
    let snapshots_before = first_i64(&snapshots_before[0], 0);

    let rollback = storage.transaction::<()>(|transaction| {
        transaction.execute_update(
            "UPDATE quackgis.main.points SET name = 'must-rollback' WHERE id = 2",
        )?;
        anyhow::bail!("intentional rollback oracle")
    });
    assert!(rollback.is_err());
    let rolled_back = storage
        .query("SELECT name FROM quackgis.main.points WHERE id = 2")
        .expect("query after rollback");
    assert_eq!(first_string(&rolled_back[0], 0), "one");
    let snapshots_after_rollback = storage
        .query("SELECT count(*) AS snapshots FROM ducklake_snapshots('quackgis')")
        .expect("snapshot count after rollback");
    assert_eq!(first_i64(&snapshots_after_rollback[0], 0), snapshots_before);

    storage
        .transaction(|transaction| {
            transaction
                .execute_update("UPDATE quackgis.main.points SET name = 'uno' WHERE id = 2")?;
            transaction.execute_update("DELETE FROM quackgis.main.points WHERE id = 1")?;
            Ok(())
        })
        .expect("one-snapshot DuckLake mutation");

    let snapshots_after = storage
        .query("SELECT count(*) AS snapshots FROM ducklake_snapshots('quackgis')")
        .expect("snapshot count after transaction");
    assert_eq!(first_i64(&snapshots_after[0], 0), snapshots_before + 1);

    let rows = storage
        .query(
            "SELECT count(*) AS rows, string_agg(name, ',' ORDER BY id) AS names \
             FROM quackgis.main.points",
        )
        .expect("query mutated DuckLake table");
    assert_eq!(first_i64(&rows[0], 0), 2);
    assert_eq!(first_string(&rows[0], 1), "uno,two");
    drop(storage);

    let reopened = DuckDbAdbcStorage::open(config).expect("reopen DuckDB ADBC storage");
    let rows = reopened
        .query(
            "SELECT count(*) AS rows FROM quackgis.main.points \
             WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_MakeEnvelope(-1, -1, 3, 3))",
        )
        .expect("query reopened DuckLake table");
    assert_eq!(first_i64(&rows[0], 0), 2);
    drop(reopened);

    let sql = format!(
        "LOAD spatial; LOAD ducklake; ATTACH 'ducklake:{}' AS qg; \
         SELECT count(*) AS rows, count(*) FILTER (WHERE ST_Intersects(\
         ST_GeomFromWKB(geom_wkb), ST_MakeEnvelope(-1, -1, 3, 3))) AS hits \
         FROM qg.main.points;",
        catalog_path.display()
    );
    let output = std::process::Command::new("duckdb")
        .args(["-csv", ":memory:", "-c", &sql])
        .output()
        .expect("run independent DuckDB CLI reader");
    assert!(
        output.status.success(),
        "independent reader failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("2,2"));
}

#[test]
#[ignore = "requires QUACKGIS_DUCKDB_ADBC_DRIVER pointing to libduckdb"]
fn engine_contract_describes_binds_discovers_and_maintains() {
    let driver_path = std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER")
        .expect("set QUACKGIS_DUCKDB_ADBC_DRIVER to an absolute libduckdb path");
    let temp = tempfile::tempdir().expect("temporary DuckLake root");
    let data_path = temp.path().join("data");
    std::fs::create_dir(&data_path).expect("DuckLake data directory");
    let catalog_path = temp.path().join("catalog.ducklake");
    let storage = DuckDbAdbcStorage::open(DuckDbAdbcConfig {
        driver_path: driver_path.into(),
        database_uri: ":memory:".to_owned(),
        ducklake_uri: format!("ducklake:{}", catalog_path.display()),
        catalog_name: "quackgis".to_owned(),
        data_path: data_path.display().to_string(),
        extension_policy: ExtensionPolicy::LoadOnly,
    })
    .expect("open DuckDB ADBC storage");
    let second_session = storage
        .open_session()
        .expect("second independent ADBC session");

    let description = storage
        .describe("SELECT ?::INTEGER AS id, ?::VARCHAR AS name")
        .expect("describe prepared statement");
    assert_eq!(description.parameter_schema.fields().len(), 2);
    assert_eq!(description.result_schema.fields().len(), 2);
    assert_eq!(description.result_schema.field(0).name(), "id");

    let parameter_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let parameters = RecordBatch::try_new(
        parameter_schema,
        vec![
            Arc::new(Int32Array::from(vec![42])),
            Arc::new(StringArray::from(vec![Some("bound")])),
        ],
    )
    .expect("parameter batch");
    let bound = storage
        .query_bound("SELECT ?::INTEGER AS id, ?::VARCHAR AS name", parameters)
        .expect("execute bound statement");
    assert_eq!(bound.schema.field(0).name(), "id");
    assert_eq!(
        bound.batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("bound Int32")
            .value(0),
        42
    );
    assert_eq!(first_string(&bound.batches[0], 1), "bound");

    let empty = storage
        .query_result("SELECT 1::INTEGER AS id, 'empty'::VARCHAR AS name WHERE false")
        .expect("empty query result");
    assert_eq!(empty.schema.fields().len(), 2);
    assert_eq!(empty.schema.field(1).name(), "name");
    assert_eq!(
        empty
            .batches
            .iter()
            .map(RecordBatch::num_rows)
            .sum::<usize>(),
        0
    );

    let table = EngineTableRef {
        catalog: "quackgis".to_owned(),
        schema: "main".to_owned(),
        table: "contract_points".to_owned(),
    };
    storage
        .ingest_contract(
            &table,
            vec![point_batch(
                vec![1],
                vec!["one"],
                vec![point_wkb(1.0, 1.0).as_slice()],
            )],
            quackgis_server::engine_api::IngestDisposition::Create,
        )
        .expect("create through engine contract");
    let second_session_rows = second_session
        .query_result("SELECT count(*) FROM quackgis.main.contract_points")
        .expect("second session observes committed create");
    assert_eq!(first_i64(&second_session_rows.batches[0], 0), 1);
    for id in 2..=4 {
        let geometry = point_wkb(f64::from(id), f64::from(id));
        storage
            .ingest_contract(
                &table,
                vec![point_batch(
                    vec![id],
                    vec!["append"],
                    vec![geometry.as_slice()],
                )],
                quackgis_server::engine_api::IngestDisposition::Append,
            )
            .expect("append through engine contract");
    }
    let schema = storage.table_schema(&table).expect("ADBC table schema");
    assert_eq!(schema.fields().len(), 3);
    assert_eq!(schema.field(2).name(), "geom_wkb");
    assert!(!storage.snapshots().expect("typed snapshots").is_empty());

    storage
        .transaction(|transaction| {
            transaction.execute_update(
                "INSERT INTO quackgis.main.contract_points VALUES \
                 (5, 'pending', ST_AsWKB(ST_Point(5, 5)))",
            )?;
            let isolated = second_session
                .query_result("SELECT count(*) FROM quackgis.main.contract_points")
                .map_err(anyhow::Error::new)?;
            assert_eq!(first_i64(&isolated.batches[0], 0), 4);
            Ok(())
        })
        .expect("commit connection-local transaction");
    let committed = second_session
        .query_result("SELECT count(*) FROM quackgis.main.contract_points")
        .expect("second session observes committed transaction");
    assert_eq!(first_i64(&committed.batches[0], 0), 5);

    storage
        .maintain(EngineMaintenanceRequest::MergeAdjacentFiles {
            schema: "main".to_owned(),
            table: "contract_points".to_owned(),
            max_compacted_files: Some(8),
            max_file_size: None,
            min_file_size: None,
        })
        .expect("official DuckLake file merge");
    let rows = storage
        .query_result("SELECT count(*) FROM quackgis.main.contract_points")
        .expect("query after maintenance");
    assert_eq!(first_i64(&rows.batches[0], 0), 5);
}

#[test]
#[ignore = "requires QUACKGIS_DUCKDB_ADBC_DRIVER pointing to libduckdb"]
fn duckdb_hidden_bbox_candidates_keep_exact_spatial_recheck() {
    let driver_path = std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER")
        .expect("set QUACKGIS_DUCKDB_ADBC_DRIVER to an absolute libduckdb path");
    let temp = tempfile::tempdir().expect("temporary DuckLake root");
    let data_path = temp.path().join("data");
    std::fs::create_dir(&data_path).expect("DuckLake data directory");
    let catalog_path = temp.path().join("catalog.ducklake");
    let config = DuckDbAdbcConfig {
        driver_path: driver_path.into(),
        database_uri: ":memory:".to_owned(),
        ducklake_uri: format!("ducklake:{}", catalog_path.display()),
        catalog_name: "quackgis".to_owned(),
        data_path: data_path.display().to_string(),
        extension_policy: ExtensionPolicy::LoadOnly,
    };
    let storage = DuckDbAdbcStorage::open(config.clone()).expect("open DuckDB layout storage");
    storage
        .ingest(
            "main",
            "layout_points",
            vec![layout_point_batch(&[
                (1, 0.0, 0.0),
                (2, 2.0, 2.0),
                (3, 5.0, 5.0),
                (4, 10.0, 10.0),
            ])],
            IngestMode::Create,
        )
        .expect("ingest hidden-layout fixture");

    let polygon = "POLYGON ((-1 -1, 6 -1, 6 6, -1 6, -1 -1), \
                   (1 1, 3 1, 3 3, 1 3, 1 1))";
    let candidates = storage
        .query(
            "SELECT count(*) FROM quackgis.main.layout_points \
             WHERE _qg_maxx >= -1 AND _qg_minx <= 6 \
               AND _qg_maxy >= -1 AND _qg_miny <= 6",
        )
        .expect("bbox candidate query");
    assert_eq!(first_i64(&candidates[0], 0), 3);

    let exact_sql = format!(
        "SELECT count(*) FROM quackgis.main.layout_points \
         WHERE _qg_maxx >= -1 AND _qg_minx <= 6 \
           AND _qg_maxy >= -1 AND _qg_miny <= 6 \
           AND ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromText('{polygon}'))"
    );
    let exact = storage.query(&exact_sql).expect("bbox plus exact recheck");
    assert_eq!(first_i64(&exact[0], 0), 2);

    let explain = storage
        .query(&format!("EXPLAIN {exact_sql}"))
        .expect("DuckDB exact-recheck plan");
    let plan = explain
        .iter()
        .flat_map(|batch| {
            batch
                .column(1)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("EXPLAIN value")
                .iter()
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    assert!(
        plan.contains("_qg_minx"),
        "plan omitted bbox filter:\n{plan}"
    );
    assert!(
        plan.contains("st_intersects"),
        "plan omitted exact recheck:\n{plan}"
    );
    drop(storage);

    let reopened = DuckDbAdbcStorage::open(config).expect("reopen DuckDB layout storage");
    let reopened_exact = reopened
        .query(&exact_sql)
        .expect("exact recheck after reopen");
    assert_eq!(first_i64(&reopened_exact[0], 0), 2);
}
