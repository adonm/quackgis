// SPDX-License-Identifier: Apache-2.0
//! Dynamic `geometry_columns` table provider.
//!
//! Scans the DuckLake catalog at query time to discover Binary columns that
//! hold WKB geometry data. This is how Martin, QGIS, and GeoServer discover
//! spatial tables — PostGIS populates `geometry_columns` from type metadata,
//! but since QuackGIS stores geometry as Binary (WKB), we use column-name
//! conventions to detect geometry columns.
//!
//! Convention: columns named `geom`, `geometry`, `the_geom`, `wkb_geometry`,
//! `wkb_geom`, `shape`, or `footprint` are treated as geometry columns. This
//! covers common QGIS/GDAL naming plus asset-index sidecar tables.

use std::sync::Arc;

use datafusion::arrow::array::{Int32Array, RecordBatch, StringArray};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::datasource::TableProvider;
use datafusion::datasource::memory::MemTable;
use datafusion::error::Result as DFResult;
use datafusion::execution::session_state::SessionState;
use datafusion::physical_plan::ExecutionPlan;
use datafusion::prelude::SessionContext;

/// Column names conventionally associated with WKB geometry data.
const GEOMETRY_COLUMN_NAMES: &[&str] = &[
    "geom",
    "geometry",
    "the_geom",
    "wkb_geometry",
    "wkb_geom",
    "shape",
    "footprint",
    "way", // OpenStreetMap convention
];

/// Catalogs scanned by the provider. The DuckLake catalog `quackgis` is the
/// authoritative spatial store. Its internal `main` schema is exposed as
/// PostgreSQL-compatible `public` metadata for clients.
const SCANNED_CATALOGS: &[&str] = &["quackgis"];

#[derive(Debug)]
pub struct GeometryColumnsProvider {
    schema: SchemaRef,
}

impl GeometryColumnsProvider {
    pub fn new() -> Self {
        Self {
            schema: crate::postgis_compat::geometry_columns_schema(),
        }
    }

    /// Scan all catalogs/schemas for tables with Binary columns matching the
    /// geometry-name convention.
    async fn collect_rows(&self, session: &dyn Session) -> DFResult<RecordBatch> {
        let mut catalogs_arr: Vec<Option<String>> = Vec::new();
        let mut schemas_arr: Vec<Option<String>> = Vec::new();
        let mut tables_arr: Vec<Option<String>> = Vec::new();
        let mut cols_arr: Vec<Option<String>> = Vec::new();
        let mut dims_arr: Vec<Option<i32>> = Vec::new();
        let mut srids_arr: Vec<Option<i32>> = Vec::new();
        let mut types_arr: Vec<Option<String>> = Vec::new();

        // Downcast to SessionState to access catalog_list().
        let state = session
            .as_any()
            .downcast_ref::<SessionState>()
            .ok_or_else(|| {
                datafusion::error::DataFusionError::Internal(
                    "GeometryColumnsProvider requires SessionState".into(),
                )
            })?;
        let catalog_list = state.catalog_list();

        for &catalog_name in SCANNED_CATALOGS {
            let Some(catalog) = catalog_list.catalog(catalog_name) else {
                continue;
            };
            for schema_name in catalog.schema_names() {
                let Some(schema) = catalog.schema(&schema_name) else {
                    continue;
                };
                for table_name in schema.table_names() {
                    let Some(table) = schema.table(&table_name).await? else {
                        continue;
                    };
                    let table_schema = table.schema();
                    for field in table_schema.fields() {
                        if !is_binary_type(field.data_type()) {
                            continue;
                        }
                        let col_name = field.name();
                        if !is_geometry_column_name(col_name) {
                            continue;
                        }
                        catalogs_arr.push(Some(catalog_name.to_string()));
                        let exposed_schema = if catalog_name == "quackgis" && schema_name == "main"
                        {
                            "public"
                        } else {
                            schema_name.as_str()
                        };
                        schemas_arr.push(Some(exposed_schema.to_string()));
                        tables_arr.push(Some(table_name.clone()));
                        cols_arr.push(Some(col_name.clone()));
                        dims_arr.push(Some(2));
                        srids_arr.push(Some(0));
                        types_arr.push(Some("GEOMETRY".to_string()));
                    }
                }
            }
        }

        let batch = RecordBatch::try_new(
            self.schema.clone(),
            vec![
                Arc::new(StringArray::from(catalogs_arr)),
                Arc::new(StringArray::from(schemas_arr)),
                Arc::new(StringArray::from(tables_arr)),
                Arc::new(StringArray::from(cols_arr)),
                Arc::new(Int32Array::from(dims_arr)),
                Arc::new(Int32Array::from(srids_arr)),
                Arc::new(StringArray::from(types_arr)),
            ],
        )?;
        Ok(batch)
    }
}

impl Default for GeometryColumnsProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn is_binary_type(dt: &datafusion::arrow::datatypes::DataType) -> bool {
    use datafusion::arrow::datatypes::DataType;
    matches!(
        dt,
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView
    )
}

pub(crate) fn is_geometry_column_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    GEOMETRY_COLUMN_NAMES.contains(&lower.as_str())
}

#[async_trait::async_trait]
impl TableProvider for GeometryColumnsProvider {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    fn table_type(&self) -> datafusion::datasource::TableType {
        datafusion::datasource::TableType::View
    }

    async fn scan(
        &self,
        session: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[datafusion::logical_expr::Expr],
        limit: Option<usize>,
    ) -> DFResult<Arc<dyn ExecutionPlan>> {
        let batch = self.collect_rows(session).await?;
        let mem_table = MemTable::try_new(self.schema.clone(), vec![vec![batch]])?;
        mem_table.scan(session, projection, filters, limit).await
    }
}

/// Register the `geometry_columns` table on the given context. Must be called
/// AFTER the DuckLake catalog is registered so `SCANNED_CATALOGS` resolves.
pub fn register_geometry_columns(ctx: &SessionContext) -> DFResult<()> {
    let provider = Arc::new(GeometryColumnsProvider::new()) as Arc<dyn TableProvider>;
    ctx.register_table("geometry_columns", provider)?;
    Ok(())
}
