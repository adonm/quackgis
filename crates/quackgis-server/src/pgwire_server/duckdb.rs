// SPDX-License-Identifier: Apache-2.0
//! Bounded DuckDB pgwire backend.
//!
//! This local profile proves the direct ADBC/Arrow protocol seam through the real
//! CLI backend. Unsupported policy, storage, statement, COPY, and parameter
//! shapes fail closed until their D2-D4 contracts pass.

use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Float32Array, Float64Array, Int16Array, Int32Array,
    Int64Array, RecordBatch, RecordBatchReader, StringArray, UInt32Array,
};
use arrow_pg::datatypes::{
    PgTypeHint, arrow_schema_to_pg_fields, field_into_pg_type, with_pg_type_hint,
};
use arrow_pg::encode_recordbatch;
use arrow_schema::{ArrowError, DataType, Field, Schema, SchemaRef};
use async_trait::async_trait;
use futures::{Sink, SinkExt};
use pgwire::api::cancel::CancelHandler;
use pgwire::api::copy::CopyHandler;
use pgwire::api::portal::{Format, Portal, PortalExecutionState};
use pgwire::api::query::{ExtendedQueryHandler, SimpleQueryHandler, send_partial_query_response};
use pgwire::api::results::{CopyResponse, QueryResponse, Response, Tag};
use pgwire::api::stmt::QueryParser;
use pgwire::api::store::PortalStore;
use pgwire::api::{
    ClientInfo, ClientPortalStore, ConnectionManager, DEFAULT_NAME, ErrorHandler,
    PgWireConnectionState, PgWireServerHandlers, Type,
};
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use pgwire::messages::PgWireBackendMessage;
use pgwire::messages::cancel::CancelRequest;
use pgwire::messages::copy::{CopyData, CopyDone, CopyFail};
use pgwire::messages::data::DataRow;
use pgwire::messages::extendedquery::Execute;
use pgwire::messages::response::TransactionStatus;
use sqlparser::ast::{
    BinaryOperator, CopySource, CopyTarget as AstCopyTarget, Expr, FunctionArg, FunctionArgExpr,
    FunctionArguments, Ident, JoinConstraint, JoinOperator, ObjectName, ObjectNamePart, SelectItem,
    Set, SetExpr, Statement, TableFactor, Value, visit_expressions, visit_expressions_mut,
    visit_relations_mut,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use super::copy_text::{CopyBatchLimits, CopyDecodeError, CopyTextDecoder};
use super::{
    LoggingErrorHandler, QuackGisStartupHandler, ServerOptions, SimpleStartupHandler,
    serve_with_handlers, serve_with_handlers_on_listener, serve_with_handlers_on_listener_until,
};
use crate::auth::{AuthConfig, AuthMode};
use crate::duckdb_adbc_storage::DuckDbAdbcStorage;
use crate::engine_api::{
    EngineCancellation, EngineError, EngineErrorKind, EngineMaintenanceRequest, EngineQueryStream,
    EngineResult, EngineStorageKernel, EngineTableRef, EngineTransactionState, IngestDisposition,
};
use crate::execution_control::{
    ActiveQueryRegistry, AdmissionController, AdmissionError, BlockingWorkerError,
    BlockingWorkerPool, OperationClass, OperationDeadline,
};

pub async fn serve_duckdb(
    storage: Arc<DuckDbAdbcStorage>,
    options: &ServerOptions,
    auth: AuthConfig,
) -> Result<(), std::io::Error> {
    let factory = Arc::new(DuckDbHandlerFactory::new(storage, auth, options));
    serve_with_handlers(factory, options).await
}

pub async fn serve_duckdb_on_listener(
    storage: Arc<DuckDbAdbcStorage>,
    listener: tokio::net::TcpListener,
    options: &ServerOptions,
    auth: AuthConfig,
) -> Result<(), std::io::Error> {
    let factory = Arc::new(DuckDbHandlerFactory::new(storage, auth, options));
    serve_with_handlers_on_listener(factory, listener, options).await
}

pub async fn serve_duckdb_until(
    storage: Arc<DuckDbAdbcStorage>,
    options: &ServerOptions,
    auth: AuthConfig,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), std::io::Error> {
    let factory = Arc::new(DuckDbHandlerFactory::new(storage, auth, options));
    let address = format!("{}:{}", options.host, options.port);
    let listener = tokio::net::TcpListener::bind(address).await?;
    serve_with_handlers_on_listener_until(factory, listener, options, shutdown).await
}

pub async fn serve_duckdb_on_listener_until(
    storage: Arc<DuckDbAdbcStorage>,
    listener: tokio::net::TcpListener,
    options: &ServerOptions,
    auth: AuthConfig,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), std::io::Error> {
    let factory = Arc::new(DuckDbHandlerFactory::new(storage, auth, options));
    serve_with_handlers_on_listener_until(factory, listener, options, shutdown).await
}

struct DuckDbHandlerFactory {
    service: Arc<DuckDbService>,
    startup: Arc<QuackGisStartupHandler>,
    cancel: Arc<DuckDbCancelHandler>,
    copy: Arc<DuckDbCopyHandler>,
}

impl DuckDbHandlerFactory {
    fn new(storage: Arc<DuckDbAdbcStorage>, auth: AuthConfig, options: &ServerOptions) -> Self {
        let manager = Arc::new(ConnectionManager::new());
        let admission = Arc::new(AdmissionController::new(
            options.max_active_queries(),
            options.max_queued_queries(),
            options.max_reader_queries(),
            options.max_writer_queries(),
            options.max_maintenance_queries(),
            options.queue_timeout(),
        ));
        let active_queries = Arc::new(ActiveQueryRegistry::default());
        let blocking_workers = Arc::new(BlockingWorkerPool::new(options.max_blocking_workers()));
        let control = Arc::new(DuckDbRuntimeControl {
            admission,
            active_queries,
            blocking_workers,
            statement_timeout: options.statement_timeout(),
            copy_limits: CopyBatchLimits {
                max_rows: options.copy_batch_rows(),
                max_bytes: options.copy_batch_bytes(),
                max_row_bytes: options.copy_max_row_bytes(),
            },
            result_batch_bytes: options.result_batch_bytes(),
        });
        let startup_auth = match auth.mode() {
            AuthMode::Trust => super::StartupAuthHandler::Trust(SimpleStartupHandler {
                connection_manager: Arc::clone(&manager),
            }),
            AuthMode::Password => super::StartupAuthHandler::Password(Box::new(
                super::PerConnectionScramStartupHandler::new(auth.clone(), Arc::clone(&manager)),
            )),
        };
        let startup = QuackGisStartupHandler {
            auth: startup_auth,
            tls_required: options.tls_required(),
        };
        Self {
            service: Arc::new(DuckDbService::new(storage, auth, Arc::clone(&control))),
            startup: Arc::new(startup),
            cancel: Arc::new(DuckDbCancelHandler {
                active_queries: Arc::clone(&control.active_queries),
                blocking_workers: Arc::clone(&control.blocking_workers),
            }),
            copy: Arc::new(DuckDbCopyHandler),
        }
    }
}

struct DuckDbCancelHandler {
    active_queries: Arc<ActiveQueryRegistry>,
    blocking_workers: Arc<BlockingWorkerPool>,
}

#[async_trait]
impl CancelHandler for DuckDbCancelHandler {
    async fn on_cancel_request(&self, request: CancelRequest) {
        let registry = Arc::clone(&self.active_queries);
        let secret = request.secret_key.to_bytes().to_vec();
        match self
            .blocking_workers
            .run_control(move || registry.cancel(request.pid, &secret))
            .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => log::warn!("DuckDB native cancellation failed: {error}"),
            Err(error) => log::warn!("DuckDB cancellation worker failed: {error:?}"),
        }
    }
}

impl PgWireServerHandlers for DuckDbHandlerFactory {
    fn simple_query_handler(&self) -> Arc<impl SimpleQueryHandler> {
        Arc::clone(&self.service)
    }

    fn extended_query_handler(&self) -> Arc<impl ExtendedQueryHandler> {
        Arc::clone(&self.service)
    }

    fn startup_handler(&self) -> Arc<impl pgwire::api::auth::StartupHandler> {
        Arc::clone(&self.startup)
    }

    fn copy_handler(&self) -> Arc<impl CopyHandler> {
        Arc::clone(&self.copy)
    }

    fn error_handler(&self) -> Arc<impl ErrorHandler> {
        Arc::new(LoggingErrorHandler)
    }

    fn cancel_handler(&self) -> Arc<impl CancelHandler> {
        Arc::clone(&self.cancel)
    }
}

#[derive(Clone, Debug)]
struct DuckDbStatement {
    sql: String,
    copy_target: Option<CopyTarget>,
    kind: StatementKind,
    parameter_schema: SchemaRef,
    result_schema: SchemaRef,
    parameter_types: Vec<Type>,
}

struct DuckDbParser {
    storage: Arc<DuckDbAdbcStorage>,
    auth: AuthConfig,
    admission: Arc<AdmissionController>,
    blocking_workers: Arc<BlockingWorkerPool>,
}

#[async_trait]
impl QueryParser for DuckDbParser {
    type Statement = DuckDbStatement;

    async fn parse_sql<C>(
        &self,
        client: &C,
        sql: &str,
        types: &[Option<Type>],
    ) -> PgWireResult<Self::Statement>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        if client.transaction_status() == TransactionStatus::Error {
            return Err(failed_transaction_error());
        }
        if let Some(copy_target) = parse_copy_target(sql)? {
            authorize_copy(client, &self.auth, &copy_target)?;
            let empty = Arc::new(Schema::empty());
            return Ok(DuckDbStatement {
                sql: sql.trim().to_owned(),
                copy_target: Some(copy_target),
                kind: StatementKind::Copy,
                parameter_schema: Arc::clone(&empty),
                result_schema: empty,
                parameter_types: Vec::new(),
            });
        }
        let validated = validate_statement(sql, ProtocolMode::Extended)?;
        let oid_parameters = catalog_oid_parameter_indexes(&validated.ast);
        authorize_statement(client, &self.auth, &validated.ast)?;
        if validated.kind == StatementKind::SessionSet {
            let empty = Arc::new(Schema::empty());
            return Ok(DuckDbStatement {
                sql: validated.sql,
                copy_target: None,
                kind: validated.kind,
                parameter_schema: Arc::clone(&empty),
                result_schema: empty,
                parameter_types: Vec::new(),
            });
        }
        let storage = client_session(
            client,
            Arc::clone(&self.storage),
            Arc::clone(&self.blocking_workers),
        )
        .await?;
        let _permit = self
            .admission
            .acquire(validated.kind.operation_class())
            .await
            .map_err(admission_error)?;
        let describe_sql = validated.sql.clone();
        let description = self
            .blocking_workers
            .run_regular(move || storage.describe(&describe_sql))
            .await
            .map_err(blocking_worker_error)?
            .map_err(engine_error)?;
        let parameter_schema = Arc::new(Schema::new(
            description
                .parameter_schema
                .fields()
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    if types
                        .get(index)
                        .is_some_and(|pg_type| pg_type.as_ref() == Some(&Type::OID))
                    {
                        return with_pg_type_hint(
                            Field::new(field.name(), DataType::UInt32, true),
                            PgTypeHint::Oid,
                        );
                    }
                    if field.data_type() == &DataType::Null
                        && let Some(Some(pg_type)) = types.get(index)
                    {
                        let field = Field::new(
                            field.name(),
                            pg_type_to_arrow(pg_type).unwrap_or(DataType::Null),
                            true,
                        );
                        return if *pg_type == Type::OID {
                            with_pg_type_hint(field, PgTypeHint::Oid)
                        } else {
                            field
                        };
                    }
                    if field.data_type() == &DataType::Null && oid_parameters.contains(&index) {
                        return with_pg_type_hint(
                            Field::new("oid", DataType::UInt32, false),
                            PgTypeHint::Oid,
                        );
                    }
                    field.as_ref().clone()
                })
                .collect::<Vec<_>>(),
        ));
        let parameter_types = parameter_schema
            .fields()
            .iter()
            .map(field_into_pg_type)
            .collect::<PgWireResult<Vec<_>>>()?;
        let result_schema = Arc::new(annotate_catalog_result_schema(
            &validated.ast,
            description.result_schema.as_ref(),
        ));
        Ok(DuckDbStatement {
            sql: validated.sql,
            copy_target: None,
            kind: validated.kind,
            parameter_schema,
            result_schema,
            parameter_types,
        })
    }

    fn get_parameter_types(&self, statement: &Self::Statement) -> PgWireResult<Vec<Type>> {
        Ok(statement.parameter_types.clone())
    }

    fn get_result_schema(
        &self,
        statement: &Self::Statement,
        format: Option<&Format>,
    ) -> PgWireResult<Vec<pgwire::api::results::FieldInfo>> {
        let default_format = Format::UnifiedText;
        arrow_schema_to_pg_fields(
            statement.result_schema.as_ref(),
            format.unwrap_or(&default_format),
            None,
        )
    }
}

fn pg_type_to_arrow(pg_type: &Type) -> Option<DataType> {
    match *pg_type {
        Type::BOOL => Some(DataType::Boolean),
        Type::INT2 => Some(DataType::Int16),
        Type::INT4 => Some(DataType::Int32),
        Type::INT8 => Some(DataType::Int64),
        Type::OID => Some(DataType::UInt32),
        Type::FLOAT4 => Some(DataType::Float32),
        Type::FLOAT8 => Some(DataType::Float64),
        Type::TEXT | Type::VARCHAR => Some(DataType::Utf8),
        Type::BYTEA => Some(DataType::Binary),
        _ => None,
    }
}

struct DuckDbService {
    storage: Arc<DuckDbAdbcStorage>,
    parser: Arc<DuckDbParser>,
    auth: AuthConfig,
    control: Arc<DuckDbRuntimeControl>,
}

struct DuckDbRuntimeControl {
    admission: Arc<AdmissionController>,
    active_queries: Arc<ActiveQueryRegistry>,
    blocking_workers: Arc<BlockingWorkerPool>,
    statement_timeout: std::time::Duration,
    copy_limits: CopyBatchLimits,
    result_batch_bytes: usize,
}

impl DuckDbService {
    fn new(
        storage: Arc<DuckDbAdbcStorage>,
        auth: AuthConfig,
        control: Arc<DuckDbRuntimeControl>,
    ) -> Self {
        Self {
            parser: Arc::new(DuckDbParser {
                storage: Arc::clone(&storage),
                auth: auth.clone(),
                admission: Arc::clone(&control.admission),
                blocking_workers: Arc::clone(&control.blocking_workers),
            }),
            storage,
            auth,
            control,
        }
    }
}

#[async_trait]
impl SimpleQueryHandler for DuckDbService {
    async fn do_query<C>(&self, client: &mut C, query: &str) -> PgWireResult<Vec<Response>>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let (sql, kind, ast) = validate_simple_sql(query)?;
        let failed_transaction = client.transaction_status() == TransactionStatus::Error;
        if failed_transaction
            && !matches!(
                &kind,
                SimpleStatementKind::Commit | SimpleStatementKind::Rollback
            )
        {
            return Err(failed_transaction_error());
        }
        match (&kind, ast.as_ref()) {
            (SimpleStatementKind::Copy(target), _) => authorize_copy(client, &self.auth, target)?,
            (SimpleStatementKind::Maintenance(command), _) => {
                authorize_maintenance(client, &self.auth, command)?
            }
            (_, Some(statement)) => authorize_statement(client, &self.auth, statement)?,
            (SimpleStatementKind::SessionSetBatch(_), None) => {}
            (_, None) => {
                return Err(user_error(
                    "XX000",
                    "validated DuckDB statement is missing its structural policy input",
                ));
            }
        }
        let storage = client_session(
            client,
            Arc::clone(&self.storage),
            Arc::clone(&self.control.blocking_workers),
        )
        .await?;
        match kind {
            SimpleStatementKind::Read => {
                let permit = self
                    .control
                    .admission
                    .acquire(OperationClass::Reader)
                    .await
                    .map_err(admission_error)?;
                let mut result = self
                    .control
                    .blocking_workers
                    .run_regular(move || storage.query_stream(&sql))
                    .await
                    .map_err(blocking_worker_error)?
                    .map_err(engine_error)?
                    .with_guard(Box::new(permit));
                if let Some(cancellation) = result.cancellation() {
                    let (pid, secret) = client.pid_and_secret_key();
                    let deadline_cancellation = Arc::clone(&cancellation);
                    let guard = self.control.active_queries.register(
                        pid,
                        secret.to_bytes().to_vec(),
                        cancellation,
                    );
                    result = result.with_guard(Box::new(guard));
                    result = result.with_guard(Box::new(OperationDeadline::start(
                        self.control.statement_timeout,
                        deadline_cancellation,
                        Arc::clone(&self.control.blocking_workers),
                    )));
                }
                let result_schema = ast.as_ref().map(|statement| {
                    annotate_catalog_result_schema(statement, result.schema.as_ref())
                });
                Ok(vec![Response::Query(query_response(
                    result,
                    &Format::UnifiedText,
                    self.control.result_batch_bytes,
                    Arc::clone(&self.control.blocking_workers),
                    result_schema.as_ref(),
                )?)])
            }
            SimpleStatementKind::Write(command) => {
                let _permit = self
                    .control
                    .admission
                    .acquire(OperationClass::Writer)
                    .await
                    .map_err(admission_error)?;
                let operation = storage.start_update_operation().map_err(engine_error)?;
                let cancellation = operation.cancellation();
                let (pid, secret) = client.pid_and_secret_key();
                let _cancellation_guard = self.control.active_queries.register(
                    pid,
                    secret.to_bytes().to_vec(),
                    Arc::clone(&cancellation),
                );
                let _deadline = OperationDeadline::start(
                    self.control.statement_timeout,
                    cancellation,
                    Arc::clone(&self.control.blocking_workers),
                );
                let affected = self
                    .control
                    .blocking_workers
                    .run_regular(move || operation.execute(&sql, None))
                    .await
                    .map_err(blocking_worker_error)?
                    .map_err(engine_error)?;
                let mut tag = Tag::new(command);
                if let Some(rows) = affected.and_then(|rows| usize::try_from(rows).ok()) {
                    tag = tag.with_rows(rows);
                }
                Ok(vec![Response::Execution(tag)])
            }
            SimpleStatementKind::Begin => {
                let _permit = self
                    .control
                    .admission
                    .acquire(OperationClass::Writer)
                    .await
                    .map_err(admission_error)?;
                self.control
                    .blocking_workers
                    .run_regular(move || storage.begin_transaction())
                    .await
                    .map_err(blocking_worker_error)?
                    .map_err(anyhow_error)?;
                Ok(vec![Response::TransactionStart(Tag::new("BEGIN"))])
            }
            SimpleStatementKind::Commit => {
                let _permit = self
                    .control
                    .admission
                    .acquire(OperationClass::Writer)
                    .await
                    .map_err(admission_error)?;
                client.portal_store().clear_portals();
                if failed_transaction {
                    self.control
                        .blocking_workers
                        .run_regular(move || storage.rollback_transaction())
                        .await
                        .map_err(blocking_worker_error)?
                        .map_err(anyhow_error)?;
                    Ok(vec![Response::TransactionEnd(Tag::new("ROLLBACK"))])
                } else {
                    self.control
                        .blocking_workers
                        .run_regular(move || storage.commit_transaction())
                        .await
                        .map_err(blocking_worker_error)?
                        .map_err(fatal_anyhow_error)?;
                    Ok(vec![Response::TransactionEnd(Tag::new("COMMIT"))])
                }
            }
            SimpleStatementKind::Rollback => {
                let _permit = self
                    .control
                    .admission
                    .acquire(OperationClass::Writer)
                    .await
                    .map_err(admission_error)?;
                client.portal_store().clear_portals();
                self.control
                    .blocking_workers
                    .run_regular(move || storage.rollback_transaction())
                    .await
                    .map_err(blocking_worker_error)?
                    .map_err(anyhow_error)?;
                Ok(vec![Response::TransactionEnd(Tag::new("ROLLBACK"))])
            }
            SimpleStatementKind::Maintenance(command) => {
                if storage.transaction_state() != EngineTransactionState::Idle {
                    return Err(user_error(
                        "25001",
                        "DuckDB maintenance cannot run inside an explicit transaction",
                    ));
                }
                let _permit = self
                    .control
                    .admission
                    .acquire(OperationClass::Maintenance)
                    .await
                    .map_err(admission_error)?;
                let user = client
                    .metadata()
                    .get("user")
                    .cloned()
                    .unwrap_or_else(|| "<unknown>".to_owned());
                let target = command.target_label();
                let result = self
                    .control
                    .blocking_workers
                    .run_regular(move || storage.maintain(command.request))
                    .await
                    .map_err(blocking_worker_error)?;
                match result {
                    Ok(report) => {
                        let rows = report
                            .affected_rows
                            .and_then(|rows| usize::try_from(rows).ok());
                        crate::audit::log_maintenance(
                            &user,
                            "merge_adjacent_files",
                            &target,
                            crate::audit::AuditOutcome::Succeeded,
                            rows,
                        );
                        let mut tag = Tag::new("CALL");
                        if let Some(rows) = rows {
                            tag = tag.with_rows(rows);
                        }
                        Ok(vec![Response::Execution(tag)])
                    }
                    Err(error) => {
                        crate::audit::log_maintenance(
                            &user,
                            "merge_adjacent_files",
                            &target,
                            crate::audit::AuditOutcome::Failed,
                            None,
                        );
                        Err(engine_error(error))
                    }
                }
            }
            SimpleStatementKind::SessionSet => Ok(vec![Response::Execution(Tag::new("SET"))]),
            SimpleStatementKind::SessionSetBatch(count) => Ok((0..count)
                .map(|_| Response::Execution(Tag::new("SET")))
                .collect()),
            SimpleStatementKind::Copy(target) => begin_copy(client, storage, target, &self.control)
                .await
                .map(|response| vec![response]),
        }
    }
}

#[async_trait]
impl ExtendedQueryHandler for DuckDbService {
    type Statement = DuckDbStatement;
    type QueryParser = DuckDbParser;

    fn query_parser(&self) -> Arc<Self::QueryParser> {
        Arc::clone(&self.parser)
    }

    async fn on_execute<C>(&self, client: &mut C, message: Execute) -> PgWireResult<()>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        if message.max_rows < 0 {
            return Err(user_error("22023", "Execute.max_rows must not be negative"));
        }
        let portal_name = message.name.as_deref().unwrap_or(DEFAULT_NAME);
        let portal = client
            .portal_store()
            .get_portal(portal_name)
            .ok_or_else(|| PgWireError::PortalNotFound(portal_name.to_owned()))?;
        if portal.statement.statement.kind != StatementKind::Read {
            return self._on_execute(client, message).await;
        }
        if !matches!(client.state(), PgWireConnectionState::ReadyForQuery) {
            return Err(PgWireError::NotReadyForQuery);
        }
        client.set_state(PgWireConnectionState::QueryInProgress);
        let initial = matches!(&*portal.state().lock().await, PortalExecutionState::Initial);
        if initial {
            match ExtendedQueryHandler::do_query(
                self,
                client,
                portal.as_ref(),
                message.max_rows as usize,
            )
            .await?
            {
                Response::Query(response) => portal.start(response).await,
                _ => {
                    client.set_state(PgWireConnectionState::ReadyForQuery);
                    return Err(user_error(
                        "XX000",
                        "DuckDB read portal produced a non-query response",
                    ));
                }
            }
        }
        let state = portal.state();
        let mut state = state.lock().await;
        let suspended = match &mut *state {
            PortalExecutionState::Suspended(response) => {
                send_partial_query_response(client, response, message.max_rows as usize).await?
            }
            PortalExecutionState::Finished => {
                client.set_state(PgWireConnectionState::ReadyForQuery);
                return Err(user_error("55000", "DuckDB portal is already finished"));
            }
            PortalExecutionState::Initial => {
                client.set_state(PgWireConnectionState::ReadyForQuery);
                return Err(user_error("XX000", "DuckDB portal did not start"));
            }
        };
        if !suspended {
            *state = PortalExecutionState::Finished;
        }
        drop(state);
        client.set_state(PgWireConnectionState::ReadyForQuery);
        Ok(())
    }

    async fn do_query<C>(
        &self,
        client: &mut C,
        portal: &Portal<Self::Statement>,
        _max_rows: usize,
    ) -> PgWireResult<Response>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let statement = &portal.statement.statement;
        if let Some(target) = &statement.copy_target {
            let storage = client_session(
                client,
                Arc::clone(&self.storage),
                Arc::clone(&self.control.blocking_workers),
            )
            .await?;
            return begin_copy(client, storage, target.clone(), &self.control).await;
        }
        let parameters = parameter_batch(portal, statement)?;
        let storage = client_session(
            client,
            Arc::clone(&self.storage),
            Arc::clone(&self.control.blocking_workers),
        )
        .await?;
        let sql = statement.sql.clone();
        match statement.kind {
            StatementKind::Read => {
                let permit = self
                    .control
                    .admission
                    .acquire(OperationClass::Reader)
                    .await
                    .map_err(admission_error)?;
                let mut result = self
                    .control
                    .blocking_workers
                    .run_regular(move || {
                        if let Some(parameters) = parameters {
                            storage.query_bound_stream(&sql, Some(parameters))
                        } else {
                            storage.query_stream(&sql)
                        }
                    })
                    .await
                    .map_err(blocking_worker_error)?
                    .map_err(engine_error)?
                    .with_guard(Box::new(permit));
                if let Some(cancellation) = result.cancellation() {
                    let (pid, secret) = client.pid_and_secret_key();
                    let deadline_cancellation = Arc::clone(&cancellation);
                    let guard = self.control.active_queries.register(
                        pid,
                        secret.to_bytes().to_vec(),
                        cancellation,
                    );
                    result = result.with_guard(Box::new(guard));
                    result = result.with_guard(Box::new(OperationDeadline::start(
                        self.control.statement_timeout,
                        deadline_cancellation,
                        Arc::clone(&self.control.blocking_workers),
                    )));
                }
                Ok(Response::Query(query_response(
                    result,
                    &portal.result_column_format,
                    self.control.result_batch_bytes,
                    Arc::clone(&self.control.blocking_workers),
                    Some(statement.result_schema.as_ref()),
                )?))
            }
            StatementKind::Write(command) => {
                let _permit = self
                    .control
                    .admission
                    .acquire(OperationClass::Writer)
                    .await
                    .map_err(admission_error)?;
                let operation = storage.start_update_operation().map_err(engine_error)?;
                let cancellation = operation.cancellation();
                let (pid, secret) = client.pid_and_secret_key();
                let _cancellation_guard = self.control.active_queries.register(
                    pid,
                    secret.to_bytes().to_vec(),
                    Arc::clone(&cancellation),
                );
                let _deadline = OperationDeadline::start(
                    self.control.statement_timeout,
                    cancellation,
                    Arc::clone(&self.control.blocking_workers),
                );
                let affected = self
                    .control
                    .blocking_workers
                    .run_regular(move || operation.execute(&sql, parameters))
                    .await
                    .map_err(blocking_worker_error)?
                    .map_err(engine_error)?;
                let mut tag = Tag::new(command);
                if let Some(rows) = affected.and_then(|rows| usize::try_from(rows).ok()) {
                    tag = tag.with_rows(rows);
                }
                Ok(Response::Execution(tag))
            }
            StatementKind::SessionSet => Ok(Response::Execution(Tag::new("SET"))),
            StatementKind::Copy
            | StatementKind::Begin
            | StatementKind::Commit
            | StatementKind::Rollback
            | StatementKind::Maintenance => Err(user_error(
                "0A000",
                "extended protocol does not support this DuckDB statement shape",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StatementKind {
    Read,
    Write(&'static str),
    Begin,
    Commit,
    Rollback,
    SessionSet,
    Maintenance,
    Copy,
}

impl StatementKind {
    fn operation_class(self) -> OperationClass {
        match self {
            Self::Read | Self::SessionSet => OperationClass::Reader,
            Self::Write(_) | Self::Begin | Self::Commit | Self::Rollback | Self::Copy => {
                OperationClass::Writer
            }
            Self::Maintenance => OperationClass::Maintenance,
        }
    }
}

#[derive(Clone, Copy)]
enum ProtocolMode {
    Simple,
    Extended,
}

#[derive(Clone, Debug)]
struct CopyTarget {
    table: EngineTableRef,
    columns: Vec<String>,
}

struct ValidatedStatement {
    sql: String,
    kind: StatementKind,
    ast: Statement,
}

fn validate_simple_sql(
    sql: &str,
) -> PgWireResult<(String, SimpleStatementKind, Option<Statement>)> {
    if let Some(target) = parse_copy_target(sql)? {
        let sql = normalize_sql(sql)?;
        return Ok((sql, SimpleStatementKind::Copy(target), None));
    }
    if let Some(count) = validate_session_set_batch(sql)? {
        return Ok((
            sql.to_owned(),
            SimpleStatementKind::SessionSetBatch(count),
            None,
        ));
    }
    let validated = validate_statement(sql, ProtocolMode::Simple)?;
    let kind = match validated.kind {
        StatementKind::Read => SimpleStatementKind::Read,
        StatementKind::Write(command) => SimpleStatementKind::Write(command),
        StatementKind::Begin => SimpleStatementKind::Begin,
        StatementKind::Commit => SimpleStatementKind::Commit,
        StatementKind::Rollback => SimpleStatementKind::Rollback,
        StatementKind::SessionSet => SimpleStatementKind::SessionSet,
        StatementKind::Maintenance => SimpleStatementKind::Maintenance(
            parse_maintenance_call(&validated.ast)?
                .ok_or_else(|| user_error("XX000", "validated maintenance call has no command"))?,
        ),
        StatementKind::Copy => unreachable!("COPY is classified before structural parsing"),
    };
    Ok((validated.sql, kind, Some(validated.ast)))
}

enum SimpleStatementKind {
    Read,
    Write(&'static str),
    Begin,
    Commit,
    Rollback,
    SessionSet,
    SessionSetBatch(usize),
    Maintenance(MaintenanceCommand),
    Copy(CopyTarget),
}

fn validate_statement(sql: &str, mode: ProtocolMode) -> PgWireResult<ValidatedStatement> {
    let sql = normalize_sql(sql)?;
    let sql = crate::spatial_compat::rewrite_postgis_sql(&sql);
    let mut statements = Parser::parse_sql(&PostgreSqlDialect {}, &sql)
        .map_err(|error| user_error("42601", &error.to_string()))?;
    if statements.len() != 1 {
        return Err(user_error(
            "0A000",
            "DuckDB pgwire backend supports exactly one statement",
        ));
    }
    let statement = statements.pop().expect("one parsed statement");
    if let Some(function) = unsupported_spatial_function(&statement) {
        return Err(user_error(
            "0A000",
            &format!("PostGIS function {function} is not supported by QuackGIS"),
        ));
    }
    if invalid_quoted_catalog_reference(&statement) {
        return Err(user_error(
            "42703",
            "quoted PostgreSQL catalog identifier does not match the maintained lowercase name",
        ));
    }
    if private_catalog_reference(&statement) {
        return Err(user_error(
            "0A000",
            "direct references to the private QuackGIS catalog projection are not supported",
        ));
    }
    if internal_control_schema_reference(&statement) {
        return Err(user_error(
            "42501",
            "direct references to the QuackGIS control schema are not permitted",
        ));
    }
    if let Some(function) = forbidden_catalog_table_function(&statement) {
        return Err(user_error(
            "0A000",
            &format!("DuckDB table function {function} is not exposed through pgwire"),
        ));
    }
    if let Some(cte) = reserved_catalog_cte(&statement) {
        return Err(user_error(
            "0A000",
            &format!("CTE name {cte} conflicts with the reserved PostgreSQL catalog namespace"),
        ));
    }
    if query_contains_table_command(&statement) {
        return Err(user_error(
            "0A000",
            "TABLE query form is not supported by QuackGIS authorization or catalog routing",
        ));
    }
    if let Some(relation) = unsupported_catalog_relation(&statement) {
        return Err(user_error(
            "0A000",
            &format!("PostgreSQL catalog relation {relation} is not implemented by QuackGIS"),
        ));
    }
    if !catalog_query_shape_supported(&statement) {
        return Err(user_error(
            "0A000",
            "PostgreSQL catalog query shape is outside the maintained projection contract",
        ));
    }
    let kind = match &statement {
        Statement::Query(_) => StatementKind::Read,
        Statement::CreateTable(_) => StatementKind::Write("CREATE TABLE"),
        Statement::Insert(_) => StatementKind::Write("INSERT"),
        Statement::Update { .. } => StatementKind::Write("UPDATE"),
        Statement::Delete(_) => StatementKind::Write("DELETE"),
        Statement::StartTransaction { .. } if matches!(mode, ProtocolMode::Simple) => {
            StatementKind::Begin
        }
        Statement::Commit { .. } if matches!(mode, ProtocolMode::Simple) => StatementKind::Commit,
        Statement::Rollback { .. } if matches!(mode, ProtocolMode::Simple) => {
            StatementKind::Rollback
        }
        Statement::Set(set) if supported_session_set(set) => StatementKind::SessionSet,
        Statement::ShowVariable { variable } if supported_show_variable(variable).is_some() => {
            StatementKind::Read
        }
        Statement::Call(_) if matches!(mode, ProtocolMode::Simple) => {
            parse_maintenance_call(&statement)?
                .ok_or_else(|| user_error("0A000", "unsupported DuckDB maintenance procedure"))?;
            StatementKind::Maintenance
        }
        _ => {
            return Err(user_error(
                "0A000",
                "unsupported DuckDB pgwire statement shape",
            ));
        }
    };
    let execution_sql = if let Statement::ShowVariable { variable } = &statement {
        match supported_show_variable(variable).expect("validated SHOW variable") {
            SessionVariable::SearchPath => "SELECT 'public'::VARCHAR AS search_path".to_owned(),
            SessionVariable::ClientEncoding => {
                "SELECT 'UTF8'::VARCHAR AS client_encoding".to_owned()
            }
            SessionVariable::StandardConformingStrings => {
                "SELECT 'on'::VARCHAR AS standard_conforming_strings".to_owned()
            }
        }
    } else {
        let mut execution = statement.clone();
        rewrite_public_relations(&mut execution);
        rewrite_pg_catalog_relations(&mut execution);
        rewrite_pg_catalog_functions(&mut execution);
        execution.to_string()
    };
    Ok(ValidatedStatement {
        sql: execution_sql,
        kind,
        ast: statement,
    })
}

fn unsupported_spatial_function(statement: &Statement) -> Option<&'static str> {
    const UNSUPPORTED: &[(&str, &str)] = &[
        ("st_ndims", "ST_NDims"),
        ("st_coorddim", "ST_CoordDim"),
        ("st_geometryn", "ST_GeometryN"),
        ("st_asewkt", "ST_AsEWKT"),
        ("st_srid", "ST_SRID"),
        ("st_zmflag", "ST_Zmflag"),
        ("st_xmax", "ST_XMax"),
        ("st_ymax", "ST_YMax"),
        ("st_extent", "ST_Extent"),
        ("find_srid", "Find_SRID"),
    ];
    let mut unsupported = None;
    let _: ControlFlow<()> = visit_expressions(statement, |expression| {
        let Expr::Function(function) = expression else {
            return ControlFlow::Continue(());
        };
        let Some(name) = function.name.0.last().and_then(|part| match part {
            ObjectNamePart::Identifier(identifier) => Some(identifier.value.as_str()),
            _ => None,
        }) else {
            return ControlFlow::Continue(());
        };
        unsupported = UNSUPPORTED
            .iter()
            .find_map(|(candidate, label)| name.eq_ignore_ascii_case(candidate).then_some(*label));
        if unsupported.is_some() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    unsupported
}

#[derive(Clone, Debug)]
struct MaintenanceCommand {
    request: EngineMaintenanceRequest,
    schema: String,
    table: String,
}

impl MaintenanceCommand {
    fn target_label(&self) -> String {
        format!("{}.{}", self.schema, self.table)
    }
}

fn parse_maintenance_call(statement: &Statement) -> PgWireResult<Option<MaintenanceCommand>> {
    let Statement::Call(function) = statement else {
        return Ok(None);
    };
    let name = object_name_values(&function.name)
        .ok_or_else(|| user_error("0A000", "maintenance procedure name must be an identifier"))?;
    if !matches!(name.as_slice(), [name] if name.eq_ignore_ascii_case("quackgis_merge_adjacent_files"))
    {
        return Ok(None);
    }
    if function.uses_odbc_syntax
        || !matches!(function.parameters, FunctionArguments::None)
        || function.filter.is_some()
        || function.null_treatment.is_some()
        || function.over.is_some()
        || !function.within_group.is_empty()
    {
        return Err(user_error(
            "0A000",
            "unsupported maintenance procedure modifiers",
        ));
    }
    let FunctionArguments::List(arguments) = &function.args else {
        return Err(user_error(
            "42601",
            "maintenance procedure requires five literal arguments",
        ));
    };
    if arguments.duplicate_treatment.is_some()
        || !arguments.clauses.is_empty()
        || arguments.args.len() != 5
    {
        return Err(user_error(
            "42601",
            "maintenance procedure requires five literal arguments",
        ));
    }
    let expression = |index: usize| -> PgWireResult<&Expr> {
        match &arguments.args[index] {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(expression)) => Ok(expression),
            _ => Err(user_error(
                "42601",
                "maintenance procedure accepts positional literals only",
            )),
        }
    };
    let string_literal = |index: usize, label: &str| -> PgWireResult<String> {
        let Expr::Value(value) = expression(index)? else {
            return Err(user_error(
                "42601",
                &format!("{label} must be a string literal"),
            ));
        };
        let Value::SingleQuotedString(value) = &value.value else {
            return Err(user_error(
                "42601",
                &format!("{label} must be a string literal"),
            ));
        };
        if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
            return Err(user_error("22023", &format!("invalid maintenance {label}")));
        }
        Ok(value.clone())
    };
    let optional_u64 = |index: usize, label: &str| -> PgWireResult<Option<u64>> {
        let Expr::Value(value) = expression(index)? else {
            return Err(user_error(
                "42601",
                &format!("{label} must be a positive integer or NULL"),
            ));
        };
        match &value.value {
            Value::Null => Ok(None),
            Value::Number(value, false) => value
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
                .map(Some)
                .ok_or_else(|| {
                    user_error(
                        "22023",
                        &format!("{label} must be a positive integer or NULL"),
                    )
                }),
            _ => Err(user_error(
                "42601",
                &format!("{label} must be a positive integer or NULL"),
            )),
        }
    };
    let schema = string_literal(0, "schema")?;
    let schema = if schema.eq_ignore_ascii_case("main") || schema.eq_ignore_ascii_case("public") {
        "main".to_owned()
    } else {
        return Err(user_error(
            "0A000",
            "maintenance supports the public schema only",
        ));
    };
    let table = string_literal(1, "table")?;
    let max_compacted_files = optional_u64(2, "max_compacted_files")?;
    let max_file_size = optional_u64(3, "max_file_size")?;
    let min_file_size = optional_u64(4, "min_file_size")?;
    Ok(Some(MaintenanceCommand {
        request: EngineMaintenanceRequest::MergeAdjacentFiles {
            schema: schema.clone(),
            table: table.clone(),
            max_compacted_files,
            max_file_size,
            min_file_size,
        },
        schema,
        table,
    }))
}

fn supported_session_set(set: &Set) -> bool {
    let Set::SingleAssignment {
        scope,
        hivevar,
        variable,
        values,
    } = set
    else {
        return false;
    };
    if scope.is_some() || *hivevar || values.len() != 1 {
        return false;
    }
    let Some(name) = object_name_values(variable)
        .and_then(|parts| (parts.len() == 1).then(|| parts[0].to_ascii_lowercase()))
    else {
        return false;
    };
    let value = values[0]
        .to_string()
        .trim_matches('\'')
        .to_ascii_lowercase();
    match name.as_str() {
        "standard_conforming_strings" => value == "on",
        "client_encoding" => matches!(value.as_str(), "utf8" | "unicode"),
        "client_min_messages" => matches!(value.as_str(), "error" | "warning" | "notice"),
        "extra_float_digits" => value == "3",
        "datestyle" => value == "iso",
        "application_name" => match &values[0] {
            Expr::Value(value) => match &value.value {
                Value::SingleQuotedString(value) => {
                    value.len() <= 64 && !value.chars().any(char::is_control)
                }
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

fn validate_session_set_batch(sql: &str) -> PgWireResult<Option<usize>> {
    let normalized = normalize_sql(sql)?;
    let statements = Parser::parse_sql(&PostgreSqlDialect {}, &normalized)
        .map_err(|error| user_error("42601", &error.to_string()))?;
    if statements.len() <= 1 {
        return Ok(None);
    }
    if statements.len() > 8 {
        return Err(user_error(
            "54000",
            "session bootstrap batch exceeds eight SET statements",
        ));
    }
    if statements
        .iter()
        .all(|statement| matches!(statement, Statement::Set(set) if supported_session_set(set)))
    {
        Ok(Some(statements.len()))
    } else {
        Err(user_error(
            "0A000",
            "multi-statement simple queries are limited to maintained session SET batches",
        ))
    }
}

#[derive(Clone, Copy)]
enum SessionVariable {
    SearchPath,
    ClientEncoding,
    StandardConformingStrings,
}

fn supported_show_variable(variable: &[Ident]) -> Option<SessionVariable> {
    let [name] = variable else {
        return None;
    };
    if name.value.eq_ignore_ascii_case("search_path") {
        Some(SessionVariable::SearchPath)
    } else if name.value.eq_ignore_ascii_case("client_encoding") {
        Some(SessionVariable::ClientEncoding)
    } else if name
        .value
        .eq_ignore_ascii_case("standard_conforming_strings")
    {
        Some(SessionVariable::StandardConformingStrings)
    } else {
        None
    }
}

fn rewrite_public_relations(statement: &mut Statement) {
    let _: ControlFlow<()> = visit_relations_mut(statement, |name| {
        let table = match name.0.as_slice() {
            [
                ObjectNamePart::Identifier(schema),
                ObjectNamePart::Identifier(table),
            ] if schema.value.eq_ignore_ascii_case("public") => Some(table.clone()),
            [
                ObjectNamePart::Identifier(catalog),
                ObjectNamePart::Identifier(schema),
                ObjectNamePart::Identifier(table),
            ] if catalog.value.eq_ignore_ascii_case("quackgis")
                && schema.value.eq_ignore_ascii_case("public") =>
            {
                Some(table.clone())
            }
            _ => None,
        };
        if let Some(table) = table {
            *name = ObjectName(vec![
                ObjectNamePart::Identifier(Ident::new("quackgis")),
                ObjectNamePart::Identifier(Ident::new("main")),
                ObjectNamePart::Identifier(table),
            ]);
        }
        ControlFlow::Continue(())
    });
}

fn rewrite_pg_catalog_relations(statement: &mut Statement) {
    let _: ControlFlow<()> = visit_relations_mut(statement, |name| {
        if let Some(table) = maintained_pg_catalog_relation(name) {
            *name = ObjectName(vec![
                ObjectNamePart::Identifier(Ident::new("quackgis_pg_catalog")),
                ObjectNamePart::Identifier(Ident::new(table)),
            ]);
        }
        ControlFlow::Continue(())
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MaintainedPgFunction {
    Database,
    Schema,
    Schemas,
}

impl MaintainedPgFunction {
    const fn private_name(self) -> &'static str {
        match self {
            Self::Database => "quackgis_current_database",
            Self::Schema => "quackgis_current_schema",
            Self::Schemas => "quackgis_current_schemas",
        }
    }

    const fn result_hint(self) -> PgTypeHint {
        match self {
            Self::Database | Self::Schema => PgTypeHint::Name,
            Self::Schemas => PgTypeHint::NameArray,
        }
    }
}

fn maintained_pg_function(name: &ObjectName) -> Option<MaintainedPgFunction> {
    let function = match name.0.as_slice() {
        [ObjectNamePart::Identifier(function)] => function,
        [
            ObjectNamePart::Identifier(schema),
            ObjectNamePart::Identifier(function),
        ] if pg_identifier_matches(schema, "pg_catalog") => function,
        _ => return None,
    };
    if pg_identifier_matches(function, "current_database") {
        Some(MaintainedPgFunction::Database)
    } else if pg_identifier_matches(function, "current_schema") {
        Some(MaintainedPgFunction::Schema)
    } else if pg_identifier_matches(function, "current_schemas") {
        Some(MaintainedPgFunction::Schemas)
    } else {
        None
    }
}

fn rewrite_pg_catalog_functions(statement: &mut Statement) {
    let _: ControlFlow<()> = visit_expressions_mut(statement, |expression| {
        let Expr::Function(function) = expression else {
            return ControlFlow::Continue(());
        };
        if let Some(maintained) = maintained_pg_function(&function.name) {
            function.name = ObjectName(vec![ObjectNamePart::Identifier(Ident::new(
                maintained.private_name(),
            ))]);
        }
        ControlFlow::Continue(())
    });
}

fn maintained_pg_catalog_relation(name: &ObjectName) -> Option<&'static str> {
    if name.0.len() > 2 {
        return None;
    }
    let (table, _) = catalog_relation_identifier(name)?;
    [
        "pg_namespace",
        "pg_database",
        "pg_type",
        "pg_range",
        "pg_collation",
        "pg_roles",
    ]
    .into_iter()
    .find(|candidate| pg_identifier_matches(table, candidate))
}

fn catalog_relation_identifier(name: &ObjectName) -> Option<(&Ident, bool)> {
    match name.0.as_slice() {
        [ObjectNamePart::Identifier(table)] => Some((table, false)),
        [
            ObjectNamePart::Identifier(schema),
            ObjectNamePart::Identifier(table),
        ] if pg_identifier_matches(schema, "pg_catalog") => Some((table, true)),
        [
            ObjectNamePart::Identifier(_catalog),
            ObjectNamePart::Identifier(schema),
            ObjectNamePart::Identifier(table),
        ] if pg_identifier_matches(schema, "pg_catalog") => Some((table, true)),
        _ => None,
    }
}

fn unsupported_catalog_relation(statement: &Statement) -> Option<String> {
    let mut clone = statement.clone();
    let mut unsupported = None;
    let _: ControlFlow<()> = visit_relations_mut(&mut clone, |name| {
        let Some((table, explicitly_catalog)) = catalog_relation_identifier(name) else {
            return ControlFlow::Continue(());
        };
        let lower_table = table.value.to_ascii_lowercase();
        let unqualified_pg_name =
            lower_table.starts_with("pg_") && pg_identifier_matches(table, &lower_table);
        if maintained_pg_catalog_relation(name).is_none()
            && (explicitly_catalog || unqualified_pg_name)
        {
            unsupported = Some(table.value.to_ascii_lowercase());
        }
        if unsupported.is_some() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    unsupported
}

fn reserved_catalog_cte(statement: &Statement) -> Option<String> {
    let Statement::Query(query) = statement else {
        return None;
    };
    reserved_catalog_cte_in_query(query)
}

fn reserved_catalog_cte_in_query(query: &sqlparser::ast::Query) -> Option<String> {
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            let name = identifier_key(&cte.alias.name);
            if name.starts_with("pg_") {
                return Some(name);
            }
            if let Some(name) = reserved_catalog_cte_in_query(&cte.query) {
                return Some(name);
            }
        }
    }
    let mut reserved = None;
    let _: ControlFlow<()> = visit_expressions(query.body.as_ref(), |expression| {
        let subquery = match expression {
            Expr::InSubquery { subquery, .. }
            | Expr::Exists { subquery, .. }
            | Expr::Subquery(subquery) => Some(subquery),
            _ => None,
        };
        if let Some(subquery) = subquery
            && let Some(name) = reserved_catalog_cte_in_query(subquery)
        {
            reserved = Some(name);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    reserved
}

fn query_contains_table_command(statement: &Statement) -> bool {
    let Statement::Query(query) = statement else {
        return false;
    };
    query_contains_table_command_inner(query)
}

fn query_contains_table_command_inner(query: &sqlparser::ast::Query) -> bool {
    if query.with.as_ref().is_some_and(|with| {
        with.cte_tables
            .iter()
            .any(|cte| query_contains_table_command_inner(&cte.query))
    }) {
        return true;
    }
    if set_expr_contains_table_command(query.body.as_ref()) {
        return true;
    }
    let mut found = false;
    let _: ControlFlow<()> = visit_expressions(query, |expression| {
        let subquery = match expression {
            Expr::InSubquery { subquery, .. }
            | Expr::Exists { subquery, .. }
            | Expr::Subquery(subquery) => Some(subquery),
            _ => None,
        };
        if subquery.is_some_and(|query| query_contains_table_command_inner(query)) {
            found = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    found
}

fn set_expr_contains_table_command(expression: &SetExpr) -> bool {
    match expression {
        SetExpr::Table(_) => true,
        SetExpr::Query(query) => query_contains_table_command_inner(query),
        SetExpr::SetOperation { left, right, .. } => {
            set_expr_contains_table_command(left) || set_expr_contains_table_command(right)
        }
        SetExpr::Select(select) => select.from.iter().any(|table| {
            table_factor_contains_table_command(&table.relation)
                || table
                    .joins
                    .iter()
                    .any(|join| table_factor_contains_table_command(&join.relation))
        }),
        _ => false,
    }
}

fn table_factor_contains_table_command(factor: &TableFactor) -> bool {
    match factor {
        TableFactor::Derived { subquery, .. } => query_contains_table_command_inner(subquery),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            table_factor_contains_table_command(&table_with_joins.relation)
                || table_with_joins
                    .joins
                    .iter()
                    .any(|join| table_factor_contains_table_command(&join.relation))
        }
        TableFactor::Pivot { table, .. }
        | TableFactor::Unpivot { table, .. }
        | TableFactor::MatchRecognize { table, .. } => table_factor_contains_table_command(table),
        _ => false,
    }
}

fn catalog_query_shape_supported(statement: &Statement) -> bool {
    let mut clone = statement.clone();
    let mut relation_count = 0usize;
    let _: ControlFlow<()> = visit_relations_mut(&mut clone, |name| {
        if maintained_pg_catalog_relation(name).is_some() {
            relation_count += 1;
        }
        ControlFlow::Continue(())
    });
    if relation_count == 0 {
        return true;
    }
    let Statement::Query(query) = statement else {
        return false;
    };
    if query.with.is_some() {
        return false;
    }
    let SetExpr::Select(select) = query.body.as_ref() else {
        return false;
    };
    if select.from.iter().any(|table| {
        !matches!(&table.relation, TableFactor::Table { .. })
            || table.joins.iter().any(|join| {
                !matches!(&join.relation, TableFactor::Table { .. })
                    || join_uses_implicit_catalog_columns(&join.join_operator)
            })
    }) {
        return false;
    }
    let mut nested_or_stale = false;
    let _: ControlFlow<()> = visit_expressions(statement, |expression| {
        if matches!(
            expression,
            Expr::InSubquery { .. } | Expr::Exists { .. } | Expr::Subquery(_)
        ) || stale_catalog_column_qualifier(expression)
        {
            nested_or_stale = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    if nested_or_stale {
        return false;
    }
    if select.projection.iter().any(|item| {
        matches!(
            item,
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _)
        )
    }) {
        return false;
    }
    let aliases = top_level_catalog_aliases(statement);
    if select.projection.iter().any(|item| {
        let expression = match item {
            SelectItem::UnnamedExpr(expression)
            | SelectItem::ExprWithAlias {
                expr: expression, ..
            } => expression,
            _ => return false,
        };
        catalog_expression_hint(expression, &aliases).is_none()
            && expression_contains_catalog_column(expression, &aliases)
    }) {
        return false;
    }
    aliases.len() == relation_count
}

fn join_uses_implicit_catalog_columns(operator: &JoinOperator) -> bool {
    let constraint = match operator {
        JoinOperator::Join(constraint)
        | JoinOperator::Inner(constraint)
        | JoinOperator::Left(constraint)
        | JoinOperator::LeftOuter(constraint)
        | JoinOperator::Right(constraint)
        | JoinOperator::RightOuter(constraint)
        | JoinOperator::FullOuter(constraint)
        | JoinOperator::CrossJoin(constraint)
        | JoinOperator::Semi(constraint)
        | JoinOperator::LeftSemi(constraint)
        | JoinOperator::RightSemi(constraint)
        | JoinOperator::Anti(constraint)
        | JoinOperator::LeftAnti(constraint)
        | JoinOperator::RightAnti(constraint)
        | JoinOperator::StraightJoin(constraint) => Some(constraint),
        JoinOperator::AsOf { constraint, .. } => Some(constraint),
        _ => None,
    };
    matches!(
        constraint,
        Some(JoinConstraint::Using(_) | JoinConstraint::Natural)
    )
}

fn expression_contains_catalog_column(
    expression: &Expr,
    aliases: &HashMap<String, &'static str>,
) -> bool {
    let mut found = false;
    let _: ControlFlow<()> = visit_expressions(expression, |nested| {
        if catalog_expression_hint(nested, aliases).is_some() {
            found = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    found
}

fn stale_catalog_column_qualifier(expression: &Expr) -> bool {
    let Expr::CompoundIdentifier(identifiers) = expression else {
        return false;
    };
    let [schema, table, _column] = identifiers.as_slice() else {
        return false;
    };
    pg_identifier_matches(schema, "pg_catalog")
        && [
            "pg_namespace",
            "pg_database",
            "pg_type",
            "pg_range",
            "pg_collation",
            "pg_roles",
        ]
        .into_iter()
        .any(|candidate| pg_identifier_matches(table, candidate))
}

fn private_catalog_reference(statement: &Statement) -> bool {
    let mut clone = statement.clone();
    let mut found = false;
    let _: ControlFlow<()> = visit_relations_mut(&mut clone, |name| {
        let private_schema = match name.0.as_slice() {
            [
                ObjectNamePart::Identifier(schema),
                ObjectNamePart::Identifier(_table),
            ] => Some(schema),
            [
                ObjectNamePart::Identifier(_catalog),
                ObjectNamePart::Identifier(schema),
                ObjectNamePart::Identifier(_table),
            ] => Some(schema),
            _ => None,
        };
        if private_schema
            .is_some_and(|schema| schema.value.eq_ignore_ascii_case("quackgis_pg_catalog"))
        {
            found = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    if found {
        return true;
    }
    let _: ControlFlow<()> = visit_expressions(statement, |expression| {
        if let Expr::Function(function) = expression
            && function.name.0.iter().any(|part| {
                matches!(part, ObjectNamePart::Identifier(identifier) if identifier.value.to_ascii_lowercase().starts_with("quackgis_current_"))
            })
        {
            found = true;
            return ControlFlow::Break(());
        }
        let Expr::CompoundIdentifier(identifiers) = expression else {
            return ControlFlow::Continue(());
        };
        if identifiers
            .iter()
            .take(identifiers.len().saturating_sub(1))
            .any(|identifier| identifier.value.eq_ignore_ascii_case("quackgis_pg_catalog"))
        {
            found = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    found
}

fn internal_control_schema_reference(statement: &Statement) -> bool {
    let mut clone = statement.clone();
    let mut found = false;
    let _: ControlFlow<()> = visit_relations_mut(&mut clone, |name| {
        let schema = match name.0.as_slice() {
            [
                ObjectNamePart::Identifier(schema),
                ObjectNamePart::Identifier(_table),
            ] => Some(schema),
            [
                ObjectNamePart::Identifier(_catalog),
                ObjectNamePart::Identifier(schema),
                ObjectNamePart::Identifier(_table),
            ] => Some(schema),
            _ => None,
        };
        if schema.is_some_and(|schema| {
            schema
                .value
                .eq_ignore_ascii_case(crate::postgres_compat::INTERNAL_SCHEMA)
        }) {
            found = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    found
}

fn forbidden_catalog_table_function(statement: &Statement) -> Option<String> {
    let mut clone = statement.clone();
    let mut forbidden = None;
    let _: ControlFlow<()> = visit_relations_mut(&mut clone, |name| {
        if let Some(name) = forbidden_catalog_function_name(name) {
            forbidden = Some(name);
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    if forbidden.is_some() {
        return forbidden;
    }
    match statement {
        Statement::Query(query) => forbidden_catalog_function_in_query(query),
        _ => None,
    }
}

fn forbidden_catalog_function_name(name: &ObjectName) -> Option<String> {
    let function = match name.0.last() {
        Some(ObjectNamePart::Identifier(function)) => function,
        _ => return None,
    };
    let lower = function.value.to_ascii_lowercase();
    ["query", "query_table", "ducklake_column_info"]
        .contains(&lower.as_str())
        .then_some(lower)
}

fn forbidden_catalog_function_in_query(query: &sqlparser::ast::Query) -> Option<String> {
    if let Some(found) = query
        .with
        .as_ref()
        .and_then(|with| {
            with.cte_tables
                .iter()
                .find_map(|cte| forbidden_catalog_function_in_query(&cte.query))
        })
        .or_else(|| forbidden_catalog_function_in_set(query.body.as_ref()))
    {
        return Some(found);
    }
    let mut forbidden = None;
    let _: ControlFlow<()> = visit_expressions(query.body.as_ref(), |expression| {
        let subquery = match expression {
            Expr::InSubquery { subquery, .. }
            | Expr::Exists { subquery, .. }
            | Expr::Subquery(subquery) => Some(subquery),
            _ => None,
        };
        if let Some(found) = subquery.and_then(|query| forbidden_catalog_function_in_query(query)) {
            forbidden = Some(found);
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    forbidden
}

fn forbidden_catalog_function_in_set(expression: &SetExpr) -> Option<String> {
    match expression {
        SetExpr::Select(select) => select.from.iter().find_map(|table| {
            forbidden_catalog_function_in_factor(&table.relation).or_else(|| {
                table
                    .joins
                    .iter()
                    .find_map(|join| forbidden_catalog_function_in_factor(&join.relation))
            })
        }),
        SetExpr::Query(query) => forbidden_catalog_function_in_query(query),
        SetExpr::SetOperation { left, right, .. } => forbidden_catalog_function_in_set(left)
            .or_else(|| forbidden_catalog_function_in_set(right)),
        _ => None,
    }
}

fn forbidden_catalog_function_in_factor(factor: &TableFactor) -> Option<String> {
    match factor {
        TableFactor::Table {
            name,
            args: Some(_),
            ..
        }
        | TableFactor::Function { name, .. } => forbidden_catalog_function_name(name),
        TableFactor::TableFunction {
            expr: Expr::Function(function),
            ..
        } => forbidden_catalog_function_name(&function.name),
        TableFactor::TableFunction { .. } => None,
        TableFactor::Derived { subquery, .. } => forbidden_catalog_function_in_query(subquery),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => forbidden_catalog_function_in_factor(&table_with_joins.relation).or_else(|| {
            table_with_joins
                .joins
                .iter()
                .find_map(|join| forbidden_catalog_function_in_factor(&join.relation))
        }),
        TableFactor::Pivot { table, .. }
        | TableFactor::Unpivot { table, .. }
        | TableFactor::MatchRecognize { table, .. } => forbidden_catalog_function_in_factor(table),
        _ => None,
    }
}

fn invalid_quoted_catalog_reference(statement: &Statement) -> bool {
    let mut clone = statement.clone();
    let mut invalid_relation = false;
    let _: ControlFlow<()> = visit_relations_mut(&mut clone, |name| {
        let (schema, table) = match name.0.as_slice() {
            [ObjectNamePart::Identifier(table)] => (None, table),
            [
                ObjectNamePart::Identifier(schema),
                ObjectNamePart::Identifier(table),
            ] => (Some(schema), table),
            [
                ObjectNamePart::Identifier(_catalog),
                ObjectNamePart::Identifier(schema),
                ObjectNamePart::Identifier(table),
            ] => (Some(schema), table),
            _ => return ControlFlow::Continue(()),
        };
        let catalog_schema = schema.is_none()
            || schema.is_some_and(|schema| schema.value.eq_ignore_ascii_case("pg_catalog"));
        let invalid_schema = schema.is_some_and(|schema| {
            schema.value.eq_ignore_ascii_case("pg_catalog")
                && !pg_identifier_matches(schema, "pg_catalog")
        });
        let lower_table = table.value.to_ascii_lowercase();
        let invalid_table = table.quote_style.is_some()
            && lower_table.starts_with("pg_")
            && table.value != lower_table;
        if catalog_schema && (invalid_schema || invalid_table) {
            invalid_relation = true;
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    if invalid_relation {
        return true;
    }

    let aliases = top_level_catalog_aliases(statement);
    let mut invalid_column = false;
    let _: ControlFlow<()> = visit_expressions(statement, |expression| {
        if invalid_quoted_catalog_column(expression, &aliases) {
            invalid_column = true;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    invalid_column
}

fn invalid_quoted_catalog_column(
    expression: &Expr,
    aliases: &HashMap<String, &'static str>,
) -> bool {
    let invalid = |relation: &str, column: &Ident| {
        column.quote_style.is_some()
            && catalog_column_names(relation)
                .iter()
                .any(|name| column.value.eq_ignore_ascii_case(name) && column.value != *name)
    };
    match expression {
        Expr::CompoundIdentifier(identifiers) if identifiers.len() == 2 => {
            let qualifier = &identifiers[0];
            let key = identifier_key(qualifier);
            if !aliases.contains_key(&key)
                && aliases
                    .keys()
                    .any(|alias| alias.eq_ignore_ascii_case(&qualifier.value))
            {
                return true;
            }
            aliases
                .get(&key)
                .is_some_and(|relation| invalid(relation, &identifiers[1]))
        }
        Expr::Identifier(column) if aliases.len() == 1 => aliases
            .values()
            .next()
            .is_some_and(|relation| invalid(relation, column)),
        Expr::BinaryOp { left, right, .. } => {
            invalid_quoted_catalog_column(left, aliases)
                || invalid_quoted_catalog_column(right, aliases)
        }
        Expr::Nested(expression) => invalid_quoted_catalog_column(expression, aliases),
        _ => false,
    }
}

fn pg_identifier_matches(identifier: &Ident, expected: &str) -> bool {
    if identifier.quote_style.is_some() {
        identifier.value == expected
    } else {
        identifier.value.eq_ignore_ascii_case(expected)
    }
}

fn identifier_key(identifier: &Ident) -> String {
    if identifier.quote_style.is_some() {
        identifier.value.clone()
    } else {
        identifier.value.to_ascii_lowercase()
    }
}

fn top_level_catalog_aliases(statement: &Statement) -> HashMap<String, &'static str> {
    let Statement::Query(query) = statement else {
        return HashMap::new();
    };
    let SetExpr::Select(select) = query.body.as_ref() else {
        return HashMap::new();
    };
    let mut aliases = HashMap::new();
    for table in &select.from {
        collect_catalog_alias(&table.relation, &mut aliases);
        for join in &table.joins {
            collect_catalog_alias(&join.relation, &mut aliases);
        }
    }
    aliases
}

fn collect_catalog_alias(factor: &TableFactor, aliases: &mut HashMap<String, &'static str>) {
    let TableFactor::Table { name, alias, .. } = factor else {
        return;
    };
    let Some(relation) = maintained_pg_catalog_relation(name) else {
        return;
    };
    let identifier = alias
        .as_ref()
        .map(|alias| &alias.name)
        .or_else(|| match name.0.last() {
            Some(ObjectNamePart::Identifier(identifier)) => Some(identifier),
            _ => None,
        });
    if let Some(identifier) = identifier {
        aliases.insert(identifier_key(identifier), relation);
    }
}

fn catalog_columns(relation: &str) -> &'static [(&'static str, PgTypeHint)] {
    match relation {
        "pg_namespace" => &[
            ("oid", PgTypeHint::Oid),
            ("nspname", PgTypeHint::Name),
            ("nspowner", PgTypeHint::Oid),
        ],
        "pg_database" => &[
            ("oid", PgTypeHint::Oid),
            ("datname", PgTypeHint::Name),
            ("datdba", PgTypeHint::Oid),
        ],
        "pg_type" => &[
            ("oid", PgTypeHint::Oid),
            ("typname", PgTypeHint::Name),
            ("typnamespace", PgTypeHint::Oid),
            ("typtype", PgTypeHint::Char),
            ("typcategory", PgTypeHint::Char),
            ("typdelim", PgTypeHint::Char),
            ("typrelid", PgTypeHint::Oid),
            ("typelem", PgTypeHint::Oid),
            ("typarray", PgTypeHint::Oid),
            ("typbasetype", PgTypeHint::Oid),
            ("typcollation", PgTypeHint::Oid),
        ],
        "pg_range" => &[
            ("rngtypid", PgTypeHint::Oid),
            ("rngsubtype", PgTypeHint::Oid),
        ],
        "pg_collation" => &[
            ("oid", PgTypeHint::Oid),
            ("collname", PgTypeHint::Name),
            ("collnamespace", PgTypeHint::Oid),
            ("collowner", PgTypeHint::Oid),
            ("collprovider", PgTypeHint::Char),
        ],
        "pg_roles" => &[("rolname", PgTypeHint::Name), ("oid", PgTypeHint::Oid)],
        _ => &[],
    }
}

fn catalog_column_names(relation: &str) -> &'static [&'static str] {
    match relation {
        "pg_namespace" => &["oid", "nspname", "nspowner"],
        "pg_database" => &["oid", "datname", "datdba"],
        "pg_type" => &[
            "oid",
            "typname",
            "typnamespace",
            "typlen",
            "typbyval",
            "typtype",
            "typcategory",
            "typispreferred",
            "typisdefined",
            "typdelim",
            "typrelid",
            "typelem",
            "typarray",
            "typnotnull",
            "typbasetype",
            "typtypmod",
            "typndims",
            "typcollation",
        ],
        "pg_range" => &["rngtypid", "rngsubtype"],
        "pg_collation" => &[
            "oid",
            "collname",
            "collnamespace",
            "collowner",
            "collprovider",
            "collisdeterministic",
            "collencoding",
            "collcollate",
            "collctype",
            "colllocale",
            "collicurules",
            "collversion",
        ],
        "pg_roles" => &["oid", "rolname"],
        _ => &[],
    }
}

fn catalog_column_hint(relation: &str, column: &Ident) -> Option<PgTypeHint> {
    catalog_columns(relation)
        .iter()
        .find_map(|(name, hint)| pg_identifier_matches(column, name).then_some(*hint))
}

fn catalog_expression_hint(
    expression: &Expr,
    aliases: &HashMap<String, &'static str>,
) -> Option<PgTypeHint> {
    match expression {
        Expr::CompoundIdentifier(identifiers) if identifiers.len() == 2 => {
            let relation = aliases.get(&identifier_key(&identifiers[0]))?;
            catalog_column_hint(relation, &identifiers[1])
        }
        Expr::Identifier(column) if aliases.len() == 1 => {
            catalog_column_hint(aliases.values().next()?, column)
        }
        Expr::Nested(expression) => catalog_expression_hint(expression, aliases),
        _ => None,
    }
}

fn annotate_catalog_result_schema(statement: &Statement, schema: &Schema) -> Schema {
    let aliases = top_level_catalog_aliases(statement);
    let Statement::Query(query) = statement else {
        return schema.clone();
    };
    let SetExpr::Select(select) = query.body.as_ref() else {
        return schema.clone();
    };
    if select.projection.len() != schema.fields().len() {
        return schema.clone();
    }
    Schema::new(
        schema
            .fields()
            .iter()
            .zip(&select.projection)
            .map(|(field, item)| {
                let expression = match item {
                    SelectItem::UnnamedExpr(expression)
                    | SelectItem::ExprWithAlias {
                        expr: expression, ..
                    } => expression,
                    _ => return field.as_ref().clone(),
                };
                let hint = catalog_expression_hint(expression, &aliases)
                    .or_else(|| maintained_function_hint(expression));
                hint.map(|hint| with_pg_type_hint(field.as_ref().clone(), hint))
                    .unwrap_or_else(|| field.as_ref().clone())
            })
            .collect::<Vec<_>>(),
    )
}

fn maintained_function_hint(expression: &Expr) -> Option<PgTypeHint> {
    let Expr::Function(function) = expression else {
        return None;
    };
    maintained_pg_function(&function.name).map(MaintainedPgFunction::result_hint)
}

fn catalog_oid_parameter_indexes(statement: &Statement) -> HashSet<usize> {
    let aliases = top_level_catalog_aliases(statement);
    if aliases.is_empty() {
        return HashSet::new();
    }
    let Statement::Query(query) = statement else {
        return HashSet::new();
    };
    let SetExpr::Select(select) = query.body.as_ref() else {
        return HashSet::new();
    };
    let mut indexes = HashSet::new();
    if let Some(selection) = &select.selection {
        collect_catalog_oid_parameters(selection, &aliases, &mut indexes);
    }
    indexes
}

fn collect_catalog_oid_parameters(
    expression: &Expr,
    aliases: &HashMap<String, &'static str>,
    indexes: &mut HashSet<usize>,
) {
    match expression {
        Expr::BinaryOp { left, op, right } => {
            if matches!(op, BinaryOperator::Eq) {
                for (column, value) in [
                    (left.as_ref(), right.as_ref()),
                    (right.as_ref(), left.as_ref()),
                ] {
                    if catalog_expression_hint(column, aliases) == Some(PgTypeHint::Oid)
                        && let Some(index) = numbered_parameter_index(value)
                    {
                        indexes.insert(index);
                    }
                }
            }
            if matches!(op, BinaryOperator::And | BinaryOperator::Or) {
                collect_catalog_oid_parameters(left, aliases, indexes);
                collect_catalog_oid_parameters(right, aliases, indexes);
            }
        }
        Expr::Nested(expression) => collect_catalog_oid_parameters(expression, aliases, indexes),
        _ => {}
    }
}

fn numbered_parameter_index(expression: &Expr) -> Option<usize> {
    let Expr::Value(value) = expression else {
        return None;
    };
    let Value::Placeholder(placeholder) = &value.value else {
        return None;
    };
    placeholder
        .strip_prefix('$')?
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}

fn object_name_values(name: &ObjectName) -> Option<Vec<String>> {
    name.0
        .iter()
        .map(|part| match part {
            ObjectNamePart::Identifier(ident) => Some(ident.value.clone()),
            _ => None,
        })
        .collect()
}

fn authorize_statement<C>(client: &C, auth: &AuthConfig, statement: &Statement) -> PgWireResult<()>
where
    C: ClientInfo + ?Sized,
{
    crate::statement_policy::authorize_statement(
        auth,
        client.metadata().get("user").map(String::as_str),
        statement,
    )
    .map_err(engine_error)
}

fn authorize_copy<C>(client: &C, auth: &AuthConfig, target: &CopyTarget) -> PgWireResult<()>
where
    C: ClientInfo + ?Sized,
{
    crate::statement_policy::authorize_copy_target(
        auth,
        client.metadata().get("user").map(String::as_str),
        &target.table.schema,
        &target.table.table,
    )
    .map_err(engine_error)
}

fn authorize_maintenance<C>(
    client: &C,
    auth: &AuthConfig,
    command: &MaintenanceCommand,
) -> PgWireResult<()>
where
    C: ClientInfo + ?Sized,
{
    let user = client.metadata().get("user").map(String::as_str);
    if auth.allows_maintenance(user, (&command.schema, &command.table)) {
        return Ok(());
    }
    let user = user.unwrap_or("<unknown>");
    let target = command.target_label();
    crate::audit::log_authorization_denied(
        user,
        "maintenance",
        &target,
        "maintenance_identity_or_table_policy",
    );
    Err(user_error(
        "42501",
        "maintenance requires the configured maintenance identity and table policy",
    ))
}

fn normalize_sql(sql: &str) -> PgWireResult<String> {
    let sql = sql.trim();
    if sql.is_empty() {
        return Err(user_error("42601", "SQL statement must not be empty"));
    }
    Ok(sql.strip_suffix(';').unwrap_or(sql).trim().to_owned())
}

fn parse_copy_target(sql: &str) -> PgWireResult<Option<CopyTarget>> {
    let normalized = normalize_sql(sql)?;
    if !normalized
        .split_whitespace()
        .next()
        .is_some_and(|token| token.eq_ignore_ascii_case("copy"))
    {
        return Ok(None);
    }
    let mut statements = Parser::parse_sql(&PostgreSqlDialect {}, &normalized)
        .map_err(|error| user_error("42601", &error.to_string()))?;
    if statements.len() != 1 {
        return Err(user_error(
            "0A000",
            "DuckDB COPY requires exactly one statement",
        ));
    }
    let Statement::Copy {
        source: CopySource::Table {
            table_name,
            columns,
        },
        to: false,
        target: AstCopyTarget::Stdin,
        options,
        legacy_options,
        values,
    } = statements.pop().expect("one COPY statement")
    else {
        return Err(user_error(
            "0A000",
            "only COPY table (columns) FROM STDIN is supported",
        ));
    };
    if !options.is_empty() || !legacy_options.is_empty() || !values.is_empty() {
        return Err(user_error("0A000", "COPY options are not supported"));
    }
    let columns = columns
        .into_iter()
        .map(|column| column.value)
        .collect::<Vec<_>>();
    if columns.is_empty() {
        return Err(user_error(
            "42601",
            "DuckDB COPY requires an explicit column list",
        ));
    }
    let parts = object_name_values(&table_name)
        .ok_or_else(|| user_error("42601", "COPY target must contain identifiers only"))?;
    let (catalog, schema, table) = match parts.as_slice() {
        [table] => ("quackgis", "main", table.as_str()),
        [schema, table]
            if schema.eq_ignore_ascii_case("public") || schema.eq_ignore_ascii_case("main") =>
        {
            ("quackgis", "main", table.as_str())
        }
        [catalog, schema, table]
            if catalog.eq_ignore_ascii_case("quackgis")
                && (schema.eq_ignore_ascii_case("public")
                    || schema.eq_ignore_ascii_case("main")) =>
        {
            ("quackgis", "main", table.as_str())
        }
        _ => {
            return Err(user_error(
                "0A000",
                "COPY target must be table, public.table, or quackgis.main.table",
            ));
        }
    };
    Ok(Some(CopyTarget {
        table: EngineTableRef {
            catalog: catalog.to_owned(),
            schema: schema.to_owned(),
            table: table.to_owned(),
        },
        columns,
    }))
}

fn parameter_batch(
    portal: &Portal<DuckDbStatement>,
    statement: &DuckDbStatement,
) -> PgWireResult<Option<RecordBatch>> {
    if statement.parameter_schema.fields().is_empty() {
        if portal.parameter_len() != 0 {
            return Err(user_error("08P01", "unexpected bound parameters"));
        }
        return Ok(None);
    }
    if portal.parameter_len() != statement.parameter_schema.fields().len() {
        return Err(user_error("08P01", "bound parameter count mismatch"));
    }
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(portal.parameter_len());
    for (index, field) in statement.parameter_schema.fields().iter().enumerate() {
        let pg_type = &statement.parameter_types[index];
        let array: ArrayRef = match field.data_type() {
            DataType::Boolean => Arc::new(BooleanArray::from(vec![
                portal.parameter::<bool>(index, pg_type)?,
            ])),
            DataType::Int16 => Arc::new(Int16Array::from(vec![
                portal.parameter::<i16>(index, pg_type)?,
            ])),
            DataType::Int32 => Arc::new(Int32Array::from(vec![
                portal.parameter::<i32>(index, pg_type)?,
            ])),
            DataType::Int64 => Arc::new(Int64Array::from(vec![
                portal.parameter::<i64>(index, pg_type)?,
            ])),
            DataType::UInt32 => Arc::new(UInt32Array::from(vec![
                portal.parameter::<u32>(index, pg_type)?,
            ])),
            DataType::Float32 => Arc::new(Float32Array::from(vec![
                portal.parameter::<f32>(index, pg_type)?,
            ])),
            DataType::Float64 => Arc::new(Float64Array::from(vec![
                portal.parameter::<f64>(index, pg_type)?,
            ])),
            DataType::Utf8 => Arc::new(StringArray::from(vec![
                portal.parameter::<String>(index, pg_type)?,
            ])),
            DataType::Binary => {
                let value = portal.parameter::<Vec<u8>>(index, pg_type)?;
                Arc::new(BinaryArray::from_opt_vec(vec![value.as_deref()]))
            }
            unsupported => {
                return Err(user_error(
                    "0A000",
                    &format!("unsupported DuckDB pgwire parameter type {unsupported}"),
                ));
            }
        };
        arrays.push(array);
    }
    RecordBatch::try_new(Arc::clone(&statement.parameter_schema), arrays)
        .map(Some)
        .map_err(|error| user_error("22000", &error.to_string()))
}

struct StreamingRowState {
    query: Option<EngineQueryStream>,
    current_rows: Option<Box<dyn Iterator<Item = PgWireResult<DataRow>> + Send>>,
    fields: Arc<Vec<pgwire::api::results::FieldInfo>>,
    blocking_workers: Arc<BlockingWorkerPool>,
}

struct MeasuredRows {
    rows: Box<dyn Iterator<Item = PgWireResult<DataRow>> + Send>,
    active: bool,
}

impl MeasuredRows {
    fn finish(&mut self) {
        if self.active {
            self.active = false;
            crate::metrics::query_batch_finished();
        }
    }
}

impl Iterator for MeasuredRows {
    type Item = PgWireResult<DataRow>;

    fn next(&mut self) -> Option<Self::Item> {
        let row = self.rows.next();
        if row.is_none() {
            self.finish();
        }
        row
    }
}

impl Drop for MeasuredRows {
    fn drop(&mut self) {
        self.finish();
    }
}

fn validate_result_batch(batch: &RecordBatch, max_bytes: usize) -> PgWireResult<usize> {
    let bytes = batch.get_array_memory_size();
    if bytes > max_bytes {
        crate::metrics::query_batch_rejected();
        return Err(user_error(
            "54000",
            "DuckDB result batch exceeds the configured Arrow batch byte limit",
        ));
    }
    Ok(bytes)
}

fn query_response(
    result: EngineQueryStream,
    format: &Format,
    max_batch_bytes: usize,
    blocking_workers: Arc<BlockingWorkerPool>,
    result_schema: Option<&Schema>,
) -> PgWireResult<QueryResponse> {
    let fields = Arc::new(arrow_schema_to_pg_fields(
        result_schema.unwrap_or(result.schema.as_ref()),
        format,
        None,
    )?);
    let rows = futures::stream::try_unfold(
        StreamingRowState {
            query: Some(result),
            current_rows: None,
            fields: Arc::clone(&fields),
            blocking_workers,
        },
        move |mut state| async move {
            loop {
                if let Some(rows) = state.current_rows.as_mut() {
                    if let Some(row) = rows.next() {
                        return row.map(|row| Some((row, state)));
                    }
                    state.current_rows = None;
                }
                let mut query = state.query.take().ok_or_else(|| {
                    user_error("XX000", "DuckDB query stream lost its native reader")
                })?;
                let blocking_workers = Arc::clone(&state.blocking_workers);
                let (returned, batch) = blocking_workers
                    .run_regular(move || {
                        let batch = query.next_batch();
                        (query, batch)
                    })
                    .await
                    .map_err(blocking_worker_error)?;
                state.query = Some(returned);
                match batch.map_err(engine_error)? {
                    Some(batch) => {
                        let bytes = validate_result_batch(&batch, max_batch_bytes)?;
                        crate::metrics::query_batch_started(bytes);
                        state.current_rows = Some(Box::new(MeasuredRows {
                            rows: encode_recordbatch(Arc::clone(&state.fields), batch),
                            active: true,
                        }));
                    }
                    None => return Ok(None),
                }
            }
        },
    );
    Ok(QueryResponse::new(fields, rows))
}

fn engine_error(error: EngineError) -> PgWireError {
    let sqlstate = error.sqlstate.clone().unwrap_or_else(|| match error.kind {
        EngineErrorKind::Unsupported => "0A000".to_owned(),
        EngineErrorKind::NotFound => "42P01".to_owned(),
        EngineErrorKind::AlreadyExists => "42P07".to_owned(),
        EngineErrorKind::Constraint => "23000".to_owned(),
        EngineErrorKind::Unauthorized => "42501".to_owned(),
        EngineErrorKind::Cancelled => "57014".to_owned(),
        EngineErrorKind::Timeout => "57014".to_owned(),
        EngineErrorKind::InvalidQuery => "42601".to_owned(),
        EngineErrorKind::Io => "58030".to_owned(),
        EngineErrorKind::Busy => "55000".to_owned(),
        EngineErrorKind::Internal
        | EngineErrorKind::IndeterminateCommit
        | EngineErrorKind::Quarantined => "XX000".to_owned(),
    });
    user_error(&sqlstate, error.message())
}

fn admission_error(error: AdmissionError) -> PgWireError {
    match error {
        AdmissionError::QueueFull => user_error("53400", "DuckDB query admission queue is full"),
        AdmissionError::QueueTimeout => {
            user_error("57014", "canceling statement due to queue timeout")
        }
        AdmissionError::Closed => user_error("57P01", "DuckDB query admission is unavailable"),
    }
}

fn failed_transaction_error() -> PgWireError {
    user_error(
        "25P02",
        "current transaction is aborted, commands ignored until end of transaction block",
    )
}

fn anyhow_error(error: anyhow::Error) -> PgWireError {
    user_error("XX000", &error.to_string())
}

fn fatal_anyhow_error(error: anyhow::Error) -> PgWireError {
    PgWireError::UserError(Box::new(ErrorInfo::new(
        "FATAL".to_owned(),
        "XX000".to_owned(),
        error.to_string(),
    )))
}

async fn client_session<C>(
    client: &C,
    database: Arc<DuckDbAdbcStorage>,
    blocking_workers: Arc<BlockingWorkerPool>,
) -> PgWireResult<Arc<DuckDbAdbcStorage>>
where
    C: ClientInfo + Unpin + Send + Sync,
{
    if let Some(session) = client.session_extensions().get::<DuckDbAdbcStorage>() {
        return Ok(session);
    }
    let session = blocking_workers
        .run_regular(move || database.open_session())
        .await
        .map_err(blocking_worker_error)?
        .map_err(anyhow_error)?;
    client.session_extensions().insert(session);
    client
        .session_extensions()
        .get::<DuckDbAdbcStorage>()
        .ok_or_else(|| user_error("XX000", "failed to initialize DuckDB client session"))
}

#[derive(Default)]
struct CopySessionState {
    request: Mutex<Option<CopyRequest>>,
}

struct CopyRequest {
    decoder: CopyTextDecoder,
    sender: Option<tokio::sync::mpsc::Sender<CopyBatchMessage>>,
    worker: Option<tokio::task::JoinHandle<EngineResult<Option<i64>>>>,
    cancellation: Arc<CopyCancellation>,
    rows: usize,
    bytes: usize,
    batches: usize,
    max_chunk_bytes: usize,
    started_at: std::time::Instant,
    metrics_recorded: Arc<AtomicBool>,
    finished: bool,
}

impl Drop for CopyRequest {
    fn drop(&mut self) {
        if !self.finished {
            self.cancellation.abort_input();
        }
        if self
            .metrics_recorded
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            crate::metrics::copy_failed(self.started_at.elapsed());
        }
    }
}

enum CopyBatchMessage {
    Batch(RecordBatch),
    Finish,
    Abort,
}

struct CopyBatchReader {
    schema: SchemaRef,
    receiver: tokio::sync::mpsc::Receiver<CopyBatchMessage>,
    aborted: Arc<AtomicBool>,
    finished: bool,
}

impl Iterator for CopyBatchReader {
    type Item = Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if self.aborted.load(Ordering::Acquire) {
            self.finished = true;
            return Some(Err(ArrowError::ExternalError(Box::new(
                std::io::Error::new(std::io::ErrorKind::Interrupted, "COPY input was aborted"),
            ))));
        }
        match self.receiver.blocking_recv() {
            Some(CopyBatchMessage::Batch(batch)) => Some(Ok(batch)),
            Some(CopyBatchMessage::Finish) => {
                self.finished = true;
                None
            }
            Some(CopyBatchMessage::Abort) => {
                self.finished = true;
                Some(Err(ArrowError::ExternalError(Box::new(
                    std::io::Error::new(std::io::ErrorKind::Interrupted, "COPY input was aborted"),
                ))))
            }
            None => {
                self.finished = true;
                Some(Err(ArrowError::ExternalError(Box::new(
                    std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "COPY input closed before completion",
                    ),
                ))))
            }
        }
    }
}

impl RecordBatchReader for CopyBatchReader {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

struct CopyCancellation {
    aborted: Arc<AtomicBool>,
    wake: tokio::sync::mpsc::Sender<CopyBatchMessage>,
    native: Arc<dyn EngineCancellation>,
}

impl CopyCancellation {
    fn abort_input(&self) {
        self.aborted.store(true, Ordering::Release);
        let _ = self.wake.try_send(CopyBatchMessage::Abort);
    }
}

impl EngineCancellation for CopyCancellation {
    fn cancel(&self) -> EngineResult<()> {
        self.abort_input();
        self.native.cancel()
    }
}

async fn begin_copy<C>(
    client: &C,
    storage: Arc<DuckDbAdbcStorage>,
    target: CopyTarget,
    control: &DuckDbRuntimeControl,
) -> PgWireResult<Response>
where
    C: ClientInfo + Unpin + Send + Sync,
{
    let permit = control
        .admission
        .acquire(OperationClass::Writer)
        .await
        .map_err(admission_error)?;
    let table = target.table.clone();
    let schema_storage = Arc::clone(&storage);
    let full_schema = control
        .blocking_workers
        .run_regular(move || schema_storage.table_schema(&table))
        .await
        .map_err(blocking_worker_error)?
        .map_err(engine_error)?;
    let fields = target
        .columns
        .iter()
        .map(|column| {
            full_schema
                .field_with_name(column)
                .cloned()
                .map_err(|_| user_error("42703", &format!("COPY column does not exist: {column}")))
        })
        .collect::<PgWireResult<Vec<_>>>()?;
    let schema = Arc::new(Schema::new(fields));
    let state = client
        .session_extensions()
        .get_or_insert_with(CopySessionState::default);
    {
        let request = state
            .request
            .lock()
            .map_err(|_| user_error("XX000", "DuckDB COPY state is poisoned"))?;
        if request.is_some() {
            return Err(user_error(
                "55000",
                "another COPY operation is already active",
            ));
        }
    }

    let operation_storage = Arc::clone(&storage);
    let operation = control
        .blocking_workers
        .run_regular(move || operation_storage.start_ingest_operation())
        .await
        .map_err(blocking_worker_error)?
        .map_err(engine_error)?;
    let native = operation.cancellation();
    let aborted = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let cancellation = Arc::new(CopyCancellation {
        aborted: Arc::clone(&aborted),
        wake: sender.clone(),
        native,
    });
    let reader = CopyBatchReader {
        schema: Arc::clone(&schema),
        receiver,
        aborted,
        finished: false,
    };
    let cancellation_trait: Arc<dyn EngineCancellation> = cancellation.clone();
    let (pid, secret) = client.pid_and_secret_key();
    let active_guard = control.active_queries.register(
        pid,
        secret.to_bytes().to_vec(),
        Arc::clone(&cancellation_trait),
    );
    let deadline = OperationDeadline::start(
        control.statement_timeout,
        cancellation_trait,
        Arc::clone(&control.blocking_workers),
    );
    let guards: Vec<Box<dyn Send>> =
        vec![Box::new(permit), Box::new(active_guard), Box::new(deadline)];
    let started_at = std::time::Instant::now();
    let metrics_recorded = Arc::new(AtomicBool::new(false));
    let worker_metrics_recorded = Arc::clone(&metrics_recorded);
    crate::metrics::copy_started();
    let ingest_table = target.table.clone();
    let worker = control
        .blocking_workers
        .spawn_regular(move || {
            let _guards = guards;
            let result =
                operation.execute(&ingest_table, Box::new(reader), IngestDisposition::Append);
            if result.is_err()
                && worker_metrics_recorded
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                crate::metrics::copy_failed(started_at.elapsed());
            }
            result
        })
        .await
        .map_err(blocking_worker_error)?;
    let decoder = CopyTextDecoder::new(schema, control.copy_limits).map_err(copy_decode_error)?;

    let mut request = state
        .request
        .lock()
        .map_err(|_| user_error("XX000", "DuckDB COPY state is poisoned"))?;
    if request.is_some() {
        return Err(user_error(
            "55000",
            "another COPY operation is already active",
        ));
    }
    *request = Some(CopyRequest {
        decoder,
        sender: Some(sender),
        worker: Some(worker),
        cancellation,
        rows: 0,
        bytes: 0,
        batches: 0,
        max_chunk_bytes: control.copy_limits.max_bytes,
        started_at,
        metrics_recorded,
        finished: false,
    });
    Ok(Response::CopyIn(CopyResponse::new(
        0,
        target.columns.len(),
        futures::stream::empty(),
    )))
}

fn join_error(error: tokio::task::JoinError) -> PgWireError {
    user_error("XX000", &format!("DuckDB worker failed: {error}"))
}

fn blocking_worker_error(error: BlockingWorkerError) -> PgWireError {
    match error {
        BlockingWorkerError::Closed => {
            user_error("57P01", "DuckDB blocking worker pool is unavailable")
        }
        BlockingWorkerError::Join(error) => join_error(error),
    }
}

fn copy_decode_error(error: CopyDecodeError) -> PgWireError {
    user_error(error.sqlstate, &error.message)
}

fn decode_copy_data(
    state: &CopySessionState,
    data: &[u8],
) -> PgWireResult<(
    tokio::sync::mpsc::Sender<CopyBatchMessage>,
    Vec<RecordBatch>,
)> {
    let mut request = state
        .request
        .lock()
        .map_err(|_| user_error("XX000", "DuckDB COPY state is poisoned"))?;
    let Some(active) = request.as_mut() else {
        return Err(user_error("55000", "no DuckDB COPY operation is active"));
    };
    if data.len() > active.max_chunk_bytes {
        return Err(user_error(
            "54000",
            "one COPY data chunk exceeds the configured batch byte limit",
        ));
    }
    let batches = match active.decoder.push(data) {
        Ok(batches) => batches,
        Err(error) => return Err(copy_decode_error(error)),
    };
    let sender = active
        .sender
        .as_ref()
        .cloned()
        .ok_or_else(|| user_error("55000", "DuckDB COPY input is already closed"))?;
    active.bytes = active.bytes.saturating_add(data.len());
    active.batches = active.batches.saturating_add(batches.len());
    active.rows = active
        .rows
        .saturating_add(batches.iter().map(RecordBatch::num_rows).sum::<usize>());
    Ok((sender, batches))
}

fn take_copy_request(state: &CopySessionState) -> Option<CopyRequest> {
    state.request.lock().ok()?.take()
}

async fn cleanup_aborted_copy(mut request: CopyRequest) {
    request.cancellation.abort_input();
    request.sender.take();
    if let Some(worker) = request.worker.take() {
        match worker.await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => log::debug!("DuckDB COPY abort cleanup completed: {error}"),
            Err(error) => log::warn!("DuckDB COPY abort worker failed: {error}"),
        }
    }
    request.finished = true;
}

fn user_error(code: &str, message: &str) -> PgWireError {
    PgWireError::UserError(Box::new(ErrorInfo::new(
        "ERROR".to_owned(),
        code.to_owned(),
        message.to_owned(),
    )))
}

struct DuckDbCopyHandler;

#[async_trait]
impl CopyHandler for DuckDbCopyHandler {
    async fn on_copy_data<C>(&self, client: &mut C, data: CopyData) -> PgWireResult<()>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let state = client
            .session_extensions()
            .get::<CopySessionState>()
            .ok_or_else(|| user_error("55000", "no DuckDB COPY operation is active"))?;
        let (sender, batches) = match decode_copy_data(&state, &data.data) {
            Ok(decoded) => decoded,
            Err(error) => {
                if let Some(request) = take_copy_request(&state) {
                    cleanup_aborted_copy(request).await;
                }
                return Err(error);
            }
        };
        for batch in batches {
            if sender.send(CopyBatchMessage::Batch(batch)).await.is_err() {
                if let Some(request) = take_copy_request(&state) {
                    cleanup_aborted_copy(request).await;
                }
                return Err(user_error("57014", "DuckDB COPY ingestion was aborted"));
            }
        }
        Ok(())
    }

    async fn on_copy_done<C>(&self, client: &mut C, _done: CopyDone) -> PgWireResult<()>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let state = client
            .session_extensions()
            .get::<CopySessionState>()
            .ok_or_else(|| user_error("55000", "no DuckDB COPY operation is active"))?;
        let mut request = state
            .request
            .lock()
            .map_err(|_| user_error("XX000", "DuckDB COPY state is poisoned"))?
            .take()
            .ok_or_else(|| user_error("55000", "no DuckDB COPY operation is active"))?;
        let batches = match request.decoder.finish() {
            Ok(batches) => batches,
            Err(error) => {
                cleanup_aborted_copy(request).await;
                return Err(copy_decode_error(error));
            }
        };
        request.batches = request.batches.saturating_add(batches.len());
        request.rows = request
            .rows
            .saturating_add(batches.iter().map(RecordBatch::num_rows).sum::<usize>());
        let rows = request.rows;
        let sender = request
            .sender
            .take()
            .ok_or_else(|| user_error("55000", "DuckDB COPY input is already closed"))?;
        let commit_started = std::time::Instant::now();
        for batch in batches {
            if sender.send(CopyBatchMessage::Batch(batch)).await.is_err() {
                cleanup_aborted_copy(request).await;
                return Err(user_error("57014", "DuckDB COPY ingestion was aborted"));
            }
        }
        if sender.send(CopyBatchMessage::Finish).await.is_err() {
            cleanup_aborted_copy(request).await;
            return Err(user_error("57014", "DuckDB COPY ingestion was aborted"));
        }
        drop(sender);
        request
            .worker
            .take()
            .ok_or_else(|| user_error("55000", "DuckDB COPY worker is unavailable"))?
            .await
            .map_err(join_error)?
            .map_err(engine_error)?;
        request.finished = true;
        if request
            .metrics_recorded
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            crate::metrics::copy_completed(
                request.rows,
                request.bytes,
                request.batches,
                request.started_at.elapsed(),
                commit_started.elapsed(),
            );
        }
        client
            .send(PgWireBackendMessage::CommandComplete(
                Tag::new("COPY").with_rows(rows).into(),
            ))
            .await?;
        client.set_state(PgWireConnectionState::AwaitingSync);
        Ok(())
    }

    async fn on_copy_fail<C>(&self, client: &mut C, fail: CopyFail) -> PgWireError
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        if let Some(state) = client.session_extensions().get::<CopySessionState>()
            && let Some(request) = take_copy_request(&state)
        {
            cleanup_aborted_copy(request).await;
        }
        user_error("57014", &format!("COPY aborted: {}", fail.message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_classifier_handles_comments_ctes_and_literal_semicolons() {
        let validated = validate_statement(
            "/* not UPDATE */ WITH ids AS (SELECT 1 AS id) SELECT ';' FROM ids;",
            ProtocolMode::Extended,
        )
        .expect("structural SELECT");
        assert_eq!(validated.kind, StatementKind::Read);

        let validated = validate_statement(
            "-- leading comment\nUPDATE quackgis.main.points SET name = $1 WHERE id = $2",
            ProtocolMode::Extended,
        )
        .expect("structural UPDATE");
        assert_eq!(validated.kind, StatementKind::Write("UPDATE"));
    }

    #[test]
    fn structural_classifier_rejects_multiple_and_unapproved_statements() {
        assert!(validate_statement("SELECT 1; SELECT 2", ProtocolMode::Simple).is_err());
        assert!(validate_statement("TRUNCATE quackgis.main.points", ProtocolMode::Simple).is_err());
        assert!(validate_statement("BEGIN", ProtocolMode::Extended).is_err());
    }

    #[test]
    fn post_commit_control_failure_is_fatal_to_the_connection() {
        let info: ErrorInfo = fatal_anyhow_error(anyhow::anyhow!(
            "DuckLake commit succeeded but catalog reconciliation failed"
        ))
        .into();
        assert!(info.is_fatal());
        assert_eq!(info.code, "XX000");
    }

    #[test]
    fn unsupported_spatial_functions_are_structurally_rejected() {
        for (function, sql) in [
            ("ST_NDims", "SELECT ST_NDims(ST_Point(1, 2))"),
            ("ST_CoordDim", "SELECT public.ST_CoordDim(ST_Point(1, 2))"),
            (
                "ST_GeometryN",
                "SELECT ST_AsText(ST_GeometryN(ST_Point(1, 2), 1))",
            ),
            ("ST_AsEWKT", "SELECT ST_AsEWKT(ST_Point(1, 2))"),
            ("ST_SRID", "SELECT ST_SRID(ST_Point(1, 2))"),
            ("ST_Zmflag", "SELECT ST_Zmflag(ST_Point(1, 2))"),
            ("ST_XMax", "SELECT ST_XMax(ST_Extent(geom)) FROM points"),
            ("ST_YMax", "SELECT ST_YMax(ST_Extent(geom)) FROM points"),
            ("ST_Extent", "SELECT ST_Extent(geom) FROM points"),
            ("Find_SRID", "SELECT Find_SRID('public', 'points', 'geom')"),
        ] {
            let error = match validate_statement(sql, ProtocolMode::Simple) {
                Ok(_) => panic!("unsupported spatial function {function}"),
                Err(error) => error,
            };
            assert!(error.to_string().contains(function), "{error}");
        }

        for sql in [
            "SELECT 'ST_NDims(g)'",
            "SELECT 1 /* ST_CoordDim(g) */",
            "SELECT ST_Dimension(ST_Point(1, 2))",
        ] {
            validate_statement(sql, ProtocolMode::Simple)
                .unwrap_or_else(|error| panic!("supported SQL {sql}: {error}"));
        }
    }

    #[test]
    fn spatial_type_catalog_relations_are_structurally_rewritten() {
        let lookup = validate_statement(
            "SELECT t.typname, t.typtype, t.typelem, r.rngsubtype, t.typbasetype, \
             n.nspname, t.typrelid FROM pg_catalog.pg_type t \
             LEFT OUTER JOIN pg_catalog.pg_range r ON r.rngtypid = t.oid \
             INNER JOIN pg_catalog.pg_namespace n ON t.typnamespace = n.oid \
             WHERE t.oid = $1",
            ProtocolMode::Extended,
        )
        .expect("structural pg_type lookup");
        assert_eq!(
            catalog_oid_parameter_indexes(&lookup.ast),
            std::collections::HashSet::from([0])
        );
        assert!(lookup.sql.contains("quackgis_pg_catalog.pg_type"));
        assert!(lookup.sql.contains("quackgis_pg_catalog.pg_range"));
        assert!(lookup.sql.contains("quackgis_pg_catalog.pg_namespace"));
        assert!(!lookup.sql.contains("CASE"));
        assert!(!lookup.sql.contains("90001"));

        let ordinary = validate_statement(
            "SELECT typname FROM pg_catalog.pg_type",
            ProtocolMode::Extended,
        )
        .expect("ordinary catalog query");
        assert!(ordinary.sql.contains("quackgis_pg_catalog.pg_type"));
        assert!(!ordinary.sql.contains("90001"));

        let implicit = validate_statement(
            "SELECT oid, typname, typtype, typelem, typlen FROM pg_type \
             WHERE oid IN (20, 29, 28, 25, 90001, 27, 26)",
            ProtocolMode::Extended,
        )
        .expect("implicit pg_catalog lookup");
        assert!(implicit.sql.contains("quackgis_pg_catalog.pg_type"));

        let collation = validate_statement(
            "SELECT c.oid, c.collname FROM pg_collation c WHERE c.oid IN (100, 950)",
            ProtocolMode::Extended,
        )
        .expect("implicit collation lookup");
        assert!(collation.sql.contains("quackgis_pg_catalog.pg_collation"));

        for relational_query in [
            "SELECT t.typname FROM pg_catalog.pg_type t \
             INNER JOIN pg_catalog.pg_namespace n ON t.typnamespace = n.oid \
             WHERE t.oid = $1",
            "SELECT t.typname, t.typtype, t.typelem, r.rngsubtype, t.typbasetype, \
             n.nspname, t.typrelid FROM pg_catalog.pg_type t \
             LEFT OUTER JOIN pg_catalog.pg_range r ON r.rngtypid = t.oid \
             INNER JOIN pg_catalog.pg_namespace n ON t.typnamespace = n.oid \
             WHERE t.oid = 90001",
            "SELECT t.typname, t.typtype, t.typelem, r.rngsubtype, t.typbasetype, \
             n.nspname, t.typrelid FROM pg_catalog.pg_type t \
             LEFT OUTER JOIN pg_catalog.pg_range r ON r.rngtypid = t.oid \
             INNER JOIN pg_catalog.pg_namespace n ON t.typnamespace = n.oid \
             WHERE t.oid = $1 AND t.typname = 'geometry'",
        ] {
            let validated = validate_statement(relational_query, ProtocolMode::Extended)
                .unwrap_or_else(|error| panic!("relational catalog SQL: {error}"));
            assert!(
                validated.sql.contains("quackgis_pg_catalog.pg_type"),
                "catalog relation was not rewritten: {relational_query}"
            );
            assert!(!validated.sql.contains("WHEN 90001"));
        }

        for unsupported in [
            "SELECT relname FROM pg_catalog.pg_class",
            "SELECT relname FROM pg_class",
            "SELECT * FROM pg_catalog.pg_tables",
            "SELECT * FROM \"pg_tables\"",
            "TABLE pg_catalog.pg_class",
            "TABLE pg_catalog.pg_type",
            "SELECT oid FROM memory.pg_catalog.pg_type",
            "SELECT EXISTS (SELECT 1 FROM pg_type)",
            "SELECT * FROM pg_type",
            "SELECT oid FROM pg_type UNION ALL SELECT oid FROM pg_type",
            "WITH pg_type AS (SELECT 1 AS oid) SELECT oid FROM pg_type",
            "SELECT t.typname FROM pg_type t WHERE EXISTS (SELECT 1 FROM quackgis.main.user_oids u WHERE u.oid = t.oid)",
            "SELECT d.oid FROM pg_type t CROSS JOIN LATERAL (SELECT t.*) d",
            "SELECT pg_catalog.pg_type.oid FROM pg_catalog.pg_type",
            "SELECT t.\"TYPLEN\" FROM pg_type t",
            "SELECT oid FROM quackgis_pg_catalog.pg_type",
            "SELECT quackgis_pg_catalog.pg_type.oid FROM pg_type",
            "SELECT t.typname FROM pg_type t JOIN pg_type u USING (oid)",
            "SELECT CASE WHEN true THEN t.oid ELSE t.oid END FROM pg_type t",
            "SELECT \"T\".oid FROM pg_type t",
            "SELECT * FROM quackgis._quackgis.catalog_state",
            "INSERT INTO quackgis._quackgis.catalog_state VALUES (true, 1, 1, NULL)",
            "SELECT * FROM _quackgis.relation_oid",
            "SELECT * FROM query_table('quackgis._quackgis.catalog_state')",
            "SELECT * FROM query('SELECT * FROM quackgis._quackgis.relation_oid')",
            "SELECT * FROM ducklake_column_info('quackgis')",
            "SELECT * FROM LATERAL query('SELECT * FROM quackgis._quackgis.relation_oid')",
            "SELECT * FROM LATERAL query_table('quackgis._quackgis.catalog_state')",
            "SELECT * FROM LATERAL ducklake_column_info('quackgis')",
        ] {
            assert!(
                validate_statement(unsupported, ProtocolMode::Extended).is_err(),
                "unimplemented catalog relation must fail closed: {unsupported}"
            );
        }

        let user_oid = validate_statement(
            "SELECT p.oid FROM quackgis.main.points p WHERE p.oid = $1",
            ProtocolMode::Extended,
        )
        .expect("user oid query");
        assert!(catalog_oid_parameter_indexes(&user_oid.ast).is_empty());

        assert!(
            validate_statement(
                "SELECT typname FROM \"PG_CATALOG\".\"PG_TYPE\"",
                ProtocolMode::Extended,
            )
            .is_err()
        );
        assert!(
            validate_statement(
                "SELECT t.\"OID\" FROM pg_catalog.pg_type t",
                ProtocolMode::Extended,
            )
            .is_err()
        );
        assert!(
            validate_statement("SELECT typname FROM \"PG_TYPE\"", ProtocolMode::Extended,).is_err()
        );
        assert!(
            validate_statement(
                "SELECT relname FROM pg_catalog.\"PG_CLASS\"",
                ProtocolMode::Extended,
            )
            .is_err()
        );
        assert!(
            validate_statement(
                "SELECT t.oid FROM pg_type t ORDER BY t.\"OID\"",
                ProtocolMode::Extended,
            )
            .is_err()
        );

        let quoted_alias = validate_statement(
            "SELECT \"t\".oid FROM pg_catalog.pg_type AS t",
            ProtocolMode::Extended,
        )
        .expect("quoted lowercase alias matches unquoted declaration");
        let quoted_alias_schema = Schema::new(vec![Field::new("oid", DataType::UInt32, false)]);
        let quoted_alias_schema =
            annotate_catalog_result_schema(&quoted_alias.ast, &quoted_alias_schema);
        let quoted_alias_field = Arc::new(quoted_alias_schema.field(0).clone());
        assert_eq!(field_into_pg_type(&quoted_alias_field).unwrap(), Type::OID);

        let owner = validate_statement(
            "SELECT n.nspname, r.rolname FROM pg_catalog.pg_namespace n \
             JOIN pg_catalog.pg_roles r ON r.oid = n.nspowner",
            ProtocolMode::Extended,
        )
        .expect("bootstrap owner query");
        assert!(owner.sql.contains("quackgis_pg_catalog.pg_roles"));
    }

    #[test]
    fn database_and_schema_discovery_are_structural_and_typed() {
        let validated = validate_statement(
            "SELECT current_database() AS database_name, \
             pg_catalog.current_schema() AS schema_name, \
             current_schemas(true) AS schemas",
            ProtocolMode::Extended,
        )
        .expect("maintained database discovery");
        assert!(validated.sql.contains("quackgis_current_database()"));
        assert!(validated.sql.contains("quackgis_current_schema()"));
        assert!(validated.sql.contains("quackgis_current_schemas(true)"));
        let schema = annotate_catalog_result_schema(
            &validated.ast,
            &Schema::new(vec![
                Field::new("database_name", DataType::Utf8, false),
                Field::new("schema_name", DataType::Utf8, false),
                Field::new(
                    "schemas",
                    DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
                    false,
                ),
            ]),
        );
        assert_eq!(
            field_into_pg_type(&Arc::new(schema.field(0).clone())).unwrap(),
            Type::NAME
        );
        assert_eq!(
            field_into_pg_type(&Arc::new(schema.field(1).clone())).unwrap(),
            Type::NAME
        );
        assert_eq!(
            field_into_pg_type(&Arc::new(schema.field(2).clone())).unwrap(),
            Type::NAME_ARRAY
        );

        let database = validate_statement(
            "SELECT d.oid, d.datname, d.datdba FROM pg_database d",
            ProtocolMode::Extended,
        )
        .expect("maintained pg_database");
        assert!(database.sql.contains("quackgis_pg_catalog.pg_database"));
        assert!(
            validate_statement("SELECT quackgis_current_database()", ProtocolMode::Extended,)
                .is_err()
        );
    }

    #[test]
    fn maintenance_call_is_literal_bounded_and_simple_protocol_only() {
        let validated = validate_statement(
            "CALL quackgis_merge_adjacent_files('public', 'points', 8, 16777216, NULL)",
            ProtocolMode::Simple,
        )
        .expect("maintenance call");
        assert_eq!(validated.kind, StatementKind::Maintenance);
        let command = parse_maintenance_call(&validated.ast)
            .expect("maintenance parse")
            .expect("maintenance command");
        assert_eq!(command.schema, "main");
        assert_eq!(command.table, "points");
        assert_eq!(
            command.request,
            EngineMaintenanceRequest::MergeAdjacentFiles {
                schema: "main".to_owned(),
                table: "points".to_owned(),
                max_compacted_files: Some(8),
                max_file_size: Some(16_777_216),
                min_file_size: None,
            }
        );
        for sql in [
            "CALL quackgis_merge_adjacent_files('main', 'points', 0, NULL, NULL)",
            "CALL quackgis_merge_adjacent_files('other', 'points', 8, NULL, NULL)",
            "CALL quackgis_merge_adjacent_files('main', current_user, 8, NULL, NULL)",
            "CALL arbitrary_procedure('main', 'points', 8, NULL, NULL)",
        ] {
            assert!(
                validate_statement(sql, ProtocolMode::Simple).is_err(),
                "{sql}"
            );
        }
        assert!(
            validate_statement(
                "CALL quackgis_merge_adjacent_files('main', 'points', 8, NULL, NULL)",
                ProtocolMode::Extended,
            )
            .is_err()
        );
    }

    #[test]
    fn result_batch_byte_limit_fails_closed() {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "value",
                DataType::Utf8,
                false,
            )])),
            vec![Arc::new(StringArray::from(vec!["bounded-result"]))],
        )
        .expect("batch");
        let bytes = batch.get_array_memory_size();
        assert_eq!(
            validate_result_batch(&batch, bytes).expect("exact ceiling"),
            bytes
        );
        assert!(validate_result_batch(&batch, bytes - 1).is_err());
    }

    #[test]
    fn client_session_and_public_schema_rules_are_structural() {
        let set = validate_statement("SET standard_conforming_strings = ON", ProtocolMode::Simple)
            .expect("supported SET");
        assert_eq!(set.kind, StatementKind::SessionSet);
        for sql in [
            "SET client_encoding TO 'UTF8'",
            "SET client_encoding = 'UNICODE'",
        ] {
            assert_eq!(
                validate_statement(sql, ProtocolMode::Simple)
                    .expect("supported client encoding")
                    .kind,
                StatementKind::SessionSet
            );
        }
        assert!(validate_statement("SET TimeZone = 'UTC'", ProtocolMode::Simple).is_err());
        let (_, batch, ast) = validate_simple_sql(
            "SET extra_float_digits=3;SET application_name=' external';\
             SET datestyle='ISO';SET client_min_messages TO error;",
        )
        .expect("QGIS session bootstrap batch");
        assert!(matches!(batch, SimpleStatementKind::SessionSetBatch(4)));
        assert!(ast.is_none());
        for invalid in [
            "SET extra_float_digits=2;SET datestyle='ISO'",
            "SET extra_float_digits=3;SELECT 1",
            "SET application_name='abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklm';SET datestyle='ISO'",
        ] {
            assert!(
                validate_simple_sql(invalid).is_err(),
                "invalid SET batch: {invalid}"
            );
        }

        for (sql, expected) in [
            (
                "SHOW search_path",
                "SELECT 'public'::VARCHAR AS search_path",
            ),
            (
                "SHOW client_encoding",
                "SELECT 'UTF8'::VARCHAR AS client_encoding",
            ),
            (
                "SHOW standard_conforming_strings",
                "SELECT 'on'::VARCHAR AS standard_conforming_strings",
            ),
        ] {
            let show = validate_statement(sql, ProtocolMode::Simple).expect("supported SHOW");
            assert_eq!(show.kind, StatementKind::Read);
            assert_eq!(show.sql, expected);
        }

        let query = validate_statement(
            "SELECT count(*) FROM \"public\".\"points\"",
            ProtocolMode::Simple,
        )
        .expect("public relation");
        assert!(query.sql.contains("quackgis.main.\"points\""));

        for sql in [
            "COPY points (id, name) FROM STDIN",
            "COPY \"public\".\"points\" (\"id\", \"name\") FROM STDIN",
            "COPY quackgis.main.points (id, name) FROM STDIN",
        ] {
            let target = parse_copy_target(sql)
                .expect("COPY parse")
                .expect("COPY target");
            assert_eq!(target.table.catalog, "quackgis");
            assert_eq!(target.table.schema, "main");
            assert_eq!(target.table.table, "points");
            assert_eq!(target.columns, ["id", "name"]);
        }
    }
}
