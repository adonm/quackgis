// SPDX-License-Identifier: Apache-2.0
//! SessionContext construction: SedonaDB function catalog + datafusion-postgres
//! pg_catalog emulation on top of a single DataFusion `SessionContext`.
//!
//! This is the integration point of the three upstream pillars. Everything
//! not owned by quackgis lives behind the calls in [`build_session_context`].

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use datafusion::prelude::SessionContext;
use datafusion_postgres::auth::AuthManager;
use datafusion_postgres::datafusion_pg_catalog::setup_pg_catalog;
use sedona::context::SedonaContext;

/// Build the QuackGIS session context: SedonaDB function catalog + pg_catalog
/// emulation + information_schema.
///
/// `catalog_name` is the catalog into which pg_catalog is attached as a
/// schema. Must already exist on the `SessionContext` — DataFusion's default
/// catalog is `"datafusion"`, which is what we use until M1 when DuckLake
/// gets its own catalog (`"quackgis"` branding comes then).
pub async fn build_session_context() -> Result<Arc<SessionContext>> {
    let sctx = SedonaContext::new_local_interactive()
        .await
        .context("SedonaContext initialization failed")?;

    let ctx = Arc::new(sctx.ctx);

    // pg_catalog needs a context provider; AuthManager implements it. M0 uses
    // the default AuthManager (postgres superuser, empty password) — RBAC
    // arrives at M6.
    let auth_manager = Arc::new(AuthManager::new());
    setup_pg_catalog(&ctx, "datafusion", auth_manager)
        .map_err(|e| anyhow!("setup_pg_catalog failed: {e}"))?;

    Ok(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke: context builds, exposes SedonaDB ST_* functions, and answers
    /// spatial SQL. This is the same gate as the M0 wire spike but driven
    /// in-process — catches regressions in either upstream without needing
    /// psql on the host.
    #[tokio::test]
    async fn context_executes_spatial_sql() {
        let ctx = build_session_context().await.expect("context builds");

        let point = ctx
            .sql("SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))")
            .await
            .expect("parse + plan")
            .collect()
            .await
            .expect("execute");

        let rendered = datafusion::arrow::util::pretty::pretty_format_batches(&point)
            .expect("render")
            .to_string();
        assert!(
            rendered.contains("POINT(1 2)"),
            "expected POINT(1 2) in output, got:\n{rendered}"
        );
    }
}
