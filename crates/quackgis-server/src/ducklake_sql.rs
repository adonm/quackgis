// SPDX-License-Identifier: Apache-2.0
//! QuackGIS SQL-to-DuckLake routing.
//!
//! datafusion-ducklake's writer API is the validated storage path. This hook
//! maps the SQL clients actually send (CTAS / INSERT) onto that writer API for
//! the `quackgis.main.<table>` catalog path.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use datafusion::arrow::array::{
    Array, ArrayRef, BinaryArray, BinaryViewArray, BooleanArray, Float64Array, Int32Array,
    Int64Array, NullArray, StringArray, StringViewArray, UInt64Array, new_null_array,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::{DFSchema, DFSchemaRef, ParamValues};
use datafusion::datasource::MemTable;
use datafusion::logical_expr::{EmptyRelation, LogicalPlan};
use datafusion::prelude::SessionContext;
use datafusion::sql::sqlparser::ast::{
    AlterTable, AlterTableOperation, AssignmentTarget, ColumnDef, CopyLegacyOption, CopyOption,
    CopySource, CopyTarget, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArguments, Ident,
    ObjectName, ObjectNamePart, Query, SelectItem, SetExpr, Statement, TableFactor,
    TableFunctionArgs, TableVersion, UnaryOperator, Value,
};
use datafusion_ducklake::{
    DeleteFileMutation, DuckLakeCatalog, DuckLakeTable, DuckLakeTableFile, DuckLakeTableWriter,
    MetadataWriter, TableMutation, TableWriteSession, WriteMode,
};
use datafusion_postgres::arrow_pg::datatypes::{arrow_schema_to_pg_fields, encode_recordbatch};
use datafusion_postgres::hooks::{HookClient, QueryHook};
use datafusion_postgres::pgwire::api::portal::Format;
use datafusion_postgres::pgwire::api::results::{CopyResponse, QueryResponse, Response, Tag};
use datafusion_postgres::pgwire::api::{ClientInfo, PgWireConnectionState};
use datafusion_postgres::pgwire::error::{PgWireError, PgWireResult};
use datafusion_postgres::pgwire::messages::PgWireBackendMessage;
use datafusion_postgres::pgwire::messages::copy::{CopyData, CopyDone, CopyFail};
use datafusion_postgres::pgwire::messages::response::TransactionStatus;
use futures::{Sink, SinkExt};
use tokio::sync::Mutex;

use crate::auth::{AccessRole, AuthConfig};
use crate::catalog_compat::SYNTHETIC_ROWID_COLUMN;
use crate::context::{DUCKLAKE_CATALOG, StoragePaths};

pub(crate) mod layout;
mod names;
mod params;
mod pruning;
mod rewrites;

use names::{
    delete_target_parts, ducklake_table_ref, insert_source_is_values, insert_target_parts,
    object_name_last, public_table_ref, quote_ident, table_name_parts, update_target_parts,
};
use params::inline_params_if_needed;
use rewrites::{
    rewrite_mojibake_string_literals, rewrite_pg_escape_bytea_literals,
    rewrite_st_geomfromwkb_zero_srid_literals,
};

static TRANSACTION_COUNTER: AtomicU64 = AtomicU64::new(1);
static QUERY_COUNTER: AtomicU64 = AtomicU64::new(1);
static WRITE_DENIED_COUNTER: AtomicU64 = AtomicU64::new(0);
static CATALOG_REFRESH_COUNTER: AtomicU64 = AtomicU64::new(0);
static SHARED_CATALOG_READ_REFRESH_COUNTER: AtomicU64 = AtomicU64::new(0);
static SHARED_CATALOG_STRONG_REFRESH_COUNTER: AtomicU64 = AtomicU64::new(0);
static NATIVE_DELETE_MUTATION_COUNTER: AtomicU64 = AtomicU64::new(0);
static NATIVE_UPDATE_MUTATION_COUNTER: AtomicU64 = AtomicU64::new(0);
static NATIVE_COMPACT_MUTATION_COUNTER: AtomicU64 = AtomicU64::new(0);
static NATIVE_MUTATION_ABORT_COUNTER: AtomicU64 = AtomicU64::new(0);
static COMPACTION_COUNTER: AtomicU64 = AtomicU64::new(0);
static NATIVE_MUTATION_FAILPOINT: OnceLock<StdMutex<Option<NativeMutationFailpoint>>> =
    OnceLock::new();
const DEFAULT_DUCKLAKE_ROW_GROUP_ROWS: usize = 512;
const DUCKLAKE_ROW_GROUP_ROWS_ENV: &str = "QUACKGIS_DUCKLAKE_ROW_GROUP_ROWS";
const DEFAULT_SHARED_CATALOG_REFRESH_MS: u64 = 1_000;
const SHARED_CATALOG_REFRESH_MS_ENV: &str = "QUACKGIS_SHARED_CATALOG_REFRESH_MS";
const DEFAULT_SELECTIVE_READ_TARGET_PARTITIONS: usize = 1;
const SELECTIVE_READ_TARGET_PARTITIONS_ENV: &str = "QUACKGIS_SELECTIVE_READ_TARGET_PARTITIONS";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeMutationKind {
    Delete,
    Update,
    Compact,
}

impl NativeMutationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Update => "update",
            Self::Compact => "compact",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            value if value.eq_ignore_ascii_case("delete") => Ok(Self::Delete),
            value if value.eq_ignore_ascii_case("update") => Ok(Self::Update),
            value if value.eq_ignore_ascii_case("compact") => Ok(Self::Compact),
            _ => Err(anyhow!(
                "unsupported native mutation failpoint operation {value:?}; expected delete, update, or compact"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeMutationStage {
    BeforeCommit,
}

impl NativeMutationStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::BeforeCommit => "before_commit",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            value if value.eq_ignore_ascii_case("before_commit") => Ok(Self::BeforeCommit),
            _ => Err(anyhow!(
                "unsupported native mutation failpoint stage {value:?}; expected before_commit"
            )),
        }
    }
}

#[derive(Debug, Clone)]
struct NativeMutationFailpoint {
    kind: NativeMutationKind,
    stage: NativeMutationStage,
    schema: Option<String>,
    table: Option<String>,
}

impl NativeMutationFailpoint {
    fn parse(spec: &str) -> Result<Self> {
        let parts = spec.split(':').collect::<Vec<_>>();
        if !(2..=3).contains(&parts.len()) {
            return Err(anyhow!(
                "native mutation failpoint must be operation:stage[:schema.table], got {spec:?}"
            ));
        }
        let kind = NativeMutationKind::parse(parts[0].trim())?;
        let stage = NativeMutationStage::parse(parts[1].trim())?;
        let (schema, table) = if let Some(target) = parts.get(2) {
            parse_failpoint_target(target.trim())?
        } else {
            (None, None)
        };
        Ok(Self {
            kind,
            stage,
            schema,
            table,
        })
    }

    fn matches(
        &self,
        kind: NativeMutationKind,
        stage: NativeMutationStage,
        schema: &str,
        table: &str,
    ) -> bool {
        self.kind == kind
            && self.stage == stage
            && self
                .schema
                .as_deref()
                .is_none_or(|expected| expected.eq_ignore_ascii_case(schema))
            && self
                .table
                .as_deref()
                .is_none_or(|expected| expected.eq_ignore_ascii_case(table))
    }
}

/// Install a one-shot native mutation failpoint for tests and local failure drills.
///
/// The syntax is `operation:stage[:schema.table]`, for example
/// `delete:before_commit:main.points`. The failpoint is process-local, triggers
/// only once, and is not exposed through pgwire.
#[doc(hidden)]
pub fn set_native_mutation_failpoint_for_tests(spec: Option<&str>) -> Result<()> {
    let failpoint = spec.map(NativeMutationFailpoint::parse).transpose()?;
    let mut guard = native_mutation_failpoint()
        .lock()
        .map_err(|_| anyhow!("native mutation failpoint lock poisoned"))?;
    *guard = failpoint;
    Ok(())
}

fn native_mutation_failpoint() -> &'static StdMutex<Option<NativeMutationFailpoint>> {
    NATIVE_MUTATION_FAILPOINT.get_or_init(|| StdMutex::new(None))
}

fn parse_failpoint_target(target: &str) -> Result<(Option<String>, Option<String>)> {
    if target.is_empty() || target == "*" {
        return Ok((None, None));
    }
    let parts = target.split('.').collect::<Vec<_>>();
    match parts.as_slice() {
        [table] if !table.is_empty() => Ok((None, Some((*table).to_string()))),
        [schema, table] if !schema.is_empty() && !table.is_empty() => {
            Ok((Some((*schema).to_string()), Some((*table).to_string())))
        }
        _ => Err(anyhow!(
            "native mutation failpoint target must be table or schema.table, got {target:?}"
        )),
    }
}

fn maybe_fail_native_mutation(
    kind: NativeMutationKind,
    stage: NativeMutationStage,
    schema: &str,
    table: &str,
) -> PgWireResult<()> {
    let failpoint = {
        let mut guard = native_mutation_failpoint()
            .lock()
            .map_err(|_| user_error(anyhow!("native mutation failpoint lock poisoned")))?;
        if guard
            .as_ref()
            .is_some_and(|fp| fp.matches(kind, stage, schema, table))
        {
            NATIVE_MUTATION_ABORT_COUNTER.fetch_add(1, Ordering::Relaxed);
            guard.take()
        } else {
            None
        }
    };

    if let Some(failpoint) = failpoint {
        return Err(user_error(anyhow!(
            "debug native mutation failpoint triggered: operation={} stage={} target={schema}.{table}",
            failpoint.kind.as_str(),
            failpoint.stage.as_str()
        )));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct DuckLakeSqlHook {
    paths: StoragePaths,
    auth: AuthConfig,
    shared_catalog_refresh: Arc<SharedCatalogRefreshState>,
}

#[derive(Debug)]
struct SharedCatalogRefreshState {
    min_interval: Duration,
    last_refresh: Mutex<Option<Instant>>,
}

impl SharedCatalogRefreshState {
    fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_refresh: Mutex::new(None),
        }
    }

    fn from_env() -> Self {
        Self::new(shared_catalog_refresh_interval())
    }

    fn refresh_is_recent(&self, now: Instant, last_refresh: Option<Instant>) -> bool {
        last_refresh.is_some_and(|last| {
            self.min_interval > Duration::ZERO
                && now.saturating_duration_since(last) < self.min_interval
        })
    }
}

#[derive(Debug, Default)]
struct ClientTransactionState {
    inner: Mutex<TransactionState>,
}

#[derive(Debug, Default)]
struct CopyInSessionState {
    inner: Mutex<Option<CopyInRequest>>,
}

#[derive(Debug)]
struct CopyInRequest {
    schema: String,
    table: String,
    columns: Vec<String>,
    options: CopyTextOptions,
    data: Vec<u8>,
}

#[derive(Debug, Clone)]
struct CopyTextOptions {
    delimiter: u8,
    null: Vec<u8>,
    header: bool,
}

impl Default for CopyTextOptions {
    fn default() -> Self {
        Self {
            delimiter: b'\t',
            null: b"\\N".to_vec(),
            header: false,
        }
    }
}

#[derive(Clone)]
pub struct DuckLakeCopyHandler {
    sql: DuckLakeSqlHook,
    session_context: Arc<SessionContext>,
}

impl DuckLakeCopyHandler {
    pub fn new(paths: StoragePaths, session_context: Arc<SessionContext>) -> Self {
        Self::new_with_auth(paths, session_context, AuthConfig::trust())
    }

    pub fn new_with_auth(
        paths: StoragePaths,
        session_context: Arc<SessionContext>,
        auth: AuthConfig,
    ) -> Self {
        Self {
            sql: DuckLakeSqlHook::new_with_auth(paths, auth),
            session_context,
        }
    }
}

#[derive(Debug, Default)]
enum TransactionState {
    #[default]
    Idle,
    Active(ActiveTransaction),
}

#[derive(Debug)]
struct ActiveTransaction {
    id: String,
    staged_tables: HashMap<TableKey, StagedTable>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct TableKey {
    schema: String,
    table: String,
}

#[derive(Debug)]
struct StagedTable {
    temp_table: String,
    batches: Vec<RecordBatch>,
    writer_session: Option<TableWriteSession>,
}

struct NativeRowPlan {
    snapshot_id: i64,
    table_id: i64,
    files: Vec<DuckLakeTableFile>,
    positions_by_file: HashMap<i64, HashSet<i64>>,
    affected_count: usize,
    rowid_context: SessionContext,
    catalog_name: String,
}

struct SnapshotQueryRewrite {
    sql: String,
    snapshot_id: i64,
    schema: String,
    table: String,
}

impl DuckLakeSqlHook {
    pub fn new(paths: StoragePaths) -> Self {
        Self::new_with_auth(paths, AuthConfig::trust())
    }

    pub fn new_with_auth(paths: StoragePaths, auth: AuthConfig) -> Self {
        Self {
            paths,
            auth,
            shared_catalog_refresh: Arc::new(SharedCatalogRefreshState::from_env()),
        }
    }

    fn ensure_write_allowed<C>(&self, client: &C, statement: &Statement) -> PgWireResult<()>
    where
        C: ClientInfo + Send + Sync + ?Sized,
    {
        match self.auth.role_for_client(client) {
            AccessRole::ReadWrite => Ok(()),
            AccessRole::ReadOnly => {
                let denied_total = WRITE_DENIED_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
                log::warn!(
                    "quackgis_write_denied user={} statement_kind={} denied_total={denied_total}",
                    client_user(client),
                    statement_kind(statement)
                );
                Err(user_error(anyhow!(
                    "read-only QuackGIS role cannot execute write or maintenance statements"
                )))
            }
        }
    }

    async fn snapshot_query_context(
        &self,
        statement: &Statement,
        session_context: &SessionContext,
    ) -> PgWireResult<Option<(SessionContext, String)>> {
        let catalog_name = format!(
            "__quackgis_snapshot_{}",
            QUERY_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let Some(rewrite) = snapshot_query_rewrite(statement, &catalog_name)? else {
            return Ok(None);
        };
        let provider = self
            .paths
            .metadata_provider()
            .await
            .map_err(storage_api_error)?;
        let schema_meta = provider
            .get_schema_by_name(&rewrite.schema, rewrite.snapshot_id)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?
            .ok_or_else(|| {
                user_error(anyhow!(
                    "schema {} was not visible at DuckLake snapshot {}",
                    rewrite.schema,
                    rewrite.snapshot_id
                ))
            })?;
        provider
            .get_table_by_name(schema_meta.schema_id, &rewrite.table, rewrite.snapshot_id)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?
            .ok_or_else(|| {
                user_error(anyhow!(
                    "table {}.{} was not visible at DuckLake snapshot {}",
                    rewrite.schema,
                    rewrite.table,
                    rewrite.snapshot_id
                ))
            })?;
        let ducklake = DuckLakeCatalog::with_snapshot(provider, rewrite.snapshot_id)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let snapshot_context = SessionContext::new_with_state(session_context.state());
        snapshot_context.register_catalog(&catalog_name, Arc::new(ducklake));
        Ok(Some((snapshot_context, rewrite.sql)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuckLakeSqlMetrics {
    pub queries_started_total: u64,
    pub transaction_ids_allocated_total: u64,
    pub writes_denied_total: u64,
    pub catalog_refresh_total: u64,
    pub shared_catalog_read_refresh_total: u64,
    pub shared_catalog_strong_refresh_total: u64,
    pub native_delete_mutations_total: u64,
    pub native_update_mutations_total: u64,
    pub native_compact_mutations_total: u64,
    pub native_mutation_aborts_total: u64,
    pub compactions_total: u64,
}

pub fn metrics_snapshot() -> DuckLakeSqlMetrics {
    DuckLakeSqlMetrics {
        queries_started_total: QUERY_COUNTER.load(Ordering::Relaxed).saturating_sub(1),
        transaction_ids_allocated_total: TRANSACTION_COUNTER
            .load(Ordering::Relaxed)
            .saturating_sub(1),
        writes_denied_total: WRITE_DENIED_COUNTER.load(Ordering::Relaxed),
        catalog_refresh_total: CATALOG_REFRESH_COUNTER.load(Ordering::Relaxed),
        shared_catalog_read_refresh_total: SHARED_CATALOG_READ_REFRESH_COUNTER
            .load(Ordering::Relaxed),
        shared_catalog_strong_refresh_total: SHARED_CATALOG_STRONG_REFRESH_COUNTER
            .load(Ordering::Relaxed),
        native_delete_mutations_total: NATIVE_DELETE_MUTATION_COUNTER.load(Ordering::Relaxed),
        native_update_mutations_total: NATIVE_UPDATE_MUTATION_COUNTER.load(Ordering::Relaxed),
        native_compact_mutations_total: NATIVE_COMPACT_MUTATION_COUNTER.load(Ordering::Relaxed),
        native_mutation_aborts_total: NATIVE_MUTATION_ABORT_COUNTER.load(Ordering::Relaxed),
        compactions_total: COMPACTION_COUNTER.load(Ordering::Relaxed),
    }
}

fn shared_catalog_refresh_interval() -> Duration {
    match std::env::var(SHARED_CATALOG_REFRESH_MS_ENV) {
        Ok(value) => match value.trim().parse::<u64>() {
            Ok(ms) => Duration::from_millis(ms),
            Err(_) => Duration::from_millis(DEFAULT_SHARED_CATALOG_REFRESH_MS),
        },
        Err(std::env::VarError::NotPresent) => {
            Duration::from_millis(DEFAULT_SHARED_CATALOG_REFRESH_MS)
        }
        Err(_) => Duration::from_millis(DEFAULT_SHARED_CATALOG_REFRESH_MS),
    }
}

#[async_trait]
impl datafusion_postgres::pgwire::api::copy::CopyHandler for DuckLakeCopyHandler {
    async fn on_copy_data<C>(&self, client: &mut C, copy_data: CopyData) -> PgWireResult<()>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: std::fmt::Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        self.sql
            .append_copy_data(client, copy_data.data.as_ref())
            .await
    }

    async fn on_copy_done<C>(&self, client: &mut C, _done: CopyDone) -> PgWireResult<()>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: std::fmt::Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let rows = self
            .sql
            .finish_copy_in(&self.session_context, client)
            .await?;
        client
            .send(PgWireBackendMessage::CommandComplete(
                Tag::new("COPY").with_rows(rows).into(),
            ))
            .await?;
        // pgwire 0.40 keeps extended COPY connections in CopyInProgress after
        // CopyDone and expects a later Sync. Move to AwaitingSync here so that
        // Sync is consumed and ReadyForQuery is sent; simple COPY is overwritten
        // back to ReadyForQuery by pgwire's simple-COPY branch.
        client.set_state(PgWireConnectionState::AwaitingSync);
        Ok(())
    }

    async fn on_copy_fail<C>(&self, client: &mut C, fail: CopyFail) -> PgWireError
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: std::fmt::Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        self.sql.abort_copy_in(client).await;
        user_error(anyhow!(
            "COPY FROM STDIN aborted by client: {}",
            fail.message
        ))
    }
}

#[async_trait]
impl QueryHook for DuckLakeSqlHook {
    async fn handle_simple_query(
        &self,
        statement: &datafusion::sql::sqlparser::ast::Statement,
        session_context: &SessionContext,
        client: &mut dyn HookClient,
    ) -> Option<PgWireResult<Response>> {
        if let Err(err) = self
            .refresh_shared_catalog(statement, session_context)
            .await
        {
            return Some(Err(err));
        }
        log_query_start(client, "simple", statement);
        if statement_requires_write(statement)
            && let Err(err) = self.ensure_write_allowed(client, statement)
        {
            return Some(Err(err));
        }
        match self
            .snapshot_query_context(statement, session_context)
            .await
        {
            Ok(Some((snapshot_context, rewritten_query))) => {
                return Some(
                    collect_query_batches(&snapshot_context, &rewritten_query)
                        .await
                        .and_then(|batches| {
                            query_response_from_batches_with_format(batches, Format::UnifiedText)
                        })
                        .map(Response::Query),
                );
            }
            Ok(None) => {}
            Err(err) => return Some(Err(err)),
        }
        if let Some(rewritten_query) =
            pruning::rewrite_spatial_pruning_query(statement, session_context).await
        {
            return Some(
                collect_selective_read_batches(session_context, &rewritten_query)
                    .await
                    .and_then(|batches| {
                        query_response_from_batches_with_format(batches, Format::UnifiedText)
                    })
                    .map(Response::Query),
            );
        }
        match statement {
            datafusion::sql::sqlparser::ast::Statement::StartTransaction { .. } => {
                Some(self.handle_begin(client).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Commit { .. } => {
                Some(self.handle_commit(session_context, client).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Rollback { .. } => {
                Some(self.handle_rollback(session_context, client).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Deallocate { .. } => {
                Some(Ok(Response::Execution(Tag::new("DEALLOCATE"))))
            }
            datafusion::sql::sqlparser::ast::Statement::CreateTable(ct)
                if table_name_parts(&ct.name).is_some() =>
            {
                Some(self.handle_create_table(ct, session_context, client).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Copy { .. }
                if copy_statement_parts(statement).is_some() =>
            {
                Some(
                    self.handle_copy_from_stdin(statement, session_context, client)
                        .await,
                )
            }
            datafusion::sql::sqlparser::ast::Statement::Call(function)
                if is_compact_call(function) =>
            {
                Some(
                    self.handle_compact_call(function, session_context, client)
                        .await,
                )
            }
            datafusion::sql::sqlparser::ast::Statement::Insert(insert)
                if insert.source.is_some() && insert_target_parts(&insert.table).is_some() =>
            {
                Some(
                    self.handle_insert(insert, session_context, Format::UnifiedText, None, client)
                        .await,
                )
            }
            datafusion::sql::sqlparser::ast::Statement::AlterTable(alter)
                if table_name_parts(&alter.name).is_some() =>
            {
                Some(
                    self.handle_alter_table(alter, session_context, client)
                        .await,
                )
            }
            datafusion::sql::sqlparser::ast::Statement::Delete(delete)
                if delete_target_parts(delete).is_some() =>
            {
                Some(
                    self.handle_delete(delete, session_context, Format::UnifiedText, None, client)
                        .await,
                )
            }
            datafusion::sql::sqlparser::ast::Statement::Update(update)
                if update_target_parts(&update.table).is_some() =>
            {
                Some(
                    self.handle_update(update, Format::UnifiedText, None, session_context, client)
                        .await,
                )
            }
            _ => None,
        }
    }

    async fn handle_extended_parse_query(
        &self,
        statement: &datafusion::sql::sqlparser::ast::Statement,
        session_context: &SessionContext,
        client: &(dyn datafusion_postgres::pgwire::api::ClientInfo + Send + Sync),
    ) -> Option<PgWireResult<LogicalPlan>> {
        if let Err(err) = self
            .refresh_shared_catalog(statement, session_context)
            .await
        {
            return Some(Err(err));
        }
        if statement_requires_write(statement)
            && let Err(err) = self.ensure_write_allowed(client, statement)
        {
            return Some(Err(err));
        }
        match self
            .snapshot_query_context(statement, session_context)
            .await
        {
            Ok(Some((snapshot_context, rewritten_query))) => {
                return Some(
                    snapshot_context
                        .sql(&rewritten_query)
                        .await
                        .map_err(|e| PgWireError::ApiError(Box::new(e)))
                        .and_then(|df| {
                            df.into_optimized_plan()
                                .map_err(|e| PgWireError::ApiError(Box::new(e)))
                        }),
                );
            }
            Ok(None) => {}
            Err(err) => return Some(Err(err)),
        }
        if let Some(plan) = self
            .returning_logical_plan(statement, session_context)
            .await
        {
            return Some(plan);
        }
        if matches!(
            statement,
            datafusion::sql::sqlparser::ast::Statement::StartTransaction { .. }
                | datafusion::sql::sqlparser::ast::Statement::Commit { .. }
                | datafusion::sql::sqlparser::ast::Statement::Rollback { .. }
                | datafusion::sql::sqlparser::ast::Statement::Deallocate { .. }
        ) {
            return Some(Ok(empty_logical_plan()));
        }
        if matches!(statement, datafusion::sql::sqlparser::ast::Statement::Call(function) if is_compact_call(function))
        {
            return Some(Ok(empty_logical_plan()));
        }
        if ducklake_statement_parts(statement).is_some() {
            return Some(Ok(empty_logical_plan()));
        }
        if let Some(rewritten_query) =
            pruning::rewrite_spatial_pruning_query(statement, session_context).await
        {
            return Some(
                session_context
                    .sql(&rewritten_query)
                    .await
                    .map_err(|e| PgWireError::ApiError(Box::new(e)))
                    .and_then(|df| {
                        df.into_optimized_plan()
                            .map_err(|e| PgWireError::ApiError(Box::new(e)))
                    }),
            );
        }
        None
    }

    async fn handle_extended_query(
        &self,
        statement: &datafusion::sql::sqlparser::ast::Statement,
        _logical_plan: &LogicalPlan,
        _params: &ParamValues,
        session_context: &SessionContext,
        client: &mut dyn HookClient,
    ) -> Option<PgWireResult<Response>> {
        if let Err(err) = self
            .refresh_shared_catalog(statement, session_context)
            .await
        {
            return Some(Err(err));
        }
        log_query_start(client, "extended", statement);
        if statement_requires_write(statement)
            && let Err(err) = self.ensure_write_allowed(client, statement)
        {
            return Some(Err(err));
        }
        match self
            .snapshot_query_context(statement, session_context)
            .await
        {
            Ok(Some((snapshot_context, rewritten_query))) => {
                return Some(
                    collect_query_batches(&snapshot_context, &rewritten_query)
                        .await
                        .and_then(|batches| {
                            query_response_from_batches_with_format(batches, Format::UnifiedBinary)
                        })
                        .map(Response::Query),
                );
            }
            Ok(None) => {}
            Err(err) => return Some(Err(err)),
        }
        if param_values_empty(_params)
            && let Some(rewritten_query) =
                pruning::rewrite_spatial_pruning_query(statement, session_context).await
        {
            return Some(
                collect_selective_read_batches(session_context, &rewritten_query)
                    .await
                    .and_then(|batches| {
                        query_response_from_batches_with_format(batches, Format::UnifiedBinary)
                    })
                    .map(Response::Query),
            );
        }
        // Route extended-protocol CTAS/INSERT too; clients differ in whether
        // they send DDL via simple or extended flow.
        match statement {
            datafusion::sql::sqlparser::ast::Statement::StartTransaction { .. } => {
                Some(self.handle_begin(client).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Commit { .. } => {
                Some(self.handle_commit(session_context, client).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Rollback { .. } => {
                Some(self.handle_rollback(session_context, client).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Deallocate { .. } => {
                Some(Ok(Response::Execution(Tag::new("DEALLOCATE"))))
            }
            datafusion::sql::sqlparser::ast::Statement::CreateTable(ct)
                if table_name_parts(&ct.name).is_some() =>
            {
                Some(self.handle_create_table(ct, session_context, client).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Copy { .. }
                if copy_statement_parts(statement).is_some() =>
            {
                Some(
                    self.handle_copy_from_stdin(statement, session_context, client)
                        .await,
                )
            }
            datafusion::sql::sqlparser::ast::Statement::Call(function)
                if is_compact_call(function) =>
            {
                Some(
                    self.handle_compact_call(function, session_context, client)
                        .await,
                )
            }
            datafusion::sql::sqlparser::ast::Statement::Insert(insert)
                if insert.source.is_some() && insert_target_parts(&insert.table).is_some() =>
            {
                Some(
                    self.handle_insert(
                        insert,
                        session_context,
                        Format::UnifiedBinary,
                        Some(_params),
                        client,
                    )
                    .await,
                )
            }
            datafusion::sql::sqlparser::ast::Statement::AlterTable(alter)
                if table_name_parts(&alter.name).is_some() =>
            {
                Some(
                    self.handle_alter_table(alter, session_context, client)
                        .await,
                )
            }
            datafusion::sql::sqlparser::ast::Statement::Delete(delete)
                if delete_target_parts(delete).is_some() =>
            {
                Some(
                    self.handle_delete(
                        delete,
                        session_context,
                        Format::UnifiedBinary,
                        Some(_params),
                        client,
                    )
                    .await,
                )
            }
            datafusion::sql::sqlparser::ast::Statement::Update(update)
                if update_target_parts(&update.table).is_some() =>
            {
                Some(
                    self.handle_update(
                        update,
                        Format::UnifiedBinary,
                        Some(_params),
                        session_context,
                        client,
                    )
                    .await,
                )
            }
            _ => None,
        }
    }
}

fn empty_logical_plan() -> LogicalPlan {
    LogicalPlan::EmptyRelation(EmptyRelation {
        produce_one_row: false,
        schema: DFSchemaRef::new(DFSchema::empty()),
    })
}

fn snapshot_query_rewrite(
    statement: &Statement,
    catalog_name: &str,
) -> PgWireResult<Option<SnapshotQueryRewrite>> {
    let Statement::Query(query) = statement else {
        return Ok(None);
    };
    let selector_count = query_snapshot_selector_count(query);
    if selector_count == 0 {
        return Ok(None);
    }
    if selector_count > 1 {
        return Err(user_error(anyhow!(
            "DuckLake snapshot reads support exactly one snapshot-qualified table in the first safe path"
        )));
    }

    let mut statement = statement.clone();
    let Statement::Query(query) = &mut statement else {
        unreachable!("checked above")
    };
    let (schema, table, snapshot_id) = rewrite_single_table_snapshot_query(query, catalog_name)?;
    Ok(Some(SnapshotQueryRewrite {
        sql: statement.to_string(),
        snapshot_id,
        schema,
        table,
    }))
}

fn rewrite_single_table_snapshot_query(
    query: &mut Query,
    catalog_name: &str,
) -> PgWireResult<(String, String, i64)> {
    let SetExpr::Select(select) = query.body.as_mut() else {
        return Err(user_error(anyhow!(
            "DuckLake snapshot reads currently support only simple SELECT statements"
        )));
    };
    if select.from.len() != 1 {
        return Err(user_error(anyhow!(
            "DuckLake snapshot reads currently support exactly one FROM table"
        )));
    }
    let table = &mut select.from[0];
    if !table.joins.is_empty() {
        return Err(user_error(anyhow!(
            "DuckLake snapshot reads currently support only single-table reads without joins"
        )));
    }
    match &mut table.relation {
        TableFactor::Table {
            name,
            args,
            version,
            ..
        } => {
            if let Some((schema, table_name, function_args)) = snapshot_table_function_parts(name) {
                if args.is_some() || version.is_some() {
                    return Err(user_error(anyhow!(
                        "DuckLake snapshot reads accept only one snapshot selector per table"
                    )));
                }
                let snapshot_id = snapshot_id_from_function_args(function_args, false)?;
                *name = snapshot_catalog_table_name(catalog_name, &schema, &table_name);
                return Ok((schema, table_name, snapshot_id));
            }
            let (schema, table_name) = snapshot_table_name_parts(name)
                .or_else(|| {
                    args.as_ref()
                        .filter(|args| snapshot_table_args_have_named_selector(args))
                        .and_then(|_| snapshot_bare_table_name_parts(name))
                })
                .ok_or_else(|| {
                    user_error(anyhow!(
                        "DuckLake snapshot reads require a QuackGIS DuckLake table with snapshot => <id>"
                    ))
                })?;
            let snapshot_id = if let Some(table_version) = version.take() {
                snapshot_id_from_table_version(&table_version)?
            } else if let Some(table_args) = args.take() {
                snapshot_id_from_table_args(&table_args)?
            } else {
                return Err(user_error(anyhow!(
                    "internal error: snapshot rewrite missing table snapshot selector"
                )));
            };
            *name = snapshot_catalog_table_name(catalog_name, &schema, &table_name);
            Ok((schema, table_name, snapshot_id))
        }
        TableFactor::Function {
            lateral,
            name,
            args,
            with_ordinality,
            alias,
        } => {
            if *lateral || *with_ordinality {
                return Err(user_error(anyhow!(
                    "DuckLake snapshot table reads do not support LATERAL or WITH ORDINALITY"
                )));
            }
            let (schema, table_name) = snapshot_table_name_parts(name).ok_or_else(|| {
                user_error(anyhow!(
                    "DuckLake snapshot reads require a schema-qualified QuackGIS DuckLake table"
                ))
            })?;
            let snapshot_id = snapshot_id_from_function_args(args, false)?;
            let alias = alias.take();
            table.relation = TableFactor::Table {
                name: snapshot_catalog_table_name(catalog_name, &schema, &table_name),
                alias,
                args: None,
                with_hints: vec![],
                version: None,
                with_ordinality: false,
                partitions: vec![],
                json_path: None,
                sample: None,
                index_hints: vec![],
            };
            Ok((schema, table_name, snapshot_id))
        }
        _ => Err(user_error(anyhow!(
            "DuckLake snapshot reads currently support only named DuckLake tables"
        ))),
    }
}

fn snapshot_catalog_table_name(catalog_name: &str, schema: &str, table: &str) -> ObjectName {
    ObjectName::from(vec![
        Ident::new(catalog_name),
        Ident::new(schema),
        Ident::new(table),
    ])
}

fn snapshot_table_name_parts(name: &ObjectName) -> Option<(String, String)> {
    let parts: Vec<String> = name
        .0
        .iter()
        .map(|p| p.to_string().trim_matches('"').to_string())
        .collect();
    match parts.as_slice() {
        [catalog, schema, table] if catalog == DUCKLAKE_CATALOG && is_snapshot_schema(schema) => {
            Some(("main".to_string(), table.clone()))
        }
        [schema, table] if is_snapshot_schema(schema) => Some(("main".to_string(), table.clone())),
        _ => None,
    }
}

fn snapshot_bare_table_name_parts(name: &ObjectName) -> Option<(String, String)> {
    match name.0.as_slice() {
        [ObjectNamePart::Identifier(table)] => Some(("main".to_string(), table.value.clone())),
        _ => None,
    }
}

fn snapshot_table_function_parts(name: &ObjectName) -> Option<(String, String, &[FunctionArg])> {
    match name.0.as_slice() {
        [
            ObjectNamePart::Identifier(catalog),
            ObjectNamePart::Identifier(schema),
            ObjectNamePart::Function(function),
        ] if catalog.value.eq_ignore_ascii_case(DUCKLAKE_CATALOG)
            && is_snapshot_schema(&schema.value) =>
        {
            Some((
                "main".to_string(),
                function.name.value.clone(),
                function.args.as_slice(),
            ))
        }
        [
            ObjectNamePart::Identifier(schema),
            ObjectNamePart::Function(function),
        ] if is_snapshot_schema(&schema.value) => Some((
            "main".to_string(),
            function.name.value.clone(),
            function.args.as_slice(),
        )),
        _ => None,
    }
}

fn is_snapshot_schema(schema: &str) -> bool {
    schema.eq_ignore_ascii_case("main") || schema.eq_ignore_ascii_case("public")
}

fn snapshot_id_from_table_version(version: &TableVersion) -> PgWireResult<i64> {
    match version {
        TableVersion::VersionAsOf(expr) => snapshot_id_from_expr(expr),
        _ => Err(user_error(anyhow!(
            "DuckLake time travel currently supports literal snapshot ids only"
        ))),
    }
}

fn snapshot_id_from_table_args(args: &TableFunctionArgs) -> PgWireResult<i64> {
    snapshot_id_from_function_args(&args.args, args.settings.is_some())
}

fn snapshot_table_args_have_named_selector(args: &TableFunctionArgs) -> bool {
    args.settings.is_none()
        && args.args.len() == 1
        && function_arg_is_snapshot_selector(&args.args[0])
}

fn function_arg_is_snapshot_selector(arg: &FunctionArg) -> bool {
    match arg {
        FunctionArg::Named { name, .. } => {
            name.value.eq_ignore_ascii_case("snapshot")
                || name.value.eq_ignore_ascii_case("snapshot_id")
        }
        FunctionArg::ExprNamed { name, .. } => match name {
            Expr::Identifier(ident) => {
                ident.value.eq_ignore_ascii_case("snapshot")
                    || ident.value.eq_ignore_ascii_case("snapshot_id")
            }
            _ => false,
        },
        FunctionArg::Unnamed(_) => false,
    }
}

fn snapshot_id_from_function_args(args: &[FunctionArg], has_settings: bool) -> PgWireResult<i64> {
    if has_settings || args.len() != 1 {
        return Err(user_error(anyhow!(
            "DuckLake snapshot table reads use exactly one snapshot id argument: public.table(<snapshot_id>)"
        )));
    }
    match &args[0] {
        FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => snapshot_id_from_expr(expr),
        FunctionArg::Named { name, arg, .. }
            if name.value.eq_ignore_ascii_case("snapshot")
                || name.value.eq_ignore_ascii_case("snapshot_id") =>
        {
            match arg {
                FunctionArgExpr::Expr(expr) => snapshot_id_from_expr(expr),
                _ => Err(user_error(anyhow!(
                    "DuckLake snapshot id must be an expression"
                ))),
            }
        }
        FunctionArg::ExprNamed {
            name: Expr::Identifier(ident),
            arg,
            ..
        } if ident.value.eq_ignore_ascii_case("snapshot")
            || ident.value.eq_ignore_ascii_case("snapshot_id") =>
        {
            match arg {
                FunctionArgExpr::Expr(expr) => snapshot_id_from_expr(expr),
                _ => Err(user_error(anyhow!(
                    "DuckLake snapshot id must be an expression"
                ))),
            }
        }
        _ => Err(user_error(anyhow!(
            "DuckLake snapshot table reads use public.table(<snapshot_id>) or public.table(snapshot => <snapshot_id>)"
        ))),
    }
}

fn snapshot_id_from_expr(expr: &Expr) -> PgWireResult<i64> {
    match expr {
        Expr::Value(value) => match &value.value {
            Value::Number(raw, _) => raw
                .parse::<i64>()
                .map_err(|e| user_error(anyhow!("invalid DuckLake snapshot id {raw:?}: {e}"))),
            _ => Err(user_error(anyhow!(
                "DuckLake snapshot reads require a numeric snapshot id"
            ))),
        },
        Expr::UnaryOp {
            op: UnaryOperator::Plus,
            expr,
        } => snapshot_id_from_expr(expr),
        _ => Err(user_error(anyhow!(
            "DuckLake snapshot reads require a literal snapshot id"
        ))),
    }
}

fn query_snapshot_selector_count(query: &Query) -> usize {
    set_expr_snapshot_selector_count(query.body.as_ref())
}

fn set_expr_snapshot_selector_count(expr: &SetExpr) -> usize {
    match expr {
        SetExpr::Select(select) => select
            .from
            .iter()
            .map(|table| {
                table_factor_snapshot_selector_count(&table.relation)
                    + table
                        .joins
                        .iter()
                        .map(|join| table_factor_snapshot_selector_count(&join.relation))
                        .sum::<usize>()
            })
            .sum(),
        SetExpr::Query(query) => query_snapshot_selector_count(query),
        SetExpr::SetOperation { left, right, .. } => {
            set_expr_snapshot_selector_count(left) + set_expr_snapshot_selector_count(right)
        }
        _ => 0,
    }
}

fn table_factor_snapshot_selector_count(factor: &TableFactor) -> usize {
    match factor {
        TableFactor::Table {
            name,
            args,
            version,
            ..
        } => {
            usize::from(version.is_some())
                + usize::from(args.is_some() && snapshot_table_name_parts(name).is_some())
                + usize::from(
                    args.as_ref()
                        .is_some_and(snapshot_table_args_have_named_selector)
                        && snapshot_bare_table_name_parts(name).is_some(),
                )
                + usize::from(snapshot_table_function_parts(name).is_some())
        }
        TableFactor::Function { name, args, .. } => {
            usize::from(!args.is_empty() && snapshot_table_name_parts(name).is_some())
        }
        TableFactor::Derived { subquery, .. } => query_snapshot_selector_count(subquery),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            table_factor_snapshot_selector_count(&table_with_joins.relation)
                + table_with_joins
                    .joins
                    .iter()
                    .map(|join| table_factor_snapshot_selector_count(&join.relation))
                    .sum::<usize>()
        }
        _ => 0,
    }
}

fn log_query_start<C>(client: &C, protocol: &str, statement: &Statement) -> u64
where
    C: ClientInfo + ?Sized,
{
    let query_id = QUERY_COUNTER.fetch_add(1, Ordering::Relaxed);
    log::info!(
        "quackgis_query_start query_id={query_id} protocol={protocol} pid={} user={} statement_kind={}",
        client.pid_and_secret_key().0,
        client_user(client),
        statement_kind(statement)
    );
    query_id
}

fn client_user<C>(client: &C) -> &str
where
    C: ClientInfo + ?Sized,
{
    client
        .metadata()
        .get("user")
        .map(String::as_str)
        .unwrap_or("unknown")
}

fn statement_kind(statement: &Statement) -> &'static str {
    match statement {
        Statement::Analyze { .. } => "analyze",
        Statement::AlterTable(_) => "alter_table",
        Statement::Call(_) => "call",
        Statement::Commit { .. } => "commit",
        Statement::Copy { .. } => "copy",
        Statement::CreateTable(_) => "create_table",
        Statement::Deallocate { .. } => "deallocate",
        Statement::Delete(_) => "delete",
        Statement::Explain { .. } => "explain",
        Statement::Insert(_) => "insert",
        Statement::Query(_) => "query",
        Statement::Rollback { .. } => "rollback",
        Statement::StartTransaction { .. } => "start_transaction",
        Statement::Update { .. } => "update",
        _ => "other",
    }
}

fn statement_requires_write(statement: &Statement) -> bool {
    match statement {
        Statement::CreateTable(ct) => table_name_parts(&ct.name).is_some(),
        Statement::Copy { .. } => copy_statement_parts(statement).is_some(),
        Statement::Call(function) => is_compact_call(function),
        Statement::Insert(insert) => {
            insert.source.is_some() && insert_target_parts(&insert.table).is_some()
        }
        Statement::AlterTable(alter) => table_name_parts(&alter.name).is_some(),
        Statement::Delete(delete) => delete_target_parts(delete).is_some(),
        Statement::Update(update) => update_target_parts(&update.table).is_some(),
        _ => false,
    }
}

fn client_transaction_state<C>(client: &C) -> Arc<ClientTransactionState>
where
    C: ClientInfo + Send + Sync + ?Sized,
{
    client
        .session_extensions()
        .get_or_insert_with(ClientTransactionState::default)
}

fn copy_in_session_state<C>(client: &C) -> Arc<CopyInSessionState>
where
    C: ClientInfo + Send + Sync + ?Sized,
{
    client
        .session_extensions()
        .get_or_insert_with(CopyInSessionState::default)
}

fn param_values_empty(params: &ParamValues) -> bool {
    match params {
        ParamValues::List(values) => values.is_empty(),
        ParamValues::Map(values) => values.is_empty(),
    }
}

fn configured_ducklake_table_writer(
    writer: Arc<dyn MetadataWriter>,
    object_store: Arc<dyn object_store::ObjectStore>,
) -> PgWireResult<DuckLakeTableWriter> {
    let mut table_writer = DuckLakeTableWriter::new(writer, object_store)
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    if let Some(rows) = configured_row_group_rows()? {
        table_writer = table_writer.with_max_row_group_rows(rows);
    }
    Ok(table_writer)
}

fn configured_row_group_rows() -> PgWireResult<Option<usize>> {
    match std::env::var(DUCKLAKE_ROW_GROUP_ROWS_ENV) {
        Ok(value) => {
            let value = value.trim();
            if value.is_empty() || value == "0" {
                return Ok(None);
            }
            let rows = value.parse::<usize>().map_err(|err| {
                user_error(anyhow!(
                    "{DUCKLAKE_ROW_GROUP_ROWS_ENV} must be a positive integer or 0 to disable: {err}"
                ))
            })?;
            if rows == 0 { Ok(None) } else { Ok(Some(rows)) }
        }
        Err(std::env::VarError::NotPresent) => Ok(Some(DEFAULT_DUCKLAKE_ROW_GROUP_ROWS)),
        Err(err) => Err(user_error(anyhow!(
            "could not read {DUCKLAKE_ROW_GROUP_ROWS_ENV}: {err}"
        ))),
    }
}

fn next_transaction_id<C>(client: &C) -> String
where
    C: ClientInfo + Send + Sync + ?Sized,
{
    let pid = client.pid_and_secret_key().0;
    let counter = TRANSACTION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("__quackgis_tx_{pid}_{counter}")
}

async fn collect_query_batches(
    session_context: &SessionContext,
    query: &str,
) -> PgWireResult<Vec<RecordBatch>> {
    session_context
        .sql(query)
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?
        .collect()
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))
}

async fn collect_selective_read_batches(
    session_context: &SessionContext,
    query: &str,
) -> PgWireResult<Vec<RecordBatch>> {
    match configured_selective_read_target_partitions()? {
        Some(target_partitions) => {
            let tuned_context =
                session_context_with_target_partitions(session_context, target_partitions);
            collect_query_batches(&tuned_context, query).await
        }
        None => collect_query_batches(session_context, query).await,
    }
}

fn session_context_with_target_partitions(
    session_context: &SessionContext,
    target_partitions: usize,
) -> SessionContext {
    // Keep scan parallelism tuning query-scoped. Mutating the shared
    // SessionContext would leak selective-read settings into concurrent broad
    // scans, writes, and compaction.
    let mut state = session_context.state();
    let config = state
        .config()
        .clone()
        .with_target_partitions(target_partitions);
    *state.config_mut() = config;
    SessionContext::new_with_state(state)
}

fn configured_selective_read_target_partitions() -> PgWireResult<Option<usize>> {
    match std::env::var(SELECTIVE_READ_TARGET_PARTITIONS_ENV) {
        Ok(value) => parse_selective_read_target_partitions_value(&value).map_err(user_error),
        Err(std::env::VarError::NotPresent) => Ok(Some(DEFAULT_SELECTIVE_READ_TARGET_PARTITIONS)),
        Err(err) => Err(user_error(anyhow!(
            "could not read {SELECTIVE_READ_TARGET_PARTITIONS_ENV}: {err}"
        ))),
    }
}

fn parse_selective_read_target_partitions_value(value: &str) -> Result<Option<usize>> {
    let value = value.trim();
    if value.is_empty() || value == "0" {
        return Ok(None);
    }

    let target_partitions = value.parse::<usize>().map_err(|err| {
        anyhow!(
            "{SELECTIVE_READ_TARGET_PARTITIONS_ENV} must be a positive integer or 0 to disable automatic selective-read tuning: {err}"
        )
    })?;
    if target_partitions == 0 {
        Ok(None)
    } else {
        Ok(Some(target_partitions))
    }
}

async fn collect_normalized_query_batches(
    session_context: &SessionContext,
    query: &str,
) -> PgWireResult<(Vec<RecordBatch>, usize)> {
    let df = session_context
        .sql(query)
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    let output_schema = Arc::new(df.schema().as_arrow().clone());
    let mut batches = df
        .collect()
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    let rows = batches.iter().map(|batch| batch.num_rows()).sum();
    if batches.is_empty() {
        let fields = output_schema
            .fields()
            .iter()
            .map(|field| field.as_ref().clone())
            .collect::<Vec<_>>();
        batches.push(empty_batch_for_fields(fields).map_err(user_error)?);
    }
    let batches = normalize_batches_for_ducklake(batches).map_err(user_error)?;
    Ok((batches, rows))
}

fn query_response_from_batches_with_format(
    batches: Vec<RecordBatch>,
    format: Format,
) -> PgWireResult<QueryResponse> {
    let schema = batches
        .first()
        .map(|batch| batch.schema())
        .unwrap_or_else(|| Arc::new(Schema::empty()));
    let fields = Arc::new(arrow_schema_to_pg_fields(schema.as_ref(), &format, None)?);
    let rows = batches
        .into_iter()
        .flat_map(|batch| encode_recordbatch(Arc::clone(&fields), batch).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    Ok(QueryResponse::new(
        fields,
        Box::pin(futures::stream::iter(rows)),
    ))
}

fn returning_select_list(returning: &[SelectItem]) -> String {
    if returning.is_empty() {
        "*".to_string()
    } else {
        returning
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn returning_query_from_source(source_query: &str, returning: &[SelectItem]) -> String {
    format!(
        "SELECT {} FROM ({source_query}) AS dml_returning",
        returning_select_list(returning)
    )
}

fn returning_query_from_table(
    table_ref: &str,
    returning: &[SelectItem],
    predicate: Option<&str>,
) -> String {
    let where_clause = predicate
        .map(|predicate| format!(" WHERE {predicate}"))
        .unwrap_or_default();
    format!(
        "SELECT {} FROM {table_ref}{where_clause}",
        returning_select_list(returning)
    )
}

fn ducklake_statement_parts(statement: &Statement) -> Option<(String, String)> {
    match statement {
        datafusion::sql::sqlparser::ast::Statement::CreateTable(ct) => table_name_parts(&ct.name),
        datafusion::sql::sqlparser::ast::Statement::Insert(insert) if insert.source.is_some() => {
            insert_target_parts(&insert.table)
        }
        datafusion::sql::sqlparser::ast::Statement::AlterTable(alter) => {
            table_name_parts(&alter.name)
        }
        datafusion::sql::sqlparser::ast::Statement::Delete(delete) => delete_target_parts(delete),
        datafusion::sql::sqlparser::ast::Statement::Update(update) => {
            update_target_parts(&update.table)
        }
        datafusion::sql::sqlparser::ast::Statement::Copy { .. } => copy_statement_parts(statement),
        _ => None,
    }
}

fn copy_statement_parts(statement: &Statement) -> Option<(String, String)> {
    match statement {
        Statement::Copy {
            source: CopySource::Table { table_name, .. },
            to,
            target,
            ..
        } if !*to && matches!(target, CopyTarget::Stdin) => table_name_parts(table_name),
        _ => None,
    }
}

fn is_compact_call(function: &Function) -> bool {
    object_name_last(&function.name).is_some_and(|name| {
        name.eq_ignore_ascii_case("quackgis_compact_table")
            || name.eq_ignore_ascii_case("quackgis_compact")
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactScope {
    WholeTable,
    LayoutBucket { time_bucket: i64, space_bucket: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompactTarget {
    schema: String,
    table: String,
    scope: CompactScope,
}

fn compact_call_parts(function: &Function) -> PgWireResult<CompactTarget> {
    let FunctionArguments::List(args) = &function.args else {
        return Err(user_error(anyhow!(
            "{} requires a table-name argument",
            function.name
        )));
    };
    if !matches!(args.args.len(), 1 | 3)
        || !args.clauses.is_empty()
        || args.duplicate_treatment.is_some()
    {
        return Err(user_error(anyhow!(
            "{} requires one table-name argument, optionally followed by time_bucket and space_bucket",
            function.name
        )));
    }
    let FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) = &args.args[0] else {
        return Err(user_error(anyhow!(
            "{} argument must be a table name string or identifier",
            function.name
        )));
    };
    let table = compact_table_arg_text(expr)?;
    let (schema, table) = table_text_parts(&table)
        .ok_or_else(|| user_error(anyhow!("invalid table name: {table}")))?;
    let scope = if args.args.len() == 3 {
        let time_bucket = compact_i64_arg(&args.args[1], "time_bucket")?;
        let space_bucket = compact_i64_arg(&args.args[2], "space_bucket")?;
        CompactScope::LayoutBucket {
            time_bucket,
            space_bucket,
        }
    } else {
        CompactScope::WholeTable
    };
    Ok(CompactTarget {
        schema,
        table,
        scope,
    })
}

fn compact_i64_arg(arg: &FunctionArg, name: &str) -> PgWireResult<i64> {
    let FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) = arg else {
        return Err(user_error(anyhow!(
            "compact table {name} argument must be an integer literal"
        )));
    };
    compact_i64_expr(expr, name)
}

fn compact_i64_expr(expr: &Expr, name: &str) -> PgWireResult<i64> {
    match expr {
        Expr::Value(value) => match &value.value {
            Value::Number(value, _) => value.parse::<i64>().map_err(|e| {
                user_error(anyhow!(
                    "compact table {name} argument must be a 64-bit integer: {e}"
                ))
            }),
            Value::SingleQuotedString(value)
            | Value::EscapedStringLiteral(value)
            | Value::DoubleQuotedString(value) => value.parse::<i64>().map_err(|e| {
                user_error(anyhow!(
                    "compact table {name} argument must be a 64-bit integer: {e}"
                ))
            }),
            other => Err(user_error(anyhow!(
                "compact table {name} argument must be an integer literal, got {other}"
            ))),
        },
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr,
        } => compact_i64_expr(expr, name).and_then(|value| {
            value.checked_neg().ok_or_else(|| {
                user_error(anyhow!(
                    "compact table {name} argument is below the 64-bit integer range"
                ))
            })
        }),
        other => Err(user_error(anyhow!(
            "compact table {name} argument must be an integer literal, got {other}"
        ))),
    }
}

fn compact_table_arg_text(expr: &Expr) -> PgWireResult<String> {
    match expr {
        Expr::Value(value) => match &value.value {
            Value::SingleQuotedString(value)
            | Value::EscapedStringLiteral(value)
            | Value::DoubleQuotedString(value) => Ok(value.clone()),
            other => Err(user_error(anyhow!(
                "compact table argument must be a string literal or identifier, got {other}"
            ))),
        },
        Expr::Identifier(ident) => Ok(ident.value.clone()),
        Expr::CompoundIdentifier(idents) => Ok(idents
            .iter()
            .map(|ident| ident.value.as_str())
            .collect::<Vec<_>>()
            .join(".")),
        other => Err(user_error(anyhow!(
            "compact table argument must be a string literal or identifier, got {other}"
        ))),
    }
}

fn table_text_parts(value: &str) -> Option<(String, String)> {
    let parts = value
        .split('.')
        .map(|part| part.trim().trim_matches('"').to_string())
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [catalog, schema, table]
            if catalog.eq_ignore_ascii_case(DUCKLAKE_CATALOG)
                && is_ducklake_user_schema(schema) =>
        {
            Some(("main".to_string(), table.clone()))
        }
        [schema, table] if is_ducklake_user_schema(schema) => {
            Some(("main".to_string(), table.clone()))
        }
        [table] if !table.is_empty() => Some(("main".to_string(), table.clone())),
        _ => None,
    }
}

fn is_ducklake_user_schema(schema: &str) -> bool {
    schema.eq_ignore_ascii_case("main") || schema.eq_ignore_ascii_case("public")
}

impl DuckLakeSqlHook {
    async fn handle_begin(&self, client: &mut dyn HookClient) -> PgWireResult<Response> {
        let tx_state = client_transaction_state(client);
        let mut state = tx_state.inner.lock().await;
        match &*state {
            TransactionState::Idle => {
                let tx_id = next_transaction_id(client);
                *state = TransactionState::Active(ActiveTransaction {
                    id: tx_id,
                    staged_tables: HashMap::new(),
                });
                // Make later statements in the same simple-query message see the
                // transaction immediately; pgwire will also derive the final
                // ReadyForQuery status from the TransactionStart response.
                client.set_transaction_status(TransactionStatus::Transaction);
                Ok(Response::TransactionStart(Tag::new("BEGIN")))
            }
            TransactionState::Active(_) => {
                log::warn!("BEGIN command ignored: already in transaction block");
                Ok(Response::Execution(Tag::new("BEGIN")))
            }
        }
    }

    async fn handle_commit(
        &self,
        session_context: &SessionContext,
        client: &mut dyn HookClient,
    ) -> PgWireResult<Response> {
        let tx_state = client_transaction_state(client);
        let active = {
            let mut state = tx_state.inner.lock().await;
            std::mem::replace(&mut *state, TransactionState::Idle)
        };
        match active {
            TransactionState::Idle => {
                client.set_transaction_status(TransactionStatus::Idle);
                Ok(Response::TransactionEnd(Tag::new("COMMIT")))
            }
            TransactionState::Active(active) => {
                if let Err(err) = self
                    .commit_active_transaction(session_context, active)
                    .await
                {
                    client.set_transaction_status(TransactionStatus::Error);
                    return Err(err);
                }
                client.set_transaction_status(TransactionStatus::Idle);
                Ok(Response::TransactionEnd(Tag::new("COMMIT")))
            }
        }
    }

    async fn handle_rollback(
        &self,
        session_context: &SessionContext,
        client: &mut dyn HookClient,
    ) -> PgWireResult<Response> {
        let tx_state = client_transaction_state(client);
        let active = {
            let mut state = tx_state.inner.lock().await;
            std::mem::replace(&mut *state, TransactionState::Idle)
        };
        if let TransactionState::Active(active) = active {
            self.cleanup_staged_tables(session_context, &active)?;
        }
        client.set_transaction_status(TransactionStatus::Idle);
        Ok(Response::TransactionEnd(Tag::new("ROLLBACK")))
    }

    async fn client_in_transaction<C>(&self, client: &C) -> bool
    where
        C: ClientInfo + Send + Sync + ?Sized,
    {
        let tx_state = client_transaction_state(client);
        let state = tx_state.inner.lock().await;
        matches!(*state, TransactionState::Active(_))
    }

    async fn commit_active_transaction(
        &self,
        session_context: &SessionContext,
        mut active: ActiveTransaction,
    ) -> PgWireResult<()> {
        let temp_tables = Self::staged_temp_tables(&active);
        for staged in active.staged_tables.values_mut() {
            let mut writer = staged
                .writer_session
                .take()
                .ok_or_else(|| user_error(anyhow!("transaction table writer already consumed")))?;
            let batches =
                layout::sort_batches_by_layout(staged.batches.clone()).map_err(user_error)?;
            for batch in &batches {
                if let Err(err) = writer.write_batch(batch) {
                    self.cleanup_temp_tables(session_context, &temp_tables)?;
                    return Err(PgWireError::ApiError(Box::new(err)));
                }
            }
            if let Err(err) = writer.finish().await {
                self.cleanup_temp_tables(session_context, &temp_tables)?;
                return Err(PgWireError::ApiError(Box::new(err)));
            }
        }
        self.cleanup_temp_tables(session_context, &temp_tables)?;
        if !active.staged_tables.is_empty() {
            self.refresh_ducklake_catalog_strong(session_context)
                .await?;
        }
        Ok(())
    }

    fn staged_temp_tables(active: &ActiveTransaction) -> Vec<String> {
        active
            .staged_tables
            .values()
            .map(|staged| staged.temp_table.clone())
            .collect()
    }

    fn cleanup_staged_tables(
        &self,
        session_context: &SessionContext,
        active: &ActiveTransaction,
    ) -> PgWireResult<()> {
        self.cleanup_temp_tables(session_context, &Self::staged_temp_tables(active))
    }

    fn cleanup_temp_tables(
        &self,
        session_context: &SessionContext,
        temp_tables: &[String],
    ) -> PgWireResult<()> {
        for temp_table in temp_tables {
            let _ = session_context.deregister_table(temp_table.clone());
        }
        Ok(())
    }

    async fn ensure_staged_table<'a>(
        &self,
        active: &'a mut ActiveTransaction,
        session_context: &SessionContext,
        schema: &str,
        table: &str,
    ) -> PgWireResult<&'a mut StagedTable> {
        let key = TableKey {
            schema: schema.to_string(),
            table: table.to_string(),
        };
        if !active.staged_tables.is_empty() && !active.staged_tables.contains_key(&key) {
            return Err(user_error(anyhow!(
                "explicit transactions currently support one DuckLake table at a time"
            )));
        }
        if !active.staged_tables.contains_key(&key) {
            let temp_table = format!("{}_{}", active.id, active.staged_tables.len() + 1);
            let staged = self
                .load_staged_table(session_context, schema, table, temp_table)
                .await?;
            active.staged_tables.insert(key.clone(), staged);
        }
        Ok(active
            .staged_tables
            .get_mut(&key)
            .expect("staged table inserted above"))
    }

    async fn load_staged_table(
        &self,
        session_context: &SessionContext,
        schema: &str,
        table: &str,
        temp_table: String,
    ) -> PgWireResult<StagedTable> {
        let table_ref = ducklake_table_ref(schema, table);
        let schema_ref = self.table_schema(session_context, &table_ref).await?;
        let writer_session = self
            .begin_transaction_table_write(schema, table, schema_ref.as_ref())
            .await?;
        let query = format!("SELECT * FROM {table_ref}");
        let (batches, _) = collect_normalized_query_batches(session_context, &query).await?;
        let batches = layout::project_batches(batches).map_err(user_error)?;
        self.register_staged_batches(session_context, &temp_table, &batches)?;
        Ok(StagedTable {
            temp_table,
            batches,
            writer_session: Some(writer_session),
        })
    }

    async fn begin_transaction_table_write(
        &self,
        schema: &str,
        table: &str,
        arrow_schema: &Schema,
    ) -> PgWireResult<TableWriteSession> {
        let writer = self.storage_writer().await?;
        let object_store = self.storage_object_store()?;
        let table_writer = configured_ducklake_table_writer(writer, object_store)?;
        table_writer
            .begin_write(schema, table, arrow_schema, WriteMode::Replace)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))
    }

    fn register_staged_batches(
        &self,
        session_context: &SessionContext,
        temp_table: &str,
        batches: &[RecordBatch],
    ) -> PgWireResult<()> {
        let schema = batches
            .first()
            .map(|batch| batch.schema())
            .ok_or_else(|| user_error(anyhow!("staged table must have at least one batch")))?;
        let mem = MemTable::try_new(schema, vec![batches.to_vec()])
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let _ = session_context.deregister_table(temp_table.to_string());
        session_context
            .register_table(temp_table.to_string(), Arc::new(mem))
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        Ok(())
    }

    fn staged_table_ref(staged: &StagedTable) -> String {
        quote_ident(&staged.temp_table)
    }

    fn replace_staged_batches(
        &self,
        session_context: &SessionContext,
        staged: &mut StagedTable,
        batches: Vec<RecordBatch>,
    ) -> PgWireResult<()> {
        let batches = layout::project_batches(batches).map_err(user_error)?;
        staged.batches = batches;
        self.register_staged_batches(session_context, &staged.temp_table, &staged.batches)
    }

    fn append_staged_batches(
        existing: &[RecordBatch],
        appended: Vec<RecordBatch>,
    ) -> Result<Vec<RecordBatch>> {
        let schema = existing
            .first()
            .or_else(|| appended.first())
            .map(|batch| batch.schema())
            .ok_or_else(|| anyhow!("staged table must have a schema"))?;
        let mut out = Vec::new();
        for batch in existing.iter().chain(appended.iter()) {
            if batch.schema().as_ref() != schema.as_ref() {
                return Err(anyhow!("staged batch schema changed during transaction"));
            }
            if batch.num_rows() > 0 {
                out.push(batch.clone());
            }
        }
        if out.is_empty() {
            let fields = schema
                .fields()
                .iter()
                .map(|field| field.as_ref().clone())
                .collect::<Vec<_>>();
            out.push(empty_batch_for_fields(fields)?);
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_insert_transactional(
        &self,
        insert: &datafusion::sql::sqlparser::ast::Insert,
        session_context: &SessionContext,
        result_format: Format,
        client: &dyn HookClient,
        schema: String,
        table: String,
        source_query: String,
    ) -> PgWireResult<Response> {
        let tx_state = client_transaction_state(client);
        let mut state = tx_state.inner.lock().await;
        let TransactionState::Active(active) = &mut *state else {
            return Err(user_error(anyhow!("transaction state is not active")));
        };
        let staged = self
            .ensure_staged_table(active, session_context, &schema, &table)
            .await?;
        let table_ref = Self::staged_table_ref(staged);
        let target_schema = staged
            .batches
            .first()
            .map(|batch| batch.schema())
            .ok_or_else(|| user_error(anyhow!("staged table must have a schema")))?;
        let query = if insert.columns.is_empty()
            && !insert_source_is_values(insert.source.as_ref().expect("guarded by caller"))
        {
            source_query
        } else {
            self.insert_source_with_schema(
                &table_ref,
                &target_schema,
                &insert.columns,
                &source_query,
            )?
        };
        let returning_batches = if let Some(returning) = insert.returning.as_deref() {
            let returning_query = returning_query_from_source(&query, returning);
            Some(collect_query_batches(session_context, &returning_query).await?)
        } else {
            None
        };
        let (new_batches, rows) = collect_normalized_query_batches(session_context, &query).await?;
        let combined =
            Self::append_staged_batches(&staged.batches, new_batches).map_err(user_error)?;
        self.replace_staged_batches(session_context, staged, combined)?;
        if let Some(batches) = returning_batches {
            return query_response_from_batches_with_format(batches, result_format)
                .map(Response::Query);
        }
        Ok(Response::Execution(Tag::new(&format!("INSERT 0 {rows}"))))
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_delete_transactional(
        &self,
        delete: &datafusion::sql::sqlparser::ast::Delete,
        session_context: &SessionContext,
        result_format: Format,
        params: Option<&ParamValues>,
        client: &dyn HookClient,
        schema: String,
        table: String,
    ) -> PgWireResult<Response> {
        let tx_state = client_transaction_state(client);
        let mut state = tx_state.inner.lock().await;
        let TransactionState::Active(active) = &mut *state else {
            return Err(user_error(anyhow!("transaction state is not active")));
        };
        let staged = self
            .ensure_staged_table(active, session_context, &schema, &table)
            .await?;
        let table_ref = Self::staged_table_ref(staged);
        let predicate = delete
            .selection
            .as_ref()
            .map(|e| inline_params_if_needed(&e.to_string(), params))
            .transpose()?;
        let where_clause = predicate
            .as_ref()
            .map(|predicate| format!("NOT ({predicate})"))
            .unwrap_or_else(|| "FALSE".to_string());
        let returning_batches = if let Some(returning) = delete.returning.as_deref() {
            let predicate = predicate.clone().unwrap_or_else(|| "TRUE".to_string());
            let returning_query =
                returning_query_from_table(&table_ref, returning, Some(&predicate));
            Some(collect_query_batches(session_context, &returning_query).await?)
        } else {
            None
        };
        let query = format!("SELECT * FROM {table_ref} WHERE {where_clause}");
        let (batches, remaining) =
            collect_normalized_query_batches(session_context, &query).await?;
        self.replace_staged_batches(session_context, staged, batches)?;
        if let Some(batches) = returning_batches {
            return query_response_from_batches_with_format(batches, result_format)
                .map(Response::Query);
        }
        Ok(Response::Execution(Tag::new(&format!(
            "DELETE {remaining}"
        ))))
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_update_transactional(
        &self,
        update: &datafusion::sql::sqlparser::ast::Update,
        result_format: Format,
        params: Option<&ParamValues>,
        session_context: &SessionContext,
        client: &dyn HookClient,
        schema: String,
        table_name: String,
    ) -> PgWireResult<Response> {
        let tx_state = client_transaction_state(client);
        let mut state = tx_state.inner.lock().await;
        let TransactionState::Active(active) = &mut *state else {
            return Err(user_error(anyhow!("transaction state is not active")));
        };
        let staged = self
            .ensure_staged_table(active, session_context, &schema, &table_name)
            .await?;
        let table_ref = Self::staged_table_ref(staged);
        let schema_ref = staged
            .batches
            .first()
            .map(|batch| batch.schema())
            .ok_or_else(|| user_error(anyhow!("staged table must have a schema")))?;
        let mut assignment_map = HashMap::new();
        for assignment in &update.assignments {
            let AssignmentTarget::ColumnName(name) = &assignment.target else {
                return Err(user_error(anyhow!(
                    "tuple UPDATE assignments are not supported yet"
                )));
            };
            let col = object_name_last(name)
                .ok_or_else(|| user_error(anyhow!("invalid UPDATE target")))?;
            let value = inline_params_if_needed(&assignment.value.to_string(), params)?;
            let value = rewrite_st_geomfromwkb_zero_srid_literals(&value);
            assignment_map.insert(col, value);
        }
        let predicate = update
            .selection
            .as_ref()
            .map(|e| inline_params_if_needed(&e.to_string(), params))
            .transpose()?;
        let mut select_items = Vec::new();
        for field in schema_ref.fields() {
            let col = field.name();
            let expr = if let Some(value) = assignment_map.get(col) {
                let sql_type = arrow_type_to_sql(field.data_type()).map_err(user_error)?;
                if let Some(pred) = &predicate {
                    format!(
                        "CAST(CASE WHEN {pred} THEN {value} ELSE {} END AS {sql_type}) AS {}",
                        quote_ident(col),
                        quote_ident(col)
                    )
                } else {
                    format!("CAST({value} AS {sql_type}) AS {}", quote_ident(col))
                }
            } else {
                quote_ident(col)
            };
            select_items.push(expr);
        }
        let query = format!("SELECT {} FROM {table_ref}", select_items.join(", "));
        let returning_batches = if let Some(returning) = update.returning.as_deref() {
            let source_query = if let Some(pred) = &predicate {
                format!(
                    "SELECT {} FROM {table_ref} WHERE {pred}",
                    select_items.join(", ")
                )
            } else {
                query.clone()
            };
            let returning_query = returning_query_from_source(&source_query, returning);
            Some(collect_query_batches(session_context, &returning_query).await?)
        } else {
            None
        };
        let (batches, rows) = collect_normalized_query_batches(session_context, &query).await?;
        self.replace_staged_batches(session_context, staged, batches)?;
        if let Some(batches) = returning_batches {
            return query_response_from_batches_with_format(batches, result_format)
                .map(Response::Query);
        }
        Ok(Response::Execution(Tag::new(&format!("UPDATE {rows}"))))
    }

    async fn handle_alter_table_transactional(
        &self,
        alter: &AlterTable,
        session_context: &SessionContext,
        client: &dyn HookClient,
    ) -> PgWireResult<Response> {
        let (schema, table) = table_name_parts(&alter.name).expect("guarded by caller");
        for operation in &alter.operations {
            match operation {
                AlterTableOperation::AddColumn {
                    if_not_exists,
                    column_def,
                    ..
                } => {
                    self.add_column_transactional(
                        session_context,
                        client,
                        &schema,
                        &table,
                        column_def,
                        *if_not_exists,
                    )
                    .await?;
                }
                other => {
                    return Err(user_error(anyhow!(
                        "unsupported ALTER TABLE operation inside explicit transaction for {schema}.{table}: {other}"
                    )));
                }
            }
        }
        Ok(Response::Execution(Tag::new("ALTER TABLE")))
    }

    async fn add_column_transactional(
        &self,
        session_context: &SessionContext,
        client: &dyn HookClient,
        schema: &str,
        table: &str,
        column_def: &ColumnDef,
        if_not_exists: bool,
    ) -> PgWireResult<()> {
        let new_field = sql_type_to_arrow_field(column_def).map_err(user_error)?;
        let tx_state = client_transaction_state(client);
        let mut state = tx_state.inner.lock().await;
        let TransactionState::Active(active) = &mut *state else {
            return Err(user_error(anyhow!("transaction state is not active")));
        };
        let staged = self
            .ensure_staged_table(active, session_context, schema, table)
            .await?;
        if staged.batches.first().is_some_and(|batch| {
            batch
                .schema()
                .fields()
                .iter()
                .any(|field| field.name() == new_field.name())
        }) {
            if if_not_exists {
                return Ok(());
            }
            return Err(user_error(anyhow!(
                "column already exists: {}",
                new_field.name()
            )));
        }

        let batches = add_null_column_to_batches(&staged.batches, new_field).map_err(user_error)?;
        let staged_schema = batches
            .first()
            .map(|batch| batch.schema())
            .ok_or_else(|| user_error(anyhow!("staged table must have a schema")))?;
        staged.writer_session = Some(
            self.begin_transaction_table_write(schema, table, staged_schema.as_ref())
                .await?,
        );
        self.replace_staged_batches(session_context, staged, batches)?;
        Ok(())
    }

    async fn returning_logical_plan(
        &self,
        statement: &datafusion::sql::sqlparser::ast::Statement,
        session_context: &SessionContext,
    ) -> Option<PgWireResult<LogicalPlan>> {
        let (table, returning) = match statement {
            datafusion::sql::sqlparser::ast::Statement::Insert(insert)
                if insert_target_parts(&insert.table).is_some() =>
            {
                let (_, table) = insert_target_parts(&insert.table)?;
                (table, insert.returning.as_deref()?)
            }
            datafusion::sql::sqlparser::ast::Statement::Delete(delete)
                if delete_target_parts(delete).is_some() =>
            {
                let (_, table) = delete_target_parts(delete)?;
                (table, delete.returning.as_deref()?)
            }
            datafusion::sql::sqlparser::ast::Statement::Update(update)
                if update_target_parts(&update.table).is_some() =>
            {
                let (_, table) = update_target_parts(&update.table)?;
                (table, update.returning.as_deref()?)
            }
            _ => return None,
        };
        let table_ref = public_table_ref(&table);
        let sql = returning_query_from_table(&table_ref, returning, Some("FALSE"));
        Some(
            session_context
                .sql(&sql)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))
                .and_then(|df| {
                    df.into_optimized_plan()
                        .map_err(|e| PgWireError::ApiError(Box::new(e)))
                }),
        )
    }

    async fn handle_create_table(
        &self,
        ct: &datafusion::sql::sqlparser::ast::CreateTable,
        session_context: &SessionContext,
        client: &dyn HookClient,
    ) -> PgWireResult<Response> {
        if self.client_in_transaction(client).await {
            return Err(user_error(anyhow!(
                "CREATE TABLE inside explicit transactions is not supported yet"
            )));
        }
        let (schema, table) = table_name_parts(&ct.name).expect("guarded by caller");
        if let Some(query) = &ct.query {
            self.write_query(
                session_context,
                &query.to_string(),
                &schema,
                &table,
                WriteDisposition::Replace,
            )
            .await?;
        } else {
            self.create_empty_table(&schema, &table, &ct.columns)
                .await?;
            self.refresh_ducklake_catalog(session_context).await?;
        }
        Ok(Response::Execution(Tag::new("CREATE TABLE")))
    }

    async fn handle_compact_call(
        &self,
        function: &Function,
        session_context: &SessionContext,
        client: &dyn HookClient,
    ) -> PgWireResult<Response> {
        if self.client_in_transaction(client).await {
            return Err(user_error(anyhow!(
                "quackgis_compact_table inside explicit transactions is not supported"
            )));
        }
        let target = compact_call_parts(function)?;
        let rows = self
            .compact_table(session_context, &target.schema, &target.table, target.scope)
            .await?;
        COMPACTION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tag = match target.scope {
            CompactScope::WholeTable => format!("COMPACT {rows}"),
            CompactScope::LayoutBucket { .. } => format!("COMPACT BUCKET {rows}"),
        };
        Ok(Response::Execution(Tag::new(&tag)))
    }

    async fn compact_table(
        &self,
        session_context: &SessionContext,
        schema: &str,
        table: &str,
        scope: CompactScope,
    ) -> PgWireResult<usize> {
        let table_ref = ducklake_table_ref(schema, table);
        if let CompactScope::LayoutBucket {
            time_bucket,
            space_bucket,
        } = scope
            && let Some(rows) = self
                .try_native_compact_bucket(
                    session_context,
                    schema,
                    table,
                    time_bucket,
                    space_bucket,
                )
                .await?
        {
            self.refresh_ducklake_catalog(session_context).await?;
            return Ok(rows);
        }

        let (batches, rows) = collect_normalized_query_batches(
            session_context,
            &format!("SELECT * FROM {table_ref}"),
        )
        .await?;
        let rows = match scope {
            CompactScope::WholeTable => rows,
            CompactScope::LayoutBucket {
                time_bucket,
                space_bucket,
            } => {
                self.compact_bucket_row_count(
                    session_context,
                    &table_ref,
                    time_bucket,
                    space_bucket,
                )
                .await?
            }
        };
        self.write_batches(schema, table, &batches, WriteDisposition::Replace)
            .await?;
        self.refresh_ducklake_catalog(session_context).await?;
        Ok(rows)
    }

    async fn try_native_compact_bucket(
        &self,
        session_context: &SessionContext,
        schema: &str,
        table: &str,
        time_bucket: i64,
        space_bucket: i64,
    ) -> PgWireResult<Option<usize>> {
        let predicate = format!(
            "{} = {time_bucket} AND {} = {space_bucket}",
            quote_ident(layout::TIME_BUCKET),
            quote_ident(layout::SPACE_BUCKET)
        );
        let Some(mut plan) = self
            .plan_native_rows(session_context, schema, table, Some(&predicate), "compact")
            .await?
        else {
            return Ok(None);
        };
        if plan.affected_count == 0 {
            return Ok(Some(0));
        }

        let table_ref = ducklake_table_ref(schema, table);
        let schema_ref = self.table_schema(session_context, &table_ref).await?;
        let select_items = schema_ref
            .fields()
            .iter()
            .map(|field| quote_ident(field.name()))
            .collect::<Vec<_>>();
        let rowid_table_ref = format!(
            "{}.{}.{}",
            quote_ident(&plan.catalog_name),
            quote_ident(schema),
            quote_ident(table)
        );
        let replacement_query = format!(
            "SELECT {} FROM {rowid_table_ref} WHERE {predicate}",
            select_items.join(", ")
        );
        let replacement_batches = match collect_query_batches(
            &plan.rowid_context,
            &replacement_query,
        )
        .await
        {
            Ok(batches) => batches,
            Err(err) => {
                log::debug!(
                    "native bucket compaction replacement-row planning failed for {schema}.{table}; falling back to rewrite: {err}"
                );
                return Ok(None);
            }
        };
        let replacement_rows = replacement_batches
            .iter()
            .map(|b| b.num_rows())
            .sum::<usize>();
        if replacement_rows != plan.affected_count || replacement_batches.is_empty() {
            log::debug!(
                "native bucket compaction planned {} rowids but {} replacement rows for {schema}.{table}; falling back to rewrite",
                plan.affected_count,
                replacement_rows
            );
            return Ok(None);
        }
        let replacement_batches =
            normalize_batches_for_ducklake(replacement_batches).map_err(user_error)?;
        let replacement_batches =
            layout::project_batches(replacement_batches).map_err(user_error)?;
        let replacement_batches =
            layout::sort_batches_by_layout(replacement_batches).map_err(user_error)?;

        let writer = self.storage_writer().await?;
        let object_store = self.storage_object_store()?;
        let table_writer = configured_ducklake_table_writer(Arc::clone(&writer), object_store)?;
        let pending_file = table_writer
            .write_pending_data_file(schema, table, &replacement_batches)
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let mutation = TableMutation::new().append_data_file(pending_file);
        let mutation = self
            .add_native_delete_files_to_mutation(&mut plan, schema, table, &table_writer, mutation)
            .await?;
        maybe_fail_native_mutation(
            NativeMutationKind::Compact,
            NativeMutationStage::BeforeCommit,
            schema,
            table,
        )?;
        writer
            .commit_table_mutation(plan.table_id, schema, table, plan.snapshot_id, &mutation)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        NATIVE_COMPACT_MUTATION_COUNTER.fetch_add(1, Ordering::Relaxed);
        Ok(Some(plan.affected_count))
    }

    async fn compact_bucket_row_count(
        &self,
        session_context: &SessionContext,
        table_ref: &str,
        time_bucket: i64,
        space_bucket: i64,
    ) -> PgWireResult<usize> {
        let time_col = quote_ident(layout::TIME_BUCKET);
        let space_col = quote_ident(layout::SPACE_BUCKET);
        let query = format!(
            "SELECT COUNT(*) FROM {table_ref} WHERE {time_col} = {time_bucket} AND {space_col} = {space_bucket}"
        );
        let (batches, _) = collect_normalized_query_batches(session_context, &query).await?;
        let batch = batches
            .first()
            .ok_or_else(|| user_error(anyhow!("compact bucket count returned no batches")))?;
        if batch.num_rows() != 1 || batch.num_columns() != 1 {
            return Err(user_error(anyhow!(
                "compact bucket count returned an unexpected shape"
            )));
        }
        let rows = scalar_i64_at(batch.column(0).as_ref(), 0).map_err(user_error)?;
        usize::try_from(rows).map_err(|e| user_error(anyhow!("compact bucket count overflow: {e}")))
    }

    async fn handle_copy_from_stdin(
        &self,
        statement: &Statement,
        session_context: &SessionContext,
        client: &dyn HookClient,
    ) -> PgWireResult<Response> {
        self.ensure_write_allowed(client, statement)?;
        let (table_name, columns, options, legacy_options, values) = match statement {
            Statement::Copy {
                source:
                    CopySource::Table {
                        table_name,
                        columns,
                    },
                to,
                target,
                options,
                legacy_options,
                values,
            } if !*to && matches!(target, CopyTarget::Stdin) => {
                (table_name, columns, options, legacy_options, values)
            }
            _ => {
                return Err(user_error(anyhow!(
                    "only COPY <table> [(columns)] FROM STDIN is supported"
                )));
            }
        };
        if !values.is_empty() {
            return Err(user_error(anyhow!(
                "COPY FROM inline VALUES is not supported; use COPY FROM STDIN"
            )));
        }
        let (schema, table) = table_name_parts(table_name).expect("guarded by caller");
        let options = parse_copy_text_options(options, legacy_options)?;
        let target_schema = self
            .copy_target_schema(session_context, client, &schema, &table)
            .await?;
        let copy_columns = copy_columns_for_request(target_schema.as_ref(), columns)?;
        let column_count = copy_columns.len();

        let state = copy_in_session_state(client);
        let mut copy = state.inner.lock().await;
        if copy.is_some() {
            return Err(user_error(anyhow!(
                "another COPY FROM STDIN operation is already active on this connection"
            )));
        }
        *copy = Some(CopyInRequest {
            schema,
            table,
            columns: copy_columns,
            options,
            data: Vec::new(),
        });

        Ok(Response::CopyIn(CopyResponse::new(
            0,
            column_count,
            futures::stream::empty(),
        )))
    }

    async fn append_copy_data<C>(&self, client: &C, data: &[u8]) -> PgWireResult<()>
    where
        C: ClientInfo + Send + Sync + ?Sized,
    {
        let state = copy_in_session_state(client);
        let mut copy = state.inner.lock().await;
        let copy = copy
            .as_mut()
            .ok_or_else(|| user_error(anyhow!("COPY data received without active COPY")))?;
        copy.data.extend_from_slice(data);
        Ok(())
    }

    async fn finish_copy_in<C>(
        &self,
        session_context: &SessionContext,
        client: &C,
    ) -> PgWireResult<usize>
    where
        C: ClientInfo + Send + Sync + ?Sized,
    {
        let state = copy_in_session_state(client);
        let request = {
            let mut copy = state.inner.lock().await;
            copy.take()
                .ok_or_else(|| user_error(anyhow!("COPY done received without active COPY")))?
        };

        if self.client_in_transaction(client).await {
            self.finish_copy_in_transaction(session_context, client, request)
                .await
        } else {
            self.finish_copy_in_autocommit(session_context, request)
                .await
        }
    }

    async fn abort_copy_in<C>(&self, client: &C)
    where
        C: ClientInfo + Send + Sync + ?Sized,
    {
        if let Some(state) = client.session_extensions().get::<CopyInSessionState>() {
            let mut copy = state.inner.lock().await;
            *copy = None;
        }
    }

    async fn copy_target_schema<C>(
        &self,
        session_context: &SessionContext,
        client: &C,
        schema: &str,
        table: &str,
    ) -> PgWireResult<SchemaRef>
    where
        C: ClientInfo + Send + Sync + ?Sized,
    {
        let key = TableKey {
            schema: schema.to_string(),
            table: table.to_string(),
        };
        let tx_state = client_transaction_state(client);
        {
            let state = tx_state.inner.lock().await;
            if let TransactionState::Active(active) = &*state
                && let Some(staged) = active.staged_tables.get(&key)
                && let Some(batch) = staged.batches.first()
            {
                return Ok(batch.schema());
            }
        }
        let table_ref = ducklake_table_ref(schema, table);
        self.table_schema(session_context, &table_ref).await
    }

    async fn finish_copy_in_autocommit(
        &self,
        session_context: &SessionContext,
        request: CopyInRequest,
    ) -> PgWireResult<usize> {
        let schema = request.schema.clone();
        let table = request.table.clone();
        let table_ref = ducklake_table_ref(&schema, &table);
        let target_schema = self.table_schema(session_context, &table_ref).await?;
        let next_rowid = if schema_has_synthetic_rowid(target_schema.as_ref()) {
            self.next_synthetic_rowid_from_table(session_context, &table_ref)
                .await?
        } else {
            1
        };
        let (batches, rows) = copy_request_to_batches(request, target_schema, next_rowid)?;
        if rows == 0 {
            return Ok(0);
        }
        self.write_batches(&schema, &table, &batches, WriteDisposition::Append)
            .await?;
        self.refresh_ducklake_catalog(session_context).await?;
        Ok(rows)
    }

    async fn finish_copy_in_transaction<C>(
        &self,
        session_context: &SessionContext,
        client: &C,
        request: CopyInRequest,
    ) -> PgWireResult<usize>
    where
        C: ClientInfo + Send + Sync + ?Sized,
    {
        let schema = request.schema.clone();
        let table = request.table.clone();
        let tx_state = client_transaction_state(client);
        let mut state = tx_state.inner.lock().await;
        let TransactionState::Active(active) = &mut *state else {
            return Err(user_error(anyhow!("transaction state is not active")));
        };
        let staged = self
            .ensure_staged_table(active, session_context, &schema, &table)
            .await?;
        let target_schema = staged
            .batches
            .first()
            .map(|batch| batch.schema())
            .ok_or_else(|| user_error(anyhow!("staged table must have a schema")))?;
        let next_rowid = if schema_has_synthetic_rowid(target_schema.as_ref()) {
            next_synthetic_rowid_from_batches(&staged.batches)?
        } else {
            1
        };
        let (new_batches, rows) = copy_request_to_batches(request, target_schema, next_rowid)?;
        if rows == 0 {
            return Ok(0);
        }
        let combined =
            Self::append_staged_batches(&staged.batches, new_batches).map_err(user_error)?;
        self.replace_staged_batches(session_context, staged, combined)?;
        Ok(rows)
    }

    async fn next_synthetic_rowid_from_table(
        &self,
        session_context: &SessionContext,
        table_ref: &str,
    ) -> PgWireResult<i64> {
        let rowid = quote_ident(SYNTHETIC_ROWID_COLUMN);
        let query = format!("SELECT COALESCE(MAX({rowid}), 0) AS max_rowid FROM {table_ref}");
        let batches = collect_query_batches(session_context, &query).await?;
        let Some(batch) = batches.first() else {
            return Ok(1);
        };
        if batch.num_rows() == 0 || batch.num_columns() == 0 || batch.column(0).is_null(0) {
            return Ok(1);
        }
        let max_rowid = scalar_i64_at(batch.column(0).as_ref(), 0).map_err(user_error)?;
        Ok(max_rowid + 1)
    }

    async fn handle_insert(
        &self,
        insert: &datafusion::sql::sqlparser::ast::Insert,
        session_context: &SessionContext,
        result_format: Format,
        params: Option<&ParamValues>,
        client: &dyn HookClient,
    ) -> PgWireResult<Response> {
        let (schema, table) = insert_target_parts(&insert.table).expect("guarded by caller");
        let source_query = insert
            .source
            .as_ref()
            .expect("guarded by caller")
            .to_string();
        let source_query = inline_params_if_needed(&source_query, params)?;
        let source_query = rewrite_pg_escape_bytea_literals(&source_query);
        let source_query = rewrite_st_geomfromwkb_zero_srid_literals(&source_query);
        let source_query = rewrite_mojibake_string_literals(&source_query);
        if self.client_in_transaction(client).await {
            return self
                .handle_insert_transactional(
                    insert,
                    session_context,
                    result_format,
                    client,
                    schema,
                    table,
                    source_query,
                )
                .await;
        }
        let query = if insert.columns.is_empty()
            && !insert_source_is_values(insert.source.as_ref().expect("guarded by caller"))
        {
            source_query
        } else {
            self.insert_source_with_target_schema(
                session_context,
                &schema,
                &table,
                &insert.columns,
                &source_query,
            )
            .await?
        };
        let returning_batches = if let Some(returning) = insert.returning.as_deref() {
            let returning_query = returning_query_from_source(&query, returning);
            Some(collect_query_batches(session_context, &returning_query).await?)
        } else {
            None
        };
        let rows = self
            .write_query(
                session_context,
                &query,
                &schema,
                &table,
                WriteDisposition::Append,
            )
            .await?;
        if let Some(batches) = returning_batches {
            return query_response_from_batches_with_format(batches, result_format)
                .map(Response::Query);
        }
        Ok(Response::Execution(Tag::new(&format!("INSERT 0 {rows}"))))
    }

    async fn handle_alter_table(
        &self,
        alter: &AlterTable,
        session_context: &SessionContext,
        client: &dyn HookClient,
    ) -> PgWireResult<Response> {
        if self.client_in_transaction(client).await {
            return self
                .handle_alter_table_transactional(alter, session_context, client)
                .await;
        }
        let (schema, table) = table_name_parts(&alter.name).expect("guarded by caller");
        for operation in &alter.operations {
            match operation {
                AlterTableOperation::AddColumn {
                    if_not_exists,
                    column_def,
                    ..
                } => {
                    self.add_column(session_context, &schema, &table, column_def, *if_not_exists)
                        .await?;
                }
                other => {
                    return Err(user_error(anyhow!(
                        "unsupported ALTER TABLE operation for {schema}.{table}: {other}"
                    )));
                }
            }
        }
        Ok(Response::Execution(Tag::new("ALTER TABLE")))
    }

    async fn handle_delete(
        &self,
        delete: &datafusion::sql::sqlparser::ast::Delete,
        session_context: &SessionContext,
        result_format: Format,
        params: Option<&ParamValues>,
        client: &dyn HookClient,
    ) -> PgWireResult<Response> {
        let (schema, table) = delete_target_parts(delete).expect("guarded by caller");
        if self.client_in_transaction(client).await {
            return self
                .handle_delete_transactional(
                    delete,
                    session_context,
                    result_format,
                    params,
                    client,
                    schema,
                    table,
                )
                .await;
        }
        let table_ref =
            format!("{DUCKLAKE_CATALOG}.{}.", quote_ident(&schema)) + &quote_ident(&table);
        let predicate = delete
            .selection
            .as_ref()
            .map(|e| inline_params_if_needed(&e.to_string(), params))
            .transpose()?;
        let where_clause = predicate
            .as_ref()
            .map(|predicate| format!("NOT ({predicate})"))
            .unwrap_or_else(|| "FALSE".to_string());
        let returning_batches = if let Some(returning) = delete.returning.as_deref() {
            let predicate = predicate.clone().unwrap_or_else(|| "TRUE".to_string());
            let returning_query =
                returning_query_from_table(&table_ref, returning, Some(&predicate));
            Some(collect_query_batches(session_context, &returning_query).await?)
        } else {
            None
        };
        if let Some(deleted) = self
            .try_native_delete(session_context, &schema, &table, predicate.as_deref())
            .await?
        {
            self.refresh_ducklake_catalog(session_context).await?;
            if let Some(batches) = returning_batches {
                return query_response_from_batches_with_format(batches, result_format)
                    .map(Response::Query);
            }
            return Ok(Response::Execution(Tag::new(&format!("DELETE {deleted}"))));
        }
        let query = format!("SELECT * FROM {table_ref} WHERE {where_clause}");
        let remaining = self
            .write_query(
                session_context,
                &query,
                &schema,
                &table,
                WriteDisposition::Replace,
            )
            .await?;
        if let Some(batches) = returning_batches {
            return query_response_from_batches_with_format(batches, result_format)
                .map(Response::Query);
        }
        Ok(Response::Execution(Tag::new(&format!(
            "DELETE {remaining}"
        ))))
    }

    async fn plan_native_rows(
        &self,
        session_context: &SessionContext,
        schema: &str,
        table: &str,
        predicate: Option<&str>,
        purpose: &str,
    ) -> PgWireResult<Option<NativeRowPlan>> {
        let provider = self
            .paths
            .metadata_provider()
            .await
            .map_err(storage_api_error)?;
        let snapshot_id = provider
            .get_current_snapshot()
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let schema_meta = provider
            .get_schema_by_name(schema, snapshot_id)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?
            .ok_or_else(|| user_error(anyhow!("schema not found: {schema}")))?;
        let table_meta = provider
            .get_table_by_name(schema_meta.schema_id, table, snapshot_id)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?
            .ok_or_else(|| user_error(anyhow!("table not found: {schema}.{table}")))?;
        let files = provider
            .get_table_files_for_select(table_meta.table_id, snapshot_id)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        if files.is_empty() {
            return Ok(Some(NativeRowPlan {
                snapshot_id,
                table_id: table_meta.table_id,
                files,
                positions_by_file: HashMap::new(),
                affected_count: 0,
                rowid_context: SessionContext::new_with_state(session_context.state()),
                catalog_name: String::new(),
            }));
        }
        if files
            .iter()
            .any(|file| file.row_id_start.is_none() || file.max_row_count.is_none())
        {
            return Ok(None);
        }

        let rowid_catalog = DuckLakeCatalog::with_snapshot(Arc::clone(&provider), snapshot_id)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?
            .with_row_lineage(true);
        let rowid_context = SessionContext::new_with_state(session_context.state());
        let catalog_name = format!(
            "__quackgis_native_{purpose}_{}",
            QUERY_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        rowid_context.register_catalog(&catalog_name, Arc::new(rowid_catalog));
        let rowid_table_ref = format!(
            "{}.{}.{}",
            quote_ident(&catalog_name),
            quote_ident(schema),
            quote_ident(table)
        );
        let where_clause = predicate
            .map(|predicate| format!(" WHERE {predicate}"))
            .unwrap_or_default();
        let rowid_query = format!(
            "SELECT {} FROM {rowid_table_ref}{where_clause}",
            quote_ident("rowid")
        );
        let rowid_batches = match collect_query_batches(&rowid_context, &rowid_query).await {
            Ok(batches) => batches,
            Err(err) => {
                log::debug!(
                    "native {purpose} rowid planning failed for {schema}.{table}; falling back to rewrite: {err}"
                );
                return Ok(None);
            }
        };
        let mut rowids = Vec::new();
        for batch in &rowid_batches {
            if batch.num_columns() != 1 {
                return Ok(None);
            }
            for row in 0..batch.num_rows() {
                if batch.column(0).is_null(row) {
                    return Ok(None);
                }
                rowids.push(scalar_i64_at(batch.column(0).as_ref(), row).map_err(user_error)?);
            }
        }
        if rowids.is_empty() {
            return Ok(Some(NativeRowPlan {
                snapshot_id,
                table_id: table_meta.table_id,
                files,
                positions_by_file: HashMap::new(),
                affected_count: 0,
                rowid_context,
                catalog_name,
            }));
        }
        let affected_count = rowids.len();

        let mut positions_by_file: HashMap<i64, HashSet<i64>> = HashMap::new();
        for rowid in rowids {
            let mut matched = false;
            for file in &files {
                let start = file.row_id_start.expect("checked above");
                let count = file.max_row_count.expect("checked above");
                if rowid >= start && rowid < start.saturating_add(count) {
                    positions_by_file
                        .entry(file.data_file_id)
                        .or_default()
                        .insert(rowid - start);
                    matched = true;
                    break;
                }
            }
            if !matched {
                log::debug!(
                    "native {purpose} could not map rowid {rowid} to a live data file for {schema}.{table}; falling back to rewrite"
                );
                return Ok(None);
            }
        }

        Ok(Some(NativeRowPlan {
            snapshot_id,
            table_id: table_meta.table_id,
            files,
            positions_by_file,
            affected_count,
            rowid_context,
            catalog_name,
        }))
    }

    async fn add_native_delete_files_to_mutation(
        &self,
        plan: &mut NativeRowPlan,
        schema: &str,
        table: &str,
        table_writer: &DuckLakeTableWriter,
        mut mutation: TableMutation,
    ) -> PgWireResult<TableMutation> {
        let table_provider = plan
            .rowid_context
            .catalog(&plan.catalog_name)
            .and_then(|catalog| catalog.schema(schema))
            .ok_or_else(|| user_error(anyhow!("native DML rowid catalog lookup failed")))?
            .table(table)
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?
            .ok_or_else(|| user_error(anyhow!("native DML rowid table lookup failed")))?;
        let ducklake_table = (table_provider.as_ref() as &dyn std::any::Any)
            .downcast_ref::<DuckLakeTable>()
            .ok_or_else(|| user_error(anyhow!("native DML expected a DuckLake table")))?;

        let state = plan.rowid_context.state();
        for file in &plan.files {
            let Some(positions) = plan.positions_by_file.get_mut(&file.data_file_id) else {
                continue;
            };
            if let Some(delete_file) = file.delete_file.as_ref() {
                let prior = ducklake_table
                    .read_delete_file_positions(&state, delete_file)
                    .await
                    .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
                positions.extend(prior);
            }
            let mut positions = positions.iter().copied().collect::<Vec<_>>();
            positions.sort_unstable();
            let delete_info = table_writer
                .write_delete_file(schema, table, &file.file.path, &positions)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
            mutation = mutation.set_delete_file(DeleteFileMutation::new(
                file.data_file_id,
                file.delete_file_id,
                delete_info,
            ));
        }
        Ok(mutation)
    }

    async fn try_native_delete(
        &self,
        session_context: &SessionContext,
        schema: &str,
        table: &str,
        predicate: Option<&str>,
    ) -> PgWireResult<Option<usize>> {
        let Some(mut plan) = self
            .plan_native_rows(session_context, schema, table, predicate, "delete")
            .await?
        else {
            return Ok(None);
        };
        if plan.affected_count == 0 {
            return Ok(Some(0));
        }

        let writer = self.storage_writer().await?;
        let object_store = self.storage_object_store()?;
        let table_writer = configured_ducklake_table_writer(Arc::clone(&writer), object_store)?;
        let mutation = self
            .add_native_delete_files_to_mutation(
                &mut plan,
                schema,
                table,
                &table_writer,
                TableMutation::new(),
            )
            .await?;
        if mutation.is_empty() {
            return Ok(Some(0));
        }
        maybe_fail_native_mutation(
            NativeMutationKind::Delete,
            NativeMutationStage::BeforeCommit,
            schema,
            table,
        )?;
        writer
            .commit_table_mutation(plan.table_id, schema, table, plan.snapshot_id, &mutation)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        NATIVE_DELETE_MUTATION_COUNTER.fetch_add(1, Ordering::Relaxed);
        Ok(Some(plan.affected_count))
    }

    async fn try_native_update(
        &self,
        session_context: &SessionContext,
        schema: &str,
        table: &str,
        predicate: Option<&str>,
        select_items: &[String],
    ) -> PgWireResult<Option<usize>> {
        let Some(mut plan) = self
            .plan_native_rows(session_context, schema, table, predicate, "update")
            .await?
        else {
            return Ok(None);
        };
        if plan.affected_count == 0 {
            return Ok(Some(0));
        }

        let rowid_table_ref = format!(
            "{}.{}.{}",
            quote_ident(&plan.catalog_name),
            quote_ident(schema),
            quote_ident(table)
        );
        let where_clause = predicate
            .map(|predicate| format!(" WHERE {predicate}"))
            .unwrap_or_default();
        let updated_query = format!(
            "SELECT {} FROM {rowid_table_ref}{where_clause}",
            select_items.join(", ")
        );
        let updated_batches = match collect_query_batches(&plan.rowid_context, &updated_query).await
        {
            Ok(batches) => batches,
            Err(err) => {
                log::debug!(
                    "native update replacement-row planning failed for {schema}.{table}; falling back to rewrite: {err}"
                );
                return Ok(None);
            }
        };
        let updated_rows = updated_batches.iter().map(|b| b.num_rows()).sum::<usize>();
        if updated_rows != plan.affected_count || updated_batches.is_empty() {
            log::debug!(
                "native update planned {} rowids but {} replacement rows for {schema}.{table}; falling back to rewrite",
                plan.affected_count,
                updated_rows
            );
            return Ok(None);
        }
        let updated_batches =
            normalize_batches_for_ducklake(updated_batches).map_err(user_error)?;
        let updated_batches = layout::project_batches(updated_batches).map_err(user_error)?;
        let updated_batches =
            layout::sort_batches_by_layout(updated_batches).map_err(user_error)?;

        let writer = self.storage_writer().await?;
        let object_store = self.storage_object_store()?;
        let table_writer = configured_ducklake_table_writer(Arc::clone(&writer), object_store)?;
        let pending_file = table_writer
            .write_pending_data_file(schema, table, &updated_batches)
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let mutation = TableMutation::new().append_data_file(pending_file);
        let mutation = self
            .add_native_delete_files_to_mutation(&mut plan, schema, table, &table_writer, mutation)
            .await?;
        maybe_fail_native_mutation(
            NativeMutationKind::Update,
            NativeMutationStage::BeforeCommit,
            schema,
            table,
        )?;
        writer
            .commit_table_mutation(plan.table_id, schema, table, plan.snapshot_id, &mutation)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        NATIVE_UPDATE_MUTATION_COUNTER.fetch_add(1, Ordering::Relaxed);
        Ok(Some(plan.affected_count))
    }

    async fn handle_update(
        &self,
        update: &datafusion::sql::sqlparser::ast::Update,
        result_format: Format,
        params: Option<&ParamValues>,
        session_context: &SessionContext,
        client: &dyn HookClient,
    ) -> PgWireResult<Response> {
        let (schema, table_name) = update_target_parts(&update.table).expect("guarded by caller");
        if self.client_in_transaction(client).await {
            return self
                .handle_update_transactional(
                    update,
                    result_format,
                    params,
                    session_context,
                    client,
                    schema,
                    table_name,
                )
                .await;
        }
        let table_ref =
            format!("{DUCKLAKE_CATALOG}.{}.", quote_ident(&schema)) + &quote_ident(&table_name);
        let schema_ref = self.table_schema(session_context, &table_ref).await?;
        let mut assignment_map = std::collections::HashMap::new();
        for assignment in &update.assignments {
            let AssignmentTarget::ColumnName(name) = &assignment.target else {
                return Err(user_error(anyhow!(
                    "tuple UPDATE assignments are not supported yet"
                )));
            };
            let col = object_name_last(name)
                .ok_or_else(|| user_error(anyhow!("invalid UPDATE target")))?;
            let value = inline_params_if_needed(&assignment.value.to_string(), params)?;
            let value = rewrite_st_geomfromwkb_zero_srid_literals(&value);
            assignment_map.insert(col, value);
        }
        let predicate = update
            .selection
            .as_ref()
            .map(|e| inline_params_if_needed(&e.to_string(), params))
            .transpose()?;
        let mut select_items = Vec::new();
        for field in schema_ref.fields() {
            let col = field.name();
            let expr = if let Some(value) = assignment_map.get(col) {
                let sql_type = arrow_type_to_sql(field.data_type()).map_err(user_error)?;
                if let Some(pred) = &predicate {
                    format!(
                        "CAST(CASE WHEN {pred} THEN {value} ELSE {} END AS {sql_type}) AS {}",
                        quote_ident(col),
                        quote_ident(col)
                    )
                } else {
                    format!("CAST({value} AS {sql_type}) AS {}", quote_ident(col))
                }
            } else {
                quote_ident(col)
            };
            select_items.push(expr);
        }
        let query = format!("SELECT {} FROM {table_ref}", select_items.join(", "));
        let returning_batches = if let Some(returning) = update.returning.as_deref() {
            let source_query = if let Some(pred) = &predicate {
                format!(
                    "SELECT {} FROM {table_ref} WHERE {pred}",
                    select_items.join(", ")
                )
            } else {
                query.clone()
            };
            let returning_query = returning_query_from_source(&source_query, returning);
            Some(collect_query_batches(session_context, &returning_query).await?)
        } else {
            None
        };
        if let Some(updated) = self
            .try_native_update(
                session_context,
                &schema,
                &table_name,
                predicate.as_deref(),
                &select_items,
            )
            .await?
        {
            self.refresh_ducklake_catalog(session_context).await?;
            if let Some(batches) = returning_batches {
                return query_response_from_batches_with_format(batches, result_format)
                    .map(Response::Query);
            }
            return Ok(Response::Execution(Tag::new(&format!("UPDATE {updated}"))));
        }

        let rows = self
            .write_query(
                session_context,
                &query,
                &schema,
                &table_name,
                WriteDisposition::Replace,
            )
            .await?;
        if let Some(batches) = returning_batches {
            return query_response_from_batches_with_format(batches, result_format)
                .map(Response::Query);
        }
        Ok(Response::Execution(Tag::new(&format!("UPDATE {rows}"))))
    }

    async fn create_empty_table(
        &self,
        schema: &str,
        table: &str,
        columns: &[ColumnDef],
    ) -> PgWireResult<()> {
        if columns.is_empty() {
            return Err(user_error(anyhow!(
                "CREATE TABLE requires at least one column"
            )));
        }
        let fields = columns
            .iter()
            .map(sql_type_to_arrow_field)
            .collect::<Result<Vec<_>>>()
            .map_err(user_error)?;
        let fields = fields_with_synthetic_rowid_if_needed(fields);
        let batch = empty_batch_for_fields(fields).map_err(user_error)?;
        self.write_batches(schema, table, &[batch], WriteDisposition::Replace)
            .await
            .map(|_| ())
    }

    async fn add_column(
        &self,
        session_context: &SessionContext,
        schema: &str,
        table: &str,
        column_def: &ColumnDef,
        if_not_exists: bool,
    ) -> PgWireResult<()> {
        let table_ref = ducklake_table_ref(schema, table);
        let schema_ref = self.table_schema(session_context, &table_ref).await?;
        let new_field = sql_type_to_arrow_field(column_def).map_err(user_error)?;
        if schema_ref
            .fields()
            .iter()
            .any(|field| field.name() == new_field.name())
        {
            if if_not_exists {
                return Ok(());
            }
            return Err(user_error(anyhow!(
                "column already exists: {}",
                new_field.name()
            )));
        }

        let mut select_items = schema_ref
            .fields()
            .iter()
            .map(|field| quote_ident(field.name()))
            .collect::<Vec<_>>();
        select_items.push(format!(
            "CAST(NULL AS {}) AS {}",
            arrow_type_to_sql(new_field.data_type()).map_err(user_error)?,
            quote_ident(new_field.name())
        ));
        let query = format!("SELECT {} FROM {table_ref}", select_items.join(", "));
        let batches = session_context
            .sql(&query)
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?
            .collect()
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let mut batches = normalize_batches_for_ducklake(batches).map_err(user_error)?;
        if batches.is_empty() {
            let mut fields = schema_ref
                .fields()
                .iter()
                .map(|field| field.as_ref().clone())
                .collect::<Vec<_>>();
            fields.push(new_field);
            batches.push(empty_batch_for_fields(fields).map_err(user_error)?);
        }
        self.write_batches(schema, table, &batches, WriteDisposition::Replace)
            .await?;
        self.refresh_ducklake_catalog(session_context).await?;
        Ok(())
    }

    async fn insert_source_with_target_schema(
        &self,
        session_context: &SessionContext,
        schema: &str,
        table: &str,
        insert_columns: &[ObjectName],
        source_query: &str,
    ) -> PgWireResult<String> {
        let table_ref = ducklake_table_ref(schema, table);
        let schema_ref = self.table_schema(session_context, &table_ref).await?;
        self.insert_source_with_schema(&table_ref, &schema_ref, insert_columns, source_query)
    }

    fn insert_source_with_schema(
        &self,
        table_ref: &str,
        schema_ref: &SchemaRef,
        insert_columns: &[ObjectName],
        source_query: &str,
    ) -> PgWireResult<String> {
        let mut insert_positions = std::collections::HashMap::new();
        if insert_columns.is_empty() {
            // INSERT INTO table VALUES (...) yields DataFusion columns named
            // column1, column2, ... . Alias them back to the target table schema
            // so Parquet/DuckLake persists the real column names.
            let mut source_idx = 1_usize;
            for field in schema_ref.fields() {
                if field.name().eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN)
                    || layout::is_layout_column(field.name())
                {
                    continue;
                }
                insert_positions.insert(field.name().clone(), source_idx);
                source_idx += 1;
            }
        } else {
            for (idx, name) in insert_columns.iter().enumerate() {
                let col = object_name_last(name)
                    .ok_or_else(|| user_error(anyhow!("invalid INSERT column")))?;
                if col.eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN)
                    || layout::is_layout_column(&col)
                {
                    continue;
                }
                insert_positions.insert(col, idx + 1); // VALUES columns are column1, column2, ...
            }
        }
        let mut items = Vec::new();
        for field in schema_ref.fields() {
            let col = field.name();
            let expr = if let Some(pos) = insert_positions.get(col) {
                format!(
                    "CAST(column{pos} AS {}) AS {}",
                    arrow_type_to_sql(field.data_type()).map_err(user_error)?,
                    quote_ident(col)
                )
            } else if col.eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN) {
                format!(
                    "CAST((SELECT COALESCE(MAX({rowid}), 0) FROM {table_ref}) + \
                     ROW_NUMBER() OVER () AS BIGINT) AS {rowid}",
                    rowid = quote_ident(SYNTHETIC_ROWID_COLUMN)
                )
            } else {
                format!(
                    "CAST(NULL AS {}) AS {}",
                    arrow_type_to_sql(field.data_type()).map_err(user_error)?,
                    quote_ident(col)
                )
            };
            items.push(expr);
        }
        Ok(format!(
            "SELECT {} FROM ({source_query}) AS v",
            items.join(", ")
        ))
    }

    async fn write_query(
        &self,
        session_context: &SessionContext,
        query: &str,
        schema: &str,
        table: &str,
        disposition: WriteDisposition,
    ) -> PgWireResult<usize> {
        let batches = session_context
            .sql(query)
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let output_schema = Arc::new(batches.schema().as_arrow().clone());
        let add_rowid = matches!(disposition, WriteDisposition::Replace)
            && needs_synthetic_rowid_for_schema(batches.schema().as_arrow());
        let mut batches = batches
            .collect()
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        if batches.is_empty() {
            let fields = output_schema
                .fields()
                .iter()
                .map(|field| field.as_ref().clone())
                .collect::<Vec<_>>();
            batches.push(empty_batch_for_fields(fields).map_err(user_error)?);
        }
        let batches = normalize_batches_for_ducklake(batches).map_err(user_error)?;
        let batches = if add_rowid {
            prepend_synthetic_rowid_to_batches(batches).map_err(user_error)?
        } else {
            batches
        };

        self.write_batches(schema, table, &batches, disposition)
            .await?;
        self.refresh_ducklake_catalog(session_context).await?;
        Ok(rows)
    }

    async fn write_batches(
        &self,
        schema: &str,
        table: &str,
        batches: &[RecordBatch],
        disposition: WriteDisposition,
    ) -> PgWireResult<usize> {
        let batches = if matches!(disposition, WriteDisposition::Replace) {
            layout::ensure_columns_for_spatial_batches(batches.to_vec()).map_err(user_error)?
        } else {
            batches.to_vec()
        };
        let batches = layout::project_batches(batches).map_err(user_error)?;
        let batches = layout::sort_batches_by_layout(batches).map_err(user_error)?;
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        let writer = self.storage_writer().await?;
        let snapshot = writer
            .create_snapshot()
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        writer
            .get_or_create_schema(schema, None, snapshot)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let object_store = self.storage_object_store()?;
        let table_writer = configured_ducklake_table_writer(writer, object_store)?;
        match disposition {
            WriteDisposition::Replace => table_writer
                .write_table(schema, table, &batches)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
            WriteDisposition::Append => table_writer
                .append_table(schema, table, &batches)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
        };
        Ok(rows)
    }

    async fn table_schema(
        &self,
        session_context: &SessionContext,
        table_ref: &str,
    ) -> PgWireResult<SchemaRef> {
        let df = session_context
            .sql(&format!("SELECT * FROM {table_ref} LIMIT 0"))
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        Ok(Arc::new(df.schema().as_arrow().clone()))
    }

    async fn refresh_ducklake_catalog(&self, session_context: &SessionContext) -> PgWireResult<()> {
        let writer = self.storage_writer().await?;
        let provider = self
            .paths
            .metadata_provider()
            .await
            .map_err(storage_api_error)?;
        let ducklake = DuckLakeCatalog::with_writer(provider, writer)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        session_context.register_catalog(DUCKLAKE_CATALOG, Arc::new(ducklake));
        crate::public_schema::register_public_schema_alias(session_context)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        CATALOG_REFRESH_COUNTER.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn refresh_ducklake_catalog_for_read(
        &self,
        session_context: &SessionContext,
    ) -> PgWireResult<()> {
        let mut last_refresh = self.shared_catalog_refresh.last_refresh.lock().await;
        if self
            .shared_catalog_refresh
            .refresh_is_recent(Instant::now(), *last_refresh)
        {
            return Ok(());
        }

        self.refresh_ducklake_catalog(session_context).await?;
        SHARED_CATALOG_READ_REFRESH_COUNTER.fetch_add(1, Ordering::Relaxed);
        *last_refresh = Some(Instant::now());
        Ok(())
    }

    async fn refresh_ducklake_catalog_strong(
        &self,
        session_context: &SessionContext,
    ) -> PgWireResult<()> {
        let mut last_refresh = self.shared_catalog_refresh.last_refresh.lock().await;
        self.refresh_ducklake_catalog(session_context).await?;
        SHARED_CATALOG_STRONG_REFRESH_COUNTER.fetch_add(1, Ordering::Relaxed);
        *last_refresh = Some(Instant::now());
        Ok(())
    }

    async fn refresh_shared_catalog(
        &self,
        statement: &datafusion::sql::sqlparser::ast::Statement,
        session_context: &SessionContext,
    ) -> PgWireResult<()> {
        if !self.paths.is_shared_catalog() {
            return Ok(());
        }

        if needs_strong_shared_catalog_refresh(statement) {
            self.refresh_ducklake_catalog_strong(session_context)
                .await?;
        } else if is_read_statement(statement) {
            self.refresh_ducklake_catalog_for_read(session_context)
                .await?;
        }
        Ok(())
    }

    async fn storage_writer(&self) -> PgWireResult<Arc<dyn MetadataWriter>> {
        self.paths
            .metadata_writer()
            .await
            .map_err(storage_api_error)
    }

    fn storage_object_store(&self) -> PgWireResult<Arc<dyn object_store::ObjectStore>> {
        self.paths.object_store().map_err(storage_api_error)
    }
}

fn storage_api_error(err: anyhow::Error) -> PgWireError {
    PgWireError::ApiError(Box::new(std::io::Error::other(err.to_string())))
}

fn is_read_statement(statement: &datafusion::sql::sqlparser::ast::Statement) -> bool {
    match statement {
        datafusion::sql::sqlparser::ast::Statement::Query(_) => true,
        datafusion::sql::sqlparser::ast::Statement::Explain { statement, .. } => {
            is_read_statement(statement)
        }
        _ => false,
    }
}

fn needs_strong_shared_catalog_refresh(
    statement: &datafusion::sql::sqlparser::ast::Statement,
) -> bool {
    ducklake_statement_parts(statement).is_some()
        || matches!(statement, datafusion::sql::sqlparser::ast::Statement::Call(function) if is_compact_call(function))
}

#[derive(Debug, Clone, Copy)]
enum WriteDisposition {
    Replace,
    Append,
}

fn parse_copy_text_options(
    options: &[CopyOption],
    legacy_options: &[CopyLegacyOption],
) -> PgWireResult<CopyTextOptions> {
    let mut parsed = CopyTextOptions::default();
    for option in options {
        match option {
            CopyOption::Format(format) if format.value.eq_ignore_ascii_case("text") => {}
            CopyOption::Format(format) => {
                return Err(user_error(anyhow!(
                    "unsupported COPY format {}; only text COPY FROM STDIN is supported",
                    format.value
                )));
            }
            CopyOption::Delimiter(delimiter) => {
                parsed.delimiter = copy_delimiter_to_byte(*delimiter)?;
            }
            CopyOption::Null(null) => {
                parsed.null = null.as_bytes().to_vec();
            }
            CopyOption::Header(header) => {
                parsed.header = *header;
            }
            CopyOption::Encoding(encoding) if encoding.eq_ignore_ascii_case("utf8") => {}
            CopyOption::Encoding(encoding) if encoding.eq_ignore_ascii_case("utf-8") => {}
            other => {
                return Err(user_error(anyhow!(
                    "unsupported COPY option for text COPY FROM STDIN: {other}"
                )));
            }
        }
    }
    for option in legacy_options {
        match option {
            CopyLegacyOption::Delimiter(delimiter) => {
                parsed.delimiter = copy_delimiter_to_byte(*delimiter)?;
            }
            CopyLegacyOption::Null(null) => {
                parsed.null = null.as_bytes().to_vec();
            }
            CopyLegacyOption::Header => {
                parsed.header = true;
            }
            CopyLegacyOption::Csv(_) | CopyLegacyOption::Binary => {
                return Err(user_error(anyhow!(
                    "unsupported COPY option {option}; only text COPY FROM STDIN is supported"
                )));
            }
            other => {
                return Err(user_error(anyhow!(
                    "unsupported COPY option for text COPY FROM STDIN: {other}"
                )));
            }
        }
    }
    Ok(parsed)
}

fn copy_delimiter_to_byte(delimiter: char) -> PgWireResult<u8> {
    if delimiter.len_utf8() != 1 {
        return Err(user_error(anyhow!(
            "COPY text delimiter must be a single-byte character"
        )));
    }
    Ok(delimiter as u8)
}

fn copy_columns_for_request(schema: &Schema, columns: &[Ident]) -> PgWireResult<Vec<String>> {
    if columns.is_empty() {
        return Ok(schema
            .fields()
            .iter()
            .filter(|field| !is_internal_copy_column(field.name()))
            .map(|field| field.name().clone())
            .collect());
    }

    let mut out: Vec<String> = Vec::with_capacity(columns.len());
    for ident in columns {
        let requested = ident.value.clone();
        if is_internal_copy_column(&requested) {
            return Err(user_error(anyhow!(
                "COPY cannot write internal QuackGIS column {requested}"
            )));
        }
        let field = schema
            .fields()
            .iter()
            .find(|field| field.name().eq_ignore_ascii_case(&requested))
            .ok_or_else(|| user_error(anyhow!("COPY column does not exist: {requested}")))?;
        if is_internal_copy_column(field.name()) {
            return Err(user_error(anyhow!(
                "COPY cannot write internal QuackGIS column {}",
                field.name()
            )));
        }
        if out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(field.name()))
        {
            return Err(user_error(anyhow!(
                "COPY column specified more than once: {}",
                field.name()
            )));
        }
        out.push(field.name().clone());
    }
    Ok(out)
}

fn is_internal_copy_column(name: &str) -> bool {
    name.eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN) || layout::is_layout_column(name)
}

fn schema_has_synthetic_rowid(schema: &Schema) -> bool {
    schema
        .fields()
        .iter()
        .any(|field| field.name().eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN))
}

fn next_synthetic_rowid_from_batches(batches: &[RecordBatch]) -> PgWireResult<i64> {
    let mut max_rowid = 0_i64;
    for batch in batches {
        let Some(index) = batch
            .schema()
            .fields()
            .iter()
            .position(|field| field.name().eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN))
        else {
            continue;
        };
        let column = batch.column(index).as_ref();
        for row in 0..batch.num_rows() {
            if !column.is_null(row) {
                max_rowid = max_rowid.max(scalar_i64_at(column, row).map_err(user_error)?);
            }
        }
    }
    Ok(max_rowid + 1)
}

fn copy_request_to_batches(
    request: CopyInRequest,
    target_schema: SchemaRef,
    next_rowid: i64,
) -> PgWireResult<(Vec<RecordBatch>, usize)> {
    let rows =
        parse_copy_text_rows(&request.data, request.options.delimiter).map_err(user_error)?;
    let rows = materialize_copy_rows(rows, request.options.header);
    for (idx, row) in rows.iter().enumerate() {
        if row.len() != request.columns.len() {
            return Err(user_error(anyhow!(
                "COPY row {} has {} fields but {} columns were expected",
                idx + 1,
                row.len(),
                request.columns.len()
            )));
        }
    }
    let row_count = rows.len();
    if row_count == 0 {
        return Ok((Vec::new(), 0));
    }

    let mut source_by_target = vec![None; target_schema.fields().len()];
    for (source_idx, column) in request.columns.iter().enumerate() {
        let target_idx = target_schema
            .fields()
            .iter()
            .position(|field| field.name().eq_ignore_ascii_case(column))
            .ok_or_else(|| user_error(anyhow!("COPY column does not exist: {column}")))?;
        source_by_target[target_idx] = Some(source_idx);
    }

    let mut arrays = Vec::with_capacity(target_schema.fields().len());
    for (target_idx, field) in target_schema.fields().iter().enumerate() {
        let array = if field.name().eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN)
            && source_by_target[target_idx].is_none()
        {
            Arc::new(Int64Array::from(
                (0..row_count)
                    .map(|row| Some(next_rowid + row as i64))
                    .collect::<Vec<_>>(),
            )) as ArrayRef
        } else if let Some(source_idx) = source_by_target[target_idx] {
            copy_source_array(field.as_ref(), &rows, source_idx, &request.options)?
        } else {
            new_null_array(field.data_type(), row_count)
        };
        arrays.push(array);
    }

    let batch = RecordBatch::try_new(target_schema, arrays)
        .map_err(|e| user_error(anyhow!("building COPY RecordBatch: {e}")))?;
    Ok((vec![batch], row_count))
}

fn materialize_copy_rows(rows: Vec<Vec<Vec<u8>>>, header: bool) -> Vec<Vec<Vec<u8>>> {
    let mut out = Vec::with_capacity(rows.len());
    for (idx, row) in rows.into_iter().enumerate() {
        if header && idx == 0 {
            continue;
        }
        if row.len() == 1 && row[0] == b"\\." {
            break;
        }
        out.push(row);
    }
    out
}

fn parse_copy_text_rows(data: &[u8], delimiter: u8) -> Result<Vec<Vec<Vec<u8>>>> {
    let mut rows = Vec::new();
    let mut row: Vec<Vec<u8>> = Vec::new();
    let mut field: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < data.len() {
        match data[i] {
            b if b == delimiter => {
                row.push(std::mem::take(&mut field));
            }
            b'\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            b'\r' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                if data.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
            }
            b'\\' => {
                field.push(b'\\');
                if let Some(next) = data.get(i + 1) {
                    i += 1;
                    field.push(*next);
                }
            }
            b => field.push(b),
        }
        i += 1;
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn copy_source_array(
    field: &Field,
    rows: &[Vec<Vec<u8>>],
    source_idx: usize,
    options: &CopyTextOptions,
) -> PgWireResult<ArrayRef> {
    match field.data_type() {
        DataType::Int32 => rows
            .iter()
            .map(|row| parse_copy_i32(&row[source_idx], options))
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(Int32Array::from(values)) as ArrayRef),
        DataType::Int64 => rows
            .iter()
            .map(|row| parse_copy_i64(&row[source_idx], options))
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(Int64Array::from(values)) as ArrayRef),
        DataType::Float64 => rows
            .iter()
            .map(|row| parse_copy_f64(&row[source_idx], options))
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(Float64Array::from(values)) as ArrayRef),
        DataType::Boolean => rows
            .iter()
            .map(|row| parse_copy_bool(&row[source_idx], options))
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(BooleanArray::from(values)) as ArrayRef),
        DataType::Utf8 => {
            let values = rows
                .iter()
                .map(|row| parse_copy_string(&row[source_idx], options))
                .collect::<PgWireResult<Vec<_>>>()?;
            let refs = values
                .iter()
                .map(|value| value.as_deref())
                .collect::<Vec<_>>();
            Ok(Arc::new(StringArray::from(refs)) as ArrayRef)
        }
        DataType::Binary => {
            let values = rows
                .iter()
                .map(|row| parse_copy_bytea(&row[source_idx], options))
                .collect::<PgWireResult<Vec<_>>>()?;
            let refs = values
                .iter()
                .map(|value| value.as_deref())
                .collect::<Vec<_>>();
            Ok(Arc::new(BinaryArray::from(refs)) as ArrayRef)
        }
        other => Err(user_error(anyhow!(
            "unsupported COPY target column type for {}: {other}",
            field.name()
        ))),
    }
}

fn parse_copy_i32(raw: &[u8], options: &CopyTextOptions) -> PgWireResult<Option<i32>> {
    parse_copy_string(raw, options)?
        .map(|value| {
            value
                .parse::<i32>()
                .map_err(|e| user_error(anyhow!("invalid COPY int4 value {value:?}: {e}")))
        })
        .transpose()
}

fn parse_copy_i64(raw: &[u8], options: &CopyTextOptions) -> PgWireResult<Option<i64>> {
    parse_copy_string(raw, options)?
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|e| user_error(anyhow!("invalid COPY int8 value {value:?}: {e}")))
        })
        .transpose()
}

fn parse_copy_f64(raw: &[u8], options: &CopyTextOptions) -> PgWireResult<Option<f64>> {
    parse_copy_string(raw, options)?
        .map(|value| {
            value
                .parse::<f64>()
                .map_err(|e| user_error(anyhow!("invalid COPY float8 value {value:?}: {e}")))
        })
        .transpose()
}

fn parse_copy_bool(raw: &[u8], options: &CopyTextOptions) -> PgWireResult<Option<bool>> {
    parse_copy_string(raw, options)?
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "t" | "true" | "1" | "y" | "yes" | "on" => Ok(true),
            "f" | "false" | "0" | "n" | "no" | "off" => Ok(false),
            _ => Err(user_error(anyhow!("invalid COPY boolean value {value:?}"))),
        })
        .transpose()
}

fn parse_copy_string(raw: &[u8], options: &CopyTextOptions) -> PgWireResult<Option<String>> {
    if raw == options.null.as_slice() {
        return Ok(None);
    }
    let bytes = copy_text_unescape(raw).map_err(user_error)?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|e| user_error(anyhow!("invalid UTF-8 in COPY text field: {e}")))
}

fn parse_copy_bytea(raw: &[u8], options: &CopyTextOptions) -> PgWireResult<Option<Vec<u8>>> {
    if raw == options.null.as_slice() {
        return Ok(None);
    }
    let text = copy_text_unescape(raw).map_err(user_error)?;
    parse_bytea_input(&text).map(Some).map_err(user_error)
}

fn copy_text_unescape(raw: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] != b'\\' {
            out.push(raw[i]);
            i += 1;
            continue;
        }
        i += 1;
        let Some(&escaped) = raw.get(i) else {
            out.push(b'\\');
            break;
        };
        match escaped {
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(0x0b),
            b'0'..=b'7' => {
                let mut value = (escaped - b'0') as u32;
                let mut digits = 1;
                while digits < 3 {
                    let Some(&next) = raw.get(i + 1) else {
                        break;
                    };
                    if !(b'0'..=b'7').contains(&next) {
                        break;
                    }
                    i += 1;
                    digits += 1;
                    value = value * 8 + (next - b'0') as u32;
                }
                if value > u8::MAX as u32 {
                    return Err(anyhow!("COPY octal escape is out of byte range"));
                }
                out.push(value as u8);
            }
            b'x' => {
                let Some(&hi) = raw.get(i + 1) else {
                    out.push(b'x');
                    i += 1;
                    continue;
                };
                let Some(&lo) = raw.get(i + 2) else {
                    out.push(b'x');
                    i += 1;
                    continue;
                };
                if let (Some(hi), Some(lo)) = (hex_value(hi), hex_value(lo)) {
                    out.push((hi << 4) | lo);
                    i += 2;
                } else {
                    out.push(b'x');
                }
            }
            other => out.push(other),
        }
        i += 1;
    }
    Ok(out)
}

fn parse_bytea_input(text: &[u8]) -> Result<Vec<u8>> {
    if text.starts_with(b"\\x") {
        let hex = &text[2..];
        if !hex.len().is_multiple_of(2) {
            return Err(anyhow!("invalid bytea hex input length"));
        }
        let mut out = Vec::with_capacity(hex.len() / 2);
        for pair in hex.chunks_exact(2) {
            let hi = hex_value(pair[0]).ok_or_else(|| anyhow!("invalid bytea hex digit"))?;
            let lo = hex_value(pair[1]).ok_or_else(|| anyhow!("invalid bytea hex digit"))?;
            out.push((hi << 4) | lo);
        }
        return Ok(out);
    }

    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i] != b'\\' {
            out.push(text[i]);
            i += 1;
            continue;
        }
        if text.get(i + 1) == Some(&b'\\') {
            out.push(b'\\');
            i += 2;
            continue;
        }
        if i + 3 < text.len()
            && (b'0'..=b'7').contains(&text[i + 1])
            && (b'0'..=b'7').contains(&text[i + 2])
            && (b'0'..=b'7').contains(&text[i + 3])
        {
            let value = ((text[i + 1] - b'0') as u32) * 64
                + ((text[i + 2] - b'0') as u32) * 8
                + (text[i + 3] - b'0') as u32;
            if value > u8::MAX as u32 {
                return Err(anyhow!("bytea octal escape is out of byte range"));
            }
            out.push(value as u8);
            i += 4;
            continue;
        }
        return Err(anyhow!("invalid bytea escape sequence"));
    }
    Ok(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn scalar_i64_at(array: &dyn Array, row: usize) -> Result<i64> {
    if let Some(values) = array.as_any().downcast_ref::<Int64Array>() {
        return Ok(values.value(row));
    }
    if let Some(values) = array.as_any().downcast_ref::<Int32Array>() {
        return Ok(values.value(row) as i64);
    }
    if let Some(values) = array.as_any().downcast_ref::<UInt64Array>() {
        return i64::try_from(values.value(row)).map_err(|e| anyhow!("row id is too large: {e}"));
    }
    Err(anyhow!(
        "expected integer row id column, got {}",
        array.data_type()
    ))
}

fn normalize_batches_for_ducklake(batches: Vec<RecordBatch>) -> Result<Vec<RecordBatch>> {
    batches
        .into_iter()
        .map(normalize_batch_for_ducklake)
        .collect()
}

fn normalize_batch_for_ducklake(batch: RecordBatch) -> Result<RecordBatch> {
    let fields = batch.schema().fields().iter().cloned().collect::<Vec<_>>();
    let mut changed = false;
    let mut new_fields = Vec::with_capacity(fields.len());
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(fields.len());

    for (field, arr) in fields.into_iter().zip(batch.columns()) {
        match field.data_type() {
            DataType::Utf8View => {
                let a = arr
                    .as_any()
                    .downcast_ref::<StringViewArray>()
                    .ok_or_else(|| anyhow!("expected StringViewArray for Utf8View"))?;
                let vals: Vec<Option<&str>> = (0..a.len())
                    .map(|i| if a.is_null(i) { None } else { Some(a.value(i)) })
                    .collect();
                arrays.push(Arc::new(StringArray::from(vals)));
                new_fields.push(Arc::new(Field::new(
                    field.name(),
                    DataType::Utf8,
                    field.is_nullable(),
                )));
                changed = true;
            }
            DataType::BinaryView => {
                let a = arr
                    .as_any()
                    .downcast_ref::<BinaryViewArray>()
                    .ok_or_else(|| anyhow!("expected BinaryViewArray for BinaryView"))?;
                let vals: Vec<Option<&[u8]>> = (0..a.len())
                    .map(|i| if a.is_null(i) { None } else { Some(a.value(i)) })
                    .collect();
                arrays.push(Arc::new(BinaryArray::from(vals)));
                new_fields.push(Arc::new(Field::new(
                    field.name(),
                    DataType::Binary,
                    field.is_nullable(),
                )));
                changed = true;
            }
            _ => {
                arrays.push(Arc::clone(arr));
                new_fields.push(field);
            }
        }
    }

    if !changed {
        return Ok(batch);
    }

    RecordBatch::try_new(Arc::new(Schema::new(new_fields)), arrays)
        .map_err(|e| anyhow!("normalizing RecordBatch for DuckLake: {e}"))
}

fn add_null_column_to_batches(
    batches: &[RecordBatch],
    new_field: Field,
) -> Result<Vec<RecordBatch>> {
    let schema = batches
        .first()
        .map(|batch| batch.schema())
        .ok_or_else(|| anyhow!("staged table must have at least one batch"))?;
    let mut fields = schema
        .fields()
        .iter()
        .map(|field| field.as_ref().clone())
        .collect::<Vec<_>>();
    fields.push(new_field.clone());
    let output_schema = Arc::new(Schema::new(fields));
    batches
        .iter()
        .map(|batch| {
            let mut columns = batch.columns().to_vec();
            columns.push(new_null_array(new_field.data_type(), batch.num_rows()));
            RecordBatch::try_new(Arc::clone(&output_schema), columns)
                .map_err(|e| anyhow!("adding staged null column: {e}"))
        })
        .collect()
}

fn fields_with_synthetic_rowid_if_needed(mut fields: Vec<Field>) -> Vec<Field> {
    if needs_synthetic_rowid_for_fields(&fields) {
        fields.insert(
            0,
            Field::new(SYNTHETIC_ROWID_COLUMN, DataType::Int64, false),
        );
    }
    fields
}

fn needs_synthetic_rowid_for_schema(schema: &Schema) -> bool {
    needs_synthetic_rowid_for_fields(
        &schema
            .fields()
            .iter()
            .map(|field| field.as_ref().clone())
            .collect::<Vec<_>>(),
    )
}

fn needs_synthetic_rowid_for_fields(fields: &[Field]) -> bool {
    let has_spatial_column = fields
        .iter()
        .any(|field| crate::geometry_columns::is_geometry_column_name(field.name()));
    let has_id = fields
        .iter()
        .any(|field| field.name().eq_ignore_ascii_case("id"));
    let has_rowid = fields
        .iter()
        .any(|field| field.name().eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN));
    has_spatial_column && !has_id && !has_rowid
}

fn prepend_synthetic_rowid_to_batches(batches: Vec<RecordBatch>) -> Result<Vec<RecordBatch>> {
    let mut next_rowid = 1_i64;
    batches
        .into_iter()
        .map(|batch| {
            let row_count = batch.num_rows();
            let rowids = (next_rowid..next_rowid + row_count as i64).collect::<Vec<_>>();
            next_rowid += row_count as i64;

            let mut fields = vec![Field::new(SYNTHETIC_ROWID_COLUMN, DataType::Int64, false)];
            fields.extend(
                batch
                    .schema()
                    .fields()
                    .iter()
                    .map(|field| field.as_ref().clone()),
            );
            let mut arrays: Vec<ArrayRef> = vec![Arc::new(Int64Array::from(rowids))];
            arrays.extend(batch.columns().iter().cloned());
            RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
                .map_err(|e| anyhow!("adding synthetic row id: {e}"))
        })
        .collect()
}

fn sql_type_to_arrow_field(col: &ColumnDef) -> Result<Field> {
    use datafusion::sql::sqlparser::ast::DataType as SqlType;
    let dt = match &col.data_type {
        SqlType::Int(_)
        | SqlType::Int4(_)
        | SqlType::Integer(_)
        | SqlType::SmallInt(_)
        | SqlType::Int2(_) => DataType::Int32,
        SqlType::BigInt(_) | SqlType::Int8(_) => DataType::Int64,
        SqlType::Real
        | SqlType::Float4
        | SqlType::Float8
        | SqlType::Float(_)
        | SqlType::Float32
        | SqlType::Float64
        | SqlType::Double(_)
        | SqlType::DoublePrecision => DataType::Float64,
        SqlType::Bool | SqlType::Boolean => DataType::Boolean,
        SqlType::Text
        | SqlType::String(_)
        | SqlType::Varchar(_)
        | SqlType::Nvarchar(_)
        | SqlType::Char(_)
        | SqlType::Character(_)
        | SqlType::CharacterVarying(_) => DataType::Utf8,
        SqlType::Bytea
        | SqlType::Binary(_)
        | SqlType::Varbinary(_)
        | SqlType::Blob(_)
        | SqlType::Bytes(_) => DataType::Binary,
        SqlType::Custom(name, _) if is_spatial_type_name(name) => DataType::Binary,
        SqlType::Custom(name, _) if custom_type_name(name).eq_ignore_ascii_case("serial") => {
            DataType::Int32
        }
        SqlType::Custom(name, _) if custom_type_name(name).eq_ignore_ascii_case("bigserial") => {
            DataType::Int64
        }
        other => {
            return Err(anyhow!(
                "unsupported CREATE TABLE column type for {}: {other}",
                col.name
            ));
        }
    };
    Ok(Field::new(ident_name(&col.name), dt, true))
}

fn ident_name(ident: &datafusion::sql::sqlparser::ast::Ident) -> String {
    ident.to_string().trim_matches('"').to_string()
}

fn arrow_type_to_sql(dt: &DataType) -> Result<&'static str> {
    match dt {
        DataType::Int32 => Ok("INT"),
        DataType::Int64 => Ok("BIGINT"),
        DataType::Float64 => Ok("DOUBLE"),
        DataType::Boolean => Ok("BOOLEAN"),
        DataType::Utf8 => Ok("VARCHAR"),
        DataType::Binary => Ok("BYTEA"),
        other => Err(anyhow!("unsupported INSERT target column type: {other}")),
    }
}

fn empty_array_for(dt: &DataType) -> Result<ArrayRef> {
    match dt {
        DataType::Int32 => Ok(Arc::new(Int32Array::from(Vec::<i32>::new()))),
        DataType::Int64 => Ok(Arc::new(Int64Array::from(Vec::<i64>::new()))),
        DataType::Float64 => Ok(Arc::new(Float64Array::from(Vec::<f64>::new()))),
        DataType::Boolean => Ok(Arc::new(BooleanArray::from(Vec::<bool>::new()))),
        DataType::Utf8 => Ok(Arc::new(StringArray::from(Vec::<String>::new()))),
        DataType::Binary => Ok(Arc::new(BinaryArray::from(Vec::<&[u8]>::new()))),
        _ => Ok(Arc::new(NullArray::new(0))),
    }
}

fn empty_batch_for_fields(fields: Vec<Field>) -> Result<RecordBatch> {
    let arrays = fields
        .iter()
        .map(|f| empty_array_for(f.data_type()))
        .collect::<Result<Vec<_>>>()?;
    RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
        .map_err(|e| anyhow!("creating empty RecordBatch: {e}"))
}

fn is_spatial_type_name(name: &ObjectName) -> bool {
    let ty = custom_type_name(name);
    ty.eq_ignore_ascii_case("geometry") || ty.eq_ignore_ascii_case("geography")
}

fn custom_type_name(name: &ObjectName) -> String {
    name.0
        .last()
        .map(|part| part.to_string().trim_matches('"').to_string())
        .unwrap_or_default()
}

fn user_error(err: anyhow::Error) -> PgWireError {
    PgWireError::UserError(Box::new(
        datafusion_postgres::pgwire::error::ErrorInfo::new(
            "ERROR".to_string(),
            "22023".to_string(),
            err.to_string(),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_catalog_refresh_recency_honors_interval() {
        let now = Instant::now();
        let state = SharedCatalogRefreshState::new(Duration::from_secs(60));
        let stale = now
            .checked_sub(Duration::from_secs(61))
            .expect("stale instant");

        assert!(state.refresh_is_recent(now, Some(now)));
        assert!(!state.refresh_is_recent(now, Some(stale)));
        assert!(!state.refresh_is_recent(now, None));
    }

    #[test]
    fn shared_catalog_refresh_zero_interval_forces_reads_to_refresh() {
        let now = Instant::now();
        let state = SharedCatalogRefreshState::new(Duration::ZERO);

        assert!(!state.refresh_is_recent(now, Some(now)));
    }

    #[test]
    fn selective_read_target_partitions_parser_accepts_positive_values() {
        assert_eq!(
            parse_selective_read_target_partitions_value("1").unwrap(),
            Some(1)
        );
        assert_eq!(
            parse_selective_read_target_partitions_value(" 4 ").unwrap(),
            Some(4)
        );
    }

    #[test]
    fn selective_read_target_partitions_parser_disables_on_zero_or_empty() {
        assert_eq!(
            parse_selective_read_target_partitions_value("").unwrap(),
            None
        );
        assert_eq!(
            parse_selective_read_target_partitions_value(" 0 ").unwrap(),
            None
        );
    }

    #[test]
    fn selective_read_target_partitions_parser_rejects_invalid_values() {
        assert!(parse_selective_read_target_partitions_value("many").is_err());
        assert!(parse_selective_read_target_partitions_value("-1").is_err());
    }
}
