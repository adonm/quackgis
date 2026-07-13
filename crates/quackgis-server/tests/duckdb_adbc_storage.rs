// SPDX-License-Identifier: Apache-2.0
use std::sync::Arc;

use adbc_core::options::IngestMode;
use arrow_array::{
    Array, BinaryArray, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use quackgis_server::duckdb_adbc_storage::{
    DuckDbAdbcConfig, DuckDbAdbcStorage, DuckDbResourceConfig, ExtensionPolicy,
};
use quackgis_server::engine_api::{
    EngineErrorKind, EngineMaintenanceRequest, EngineStorageKernel, EngineTableRef,
    EngineTransactionState,
};

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut wkb = Vec::with_capacity(21);
    wkb.push(1); // little-endian
    wkb.extend_from_slice(&1_u32.to_le_bytes()); // Point
    wkb.extend_from_slice(&x.to_le_bytes());
    wkb.extend_from_slice(&y.to_le_bytes());
    wkb
}

fn empty_point_wkb() -> Vec<u8> {
    point_wkb(f64::NAN, f64::NAN)
}

fn invalid_bowtie_wkb() -> Vec<u8> {
    let mut wkb = Vec::new();
    wkb.push(1);
    wkb.extend_from_slice(&3_u32.to_le_bytes());
    wkb.extend_from_slice(&1_u32.to_le_bytes());
    wkb.extend_from_slice(&5_u32.to_le_bytes());
    for (x, y) in [
        (0.0_f64, 0.0_f64),
        (2.0, 2.0),
        (0.0, 2.0),
        (2.0, 0.0),
        (0.0, 0.0),
    ] {
        wkb.extend_from_slice(&x.to_le_bytes());
        wkb.extend_from_slice(&y.to_le_bytes());
    }
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
        Field::new("geom_wkb", DataType::Binary, true),
        Field::new("_qg_minx", DataType::Float64, true),
        Field::new("_qg_miny", DataType::Float64, true),
        Field::new("_qg_maxx", DataType::Float64, true),
        Field::new("_qg_maxy", DataType::Float64, true),
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

fn layout_edge_geometry_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("geom_wkb", DataType::Binary, true),
        Field::new("_qg_minx", DataType::Float64, true),
        Field::new("_qg_miny", DataType::Float64, true),
        Field::new("_qg_maxx", DataType::Float64, true),
        Field::new("_qg_maxy", DataType::Float64, true),
    ]));
    let empty = empty_point_wkb();
    let invalid = invalid_bowtie_wkb();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(vec![6, 7, 8])),
            Arc::new(BinaryArray::from_opt_vec(vec![
                None,
                Some(empty.as_slice()),
                Some(invalid.as_slice()),
            ])),
            Arc::new(Float64Array::from(vec![None, None, Some(0.0)])),
            Arc::new(Float64Array::from(vec![None, None, Some(0.0)])),
            Arc::new(Float64Array::from(vec![None, None, Some(2.0)])),
            Arc::new(Float64Array::from(vec![None, None, Some(2.0)])),
        ],
    )
    .expect("layout edge geometry batch")
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

    let lifecycle = storage.lifecycle();
    storage
        .transaction(|transaction| {
            assert_eq!(lifecycle.active_transactions(), 1);
            transaction
                .execute_update("UPDATE quackgis.main.points SET name = 'uno' WHERE id = 2")?;
            transaction.execute_update("DELETE FROM quackgis.main.points WHERE id = 1")?;
            Ok(())
        })
        .expect("one-snapshot DuckLake mutation");
    assert_eq!(lifecycle.active_transactions(), 0);

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
                (4, 6.0, 6.0),
                (5, 10.0, 10.0),
            ])],
            IngestMode::Create,
        )
        .expect("ingest hidden-layout fixture");
    storage
        .ingest(
            "main",
            "layout_points",
            vec![layout_edge_geometry_batch()],
            IngestMode::Append,
        )
        .expect("append NULL/empty/invalid geometry fixture");

    let polygon = "POLYGON ((-1 -1, 6 -1, 6 6, -1 6, -1 -1), \
                   (1 1, 3 1, 3 3, 1 3, 1 1))";
    let candidates = storage
        .query(
            "SELECT count(*) FROM quackgis.main.layout_points \
             WHERE id < 8 AND _qg_maxx >= -1 AND _qg_minx <= 6 \
               AND _qg_maxy >= -1 AND _qg_miny <= 6",
        )
        .expect("bbox candidate query");
    assert_eq!(first_i64(&candidates[0], 0), 4);

    let exact_sql = format!(
        "SELECT count(*) FROM quackgis.main.layout_points \
         WHERE id < 8 AND \
           ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromText('{polygon}'))"
    );
    let exact = storage
        .query(&exact_sql)
        .expect("automatically injected bbox plus exact recheck");
    assert_eq!(first_i64(&exact[0], 0), 3);

    let bound_sql = "SELECT count(*) FROM quackgis.main.layout_points \
                     WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), \
                                         ST_GeomFromWKB($1::BLOB))";
    let description = storage
        .describe(bound_sql)
        .expect("describe bound bbox query");
    assert_eq!(description.parameter_schema.fields().len(), 1);
    let point = point_wkb(5.0, 5.0);
    let parameters = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "probe_wkb",
            DataType::Binary,
            false,
        )])),
        vec![Arc::new(BinaryArray::from_vec(vec![point.as_slice()]))],
    )
    .expect("bound bbox parameter");
    let bound = storage
        .query_bound(bound_sql, parameters)
        .expect("bound bbox query");
    assert_eq!(first_i64(&bound.batches[0], 0), 1);

    for (label, injected, exact_only) in [
        (
            "NULL and empty data",
            "SELECT count(*) FROM quackgis.main.layout_points WHERE id IN (6, 7) AND ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_MakeEnvelope(-1, -1, 1, 1))",
            "SELECT count(*) FROM quackgis.main.layout_points WHERE id IN (6, 7) AND (ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_MakeEnvelope(-1, -1, 1, 1))) IS TRUE",
        ),
        (
            "empty probe",
            "SELECT count(*) FROM quackgis.main.layout_points WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromText('POINT EMPTY'))",
            "SELECT count(*) FROM quackgis.main.layout_points WHERE (ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromText('POINT EMPTY'))) IS TRUE",
        ),
        (
            "invalid data",
            "SELECT count(*) FROM quackgis.main.layout_points WHERE id = 8 AND ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromText('POINT (1 1)'))",
            "SELECT count(*) FROM quackgis.main.layout_points WHERE id = 8 AND (ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromText('POINT (1 1)'))) IS TRUE",
        ),
        (
            "invalid probe",
            "SELECT count(*) FROM quackgis.main.layout_points WHERE id < 8 AND ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromText('POLYGON((0 0,2 2,0 2,2 0,0 0))'))",
            "SELECT count(*) FROM quackgis.main.layout_points WHERE id < 8 AND (ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromText('POLYGON((0 0,2 2,0 2,2 0,0 0))'))) IS TRUE",
        ),
    ] {
        let injected = storage
            .query(injected)
            .unwrap_or_else(|error| panic!("{label} injected query: {error}"));
        let exact_only = storage
            .query(exact_only)
            .unwrap_or_else(|error| panic!("{label} exact-only query: {error}"));
        assert_eq!(
            first_i64(&injected[0], 0),
            first_i64(&exact_only[0], 0),
            "{label} candidate must equal exact oracle"
        );
    }

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
    assert_eq!(first_i64(&reopened_exact[0], 0), 3);
}

#[test]
#[ignore = "requires QUACKGIS_DUCKDB_ADBC_DRIVER pointing to libduckdb"]
fn query_stream_owns_connection_and_reads_incrementally() {
    let driver =
        std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER").expect("set QUACKGIS_DUCKDB_ADBC_DRIVER");
    let temp = tempfile::tempdir().expect("tempdir");
    let data_path = temp.path().join("data");
    std::fs::create_dir(&data_path).expect("data path");
    let config = DuckDbAdbcConfig {
        driver_path: driver.into(),
        database_uri: ":memory:".to_owned(),
        ducklake_uri: format!(
            "ducklake:{}",
            temp.path().join("catalog.ducklake").display()
        ),
        catalog_name: "quackgis".to_owned(),
        data_path: data_path.display().to_string(),
        extension_policy: ExtensionPolicy::LoadOnly,
    };
    let storage = Arc::new(
        DuckDbAdbcStorage::open_with_resources(
            config,
            DuckDbResourceConfig {
                threads: 4,
                memory_limit_bytes: 1_073_741_824,
                temp_directory: data_path.join(".tmp"),
                max_temp_directory_bytes: 10_737_418_240,
            },
        )
        .expect("open storage"),
    );
    let settings = storage
        .query(
            "SELECT name, value FROM duckdb_settings() \
             WHERE name IN ('max_temp_directory_size', 'memory_limit', 'temp_directory', 'threads') \
             ORDER BY name",
        )
        .expect("resource settings");
    let names = settings[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("setting names");
    let values = settings[0]
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("setting values");
    assert_eq!(names.value(0), "max_temp_directory_size");
    assert_eq!(values.value(0), "10.0 GiB");
    assert_eq!(names.value(1), "memory_limit");
    assert_eq!(values.value(1), "1.0 GiB");
    assert_eq!(names.value(2), "temp_directory");
    assert_eq!(values.value(2), data_path.join(".tmp").to_string_lossy());
    assert_eq!(names.value(3), "threads");
    assert_eq!(values.value(3), "4");
    let resources = storage.resource_sample().expect("DuckDB resource sample");
    assert!(resources.memory_bytes > 0);
    storage
        .readiness_probe()
        .expect("empty DuckLake readiness probe");

    let mut stream = storage
        .query_stream("SELECT i::BIGINT AS id FROM range(100000) AS rows(i) ORDER BY i")
        .expect("open query stream");
    assert_eq!(stream.schema.fields().len(), 1);
    let first = stream
        .next_batch()
        .expect("first batch")
        .expect("non-empty stream");
    assert!(
        first.num_rows() < 100_000,
        "driver must expose multiple batches"
    );
    assert!(
        storage.query("SELECT 1").is_err(),
        "live stream retains the sole session connection"
    );
    let mut rows = first.num_rows();
    while let Some(batch) = stream.next_batch().expect("next batch") {
        rows += batch.num_rows();
    }
    assert_eq!(rows, 100_000);
    drop(stream);
    assert_eq!(
        storage
            .query("SELECT 1")
            .expect("connection returned")
            .len(),
        1
    );

    let mut empty = storage
        .query_stream("SELECT 1::INTEGER AS id WHERE false")
        .expect("empty stream");
    assert_eq!(empty.schema.fields().len(), 1);
    assert!(empty.next_batch().expect("empty batch read").is_none());
    drop(empty);
    storage
        .query("SELECT 1")
        .expect("empty stream returned connection");

    let quarantined = Arc::new(storage.open_session().expect("independent stream session"));
    let mut partial = quarantined
        .query_stream("SELECT i::BIGINT AS id FROM range(100000) AS rows(i) ORDER BY i")
        .expect("open partial stream");
    assert!(partial.next_batch().expect("partial first batch").is_some());
    drop(partial);
    assert_eq!(
        quarantined.transaction_state(),
        EngineTransactionState::Quarantined
    );
    let error = match quarantined.query_stream("SELECT 1") {
        Ok(_) => panic!("quarantined session must reject reuse"),
        Err(error) => error,
    };
    assert_eq!(error.kind, EngineErrorKind::Quarantined);
    storage
        .readiness_probe()
        .expect("independent DuckLake readiness after query work");
}

#[test]
#[ignore = "requires QUACKGIS_DUCKDB_ADBC_DRIVER pointing to libduckdb"]
fn offline_backup_restores_exact_catalog_snapshot() {
    let driver =
        std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER").expect("set QUACKGIS_DUCKDB_ADBC_DRIVER");
    let temp = tempfile::tempdir().expect("tempdir");
    let data_path = temp.path().join("data");
    std::fs::create_dir(&data_path).expect("data path");
    let catalog_path = temp.path().join("catalog.ducklake");
    let backup_path = temp.path().join("backup");
    let config = DuckDbAdbcConfig {
        driver_path: driver.into(),
        database_uri: ":memory:".to_owned(),
        ducklake_uri: format!("ducklake:{}", catalog_path.display()),
        catalog_name: "quackgis".to_owned(),
        data_path: data_path.display().to_string(),
        extension_policy: ExtensionPolicy::LoadOnly,
    };
    let storage = DuckDbAdbcStorage::open(config.clone()).expect("open backup source");
    storage
        .execute_update("CREATE TABLE quackgis.main.recovery(id INTEGER, name VARCHAR)")
        .expect("create recovery table");
    storage
        .execute_update("INSERT INTO quackgis.main.recovery VALUES (1, 'one'), (2, 'two')")
        .expect("seed recovery table");
    let snapshot_id = storage
        .snapshots()
        .expect("source snapshots")
        .last()
        .unwrap()
        .id;
    drop(storage);

    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/duckdb_local_backup.py");
    let backup = std::process::Command::new("python3")
        .arg(&script)
        .arg("backup")
        .arg("--catalog")
        .arg(&catalog_path)
        .arg("--data-root")
        .arg(&data_path)
        .arg("--destination")
        .arg(&backup_path)
        .status()
        .expect("run local backup");
    assert!(backup.success());
    std::fs::remove_file(&catalog_path).expect("remove source catalog");
    std::fs::remove_dir_all(&data_path).expect("remove source data");
    let restore = std::process::Command::new("python3")
        .arg(&script)
        .arg("restore")
        .arg("--backup")
        .arg(&backup_path)
        .arg("--catalog")
        .arg(&catalog_path)
        .arg("--data-root")
        .arg(&data_path)
        .status()
        .expect("run local restore");
    assert!(restore.success());

    let restored = DuckDbAdbcStorage::open(config).expect("open restored catalog");
    let rows = restored
        .query("SELECT count(*)::BIGINT, sum(id)::BIGINT FROM quackgis.main.recovery")
        .expect("query restored rows");
    assert_eq!(first_i64(&rows[0], 0), 2);
    assert_eq!(first_i64(&rows[0], 1), 3);
    assert_eq!(
        restored
            .snapshots()
            .expect("restored snapshots")
            .last()
            .unwrap()
            .id,
        snapshot_id
    );
}
