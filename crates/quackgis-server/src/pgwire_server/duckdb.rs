// SPDX-License-Identifier: Apache-2.0
//! Bounded DuckDB pgwire backend.
//!
//! This local profile proves the direct ADBC/Arrow protocol seam through the real
//! CLI backend. Unsupported policy, storage, statement, COPY, and parameter
//! shapes fail closed until their D2-D4 contracts pass.

use std::fmt::Debug;
use std::sync::{Arc, LazyLock, Mutex};

use arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Decimal128Array, Float32Array, Float64Array,
    Int16Array, Int32Array, Int64Array, RecordBatch, StringArray, TimestampMicrosecondArray,
};
use arrow_pg::datatypes::{arrow_schema_to_pg_fields, field_into_pg_type};
use arrow_pg::encode_recordbatch;
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use async_trait::async_trait;
use chrono::{NaiveDate, NaiveDateTime};
use futures::{Sink, SinkExt};
use pgwire::api::cancel::{CancelHandler, DefaultCancelHandler};
use pgwire::api::copy::CopyHandler;
use pgwire::api::portal::{Format, Portal};
use pgwire::api::query::{ExtendedQueryHandler, SimpleQueryHandler};
use pgwire::api::results::{CopyResponse, QueryResponse, Response, Tag};
use pgwire::api::stmt::QueryParser;
use pgwire::api::store::PortalStore;
use pgwire::api::{
    ClientInfo, ClientPortalStore, ConnectionManager, ErrorHandler, PgWireConnectionState,
    PgWireServerHandlers, Type,
};
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use pgwire::messages::PgWireBackendMessage;
use pgwire::messages::copy::{CopyData, CopyDone, CopyFail};
use regex::Regex;
use sqlparser::ast::Statement;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use super::{
    LoggingErrorHandler, QuackGisStartupHandler, ServerOptions, SimpleStartupHandler,
    serve_with_handlers, serve_with_handlers_on_listener,
};
use crate::auth::{AuthConfig, AuthMode};
use crate::duckdb_adbc_storage::DuckDbAdbcStorage;
use crate::engine_api::{
    EngineError, EngineErrorKind, EngineQueryResult, EngineStorageKernel, EngineTableRef,
    IngestDisposition,
};

pub async fn serve_duckdb(
    storage: Arc<DuckDbAdbcStorage>,
    options: &ServerOptions,
    auth: AuthConfig,
) -> Result<(), std::io::Error> {
    let factory = Arc::new(DuckDbHandlerFactory::new(storage, auth));
    serve_with_handlers(factory, options).await
}

pub async fn serve_duckdb_on_listener(
    storage: Arc<DuckDbAdbcStorage>,
    listener: tokio::net::TcpListener,
    options: &ServerOptions,
    auth: AuthConfig,
) -> Result<(), std::io::Error> {
    let factory = Arc::new(DuckDbHandlerFactory::new(storage, auth));
    serve_with_handlers_on_listener(factory, listener, options).await
}

struct DuckDbHandlerFactory {
    service: Arc<DuckDbService>,
    startup: Arc<QuackGisStartupHandler>,
    cancel: Arc<DefaultCancelHandler>,
    copy: Arc<DuckDbCopyHandler>,
}

impl DuckDbHandlerFactory {
    fn new(storage: Arc<DuckDbAdbcStorage>, auth: AuthConfig) -> Self {
        let manager = Arc::new(ConnectionManager::new());
        let startup = match auth.mode() {
            AuthMode::Trust => QuackGisStartupHandler::Trust(SimpleStartupHandler {
                connection_manager: Arc::clone(&manager),
            }),
            AuthMode::Password => QuackGisStartupHandler::Password(Box::new(
                super::PerConnectionScramStartupHandler::new(auth.clone(), Arc::clone(&manager)),
            )),
        };
        Self {
            service: Arc::new(DuckDbService::new(storage, auth)),
            startup: Arc::new(startup),
            cancel: Arc::new(DefaultCancelHandler::new(manager)),
            copy: Arc::new(DuckDbCopyHandler),
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
        authorize_statement(client, &self.auth, &validated.ast)?;
        let storage = client_session(client, Arc::clone(&self.storage)).await?;
        let describe_sql = validated.sql.clone();
        let description = tokio::task::spawn_blocking(move || storage.describe(&describe_sql))
            .await
            .map_err(join_error)?
            .map_err(engine_error)?;
        let parameter_schema = Arc::new(Schema::new(
            description
                .parameter_schema
                .fields()
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    if field.data_type() == &DataType::Null
                        && let Some(Some(pg_type)) = types.get(index)
                    {
                        return Field::new(
                            field.name(),
                            pg_type_to_arrow(pg_type).unwrap_or(DataType::Null),
                            true,
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
        Ok(DuckDbStatement {
            sql: validated.sql,
            copy_target: None,
            kind: validated.kind,
            parameter_schema,
            result_schema: description.result_schema,
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
}

impl DuckDbService {
    fn new(storage: Arc<DuckDbAdbcStorage>, auth: AuthConfig) -> Self {
        Self {
            parser: Arc::new(DuckDbParser {
                storage: Arc::clone(&storage),
                auth: auth.clone(),
            }),
            storage,
            auth,
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
        match (&kind, ast.as_ref()) {
            (SimpleStatementKind::Copy(target), _) => authorize_copy(client, &self.auth, target)?,
            (_, Some(statement)) => authorize_statement(client, &self.auth, statement)?,
            (_, None) => {
                return Err(user_error(
                    "XX000",
                    "validated DuckDB statement is missing its structural policy input",
                ));
            }
        }
        let storage = client_session(client, Arc::clone(&self.storage)).await?;
        match kind {
            SimpleStatementKind::Read => {
                let result = tokio::task::spawn_blocking(move || storage.query_result(&sql))
                    .await
                    .map_err(join_error)?
                    .map_err(engine_error)?;
                Ok(vec![Response::Query(query_response(
                    result,
                    &Format::UnifiedText,
                )?)])
            }
            SimpleStatementKind::Write(command) => {
                let affected =
                    tokio::task::spawn_blocking(move || storage.execute_update_contract(&sql))
                        .await
                        .map_err(join_error)?
                        .map_err(engine_error)?;
                let mut tag = Tag::new(command);
                if let Some(rows) = affected.and_then(|rows| usize::try_from(rows).ok()) {
                    tag = tag.with_rows(rows);
                }
                Ok(vec![Response::Execution(tag)])
            }
            SimpleStatementKind::Begin => {
                tokio::task::spawn_blocking(move || storage.begin_transaction())
                    .await
                    .map_err(join_error)?
                    .map_err(anyhow_error)?;
                Ok(vec![Response::TransactionStart(Tag::new("BEGIN"))])
            }
            SimpleStatementKind::Commit => {
                tokio::task::spawn_blocking(move || storage.commit_transaction())
                    .await
                    .map_err(join_error)?
                    .map_err(anyhow_error)?;
                Ok(vec![Response::TransactionEnd(Tag::new("COMMIT"))])
            }
            SimpleStatementKind::Rollback => {
                tokio::task::spawn_blocking(move || storage.rollback_transaction())
                    .await
                    .map_err(join_error)?
                    .map_err(anyhow_error)?;
                Ok(vec![Response::TransactionEnd(Tag::new("ROLLBACK"))])
            }
            SimpleStatementKind::Copy(target) => begin_copy(client, storage, target)
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
            let storage = client_session(client, Arc::clone(&self.storage)).await?;
            return begin_copy(client, storage, target.clone()).await;
        }
        let parameters = parameter_batch(portal, statement)?;
        let storage = client_session(client, Arc::clone(&self.storage)).await?;
        let sql = statement.sql.clone();
        match statement.kind {
            StatementKind::Read => {
                let result = tokio::task::spawn_blocking(move || {
                    if let Some(parameters) = parameters {
                        storage.query_bound(&sql, parameters)
                    } else {
                        storage.query_result(&sql)
                    }
                })
                .await
                .map_err(join_error)?
                .map_err(engine_error)?;
                Ok(Response::Query(query_response(
                    result,
                    &portal.result_column_format,
                )?))
            }
            StatementKind::Write(command) => {
                let affected = tokio::task::spawn_blocking(move || {
                    if let Some(parameters) = parameters {
                        storage.execute_update_bound(&sql, parameters)
                    } else {
                        storage.execute_update_contract(&sql)
                    }
                })
                .await
                .map_err(join_error)?
                .map_err(engine_error)?;
                let mut tag = Tag::new(command);
                if let Some(rows) = affected.and_then(|rows| usize::try_from(rows).ok()) {
                    tag = tag.with_rows(rows);
                }
                Ok(Response::Execution(tag))
            }
            StatementKind::Copy
            | StatementKind::Begin
            | StatementKind::Commit
            | StatementKind::Rollback => Err(user_error(
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
    Copy,
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
    let validated = validate_statement(sql, ProtocolMode::Simple)?;
    let kind = match validated.kind {
        StatementKind::Read => SimpleStatementKind::Read,
        StatementKind::Write(command) => SimpleStatementKind::Write(command),
        StatementKind::Begin => SimpleStatementKind::Begin,
        StatementKind::Commit => SimpleStatementKind::Commit,
        StatementKind::Rollback => SimpleStatementKind::Rollback,
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
        _ => {
            return Err(user_error(
                "0A000",
                "unsupported DuckDB pgwire statement shape",
            ));
        }
    };
    Ok(ValidatedStatement {
        sql,
        kind,
        ast: statement,
    })
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

fn normalize_sql(sql: &str) -> PgWireResult<String> {
    let sql = sql.trim();
    if sql.is_empty() {
        return Err(user_error("42601", "SQL statement must not be empty"));
    }
    Ok(sql.strip_suffix(';').unwrap_or(sql).trim().to_owned())
}

fn parse_copy_target(sql: &str) -> PgWireResult<Option<CopyTarget>> {
    let Some(captures) = copy_regex().captures(sql.trim().trim_end_matches(';').trim()) else {
        return Ok(None);
    };
    let target = captures
        .name("target")
        .expect("COPY target capture")
        .as_str()
        .split('.')
        .collect::<Vec<_>>();
    if target.len() != 3 {
        return Err(user_error(
            "0A000",
            "DuckDB COPY checkpoint requires catalog.schema.table",
        ));
    }
    let columns = captures
        .name("columns")
        .expect("COPY columns capture")
        .as_str()
        .split(',')
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if columns.is_empty() {
        return Err(user_error(
            "42601",
            "DuckDB COPY requires an explicit column list",
        ));
    }
    Ok(Some(CopyTarget {
        table: EngineTableRef {
            catalog: target[0].to_owned(),
            schema: target[1].to_owned(),
            table: target[2].to_owned(),
        },
        columns,
    }))
}

fn copy_regex() -> &'static Regex {
    static COPY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^COPY\s+(?P<target>[a-z_][a-z0-9_.]*)\s*\((?P<columns>[^)]+)\)\s+FROM\s+STDIN$",
        )
        .expect("bounded COPY regex")
    });
    &COPY
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

fn query_response(result: EngineQueryResult, format: &Format) -> PgWireResult<QueryResponse> {
    let fields = Arc::new(arrow_schema_to_pg_fields(
        result.schema.as_ref(),
        format,
        None,
    )?);
    let row_fields = Arc::clone(&fields);
    let rows = result
        .batches
        .into_iter()
        .flat_map(move |batch| encode_recordbatch(Arc::clone(&row_fields), batch));
    Ok(QueryResponse::new(fields, futures::stream::iter(rows)))
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

fn anyhow_error(error: anyhow::Error) -> PgWireError {
    user_error("XX000", &error.to_string())
}

async fn client_session<C>(
    client: &C,
    database: Arc<DuckDbAdbcStorage>,
) -> PgWireResult<Arc<DuckDbAdbcStorage>>
where
    C: ClientInfo + Unpin + Send + Sync,
{
    if let Some(session) = client.session_extensions().get::<DuckDbAdbcStorage>() {
        return Ok(session);
    }
    let session = tokio::task::spawn_blocking(move || database.open_session())
        .await
        .map_err(join_error)?
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
    table: EngineTableRef,
    schema: SchemaRef,
    data: Vec<u8>,
}

async fn begin_copy<C>(
    client: &C,
    storage: Arc<DuckDbAdbcStorage>,
    target: CopyTarget,
) -> PgWireResult<Response>
where
    C: ClientInfo + Unpin + Send + Sync,
{
    let table = target.table.clone();
    let full_schema = tokio::task::spawn_blocking(move || storage.table_schema(&table))
        .await
        .map_err(join_error)?
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
        table: target.table,
        schema,
        data: Vec::new(),
    });
    Ok(Response::CopyIn(CopyResponse::new(
        0,
        target.columns.len(),
        futures::stream::empty(),
    )))
}

fn copy_record_batch(schema: SchemaRef, data: &[u8]) -> PgWireResult<(RecordBatch, usize)> {
    let text = std::str::from_utf8(data)
        .map_err(|_| user_error("22021", "COPY text must be valid UTF-8"))?;
    let rows = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Err(user_error("22000", "COPY requires at least one data row"));
    }
    if rows.iter().any(|row| row.len() != schema.fields().len()) {
        return Err(user_error(
            "22P04",
            "COPY row has the wrong number of columns",
        ));
    }
    let arrays = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(column, field)| copy_array(field.data_type(), &rows, column))
        .collect::<PgWireResult<Vec<_>>>()?;
    let row_count = rows.len();
    RecordBatch::try_new(schema, arrays)
        .map(|batch| (batch, row_count))
        .map_err(|error| user_error("22000", &error.to_string()))
}

fn copy_array(data_type: &DataType, rows: &[Vec<&str>], column: usize) -> PgWireResult<ArrayRef> {
    fn value<'a>(row: &'a [&'a str], column: usize) -> Option<&'a str> {
        (row[column] != r"\N").then_some(row[column])
    }
    match data_type {
        DataType::Boolean => rows
            .iter()
            .map(|row| value(row, column).map(parse_copy_bool).transpose())
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(BooleanArray::from(values)) as ArrayRef),
        DataType::Int16 => rows
            .iter()
            .map(|row| {
                value(row, column)
                    .map(str::parse::<i16>)
                    .transpose()
                    .map_err(|_| user_error("22P02", "invalid COPY Int16 value"))
            })
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(Int16Array::from(values)) as ArrayRef),
        DataType::Int32 => rows
            .iter()
            .map(|row| {
                value(row, column)
                    .map(str::parse::<i32>)
                    .transpose()
                    .map_err(|_| user_error("22P02", "invalid COPY Int32 value"))
            })
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(Int32Array::from(values)) as ArrayRef),
        DataType::Int64 => rows
            .iter()
            .map(|row| {
                value(row, column)
                    .map(str::parse::<i64>)
                    .transpose()
                    .map_err(|_| user_error("22P02", "invalid COPY Int64 value"))
            })
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(Int64Array::from(values)) as ArrayRef),
        DataType::Float32 => rows
            .iter()
            .map(|row| {
                value(row, column)
                    .map(str::parse::<f32>)
                    .transpose()
                    .map_err(|_| user_error("22P02", "invalid COPY Float32 value"))
            })
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(Float32Array::from(values)) as ArrayRef),
        DataType::Float64 => rows
            .iter()
            .map(|row| {
                value(row, column)
                    .map(str::parse::<f64>)
                    .transpose()
                    .map_err(|_| user_error("22P02", "invalid COPY Float64 value"))
            })
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(Float64Array::from(values)) as ArrayRef),
        DataType::Decimal128(precision, scale) => {
            let values = rows
                .iter()
                .map(|row| {
                    value(row, column)
                        .map(|value| parse_copy_decimal(value, *precision, *scale))
                        .transpose()
                })
                .collect::<PgWireResult<Vec<_>>>()?;
            Decimal128Array::from(values)
                .with_precision_and_scale(*precision, *scale)
                .map(|array| Arc::new(array) as ArrayRef)
                .map_err(|error| user_error("22003", &error.to_string()))
        }
        DataType::Date32 => rows
            .iter()
            .map(|row| value(row, column).map(parse_copy_date).transpose())
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(Date32Array::from(values)) as ArrayRef),
        DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, None) => rows
            .iter()
            .map(|row| value(row, column).map(parse_copy_timestamp).transpose())
            .collect::<PgWireResult<Vec<_>>>()
            .map(|values| Arc::new(TimestampMicrosecondArray::from(values)) as ArrayRef),
        DataType::Utf8 => Ok(Arc::new(StringArray::from(
            rows.iter()
                .map(|row| value(row, column))
                .collect::<Vec<_>>(),
        ))),
        DataType::Binary => {
            let values = rows
                .iter()
                .map(|row| value(row, column).map(parse_copy_hex).transpose())
                .collect::<PgWireResult<Vec<_>>>()?;
            Ok(Arc::new(BinaryArray::from_opt_vec(
                values.iter().map(|value| value.as_deref()).collect(),
            )))
        }
        unsupported => Err(user_error(
            "0A000",
            &format!("unsupported DuckDB COPY Arrow type {unsupported}"),
        )),
    }
}

fn parse_copy_bool(value: &str) -> PgWireResult<bool> {
    match value.to_ascii_lowercase().as_str() {
        "t" | "true" | "1" | "y" | "yes" | "on" => Ok(true),
        "f" | "false" | "0" | "n" | "no" | "off" => Ok(false),
        _ => Err(user_error("22P02", "invalid COPY Boolean value")),
    }
}

fn parse_copy_decimal(value: &str, precision: u8, scale: i8) -> PgWireResult<i128> {
    let decimal = value
        .parse::<rust_decimal::Decimal>()
        .map_err(|_| user_error("22P02", "invalid COPY Decimal128 value"))?;
    let target_scale = u32::try_from(scale)
        .map_err(|_| user_error("0A000", "negative Decimal128 COPY scale is unsupported"))?;
    let source_scale = decimal.scale();
    let mut mantissa = decimal.mantissa();
    if source_scale < target_scale {
        let factor = 10_i128
            .checked_pow(target_scale - source_scale)
            .ok_or_else(|| user_error("22003", "COPY Decimal128 scale overflows"))?;
        mantissa = mantissa
            .checked_mul(factor)
            .ok_or_else(|| user_error("22003", "COPY Decimal128 value overflows"))?;
    } else if source_scale > target_scale {
        let divisor = 10_i128
            .checked_pow(source_scale - target_scale)
            .ok_or_else(|| user_error("22003", "COPY Decimal128 scale overflows"))?;
        if mantissa % divisor != 0 {
            return Err(user_error(
                "22003",
                "COPY Decimal128 value exceeds the target scale",
            ));
        }
        mantissa /= divisor;
    }
    let digits = mantissa.unsigned_abs().to_string().len();
    if digits > usize::from(precision) {
        return Err(user_error(
            "22003",
            "COPY Decimal128 value exceeds the target precision",
        ));
    }
    Ok(mantissa)
}

fn parse_copy_date(value: &str) -> PgWireResult<i32> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| user_error("22007", "invalid COPY Date32 value"))?;
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid Unix epoch");
    i32::try_from(date.signed_duration_since(epoch).num_days())
        .map_err(|_| user_error("22008", "COPY Date32 value is out of range"))
}

fn parse_copy_timestamp(value: &str) -> PgWireResult<i64> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
        .map(|timestamp| timestamp.and_utc().timestamp_micros())
        .map_err(|_| user_error("22007", "invalid COPY Timestamp value"))
}

fn parse_copy_hex(value: &str) -> PgWireResult<Vec<u8>> {
    let hex = value
        .strip_prefix(r"\x")
        .ok_or_else(|| user_error("22P02", "COPY binary value must use \\x hex format"))?;
    if !hex.len().is_multiple_of(2) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(user_error(
            "22P02",
            "COPY binary value contains invalid hex",
        ));
    }
    (0..hex.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&hex[index..index + 2], 16)
                .map_err(|_| user_error("22P02", "COPY binary value contains invalid hex"))
        })
        .collect()
}

fn join_error(error: tokio::task::JoinError) -> PgWireError {
    user_error("XX000", &format!("DuckDB worker failed: {error}"))
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
        const MAX_COPY_BYTES: usize = 16 * 1024 * 1024;
        let state = client
            .session_extensions()
            .get::<CopySessionState>()
            .ok_or_else(|| user_error("55000", "no DuckDB COPY operation is active"))?;
        let mut request = state
            .request
            .lock()
            .map_err(|_| user_error("XX000", "DuckDB COPY state is poisoned"))?;
        let request = request
            .as_mut()
            .ok_or_else(|| user_error("55000", "no DuckDB COPY operation is active"))?;
        if request.data.len().saturating_add(data.data.len()) > MAX_COPY_BYTES {
            return Err(user_error("54000", "DuckDB COPY checkpoint exceeds 16 MiB"));
        }
        request.data.extend_from_slice(&data.data);
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
        let request = state
            .request
            .lock()
            .map_err(|_| user_error("XX000", "DuckDB COPY state is poisoned"))?
            .take()
            .ok_or_else(|| user_error("55000", "no DuckDB COPY operation is active"))?;
        let (batch, rows) = copy_record_batch(request.schema, &request.data)?;
        let session = client
            .session_extensions()
            .get::<DuckDbAdbcStorage>()
            .ok_or_else(|| user_error("XX000", "DuckDB client session is unavailable"))?;
        tokio::task::spawn_blocking(move || {
            session.ingest_contract(&request.table, vec![batch], IngestDisposition::Append)
        })
        .await
        .map_err(join_error)?
        .map_err(engine_error)?;
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
            && let Ok(mut request) = state.request.lock()
        {
            *request = None;
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
}
