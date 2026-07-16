// SPDX-License-Identifier: Apache-2.0
//! DuckDB/ADBC storage kernel backed by official DuckLake.
//!
//! ADBC is the Arrow transport. DuckLake compatibility comes from executing
//! writes through DuckDB's official `ducklake` extension.

use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use adbc_core::error::{Error as AdbcError, Status as AdbcStatus};
use adbc_core::options::{
    AdbcVersion, IngestMode, OptionConnection, OptionDatabase, OptionStatement,
};
use adbc_core::sync::{Connection, Database, Driver, Optionable, Statement};
use adbc_driver_manager::{ManagedConnection, ManagedDatabase, ManagedDriver, ManagedStatement};
use anyhow::{Context, Result, anyhow, bail};
use arrow_array::{
    Array, Int64Array, RecordBatch, RecordBatchIterator, RecordBatchReader, StringArray,
};
use arrow_pg::datatypes::{SpatialFamily, classify_spatial_field};
use arrow_schema::{ArrowError, SchemaRef};
use sha2::{Digest, Sha256};
use sqlparser::ast::{
    AssignmentTarget, BinaryOperator, Expr, Function, FunctionArg, FunctionArgExpr,
    FunctionArguments, Ident, ObjectName, ObjectNamePart, Query, SetExpr,
    Statement as SqlStatement, TableFactor, TableObject, UnaryOperator, Value,
};
use sqlparser::dialect::{DuckDbDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;

use crate::engine_api::{
    EngineBatchStream, EngineCancellation, EngineError, EngineErrorKind, EngineMaintenanceReport,
    EngineMaintenanceRequest, EngineQueryResult, EngineQueryStream, EngineResourceSample,
    EngineResult, EngineSnapshot, EngineStatementDescription, EngineStorageKernel, EngineTableRef,
    EngineTransactionState, IngestDisposition, TransactionOutcome,
};
use crate::lifecycle::RuntimeLifecycle;
use crate::storage_authority::claim_local_root;

const DUCKDB_ADBC_ENTRYPOINT: &[u8] = b"duckdb_adbc_init";
const SUPPORTED_DUCKDB_VERSION: &str = "v1.5.4";
const SUPPORTED_LIBDUCKDB_SHA256: &str =
    "d7f30ef2ef4b813edb94ce82906329cc689672624a4161617ea33431040ce174";
const FALLBACK_DUCKDB_THREADS: usize = 4;
const FALLBACK_DUCKDB_MEMORY_LIMIT_BYTES: u64 = 1_073_741_824;
const MIN_DUCKDB_MAX_TEMP_DIRECTORY_BYTES: u64 = 10_737_418_240;
const CGROUP_UNLIMITED_THRESHOLD: u64 = 1_u64 << 60;
static COPY_STAGE_ID: AtomicU64 = AtomicU64::new(1);
static READINESS_PROBE_ID: AtomicU64 = AtomicU64::new(1);
const MAINTAINED_BBOX_COLUMNS: [&str; 4] = ["_qg_minx", "_qg_miny", "_qg_maxx", "_qg_maxy"];
const MAX_BBOX_WKT_BYTES: usize = 65_536;
const MAX_BBOX_NUMERIC_LITERAL_BYTES: usize = 64;

struct MaintainedBboxLayout {
    geometry: String,
}

struct BboxQueryTarget {
    table: EngineTableRef,
    qualifier: Ident,
    geometry: Ident,
    probe: Expr,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogColumnIdentity {
    pub name: String,
    pub relation_oid: u32,
    pub attribute_number: i16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogTableIdentity {
    pub schema_epoch: u64,
    pub columns: Vec<CatalogColumnIdentity>,
}

/// Whether DuckDB may download the DuckLake extension during initialization.
///
/// `LoadOnly` is the production-safe default: image construction must install
/// and pin the extension in advance. `InstallAndLoad` is intended only for local
/// evaluation where network access and extension provenance are explicit.
/// `DevelopmentDuckLake` permits one exact unsigned native artifact after local
/// path and digest validation; it must never be selected from client input.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ExtensionPolicy {
    #[default]
    LoadOnly,
    InstallAndLoad,
    DevelopmentDuckLake {
        path: PathBuf,
        sha256: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuckDbResourceConfig {
    pub threads: usize,
    pub memory_limit_bytes: u64,
    pub temp_directory: PathBuf,
    pub max_temp_directory_bytes: u64,
}

impl DuckDbResourceConfig {
    /// Size DuckDB from effective host/container capacity. DuckDB receives 60%
    /// of memory, leaving the remainder for Arrow, pgwire, and the OS. The spill
    /// ceiling is at least 10 GiB and otherwise four times that memory budget.
    pub fn for_data_path(data_path: impl AsRef<Path>) -> Self {
        Self::for_capacity(
            data_path,
            effective_memory_capacity_bytes(),
            effective_parallelism(),
        )
    }

    fn for_capacity(
        data_path: impl AsRef<Path>,
        memory_capacity_bytes: Option<u64>,
        parallelism: usize,
    ) -> Self {
        let memory_limit_bytes = memory_capacity_bytes
            .map(|capacity| capacity.saturating_mul(3) / 5)
            .filter(|limit| *limit > 0)
            .unwrap_or(FALLBACK_DUCKDB_MEMORY_LIMIT_BYTES);
        Self {
            threads: parallelism.max(1),
            memory_limit_bytes,
            temp_directory: data_path.as_ref().join(".tmp"),
            max_temp_directory_bytes: memory_limit_bytes
                .saturating_mul(4)
                .max(MIN_DUCKDB_MAX_TEMP_DIRECTORY_BYTES),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.threads == 0 || self.memory_limit_bytes == 0 || self.max_temp_directory_bytes == 0 {
            bail!("DuckDB resource limits must be positive");
        }
        if self.temp_directory.as_os_str().is_empty()
            || self.temp_directory.to_string_lossy().contains("://")
        {
            bail!("DuckDB temporary directory must be a non-empty local path");
        }
        Ok(())
    }

    fn sql(&self) -> String {
        format!(
            "SET threads={}; SET memory_limit='{}B'; SET temp_directory={}; SET max_temp_directory_size='{}B';",
            self.threads,
            self.memory_limit_bytes,
            quote_literal(&self.temp_directory.display().to_string()),
            self.max_temp_directory_bytes,
        )
    }
}

fn effective_memory_capacity_bytes() -> Option<u64> {
    let host = read_mem_total_bytes();
    let cgroup = [
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/memory/memory.limit_in_bytes",
    ]
    .into_iter()
    .filter_map(read_finite_u64)
    .min();
    match (host, cgroup) {
        (Some(host), Some(cgroup)) => Some(host.min(cgroup)),
        (host, cgroup) => host.or(cgroup),
    }
}

fn read_mem_total_bytes() -> Option<u64> {
    std::fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find_map(|line| {
            let kib = line.strip_prefix("MemTotal:")?.trim();
            let kib = kib.strip_suffix("kB")?.trim().parse::<u64>().ok()?;
            kib.checked_mul(1024)
        })
}

fn read_finite_u64(path: &str) -> Option<u64> {
    let value = std::fs::read_to_string(path).ok()?.trim().parse().ok()?;
    (value > 0 && value < CGROUP_UNLIMITED_THRESHOLD).then_some(value)
}

fn effective_parallelism() -> usize {
    let host = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(FALLBACK_DUCKDB_THREADS);
    cgroup_cpu_quota()
        .map_or(host, |quota| host.min(quota))
        .max(1)
}

fn cgroup_cpu_quota() -> Option<usize> {
    if let Ok(raw) = std::fs::read_to_string("/sys/fs/cgroup/cpu.max") {
        let mut values = raw.split_whitespace();
        let quota = values.next()?;
        let period = values.next()?.parse::<u64>().ok()?;
        if quota != "max" {
            return cpu_quota_threads(quota.parse().ok()?, period);
        }
    }
    let quota = std::fs::read_to_string("/sys/fs/cgroup/cpu/cpu.cfs_quota_us")
        .ok()?
        .trim()
        .parse::<i64>()
        .ok()?;
    let period = std::fs::read_to_string("/sys/fs/cgroup/cpu/cpu.cfs_period_us")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    (quota > 0).then(|| cpu_quota_threads(quota as u64, period))?
}

fn cpu_quota_threads(quota: u64, period: u64) -> Option<usize> {
    let threads = quota.checked_add(period.checked_sub(1)?)? / period;
    usize::try_from(threads.max(1)).ok()
}

/// Configuration for one DuckDB process and one attached DuckLake catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuckDbAdbcConfig {
    /// Absolute path to an operator-selected `libduckdb` shared library.
    ///
    /// Loading this file executes native code in the QuackGIS process. Never
    /// accept this value from an untrusted client or SQL statement.
    pub driver_path: PathBuf,
    /// DuckDB control database URI. `:memory:` is sufficient when durable state
    /// lives in the attached DuckLake catalog and object path.
    pub database_uri: String,
    /// DuckLake catalog URI. The current server constructs this from a validated
    /// local catalog path; shared catalog URIs are reserved for a future profile.
    pub ducklake_uri: String,
    /// Name used for the attached DuckLake catalog.
    pub catalog_name: String,
    /// DuckLake data path. The current server accepts local paths only.
    pub data_path: String,
    /// Extension installation policy. Defaults to fail-closed `LOAD` only.
    pub extension_policy: ExtensionPolicy,
}

impl DuckDbAdbcConfig {
    fn validate(&self) -> Result<()> {
        if !self.driver_path.is_absolute() {
            bail!("DuckDB ADBC driver_path must be absolute");
        }
        let metadata = self.driver_path.metadata().with_context(|| {
            format!(
                "reading DuckDB ADBC driver metadata at {}",
                self.driver_path.display()
            )
        })?;
        if !metadata.is_file() {
            bail!(
                "DuckDB ADBC driver_path is not a regular file: {}",
                self.driver_path.display()
            );
        }
        for (label, value) in [
            ("database_uri", self.database_uri.as_str()),
            ("ducklake_uri", self.ducklake_uri.as_str()),
            ("catalog_name", self.catalog_name.as_str()),
            ("data_path", self.data_path.as_str()),
        ] {
            if value.trim().is_empty() {
                bail!("DuckDB ADBC {label} must not be empty");
            }
            if value.contains('\0') {
                bail!("DuckDB ADBC {label} must not contain NUL bytes");
            }
        }
        if let ExtensionPolicy::DevelopmentDuckLake { path, sha256 } = &self.extension_policy {
            validate_development_extension(path, sha256)?;
        }
        Ok(())
    }

    fn bootstrap_sql(&self) -> String {
        let extension_sql = match &self.extension_policy {
            ExtensionPolicy::LoadOnly => "LOAD ducklake;\nLOAD spatial;",
            ExtensionPolicy::InstallAndLoad => {
                "INSTALL ducklake;\nINSTALL spatial;\nLOAD ducklake;\nLOAD spatial;"
            }
            ExtensionPolicy::DevelopmentDuckLake { path, .. } => {
                return self.bootstrap_sql_with_extensions(&format!(
                    "LOAD {};\nLOAD spatial;",
                    quote_literal(
                        path.to_str()
                            .expect("development extension path validated as UTF-8"),
                    )
                ));
            }
        };
        self.bootstrap_sql_with_extensions(extension_sql)
    }

    fn bootstrap_sql_with_extensions(&self, extension_sql: &str) -> String {
        format!(
            "{extension_sql}\n\
             {}\n\
             {}\n\
             SET ducklake_default_data_inlining_row_limit = 0;\n\
             ATTACH {} AS {} (DATA_PATH {}, DATA_INLINING_ROW_LIMIT 0);",
            crate::spatial_compat::DUCKDB_COMPATIBILITY_MACROS,
            crate::postgres_compat::duckdb_catalog_bootstrap_sql(),
            quote_literal(&self.ducklake_uri),
            quote_identifier(&self.catalog_name),
            quote_literal(&self.data_path),
        )
    }

    fn allows_unsigned_extensions(&self) -> bool {
        matches!(
            self.extension_policy,
            ExtensionPolicy::DevelopmentDuckLake { .. }
        )
    }
}

/// Synchronous DuckDB ADBC kernel.
///
/// The driver manager serializes access internally and this type additionally
/// keeps transaction ownership explicit with one mutex-protected connection.
/// Async callers must invoke it through `tokio::task::spawn_blocking`; the
/// production adapter will own that policy after the experimental gates pass.
pub struct DuckDbAdbcStorage {
    // ADBC requires the database handle to outlive all of its connections.
    _database: ManagedDatabase,
    // A transaction takes the connection out of this slot. Reentrant calls then
    // fail immediately instead of deadlocking, and indeterminate native failures
    // quarantine the connection by leaving the slot empty.
    connection: Mutex<Option<ManagedConnection>>,
    catalog_name: String,
    data_path: PathBuf,
    catalog_identity_enabled: bool,
    catalog_commit_lock: Arc<Mutex<()>>,
    transaction_state: Mutex<EngineTransactionState>,
    lifecycle: Arc<RuntimeLifecycle>,
}

pub struct DuckDbIngestOperation {
    owner: Arc<DuckDbAdbcStorage>,
    connection: Option<ManagedConnection>,
    cancellation: Arc<DuckDbCancelHandle>,
}

pub struct DuckDbUpdateOperation {
    owner: Arc<DuckDbAdbcStorage>,
    connection: Option<ManagedConnection>,
    cancellation: Arc<DuckDbCancelHandle>,
}

struct DuckDbBatchStream {
    owner: Arc<DuckDbAdbcStorage>,
    reader: Option<Box<dyn RecordBatchReader + Send>>,
    statement: Option<ManagedStatement>,
    connection: Option<ManagedConnection>,
    state: BatchStreamState,
    cancellation: Arc<DuckDbCancelHandle>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BatchStreamState {
    Active,
    Exhausted,
    Failed,
}

struct DuckDbCancelHandle {
    connection: Mutex<ManagedConnection>,
    requested: AtomicBool,
    closed: AtomicBool,
}

impl EngineCancellation for DuckDbCancelHandle {
    fn cancel(&self) -> EngineResult<()> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        self.requested.store(true, Ordering::Release);
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        self.connection
            .lock()
            .map_err(|_| {
                EngineError::new(
                    EngineErrorKind::Quarantined,
                    "DuckDB cancellation handle is poisoned",
                )
            })?
            .cancel()
            .map_err(engine_error)
    }
}

impl DuckDbCancelHandle {
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

impl EngineBatchStream for DuckDbBatchStream {
    fn next_batch(&mut self) -> EngineResult<Option<RecordBatch>> {
        match self.reader.as_mut().and_then(Iterator::next) {
            Some(Ok(batch)) => Ok(Some(batch)),
            Some(Err(error)) => {
                self.state = BatchStreamState::Failed;
                Err(EngineError::new(
                    if self.cancellation.requested.load(Ordering::Acquire) {
                        EngineErrorKind::Cancelled
                    } else {
                        EngineErrorKind::Internal
                    },
                    error.to_string(),
                ))
            }
            None => {
                self.state = BatchStreamState::Exhausted;
                Ok(None)
            }
        }
    }
}

impl Drop for DuckDbBatchStream {
    fn drop(&mut self) {
        self.reader.take();
        self.statement.take();
        let Some(connection) = self.connection.take() else {
            return;
        };
        let clean = self.state == BatchStreamState::Exhausted
            && !self.cancellation.requested.load(Ordering::Acquire);
        if !clean || self.owner.return_connection(connection).is_err() {
            let _ = self
                .owner
                .set_transaction_state(EngineTransactionState::Quarantined);
        }
    }
}

impl Drop for DuckDbAdbcStorage {
    fn drop(&mut self) {
        if self.transaction_state() == EngineTransactionState::Active {
            let _ = self.rollback_transaction();
        }
    }
}

impl DuckDbAdbcStorage {
    /// Load the trusted DuckDB driver, open a connection, and attach DuckLake.
    pub fn open(config: DuckDbAdbcConfig) -> Result<Self> {
        let resources = DuckDbResourceConfig::for_data_path(&config.data_path);
        Self::open_with_resources(config, resources)
    }

    pub fn open_with_resources(
        config: DuckDbAdbcConfig,
        resources: DuckDbResourceConfig,
    ) -> Result<Self> {
        config.validate()?;
        resources.validate()?;
        verify_driver_digest(&config.driver_path)?;

        if config.data_path.contains("://") {
            bail!(
                "DuckDB ADBC remote data paths are disabled until shared-profile authority markers and credentials are wired"
            );
        }
        let mut driver = ManagedDriver::load_dynamic_from_filename(
            &config.driver_path,
            Some(DUCKDB_ADBC_ENTRYPOINT),
            AdbcVersion::V110,
        )
        .with_context(|| {
            format!(
                "loading DuckDB ADBC driver from {}",
                config.driver_path.display()
            )
        })?;
        let catalog_identity_enabled = config.allows_unsigned_extensions();
        let database = if catalog_identity_enabled {
            driver.new_database_with_opts([
                (OptionDatabase::Uri, config.database_uri.clone().into()),
                (
                    OptionDatabase::Other("allow_unsigned_extensions".to_owned()),
                    "true".into(),
                ),
            ])
        } else {
            driver
                .new_database_with_opts([(OptionDatabase::Uri, config.database_uri.clone().into())])
        }
        .context("initializing DuckDB ADBC database")?;
        let mut connection = database
            .new_connection()
            .context("opening DuckDB ADBC connection")?;
        verify_runtime_version(&mut connection)?;
        std::fs::create_dir_all(&resources.temp_directory).with_context(|| {
            format!(
                "creating DuckDB temporary directory {}",
                resources.temp_directory.display()
            )
        })?;
        execute_update_on(&mut connection, &resources.sql())
            .context("configuring DuckDB resource limits")?;
        claim_local_root(std::path::Path::new(&config.data_path))
            .context("claiming local DuckDB official-DuckLake data root")?;
        let bootstrap_sql = config.bootstrap_sql();
        execute_update_on(&mut connection, &bootstrap_sql)
            .context("loading and attaching the official DuckLake extension")?;
        if catalog_identity_enabled {
            initialize_catalog_identity_registry_on(&mut connection, &config.catalog_name)
                .context("initializing transactional PostgreSQL catalog identity")?;
            execute_update_on(
                &mut connection,
                &crate::postgres_compat::duckdb_identity_catalog_bootstrap_sql(
                    &config.catalog_name,
                ),
            )
            .context("projecting registry-backed PostgreSQL catalogs")?;
        }

        Ok(Self {
            _database: database,
            connection: Mutex::new(Some(connection)),
            catalog_name: config.catalog_name,
            data_path: PathBuf::from(config.data_path),
            catalog_identity_enabled,
            catalog_commit_lock: Arc::new(Mutex::new(())),
            transaction_state: Mutex::new(EngineTransactionState::Idle),
            lifecycle: Arc::new(RuntimeLifecycle::default()),
        })
    }

    /// Open an independent connection/session against the same official catalog.
    ///
    /// Each session has isolated transaction ownership and fail-closed quarantine
    /// state while retaining the process-owned ADBC database/driver lifetime.
    pub fn open_session(&self) -> Result<Self> {
        let connection = self
            ._database
            .new_connection()
            .context("opening DuckDB ADBC session connection")?;
        Ok(Self {
            _database: self._database.clone(),
            connection: Mutex::new(Some(connection)),
            catalog_name: self.catalog_name.clone(),
            data_path: self.data_path.clone(),
            catalog_identity_enabled: self.catalog_identity_enabled,
            catalog_commit_lock: Arc::clone(&self.catalog_commit_lock),
            transaction_state: Mutex::new(EngineTransactionState::Idle),
            lifecycle: Arc::clone(&self.lifecycle),
        })
    }

    pub fn lifecycle(&self) -> Arc<RuntimeLifecycle> {
        Arc::clone(&self.lifecycle)
    }

    pub fn catalog_identity_enabled(&self) -> bool {
        self.catalog_identity_enabled
    }

    pub fn install_role_catalog(
        &self,
        catalog: &crate::role::RoleCatalog,
        auth: &crate::auth::AuthConfig,
    ) -> Result<()> {
        self.execute_update(&crate::postgres_compat::duckdb_role_catalog_sql(
            catalog,
            auth,
            self.catalog_identity_enabled,
        ))
        .context("installing immutable PostgreSQL role catalogs")?;
        Ok(())
    }

    pub fn catalog_schema_epoch(&self) -> EngineResult<Option<u64>> {
        if !self.catalog_identity_enabled {
            return Ok(None);
        }
        self.with_connection_engine(|connection| {
            let _catalog_commit = self.catalog_commit_guard().map_err(anyhow_engine_error)?;
            catalog_schema_epoch_on(connection, &self.catalog_name).map(Some)
        })
    }

    pub fn catalog_table_identity(
        &self,
        table: &EngineTableRef,
    ) -> EngineResult<Option<CatalogTableIdentity>> {
        if !self.catalog_identity_enabled {
            return Ok(None);
        }
        if table.catalog != self.catalog_name {
            return Err(EngineError::new(
                EngineErrorKind::Unsupported,
                "PostgreSQL catalog identity is available only for the configured DuckLake catalog",
            ));
        }
        let schema = if table.schema.eq_ignore_ascii_case("public") {
            "main"
        } else {
            table.schema.as_str()
        };
        let sql = format!(
            "SELECT c.column_name, CAST(c.relation_oid AS BIGINT), \
                    CAST(c.attnum AS BIGINT), CAST(s.schema_epoch AS BIGINT) \
             FROM quackgis_pg_catalog._current_columns c, \
                  {}.{}.catalog_state s \
             WHERE s.singleton AND lower(c.schema_name) = lower({}) \
               AND lower(c.table_name) = lower({}) \
             ORDER BY c.ordinal_position",
            quote_identifier(&self.catalog_name),
            quote_identifier(crate::postgres_compat::INTERNAL_SCHEMA),
            quote_literal(schema),
            quote_literal(&table.table),
        );
        self.with_connection_engine(|connection| {
            let _catalog_commit = self.catalog_commit_guard().map_err(anyhow_engine_error)?;
            let result = query_result_on(connection, &sql, None)?;
            let mut columns = Vec::new();
            let mut schema_epoch = None;
            for batch in result.batches {
                let names = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| {
                        EngineError::new(
                            EngineErrorKind::Internal,
                            "catalog column name is not Utf8",
                        )
                    })?;
                let relation_oids = int64_column(&batch, 1, "catalog relation OID")?;
                let attribute_numbers = int64_column(&batch, 2, "catalog attribute number")?;
                let epochs = int64_column(&batch, 3, "catalog schema epoch")?;
                for row in 0..batch.num_rows() {
                    let relation_oid = u32::try_from(relation_oids.value(row)).map_err(|_| {
                        EngineError::new(
                            EngineErrorKind::Internal,
                            "catalog relation OID is outside the PostgreSQL OID range",
                        )
                    })?;
                    let attribute_number =
                        i16::try_from(attribute_numbers.value(row)).map_err(|_| {
                            EngineError::new(
                                EngineErrorKind::Internal,
                                "catalog attribute number is outside the PostgreSQL range",
                            )
                        })?;
                    if attribute_number <= 0 {
                        return Err(EngineError::new(
                            EngineErrorKind::Internal,
                            "catalog attribute number is not positive",
                        ));
                    }
                    let epoch = u64::try_from(epochs.value(row)).map_err(|_| {
                        EngineError::new(
                            EngineErrorKind::Internal,
                            "catalog schema epoch is outside the supported range",
                        )
                    })?;
                    if schema_epoch
                        .replace(epoch)
                        .is_some_and(|current| current != epoch)
                    {
                        return Err(EngineError::new(
                            EngineErrorKind::Internal,
                            "catalog table identity spans multiple schema epochs",
                        ));
                    }
                    columns.push(CatalogColumnIdentity {
                        name: names.value(row).to_owned(),
                        relation_oid,
                        attribute_number,
                    });
                }
            }
            Ok(schema_epoch.map(|schema_epoch| CatalogTableIdentity {
                schema_epoch,
                columns,
            }))
        })
    }

    pub fn transaction_state(&self) -> EngineTransactionState {
        self.transaction_state
            .lock()
            .map(|state| *state)
            .unwrap_or(EngineTransactionState::Quarantined)
    }

    pub fn begin_transaction(&self) -> Result<()> {
        self.require_transaction_state(EngineTransactionState::Idle)?;
        if !self.lifecycle.try_start_transaction() {
            bail!("QuackGIS is draining and cannot start a new transaction");
        }
        if let Err(error) = self.with_connection(|connection| {
            connection
                .set_option(OptionConnection::AutoCommit, "false".into())
                .context("disabling DuckDB ADBC autocommit")
        }) {
            self.lifecycle.transaction_finished();
            return Err(error);
        }
        if let Err(error) = self.set_transaction_state(EngineTransactionState::Active) {
            self.lifecycle.transaction_finished();
            return Err(error);
        }
        Ok(())
    }

    pub fn commit_transaction(&self) -> Result<()> {
        self.require_transaction_state(EngineTransactionState::Active)?;
        let mut connection = self.take_connection()?;
        let _catalog_commit = match self.catalog_commit_guard() {
            Ok(guard) => guard,
            Err(error) => {
                let _ = connection.rollback();
                self.set_transaction_state(EngineTransactionState::Quarantined)?;
                return Err(error).context(
                    "catalog identity commit serialization failed; the connection was quarantined",
                );
            }
        };
        if let Err(error) = connection.commit() {
            self.set_transaction_state(EngineTransactionState::Quarantined)?;
            return Err(error).context(
                "DuckDB ADBC commit failed; transaction outcome is indeterminate and the connection was quarantined",
            );
        }
        if self.catalog_identity_enabled
            && let Err(error) =
                reconcile_catalog_identity_registry_on(&mut connection, &self.catalog_name)
        {
            let _ = connection.rollback();
            self.set_transaction_state(EngineTransactionState::Quarantined)?;
            return Err(error).context(
                "DuckLake commit succeeded but PostgreSQL catalog identity reconciliation failed; the connection was quarantined",
            );
        }
        if let Err(error) = restore_autocommit(&mut connection) {
            self.set_transaction_state(EngineTransactionState::Quarantined)?;
            return Err(error).context(
                "DuckDB ADBC commit succeeded but autocommit restoration failed; the connection was quarantined",
            );
        }
        self.return_connection(connection)?;
        self.set_transaction_state(EngineTransactionState::Idle)
    }

    pub fn rollback_transaction(&self) -> Result<()> {
        self.require_transaction_state(EngineTransactionState::Active)?;
        let mut connection = self.take_connection()?;
        if let Err(error) = connection.rollback() {
            self.set_transaction_state(EngineTransactionState::Quarantined)?;
            return Err(error)
                .context("DuckDB ADBC rollback failed and the connection was quarantined");
        }
        if let Err(error) = restore_autocommit(&mut connection) {
            self.set_transaction_state(EngineTransactionState::Quarantined)?;
            return Err(error).context(
                "DuckDB ADBC rollback succeeded but autocommit restoration failed; the connection was quarantined",
            );
        }
        self.return_connection(connection)?;
        self.set_transaction_state(EngineTransactionState::Idle)
    }

    /// Execute DDL or DML in autocommit mode.
    pub fn execute_update(&self, sql: &str) -> Result<Option<i64>> {
        if self.catalog_identity_enabled {
            return self.transaction(|transaction| transaction.execute_update(sql));
        }
        self.with_connection(|connection| execute_update_on(connection, sql))
    }

    /// Execute a query and materialize its Arrow batches.
    pub fn query(&self, sql: &str) -> Result<Vec<RecordBatch>> {
        self.with_connection(|connection| query_on(connection, sql))
    }

    /// Verify that the configured official DuckLake catalog remains queryable.
    /// The probe is read-only and callers should use a dedicated session.
    pub fn readiness_probe(&self) -> EngineResult<()> {
        let sql = format!(
            "SELECT CAST(count(*) AS BIGINT) FROM ducklake_snapshots({})",
            quote_literal(&self.catalog_name)
        );
        let result =
            self.with_connection_engine(|connection| query_result_on(connection, &sql, None))?;
        let count = result
            .batches
            .first()
            .and_then(|batch| batch.column(0).as_any().downcast_ref::<Int64Array>())
            .filter(|values| !values.is_empty())
            .map(|values| values.value(0))
            .ok_or_else(|| {
                EngineError::new(
                    EngineErrorKind::Internal,
                    "DuckLake readiness probe returned no snapshot count",
                )
            })?;
        if count < 0 {
            return Err(EngineError::new(
                EngineErrorKind::Internal,
                "DuckLake readiness probe returned an invalid snapshot count",
            ));
        }
        Ok(())
    }

    /// Verify local data-root and transactional DuckLake write capacity without
    /// publishing a table, row, or snapshot.
    pub fn write_readiness_probe(&self) -> EngineResult<()> {
        if !self.lifecycle.is_accepting() {
            return Err(EngineError::new(
                EngineErrorKind::Busy,
                "QuackGIS is draining and cannot start a write-capacity probe",
            ));
        }
        self.local_data_write_probe()?;
        let probe_id = READINESS_PROBE_ID.fetch_add(1, Ordering::Relaxed);
        let table = format!("__readiness_{}_{}", std::process::id(), probe_id);
        let sql = format!(
            "CREATE SCHEMA IF NOT EXISTS {}.{}; \
             CREATE TABLE {}.{}.{}(probe INTEGER)",
            quote_identifier(&self.catalog_name),
            quote_identifier(crate::postgres_compat::INTERNAL_SCHEMA),
            quote_identifier(&self.catalog_name),
            quote_identifier(crate::postgres_compat::INTERNAL_SCHEMA),
            quote_identifier(&table),
        );
        self.rollback_write_probe(&sql)
    }

    pub fn operational_readiness_probe(&self) -> EngineResult<()> {
        self.readiness_probe()?;
        self.write_readiness_probe()
    }

    fn local_data_write_probe(&self) -> EngineResult<()> {
        let probe_id = READINESS_PROBE_ID.fetch_add(1, Ordering::Relaxed);
        let path = self
            .data_path
            .join("_quackgis")
            .join(format!(".readiness-{}-{probe_id}", std::process::id()));
        let result = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(&[0_u8; 4096])?;
            file.sync_data()?;
            drop(file);
            std::fs::remove_file(&path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&path);
            return Err(EngineError::new(
                EngineErrorKind::Internal,
                "local DuckLake data root failed its write-capacity probe",
            ));
        }
        Ok(())
    }

    fn rollback_write_probe(&self, sql: &str) -> EngineResult<()> {
        self.require_transaction_state(EngineTransactionState::Idle)
            .map_err(anyhow_engine_error)?;
        if !self.lifecycle.try_start_transaction() {
            return Err(EngineError::new(
                EngineErrorKind::Busy,
                "QuackGIS is draining and cannot start a write-capacity probe",
            ));
        }
        let mut connection = match self.take_connection_engine() {
            Ok(connection) => connection,
            Err(error) => {
                self.lifecycle.transaction_finished();
                return Err(error);
            }
        };
        if let Err(error) = connection.set_option(OptionConnection::AutoCommit, "false".into()) {
            let _ = self.return_connection(connection);
            self.lifecycle.transaction_finished();
            return Err(engine_error(error));
        }
        if let Err(error) = self.set_transaction_state(EngineTransactionState::Active) {
            let _ = connection.rollback();
            let _ = restore_autocommit(&mut connection);
            let _ = self.return_connection(connection);
            self.lifecycle.transaction_finished();
            return Err(anyhow_engine_error(error));
        }

        let operation = execute_update_engine_on(&mut connection, sql).map(|_| ());
        if let Err(error) = connection.rollback() {
            let _ = self.set_transaction_state(EngineTransactionState::Quarantined);
            return Err(EngineError::new(
                EngineErrorKind::Quarantined,
                format!(
                    "DuckLake write-capacity rollback failed; the probe session was quarantined: {error}"
                ),
            ));
        }
        if let Err(error) = restore_autocommit(&mut connection) {
            let _ = self.set_transaction_state(EngineTransactionState::Quarantined);
            return Err(EngineError::new(
                EngineErrorKind::Quarantined,
                format!(
                    "DuckLake write-capacity rollback cleanup failed; the probe session was quarantined: {error}"
                ),
            ));
        }
        if let Err(error) = self.return_connection(connection) {
            let _ = self.set_transaction_state(EngineTransactionState::Quarantined);
            return Err(anyhow_engine_error(error));
        }
        if let Err(error) = self.set_transaction_state(EngineTransactionState::Idle) {
            self.lifecycle.transaction_finished();
            return Err(anyhow_engine_error(error));
        }
        operation
    }

    pub fn query_stream(self: &Arc<Self>, sql: &str) -> EngineResult<EngineQueryStream> {
        self.query_bound_stream_at_catalog_epoch(sql, None, None)
    }

    pub fn query_bound_stream(
        self: &Arc<Self>,
        sql: &str,
        parameters: Option<RecordBatch>,
    ) -> EngineResult<EngineQueryStream> {
        self.query_bound_stream_at_catalog_epoch(sql, parameters, None)
    }

    pub fn query_bound_stream_at_catalog_epoch(
        self: &Arc<Self>,
        sql: &str,
        parameters: Option<RecordBatch>,
        expected_catalog_epoch: Option<u64>,
    ) -> EngineResult<EngineQueryStream> {
        if parameters
            .as_ref()
            .is_some_and(|batch| batch.num_rows() != 1)
        {
            return Err(EngineError::new(
                EngineErrorKind::InvalidQuery,
                "prepared execution requires exactly one Arrow parameter row",
            ));
        }
        validate_read_query_sql(sql)?;
        let mut connection = self.take_connection_engine()?;
        let catalog_commit = self.catalog_commit_guard().map_err(anyhow_engine_error)?;
        if let Some(expected) = expected_catalog_epoch {
            let current = match catalog_schema_epoch_on(&mut connection, &self.catalog_name) {
                Ok(current) => current,
                Err(error) => {
                    drop(catalog_commit);
                    self.return_connection(connection)
                        .map_err(anyhow_engine_error)?;
                    return Err(error);
                }
            };
            if current != expected {
                drop(catalog_commit);
                self.return_connection(connection)
                    .map_err(anyhow_engine_error)?;
                return Err(EngineError::new(
                    EngineErrorKind::Unsupported,
                    "cached PostgreSQL statement was invalidated by a schema change",
                ));
            }
        }
        let cancellation = Arc::new(DuckDbCancelHandle {
            connection: Mutex::new(connection.clone()),
            requested: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });
        let setup: EngineResult<_> = (|| {
            let sql = prepare_maintained_bbox_query(&mut connection, sql)?;
            let mut statement = connection.new_statement().map_err(engine_error)?;
            statement.set_sql_query(&sql).map_err(engine_error)?;
            if let Some(parameters) = parameters {
                statement.prepare().map_err(engine_error)?;
                statement.bind(parameters).map_err(engine_error)?;
            }
            let reader = statement.execute().map_err(engine_error)?;
            let schema = reader.schema();
            Ok((statement, reader, schema))
        })();
        drop(catalog_commit);
        match setup {
            Ok((statement, reader, schema)) => Ok(EngineQueryStream::new(
                schema,
                Box::new(DuckDbBatchStream {
                    owner: Arc::clone(self),
                    reader: Some(reader),
                    statement: Some(statement),
                    connection: Some(connection),
                    state: BatchStreamState::Active,
                    cancellation: Arc::clone(&cancellation),
                }),
            )
            .with_cancellation(cancellation)),
            Err(error) => {
                let _ = self.return_connection(connection);
                Err(error)
            }
        }
    }

    pub fn start_ingest_operation(self: &Arc<Self>) -> EngineResult<DuckDbIngestOperation> {
        let connection = self.take_connection_engine()?;
        let cancellation = Arc::new(DuckDbCancelHandle {
            connection: Mutex::new(connection.clone()),
            requested: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });
        Ok(DuckDbIngestOperation {
            owner: Arc::clone(self),
            connection: Some(connection),
            cancellation,
        })
    }

    pub fn start_update_operation(self: &Arc<Self>) -> EngineResult<DuckDbUpdateOperation> {
        let connection = self.take_connection_engine()?;
        let cancellation = Arc::new(DuckDbCancelHandle {
            connection: Mutex::new(connection.clone()),
            requested: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });
        Ok(DuckDbUpdateOperation {
            owner: Arc::clone(self),
            connection: Some(connection),
            cancellation,
        })
    }

    /// Ingest Arrow batches directly into the attached DuckLake catalog.
    ///
    /// DuckDB ADBC's target-catalog option routes the operation through the
    /// official DuckLake extension. Data inlining is disabled so durable rows
    /// remain in independently inspectable Parquet files.
    pub fn ingest(
        &self,
        schema: &str,
        table: &str,
        batches: Vec<RecordBatch>,
        mode: IngestMode,
    ) -> Result<Option<i64>> {
        if self.catalog_identity_enabled {
            return self
                .transaction(|transaction| transaction.ingest(schema, table, batches, mode));
        }
        self.with_connection(|connection| {
            ingest_on(connection, &self.catalog_name, schema, table, batches, mode)
        })
    }

    /// Run multiple DuckLake changes under one ADBC transaction.
    ///
    /// DuckLake maps one committed transaction to one visible snapshot. The
    /// callback must use only this transaction handle; mixing SQL `BEGIN`/
    /// `COMMIT` with the ADBC transaction API is intentionally unsupported.
    pub fn transaction<T>(
        &self,
        operation: impl FnOnce(&mut DuckDbAdbcTransaction<'_>) -> Result<T>,
    ) -> Result<T> {
        self.require_transaction_state(EngineTransactionState::Idle)?;
        if !self.lifecycle.try_start_transaction() {
            bail!("QuackGIS is draining and cannot start a new transaction");
        }
        let mut connection = match self.take_connection() {
            Ok(connection) => connection,
            Err(error) => {
                self.lifecycle.transaction_finished();
                return Err(error);
            }
        };
        if let Err(error) = connection.set_option(OptionConnection::AutoCommit, "false".into()) {
            self.return_connection(connection)?;
            self.lifecycle.transaction_finished();
            return Err(error).context("disabling DuckDB ADBC autocommit");
        }
        if let Err(error) = self.set_transaction_state(EngineTransactionState::Active) {
            self.lifecycle.transaction_finished();
            return Err(error);
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            operation(&mut DuckDbAdbcTransaction {
                connection: &mut connection,
                catalog_name: &self.catalog_name,
            })
        }));

        match result {
            Ok(Ok(value)) => {
                let _catalog_commit = match self.catalog_commit_guard() {
                    Ok(guard) => guard,
                    Err(error) => {
                        let _ = connection.rollback();
                        self.set_transaction_state(EngineTransactionState::Quarantined)?;
                        return Err(error).context(
                            "catalog identity commit serialization failed; the connection was quarantined",
                        );
                    }
                };
                if let Err(error) = connection.commit() {
                    // A failed commit has an indeterminate durable outcome. Do
                    // not reuse this native connection or imply rollback.
                    self.set_transaction_state(EngineTransactionState::Quarantined)?;
                    return Err(error).context(
                        "DuckDB ADBC commit failed; transaction outcome is indeterminate and the connection was quarantined",
                    );
                }
                if self.catalog_identity_enabled
                    && let Err(error) =
                        reconcile_catalog_identity_registry_on(&mut connection, &self.catalog_name)
                {
                    let _ = connection.rollback();
                    self.set_transaction_state(EngineTransactionState::Quarantined)?;
                    return Err(error).context(
                        "DuckLake commit succeeded but PostgreSQL catalog identity reconciliation failed; the connection was quarantined",
                    );
                }
                if let Err(error) = restore_autocommit(&mut connection) {
                    self.set_transaction_state(EngineTransactionState::Quarantined)?;
                    return Err(error).context(
                        "DuckDB ADBC commit succeeded but autocommit restoration failed; the connection was quarantined",
                    );
                }
                self.return_connection(connection)?;
                self.set_transaction_state(EngineTransactionState::Idle)?;
                Ok(value)
            }
            Ok(Err(operation_error)) => {
                if let Err(rollback_error) = connection.rollback() {
                    self.set_transaction_state(EngineTransactionState::Quarantined)?;
                    return Err(operation_error.context(format!(
                        "DuckDB ADBC rollback failed; the connection was quarantined: {rollback_error}"
                    )));
                }
                if let Err(autocommit_error) = restore_autocommit(&mut connection) {
                    self.set_transaction_state(EngineTransactionState::Quarantined)?;
                    return Err(operation_error.context(format!(
                        "DuckDB ADBC rollback succeeded but autocommit restoration failed; the connection was quarantined: {autocommit_error}"
                    )));
                }
                self.return_connection(connection)?;
                self.set_transaction_state(EngineTransactionState::Idle)?;
                Err(operation_error)
            }
            Err(panic) => {
                // Preserve Rust panic semantics, but return the connection only
                // when rollback and cleanup both prove it reusable.
                if connection.rollback().is_ok() && restore_autocommit(&mut connection).is_ok() {
                    let _ = self.return_connection(connection);
                    let _ = self.set_transaction_state(EngineTransactionState::Idle);
                } else {
                    let _ = self.set_transaction_state(EngineTransactionState::Quarantined);
                }
                resume_unwind(panic)
            }
        }
    }

    fn connection_slot(&self) -> Result<MutexGuard<'_, Option<ManagedConnection>>> {
        self.connection
            .lock()
            .map_err(|_| anyhow!("DuckDB ADBC connection mutex is poisoned"))
    }

    fn with_connection<T>(
        &self,
        operation: impl FnOnce(&mut ManagedConnection) -> Result<T>,
    ) -> Result<T> {
        let mut slot = self.connection_slot()?;
        let connection = slot.as_mut().ok_or_else(|| {
            anyhow!("DuckDB ADBC connection is busy or quarantined after a native failure")
        })?;
        operation(connection)
    }

    fn take_connection(&self) -> Result<ManagedConnection> {
        self.connection_slot()?.take().ok_or_else(|| {
            anyhow!("DuckDB ADBC connection is busy or quarantined after a native failure")
        })
    }

    fn return_connection(&self, connection: ManagedConnection) -> Result<()> {
        let mut slot = self.connection_slot()?;
        if slot.is_some() {
            bail!("DuckDB ADBC connection slot was unexpectedly occupied");
        }
        *slot = Some(connection);
        Ok(())
    }

    fn require_transaction_state(&self, required: EngineTransactionState) -> Result<()> {
        let state = self.transaction_state();
        if state != required {
            bail!("DuckDB ADBC transaction state is {state:?}; expected {required:?}");
        }
        Ok(())
    }

    fn set_transaction_state(&self, state: EngineTransactionState) -> Result<()> {
        let mut current = self
            .transaction_state
            .lock()
            .map_err(|_| anyhow!("DuckDB ADBC transaction-state mutex is poisoned"))?;
        if state == EngineTransactionState::Quarantined
            && *current != EngineTransactionState::Quarantined
        {
            crate::metrics::connection_quarantined();
        }
        if *current == EngineTransactionState::Active && state != EngineTransactionState::Active {
            self.lifecycle.transaction_finished();
        }
        *current = state;
        Ok(())
    }

    fn catalog_commit_guard(&self) -> Result<Option<MutexGuard<'_, ()>>> {
        self.catalog_identity_enabled
            .then(|| {
                self.catalog_commit_lock
                    .lock()
                    .map_err(|_| anyhow!("catalog identity commit mutex is poisoned"))
            })
            .transpose()
    }

    fn with_connection_engine<T>(
        &self,
        operation: impl FnOnce(&mut ManagedConnection) -> EngineResult<T>,
    ) -> EngineResult<T> {
        let mut slot = self.connection.lock().map_err(|_| {
            EngineError::new(
                EngineErrorKind::Quarantined,
                "DuckDB ADBC connection mutex is poisoned",
            )
        })?;
        let connection = slot
            .as_mut()
            .ok_or_else(|| self.unavailable_connection_error())?;
        operation(connection)
    }

    fn take_connection_engine(&self) -> EngineResult<ManagedConnection> {
        self.connection
            .lock()
            .map_err(|_| {
                EngineError::new(
                    EngineErrorKind::Quarantined,
                    "DuckDB ADBC connection mutex is poisoned",
                )
            })?
            .take()
            .ok_or_else(|| self.unavailable_connection_error())
    }

    fn unavailable_connection_error(&self) -> EngineError {
        if self.transaction_state() == EngineTransactionState::Quarantined {
            EngineError::new(
                EngineErrorKind::Quarantined,
                "DuckDB ADBC connection was quarantined after uncertain native cleanup",
            )
        } else {
            EngineError::new(
                EngineErrorKind::Busy,
                "DuckDB ADBC connection is busy with another native operation",
            )
        }
    }
}

impl DuckDbUpdateOperation {
    pub fn cancellation(&self) -> Arc<dyn EngineCancellation> {
        self.cancellation.clone()
    }

    pub fn execute(
        mut self,
        sql: &str,
        parameters: Option<RecordBatch>,
    ) -> EngineResult<Option<i64>> {
        let mut connection = self.connection.take().ok_or_else(|| {
            EngineError::new(
                EngineErrorKind::Quarantined,
                "DuckDB update connection is unavailable",
            )
        })?;
        let explicit_transaction = self.owner.transaction_state() == EngineTransactionState::Active;
        if !explicit_transaction
            && let Err(error) = connection.set_option(OptionConnection::AutoCommit, "false".into())
        {
            let _ = self.owner.return_connection(connection);
            return Err(engine_error(error));
        }

        let result = (|| {
            let sql = prepare_maintained_bbox_mutation(&mut connection, sql)?;
            match parameters {
                Some(parameters) => {
                    execute_update_engine_on_bound(&mut connection, &sql, parameters)
                }
                None => execute_update_engine_on(&mut connection, &sql),
            }
        })();
        self.cancellation.close();

        if self.cancellation.requested.load(Ordering::Acquire) {
            let rolled_back =
                connection.rollback().is_ok() && restore_autocommit(&mut connection).is_ok();
            if explicit_transaction {
                let _ = self
                    .owner
                    .set_transaction_state(EngineTransactionState::Quarantined);
            } else if rolled_back && self.owner.return_connection(connection).is_ok() {
                return Err(EngineError::new(
                    EngineErrorKind::Cancelled,
                    "DuckDB write was cancelled and rolled back",
                )
                .with_transaction_outcome(TransactionOutcome::RolledBack));
            } else {
                let _ = self
                    .owner
                    .set_transaction_state(EngineTransactionState::Quarantined);
            }
            return Err(EngineError::new(
                if rolled_back {
                    EngineErrorKind::Cancelled
                } else {
                    EngineErrorKind::Quarantined
                },
                if rolled_back {
                    "DuckDB transactional write was cancelled and rolled back; the session was quarantined"
                } else {
                    "DuckDB write cancellation cleanup was uncertain; the session was quarantined"
                },
            )
            .with_transaction_outcome(if rolled_back {
                TransactionOutcome::RolledBack
            } else {
                TransactionOutcome::Indeterminate
            }));
        }

        if explicit_transaction {
            self.owner.return_connection(connection).map_err(|error| {
                EngineError::new(EngineErrorKind::Quarantined, error.to_string())
            })?;
            return result;
        }

        match result {
            Ok(affected) => {
                let _catalog_commit = match self.owner.catalog_commit_guard() {
                    Ok(guard) => guard,
                    Err(error) => {
                        let _ = connection.rollback();
                        let _ = self
                            .owner
                            .set_transaction_state(EngineTransactionState::Quarantined);
                        return Err(EngineError::new(
                            EngineErrorKind::Quarantined,
                            format!(
                                "catalog identity commit serialization failed; the session was quarantined: {error}"
                            ),
                        ));
                    }
                };
                if let Err(error) = connection.commit() {
                    let _ = self
                        .owner
                        .set_transaction_state(EngineTransactionState::Quarantined);
                    return Err(EngineError::new(
                        EngineErrorKind::IndeterminateCommit,
                        format!(
                            "DuckDB ADBC commit failed; outcome is indeterminate and the session was quarantined: {error}"
                        ),
                    )
                    .with_transaction_outcome(TransactionOutcome::Indeterminate));
                }
                if self.owner.catalog_identity_enabled
                    && let Err(error) = reconcile_catalog_identity_registry_on(
                        &mut connection,
                        &self.owner.catalog_name,
                    )
                {
                    let _ = connection.rollback();
                    let _ = self
                        .owner
                        .set_transaction_state(EngineTransactionState::Quarantined);
                    return Err(EngineError::new(
                        EngineErrorKind::Quarantined,
                        format!(
                            "DuckDB write committed but PostgreSQL catalog identity reconciliation failed; the session was quarantined: {error}"
                        ),
                    )
                    .with_transaction_outcome(TransactionOutcome::Committed));
                }
                if let Err(error) = restore_autocommit(&mut connection) {
                    let _ = self
                        .owner
                        .set_transaction_state(EngineTransactionState::Quarantined);
                    return Err(EngineError::new(
                        EngineErrorKind::Quarantined,
                        format!(
                            "DuckDB write committed but autocommit restoration failed; the session was quarantined: {error}"
                        ),
                    )
                    .with_transaction_outcome(TransactionOutcome::Committed));
                }
                self.owner.return_connection(connection).map_err(|error| {
                    EngineError::new(EngineErrorKind::Quarantined, error.to_string())
                })?;
                Ok(affected)
            }
            Err(error) => {
                let clean = connection.rollback().is_ok()
                    && restore_autocommit(&mut connection).is_ok()
                    && self.owner.return_connection(connection).is_ok();
                if !clean {
                    let _ = self
                        .owner
                        .set_transaction_state(EngineTransactionState::Quarantined);
                    return Err(EngineError::new(
                        EngineErrorKind::Quarantined,
                        format!(
                            "DuckDB write failed and rollback cleanup was uncertain; the session was quarantined: {error}"
                        ),
                    ));
                }
                Err(error)
            }
        }
    }
}

impl DuckDbIngestOperation {
    pub fn cancellation(&self) -> Arc<dyn EngineCancellation> {
        self.cancellation.clone()
    }

    pub fn execute(
        mut self,
        table: &EngineTableRef,
        reader: Box<dyn RecordBatchReader + Send>,
        disposition: IngestDisposition,
    ) -> EngineResult<Option<i64>> {
        let mode = match disposition {
            IngestDisposition::Create => IngestMode::Create,
            IngestDisposition::Append => IngestMode::Append,
            IngestDisposition::Replace => IngestMode::Replace,
        };
        let mut connection = self.connection.take().ok_or_else(|| {
            EngineError::new(
                EngineErrorKind::Quarantined,
                "DuckDB ADBC ingestion connection is unavailable",
            )
        })?;
        let reader_schema = reader.schema();
        let target_schema: SchemaRef = connection
            .get_table_schema(Some(&table.catalog), Some(&table.schema), &table.table)
            .map(Into::into)
            .map_err(engine_error)?;
        let stage = format!(
            "__quackgis_copy_{}",
            COPY_STAGE_ID.fetch_add(1, Ordering::Relaxed)
        );
        let columns = reader_schema
            .fields()
            .iter()
            .map(|field| quote_identifier(field.name()))
            .collect::<Vec<_>>()
            .join(", ");
        let target = format!(
            "{}.{}.{}",
            quote_identifier(&table.catalog),
            quote_identifier(&table.schema),
            quote_identifier(&table.table)
        );
        let create_stage = format!(
            "CREATE TEMP TABLE {} AS SELECT {columns} FROM {target} WHERE false",
            quote_identifier(&stage)
        );
        let (publish_columns, publish_values) = match bbox_publish_projection(
            target_schema.as_ref(),
            reader_schema.as_ref(),
            &columns,
        ) {
            Ok(projection) => projection,
            Err(error) => {
                if let Err(return_error) = self.owner.return_connection(connection) {
                    let _ = self
                        .owner
                        .set_transaction_state(EngineTransactionState::Quarantined);
                    return Err(EngineError::new(
                        EngineErrorKind::Quarantined,
                        format!(
                            "DuckDB COPY layout validation failed and the connection could not be returned: {return_error}"
                        ),
                    ));
                }
                return Err(error);
            }
        };
        let publish = format!(
            "INSERT INTO {target} ({publish_columns}) SELECT {publish_values} FROM {}",
            quote_identifier(&stage)
        );
        let result = execute_update_engine_on(&mut connection, &create_stage)
            .and_then(|_| {
                ingest_reader_engine_on(
                    &mut connection,
                    Some("temp"),
                    &table.schema,
                    &stage,
                    reader,
                    mode,
                )
            })
            .and_then(|rows| {
                if self.cancellation.requested.load(Ordering::Acquire) {
                    Err(EngineError::new(
                        EngineErrorKind::Cancelled,
                        "DuckDB ADBC ingestion was cancelled before publication",
                    ))
                } else {
                    Ok(rows)
                }
            })
            .and_then(|_| execute_update_engine_on(&mut connection, &publish));
        let published = result.is_ok();
        let drop_stage = format!("DROP TABLE IF EXISTS {}", quote_identifier(&stage));
        let cleanup_result = execute_update_engine_on(&mut connection, &drop_stage);
        let cancelled = self.cancellation.requested.load(Ordering::Acquire);
        if cancelled || cleanup_result.is_err() {
            let _ = self
                .owner
                .set_transaction_state(EngineTransactionState::Quarantined);
            drop(connection);
        } else if let Err(error) = self.owner.return_connection(connection) {
            let _ = self
                .owner
                .set_transaction_state(EngineTransactionState::Quarantined);
            return Err(EngineError::new(
                EngineErrorKind::Quarantined,
                error.to_string(),
            ));
        }
        if published {
            return result;
        }
        if cancelled {
            return Err(EngineError::new(
                EngineErrorKind::Cancelled,
                "DuckDB ADBC ingestion was cancelled",
            ));
        }
        if let Err(error) = cleanup_result {
            return Err(EngineError::new(
                EngineErrorKind::Quarantined,
                format!("DuckDB COPY staging cleanup failed: {error}"),
            ));
        }
        result
    }
}

fn bbox_publish_projection(
    target_schema: &arrow_schema::Schema,
    input_schema: &arrow_schema::Schema,
    input_columns: &str,
) -> EngineResult<(String, String)> {
    let Some(layout) = inspect_maintained_bbox_layout(target_schema)? else {
        return Ok((input_columns.to_owned(), input_columns.to_owned()));
    };
    for name in MAINTAINED_BBOX_COLUMNS {
        if input_schema.field_with_name(name).is_ok() {
            return Err(EngineError::new(
                EngineErrorKind::Unsupported,
                "COPY input must not supply reserved bbox columns",
            ));
        }
    }

    let geometry = input_schema
        .field_with_name(&layout.geometry)
        .ok()
        .filter(|field| classify_spatial_field(field) == Some(SpatialFamily::Geometry))
        .map(|field| quote_identifier(field.name()));
    let bbox_columns = MAINTAINED_BBOX_COLUMNS
        .iter()
        .map(|name| quote_identifier(name))
        .collect::<Vec<_>>()
        .join(", ");
    let accessors = if let Some(geometry) = geometry {
        bbox_accessor_values(&geometry)
    } else {
        ["NULL", "NULL", "NULL", "NULL"].join(", ")
    };
    Ok((
        format!("{input_columns}, {bbox_columns}"),
        format!("{input_columns}, {accessors}"),
    ))
}

fn inspect_maintained_bbox_layout(
    schema: &arrow_schema::Schema,
) -> EngineResult<Option<MaintainedBboxLayout>> {
    let bbox_count = MAINTAINED_BBOX_COLUMNS
        .iter()
        .filter(|name| schema.field_with_name(name).is_ok())
        .count();
    if bbox_count == 0 {
        return Ok(None);
    }
    if bbox_count != MAINTAINED_BBOX_COLUMNS.len() {
        return Err(EngineError::new(
            EngineErrorKind::Unsupported,
            "DuckDB table has a partial reserved bbox layout",
        ));
    }
    for name in MAINTAINED_BBOX_COLUMNS {
        let field = schema
            .field_with_name(name)
            .expect("all reserved bbox columns were counted");
        if field.data_type() != &arrow_schema::DataType::Float64 || !field.is_nullable() {
            return Err(EngineError::new(
                EngineErrorKind::Unsupported,
                "reserved bbox columns must all be nullable DOUBLE values",
            ));
        }
    }

    let geometry = schema
        .fields()
        .iter()
        .filter(|field| classify_spatial_field(field) == Some(SpatialFamily::Geometry))
        .collect::<Vec<_>>();
    if geometry.len() != 1 {
        return Err(EngineError::new(
            EngineErrorKind::Unsupported,
            "reserved bbox layout requires exactly one recognized geometry column",
        ));
    }
    Ok(Some(MaintainedBboxLayout {
        geometry: geometry[0].name().to_owned(),
    }))
}

fn bbox_accessor_values(geometry: &str) -> String {
    ["ST_XMin", "ST_YMin", "ST_XMax", "ST_YMax"]
        .into_iter()
        .map(|accessor| {
            format!(
                "CASE WHEN {geometry} IS NULL THEN NULL ELSE {accessor}(ST_Extent(ST_GeomFromWKB({geometry}))) END"
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl EngineStorageKernel for DuckDbAdbcStorage {
    fn describe(&self, sql: &str) -> EngineResult<EngineStatementDescription> {
        self.with_connection_engine(|connection| describe_on(connection, sql))
    }

    fn query_result(&self, sql: &str) -> EngineResult<EngineQueryResult> {
        self.with_connection_engine(|connection| query_result_on(connection, sql, None))
    }

    fn query_bound(&self, sql: &str, parameters: RecordBatch) -> EngineResult<EngineQueryResult> {
        if parameters.num_rows() != 1 {
            return Err(EngineError::new(
                EngineErrorKind::InvalidQuery,
                "prepared execution requires exactly one Arrow parameter row",
            ));
        }
        self.with_connection_engine(|connection| query_result_on(connection, sql, Some(parameters)))
    }

    fn execute_update_contract(&self, sql: &str) -> EngineResult<Option<i64>> {
        if self.catalog_identity_enabled {
            return self
                .transaction(|transaction| {
                    let sql = prepare_maintained_bbox_mutation(transaction.connection, sql)
                        .map_err(anyhow::Error::new)?;
                    execute_update_engine_on(transaction.connection, &sql)
                        .map_err(anyhow::Error::new)
                })
                .map_err(anyhow_engine_error);
        }
        self.with_connection_engine(|connection| {
            let sql = prepare_maintained_bbox_mutation(connection, sql)?;
            execute_update_engine_on(connection, &sql)
        })
    }

    fn execute_update_bound(
        &self,
        sql: &str,
        parameters: RecordBatch,
    ) -> EngineResult<Option<i64>> {
        if parameters.num_rows() != 1 {
            return Err(EngineError::new(
                EngineErrorKind::InvalidQuery,
                "prepared execution requires exactly one Arrow parameter row",
            ));
        }
        if self.catalog_identity_enabled {
            return self
                .transaction(|transaction| {
                    let sql = prepare_maintained_bbox_mutation(transaction.connection, sql)
                        .map_err(anyhow::Error::new)?;
                    execute_update_engine_on_bound(transaction.connection, &sql, parameters)
                        .map_err(anyhow::Error::new)
                })
                .map_err(anyhow_engine_error);
        }
        self.with_connection_engine(|connection| {
            let sql = prepare_maintained_bbox_mutation(connection, sql)?;
            execute_update_engine_on_bound(connection, &sql, parameters)
        })
    }

    fn table_schema(&self, table: &EngineTableRef) -> EngineResult<SchemaRef> {
        self.with_connection_engine(|connection| {
            connection
                .get_table_schema(Some(&table.catalog), Some(&table.schema), &table.table)
                .map(Into::into)
                .map_err(engine_error)
        })
    }

    fn ingest_contract(
        &self,
        table: &EngineTableRef,
        batches: Vec<RecordBatch>,
        disposition: IngestDisposition,
    ) -> EngineResult<Option<i64>> {
        let mode = match disposition {
            IngestDisposition::Create => IngestMode::Create,
            IngestDisposition::Append => IngestMode::Append,
            IngestDisposition::Replace => IngestMode::Replace,
        };
        if self.catalog_identity_enabled {
            if table.catalog != self.catalog_name {
                return Err(EngineError::new(
                    EngineErrorKind::Unsupported,
                    "catalog identity reconciliation supports only the configured DuckLake catalog",
                ));
            }
            return self
                .transaction(|transaction| {
                    transaction.ingest(&table.schema, &table.table, batches, mode)
                })
                .map_err(anyhow_engine_error);
        }
        self.with_connection_engine(|connection| {
            ingest_engine_on(
                connection,
                &table.catalog,
                &table.schema,
                &table.table,
                batches,
                mode,
            )
        })
    }

    fn snapshots(&self) -> EngineResult<Vec<EngineSnapshot>> {
        let sql = format!(
            "SELECT CAST(snapshot_id AS BIGINT), CAST(snapshot_time AS VARCHAR) \
             FROM ducklake_snapshots({}) ORDER BY snapshot_id",
            quote_literal(&self.catalog_name)
        );
        let result = self.query_result(&sql)?;
        let mut snapshots = Vec::new();
        for batch in result.batches {
            let ids = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| {
                    EngineError::new(EngineErrorKind::Internal, "snapshot id is not Int64")
                })?;
            let timestamps = batch
                .column(1)
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| {
                    EngineError::new(EngineErrorKind::Internal, "snapshot timestamp is not Utf8")
                })?;
            for row in 0..batch.num_rows() {
                snapshots.push(EngineSnapshot {
                    id: ids.value(row),
                    timestamp: timestamps.value(row).to_owned(),
                });
            }
        }
        Ok(snapshots)
    }

    fn resource_sample(&self) -> EngineResult<EngineResourceSample> {
        let result = self.query_result(
            "SELECT CAST(COALESCE(sum(memory_usage_bytes), 0) AS BIGINT), \
                    CAST(COALESCE(sum(temporary_storage_bytes), 0) AS BIGINT) \
             FROM duckdb_memory()",
        )?;
        let batch = result.batches.first().ok_or_else(|| {
            EngineError::new(
                EngineErrorKind::Internal,
                "DuckDB resource sample returned no batch",
            )
        })?;
        let value = |column: usize, label: &str| -> EngineResult<u64> {
            let values = batch
                .column(column)
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| {
                    EngineError::new(
                        EngineErrorKind::Internal,
                        format!("DuckDB {label} resource sample is not Int64"),
                    )
                })?;
            u64::try_from(values.value(0)).map_err(|_| {
                EngineError::new(
                    EngineErrorKind::Internal,
                    format!("DuckDB {label} resource sample is negative"),
                )
            })
        };
        Ok(EngineResourceSample {
            memory_bytes: value(0, "memory")?,
            temporary_storage_bytes: value(1, "temporary storage")?,
        })
    }

    fn maintain(&self, request: EngineMaintenanceRequest) -> EngineResult<EngineMaintenanceReport> {
        let sql = match request {
            EngineMaintenanceRequest::MergeAdjacentFiles {
                schema,
                table,
                max_compacted_files,
                max_file_size,
                min_file_size,
            } => merge_adjacent_files_sql(
                &self.catalog_name,
                &schema,
                &table,
                max_compacted_files,
                max_file_size,
                min_file_size,
            ),
            EngineMaintenanceRequest::RewriteDataFiles {
                schema,
                table,
                delete_threshold,
            } => {
                if !(0.0..=1.0).contains(&delete_threshold) {
                    return Err(EngineError::new(
                        EngineErrorKind::InvalidQuery,
                        "DuckLake delete_threshold must be between 0 and 1",
                    ));
                }
                format!(
                    "CALL ducklake_rewrite_data_files({}, {}, schema => {}, delete_threshold => {})",
                    quote_literal(&self.catalog_name),
                    quote_literal(&table),
                    quote_literal(&schema),
                    delete_threshold,
                )
            }
        };
        self.execute_update_contract(&sql)
            .map(|affected_rows| EngineMaintenanceReport { affected_rows })
    }
}

fn prepare_maintained_bbox_query(
    connection: &mut ManagedConnection,
    sql: &str,
) -> EngineResult<String> {
    let Ok(mut statements) = Parser::parse_sql(&PostgreSqlDialect {}, sql) else {
        return Ok(sql.to_owned());
    };
    if statements.len() != 1 {
        return Ok(sql.to_owned());
    }
    let mut statement = statements.pop().expect("one query statement");
    let Some(target) = bbox_query_target(&statement) else {
        return Ok(sql.to_owned());
    };
    let schema = connection
        .get_table_schema(
            Some(&target.table.catalog),
            Some(&target.table.schema),
            &target.table.table,
        )
        .map_err(engine_error)?;
    let Some(layout) = inspect_maintained_bbox_layout(&schema)? else {
        return Ok(sql.to_owned());
    };
    if !identifier_matches(&target.geometry, &layout.geometry) {
        return Ok(sql.to_owned());
    }
    inject_bbox_candidate(&mut statement, &target)?;
    Ok(statement.to_string())
}

fn inject_bbox_candidate(
    statement: &mut SqlStatement,
    target: &BboxQueryTarget,
) -> EngineResult<()> {
    let candidate = bbox_candidate_expression(&target.qualifier, &target.probe)?;
    let query = bbox_query_mut(statement).expect("query target was already classified");
    let SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!("bbox query target requires a SELECT")
    };
    let exact = select.selection.take().expect("bbox target requires WHERE");
    select.selection = Some(Expr::BinaryOp {
        left: Box::new(candidate),
        op: BinaryOperator::And,
        right: Box::new(exact),
    });
    Ok(())
}

fn bbox_query(statement: &SqlStatement) -> Option<&Query> {
    match statement {
        SqlStatement::Query(query) => Some(query),
        SqlStatement::Explain { statement, .. } => match statement.as_ref() {
            SqlStatement::Query(query) => Some(query),
            _ => None,
        },
        _ => None,
    }
}

fn bbox_query_mut(statement: &mut SqlStatement) -> Option<&mut Query> {
    match statement {
        SqlStatement::Query(query) => Some(query),
        SqlStatement::Explain { statement, .. } => match statement.as_mut() {
            SqlStatement::Query(query) => Some(query),
            _ => None,
        },
        _ => None,
    }
}

fn bbox_query_target(statement: &SqlStatement) -> Option<BboxQueryTarget> {
    let query = bbox_query(statement)?;
    if query.with.is_some() {
        return None;
    }
    let SetExpr::Select(select) = query.body.as_ref() else {
        return None;
    };
    let [from] = select.from.as_slice() else {
        return None;
    };
    if !from.joins.is_empty() || !select.lateral_views.is_empty() || select.prewhere.is_some() {
        return None;
    }
    let TableFactor::Table {
        name,
        alias,
        args: None,
        with_hints,
        version: None,
        with_ordinality: false,
        partitions,
        json_path: None,
        sample: None,
        index_hints,
    } = &from.relation
    else {
        return None;
    };
    if !with_hints.is_empty() || !partitions.is_empty() || !index_hints.is_empty() {
        return None;
    }
    if alias
        .as_ref()
        .is_some_and(|alias| !alias.columns.is_empty())
    {
        return None;
    }
    let table = local_table_ref(name).ok()?;
    let qualifier =
        alias
            .as_ref()
            .map(|alias| alias.name.clone())
            .or_else(|| match name.0.last() {
                Some(ObjectNamePart::Identifier(identifier)) => Some(identifier.clone()),
                _ => None,
            })?;
    let selection = select.selection.as_ref()?;
    let mut predicates = Vec::new();
    collect_mandatory_bbox_predicates(selection, &qualifier, &mut predicates);
    let [(geometry, probe)] = predicates.as_slice() else {
        return None;
    };
    Some(BboxQueryTarget {
        table,
        qualifier,
        geometry: geometry.clone(),
        probe: probe.clone(),
    })
}

fn collect_mandatory_bbox_predicates(
    expression: &Expr,
    qualifier: &Ident,
    predicates: &mut Vec<(Ident, Expr)>,
) {
    match expression {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            collect_mandatory_bbox_predicates(left, qualifier, predicates);
            collect_mandatory_bbox_predicates(right, qualifier, predicates);
        }
        Expr::Nested(expression) => {
            collect_mandatory_bbox_predicates(expression, qualifier, predicates);
        }
        Expr::Function(function) => {
            if let Some(predicate) = maintained_intersection(function, qualifier) {
                predicates.push(predicate);
            }
        }
        _ => {}
    }
}

fn maintained_intersection(function: &Function, qualifier: &Ident) -> Option<(Ident, Expr)> {
    if !plain_function_named(function, "st_intersects") {
        return None;
    }
    let arguments = plain_function_arguments(function)?;
    let [left, right] = arguments.as_slice() else {
        return None;
    };
    for (geometry, probe) in [(*left, *right), (*right, *left)] {
        if let Some(column) = maintained_geometry_column(geometry, qualifier)
            && stable_probe_geometry(probe)
        {
            return Some((column, probe.clone()));
        }
    }
    None
}

fn maintained_geometry_column(expression: &Expr, qualifier: &Ident) -> Option<Ident> {
    let Expr::Function(function) = expression else {
        return None;
    };
    if !plain_function_named(function, "st_geomfromwkb") {
        return None;
    }
    let arguments = plain_function_arguments(function)?;
    let [column] = arguments.as_slice() else {
        return None;
    };
    match column {
        Expr::Identifier(column) => Some(column.clone()),
        Expr::CompoundIdentifier(parts) if matches!(parts.as_slice(), [source, _] if identifiers_match(source, qualifier)) => {
            parts.last().cloned()
        }
        _ => None,
    }
}

fn stable_probe_geometry(expression: &Expr) -> bool {
    let Expr::Function(function) = expression else {
        return false;
    };
    let Some(arguments) = plain_function_arguments(function) else {
        return false;
    };
    if plain_function_named(function, "st_makeenvelope") {
        arguments.len() == 4 && arguments.into_iter().all(stable_numeric_value)
    } else if plain_function_named(function, "st_geomfromtext") {
        matches!(arguments.as_slice(), [value] if stable_string_value(value))
    } else if plain_function_named(function, "st_geomfromwkb") {
        matches!(arguments.as_slice(), [value] if stable_bound_value(value))
    } else {
        false
    }
}

fn plain_function_named(function: &Function, expected: &str) -> bool {
    matches!(
        function.name.0.as_slice(),
        [ObjectNamePart::Identifier(identifier)]
            if identifier.quote_style.is_none() && identifier.value.eq_ignore_ascii_case(expected)
    ) && !function.uses_odbc_syntax
        && matches!(function.parameters, FunctionArguments::None)
        && function.filter.is_none()
        && function.null_treatment.is_none()
        && function.over.is_none()
        && function.within_group.is_empty()
}

fn plain_function_arguments(function: &Function) -> Option<Vec<&Expr>> {
    let FunctionArguments::List(arguments) = &function.args else {
        return None;
    };
    if arguments.duplicate_treatment.is_some() || !arguments.clauses.is_empty() {
        return None;
    }
    arguments
        .args
        .iter()
        .map(|argument| match argument {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(expression)) => Some(expression),
            _ => None,
        })
        .collect()
}

fn stable_numeric_value(expression: &Expr) -> bool {
    match expression {
        Expr::Value(value) => match &value.value {
            Value::Number(value, false) => {
                value.len() <= MAX_BBOX_NUMERIC_LITERAL_BYTES
                    && value.parse::<f64>().is_ok_and(f64::is_finite)
            }
            Value::Placeholder(value) => numbered_parameter(value),
            _ => false,
        },
        Expr::UnaryOp {
            op: UnaryOperator::Plus | UnaryOperator::Minus,
            expr,
        }
        | Expr::Nested(expr)
        | Expr::Cast { expr, .. } => stable_numeric_value(expr),
        _ => false,
    }
}

fn stable_string_value(expression: &Expr) -> bool {
    match expression {
        Expr::Value(value) => {
            matches!(&value.value, Value::SingleQuotedString(value) if value.len() <= MAX_BBOX_WKT_BYTES)
        }
        Expr::Nested(expr) | Expr::Cast { expr, .. } => stable_string_value(expr),
        _ => false,
    }
}

fn stable_bound_value(expression: &Expr) -> bool {
    match expression {
        Expr::Value(value) => {
            matches!(&value.value, Value::Placeholder(value) if numbered_parameter(value))
        }
        Expr::Nested(expr) | Expr::Cast { expr, .. } => stable_bound_value(expr),
        _ => false,
    }
}

fn numbered_parameter(value: &str) -> bool {
    value
        .strip_prefix(char::from(36_u8))
        .and_then(|digits| digits.parse::<usize>().ok())
        .is_some_and(|index| index > 0)
}

fn bbox_candidate_expression(qualifier: &Ident, probe: &Expr) -> EngineResult<Expr> {
    let qualifier = qualifier.to_string();
    let probe = probe.to_string();
    let sql = format!(
        "SELECT 1 WHERE \
         {qualifier}._qg_maxx >= ST_XMin(ST_Extent({probe})) AND \
         {qualifier}._qg_minx <= ST_XMax(ST_Extent({probe})) AND \
         {qualifier}._qg_maxy >= ST_YMin(ST_Extent({probe})) AND \
         {qualifier}._qg_miny <= ST_YMax(ST_Extent({probe}))"
    );
    let mut statements = Parser::parse_sql(&PostgreSqlDialect {}, &sql).map_err(|error| {
        EngineError::new(
            EngineErrorKind::Internal,
            format!("cannot build maintained bbox candidate: {error}"),
        )
    })?;
    let SqlStatement::Query(query) = statements.remove(0) else {
        unreachable!("generated bbox candidate is SELECT")
    };
    let SetExpr::Select(select) = query.body.as_ref() else {
        unreachable!("generated bbox candidate is SELECT")
    };
    select
        .selection
        .clone()
        .ok_or_else(|| EngineError::new(EngineErrorKind::Internal, "bbox candidate has no filter"))
}

fn identifier_matches(identifier: &Ident, expected: &str) -> bool {
    if identifier.quote_style.is_some() {
        identifier.value == expected
    } else {
        identifier.value.eq_ignore_ascii_case(expected)
    }
}

fn identifiers_match(left: &Ident, right: &Ident) -> bool {
    match (left.quote_style, right.quote_style) {
        (Some(_), _) | (_, Some(_)) => left.value == right.value,
        (None, None) => left.value.eq_ignore_ascii_case(&right.value),
    }
}

fn prepare_maintained_bbox_mutation(
    connection: &mut ManagedConnection,
    sql: &str,
) -> EngineResult<String> {
    let mut statements = Parser::parse_sql(&PostgreSqlDialect {}, sql).map_err(|error| {
        EngineError::new(
            EngineErrorKind::InvalidQuery,
            format!("cannot inspect DuckDB mutation: {error}"),
        )
    })?;
    if statements.len() != 1 {
        return Err(EngineError::new(
            EngineErrorKind::InvalidQuery,
            "DuckDB mutation inspection requires exactly one statement",
        ));
    }
    let mut statement = statements.pop().expect("one mutation statement");
    let name = match &statement {
        SqlStatement::Insert(insert) => match &insert.table {
            TableObject::TableName(name) => Some(name),
            _ => None,
        },
        SqlStatement::Update(update) => match &update.table.relation {
            TableFactor::Table { name, .. } => Some(name),
            _ => None,
        },
        _ => return Ok(sql.to_owned()),
    }
    .ok_or_else(|| {
        EngineError::new(
            EngineErrorKind::Unsupported,
            "unsupported DuckDB mutation target shape",
        )
    })?;
    let table = local_table_ref(name)?;
    let schema = connection
        .get_table_schema(Some(&table.catalog), Some(&table.schema), &table.table)
        .map_err(engine_error)?;
    if let Some(layout) = inspect_maintained_bbox_layout(&schema)? {
        match &mut statement {
            SqlStatement::Insert(_) => {
                return Err(EngineError::new(
                    EngineErrorKind::Unsupported,
                    "direct INSERT on a maintained bbox table is unsupported; use COPY",
                ));
            }
            SqlStatement::Update(update) => {
                rewrite_safe_bbox_update(&layout, &mut update.assignments)?;
            }
            _ => unreachable!("mutation kind classified above"),
        }
        return Ok(statement.to_string());
    }
    Ok(sql.to_owned())
}

fn rewrite_safe_bbox_update(
    layout: &MaintainedBboxLayout,
    assignments: &mut Vec<sqlparser::ast::Assignment>,
) -> EngineResult<()> {
    let mut geometry_value = None;
    for assignment in assignments.iter() {
        let targets = match &assignment.target {
            AssignmentTarget::ColumnName(target) => std::slice::from_ref(target),
            AssignmentTarget::Tuple(targets) => targets.as_slice(),
        };
        for target in targets {
            let Some(name) = target.0.last().and_then(|part| match part {
                ObjectNamePart::Identifier(identifier) => Some(identifier.value.as_str()),
                _ => None,
            }) else {
                return Err(EngineError::new(
                    EngineErrorKind::Unsupported,
                    "maintained bbox UPDATE targets must be identifiers",
                ));
            };
            if MAINTAINED_BBOX_COLUMNS
                .iter()
                .any(|bbox| name.eq_ignore_ascii_case(bbox))
            {
                return Err(EngineError::new(
                    EngineErrorKind::Unsupported,
                    "maintained bbox UPDATE cannot assign reserved bbox columns",
                ));
            }
            if name.eq_ignore_ascii_case(&layout.geometry) {
                if targets.len() != 1 || geometry_value.is_some() {
                    return Err(EngineError::new(
                        EngineErrorKind::Unsupported,
                        "maintained bbox UPDATE must assign geometry exactly once outside a tuple",
                    ));
                }
                if !is_stable_geometry_update(&assignment.value) {
                    return Err(EngineError::new(
                        EngineErrorKind::Unsupported,
                        "maintained bbox geometry UPDATE requires a numbered parameter or NULL",
                    ));
                }
                geometry_value = Some(assignment.value.clone());
            }
        }
    }
    let Some(geometry_value) = geometry_value else {
        return Ok(());
    };
    let generated = format!(
        "UPDATE qg SET {}",
        MAINTAINED_BBOX_COLUMNS
            .iter()
            .zip(["ST_XMin", "ST_YMin", "ST_XMax", "ST_YMax"])
            .map(|(column, accessor)| format!(
                "{} = CASE WHEN {geometry_value} IS NULL THEN NULL ELSE {accessor}(ST_Extent(ST_GeomFromWKB({geometry_value}))) END",
                quote_identifier(column),
            ))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut generated = Parser::parse_sql(&PostgreSqlDialect {}, &generated).map_err(|error| {
        EngineError::new(
            EngineErrorKind::Internal,
            format!("cannot build maintained bbox UPDATE: {error}"),
        )
    })?;
    let SqlStatement::Update(generated) = generated.remove(0) else {
        unreachable!("generated bbox statement is UPDATE")
    };
    assignments.extend(generated.assignments);
    Ok(())
}

fn is_stable_geometry_update(expression: &Expr) -> bool {
    match expression {
        Expr::Value(value) => match &value.value {
            Value::Null => true,
            Value::Placeholder(placeholder) => {
                placeholder.strip_prefix('$').is_some_and(|digits| {
                    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                })
            }
            _ => false,
        },
        Expr::Cast { expr, .. } | Expr::Nested(expr) => is_stable_geometry_update(expr),
        _ => false,
    }
}

fn local_table_ref(name: &ObjectName) -> EngineResult<EngineTableRef> {
    let parts = name
        .0
        .iter()
        .map(|part| match part {
            ObjectNamePart::Identifier(identifier) => Some(identifier.value.as_str()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            EngineError::new(
                EngineErrorKind::Unsupported,
                "DuckDB mutation target must contain identifiers only",
            )
        })?;
    let (catalog, schema, table) = match parts.as_slice() {
        [table] => ("quackgis", "main", *table),
        [schema, table]
            if schema.eq_ignore_ascii_case("main") || schema.eq_ignore_ascii_case("public") =>
        {
            ("quackgis", "main", *table)
        }
        [catalog, schema, table]
            if catalog.eq_ignore_ascii_case("quackgis")
                && (schema.eq_ignore_ascii_case("main")
                    || schema.eq_ignore_ascii_case("public")) =>
        {
            ("quackgis", "main", *table)
        }
        _ => {
            return Err(EngineError::new(
                EngineErrorKind::Unsupported,
                "DuckDB mutation target must be a local QuackGIS table",
            ));
        }
    };
    Ok(EngineTableRef {
        catalog: catalog.to_owned(),
        schema: schema.to_owned(),
        table: table.to_owned(),
    })
}

/// Operations available inside one DuckDB/DuckLake transaction.
pub struct DuckDbAdbcTransaction<'a> {
    connection: &'a mut ManagedConnection,
    catalog_name: &'a str,
}

impl DuckDbAdbcTransaction<'_> {
    pub fn execute_update(&mut self, sql: &str) -> Result<Option<i64>> {
        execute_update_on(self.connection, sql)
    }

    pub fn query(&mut self, sql: &str) -> Result<Vec<RecordBatch>> {
        query_on(self.connection, sql)
    }

    pub fn ingest(
        &mut self,
        schema: &str,
        table: &str,
        batches: Vec<RecordBatch>,
        mode: IngestMode,
    ) -> Result<Option<i64>> {
        ingest_on(
            self.connection,
            self.catalog_name,
            schema,
            table,
            batches,
            mode,
        )
    }
}

fn execute_update_on(connection: &mut ManagedConnection, sql: &str) -> Result<Option<i64>> {
    if sql.trim().is_empty() {
        bail!("refusing to execute empty DuckDB ADBC SQL");
    }
    let mut statement = connection
        .new_statement()
        .context("creating DuckDB ADBC statement")?;
    statement
        .set_sql_query(sql)
        .context("setting DuckDB ADBC SQL")?;
    statement
        .execute_update()
        .context("executing DuckDB ADBC update")
}

fn execute_update_engine_on(
    connection: &mut ManagedConnection,
    sql: &str,
) -> EngineResult<Option<i64>> {
    validate_sql(sql)?;
    let mut statement = connection.new_statement().map_err(engine_error)?;
    statement.set_sql_query(sql).map_err(engine_error)?;
    statement.execute_update().map_err(engine_error)
}

fn restore_autocommit(connection: &mut ManagedConnection) -> Result<()> {
    connection
        .set_option(OptionConnection::AutoCommit, "true".into())
        .context("restoring DuckDB ADBC autocommit")
}

fn initialize_catalog_identity_registry_on(
    connection: &mut ManagedConnection,
    catalog: &str,
) -> Result<()> {
    execute_update_on(
        connection,
        &crate::postgres_compat::ducklake_identity_registry_bootstrap_sql(catalog),
    )
    .context("creating DuckLake catalog identity registry")?;
    connection
        .set_option(OptionConnection::AutoCommit, "false".into())
        .context("starting DuckLake catalog identity reconciliation")?;
    match reconcile_catalog_identity_registry_on(connection, catalog) {
        Ok(()) => restore_autocommit(connection),
        Err(error) => {
            let rollback = connection
                .rollback()
                .context("rolling back catalog identity initialization");
            let autocommit = restore_autocommit(connection);
            if let Err(cleanup_error) = rollback.and(autocommit) {
                return Err(error.context(format!(
                    "catalog identity initialization cleanup failed: {cleanup_error}"
                )));
            }
            Err(error)
        }
    }
}

/// Reconcile one committed DuckLake snapshot in a separate atomic transaction.
/// The caller keeps autocommit disabled across the preceding user commit and
/// this function's registry commit.
fn reconcile_catalog_identity_registry_on(
    connection: &mut ManagedConnection,
    catalog: &str,
) -> Result<()> {
    validate_catalog_identity_registry_on(connection, catalog)?;
    execute_update_on(
        connection,
        &crate::postgres_compat::ducklake_identity_registry_reconcile_sql(catalog),
    )
    .context("reconciling committed DuckLake identities")?;
    validate_catalog_identity_registry_on(connection, catalog)?;
    validate_catalog_identity_coverage_on(connection, catalog)?;
    connection
        .commit()
        .context("committing PostgreSQL catalog identity reconciliation")
}

fn validate_catalog_identity_registry_on(
    connection: &mut ManagedConnection,
    catalog: &str,
) -> Result<()> {
    let batches = query_on(
        connection,
        &crate::postgres_compat::ducklake_identity_registry_validation_sql(catalog),
    )
    .context("validating DuckLake catalog identity registry")?;
    if batches.len() != 1 || batches[0].num_rows() != 1 {
        bail!("DuckLake catalog identity validation returned an invalid result shape");
    }
    Ok(())
}

fn validate_catalog_identity_coverage_on(
    connection: &mut ManagedConnection,
    catalog: &str,
) -> Result<()> {
    let batches = query_on(
        connection,
        &crate::postgres_compat::ducklake_identity_registry_coverage_sql(catalog),
    )
    .context("validating committed DuckLake catalog identity coverage")?;
    if batches.len() != 1 || batches[0].num_rows() != 1 {
        bail!("DuckLake catalog identity coverage returned an invalid result shape");
    }
    Ok(())
}

fn catalog_schema_epoch_on(connection: &mut ManagedConnection, catalog: &str) -> EngineResult<u64> {
    let sql = format!(
        "SELECT CAST(schema_epoch AS BIGINT) FROM {}.{}.catalog_state WHERE singleton",
        quote_identifier(catalog),
        quote_identifier(crate::postgres_compat::INTERNAL_SCHEMA),
    );
    let result = query_result_on(connection, &sql, None)?;
    let batch = result.batches.first().ok_or_else(|| {
        EngineError::new(
            EngineErrorKind::Internal,
            "catalog schema epoch query returned no batch",
        )
    })?;
    if result.batches.len() != 1 || batch.num_rows() != 1 {
        return Err(EngineError::new(
            EngineErrorKind::Internal,
            "catalog schema epoch query returned an invalid result shape",
        ));
    }
    let epoch = int64_column(batch, 0, "catalog schema epoch")?.value(0);
    u64::try_from(epoch).map_err(|_| {
        EngineError::new(
            EngineErrorKind::Internal,
            "catalog schema epoch is outside the supported range",
        )
    })
}

fn int64_column<'a>(
    batch: &'a RecordBatch,
    column: usize,
    label: &str,
) -> EngineResult<&'a Int64Array> {
    batch
        .column(column)
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| EngineError::new(EngineErrorKind::Internal, format!("{label} is not Int64")))
}

fn query_on(connection: &mut ManagedConnection, sql: &str) -> Result<Vec<RecordBatch>> {
    validate_read_query_sql(sql).map_err(anyhow::Error::new)?;
    let sql = prepare_maintained_bbox_query(connection, sql).map_err(anyhow::Error::new)?;
    let mut statement = connection
        .new_statement()
        .context("creating DuckDB ADBC statement")?;
    statement
        .set_sql_query(&sql)
        .context("setting DuckDB ADBC SQL")?;
    statement
        .execute()
        .context("executing DuckDB ADBC query")?
        .collect::<Result<Vec<_>, ArrowError>>()
        .context("reading DuckDB ADBC Arrow result")
}

fn describe_on(
    connection: &mut ManagedConnection,
    sql: &str,
) -> EngineResult<EngineStatementDescription> {
    validate_sql(sql)?;
    let sql = prepare_maintained_bbox_query(connection, sql)?;
    let mut statement = connection.new_statement().map_err(engine_error)?;
    statement.set_sql_query(&sql).map_err(engine_error)?;
    statement.prepare().map_err(engine_error)?;
    let parameter_schema = statement.get_parameter_schema().map_err(engine_error)?;
    let result_schema = statement.execute_schema().map_err(engine_error)?;
    Ok(EngineStatementDescription {
        parameter_schema: parameter_schema.into(),
        result_schema: result_schema.into(),
    })
}

fn query_result_on(
    connection: &mut ManagedConnection,
    sql: &str,
    parameters: Option<RecordBatch>,
) -> EngineResult<EngineQueryResult> {
    validate_read_query_sql(sql)?;
    let sql = prepare_maintained_bbox_query(connection, sql)?;
    let mut statement = connection.new_statement().map_err(engine_error)?;
    statement.set_sql_query(&sql).map_err(engine_error)?;
    if let Some(parameters) = parameters {
        statement.prepare().map_err(engine_error)?;
        statement.bind(parameters).map_err(engine_error)?;
    }
    let reader = statement.execute().map_err(engine_error)?;
    let schema = reader.schema();
    let batches = reader
        .collect::<Result<Vec<_>, ArrowError>>()
        .map_err(|error| EngineError::new(EngineErrorKind::Internal, error.to_string()))?;
    Ok(EngineQueryResult { schema, batches })
}

fn execute_update_engine_on_bound(
    connection: &mut ManagedConnection,
    sql: &str,
    parameters: RecordBatch,
) -> EngineResult<Option<i64>> {
    validate_sql(sql)?;
    let mut statement = connection.new_statement().map_err(engine_error)?;
    statement.set_sql_query(sql).map_err(engine_error)?;
    statement.prepare().map_err(engine_error)?;
    statement.bind(parameters).map_err(engine_error)?;
    statement.execute_update().map_err(engine_error)
}

fn ingest_on(
    connection: &mut ManagedConnection,
    catalog: &str,
    schema: &str,
    table: &str,
    batches: Vec<RecordBatch>,
    mode: IngestMode,
) -> Result<Option<i64>> {
    if batches.is_empty() {
        bail!("DuckDB ADBC ingestion requires at least one RecordBatch");
    }
    if catalog.is_empty() || schema.is_empty() || table.is_empty() {
        bail!("DuckDB ADBC ingestion target names must not be empty");
    }
    let batch_schema = batches[0].schema();
    if batches.iter().any(|batch| batch.schema() != batch_schema) {
        bail!("DuckDB ADBC ingestion batches must have identical Arrow schemas");
    }

    let reader = RecordBatchIterator::new(batches.into_iter().map(Ok), batch_schema);
    let mut statement = connection
        .new_statement()
        .context("creating DuckDB ADBC ingestion statement")?;
    statement
        .set_option(OptionStatement::TargetCatalog, catalog.into())
        .context("setting DuckDB ADBC ingestion catalog")?;
    statement
        .set_option(OptionStatement::TargetDbSchema, schema.into())
        .context("setting DuckDB ADBC ingestion schema")?;
    statement
        .set_option(OptionStatement::TargetTable, table.into())
        .context("setting DuckDB ADBC ingestion table")?;
    statement
        .set_option(OptionStatement::IngestMode, mode.into())
        .context("setting DuckDB ADBC ingestion mode")?;
    statement
        .bind_stream(Box::new(reader))
        .context("binding DuckDB ADBC Arrow stream")?;
    statement
        .execute_update()
        .context("executing DuckDB ADBC Arrow ingestion")
}

fn ingest_engine_on(
    connection: &mut ManagedConnection,
    catalog: &str,
    schema: &str,
    table: &str,
    batches: Vec<RecordBatch>,
    mode: IngestMode,
) -> EngineResult<Option<i64>> {
    if batches.is_empty() {
        return Err(EngineError::new(
            EngineErrorKind::InvalidQuery,
            "DuckDB ADBC ingestion requires at least one RecordBatch",
        ));
    }
    if catalog.is_empty() || schema.is_empty() || table.is_empty() {
        return Err(EngineError::new(
            EngineErrorKind::InvalidQuery,
            "DuckDB ADBC ingestion target names must not be empty",
        ));
    }
    let batch_schema = batches[0].schema();
    if batches.iter().any(|batch| batch.schema() != batch_schema) {
        return Err(EngineError::new(
            EngineErrorKind::InvalidQuery,
            "DuckDB ADBC ingestion batches must have identical Arrow schemas",
        ));
    }

    let reader = RecordBatchIterator::new(batches.into_iter().map(Ok), batch_schema);
    ingest_reader_engine_on(
        connection,
        Some(catalog),
        schema,
        table,
        Box::new(reader),
        mode,
    )
}

fn ingest_reader_engine_on(
    connection: &mut ManagedConnection,
    catalog: Option<&str>,
    schema: &str,
    table: &str,
    reader: Box<dyn RecordBatchReader + Send>,
    mode: IngestMode,
) -> EngineResult<Option<i64>> {
    if catalog.is_some_and(str::is_empty) || schema.is_empty() || table.is_empty() {
        return Err(EngineError::new(
            EngineErrorKind::InvalidQuery,
            "DuckDB ADBC ingestion target names must not be empty",
        ));
    }
    let mut statement = connection.new_statement().map_err(engine_error)?;
    if let Some(catalog) = catalog {
        statement
            .set_option(OptionStatement::TargetCatalog, catalog.into())
            .map_err(engine_error)?;
    }
    statement
        .set_option(OptionStatement::TargetDbSchema, schema.into())
        .map_err(engine_error)?;
    statement
        .set_option(OptionStatement::TargetTable, table.into())
        .map_err(engine_error)?;
    statement
        .set_option(OptionStatement::IngestMode, mode.into())
        .map_err(engine_error)?;
    statement.bind_stream(reader).map_err(engine_error)?;
    statement.execute_update().map_err(engine_error)
}

fn validate_sql(sql: &str) -> EngineResult<()> {
    if sql.trim().is_empty() {
        return Err(EngineError::new(
            EngineErrorKind::InvalidQuery,
            "refusing to execute empty DuckDB ADBC SQL",
        ));
    }
    Ok(())
}

fn validate_read_query_sql(sql: &str) -> EngineResult<()> {
    validate_sql(sql)?;
    let statements = Parser::parse_sql(&DuckDbDialect {}, sql).map_err(|error| {
        EngineError::new(
            EngineErrorKind::InvalidQuery,
            format!("query API could not parse SQL: {error}"),
        )
    })?;
    let read_only = match statements.first() {
        Some(SqlStatement::Query(_)) => true,
        Some(SqlStatement::Explain { statement, .. }) => {
            matches!(statement.as_ref(), SqlStatement::Query(_))
        }
        _ => false,
    };
    if statements.len() != 1 || !read_only {
        return Err(EngineError::new(
            EngineErrorKind::InvalidQuery,
            "query API accepts exactly one read query",
        ));
    }
    Ok(())
}

fn anyhow_engine_error(error: anyhow::Error) -> EngineError {
    EngineError::new(EngineErrorKind::Internal, error.to_string())
}

fn engine_error(error: AdbcError) -> EngineError {
    let semantic_error = [
        (
            "PostgreSQL relation does not exist",
            EngineErrorKind::NotFound,
            "42P01",
        ),
        (
            "PostgreSQL type does not exist",
            EngineErrorKind::NotFound,
            "42704",
        ),
        (
            "PostgreSQL schema does not exist",
            EngineErrorKind::NotFound,
            "3F000",
        ),
        (
            "PostgreSQL role does not exist",
            EngineErrorKind::NotFound,
            "42704",
        ),
    ]
    .into_iter()
    .find(|(message, _, _)| error.message.contains(message));
    let kind = semantic_error.map_or_else(
        || match error.status {
            AdbcStatus::NotImplemented => EngineErrorKind::Unsupported,
            AdbcStatus::NotFound => EngineErrorKind::NotFound,
            AdbcStatus::AlreadyExists => EngineErrorKind::AlreadyExists,
            AdbcStatus::InvalidArguments | AdbcStatus::InvalidData => EngineErrorKind::InvalidQuery,
            AdbcStatus::InvalidState => EngineErrorKind::Busy,
            AdbcStatus::Integrity => EngineErrorKind::Constraint,
            AdbcStatus::IO => EngineErrorKind::Io,
            AdbcStatus::Cancelled => EngineErrorKind::Cancelled,
            AdbcStatus::Timeout => EngineErrorKind::Timeout,
            AdbcStatus::Unauthenticated | AdbcStatus::Unauthorized => EngineErrorKind::Unauthorized,
            AdbcStatus::Ok | AdbcStatus::Unknown | AdbcStatus::Internal => {
                EngineErrorKind::Internal
            }
        },
        |(_, kind, _)| kind,
    );
    let sqlstate_bytes: Vec<u8> = error.sqlstate.iter().map(|value| *value as u8).collect();
    let sqlstate = semantic_error
        .map(|(_, _, sqlstate)| sqlstate.to_owned())
        .or_else(|| {
            if sqlstate_bytes
                .iter()
                .all(|value| value.is_ascii_alphanumeric())
            {
                String::from_utf8(sqlstate_bytes).ok()
            } else {
                None
            }
        });
    let mut mapped = EngineError::new(kind, error.message);
    mapped.sqlstate = sqlstate;
    mapped.vendor_code = error.vendor_code;
    mapped
}

fn merge_adjacent_files_sql(
    catalog: &str,
    schema: &str,
    table: &str,
    max_compacted_files: Option<u64>,
    max_file_size: Option<u64>,
    min_file_size: Option<u64>,
) -> String {
    let mut arguments = vec![
        quote_literal(catalog),
        quote_literal(table),
        format!("schema => {}", quote_literal(schema)),
    ];
    for (name, value) in [
        ("max_compacted_files", max_compacted_files),
        ("max_file_size", max_file_size),
        ("min_file_size", min_file_size),
    ] {
        if let Some(value) = value {
            arguments.push(format!("{name} => {value}"));
        }
    }
    format!(
        "CALL ducklake_merge_adjacent_files({})",
        arguments.join(", ")
    )
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn verify_driver_digest(path: &std::path::Path) -> Result<()> {
    let actual = file_sha256(path)?;
    if actual != SUPPORTED_LIBDUCKDB_SHA256 {
        bail!(
            "DuckDB ADBC driver checksum mismatch: expected {SUPPORTED_LIBDUCKDB_SHA256}, got {actual}"
        );
    }
    Ok(())
}

fn validate_development_extension(path: &Path, expected_sha256: &str) -> Result<()> {
    if !path.is_absolute() {
        bail!("development DuckLake extension path must be absolute");
    }
    if path.to_str().is_none() {
        bail!("development DuckLake extension path must be valid UTF-8");
    }
    if expected_sha256.len() != 64
        || !expected_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("development DuckLake extension SHA-256 must be 64 lowercase hexadecimal characters");
    }
    let metadata = path.symlink_metadata().with_context(|| {
        format!(
            "reading development DuckLake extension metadata at {}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "development DuckLake extension must be a non-symlink regular file: {}",
            path.display()
        );
    }
    let actual = file_sha256(path)?;
    if actual != expected_sha256 {
        bail!(
            "development DuckLake extension checksum mismatch: expected {expected_sha256}, got {actual}"
        );
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("opening native artifact at {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let bytes = file
            .read(&mut buffer)
            .with_context(|| format!("hashing native artifact at {}", path.display()))?;
        if bytes == 0 {
            break;
        }
        digest.update(&buffer[..bytes]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn verify_runtime_version(connection: &mut ManagedConnection) -> Result<()> {
    let batches =
        query_on(connection, "SELECT version()").context("querying DuckDB ADBC runtime version")?;
    let version = batches
        .first()
        .and_then(|batch| batch.column(0).as_any().downcast_ref::<StringArray>())
        .filter(|array| !array.is_empty())
        .map(|array| array.value(0))
        .ok_or_else(|| anyhow!("DuckDB ADBC runtime returned no version string"))?;
    if version != SUPPORTED_DUCKDB_VERSION {
        bail!(
            "unsupported DuckDB ADBC runtime version {version:?}; expected {}",
            SUPPORTED_DUCKDB_VERSION
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn bootstrap_uses_official_ducklake_and_disables_inlining() {
        let config = DuckDbAdbcConfig {
            driver_path: Path::new("/opt/quackgis/libduckdb.so").to_path_buf(),
            database_uri: ":memory:".to_owned(),
            ducklake_uri: "ducklake:metadata.ducklake".to_owned(),
            catalog_name: "quack\"gis".to_owned(),
            data_path: "/data/it's-here".to_owned(),
            extension_policy: ExtensionPolicy::LoadOnly,
        };

        let sql = config.bootstrap_sql();
        assert!(sql.starts_with("LOAD ducklake;\nLOAD spatial;"));
        assert!(sql.contains("quackgis_st_geomfromewkt"));
        assert!(sql.contains("quackgis_st_geometry_type"));
        assert!(sql.contains("quackgis_pg_catalog.pg_namespace"));
        assert!(sql.contains("quackgis_pg_catalog.pg_database"));
        assert!(sql.contains("quackgis_pg_catalog.pg_type"));
        assert!(sql.contains("quackgis_pg_catalog.pg_range"));
        assert!(sql.contains("quackgis_pg_catalog.pg_collation"));
        assert!(sql.contains("quackgis_pg_catalog.pg_roles"));
        assert!(sql.contains("ducklake_default_data_inlining_row_limit = 0"));
        assert!(sql.contains("AS \"quack\"\"gis\""));
        assert!(sql.contains("DATA_PATH '/data/it''s-here'"));
        assert!(sql.contains("DATA_INLINING_ROW_LIMIT 0"));
    }

    #[test]
    fn local_install_policy_is_explicit() {
        let config = DuckDbAdbcConfig {
            driver_path: Path::new("/opt/quackgis/libduckdb.so").to_path_buf(),
            database_uri: ":memory:".to_owned(),
            ducklake_uri: "ducklake:metadata.ducklake".to_owned(),
            catalog_name: "quackgis".to_owned(),
            data_path: "/data".to_owned(),
            extension_policy: ExtensionPolicy::InstallAndLoad,
        };

        assert!(
            config
                .bootstrap_sql()
                .starts_with("INSTALL ducklake;\nINSTALL spatial;\nLOAD ducklake;\nLOAD spatial;")
        );
    }

    #[test]
    fn development_extension_requires_an_exact_regular_file_digest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let extension = temp.path().join("ducklake.duckdb_extension");
        std::fs::write(&extension, b"development ducklake").expect("extension fixture");
        let extension = extension.canonicalize().expect("absolute extension path");
        let digest = file_sha256(&extension).expect("extension digest");
        let config = DuckDbAdbcConfig {
            driver_path: Path::new("/opt/quackgis/libduckdb.so").to_path_buf(),
            database_uri: ":memory:".to_owned(),
            ducklake_uri: "ducklake:metadata.ducklake".to_owned(),
            catalog_name: "quackgis".to_owned(),
            data_path: "/data".to_owned(),
            extension_policy: ExtensionPolicy::DevelopmentDuckLake {
                path: extension.clone(),
                sha256: digest.clone(),
            },
        };

        validate_development_extension(&extension, &digest).expect("valid override");
        let sql = config.bootstrap_sql();
        assert!(sql.starts_with(&format!(
            "LOAD {};\nLOAD spatial;",
            quote_literal(&extension.display().to_string())
        )));
        assert!(!sql.contains("INSTALL ducklake"));
        assert!(config.allows_unsigned_extensions());

        let replacement = if digest.starts_with('0') { "1" } else { "0" };
        let wrong_digest = format!("{replacement}{}", &digest[1..]);
        assert!(validate_development_extension(&extension, &wrong_digest).is_err());
        assert!(validate_development_extension(&extension, "ABC").is_err());
        assert!(
            validate_development_extension(Path::new("relative.duckdb_extension"), &digest)
                .is_err()
        );

        #[cfg(unix)]
        {
            let link = temp.path().join("linked.duckdb_extension");
            std::os::unix::fs::symlink(&extension, &link).expect("extension symlink");
            assert!(validate_development_extension(&link, &digest).is_err());
        }
    }

    #[test]
    fn sql_quoting_handles_literals_and_identifiers() {
        assert_eq!(quote_literal("a'b\\c"), "'a''b\\c'");
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn query_api_rejects_mutations_and_batches() {
        validate_read_query_sql("SELECT 1").expect("single read query");
        validate_read_query_sql("SELECT ?").expect("DuckDB bound read query");
        validate_read_query_sql("EXPLAIN SELECT 1").expect("read-only explain");
        for sql in [
            "CREATE TABLE quackgis.main.bypass(id INTEGER)",
            "INSERT INTO quackgis.main.bypass VALUES (1)",
            "SELECT 1; SELECT 2",
        ] {
            assert!(validate_read_query_sql(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn default_resources_enable_bounded_local_spilling() {
        let resources = DuckDbResourceConfig::for_capacity(
            "/var/lib/quackgis/data",
            Some(16 * 1_073_741_824),
            8,
        );

        assert_eq!(resources.threads, 8);
        assert_eq!(resources.memory_limit_bytes, 10_307_921_510);
        assert_eq!(
            resources.temp_directory,
            Path::new("/var/lib/quackgis/data/.tmp")
        );
        assert_eq!(resources.max_temp_directory_bytes, 41_231_686_040);
        assert_eq!(
            resources.sql(),
            "SET threads=8; SET memory_limit='10307921510B'; SET temp_directory='/var/lib/quackgis/data/.tmp'; SET max_temp_directory_size='41231686040B';"
        );
    }

    #[test]
    fn resource_defaults_retain_safe_fallbacks() {
        let resources = DuckDbResourceConfig::for_capacity("data", None, 0);
        assert_eq!(resources.threads, 1);
        assert_eq!(resources.memory_limit_bytes, 1_073_741_824);
        assert_eq!(resources.max_temp_directory_bytes, 10_737_418_240);
        assert_eq!(cpu_quota_threads(150_000, 100_000), Some(2));
    }

    #[test]
    fn bbox_projection_is_enabled_only_for_the_explicit_layout_contract() {
        let target = arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("id", arrow_schema::DataType::Int32, false),
            arrow_schema::Field::new("geom_wkb", arrow_schema::DataType::Binary, true),
            arrow_schema::Field::new("_qg_minx", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_miny", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_maxx", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_maxy", arrow_schema::DataType::Float64, true),
        ]);
        let input = arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("id", arrow_schema::DataType::Int32, false),
            arrow_schema::Field::new("geom_wkb", arrow_schema::DataType::Binary, true),
        ]);
        let (columns, values) = bbox_publish_projection(&target, &input, "\"id\", \"geom_wkb\"")
            .expect("valid bbox projection");
        assert!(columns.contains("\"_qg_minx\""));
        assert!(values.contains("ST_XMin(ST_Extent(ST_GeomFromWKB(\"geom_wkb\")))"));

        let plain_target = arrow_schema::Schema::new(input.fields().to_vec());
        assert_eq!(
            bbox_publish_projection(&plain_target, &input, "\"id\", \"geom_wkb\""),
            Ok((
                "\"id\", \"geom_wkb\"".to_owned(),
                "\"id\", \"geom_wkb\"".to_owned()
            ))
        );
    }

    #[test]
    fn bbox_projection_rejects_layout_bypass_and_ambiguity() {
        let valid_fields = vec![
            arrow_schema::Field::new("id", arrow_schema::DataType::Int32, false),
            arrow_schema::Field::new("geom_wkb", arrow_schema::DataType::Binary, true),
            arrow_schema::Field::new("_qg_minx", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_miny", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_maxx", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_maxy", arrow_schema::DataType::Float64, true),
        ];
        let valid = arrow_schema::Schema::new(valid_fields.clone());

        let supplied_bbox = arrow_schema::Schema::new(valid_fields.clone());
        let error = bbox_publish_projection(&valid, &supplied_bbox, "input")
            .expect_err("caller-supplied bbox must fail");
        assert_eq!(error.kind, EngineErrorKind::Unsupported);

        let partial = arrow_schema::Schema::new(valid_fields[..3].to_vec());
        let geometry_input = arrow_schema::Schema::new(valid_fields[..2].to_vec());
        assert!(bbox_publish_projection(&partial, &geometry_input, "input").is_err());

        let mut wrong_type_fields = valid_fields.clone();
        wrong_type_fields[2] =
            arrow_schema::Field::new("_qg_minx", arrow_schema::DataType::Int64, true);
        assert!(
            bbox_publish_projection(
                &arrow_schema::Schema::new(wrong_type_fields),
                &geometry_input,
                "input"
            )
            .is_err()
        );

        let mut ambiguous_fields = valid_fields;
        ambiguous_fields.insert(
            2,
            arrow_schema::Field::new("geometry", arrow_schema::DataType::Binary, true),
        );
        assert!(
            bbox_publish_projection(
                &arrow_schema::Schema::new(ambiguous_fields),
                &geometry_input,
                "input"
            )
            .is_err()
        );
    }

    #[test]
    fn bbox_projection_writes_null_bounds_when_geometry_is_omitted() {
        let target = arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("id", arrow_schema::DataType::Int32, false),
            arrow_schema::Field::new("geom_wkb", arrow_schema::DataType::Binary, true),
            arrow_schema::Field::new("_qg_minx", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_miny", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_maxx", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_maxy", arrow_schema::DataType::Float64, true),
        ]);
        let input = arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "id",
            arrow_schema::DataType::Int32,
            false,
        )]);
        let (columns, values) =
            bbox_publish_projection(&target, &input, "\"id\"").expect("nullable geometry");
        assert!(columns.contains("\"_qg_minx\""));
        assert_eq!(values, "\"id\", NULL, NULL, NULL, NULL");
    }

    #[test]
    fn maintained_bbox_updates_refresh_only_stable_geometry_assignments() {
        let schema = arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("id", arrow_schema::DataType::Int32, false),
            arrow_schema::Field::new("name", arrow_schema::DataType::Utf8, true),
            arrow_schema::Field::new("geom_wkb", arrow_schema::DataType::Binary, true),
            arrow_schema::Field::new("_qg_minx", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_miny", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_maxx", arrow_schema::DataType::Float64, true),
            arrow_schema::Field::new("_qg_maxy", arrow_schema::DataType::Float64, true),
        ]);
        let assignments = |sql: &str| {
            let mut statements = Parser::parse_sql(&PostgreSqlDialect {}, sql).expect("UPDATE");
            match statements.remove(0) {
                SqlStatement::Update(update) => update.assignments,
                _ => panic!("expected UPDATE"),
            }
        };
        let layout = inspect_maintained_bbox_layout(&schema)
            .expect("valid layout")
            .expect("maintained layout");

        let mut ordinary = assignments("UPDATE points SET id = 2, name = 'safe'");
        rewrite_safe_bbox_update(&layout, &mut ordinary).expect("ordinary update");
        assert_eq!(ordinary.len(), 2);

        for sql in [
            "UPDATE points SET geom_wkb = $1::BLOB",
            "UPDATE points SET geom_wkb = NULL",
        ] {
            let mut rewritten = assignments(sql);
            rewrite_safe_bbox_update(&layout, &mut rewritten).expect("safe geometry update");
            assert_eq!(rewritten.len(), 5, "{sql}");
            let rendered = rewritten
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            for (column, accessor) in MAINTAINED_BBOX_COLUMNS
                .iter()
                .zip(["ST_XMin", "ST_YMin", "ST_XMax", "ST_YMax"])
            {
                assert!(rendered.contains(column), "{rendered}");
                assert!(rendered.contains(accessor), "{rendered}");
            }
        }

        for sql in [
            "UPDATE points SET _qg_minx = 0",
            "UPDATE points SET (name, _qg_maxy) = ('forged', 0)",
            "UPDATE points SET geom_wkb = other_geom",
            "UPDATE points SET geom_wkb = ST_AsWKB(ST_Point(1, 2))",
            "UPDATE points SET (geom_wkb, name) = ($1, 'unsafe')",
        ] {
            let mut unsafe_assignments = assignments(sql);
            assert!(
                rewrite_safe_bbox_update(&layout, &mut unsafe_assignments).is_err(),
                "{sql}"
            );
        }
    }

    #[test]
    fn bbox_query_injection_is_conservative_and_keeps_exact_recheck() {
        let parse = |sql: &str| {
            Parser::parse_sql(&PostgreSqlDialect {}, sql)
                .expect("query")
                .remove(0)
        };
        for sql in [
            "SELECT id FROM quackgis.main.points WHERE \
             ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_MakeEnvelope(-1, -2, $1, $2))",
            "SELECT p.id FROM public.points AS p WHERE p.id > 0 AND \
             ST_Intersects(ST_GeomFromText('POLYGON((0 0,2 0,2 2,0 2,0 0))'), \
                           ST_GeomFromWKB(p.geom_wkb))",
            "EXPLAIN SELECT id FROM points WHERE \
             ST_Intersects(ST_GeomFromWKB(points.geom_wkb), \
                           ST_GeomFromWKB($1::BLOB))",
        ] {
            let mut statement = parse(sql);
            let target = bbox_query_target(&statement)
                .unwrap_or_else(|| panic!("supported bbox query: {sql}"));
            inject_bbox_candidate(&mut statement, &target).expect("inject bbox candidate");
            let rendered = statement.to_string();
            for column in MAINTAINED_BBOX_COLUMNS {
                assert!(rendered.contains(column), "{rendered}");
            }
            assert_eq!(
                rendered
                    .to_ascii_lowercase()
                    .matches("st_intersects")
                    .count(),
                1,
                "exact predicate must remain once: {rendered}"
            );
            assert!(
                rendered.to_ascii_lowercase().contains("st_extent"),
                "probe bounds must stay planner-visible: {rendered}"
            );
        }

        for sql in [
            "SELECT id FROM points WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_MakeEnvelope(0, 0, 1, 1)) OR id = 1",
            "SELECT id FROM points WHERE NOT ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_MakeEnvelope(0, 0, 1, 1))",
            "SELECT id FROM points WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_Buffer(ST_Point(0, 0), 1))",
            "SELECT p.id FROM points p JOIN categories c ON c.id = p.id WHERE ST_Intersects(ST_GeomFromWKB(p.geom_wkb), ST_MakeEnvelope(0, 0, 1, 1))",
            "SELECT id FROM points WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_MakeEnvelope(0, 0, 1, 1)) AND ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_MakeEnvelope(2, 2, 3, 3))",
            "SELECT id FROM points WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromWKB($0::BLOB))",
        ] {
            assert!(
                bbox_query_target(&parse(sql)).is_none(),
                "unsupported query must not receive a candidate: {sql}"
            );
        }

        for oversized in [
            format!(
                "SELECT id FROM points WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_GeomFromText('{}'))",
                "0".repeat(MAX_BBOX_WKT_BYTES + 1)
            ),
            format!(
                "SELECT id FROM points WHERE ST_Intersects(ST_GeomFromWKB(geom_wkb), ST_MakeEnvelope(0, 0, {}, 1))",
                "9".repeat(MAX_BBOX_NUMERIC_LITERAL_BYTES + 1)
            ),
        ] {
            assert!(
                bbox_query_target(&parse(&oversized)).is_none(),
                "oversized inline probe must not be duplicated"
            );
        }
    }

    #[test]
    fn modified_driver_fails_before_claiming_data_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let driver = temp.path().join("libduckdb.so");
        std::fs::write(&driver, b"not duckdb").expect("fake driver");
        let data_path = temp.path().join("must-not-exist");
        let error = match DuckDbAdbcStorage::open(DuckDbAdbcConfig {
            driver_path: driver,
            database_uri: ":memory:".to_owned(),
            ducklake_uri: format!("ducklake:{}", temp.path().join("catalog").display()),
            catalog_name: "quackgis".to_owned(),
            data_path: data_path.display().to_string(),
            extension_policy: ExtensionPolicy::LoadOnly,
        }) {
            Ok(_) => panic!("modified native driver must fail closed"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("checksum mismatch"));
        assert!(!data_path.exists());
    }
}
