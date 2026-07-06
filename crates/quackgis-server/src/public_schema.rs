// SPDX-License-Identifier: Apache-2.0
//! PostgreSQL-style `public` schema compatibility.
//!
//! DuckLake storage lives at `quackgis.main.<table>`, but clients such as QGIS
//! open layers as `public.<table>`. This schema provider preserves QuackGIS'
//! default in-memory `public` tables (compat views such as `geometry_columns`)
//! and delegates missing table lookups to DuckLake's `main` schema.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::catalog::Session;
use datafusion::catalog::memory::MemorySchemaProvider;
use datafusion::catalog::{SchemaProvider, TableProvider};
use datafusion::common::{DataFusionError, Result as DFResult};
use datafusion::execution::session_state::SessionState;
use datafusion::logical_expr::{Expr, LogicalPlanBuilder, TableProviderFilterPushDown, TableType};
use datafusion::physical_plan::ExecutionPlan;
use datafusion::prelude::SessionContext;

use crate::catalog_compat::SYNTHETIC_ROWID_COLUMN;
use crate::context::DUCKLAKE_CATALOG;

#[derive(Debug)]
struct PublicSchemaAlias {
    base: Arc<dyn SchemaProvider>,
    ducklake_main: Arc<dyn SchemaProvider>,
}

#[async_trait]
impl SchemaProvider for PublicSchemaAlias {
    fn table_names(&self) -> Vec<String> {
        let mut names = self.base.table_names();
        for name in self.ducklake_main.table_names() {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names.sort();
        names
    }

    async fn table(&self, name: &str) -> Result<Option<Arc<dyn TableProvider>>, DataFusionError> {
        if let Some(table) = self.base.table(name).await? {
            return Ok(Some(table));
        }
        let Some(table) = self.ducklake_main.table(name).await? else {
            return Ok(None);
        };
        if needs_virtual_rowid(table.schema().as_ref()) {
            return Ok(Some(Arc::new(RowIdTableAlias::new(name, table.schema()))));
        }
        Ok(Some(table))
    }

    async fn table_type(&self, name: &str) -> Result<Option<TableType>, DataFusionError> {
        if let Some(table_type) = self.base.table_type(name).await? {
            return Ok(Some(table_type));
        }
        self.ducklake_main.table_type(name).await
    }

    fn register_table(
        &self,
        name: String,
        table: Arc<dyn TableProvider>,
    ) -> DFResult<Option<Arc<dyn TableProvider>>> {
        self.base.register_table(name, table)
    }

    fn deregister_table(&self, name: &str) -> DFResult<Option<Arc<dyn TableProvider>>> {
        self.base.deregister_table(name)
    }

    fn table_exist(&self, name: &str) -> bool {
        self.base.table_exist(name) || self.ducklake_main.table_exist(name)
    }
}

#[derive(Debug)]
struct RowIdTableAlias {
    table_name: String,
    schema: SchemaRef,
}

impl RowIdTableAlias {
    fn new(table_name: &str, base_schema: SchemaRef) -> Self {
        let mut fields = vec![Arc::new(Field::new(
            SYNTHETIC_ROWID_COLUMN,
            DataType::Int64,
            false,
        ))];
        fields.extend(base_schema.fields().iter().cloned());
        Self {
            table_name: table_name.to_string(),
            schema: Arc::new(Schema::new(fields)),
        }
    }

    fn sql(&self) -> String {
        format!(
            "SELECT CAST(ROW_NUMBER() OVER () AS BIGINT) AS {}, * FROM {DUCKLAKE_CATALOG}.main.{}",
            quote_ident(SYNTHETIC_ROWID_COLUMN),
            quote_ident(&self.table_name)
        )
    }
}

#[async_trait]
impl TableProvider for RowIdTableAlias {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::View
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DFResult<Vec<TableProviderFilterPushDown>> {
        Ok(vec![TableProviderFilterPushDown::Exact; filters.len()])
    }

    async fn scan(
        &self,
        session: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DFResult<Arc<dyn ExecutionPlan>> {
        let state = session
            .as_any()
            .downcast_ref::<SessionState>()
            .ok_or_else(|| {
                DataFusionError::Internal("RowIdTableAlias requires SessionState".into())
            })?;
        let mut plan = LogicalPlanBuilder::from(state.create_logical_plan(&self.sql()).await?);

        if let Some(filter) = filters.iter().cloned().reduce(|acc, expr| acc.and(expr)) {
            plan = plan.filter(filter)?;
        }

        let mut plan = if let Some(projection) = projection {
            let exprs = projection
                .iter()
                .map(|idx| {
                    let field = self.schema.field(*idx);
                    Expr::Column(datafusion::common::Column::from_name(field.name().clone()))
                })
                .collect::<Vec<_>>();
            plan.project(exprs)?
        } else {
            plan
        };

        if let Some(limit) = limit {
            plan = plan.limit(0, Some(limit))?;
        }

        session.create_physical_plan(&plan.build()?).await
    }
}

pub(crate) fn needs_virtual_rowid(schema: &Schema) -> bool {
    has_spatial_column(schema)
        && !has_field(schema, "id")
        && !has_field(schema, SYNTHETIC_ROWID_COLUMN)
}

fn has_spatial_column(schema: &Schema) -> bool {
    schema
        .fields()
        .iter()
        .any(|field| crate::geometry_columns::is_geometry_column_name(field.name()))
}

fn has_field(schema: &Schema, name: &str) -> bool {
    schema
        .fields()
        .iter()
        .any(|field| field.name().eq_ignore_ascii_case(name))
}

fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub fn register_public_schema_alias(ctx: &SessionContext) -> DFResult<()> {
    let datafusion_catalog = ctx.catalog("datafusion").ok_or_else(|| {
        DataFusionError::Internal("missing datafusion catalog for public schema alias".into())
    })?;
    let ducklake_catalog = ctx.catalog(DUCKLAKE_CATALOG).ok_or_else(|| {
        DataFusionError::Internal("missing quackgis catalog for public schema alias".into())
    })?;
    let ducklake_main = ducklake_catalog.schema("main").ok_or_else(|| {
        DataFusionError::Internal("missing quackgis.main schema for public schema alias".into())
    })?;

    let existing_public = datafusion_catalog
        .schema("public")
        .unwrap_or_else(|| Arc::new(MemorySchemaProvider::new()));
    let base = existing_public
        .downcast_ref::<PublicSchemaAlias>()
        .map(|alias| Arc::clone(&alias.base))
        .unwrap_or(existing_public);

    datafusion_catalog.register_schema(
        "public",
        Arc::new(PublicSchemaAlias {
            base,
            ducklake_main,
        }),
    )?;
    Ok(())
}
