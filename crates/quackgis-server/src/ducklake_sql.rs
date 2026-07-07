// SPDX-License-Identifier: Apache-2.0
//! QuackGIS SQL-to-DuckLake routing.
//!
//! datafusion-ducklake's writer API is the validated storage path. This hook
//! maps the SQL clients actually send (CTAS / INSERT) onto that writer API for
//! the `quackgis.main.<table>` catalog path.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use datafusion::arrow::array::{
    Array, ArrayRef, BinaryArray, BinaryViewArray, BooleanArray, Float64Array, Int32Array,
    Int64Array, NullArray, StringArray, StringViewArray, new_null_array,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::{DFSchema, DFSchemaRef, ParamValues};
use datafusion::datasource::MemTable;
use datafusion::logical_expr::{EmptyRelation, LogicalPlan};
use datafusion::prelude::SessionContext;
use datafusion::sql::sqlparser::ast::{
    AlterTable, AlterTableOperation, AssignmentTarget, ColumnDef, ObjectName, SelectItem,
};
use datafusion_ducklake::{
    DuckLakeCatalog, DuckLakeTableWriter, MetadataWriter, SqliteMetadataProvider,
    SqliteMetadataWriter, TableWriteSession, WriteMode,
};
use datafusion_postgres::arrow_pg::datatypes::{arrow_schema_to_pg_fields, encode_recordbatch};
use datafusion_postgres::hooks::{HookClient, QueryHook};
use datafusion_postgres::pgwire::api::portal::Format;
use datafusion_postgres::pgwire::api::results::{QueryResponse, Response, Tag};
use datafusion_postgres::pgwire::error::{PgWireError, PgWireResult};
use datafusion_postgres::pgwire::messages::response::TransactionStatus;
use object_store::local::LocalFileSystem;
use tokio::sync::Mutex;

use crate::catalog_compat::SYNTHETIC_ROWID_COLUMN;
use crate::context::{DUCKLAKE_CATALOG, StoragePaths};

mod names;
mod params;
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

#[derive(Debug, Clone)]
pub struct DuckLakeSqlHook {
    paths: StoragePaths,
}

#[derive(Debug, Default)]
struct ClientTransactionState {
    inner: Mutex<TransactionState>,
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

impl DuckLakeSqlHook {
    pub fn new(paths: StoragePaths) -> Self {
        Self { paths }
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
        _client: &(dyn datafusion_postgres::pgwire::api::ClientInfo + Send + Sync),
    ) -> Option<PgWireResult<LogicalPlan>> {
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
        if ducklake_statement_parts(statement).is_some() {
            return Some(Ok(empty_logical_plan()));
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

fn client_transaction_state<C>(client: &C) -> Arc<ClientTransactionState>
where
    C: datafusion_postgres::pgwire::api::ClientInfo + Send + Sync + ?Sized,
{
    client
        .session_extensions()
        .get_or_insert_with(ClientTransactionState::default)
}

fn next_transaction_id<C>(client: &C) -> String
where
    C: datafusion_postgres::pgwire::api::ClientInfo + Send + Sync + ?Sized,
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

fn ducklake_statement_parts(
    statement: &datafusion::sql::sqlparser::ast::Statement,
) -> Option<(String, String)> {
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
        _ => None,
    }
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

    async fn client_in_transaction(&self, client: &dyn HookClient) -> bool {
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
            for batch in &staged.batches {
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
            self.refresh_ducklake_catalog(session_context).await?;
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
        let writer = Arc::new(
            SqliteMetadataWriter::new_with_init(&self.paths.catalog_conn)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
        );
        writer
            .set_data_path(&self.paths.data_path)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(LocalFileSystem::new());
        let table_writer = DuckLakeTableWriter::new(writer, object_store)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
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
            let predicate = predicate.unwrap_or_else(|| "TRUE".to_string());
            let returning_query =
                returning_query_from_table(&table_ref, returning, Some(&predicate));
            Some(collect_query_batches(session_context, &returning_query).await?)
        } else {
            None
        };
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
                if field.name().eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN) {
                    continue;
                }
                insert_positions.insert(field.name().clone(), source_idx);
                source_idx += 1;
            }
        } else {
            for (idx, name) in insert_columns.iter().enumerate() {
                let col = object_name_last(name)
                    .ok_or_else(|| user_error(anyhow!("invalid INSERT column")))?;
                if col.eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN) {
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
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        let writer = Arc::new(
            SqliteMetadataWriter::new_with_init(&self.paths.catalog_conn)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
        );
        writer
            .set_data_path(&self.paths.data_path)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let snapshot = writer
            .create_snapshot()
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        writer
            .get_or_create_schema(schema, None, snapshot)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(LocalFileSystem::new());
        let table_writer = DuckLakeTableWriter::new(writer, object_store)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        match disposition {
            WriteDisposition::Replace => table_writer
                .write_table(schema, table, batches)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
            WriteDisposition::Append => table_writer
                .append_table(schema, table, batches)
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
        let writer = Arc::new(
            SqliteMetadataWriter::new_with_init(&self.paths.catalog_conn)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
        );
        writer
            .set_data_path(&self.paths.data_path)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let provider = SqliteMetadataProvider::new(&self.paths.catalog_conn)
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let ducklake = DuckLakeCatalog::with_writer(Arc::new(provider), writer)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        session_context.register_catalog(DUCKLAKE_CATALOG, Arc::new(ducklake));
        crate::public_schema::register_public_schema_alias(session_context)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum WriteDisposition {
    Replace,
    Append,
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
