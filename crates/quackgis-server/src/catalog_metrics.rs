// SPDX-License-Identifier: Apache-2.0
//! Process-local PostgreSQL DuckLake catalog provider-call accounting.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use datafusion_ducklake::metadata_provider::{
    ColumnWithTable, DataFileChange, DeleteFileChange, DuckLakeTableColumn, DuckLakeTableFile,
    FileWithTable, SchemaMetadata, SnapshotMetadata, TableMetadata, TableWithSchema,
};
use datafusion_ducklake::{MetadataProvider, Result as DuckLakeResult};

static CATALOG_READ_PROVIDER_CALLS: LazyLock<Arc<AtomicU64>> =
    LazyLock::new(|| Arc::new(AtomicU64::new(0)));

/// Return the PostgreSQL catalog metadata-provider calls observed by this process.
pub(crate) fn catalog_read_provider_calls_snapshot() -> u64 {
    CATALOG_READ_PROVIDER_CALLS.load(Ordering::Relaxed)
}

/// Counts once before each delegated `MetadataProvider` call.
///
/// QuackGIS wraps only the PostgreSQL multicatalog provider with this type. The
/// counter intentionally describes provider calls rather than PostgreSQL wire
/// roundtrips: SQLx connection and prepared-statement behavior is below this
/// instrumentation boundary.
#[derive(Debug)]
pub(crate) struct MeteredMetadataProvider {
    inner: Arc<dyn MetadataProvider>,
    counter: Arc<AtomicU64>,
}

impl MeteredMetadataProvider {
    pub(crate) fn new(inner: Arc<dyn MetadataProvider>) -> Self {
        Self {
            inner,
            counter: Arc::clone(&CATALOG_READ_PROVIDER_CALLS),
        }
    }

    #[cfg(test)]
    fn with_counter(inner: Arc<dyn MetadataProvider>, counter: Arc<AtomicU64>) -> Self {
        Self { inner, counter }
    }

    fn count_call(&self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
    }
}

impl MetadataProvider for MeteredMetadataProvider {
    fn get_current_snapshot(&self) -> DuckLakeResult<i64> {
        self.count_call();
        self.inner.get_current_snapshot()
    }

    fn get_data_path(&self) -> DuckLakeResult<String> {
        self.count_call();
        self.inner.get_data_path()
    }

    fn list_snapshots(&self) -> DuckLakeResult<Vec<SnapshotMetadata>> {
        self.count_call();
        self.inner.list_snapshots()
    }

    fn list_schemas(&self, snapshot_id: i64) -> DuckLakeResult<Vec<SchemaMetadata>> {
        self.count_call();
        self.inner.list_schemas(snapshot_id)
    }

    fn list_tables(&self, schema_id: i64, snapshot_id: i64) -> DuckLakeResult<Vec<TableMetadata>> {
        self.count_call();
        self.inner.list_tables(schema_id, snapshot_id)
    }

    fn get_table_structure(
        &self,
        table_id: i64,
        snapshot_id: i64,
    ) -> DuckLakeResult<Vec<DuckLakeTableColumn>> {
        self.count_call();
        self.inner.get_table_structure(table_id, snapshot_id)
    }

    fn get_table_files_for_select(
        &self,
        table_id: i64,
        snapshot_id: i64,
    ) -> DuckLakeResult<Vec<DuckLakeTableFile>> {
        self.count_call();
        self.inner.get_table_files_for_select(table_id, snapshot_id)
    }

    fn get_table_row_count(&self, table_id: i64, snapshot_id: i64) -> DuckLakeResult<u64> {
        self.count_call();
        self.inner.get_table_row_count(table_id, snapshot_id)
    }

    fn get_schema_by_name(
        &self,
        name: &str,
        snapshot_id: i64,
    ) -> DuckLakeResult<Option<SchemaMetadata>> {
        self.count_call();
        self.inner.get_schema_by_name(name, snapshot_id)
    }

    fn get_table_by_name(
        &self,
        schema_id: i64,
        name: &str,
        snapshot_id: i64,
    ) -> DuckLakeResult<Option<TableMetadata>> {
        self.count_call();
        self.inner.get_table_by_name(schema_id, name, snapshot_id)
    }

    fn table_exists(&self, schema_id: i64, name: &str, snapshot_id: i64) -> DuckLakeResult<bool> {
        self.count_call();
        self.inner.table_exists(schema_id, name, snapshot_id)
    }

    fn list_all_tables(&self, snapshot_id: i64) -> DuckLakeResult<Vec<TableWithSchema>> {
        self.count_call();
        self.inner.list_all_tables(snapshot_id)
    }

    fn list_all_columns(&self, snapshot_id: i64) -> DuckLakeResult<Vec<ColumnWithTable>> {
        self.count_call();
        self.inner.list_all_columns(snapshot_id)
    }

    fn list_all_files(&self, snapshot_id: i64) -> DuckLakeResult<Vec<FileWithTable>> {
        self.count_call();
        self.inner.list_all_files(snapshot_id)
    }

    fn get_data_files_added_between_snapshots(
        &self,
        table_id: i64,
        start_snapshot: i64,
        end_snapshot: i64,
    ) -> DuckLakeResult<Vec<DataFileChange>> {
        self.count_call();
        self.inner
            .get_data_files_added_between_snapshots(table_id, start_snapshot, end_snapshot)
    }

    fn get_delete_files_added_between_snapshots(
        &self,
        table_id: i64,
        start_snapshot: i64,
        end_snapshot: i64,
    ) -> DuckLakeResult<Vec<DeleteFileChange>> {
        self.count_call();
        self.inner
            .get_delete_files_added_between_snapshots(table_id, start_snapshot, end_snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::catalog::CatalogProvider;
    use datafusion::datasource::object_store::ObjectStoreUrl;
    use datafusion::prelude::SessionContext;
    use datafusion_ducklake::{DuckLakeCatalog, DuckLakeError, DuckLakeSchema};

    #[derive(Debug)]
    struct FakeProvider {
        fail_data_path: bool,
        panic_on_files: bool,
    }

    impl MetadataProvider for FakeProvider {
        fn get_current_snapshot(&self) -> DuckLakeResult<i64> {
            Ok(7)
        }

        fn get_data_path(&self) -> DuckLakeResult<String> {
            if self.fail_data_path {
                Err(DuckLakeError::Internal("fake failure".to_string()))
            } else {
                Ok("file:///".to_string())
            }
        }

        fn list_snapshots(&self) -> DuckLakeResult<Vec<SnapshotMetadata>> {
            Ok(Vec::new())
        }

        fn list_schemas(&self, _snapshot_id: i64) -> DuckLakeResult<Vec<SchemaMetadata>> {
            Ok(Vec::new())
        }

        fn list_tables(
            &self,
            _schema_id: i64,
            _snapshot_id: i64,
        ) -> DuckLakeResult<Vec<TableMetadata>> {
            Ok(Vec::new())
        }

        fn get_table_structure(
            &self,
            _table_id: i64,
            _snapshot_id: i64,
        ) -> DuckLakeResult<Vec<DuckLakeTableColumn>> {
            Ok(vec![DuckLakeTableColumn::new(
                1,
                "value".to_string(),
                "int64".to_string(),
                false,
            )])
        }

        fn get_table_files_for_select(
            &self,
            _table_id: i64,
            _snapshot_id: i64,
        ) -> DuckLakeResult<Vec<DuckLakeTableFile>> {
            assert!(!self.panic_on_files, "schema-only lookup requested files");
            Ok(Vec::new())
        }

        fn get_schema_by_name(
            &self,
            name: &str,
            _snapshot_id: i64,
        ) -> DuckLakeResult<Option<SchemaMetadata>> {
            Ok((name == "main").then(|| SchemaMetadata {
                schema_id: 1,
                schema_name: name.to_string(),
                path: name.to_string(),
                path_is_relative: true,
            }))
        }

        fn get_table_by_name(
            &self,
            _schema_id: i64,
            name: &str,
            _snapshot_id: i64,
        ) -> DuckLakeResult<Option<TableMetadata>> {
            Ok((name == "present").then(|| TableMetadata {
                table_id: 11,
                table_name: name.to_string(),
                path: name.to_string(),
                path_is_relative: true,
            }))
        }

        fn table_exists(
            &self,
            _schema_id: i64,
            _name: &str,
            _snapshot_id: i64,
        ) -> DuckLakeResult<bool> {
            Ok(false)
        }

        fn list_all_tables(&self, _snapshot_id: i64) -> DuckLakeResult<Vec<TableWithSchema>> {
            Ok(Vec::new())
        }

        fn list_all_columns(&self, _snapshot_id: i64) -> DuckLakeResult<Vec<ColumnWithTable>> {
            Ok(Vec::new())
        }

        fn list_all_files(&self, _snapshot_id: i64) -> DuckLakeResult<Vec<FileWithTable>> {
            Ok(Vec::new())
        }

        fn get_data_files_added_between_snapshots(
            &self,
            _table_id: i64,
            _start_snapshot: i64,
            _end_snapshot: i64,
        ) -> DuckLakeResult<Vec<DataFileChange>> {
            Ok(Vec::new())
        }

        fn get_delete_files_added_between_snapshots(
            &self,
            _table_id: i64,
            _start_snapshot: i64,
            _end_snapshot: i64,
        ) -> DuckLakeResult<Vec<DeleteFileChange>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn increments_once_before_successful_and_failed_calls() {
        let counter = Arc::new(AtomicU64::new(0));
        let success = MeteredMetadataProvider::with_counter(
            Arc::new(FakeProvider {
                fail_data_path: false,
                panic_on_files: false,
            }),
            Arc::clone(&counter),
        );
        assert_eq!(success.get_current_snapshot().unwrap(), 7);
        assert_eq!(counter.load(Ordering::Relaxed), 1);

        let failure = MeteredMetadataProvider::with_counter(
            Arc::new(FakeProvider {
                fail_data_path: true,
                panic_on_files: false,
            }),
            Arc::clone(&counter),
        );
        assert!(failure.get_data_path().is_err());
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn schema_only_lookup_is_exactly_two_provider_calls() {
        let counter = Arc::new(AtomicU64::new(0));
        let provider: Arc<dyn MetadataProvider> = Arc::new(MeteredMetadataProvider::with_counter(
            Arc::new(FakeProvider {
                fail_data_path: false,
                panic_on_files: true,
            }),
            Arc::clone(&counter),
        ));
        let schema = DuckLakeSchema::new(
            1,
            "main",
            provider,
            7,
            Arc::new(ObjectStoreUrl::parse("file:///").unwrap()),
            "/".to_string(),
        );

        let table_schema = schema.table_schema("present").unwrap().unwrap();
        assert_eq!(table_schema.field(0).name(), "value");
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn warm_execution_and_direct_planning_call_arithmetic_is_seven_and_four() {
        let counter = Arc::new(AtomicU64::new(0));
        let provider: Arc<dyn MetadataProvider> = Arc::new(MeteredMetadataProvider::with_counter(
            Arc::new(FakeProvider {
                fail_data_path: false,
                panic_on_files: false,
            }),
            Arc::clone(&counter),
        ));
        let catalog = DuckLakeCatalog::with_snapshot(provider, 7).unwrap();

        counter.store(0, Ordering::Relaxed);
        let schema = catalog.schema("main").unwrap();
        let schema = schema.downcast_ref::<DuckLakeSchema>().unwrap();
        assert!(schema.table_schema("present").unwrap().is_some());
        let preflight_calls = counter.load(Ordering::Relaxed);
        assert_eq!(preflight_calls, 3);

        counter.store(0, Ordering::Relaxed);
        let context = SessionContext::new();
        context.register_catalog("quackgis", Arc::new(catalog));
        context
            .sql("SELECT value FROM quackgis.main.present")
            .await
            .unwrap()
            .into_optimized_plan()
            .unwrap();
        let direct_plan_calls = counter.load(Ordering::Relaxed);
        assert_eq!(direct_plan_calls, 4);
        assert_eq!(preflight_calls + direct_plan_calls, 7);
    }
}
