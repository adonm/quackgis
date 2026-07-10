// SPDX-License-Identifier: Apache-2.0
//! Dynamic `geometry_columns` and `geography_columns` table providers.
//!
//! Scans the DuckLake catalog at query time to discover Binary WKB/EWKB fields.
//! Validated family metadata wins; conventional names remain a migration
//! fallback for catalogs written before durable family identity.

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
use datafusion_postgres::arrow_pg::datatypes::{SpatialFamily, classify_spatial_field};

/// Catalogs scanned by the provider. The DuckLake catalog `quackgis` is the
/// authoritative spatial store. Its internal `main` schema is exposed as
/// PostgreSQL-compatible `public` metadata for clients.
const SCANNED_CATALOGS: &[&str] = &["quackgis"];

#[derive(Debug)]
pub struct GeometryColumnsProvider {
    schema: SchemaRef,
    family: SpatialFamily,
}

impl GeometryColumnsProvider {
    pub fn new() -> Self {
        Self {
            schema: crate::postgis_compat::geometry_columns_schema(),
            family: SpatialFamily::Geometry,
        }
    }

    fn geography() -> Self {
        Self {
            schema: crate::postgis_compat::geography_columns_schema(),
            family: SpatialFamily::Geography,
        }
    }

    /// Scan all catalogs/schemas for fields in this provider's spatial family.
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
                        let col_name = field.name();
                        if classify_spatial_field(field) != Some(self.family) {
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
                        types_arr.push(Some(self.family.as_str().to_ascii_uppercase()));
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

/// Register both spatial-family metadata tables. Must be called
/// AFTER the DuckLake catalog is registered so `SCANNED_CATALOGS` resolves.
pub fn register_geometry_columns(ctx: &SessionContext) -> DFResult<()> {
    let provider = Arc::new(GeometryColumnsProvider::new()) as Arc<dyn TableProvider>;
    ctx.register_table("geometry_columns", provider)?;
    let provider = Arc::new(GeometryColumnsProvider::geography()) as Arc<dyn TableProvider>;
    ctx.register_table("geography_columns", provider)?;
    Ok(())
}
