// SPDX-License-Identifier: Apache-2.0
//! PostgreSQL-style `public` schema compatibility.
//!
//! DuckLake storage lives at `quackgis.main.<table>`, but clients such as QGIS
//! open layers as `public.<table>`. This schema provider preserves QuackGIS'
//! default in-memory `public` tables (compat views such as `geometry_columns`)
//! and delegates missing table lookups to DuckLake's `main` schema.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::catalog::memory::MemorySchemaProvider;
use datafusion::catalog::{SchemaProvider, TableProvider};
use datafusion::common::{DataFusionError, Result as DFResult};
use datafusion::logical_expr::TableType;
use datafusion::prelude::SessionContext;

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
        self.ducklake_main.table(name).await
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
