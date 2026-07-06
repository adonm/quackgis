// SPDX-License-Identifier: Apache-2.0
//! SessionContext construction: SedonaDB function catalog, DuckLake storage,
//! and datafusion-postgres pg_catalog emulation on top of one DataFusion
//! `SessionContext`.
//!
//! This is the integration point of the four upstream pillars. Everything
//! not owned by quackgis lives behind the calls in [`build_session_context`].

use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use datafusion::execution::runtime_env::RuntimeEnv;
use datafusion::execution::session_state::SessionStateBuilder;
use datafusion::prelude::{SessionConfig, SessionContext};
use datafusion_ducklake::{
    DuckLakeCatalog, MetadataWriter, SqliteMetadataProvider, SqliteMetadataWriter,
};
use datafusion_postgres::auth::AuthManager;
use datafusion_postgres::datafusion_pg_catalog::setup_pg_catalog;

/// Default name of the DuckLake catalog as seen by clients. Persisted tables
/// live under `quackgis.main.<table>`. The default catalog for unqualified
/// names remains `"datafusion"` (in-memory) so `setup_pg_catalog` can attach
/// `pg_catalog` to it — DuckLake's catalog rejects schema registration.
pub const DUCKLAKE_CATALOG: &str = "quackgis";

/// Where the DuckLake SQLite catalog file lives and the Parquet data files
/// live underneath. Both are configurable so callers (CLI, tests) can place
/// them anywhere — typically a tempdir per test, and a PVC path in prod.
#[derive(Debug, Clone)]
pub struct StoragePaths {
    /// SQLite connection string of the form `sqlite:<path>?mode=rwc`. The
    /// `mode=rwc` lets the writer create the file on first run.
    pub catalog_conn: String,
    /// Absolute filesystem path under which Parquet data files are stored.
    /// Created if missing.
    pub data_path: String,
}

impl StoragePaths {
    /// Production defaults: `quackgis.db` and `./data/` relative to CWD.
    /// Override via `QUACKGIS_CATALOG_PATH` and `QUACKGIS_DATA_PATH`.
    pub fn from_env_or_defaults() -> Result<Self> {
        let catalog_path =
            std::env::var("QUACKGIS_CATALOG_PATH").unwrap_or_else(|_| "quackgis.db".to_string());
        let data_path =
            std::env::var("QUACKGIS_DATA_PATH").unwrap_or_else(|_| "./data".to_string());
        Self::new(&catalog_path, &data_path)
    }

    /// Construct from explicit paths. `catalog_path` is a filesystem path to
    /// the SQLite file; `data_path` is the directory for Parquet data.
    /// Both are resolved to absolute paths; the data dir is created if
    /// missing.
    pub fn new(catalog_path: &str, data_path: &str) -> Result<Self> {
        let abs_catalog = std::path::Path::new(catalog_path)
            .canonicalize()
            .or_else(|_| {
                // canonicalize fails if the file doesn't exist yet. Resolve
                // via parent dir + file_name to still produce an absolute
                // path the SQLite driver will create on demand.
                let p = std::path::Path::new(catalog_path);
                let parent = p
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|p| std::path::Path::new(".").join(p))
                    .unwrap_or_else(|| std::path::Path::new(".").to_path_buf());
                let abs_parent = parent
                    .canonicalize()
                    .with_context(|| format!("canonicalizing parent of {catalog_path}"))?;
                let file_name = p
                    .file_name()
                    .ok_or_else(|| anyhow!("invalid catalog path: {catalog_path}"))?;
                Ok::<_, anyhow::Error>(abs_parent.join(file_name))
            })?;
        let abs_data = {
            std::fs::create_dir_all(data_path)
                .with_context(|| format!("creating data dir {data_path}"))?;
            std::path::Path::new(data_path)
                .canonicalize()
                .with_context(|| format!("canonicalizing data path {data_path}"))?
        };

        let catalog_conn = format!("sqlite:{}?mode=rwc", abs_catalog.display());
        let data_path = abs_data.to_string_lossy().to_string();
        Ok(Self {
            catalog_conn,
            data_path,
        })
    }
}

/// Build the QuarkGIS session context: SedonaDB function catalog + DuckLake
/// storage + pg_catalog emulation + information_schema.
///
/// Construction order matters:
///   1. Build the DuckLake catalog with write support (SQLite backend).
///   2. Construct a DataFusion SessionContext with DuckLake as the default
///      catalog, then register the narrow SedonaDB function surface that
///      QuackGIS needs: base functions, pure-Rust geo kernels, and Rust PROJ.
///   3. Attach pg_catalog as a schema in the default catalog so introspection
///      queries (`SELECT * FROM pg_catalog.pg_class`) work.
pub async fn build_session_context() -> Result<Arc<SessionContext>> {
    build_session_context_with_storage(StoragePaths::from_env_or_defaults()?).await
}

/// Same as [`build_session_context`] but with explicit storage paths. Used by
/// tests so each test gets an isolated DuckLake in its own tempdir.
pub async fn build_session_context_with_storage(
    paths: StoragePaths,
) -> Result<Arc<SessionContext>> {
    // 0. Configure the pure-Rust CRS engine before any SedonaDB functions
    //    are registered. This replaces libproj with proj-wkt/proj-core.
    //    Idempotent: safe to call multiple times (subsequent calls no-op).
    let _ = sedona_proj::rust_engine::configure_global_rust_engine();

    // 1. DuckLake: writer creates the catalog schema if missing, then a
    //    snapshot is required before any read or write can happen.
    let writer = SqliteMetadataWriter::new_with_init(&paths.catalog_conn)
        .await
        .map_err(|e| anyhow!("SqliteMetadataWriter init: {e}"))?;
    writer
        .set_data_path(&paths.data_path)
        .map_err(|e| anyhow!("set_data_path: {e}"))?;
    let initial_snapshot = writer
        .create_snapshot()
        .map_err(|e| anyhow!("create_snapshot: {e}"))?;

    // Pre-create the `main` schema so SQL like `quackgis.main.<table>` can
    // resolve at plan time before any CREATE TABLE has run. DuckLakeCatalog
    // rejects CREATE SCHEMA, but the writer's get_or_create_schema is the
    // internal API the insert path uses to make this row.
    let _ = writer
        .get_or_create_schema("main", None, initial_snapshot)
        .map_err(|e| anyhow!("get_or_create_schema(main): {e}"))?;

    let provider = SqliteMetadataProvider::new(&paths.catalog_conn)
        .await
        .map_err(|e| anyhow!("SqliteMetadataProvider: {e}"))?;
    let ducklake = DuckLakeCatalog::with_writer(Arc::new(provider), Arc::new(writer))
        .map_err(|e| anyhow!("DuckLakeCatalog::with_writer: {e}"))?;

    // 2. SessionContext. Keep "datafusion" as the default catalog (it's the
    //    in-memory one, where setup_pg_catalog can attach the pg_catalog
    //    schema — DuckLake rejects schema registration). DuckLake is
    //    registered alongside as a separate catalog; persisted tables are
    //    accessed as `quackgis.main.<table>`. information_schema on.
    let config = SessionConfig::new().with_information_schema(true);
    let runtime = Arc::new(RuntimeEnv::default());
    let state = SessionStateBuilder::new()
        .with_default_features()
        .with_config(config)
        .with_runtime_env(runtime)
        .build();
    let ctx = SessionContext::new_with_state(state);
    ctx.register_catalog(DUCKLAKE_CATALOG, Arc::new(ducklake));

    register_sedona_function_catalog(&ctx)
        .map_err(|e| anyhow!("register SedonaDB function catalog failed: {e}"))?;
    let ctx = Arc::new(ctx);

    // Expose DuckLake `quackgis.main` tables as PostgreSQL-style `public.*`
    // before pg_catalog is installed so QGIS pg_class/pg_namespace probes see
    // the alias schema.
    crate::public_schema::register_public_schema_alias(&ctx)
        .map_err(|e| anyhow!("register_public_schema_alias failed: {e}"))?;

    // 3. pg_catalog attached to the in-memory "datafusion" catalog. Default
    //    AuthManager has the single 'postgres' role; RBAC arrives at M6.
    let auth_manager = Arc::new(AuthManager::new());
    setup_pg_catalog(&ctx, "datafusion", auth_manager)
        .map_err(|e| anyhow!("setup_pg_catalog failed: {e}"))?;

    crate::postgis_compat::register_postgis_compat(&ctx)
        .map_err(|e| anyhow!("register_postgis_compat failed: {e}"))?;

    crate::geometry_columns::register_geometry_columns(&ctx)
        .map_err(|e| anyhow!("register_geometry_columns failed: {e}"))?;

    crate::spatial_udfs::register_spatial_udfs(&ctx)
        .map_err(|e| anyhow!("register_spatial_udfs failed: {e}"))?;

    Ok(ctx)
}

fn register_sedona_function_catalog(ctx: &SessionContext) -> datafusion::common::Result<()> {
    let mut functions = sedona_functions::register::default_function_set();

    for (name, kernels) in sedona_geo::register::scalar_kernels() {
        functions.add_scalar_udf_impl(name, kernels)?;
    }
    for (name, kernel) in sedona_geo::register::aggregate_kernels() {
        functions.add_aggregate_udf_kernel(name, kernel)?;
    }
    for (name, kernel) in sedona_proj::register::scalar_kernels() {
        functions.add_scalar_udf_impl(name, kernel)?;
    }

    for udf in functions.scalar_udfs() {
        ctx.register_udf(udf.clone().into());
    }
    for udaf in functions.aggregate_udfs() {
        ctx.register_udaf(udaf.clone().into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke: context builds, exposes SedonaDB ST_* functions, and answers
    /// spatial SQL. Same gate as the M0 wire spike but driven in-process —
    /// catches regressions in either upstream without needing psql on the host.
    #[tokio::test(flavor = "multi_thread")]
    async fn context_executes_spatial_sql() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let paths = StoragePaths::new(
            tmp.path().join("quackgis.db").to_str().unwrap(),
            tmp.path().join("data").to_str().unwrap(),
        )
        .expect("paths");
        let ctx = build_session_context_with_storage(paths)
            .await
            .expect("context builds");

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
