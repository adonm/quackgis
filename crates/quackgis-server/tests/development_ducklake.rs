// SPDX-License-Identifier: Apache-2.0
use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use quackgis_server::duckdb_adbc_storage::{DuckDbAdbcConfig, DuckDbAdbcStorage, ExtensionPolicy};
use quackgis_server::engine_api::EngineStorageKernel;

#[derive(Clone, Debug, Eq, PartialEq)]
struct IdentityRow {
    schema_name: String,
    schema_id: i64,
    schema_uuid: String,
    table_name: String,
    table_id: i64,
    table_uuid: String,
    column_name: String,
    column_id: i64,
}

fn identity_rows(batches: &[RecordBatch]) -> Vec<IdentityRow> {
    let mut rows = Vec::new();
    for batch in batches {
        let strings = |column: usize| {
            batch
                .column(column)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("identity string column")
        };
        let integers = |column: usize| {
            batch
                .column(column)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("identity BIGINT column")
        };
        for row in 0..batch.num_rows() {
            rows.push(IdentityRow {
                schema_name: strings(0).value(row).to_owned(),
                schema_id: integers(1).value(row),
                schema_uuid: strings(2).value(row).to_owned(),
                table_name: strings(3).value(row).to_owned(),
                table_id: integers(4).value(row),
                table_uuid: strings(5).value(row).to_owned(),
                column_name: strings(6).value(row).to_owned(),
                column_id: integers(7).value(row),
            });
        }
    }
    rows
}

const COLUMN_IDENTITY_SQL: &str = "SELECT schema_name, schema_id, CAST(schema_uuid AS VARCHAR), table_name, \
            table_id, CAST(table_uuid AS VARCHAR), column_name, column_id \
     FROM ducklake_column_info('quackgis') ORDER BY table_id, column_id";

#[test]
#[ignore = "requires an explicitly selected checksum-pinned development extension"]
fn development_ducklake_column_identity_contract() {
    let driver_path =
        std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER").expect("set QUACKGIS_DUCKDB_ADBC_DRIVER");
    let extension_path = std::env::var_os("QUACKGIS_DEV_DUCKLAKE_EXTENSION")
        .expect("set QUACKGIS_DEV_DUCKLAKE_EXTENSION");
    let extension_sha256 = std::env::var("QUACKGIS_DEV_DUCKLAKE_EXTENSION_SHA256")
        .expect("set QUACKGIS_DEV_DUCKLAKE_EXTENSION_SHA256");
    let temp = tempfile::tempdir().expect("temporary DuckLake root");
    let data_path = temp.path().join("data");
    std::fs::create_dir(&data_path).expect("DuckLake data directory");
    let config = DuckDbAdbcConfig {
        driver_path: driver_path.into(),
        database_uri: ":memory:".to_owned(),
        ducklake_uri: format!(
            "ducklake:{}",
            temp.path().join("catalog.ducklake").display()
        ),
        catalog_name: "quackgis".to_owned(),
        data_path: data_path.display().to_string(),
        extension_policy: ExtensionPolicy::DevelopmentDuckLake {
            path: extension_path.into(),
            sha256: extension_sha256,
        },
    };
    let storage = DuckDbAdbcStorage::open(config.clone()).expect("open development DuckLake");

    let description = storage
        .describe("SELECT * FROM ducklake_column_info('quackgis')")
        .expect("describe column identity function");
    assert_eq!(
        description
            .result_schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        [
            "schema_name",
            "schema_id",
            "schema_uuid",
            "table_name",
            "table_id",
            "table_uuid",
            "column_name",
            "column_id",
        ]
    );

    storage
        .execute_update(
            "CREATE TABLE quackgis.main.identity_probe(\
             id BIGINT, label VARCHAR, payload STRUCT(x INTEGER))",
        )
        .expect("create empty identity table");
    storage
        .execute_update("CREATE VIEW quackgis.main.identity_view AS SELECT 1 AS id")
        .expect("create excluded view");
    let initial = identity_rows(
        &storage
            .query(COLUMN_IDENTITY_SQL)
            .expect("initial column identities"),
    );
    assert_eq!(initial.len(), 3);
    assert!(initial.iter().all(|row| row.schema_name == "main"));
    assert!(initial.iter().all(|row| row.table_name == "identity_probe"));
    assert_eq!(
        initial
            .iter()
            .map(|row| (row.column_name.as_str(), row.column_id))
            .collect::<Vec<_>>(),
        [("id", 1), ("label", 2), ("payload", 3)]
    );
    assert!(
        initial
            .iter()
            .all(|row| row.table_id == initial[0].table_id)
    );
    assert!(
        initial
            .iter()
            .all(|row| row.table_uuid == initial[0].table_uuid)
    );

    storage
        .transaction(|transaction| {
            transaction.execute_update(
                "ALTER TABLE quackgis.main.identity_probe RENAME TO identity_renamed",
            )?;
            transaction.execute_update(
                "ALTER TABLE quackgis.main.identity_renamed RENAME COLUMN label TO title",
            )?;
            Ok(())
        })
        .expect("commit supported renames");
    let renamed = identity_rows(
        &storage
            .query(COLUMN_IDENTITY_SQL)
            .expect("renamed identities"),
    );
    assert_eq!(
        renamed
            .iter()
            .map(|row| (row.column_name.as_str(), row.column_id))
            .collect::<Vec<_>>(),
        [("id", 1), ("title", 2), ("payload", 3)]
    );
    for (before, after) in initial.iter().zip(&renamed) {
        assert_eq!(before.schema_id, after.schema_id);
        assert_eq!(before.schema_uuid, after.schema_uuid);
        assert_eq!(before.table_id, after.table_id);
        assert_eq!(before.table_uuid, after.table_uuid);
        assert_eq!(before.column_id, after.column_id);
    }

    let rollback = storage.transaction::<()>(|transaction| {
        transaction
            .execute_update("ALTER TABLE quackgis.main.identity_renamed RENAME TO must_rollback")?;
        let pinned = identity_rows(&transaction.query(COLUMN_IDENTITY_SQL)?);
        assert!(
            pinned
                .iter()
                .all(|row| row.table_name == "identity_renamed")
        );
        anyhow::bail!("intentional identity rollback")
    });
    assert!(rollback.is_err());
    assert_eq!(
        identity_rows(
            &storage
                .query(COLUMN_IDENTITY_SQL)
                .expect("identities after rollback")
        ),
        renamed
    );

    storage
        .execute_update("ALTER TABLE quackgis.main.identity_renamed ADD COLUMN added BOOLEAN")
        .expect("add committed column");
    let with_added = identity_rows(
        &storage
            .query(COLUMN_IDENTITY_SQL)
            .expect("identities with new column"),
    );
    assert_eq!(
        with_added.last().expect("added column").column_name,
        "added"
    );
    let added = with_added.last().expect("added column");
    assert!(added.column_id > renamed.iter().map(|row| row.column_id).max().unwrap());
    assert!(
        renamed
            .iter()
            .all(|existing| existing.column_id != added.column_id)
    );
    drop(storage);

    let reopened = DuckDbAdbcStorage::open(config).expect("reopen development DuckLake");
    assert_eq!(
        identity_rows(
            &reopened
                .query(COLUMN_IDENTITY_SQL)
                .expect("reopened identities")
        ),
        with_added
    );
    let old_table_id = with_added[0].table_id;
    let old_table_uuid = with_added[0].table_uuid.clone();
    reopened
        .execute_update("DROP TABLE quackgis.main.identity_renamed")
        .expect("drop identity table");
    reopened
        .execute_update("CREATE TABLE quackgis.main.identity_renamed(id BIGINT)")
        .expect("recreate identity table");
    let recreated = identity_rows(
        &reopened
            .query(COLUMN_IDENTITY_SQL)
            .expect("recreated identities"),
    );
    assert_eq!(recreated.len(), 1);
    assert_ne!(recreated[0].table_id, old_table_id);
    assert_ne!(recreated[0].table_uuid, old_table_uuid);
}
