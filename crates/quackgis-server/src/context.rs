// SPDX-License-Identifier: Apache-2.0
//! SessionContext construction: SedonaDB function catalog, DuckLake storage,
//! and datafusion-postgres pg_catalog emulation on top of one DataFusion
//! `SessionContext`.
//!
//! This is the integration point of the four upstream pillars. Everything
//! not owned by quackgis lives behind the calls in [`build_session_context`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use chrono::{DateTime, Utc};
use datafusion::execution::runtime_env::RuntimeEnv;
use datafusion::execution::session_state::SessionStateBuilder;
use datafusion::prelude::{SessionConfig, SessionContext};
use datafusion_ducklake::maintenance::{
    CleanupCriteria, delete_orphaned_files_multicatalog, delete_orphaned_files_sqlite,
};
use datafusion_ducklake::{
    DuckLakeCatalog, MetadataProvider, MetadataWriter, MulticatalogManager, MulticatalogProvider,
    PostgresMetadataWriter, SqliteMetadataProvider, SqliteMetadataWriter,
    initialize_multicatalog_schema,
    table_functions::{
        DucklakeListFilesFunction, DucklakeSnapshotsFunction, DucklakeTableInfoFunction,
    },
};
use datafusion_postgres::auth::AuthManager;
use datafusion_postgres::datafusion_pg_catalog::pg_catalog::context::{
    Grant, Permission, ResourceType, Role, User,
};
use datafusion_postgres::datafusion_pg_catalog::setup_pg_catalog;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt};
use sqlx::postgres::{PgPool, PgPoolOptions};
use tokio::sync::OnceCell;
use url::Url;

use crate::auth::{AccessRole, AuthConfig, AuthMode};
use crate::catalog_metrics::MeteredMetadataProvider;

/// Default name of the DuckLake catalog as seen by clients. Persisted tables
/// live under `quackgis.main.<table>`. The default catalog for unqualified
/// names remains `"datafusion"` (in-memory) so `setup_pg_catalog` can attach
/// `pg_catalog` to it — DuckLake's catalog rejects schema registration.
pub const DUCKLAKE_CATALOG: &str = "quackgis";
const TARGET_PARTITIONS_ENV: &str = "QUACKGIS_TARGET_PARTITIONS";

/// DuckLake storage profile. Local development defaults to SQLite catalog +
/// filesystem Parquet; Kind Alpha can use a PostgreSQL catalog + S3-compatible
/// object storage while keeping the same DuckLake table layout.
#[derive(Debug, Clone)]
pub struct StoragePaths {
    profile: StorageProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanQuarantineEntry {
    pub source: String,
    pub quarantine: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanQuarantineReport {
    pub dry_run: bool,
    pub candidates: Vec<OrphanQuarantineEntry>,
    pub copied_count: usize,
    pub deleted_count: usize,
}

#[derive(Debug, Clone)]
enum StorageProfile {
    SqliteLocal {
        /// SQLite connection string of the form `sqlite:<path>?mode=rwc`. The
        /// `mode=rwc` lets the writer create the file on first run.
        catalog_conn: String,
        /// Absolute filesystem path under which Parquet data files are stored.
        /// Created if missing.
        data_path: String,
    },
    Postgres {
        catalog_url: String,
        catalog_name: String,
        data_path: String,
        s3: Option<S3StorageOptions>,
        pool: Arc<OnceCell<PgPool>>,
        catalog_id: Arc<OnceCell<i64>>,
    },
}

#[derive(Debug, Clone)]
pub struct S3StorageOptions {
    pub endpoint: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub region: String,
    pub allow_http: bool,
}

impl StoragePaths {
    /// Production defaults: `quackgis.db` and `./data/` relative to CWD.
    /// Override via `QUACKGIS_CATALOG_PATH` and `QUACKGIS_DATA_PATH`.
    pub fn from_env_or_defaults() -> Result<Self> {
        if let Ok(catalog_url) = std::env::var("QUACKGIS_CATALOG_URL") {
            let catalog_name = std::env::var("QUACKGIS_DUCKLAKE_CATALOG_NAME")
                .unwrap_or_else(|_| "quackgis".to_string());
            let data_path =
                std::env::var("QUACKGIS_DATA_PATH").unwrap_or_else(|_| "./data".to_string());
            let s3 = S3StorageOptions::from_env()?;
            return Self::postgres(catalog_url, catalog_name, data_path, s3);
        }

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
            profile: StorageProfile::SqliteLocal {
                catalog_conn,
                data_path,
            },
        })
    }

    /// Construct a PostgreSQL-backed DuckLake metadata profile. `data_path` may
    /// be local or object-store backed; for the Kind Alpha path it is an
    /// S3-compatible URL such as `s3://quackgis/ducklake`.
    pub fn postgres(
        catalog_url: String,
        catalog_name: String,
        data_path: String,
        s3: Option<S3StorageOptions>,
    ) -> Result<Self> {
        if catalog_url.trim().is_empty() {
            return Err(anyhow!("PostgreSQL catalog URL cannot be empty"));
        }
        if catalog_name.trim().is_empty() {
            return Err(anyhow!("DuckLake catalog name cannot be empty"));
        }
        if data_path.trim().is_empty() {
            return Err(anyhow!("DuckLake data path cannot be empty"));
        }
        if data_path.to_ascii_lowercase().starts_with("s3://") && s3.is_none() {
            return Err(anyhow!("s3:// data paths require S3 client configuration"));
        }

        Ok(Self {
            profile: StorageProfile::Postgres {
                catalog_url,
                catalog_name,
                data_path,
                s3,
                pool: Arc::new(OnceCell::new()),
                catalog_id: Arc::new(OnceCell::new()),
            },
        })
    }

    pub fn data_path(&self) -> &str {
        match &self.profile {
            StorageProfile::SqliteLocal { data_path, .. }
            | StorageProfile::Postgres { data_path, .. } => data_path,
        }
    }

    pub fn is_shared_catalog(&self) -> bool {
        matches!(self.profile, StorageProfile::Postgres { .. })
    }

    /// Inventory old unreferenced Parquet candidates without deleting anything.
    ///
    /// The cutoff is mandatory at the operator boundary so in-flight prewrites
    /// newer than it are excluded. PostgreSQL multicatalog inventory is global to
    /// the shared data path and subtracts references from every catalog.
    pub async fn orphan_candidates_before(&self, cutoff: DateTime<Utc>) -> Result<Vec<String>> {
        let object_store = self.object_store()?;
        match &self.profile {
            StorageProfile::SqliteLocal { catalog_conn, .. } => {
                let catalog_path = catalog_conn
                    .strip_prefix("sqlite:")
                    .and_then(|value| value.split('?').next())
                    .ok_or_else(|| anyhow!("invalid SQLite catalog connection string"))?;
                if !std::path::Path::new(catalog_path).is_file() {
                    return Err(anyhow!(
                        "refusing orphan inventory because SQLite catalog does not exist: {catalog_path}"
                    ));
                }
                let writer = SqliteMetadataWriter::new(catalog_conn)
                    .await
                    .map_err(|e| anyhow!("open SQLite catalog for orphan inventory: {e}"))?;
                delete_orphaned_files_sqlite(
                    &writer,
                    object_store,
                    CleanupCriteria::OlderThan(cutoff),
                    true,
                )
                .await
                .map_err(|e| anyhow!("inventory SQLite orphan candidates: {e}"))
            }
            StorageProfile::Postgres { catalog_url, .. } => {
                // Do not initialize schemas or create a catalog in an inventory
                // operation. Missing maintenance metadata must fail closed.
                let pool = PgPoolOptions::new()
                    .max_connections(2)
                    .connect(catalog_url)
                    .await
                    .with_context(|| "connecting to PostgreSQL for orphan inventory")?;
                let manager = MulticatalogManager::new(pool);
                delete_orphaned_files_multicatalog(
                    &manager,
                    object_store,
                    CleanupCriteria::OlderThan(cutoff),
                    true,
                )
                .await
                .map_err(|e| anyhow!("inventory multicatalog orphan candidates: {e}"))
            }
        }
    }

    /// Quarantine old unreferenced Parquet candidates outside the live data
    /// prefix. The default dry-run mode only returns the planned source ->
    /// quarantine paths; apply mode copies first and deletes the source only
    /// after rechecking that it is still an orphan candidate.
    pub async fn quarantine_orphan_candidates_before(
        &self,
        cutoff: DateTime<Utc>,
        quarantine_prefix: &str,
        apply: bool,
    ) -> Result<OrphanQuarantineReport> {
        let quarantine_prefix = self.quarantine_prefix_key(quarantine_prefix)?;
        let mut candidates = self.orphan_candidates_before(cutoff).await?;
        candidates.sort();
        let candidates = candidates
            .into_iter()
            .map(|source| {
                let source_key = object_key_from_absolute_style(&source)?;
                let quarantine_key = quarantine_destination_key(&quarantine_prefix, &source_key);
                Ok(OrphanQuarantineEntry {
                    source,
                    quarantine: absolute_style_path(&quarantine_key),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        if !apply {
            return Ok(OrphanQuarantineReport {
                dry_run: true,
                candidates,
                copied_count: 0,
                deleted_count: 0,
            });
        }

        let object_store = self.object_store()?;
        let expected: BTreeSet<_> = candidates
            .iter()
            .map(|entry| entry.source.clone())
            .collect();
        let fresh: BTreeSet<_> = self
            .orphan_candidates_before(cutoff)
            .await?
            .into_iter()
            .collect();
        if expected != fresh {
            return Err(anyhow!(
                "refusing orphan quarantine because candidate set changed during preflight"
            ));
        }

        let mut copied_count = 0;
        let mut deleted_count = 0;
        for entry in &candidates {
            let source_key = object_key_from_absolute_style(&entry.source)?;
            let quarantine_key = object_key_from_absolute_style(&entry.quarantine)?;
            match object_store.head(&quarantine_key).await {
                Ok(_) => {
                    return Err(anyhow!(
                        "refusing to overwrite existing quarantine object {}",
                        entry.quarantine
                    ));
                }
                Err(object_store::Error::NotFound { .. }) => {}
                Err(error) => return Err(error).context("checking quarantine destination"),
            }
            self.ensure_local_parent_for_key(&quarantine_key)?;
            object_store
                .copy(&source_key, &quarantine_key)
                .await
                .with_context(|| {
                    format!(
                        "copy orphan candidate {} to quarantine {}",
                        entry.source, entry.quarantine
                    )
                })?;
            copied_count += 1;

            let latest: BTreeSet<_> = self
                .orphan_candidates_before(cutoff)
                .await?
                .into_iter()
                .collect();
            if !latest.contains(&entry.source) {
                return Err(anyhow!(
                    "refusing to remove {} because it is no longer an orphan candidate after copy",
                    entry.source
                ));
            }
            match object_store.delete(&source_key).await {
                Ok(()) => deleted_count += 1,
                Err(object_store::Error::NotFound { .. }) => {}
                Err(error) => return Err(error).context("removing quarantined orphan source"),
            }
        }

        Ok(OrphanQuarantineReport {
            dry_run: false,
            candidates,
            copied_count,
            deleted_count,
        })
    }

    pub async fn metadata_writer(&self) -> Result<Arc<dyn MetadataWriter>> {
        match &self.profile {
            StorageProfile::SqliteLocal {
                catalog_conn,
                data_path,
            } => {
                let writer = SqliteMetadataWriter::new_with_init(catalog_conn)
                    .await
                    .map_err(|e| anyhow!("SqliteMetadataWriter init: {e}"))?;
                writer
                    .set_data_path(data_path)
                    .map_err(|e| anyhow!("set_data_path: {e}"))?;
                Ok(Arc::new(writer))
            }
            StorageProfile::Postgres {
                catalog_url,
                catalog_name,
                data_path,
                pool,
                catalog_id,
                ..
            } => {
                let pool = postgres_pool(catalog_url, pool).await?;
                let catalog_id = postgres_catalog_id(&pool, catalog_name, catalog_id).await?;
                let writer = PostgresMetadataWriter::with_pool(pool, catalog_id)
                    .await
                    .map_err(|e| anyhow!("PostgresMetadataWriter init: {e}"))?;
                writer
                    .set_data_path(data_path)
                    .map_err(|e| anyhow!("set_data_path: {e}"))?;
                Ok(Arc::new(writer))
            }
        }
    }

    pub async fn metadata_provider(&self) -> Result<Arc<dyn MetadataProvider>> {
        match &self.profile {
            StorageProfile::SqliteLocal { catalog_conn, .. } => {
                let provider = SqliteMetadataProvider::new(catalog_conn)
                    .await
                    .map_err(|e| anyhow!("SqliteMetadataProvider: {e}"))?;
                Ok(Arc::new(provider))
            }
            StorageProfile::Postgres {
                catalog_url,
                catalog_name,
                pool,
                catalog_id,
                ..
            } => {
                let pool = postgres_pool(catalog_url, pool).await?;
                let catalog_id = postgres_catalog_id(&pool, catalog_name, catalog_id).await?;
                let provider = MulticatalogProvider::with_pool_and_id(pool, catalog_id)
                    .await
                    .map_err(|e| anyhow!("MulticatalogProvider: {e}"))?;
                Ok(Arc::new(MeteredMetadataProvider::new(Arc::new(provider))))
            }
        }
    }

    pub async fn init_ducklake_metadata(
        &self,
    ) -> Result<(Arc<dyn MetadataProvider>, Arc<dyn MetadataWriter>)> {
        let writer = self.metadata_writer().await?;
        let provider = self.metadata_provider().await?;

        // Pre-create the `main` schema so SQL like `quackgis.main.<table>` can
        // resolve at plan time before any CREATE TABLE has run. Reopening an
        // initialized catalog must be read-only: manufacturing a bare snapshot
        // on every process start pollutes history and weakens retention evidence.
        let current_snapshot = provider
            .get_current_snapshot()
            .map_err(|e| anyhow!("get_current_snapshot: {e}"))?;
        let main_exists = current_snapshot > 0
            && provider
                .get_schema_by_name("main", current_snapshot)
                .map_err(|e| anyhow!("get_schema_by_name(main): {e}"))?
                .is_some();
        if !main_exists {
            let initial_snapshot = writer
                .create_snapshot()
                .map_err(|e| anyhow!("create_snapshot: {e}"))?;
            let _ = writer
                .get_or_create_schema("main", None, initial_snapshot)
                .map_err(|e| anyhow!("get_or_create_schema(main): {e}"))?;
        }

        Ok((provider, writer))
    }

    pub fn object_store(&self) -> Result<Arc<dyn ObjectStore>> {
        if self.data_path().to_ascii_lowercase().starts_with("s3://") {
            let (bucket, s3) = self.s3_bucket_and_options()?;
            return Ok(Arc::new(s3.build_for_bucket(&bucket)?));
        }
        Ok(Arc::new(LocalFileSystem::new()))
    }

    pub fn register_runtime_object_store(&self, runtime: &RuntimeEnv) -> Result<()> {
        if self.data_path().to_ascii_lowercase().starts_with("s3://") {
            let bucket = s3_bucket_name(self.data_path())?;
            let object_store = self.object_store()?;
            let url = Url::parse(&format!("s3://{bucket}"))?;
            runtime.register_object_store(&url, object_store);
        }
        Ok(())
    }

    fn s3_bucket_and_options(&self) -> Result<(String, &S3StorageOptions)> {
        let bucket = s3_bucket_name(self.data_path())?;
        let s3 = match &self.profile {
            StorageProfile::Postgres { s3: Some(s3), .. } => s3,
            _ => {
                return Err(anyhow!(
                    "s3:// data path requires S3 storage options for object-store client"
                ));
            }
        };
        Ok((bucket, s3))
    }

    fn quarantine_prefix_key(&self, quarantine_prefix: &str) -> Result<String> {
        let quarantine_prefix = quarantine_prefix.trim();
        if quarantine_prefix.is_empty() {
            return Err(anyhow!("orphan quarantine prefix cannot be empty"));
        }

        if self.data_path().to_ascii_lowercase().starts_with("s3://") {
            let data_url = Url::parse(self.data_path())?;
            let quarantine_url = Url::parse(quarantine_prefix).with_context(
                || "S3-backed orphan quarantine prefixes must be s3://bucket/prefix URLs",
            )?;
            if quarantine_url.scheme() != "s3" {
                return Err(anyhow!("S3-backed orphan quarantine prefix must use s3://"));
            }
            if quarantine_url.host_str() != data_url.host_str() {
                return Err(anyhow!(
                    "orphan quarantine prefix must use the same S3 bucket as the live data path"
                ));
            }
            let data_key = trim_object_key(data_url.path());
            let quarantine_key = trim_object_key(quarantine_url.path());
            if quarantine_key.is_empty() {
                return Err(anyhow!(
                    "orphan quarantine prefix must include an object key prefix"
                ));
            }
            reject_live_prefix(&data_key, &quarantine_key)?;
            return Ok(quarantine_key);
        }

        let data_path = Path::new(self.data_path())
            .canonicalize()
            .with_context(|| format!("canonicalizing live data path {}", self.data_path()))?;
        let quarantine_path = absolute_path_without_creating(quarantine_prefix)?;
        if quarantine_path == data_path || quarantine_path.starts_with(&data_path) {
            return Err(anyhow!(
                "orphan quarantine prefix must be outside the live data path"
            ));
        }
        let key = quarantine_path
            .to_string_lossy()
            .trim_start_matches('/')
            .trim_end_matches('/')
            .to_string();
        if key.is_empty() {
            return Err(anyhow!(
                "orphan quarantine prefix cannot resolve to the filesystem root"
            ));
        }
        Ok(key)
    }

    fn ensure_local_parent_for_key(&self, key: &ObjectPath) -> Result<()> {
        if self.data_path().to_ascii_lowercase().starts_with("s3://") {
            return Ok(());
        }
        let absolute_path = Path::new("/").join(key.as_ref());
        if let Some(parent) = absolute_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating quarantine directory {}", parent.display()))?;
        }
        Ok(())
    }
}

fn trim_object_key(value: &str) -> String {
    value.trim_matches('/').to_string()
}

fn reject_live_prefix(data_key: &str, quarantine_key: &str) -> Result<()> {
    if data_key.is_empty() {
        return Err(anyhow!(
            "refusing orphan quarantine for an S3 data path at the bucket root"
        ));
    }
    if quarantine_key == data_key
        || quarantine_key
            .strip_prefix(data_key)
            .is_some_and(|suffix| suffix.starts_with('/'))
    {
        return Err(anyhow!(
            "orphan quarantine prefix must be outside the live data path"
        ));
    }
    Ok(())
}

fn absolute_path_without_creating(raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .with_context(|| "resolving current directory for quarantine prefix")?
            .join(path)
    };
    let parent = candidate
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| anyhow!("invalid orphan quarantine prefix {raw:?}"))?;
    let file_name = candidate
        .file_name()
        .ok_or_else(|| anyhow!("invalid orphan quarantine prefix {raw:?}"))?;
    let abs_parent = parent
        .canonicalize()
        .with_context(|| format!("canonicalizing parent of quarantine prefix {raw}"))?;
    Ok(abs_parent.join(file_name))
}

fn object_key_from_absolute_style(path: &str) -> Result<ObjectPath> {
    let key = path.trim_start_matches('/');
    if key.is_empty() {
        return Err(anyhow!("object path cannot be empty"));
    }
    Ok(ObjectPath::from(key))
}

fn quarantine_destination_key(prefix_key: &str, source_key: &ObjectPath) -> ObjectPath {
    ObjectPath::from(format!(
        "{}/{}",
        prefix_key.trim_end_matches('/'),
        source_key.as_ref().trim_start_matches('/')
    ))
}

fn absolute_style_path(key: &ObjectPath) -> String {
    format!("/{key}")
}

async fn postgres_pool(catalog_url: &str, cell: &OnceCell<PgPool>) -> Result<PgPool> {
    cell.get_or_try_init(|| async {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(catalog_url)
            .await
            .with_context(|| "connecting to DuckLake PostgreSQL catalog")?;
        initialize_multicatalog_schema(&pool)
            .await
            .map_err(|e| anyhow!("initialize DuckLake multicatalog schema: {e}"))?;
        Ok::<PgPool, anyhow::Error>(pool)
    })
    .await
    .cloned()
}

async fn postgres_catalog_id(
    pool: &PgPool,
    catalog_name: &str,
    cell: &OnceCell<i64>,
) -> Result<i64> {
    cell.get_or_try_init(|| async {
        let manager = MulticatalogManager::new(pool.clone());
        manager
            .create_catalog(catalog_name)
            .await
            .map_err(|e| anyhow!("create DuckLake catalog {catalog_name:?}: {e}"))
    })
    .await
    .copied()
}

impl S3StorageOptions {
    pub fn new(
        endpoint: Option<String>,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        region: String,
        allow_http: bool,
    ) -> Result<Option<Self>> {
        if endpoint.is_none() && access_key_id.is_none() && secret_access_key.is_none() {
            return Ok(None);
        }
        if access_key_id.is_some() != secret_access_key.is_some() {
            return Err(anyhow!(
                "S3 access key id and secret access key must be specified together"
            ));
        }
        if let Some(endpoint) = endpoint.as_deref()
            && endpoint.starts_with("http://")
            && !allow_http
        {
            return Err(anyhow!(
                "S3 endpoint {endpoint:?} uses HTTP; set --s3-allow-http for local development endpoints"
            ));
        }

        Ok(Some(Self {
            endpoint,
            access_key_id,
            secret_access_key,
            region,
            allow_http,
        }))
    }

    fn from_env() -> Result<Option<Self>> {
        let endpoint = std::env::var("QUACKGIS_S3_ENDPOINT").ok();
        let access_key_id = std::env::var("QUACKGIS_S3_ACCESS_KEY_ID").ok();
        let secret_access_key = std::env::var("QUACKGIS_S3_SECRET_ACCESS_KEY").ok();
        let region =
            std::env::var("QUACKGIS_S3_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        let allow_http = std::env::var("QUACKGIS_S3_ALLOW_HTTP")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);
        Self::new(
            endpoint,
            access_key_id,
            secret_access_key,
            region,
            allow_http,
        )
    }

    fn build_for_bucket(&self, bucket: &str) -> Result<object_store::aws::AmazonS3> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(bucket)
            .with_region(&self.region)
            .with_allow_http(self.allow_http)
            .with_virtual_hosted_style_request(false);
        if let Some(endpoint) = &self.endpoint {
            builder = builder.with_endpoint(endpoint);
        }
        if let Some(access_key_id) = &self.access_key_id {
            builder = builder.with_access_key_id(access_key_id);
        }
        if let Some(secret_access_key) = &self.secret_access_key {
            builder = builder.with_secret_access_key(secret_access_key);
        }
        builder
            .build()
            .with_context(|| format!("building S3 object store client for bucket {bucket:?}"))
    }
}

fn s3_bucket_name(data_path: &str) -> Result<String> {
    let url =
        Url::parse(data_path).with_context(|| format!("parsing S3 data path {data_path:?}"))?;
    url.host_str()
        .map(ToString::to_string)
        .ok_or_else(|| anyhow!("S3 data path {data_path:?} is missing a bucket name"))
}

/// Build the QuackGIS session context: SedonaDB function catalog + DuckLake
/// storage + pg_catalog emulation + information_schema.
///
/// Construction order matters:
///   1. Build the DuckLake catalog with write support for the selected storage
///      profile.
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
    let auth = AuthConfig::trust();
    build_session_context_with_storage_and_auth(paths, &auth).await
}

/// Same as [`build_session_context_with_storage`] but with explicit pgwire auth
/// metadata so `pg_catalog.pg_roles` matches the login profile clients see.
pub async fn build_session_context_with_storage_and_auth(
    paths: StoragePaths,
    auth: &AuthConfig,
) -> Result<Arc<SessionContext>> {
    // 0. Configure the pure-Rust CRS engine before any SedonaDB functions
    //    are registered. This replaces libproj with proj-wkt/proj-core.
    //    Idempotent: safe to call multiple times (subsequent calls no-op).
    let _ = sedona_proj::rust_engine::configure_global_rust_engine();

    // 1. DuckLake: writer creates the catalog schema if missing, then a
    //    snapshot is required before any read or write can happen.
    let (provider, writer) = paths.init_ducklake_metadata().await?;
    let ducklake_function_provider = Arc::clone(&provider);
    let ducklake = DuckLakeCatalog::with_writer(provider, writer)
        .map_err(|e| anyhow!("DuckLakeCatalog::with_writer: {e}"))?;

    // 2. SessionContext. Keep "datafusion" as the default catalog (it's the
    //    in-memory one, where setup_pg_catalog can attach the pg_catalog
    //    schema — DuckLake rejects schema registration). DuckLake is
    //    registered alongside as a separate catalog; persisted tables are
    //    accessed as `quackgis.main.<table>`. information_schema on.
    let mut config = SessionConfig::new().with_information_schema(true);
    if let Some(target_partitions) = configured_target_partitions()? {
        config = config.with_target_partitions(target_partitions);
    }
    let runtime = Arc::new(RuntimeEnv::default());
    paths.register_runtime_object_store(&runtime)?;
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
    register_ducklake_metadata_functions(&ctx, ducklake_function_provider);

    // Expose DuckLake `quackgis.main` tables as PostgreSQL-style `public.*`
    // before pg_catalog is installed so QGIS pg_class/pg_namespace probes see
    // the alias schema.
    crate::public_schema::register_public_schema_alias(&ctx)
        .map_err(|e| anyhow!("register_public_schema_alias failed: {e}"))?;

    // 3. pg_catalog attached to the in-memory "datafusion" catalog. In password
    //    mode, mirror the configured login roles so client metadata does not
    //    advertise a superuser-only world.
    let auth_manager = Arc::new(AuthManager::new());
    configure_pg_catalog_auth(auth_manager.as_ref(), auth).await?;
    setup_pg_catalog(&ctx, "datafusion", auth_manager)
        .map_err(|e| anyhow!("setup_pg_catalog failed: {e}"))?;

    crate::postgis_compat::register_postgis_compat(&ctx, auth)
        .map_err(|e| anyhow!("register_postgis_compat failed: {e}"))?;

    crate::geometry_columns::register_geometry_columns(&ctx)
        .map_err(|e| anyhow!("register_geometry_columns failed: {e}"))?;

    crate::spatial_udfs::register_spatial_udfs(&ctx)
        .map_err(|e| anyhow!("register_spatial_udfs failed: {e}"))?;

    Ok(ctx)
}

async fn configure_pg_catalog_auth(auth_manager: &AuthManager, auth: &AuthConfig) -> Result<()> {
    if auth.mode() == AuthMode::Trust {
        return Ok(());
    }

    for (username, user) in auth.users() {
        let role = pg_catalog_role(username, user.role);
        let account = User {
            username: username.to_string(),
            password_hash: "SCRAM-SHA-256:<redacted>".to_string(),
            roles: vec![username.to_string()],
            is_superuser: false,
            can_login: true,
            connection_limit: None,
        };
        auth_manager
            .add_role(role)
            .await
            .map_err(|e| anyhow!("register pg_catalog role {username:?}: {e}"))?;
        auth_manager
            .add_user(account)
            .await
            .map_err(|e| anyhow!("register pg_catalog user {username:?}: {e}"))?;
    }

    Ok(())
}

fn pg_catalog_role(name: &str, access: AccessRole) -> Role {
    Role {
        name: name.to_string(),
        is_superuser: false,
        can_login: true,
        can_create_db: false,
        can_create_role: false,
        can_create_user: false,
        can_replication: false,
        grants: pg_catalog_grants(access),
        inherited_roles: vec![],
    }
}

fn pg_catalog_grants(access: AccessRole) -> Vec<Grant> {
    let permissions = match access {
        AccessRole::ReadWrite => vec![Permission::All],
        AccessRole::ReadOnly => vec![Permission::Select, Permission::Usage, Permission::Connect],
    };

    permissions
        .into_iter()
        .map(|permission| Grant {
            permission,
            resource: ResourceType::All,
            granted_by: "quackgis".to_string(),
            with_grant_option: false,
        })
        .collect()
}

fn register_ducklake_metadata_functions(ctx: &SessionContext, provider: Arc<dyn MetadataProvider>) {
    // Safe catalog-inspection UDTFs for operations and trend probes. CDC row
    // functions remain unregistered here until their projected row schema is
    // safe through the pgwire/arrow-pg encoder path.
    ctx.register_udtf(
        "ducklake_snapshots",
        Arc::new(DucklakeSnapshotsFunction::new(Arc::clone(&provider))),
    );
    ctx.register_udtf(
        "ducklake_table_info",
        Arc::new(DucklakeTableInfoFunction::new(Arc::clone(&provider))),
    );
    ctx.register_udtf(
        "ducklake_list_files",
        Arc::new(DucklakeListFilesFunction::new(provider)),
    );
}

fn configured_target_partitions() -> Result<Option<usize>> {
    match std::env::var(TARGET_PARTITIONS_ENV) {
        Ok(value) => parse_target_partitions_value(&value),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(anyhow!("could not read {TARGET_PARTITIONS_ENV}: {err}")),
    }
}

fn parse_target_partitions_value(value: &str) -> Result<Option<usize>> {
    let value = value.trim();
    if value.is_empty() || value == "0" {
        return Ok(None);
    }

    let target_partitions = value.parse::<usize>().map_err(|err| {
        anyhow!(
            "{TARGET_PARTITIONS_ENV} must be a positive integer or 0 to preserve DataFusion's default: {err}"
        )
    })?;
    if target_partitions == 0 {
        Ok(None)
    } else {
        Ok(Some(target_partitions))
    }
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

    #[test]
    fn target_partitions_parser_preserves_default_for_unset_values() {
        assert_eq!(parse_target_partitions_value("").unwrap(), None);
        assert_eq!(parse_target_partitions_value(" 0 ").unwrap(), None);
    }

    #[test]
    fn target_partitions_parser_accepts_positive_values() {
        assert_eq!(parse_target_partitions_value("1").unwrap(), Some(1));
        assert_eq!(parse_target_partitions_value(" 8 ").unwrap(), Some(8));
    }

    #[test]
    fn target_partitions_parser_rejects_invalid_values() {
        assert!(parse_target_partitions_value("many").is_err());
        assert!(parse_target_partitions_value("-1").is_err());
    }

    /// Smoke: context builds, exposes SedonaDB ST_* functions, and answers
    /// spatial SQL in-process so regressions in either upstream are caught
    /// without needing psql on the host.
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
