// SPDX-License-Identifier: Apache-2.0
//! Experimental DuckDB/ADBC storage-authority boundary.
//!
//! ADBC is the Arrow transport. DuckLake compatibility comes from executing
//! writes through DuckDB's official `ducklake` extension. This module is kept
//! behind `duckdb-adbc` until the persistence, concurrency, crash, and client
//! parity gates in `docs/DUCKDB_ADBC_EVALUATION.md` pass.

use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use adbc_core::error::{Error as AdbcError, Status as AdbcStatus};
use adbc_core::options::{
    AdbcVersion, IngestMode, OptionConnection, OptionDatabase, OptionStatement,
};
use adbc_core::sync::{Connection, Database, Driver, Optionable, Statement};
use adbc_driver_manager::{ManagedConnection, ManagedDatabase, ManagedDriver};
use anyhow::{Context, Result, anyhow, bail};
use arrow_array::{Array, Int64Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{ArrowError, SchemaRef};
use sha2::{Digest, Sha256};

use crate::engine_api::{
    EngineError, EngineErrorKind, EngineMaintenanceReport, EngineMaintenanceRequest,
    EngineQueryResult, EngineResult, EngineSnapshot, EngineStatementDescription,
    EngineStorageKernel, EngineTableRef, EngineTransactionState, IngestDisposition,
};
use crate::storage_authority::{StorageAuthority, claim_local_root};

const DUCKDB_ADBC_ENTRYPOINT: &[u8] = b"duckdb_adbc_init";
const SUPPORTED_DUCKDB_VERSION: &str = "v1.5.4";
const SUPPORTED_LIBDUCKDB_SHA256: &str =
    "d7f30ef2ef4b813edb94ce82906329cc689672624a4161617ea33431040ce174";

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
    /// DuckLake catalog URI, for example `ducklake:metadata.ducklake`,
    /// `ducklake:sqlite:metadata.sqlite`, or a PostgreSQL catalog URI.
    pub ducklake_uri: String,
    /// Name used for the attached DuckLake catalog.
    pub catalog_name: String,
    /// DuckLake data path. This may be a local path or object-store URI.
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
             SET ducklake_default_data_inlining_row_limit = 0;\n\
             ATTACH {} AS {} (DATA_PATH {}, DATA_INLINING_ROW_LIMIT 0);",
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
        config.validate()?;
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
        claim_local_root(
            std::path::Path::new(&config.data_path),
            StorageAuthority::DuckDbOfficialDuckLake,
        )
        .context("claiming local DuckDB official-DuckLake data root")?;
        let bootstrap_sql = config.bootstrap_sql();
        execute_update_on(&mut connection, &bootstrap_sql)
            .context("loading and attaching the official DuckLake extension")?;

        Ok(Self {
            _database: database,
            connection: Mutex::new(Some(connection)),
            catalog_name: config.catalog_name,
            transaction_state: Mutex::new(EngineTransactionState::Idle),
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
        self.with_connection(|connection| {
            connection
                .set_option(OptionConnection::AutoCommit, "false".into())
                .context("disabling DuckDB ADBC autocommit")
        })?;
        self.set_transaction_state(EngineTransactionState::Active)?;
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

    /// Ingest Arrow batches directly into the attached DuckLake catalog.
    ///
    /// DuckDB ADBC's target-catalog option routes the operation through the
    /// official DuckLake extension. Data inlining is disabled during bootstrap
    /// so DataFusion comparison readers only need Parquet/delete-file support.
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
        let mut connection = self.take_connection()?;
        if let Err(error) = connection.set_option(OptionConnection::AutoCommit, "false".into()) {
            self.return_connection(connection)?;
            return Err(error).context("disabling DuckDB ADBC autocommit");
        }
        self.set_transaction_state(EngineTransactionState::Active)?;

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
        *self
            .transaction_state
            .lock()
            .map_err(|_| anyhow!("DuckDB ADBC transaction-state mutex is poisoned"))? = state;
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
        let connection = slot.as_mut().ok_or_else(|| {
            EngineError::new(
                EngineErrorKind::Busy,
                "DuckDB ADBC connection is busy or quarantined after a native failure",
            )
        })?;
        operation(connection)
    }
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
        self.with_connection_engine(|connection| execute_update_engine_on(connection, sql))
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
            execute_update_engine_on_bound(connection, sql, parameters)
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
    let mut statement = connection.new_statement().map_err(engine_error)?;
    statement
        .set_option(OptionStatement::TargetCatalog, catalog.into())
        .map_err(engine_error)?;
    statement
        .set_option(OptionStatement::TargetDbSchema, schema.into())
        .map_err(engine_error)?;
    statement
        .set_option(OptionStatement::TargetTable, table.into())
        .map_err(engine_error)?;
    statement
        .set_option(OptionStatement::IngestMode, mode.into())
        .map_err(engine_error)?;
    statement
        .bind_stream(Box::new(reader))
        .map_err(engine_error)?;
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
