// SPDX-License-Identifier: Apache-2.0
//! DuckDB/ADBC storage kernel backed by official DuckLake.
//!
//! ADBC is the Arrow transport. DuckLake compatibility comes from executing
//! writes through DuckDB's official `ducklake` extension.

use std::io::Read;
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
    AssignmentTarget, Expr, ObjectName, ObjectNamePart, Statement as SqlStatement, TableFactor,
    TableObject, Value,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::engine_api::{
    EngineBatchStream, EngineCancellation, EngineError, EngineErrorKind, EngineMaintenanceReport,
    EngineMaintenanceRequest, EngineQueryResult, EngineQueryStream, EngineResourceSample,
    EngineResult, EngineSnapshot, EngineStatementDescription, EngineStorageKernel, EngineTableRef,
    EngineTransactionState, IngestDisposition,
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
const MAINTAINED_BBOX_COLUMNS: [&str; 4] = ["_qg_minx", "_qg_miny", "_qg_maxx", "_qg_maxy"];

struct MaintainedBboxLayout {
    geometry: String,
}

/// Whether DuckDB may download the DuckLake extension during initialization.
///
/// `LoadOnly` is the production-safe default: image construction must install
/// and pin the extension in advance. `InstallAndLoad` is intended only for local
/// evaluation where network access and extension provenance are explicit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExtensionPolicy {
    #[default]
    LoadOnly,
    InstallAndLoad,
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
        Ok(())
    }

    fn bootstrap_sql(&self) -> String {
        let extension_sql = match self.extension_policy {
            ExtensionPolicy::LoadOnly => "LOAD ducklake;\nLOAD spatial;",
            ExtensionPolicy::InstallAndLoad => {
                "INSTALL ducklake;\nINSTALL spatial;\nLOAD ducklake;\nLOAD spatial;"
            }
        };
        format!(
            "{extension_sql}\n\
             {}\n\
             SET ducklake_default_data_inlining_row_limit = 0;\n\
             ATTACH {} AS {} (DATA_PATH {}, DATA_INLINING_ROW_LIMIT 0);",
            crate::spatial_compat::DUCKDB_COMPATIBILITY_MACROS,
            quote_literal(&self.ducklake_uri),
            quote_identifier(&self.catalog_name),
            quote_literal(&self.data_path),
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
    transaction_state: Mutex<EngineTransactionState>,
    lifecycle: Arc<RuntimeLifecycle>,
}

pub struct DuckDbIngestOperation {
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
}

impl EngineCancellation for DuckDbCancelHandle {
    fn cancel(&self) -> EngineResult<()> {
        self.requested.store(true, Ordering::Release);
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
        let database = driver
            .new_database_with_opts([(OptionDatabase::Uri, config.database_uri.clone().into())])
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

        Ok(Self {
            _database: database,
            connection: Mutex::new(Some(connection)),
            catalog_name: config.catalog_name,
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
            transaction_state: Mutex::new(EngineTransactionState::Idle),
            lifecycle: Arc::clone(&self.lifecycle),
        })
    }

    pub fn lifecycle(&self) -> Arc<RuntimeLifecycle> {
        Arc::clone(&self.lifecycle)
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
        if let Err(error) = connection.commit() {
            self.set_transaction_state(EngineTransactionState::Quarantined)?;
            return Err(error).context(
                "DuckDB ADBC commit failed; transaction outcome is indeterminate and the connection was quarantined",
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

    pub fn query_stream(self: &Arc<Self>, sql: &str) -> EngineResult<EngineQueryStream> {
        self.query_bound_stream(sql, None)
    }

    pub fn query_bound_stream(
        self: &Arc<Self>,
        sql: &str,
        parameters: Option<RecordBatch>,
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
        validate_sql(sql)?;
        let mut connection = self.take_connection_engine()?;
        let cancellation = Arc::new(DuckDbCancelHandle {
            connection: Mutex::new(connection.clone()),
            requested: AtomicBool::new(false),
        });
        let setup: EngineResult<_> = (|| {
            let mut statement = connection.new_statement().map_err(engine_error)?;
            statement.set_sql_query(sql).map_err(engine_error)?;
            if let Some(parameters) = parameters {
                statement.prepare().map_err(engine_error)?;
                statement.bind(parameters).map_err(engine_error)?;
            }
            let reader = statement.execute().map_err(engine_error)?;
            let schema = reader.schema();
            Ok((statement, reader, schema))
        })();
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
        });
        Ok(DuckDbIngestOperation {
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
                if let Err(error) = connection.commit() {
                    // A failed commit has an indeterminate durable outcome. Do
                    // not reuse this native connection or imply rollback.
                    self.set_transaction_state(EngineTransactionState::Quarantined)?;
                    return Err(error).context(
                        "DuckDB ADBC commit failed; transaction outcome is indeterminate and the connection was quarantined",
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

fn query_on(connection: &mut ManagedConnection, sql: &str) -> Result<Vec<RecordBatch>> {
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
    let mut statement = connection.new_statement().map_err(engine_error)?;
    statement.set_sql_query(sql).map_err(engine_error)?;
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
    validate_sql(sql)?;
    let mut statement = connection.new_statement().map_err(engine_error)?;
    statement.set_sql_query(sql).map_err(engine_error)?;
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

fn engine_error(error: AdbcError) -> EngineError {
    let kind = match error.status {
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
        AdbcStatus::Ok | AdbcStatus::Unknown | AdbcStatus::Internal => EngineErrorKind::Internal,
    };
    let sqlstate_bytes: Vec<u8> = error.sqlstate.iter().map(|value| *value as u8).collect();
    let sqlstate = if sqlstate_bytes
        .iter()
        .all(|value| value.is_ascii_alphanumeric())
    {
        String::from_utf8(sqlstate_bytes).ok()
    } else {
        None
    };
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
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("opening DuckDB ADBC driver at {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let bytes = file
            .read(&mut buffer)
            .with_context(|| format!("hashing DuckDB ADBC driver at {}", path.display()))?;
        if bytes == 0 {
            break;
        }
        digest.update(&buffer[..bytes]);
    }
    let actual = format!("{:x}", digest.finalize());
    if actual != SUPPORTED_LIBDUCKDB_SHA256 {
        bail!(
            "DuckDB ADBC driver checksum mismatch: expected {SUPPORTED_LIBDUCKDB_SHA256}, got {actual}"
        );
    }
    Ok(())
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
    fn sql_quoting_handles_literals_and_identifiers() {
        assert_eq!(quote_literal("a'b\\c"), "'a''b\\c'");
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
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
