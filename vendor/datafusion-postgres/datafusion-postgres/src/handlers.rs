use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::datatypes::DataType;
use datafusion::common::ParamValues;
use datafusion::error::DataFusionError;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::*;
use datafusion::sql::parser::Statement;
use datafusion::sql::sqlparser;
use log::info;
use pgwire::api::auth::StartupHandler;
use pgwire::api::auth::noop::NoopStartupHandler;
use pgwire::api::cancel::{CancelHandler, DefaultCancelHandler};
use pgwire::api::portal::{Format, Portal};
use pgwire::api::query::{ExtendedQueryHandler, SimpleQueryHandler};
use pgwire::api::results::{FieldInfo, Response, Tag};
use pgwire::api::stmt::QueryParser;
use pgwire::api::store::PortalStore;
use pgwire::api::{
    ClientInfo, ClientPortalStore, ConnectionManager, ErrorHandler, PgWireServerHandlers, Type,
};
use pgwire::error::{PgWireError, PgWireResult};
use pgwire::messages::PgWireBackendMessage;
use pgwire::types::format::FormatOptions;

use crate::hooks::cursor::CursorStatementHook;
use crate::hooks::set_show::SetShowHook;
use crate::hooks::transactions::TransactionStatementHook;
use crate::hooks::{ExtendedQueryPlan, QueryHook};
use crate::{client, planner};
use arrow_pg::datatypes::df;
use arrow_pg::datatypes::{arrow_schema_to_pg_fields, into_pg_type};
use datafusion_pg_catalog::sql::PostgresCompatibilityParser;

/// Simple startup handler that does no authentication
pub struct SimpleStartupHandler {
    connection_manager: Arc<ConnectionManager>,
}

#[async_trait::async_trait]
impl NoopStartupHandler for SimpleStartupHandler {
    fn connection_manager(&self) -> Option<Arc<ConnectionManager>> {
        Some(self.connection_manager.clone())
    }
}

pub struct HandlerFactory {
    pub session_service: Arc<DfSessionService>,
    cancel_handler: Arc<DefaultCancelHandler>,
    startup_handler: Arc<SimpleStartupHandler>,
}

impl HandlerFactory {
    pub fn new(session_context: Arc<SessionContext>) -> Self {
        let session_service = Arc::new(DfSessionService::new(session_context));
        let connection_manager = Arc::new(ConnectionManager::new());
        HandlerFactory {
            session_service,
            cancel_handler: Arc::new(DefaultCancelHandler::new(connection_manager.clone())),
            startup_handler: Arc::new(SimpleStartupHandler {
                connection_manager: connection_manager.clone(),
            }),
        }
    }

    pub fn new_with_hooks(
        session_context: Arc<SessionContext>,
        query_hooks: Vec<Arc<dyn QueryHook>>,
    ) -> Self {
        let session_service = Arc::new(DfSessionService::new_with_hooks(
            session_context,
            query_hooks,
        ));
        let connection_manager = Arc::new(ConnectionManager::new());
        HandlerFactory {
            session_service,
            cancel_handler: Arc::new(DefaultCancelHandler::new(connection_manager.clone())),
            startup_handler: Arc::new(SimpleStartupHandler {
                connection_manager: connection_manager.clone(),
            }),
        }
    }
}

impl PgWireServerHandlers for HandlerFactory {
    fn simple_query_handler(&self) -> Arc<impl SimpleQueryHandler> {
        self.session_service.clone()
    }

    fn extended_query_handler(&self) -> Arc<impl ExtendedQueryHandler> {
        self.session_service.clone()
    }

    fn startup_handler(&self) -> Arc<impl StartupHandler> {
        self.startup_handler.clone()
    }

    fn error_handler(&self) -> Arc<impl ErrorHandler> {
        Arc::new(LoggingErrorHandler)
    }

    fn cancel_handler(&self) -> Arc<impl CancelHandler> {
        self.cancel_handler.clone()
    }
}

struct LoggingErrorHandler;

impl ErrorHandler for LoggingErrorHandler {
    fn on_error<C>(&self, _client: &C, error: &mut PgWireError)
    where
        C: ClientInfo,
    {
        info!("Sending error: {error}")
    }
}

/// The pgwire handler backed by a datafusion `SessionContext`
pub struct DfSessionService {
    session_context: Arc<SessionContext>,
    parser: Arc<Parser>,
    query_hooks: Vec<Arc<dyn QueryHook>>,
}

impl DfSessionService {
    pub fn new(session_context: Arc<SessionContext>) -> DfSessionService {
        let hooks: Vec<Arc<dyn QueryHook>> = vec![
            Arc::new(CursorStatementHook),
            Arc::new(SetShowHook),
            Arc::new(TransactionStatementHook),
        ];
        Self::new_with_hooks(session_context, hooks)
    }

    pub fn new_with_hooks(
        session_context: Arc<SessionContext>,
        query_hooks: Vec<Arc<dyn QueryHook>>,
    ) -> DfSessionService {
        let parser = Arc::new(Parser {
            session_context: session_context.clone(),
            sql_parser: PostgresCompatibilityParser::new(),
            query_hooks: query_hooks.clone(),
        });
        DfSessionService {
            session_context,
            parser,
            query_hooks,
        }
    }
}

#[async_trait]
impl SimpleQueryHandler for DfSessionService {
    async fn do_query<C>(&self, client: &mut C, query: &str) -> PgWireResult<Vec<Response>>
    where
        C: ClientInfo
            + ClientPortalStore
            + futures::Sink<PgWireBackendMessage>
            + Unpin
            + Send
            + Sync,
        C::PortalStore: PortalStore,
        C::Error: std::fmt::Debug,
        PgWireError: From<<C as futures::Sink<PgWireBackendMessage>>::Error>,
    {
        log::debug!("Received query: {query}");

        // Preprocess PostGIS-specific syntax that DataFusion's parser/planner
        // doesn't understand. Low-maintenance string-level transforms for
        // patterns that have exact semantic equivalents in DataFusion.
        // — ::geometry / ::geography → ::bytea (WKB IS bytea; the type
        //   difference only matters for pg_type introspection, not for data
        //   transfer, and QuackGIS registers geometry_columns separately).
        let query = preprocess_sql_with_hooks(query, &self.query_hooks);

        let mut statements = self
            .parser
            .sql_parser
            .parse(&query)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        rewrite_pg_overlap(&mut statements);

        // empty query
        if statements.is_empty() {
            return Ok(vec![Response::EmptyQuery]);
        }

        let mut results = vec![];
        'stmt: for statement in statements {
            // Call query hooks with the parsed statement
            for hook in &self.query_hooks {
                if let Some(result) = hook
                    .handle_simple_query(&statement, &self.session_context, client)
                    .await
                {
                    results.push(result?);
                    continue 'stmt;
                }
            }

            let df_result = {
                let query = statement.to_string();

                let timeout = client::get_statement_timeout(client);
                if let Some(timeout_duration) = timeout {
                    tokio::time::timeout(timeout_duration, self.session_context.sql(&query))
                        .await
                        .map_err(|_| {
                            PgWireError::UserError(Box::new(pgwire::error::ErrorInfo::new(
                                "ERROR".to_string(),
                                "57014".to_string(), // query_canceled error code
                                "canceling statement due to statement timeout".to_string(),
                            )))
                        })?
                } else {
                    self.session_context.sql(&query).await
                }
            };

            // Handle query execution errors and transaction state
            let df = match df_result {
                Ok(df) => df,
                Err(e) => {
                    return Err(PgWireError::ApiError(Box::new(e)));
                }
            };

            if matches!(statement, sqlparser::ast::Statement::Insert(_)) {
                let resp = map_rows_affected_for_insert(&df).await?;
                results.push(resp);
            } else {
                // For non-INSERT queries, return a regular Query response
                let format_options =
                    Arc::new(FormatOptions::from_client_metadata(client.metadata()));
                let resp =
                    df::encode_dataframe(df, &Format::UnifiedText, Some(format_options)).await?;
                results.push(Response::Query(resp));
            }
        }
        Ok(results)
    }
}

#[async_trait]
impl ExtendedQueryHandler for DfSessionService {
    type Statement = (String, Option<(sqlparser::ast::Statement, LogicalPlan)>);
    type QueryParser = Parser;

    fn query_parser(&self) -> Arc<Self::QueryParser> {
        self.parser.clone()
    }

    async fn do_query<C>(
        &self,
        client: &mut C,
        portal: &Portal<Self::Statement>,
        _max_rows: usize,
    ) -> PgWireResult<Response>
    where
        C: ClientInfo
            + ClientPortalStore
            + futures::Sink<PgWireBackendMessage>
            + Unpin
            + Send
            + Sync,
        C::PortalStore: PortalStore,
        C::Error: std::fmt::Debug,
        PgWireError: From<<C as futures::Sink<PgWireBackendMessage>>::Error>,
    {
        let query = &portal.statement.statement.0;
        log::debug!("Received execute extended query: {query}");
        if let (_, Some((statement, prepared_plan))) = &portal.statement.statement {
            let param_types = planner::get_inferred_parameter_types(prepared_plan)
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?;

            let param_values: ParamValues =
                df::deserialize_parameters(portal, &ordered_param_types(&param_types))?;

            for hook in &self.query_hooks {
                if let Some(result) = hook
                    .handle_extended_query(
                        statement,
                        prepared_plan,
                        &param_values,
                        &self.session_context,
                        client,
                    )
                    .await
                {
                    return result;
                }
            }

            let mut execute_plan = ExtendedQueryPlan {
                logical_plan: prepared_plan.clone(),
                session_context: self.session_context.as_ref().clone(),
            };
            for hook in &self.query_hooks {
                if let Some(result) = hook
                    .replan_extended_query(statement, prepared_plan, &self.session_context)
                    .await
                {
                    execute_plan = result?;
                    break;
                }
            }
            if execute_plan.logical_plan.schema() != prepared_plan.schema() {
                return Err(PgWireError::ApiError(Box::new(DataFusionError::Plan(
                    "cached plan must not change result type".to_string(),
                ))));
            }
            let plan = execute_plan
                .logical_plan
                .replace_params_with_values(&param_values)
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
            let optimised = execute_plan
                .session_context
                .state()
                .optimize(&plan)
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
            let dataframe = match client::get_statement_timeout(client) {
                Some(timeout_duration) => tokio::time::timeout(
                    timeout_duration,
                    execute_plan.session_context.execute_logical_plan(optimised),
                )
                .await
                .map_err(|_| {
                    PgWireError::UserError(Box::new(pgwire::error::ErrorInfo::new(
                        "ERROR".to_string(),
                        "57014".to_string(),
                        "canceling statement due to statement timeout".to_string(),
                    )))
                })?
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
                None => execute_plan
                    .session_context
                    .execute_logical_plan(optimised)
                    .await
                    .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
            };

            if matches!(statement, sqlparser::ast::Statement::Insert(_)) {
                map_rows_affected_for_insert(&dataframe).await
            } else {
                let format_options =
                    Arc::new(FormatOptions::from_client_metadata(client.metadata()));
                let response = df::encode_dataframe(
                    dataframe,
                    &portal.result_column_format,
                    Some(format_options),
                )
                .await?;
                Ok(Response::Query(response))
            }
        } else {
            Ok(Response::EmptyQuery)
        }
    }
}

async fn map_rows_affected_for_insert(df: &DataFrame) -> PgWireResult<Response> {
    // For INSERT queries, we need to execute the query to get the row count
    // and return an Execution response with the proper tag
    let result = df
        .clone()
        .collect()
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;

    // Extract count field from the first batch
    let rows_affected = result
        .first()
        .and_then(|batch| batch.column_by_name("count"))
        .and_then(|col| {
            col.as_any()
                .downcast_ref::<datafusion::arrow::array::UInt64Array>()
        })
        .map_or(0, |array| array.value(0) as usize);

    // Create INSERT tag with the affected row count
    let tag = Tag::new("INSERT").with_oid(0).with_rows(rows_affected);
    Ok(Response::Execution(tag))
}

pub struct Parser {
    session_context: Arc<SessionContext>,
    sql_parser: PostgresCompatibilityParser,
    query_hooks: Vec<Arc<dyn QueryHook>>,
}

#[async_trait]
impl QueryParser for Parser {
    type Statement = (String, Option<(sqlparser::ast::Statement, LogicalPlan)>);

    async fn parse_sql<C>(
        &self,
        client: &C,
        sql: &str,
        _types: &[Option<Type>],
    ) -> PgWireResult<Self::Statement>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        log::debug!("Received parse extended query: {sql}");
        let sql = preprocess_sql_with_hooks(sql, &self.query_hooks);
        let mut statements = self
            .sql_parser
            .parse(&sql)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        rewrite_pg_overlap(&mut statements);
        if statements.is_empty() {
            return Ok((sql, None));
        }

        let statement = statements.remove(0);
        let query = statement.to_string();

        let context = &self.session_context;
        let state = context.state();

        for hook in &self.query_hooks {
            if let Some(logical_plan) = hook
                .handle_extended_parse_query(&statement, context, client)
                .await
            {
                return Ok((query, Some((statement, logical_plan?))));
            }
        }

        let logical_plan = state
            .statement_to_plan(Statement::Statement(Box::new(statement.clone())))
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        Ok((query, Some((statement, logical_plan))))
    }

    fn get_parameter_types(&self, stmt: &Self::Statement) -> PgWireResult<Vec<Type>> {
        if let (query, Some((_, plan))) = stmt {
            if let Some(types) = postgres_catalog_oid_parameter_types(query) {
                return Ok(types);
            }
            let params = planner::get_inferred_parameter_types(plan)
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?;

            let mut param_types = Vec::with_capacity(params.len());
            for param_type in ordered_param_types(&params).iter() {
                if let Some(datatype) = param_type {
                    let pgtype = into_pg_type(datatype)?;
                    param_types.push(pgtype);
                } else {
                    param_types.push(Type::UNKNOWN);
                }
            }

            Ok(param_types)
        } else {
            Ok(vec![])
        }
    }

    fn get_result_schema(
        &self,
        stmt: &Self::Statement,
        column_format: Option<&Format>,
    ) -> PgWireResult<Vec<FieldInfo>> {
        if let (_, Some((_, plan))) = stmt {
            if !matches!(plan, LogicalPlan::Ddl(_) | LogicalPlan::Dml(_)) {
                let schema = plan.schema();
                let fields = arrow_schema_to_pg_fields(
                    schema.as_arrow(),
                    column_format.unwrap_or(&Format::UnifiedText),
                    None,
                )?;

                Ok(fields)
            } else {
                Ok(vec![])
            }
        } else {
            Ok(vec![])
        }
    }
}

fn postgres_catalog_oid_parameter_types(query: &str) -> Option<Vec<Type>> {
    let normalized = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    // tokio-postgres resolves unknown type OIDs by preparing internal catalog
    // lookups whose `$1` parameter is an OID. DataFusion infers those columns as
    // Int32 because the backing Arrow tables store raw oid values as Int32, but
    // PostgreSQL advertises the parameter as `oid`; without this override the
    // client tries to serialize a Rust u32 into int4 and fails before binding.
    if (normalized.contains("from pg_catalog.pg_type t") && normalized.contains("where t.oid = $1"))
        || (normalized.contains("from pg_catalog.pg_enum")
            && normalized.contains("where enumtypid = $1"))
        || (normalized.contains("from pg_catalog.pg_attribute")
            && normalized.contains("where attrelid = $1"))
    {
        return Some(vec![Type::OID]);
    }

    None
}

fn ordered_param_types(types: &HashMap<String, Option<DataType>>) -> Vec<Option<&DataType>> {
    // Datafusion stores the parameters as a map.  In our case, the keys will be
    // `$1`, `$2` etc.  The values will be the parameter types.
    let mut types = types.iter().collect::<Vec<_>>();
    types.sort_by_key(|(key, _)| {
        key.trim_start_matches('$')
            .parse::<u32>()
            .unwrap_or(u32::MAX)
    });
    types.into_iter().map(|pt| pt.1.as_ref()).collect()
}

fn preprocess_sql_with_hooks(sql: &str, hooks: &[Arc<dyn QueryHook>]) -> String {
    let mut sql = preprocess_quackgis_sql(sql);
    for hook in hooks {
        if let Some(rewritten) = hook.rewrite_sql(&sql) {
            sql = rewritten;
        }
    }
    sql
}

fn preprocess_quackgis_sql(sql: &str) -> String {
    // Preprocess PostGIS-specific syntax that DataFusion's parser/planner
    // doesn't understand. Low-maintenance string-level transforms for
    // patterns that have exact semantic equivalents in DataFusion.
    let sql = rewrite_postgis_ddl(&sql);

    let sql = sql
        .replace("::geometry", "::bytea")
        .replace("::Geometry", "::bytea")
        .replace("::GEOMETRY", "::bytea")
        .replace("::geography", "::bytea")
        .replace("::Geography", "::bytea")
        .replace("::GEOGRAPHY", "::bytea")
        .replace("::jsonb", "::varchar")
        .replace("::Jsonb", "::varchar")
        .replace("::JSONB", "::varchar")
        // DataFusion does not support PostgreSQL named function arguments.
        // Martin uses this optional ST_TileEnvelope margin for bbox filtering;
        // QuackGIS accepts a 4th positional Float64 as a margin-with-default-
        // bounds compatibility overload.
        .replace(", margin => 0.015625", ", 0.015625");

    if is_martin_available_tables_query(&sql) {
        return r#"
SELECT
    gc.f_table_schema AS schema,
    gc.f_table_name AS name,
    gc.spatial_column AS geom,
    gc.srid,
    gc.type,
    CAST(NULL AS TINYINT) AS relkind,
    CAST(FALSE AS BOOLEAN) AS geom_idx,
    CAST(NULL AS VARCHAR) AS description,
    CAST(
        COALESCE(
            CONCAT(
                '{',
                STRING_AGG(
                    CONCAT('"', attr.attname, '":"', tp.typname, '"'),
                    ','
                ),
                '}'
            ),
            '{}'
        ) AS VARCHAR
    ) AS properties
FROM (
    SELECT f_table_schema, f_table_name, f_geometry_column AS spatial_column, srid, type
    FROM geometry_columns
    UNION ALL
    SELECT f_table_schema, f_table_name, f_geography_column AS spatial_column, srid, type
    FROM geography_columns
) AS gc
LEFT JOIN pg_catalog.pg_namespace AS ns
    ON gc.f_table_schema = ns.nspname
LEFT JOIN pg_catalog.pg_class AS cls
    ON ns.oid = cls.relnamespace AND gc.f_table_name = cls.relname
LEFT JOIN pg_catalog.pg_attribute AS attr
    ON cls.oid = attr.attrelid
    AND attr.attnum > 0
    AND NOT attr.attisdropped
    AND attr.attname != gc.spatial_column
LEFT JOIN pg_catalog.pg_type AS tp
    ON attr.atttypid = tp.oid
GROUP BY
    gc.f_table_schema,
    gc.f_table_name,
    gc.spatial_column,
    gc.srid,
    gc.type
"#
        .to_string();
    }

    if is_martin_available_functions_query(&sql) {
        return r#"
SELECT
    CAST(NULL AS VARCHAR) AS schema,
    CAST(NULL AS VARCHAR) AS name,
    CAST(NULL AS VARCHAR) AS output_type,
    CAST(NULL AS VARCHAR) AS output_record_types,
    CAST(NULL AS VARCHAR) AS output_record_names,
    CAST(NULL AS VARCHAR) AS input_types,
    CAST(NULL AS VARCHAR) AS input_names,
    CAST(NULL AS VARCHAR) AS description
WHERE FALSE
"#
        .to_string();
    }

    sql
}

fn is_martin_available_tables_query(sql: &str) -> bool {
    sql.contains("annotated_geometry_columns")
        && sql.contains("annotated_geography_columns")
        && sql.contains("jsonb_object_agg")
        && sql.contains("FROM geometry_columns")
        && sql.contains("FROM geography_columns")
}

fn is_martin_available_functions_query(sql: &str) -> bool {
    sql.contains("jsonb_array_length(inputs.input_names)")
        && sql.contains("information_schema.routines")
        && sql.contains("information_schema.parameters")
        && sql.contains("inputs.input_types ->> 0")
}

/// Rewrite PostGIS/PostgreSQL DDL that DataFusion's parser cannot handle.
///
/// - `CREATE EXTENSION ...` → `SELECT 1 WHERE FALSE` (no-op)
/// - `DO $$ ... $$` PL/pgSQL blocks → no-op
/// - `CREATE INDEX ...` and `CLUSTER ...` → no-op (QuackGIS has no GiST yet)
/// - `COMMENT ON ...` → no-op
/// - `CREATE MATERIALIZED VIEW ...` → `CREATE VIEW ...`
/// - `serial` / `bigserial` column types → `int` / `bigint`
///
/// Geometry/geography column declarations are intentionally left intact for a
/// QuackGIS DDL hook to annotate before lowering to physical Arrow Binary.
fn rewrite_postgis_ddl(sql: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;

    static CREATE_EXT_RE: OnceLock<Regex> = OnceLock::new();
    static DO_BLOCK_RE: OnceLock<Regex> = OnceLock::new();
    static CREATE_INDEX_RE: OnceLock<Regex> = OnceLock::new();
    static CLUSTER_RE: OnceLock<Regex> = OnceLock::new();
    static COMMENT_ON_RE: OnceLock<Regex> = OnceLock::new();
    static CREATE_MATERIALIZED_VIEW_RE: OnceLock<Regex> = OnceLock::new();
    static SERIAL_RE: OnceLock<Regex> = OnceLock::new();

    let create_ext = CREATE_EXT_RE.get_or_init(|| {
        Regex::new(r"(?i)\bCREATE\s+EXTENSION\s+IF\s+NOT\s+EXISTS\s+\w+\s*;|\bCREATE\s+EXTENSION\s+\w+\s*;")
            .unwrap()
    });
    let do_block =
        DO_BLOCK_RE.get_or_init(|| Regex::new(r"(?is)\bDO\s+\$[\w$]*\$.*?\$[\w$]*\s*;").unwrap());
    let create_index = CREATE_INDEX_RE.get_or_init(|| {
        Regex::new(r"(?is)\bCREATE\s+(?:UNIQUE\s+)?INDEX\s+(?:CONCURRENTLY\s+)?.*?;").unwrap()
    });
    let cluster = CLUSTER_RE.get_or_init(|| Regex::new(r"(?is)\bCLUSTER\s+.*?;").unwrap());
    let comment_on =
        COMMENT_ON_RE.get_or_init(|| Regex::new(r"(?is)\bCOMMENT\s+ON\s+.*?;").unwrap());
    let create_materialized_view = CREATE_MATERIALIZED_VIEW_RE
        .get_or_init(|| Regex::new(r"(?i)\bCREATE\s+MATERIALIZED\s+VIEW\b").unwrap());
    let serial = SERIAL_RE.get_or_init(|| Regex::new(r"(?i)\bbigserial\b|\bserial\b").unwrap());

    let sql = create_ext.replace_all(&sql, "SELECT 1 WHERE FALSE;");
    let sql = do_block.replace_all(&sql, "SELECT 1 WHERE FALSE;");
    let sql = create_index.replace_all(&sql, "SELECT 1 WHERE FALSE;");
    let sql = cluster.replace_all(&sql, "SELECT 1 WHERE FALSE;");
    let sql = comment_on.replace_all(&sql, "SELECT 1 WHERE FALSE;");
    let sql = create_materialized_view.replace_all(&sql, "CREATE VIEW");
    let sql = serial.replace_all(&sql, |caps: &regex::Captures| {
        let matched = caps[0].to_lowercase();
        if matched == "bigserial" {
            "bigint"
        } else {
            "int"
        }
    });

    sql.into_owned()
}

#[cfg(test)]
mod tests {
    use datafusion::prelude::SessionContext;

    use super::*;
    use crate::testing::MockClient;

    use crate::hooks::HookClient;

    struct TestHook;

    #[async_trait]
    impl QueryHook for TestHook {
        fn rewrite_sql(&self, sql: &str) -> Option<String> {
            (sql == "SELECT * FROM raw_rewrite AS OF SNAPSHOT 1")
                .then(|| "SELECT magic".to_string())
        }

        async fn handle_simple_query(
            &self,
            statement: &sqlparser::ast::Statement,
            _ctx: &SessionContext,
            _client: &mut dyn HookClient,
        ) -> Option<PgWireResult<Response>> {
            if statement.to_string().contains("magic") {
                Some(Ok(Response::EmptyQuery))
            } else {
                None
            }
        }

        async fn handle_extended_parse_query(
            &self,
            _statement: &sqlparser::ast::Statement,
            _session_context: &SessionContext,
            _client: &(dyn ClientInfo + Send + Sync),
        ) -> Option<PgWireResult<LogicalPlan>> {
            None
        }

        async fn handle_extended_query(
            &self,
            _statement: &sqlparser::ast::Statement,
            _logical_plan: &LogicalPlan,
            _params: &ParamValues,
            _session_context: &SessionContext,
            _client: &mut dyn HookClient,
        ) -> Option<PgWireResult<Response>> {
            None
        }
    }

    #[test]
    fn test_ordered_param_types_sorts_placeholders_numerically() {
        let params = HashMap::from([
            ("$1".to_string(), Some(DataType::Boolean)),
            ("$2".to_string(), Some(DataType::Int64)),
            ("$10".to_string(), Some(DataType::Utf8)),
        ]);

        let ordered = ordered_param_types(&params)
            .into_iter()
            .map(|ty| ty.cloned())
            .collect::<Vec<_>>();

        assert_eq!(
            ordered,
            vec![
                Some(DataType::Boolean),
                Some(DataType::Int64),
                Some(DataType::Utf8)
            ]
        );
    }

    #[test]
    fn test_preprocess_quackgis_sql_strips_martin_tileenvelope_margin() {
        let sql =
            "SELECT ST_TileEnvelope($1::integer, $2::integer, $3::integer, margin => 0.015625)";

        assert_eq!(
            preprocess_quackgis_sql(sql),
            "SELECT ST_TileEnvelope($1::integer, $2::integer, $3::integer, 0.015625)"
        );
    }

    #[test]
    fn test_rewrite_postgis_ddl_creates_table_with_geometry_and_serial() {
        let sql = "CREATE TABLE table_source (\n    gid serial PRIMARY KEY, geom GEOMETRY (GEOMETRY, 4326)\n)";
        let result = rewrite_postgis_ddl(sql);
        assert!(
            result.contains("int"),
            "serial should be replaced: {result}"
        );
        assert!(
            result.contains("GEOMETRY"),
            "GEOMETRY should survive: {result}"
        );
        assert!(
            !result.to_lowercase().contains("serial"),
            "no serial should remain: {result}"
        );
        assert!(
            result.contains("4326"),
            "geometry modifiers should survive: {result}"
        );
    }

    #[test]
    fn test_rewrite_postgis_ddl_skips_create_extension() {
        let sql = "CREATE EXTENSION IF NOT EXISTS postgis; CREATE TABLE foo (x int);";
        let result = rewrite_postgis_ddl(sql);
        assert!(
            result.contains("SELECT 1 WHERE FALSE"),
            "CREATE EXTENSION should be no-op: {result}"
        );
        assert!(
            result.contains("CREATE TABLE foo"),
            "CREATE TABLE should be preserved: {result}"
        );
    }

    #[test]
    fn test_rewrite_postgis_ddl_skips_do_blocks() {
        let sql = "DO $do$ BEGIN EXECUTE 'COMMENT ON TABLE foo'; END $do$;";
        let result = rewrite_postgis_ddl(sql);
        assert!(
            result.contains("SELECT 1 WHERE FALSE"),
            "DO block should be no-op: {result}"
        );
    }

    #[test]
    fn test_rewrite_postgis_ddl_handles_geography_type() {
        let sql = "CREATE TABLE t (geom geography(Point, 4326))";
        let result = rewrite_postgis_ddl(sql);
        assert_eq!(result, sql);
    }

    #[test]
    fn test_preprocess_preserves_spatial_declarations_but_lowers_casts() {
        let sql = "CREATE TABLE t (location GEOMETRY(Point,4326), earth GEOGRAPHY(Point,4326)); SELECT location::geometry, earth::geography FROM t";
        let result = preprocess_quackgis_sql(sql);
        assert!(result.contains("location GEOMETRY(Point,4326)"));
        assert!(result.contains("earth GEOGRAPHY(Point,4326)"));
        assert!(result.contains("location::bytea"));
        assert!(result.contains("earth::bytea"));
    }

    #[test]
    fn test_rewrite_postgis_ddl_skips_index_and_cluster() {
        let sql = "CREATE INDEX CONCURRENTLY ON points1 USING gist (geom); CLUSTER points1_geom_idx ON points1;";
        let result = rewrite_postgis_ddl(sql);
        assert_eq!(result, "SELECT 1 WHERE FALSE; SELECT 1 WHERE FALSE;");
    }

    #[test]
    fn test_rewrite_postgis_ddl_materialized_view_as_view() {
        let sql = "CREATE MATERIALIZED VIEW mat_view AS SELECT 1 AS id";
        let result = rewrite_postgis_ddl(sql);
        assert_eq!(result, "CREATE VIEW mat_view AS SELECT 1 AS id");
    }

    #[test]
    fn test_rewrite_postgis_ddl_skips_comments() {
        let sql = "COMMENT ON TABLE points2 IS 'A table with points'; COMMENT ON COLUMN points2.geom IS 'The geometry column';";
        let result = rewrite_postgis_ddl(sql);
        assert_eq!(result, "SELECT 1 WHERE FALSE; SELECT 1 WHERE FALSE;");
    }

    #[tokio::test]
    async fn test_query_hooks() {
        let hook = TestHook;
        let ctx = SessionContext::new();
        let mut client = MockClient::new();

        // Parse a statement that contains "magic"
        let parser = PostgresCompatibilityParser::new();
        let statements = parser.parse("SELECT magic").unwrap();
        let stmt = &statements[0];

        // Hook should intercept
        let result = hook.handle_simple_query(stmt, &ctx, &mut client).await;
        assert!(result.is_some());

        // Parse a normal statement
        let statements = parser.parse("SELECT 1").unwrap();
        let stmt = &statements[0];

        // Hook should not intercept
        let result = hook.handle_simple_query(stmt, &ctx, &mut client).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_multiple_statements_with_hook_continue() {
        // Bug #227: when a hook returned a result, the code used `break 'stmt`
        // which would exit the entire statement loop, preventing subsequent statements
        // from being processed.
        let session_context = Arc::new(SessionContext::new());

        let hooks: Vec<Arc<dyn QueryHook>> = vec![Arc::new(TestHook)];
        let service = DfSessionService::new_with_hooks(session_context, hooks);

        let mut client = MockClient::new();

        // Mix of queries with hooks and those without
        let query = "SELECT magic; SELECT 1; SELECT magic; SELECT 1";

        let results =
            <DfSessionService as SimpleQueryHandler>::do_query(&service, &mut client, query)
                .await
                .unwrap();

        assert_eq!(results.len(), 4, "Expected 4 responses");

        assert!(matches!(results[0], Response::EmptyQuery));
        assert!(matches!(results[1], Response::Query(_)));
        assert!(matches!(results[2], Response::EmptyQuery));
        assert!(matches!(results[3], Response::Query(_)));
    }

    #[tokio::test]
    async fn test_rewrite_sql_hook_runs_before_simple_query_parse() {
        let session_context = Arc::new(SessionContext::new());
        let hooks: Vec<Arc<dyn QueryHook>> = vec![Arc::new(TestHook)];
        let service = DfSessionService::new_with_hooks(session_context, hooks);
        let mut client = MockClient::new();

        let results = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "SELECT * FROM raw_rewrite AS OF SNAPSHOT 1",
        )
        .await
        .unwrap();

        assert_eq!(results.len(), 1);
        assert!(matches!(results[0], Response::EmptyQuery));
    }

    #[tokio::test]
    async fn test_set_sends_parameter_status_via_sink() {
        use pgwire::messages::PgWireBackendMessage;

        let service = crate::testing::setup_handlers();
        let mut client = MockClient::new();

        let test_cases = vec![
            ("SET datestyle = 'ISO, MDY'", "DateStyle", "ISO, MDY"),
            (
                "SET intervalstyle = 'postgres'",
                "IntervalStyle",
                "postgres",
            ),
            ("SET bytea_output = 'hex'", "bytea_output", "hex"),
            (
                "SET application_name = 'myapp'",
                "application_name",
                "myapp",
            ),
            ("SET search_path = 'public'", "search_path", "public"),
            ("SET extra_float_digits = '2'", "extra_float_digits", "2"),
            (
                "SET TIME ZONE 'America/New_York'",
                "TimeZone",
                "America/New_York",
            ),
        ];

        for (sql, expected_key, expected_value) in test_cases {
            client.sent_messages.clear();

            let responses =
                <DfSessionService as SimpleQueryHandler>::do_query(&service, &mut client, sql)
                    .await
                    .unwrap();

            assert!(
                matches!(responses[0], Response::Execution(_)),
                "Expected SET tag for {sql}"
            );

            let ps_msgs: Vec<_> = client
                .sent_messages()
                .iter()
                .filter_map(|m| match m {
                    PgWireBackendMessage::ParameterStatus(ps) => Some(ps),
                    _ => None,
                })
                .collect();

            assert_eq!(ps_msgs.len(), 1, "Expected 1 ParameterStatus for {sql}");
            assert_eq!(ps_msgs[0].name, expected_key, "Wrong key for {sql}");
            assert_eq!(ps_msgs[0].value, expected_value, "Wrong value for {sql}");
        }
    }

    #[tokio::test]
    async fn test_set_statement_timeout_no_parameter_status() {
        use pgwire::messages::PgWireBackendMessage;

        let service = crate::testing::setup_handlers();
        let mut client = MockClient::new();

        <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "SET statement_timeout TO '5000ms'",
        )
        .await
        .unwrap();

        let has_ps = client
            .sent_messages()
            .iter()
            .any(|m| matches!(m, PgWireBackendMessage::ParameterStatus(_)));

        assert!(!has_ps, "statement_timeout should not send ParameterStatus");
    }

    fn assert_execution_tag(response: &Response, expected: &str) {
        match response {
            Response::Execution(tag) => {
                let cc = pgwire::messages::response::CommandComplete::from(tag.clone());
                assert_eq!(cc.tag, expected, "Unexpected execution tag");
            }
            other => panic!("Expected Execution response, got: {other:?}"),
        }
    }

    async fn assert_query_response_empty(response: &mut Response) {
        use futures::StreamExt;

        let Response::Query(qr) = response else {
            panic!("Expected Query response, got: {response:?}");
        };

        let mut count = 0;
        while qr.data_rows().next().await.is_some() {
            count += 1;
        }
        assert_eq!(count, 0, "Expected no rows from exhausted cursor");
    }

    #[tokio::test]
    async fn test_declare_fetch_close_cursor() {
        let service = crate::testing::setup_handlers();
        let mut client = MockClient::new();

        let responses = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "DECLARE test_cursor CURSOR FOR SELECT 1 AS col",
        )
        .await
        .unwrap();

        assert_eq!(responses.len(), 1);
        assert_execution_tag(&responses[0], "DECLARE CURSOR");

        let responses = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "FETCH NEXT FROM test_cursor",
        )
        .await
        .unwrap();

        assert_eq!(responses.len(), 1);
        assert!(
            matches!(&responses[0], Response::Query(_)),
            "Expected Query response for FETCH"
        );

        let mut responses = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "FETCH NEXT FROM test_cursor",
        )
        .await
        .unwrap();

        assert_eq!(responses.len(), 1);
        assert_query_response_empty(&mut responses[0]).await;

        let responses = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "CLOSE test_cursor",
        )
        .await
        .unwrap();

        assert_eq!(responses.len(), 1);
        assert_execution_tag(&responses[0], "CLOSE CURSOR");
    }

    #[tokio::test]
    async fn test_fetch_nonexistent_cursor() {
        let service = crate::testing::setup_handlers();
        let mut client = MockClient::new();

        let result = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "FETCH NEXT FROM nonexistent",
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_close_all_portals() {
        let service = crate::testing::setup_handlers();
        let mut client = MockClient::new();

        <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "DECLARE c1 CURSOR FOR SELECT 1",
        )
        .await
        .unwrap();

        <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "DECLARE c2 CURSOR FOR SELECT 2",
        )
        .await
        .unwrap();

        let responses =
            <DfSessionService as SimpleQueryHandler>::do_query(&service, &mut client, "CLOSE ALL")
                .await
                .unwrap();

        assert!(matches!(&responses[0], Response::Execution(_)),);

        let result = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "FETCH NEXT FROM c1",
        )
        .await;
        assert!(result.is_err(), "c1 should be closed");
    }

    #[tokio::test]
    async fn test_fetch_forward_n() {
        let service = crate::testing::setup_handlers();
        let mut client = MockClient::new();

        <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "CREATE TABLE nums AS SELECT 1 AS n UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5",
        )
        .await
        .unwrap();

        <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "DECLARE mycur CURSOR FOR SELECT n FROM nums ORDER BY n",
        )
        .await
        .unwrap();

        let responses = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "FETCH FORWARD 3 FROM mycur",
        )
        .await
        .unwrap();

        assert!(
            matches!(&responses[0], Response::Query(_)),
            "Expected Query response for FORWARD 3"
        );

        let responses = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "FETCH FORWARD ALL FROM mycur",
        )
        .await
        .unwrap();

        let resp_desc = match &responses[0] {
            Response::Query(_) => "Query".to_string(),
            Response::Execution(tag) => {
                let cc = pgwire::messages::response::CommandComplete::from(tag.clone());
                format!("Execution({})", cc.tag)
            }
            other => format!("{:?}", other),
        };
        assert!(
            matches!(&responses[0], Response::Query(_)),
            "Expected Query response for remaining rows, got: {resp_desc}"
        );

        let mut responses = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "FETCH NEXT FROM mycur",
        )
        .await
        .unwrap();

        assert_query_response_empty(&mut responses[0]).await;
    }

    #[tokio::test]
    async fn test_scroll_cursor_error() {
        let service = crate::testing::setup_handlers();
        let mut client = MockClient::new();

        <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "DECLARE mycur CURSOR FOR SELECT 1",
        )
        .await
        .unwrap();

        let result = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "FETCH PRIOR FROM mycur",
        )
        .await;

        assert!(result.is_err(), "PRIOR should fail on forward-only cursor");
    }

    #[tokio::test]
    async fn test_move_cursor() {
        let service = crate::testing::setup_handlers();
        let mut client = MockClient::new();

        <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "DECLARE mycur CURSOR FOR SELECT generate_series(1, 5) AS n",
        )
        .await
        .unwrap();

        let responses = <DfSessionService as SimpleQueryHandler>::do_query(
            &service,
            &mut client,
            "FETCH FORWARD 3 FROM mycur",
        )
        .await
        .unwrap();

        assert!(matches!(&responses[0], Response::Query(_)));
    }
}

/// Walk a parsed SQL statement and rewrite PostGIS `&&` (PGOverlap) operators
/// to `st_overlaps_bbox(left, right)` function calls. DataFusion's planner
/// doesn't recognize PGOverlap; this rewrite makes it transparent.
fn rewrite_pg_overlap(statements: &mut [sqlparser::ast::Statement]) {
    for stmt in statements.iter_mut() {
        rewrite_pg_overlap_stmt(stmt);
    }
}

fn rewrite_pg_overlap_stmt(stmt: &mut sqlparser::ast::Statement) {
    match stmt {
        sqlparser::ast::Statement::Query(q) => {
            rewrite_pg_overlap_query(q);
        }
        _ => {}
    }
}

fn rewrite_pg_overlap_query(q: &mut sqlparser::ast::Query) {
    rewrite_pg_overlap_set_expr(&mut q.body);
}

fn rewrite_pg_overlap_set_expr(expr: &mut sqlparser::ast::SetExpr) {
    match expr {
        sqlparser::ast::SetExpr::Select(select) => {
            if let Some(selection) = select.selection.as_mut() {
                rewrite_pg_overlap_expr(selection);
            }
            rewrite_postgis_st_asmvt_record_select(select);
            for expr in select.projection.iter_mut() {
                rewrite_pg_overlap_select_item(expr);
            }
            for table in select.from.iter_mut() {
                rewrite_pg_overlap_table_with_joins(table);
            }
        }
        _ => {}
    }
}

/// Expand Martin's PostgreSQL record-form `ST_AsMVT(tile, ..., 'geom')` into
/// QuackGIS's scalar aggregate arguments. The derived table projection is the
/// authoritative record shape, so non-geometry columns become MVT attributes
/// instead of being silently discarded by the geometry-only fallback.
fn rewrite_postgis_st_asmvt_record_select(select: &mut sqlparser::ast::Select) {
    let Some((record_alias, projected_columns)) = martin_record_projection(select) else {
        return;
    };

    for item in &mut select.projection {
        let expr = match item {
            sqlparser::ast::SelectItem::ExprWithAlias { expr, .. }
            | sqlparser::ast::SelectItem::UnnamedExpr(expr) => expr,
            _ => continue,
        };
        let sqlparser::ast::Expr::Function(func) = expr else {
            continue;
        };
        let Some(name) = func.name.0.last() else {
            continue;
        };
        if !name.to_string().eq_ignore_ascii_case("st_asmvt") {
            continue;
        }
        let sqlparser::ast::FunctionArguments::List(list) = &mut func.args else {
            continue;
        };
        if list.args.len() < 4 {
            continue;
        }
        let matches_alias = matches!(
            &list.args[0],
            sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(
                sqlparser::ast::Expr::Identifier(ident)
            )) if ident.value.eq_ignore_ascii_case(&record_alias)
        );
        if !matches_alias {
            continue;
        }
        let geom_col = match &list.args[3] {
            sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(
                sqlparser::ast::Expr::Value(value),
            )) => match &value.value {
                sqlparser::ast::Value::SingleQuotedString(value)
                | sqlparser::ast::Value::DoubleQuotedString(value) => value.clone(),
                _ => continue,
            },
            _ => continue,
        };
        if !projected_columns
            .iter()
            .any(|column| column.eq_ignore_ascii_case(&geom_col))
        {
            continue;
        }

        let layer_name = list.args[1].clone();
        let extent = list.args[2].clone();
        let mut expanded = vec![
            qualified_function_arg(&record_alias, &geom_col),
            layer_name,
            extent,
        ];
        expanded.extend(
            projected_columns
                .iter()
                .filter(|column| !column.eq_ignore_ascii_case(&geom_col))
                .map(|column| qualified_function_arg(&record_alias, column)),
        );
        list.args = expanded;
    }
}

fn martin_record_projection(select: &sqlparser::ast::Select) -> Option<(String, Vec<String>)> {
    let [from] = select.from.as_slice() else {
        return None;
    };
    if !from.joins.is_empty() {
        return None;
    }
    let sqlparser::ast::TableFactor::Derived {
        subquery,
        alias: Some(alias),
        ..
    } = &from.relation
    else {
        return None;
    };
    let sqlparser::ast::SetExpr::Select(derived) = subquery.body.as_ref() else {
        return None;
    };
    let mut columns = Vec::with_capacity(derived.projection.len());
    for item in &derived.projection {
        let name = match item {
            sqlparser::ast::SelectItem::ExprWithAlias { alias, .. } => alias.value.clone(),
            sqlparser::ast::SelectItem::UnnamedExpr(sqlparser::ast::Expr::Identifier(ident)) => {
                ident.value.clone()
            }
            sqlparser::ast::SelectItem::UnnamedExpr(sqlparser::ast::Expr::CompoundIdentifier(
                idents,
            )) => idents.last()?.value.clone(),
            _ => return None,
        };
        columns.push(name);
    }
    Some((alias.name.value.clone(), columns))
}

fn qualified_function_arg(record_alias: &str, column: &str) -> sqlparser::ast::FunctionArg {
    sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(
        sqlparser::ast::Expr::CompoundIdentifier(vec![
            sqlparser::ast::Ident::new(record_alias),
            sqlparser::ast::Ident::new(column),
        ]),
    ))
}

fn rewrite_pg_overlap_table_with_joins(table: &mut sqlparser::ast::TableWithJoins) {
    rewrite_pg_overlap_table_factor(&mut table.relation);
    for join in table.joins.iter_mut() {
        rewrite_pg_overlap_table_factor(&mut join.relation);
        match &mut join.join_operator {
            sqlparser::ast::JoinOperator::Inner(constraint)
            | sqlparser::ast::JoinOperator::LeftOuter(constraint)
            | sqlparser::ast::JoinOperator::RightOuter(constraint)
            | sqlparser::ast::JoinOperator::FullOuter(constraint)
            | sqlparser::ast::JoinOperator::LeftSemi(constraint)
            | sqlparser::ast::JoinOperator::RightSemi(constraint)
            | sqlparser::ast::JoinOperator::LeftAnti(constraint)
            | sqlparser::ast::JoinOperator::RightAnti(constraint) => {
                if let sqlparser::ast::JoinConstraint::On(expr) = constraint {
                    rewrite_pg_overlap_expr(expr);
                }
            }
            _ => {}
        }
    }
}

fn rewrite_pg_overlap_table_factor(factor: &mut sqlparser::ast::TableFactor) {
    match factor {
        sqlparser::ast::TableFactor::Derived { subquery, .. } => {
            rewrite_pg_overlap_query(subquery);
        }
        sqlparser::ast::TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            rewrite_pg_overlap_table_with_joins(table_with_joins);
        }
        _ => {}
    }
}

fn rewrite_pg_overlap_select_item(item: &mut sqlparser::ast::SelectItem) {
    match item {
        sqlparser::ast::SelectItem::ExprWithAlias { expr, .. }
        | sqlparser::ast::SelectItem::UnnamedExpr(expr) => {
            rewrite_pg_overlap_expr(expr);
        }
        _ => {}
    }
}

fn rewrite_pg_overlap_expr(expr: &mut sqlparser::ast::Expr) {
    match expr {
        sqlparser::ast::Expr::BinaryOp { left, op, right }
            if *op == sqlparser::ast::BinaryOperator::PGOverlap =>
        {
            let left = std::mem::replace(
                left.as_mut(),
                sqlparser::ast::Expr::Value(sqlparser::ast::Value::Null.with_empty_span()),
            );
            let right = std::mem::replace(
                right.as_mut(),
                sqlparser::ast::Expr::Value(sqlparser::ast::Value::Null.with_empty_span()),
            );
            *expr = sqlparser::ast::Expr::Function(sqlparser::ast::Function {
                name: sqlparser::ast::ObjectName(vec![sqlparser::ast::ObjectNamePart::Identifier(
                    sqlparser::ast::Ident::new("st_overlaps_bbox"),
                )]),
                uses_odbc_syntax: false,
                parameters: sqlparser::ast::FunctionArguments::None,
                args: sqlparser::ast::FunctionArguments::List(
                    sqlparser::ast::FunctionArgumentList {
                        duplicate_treatment: None,
                        args: vec![
                            sqlparser::ast::FunctionArg::Unnamed(
                                sqlparser::ast::FunctionArgExpr::Expr(left),
                            ),
                            sqlparser::ast::FunctionArg::Unnamed(
                                sqlparser::ast::FunctionArgExpr::Expr(right),
                            ),
                        ],
                        clauses: vec![],
                    },
                ),
                filter: None,
                null_treatment: None,
                over: None,
                within_group: vec![],
            });
        }
        sqlparser::ast::Expr::BinaryOp { left, right, .. } => {
            rewrite_pg_overlap_expr(left);
            rewrite_pg_overlap_expr(right);
        }
        sqlparser::ast::Expr::Nested(inner) => {
            rewrite_pg_overlap_expr(inner);
        }
        sqlparser::ast::Expr::Function(func) => {
            rewrite_pg_overlap_function_args(func);
            rewrite_postgis_st_asmvt_record_arg(func);
        }
        sqlparser::ast::Expr::IsTrue(inner)
        | sqlparser::ast::Expr::IsNotTrue(inner)
        | sqlparser::ast::Expr::IsFalse(inner)
        | sqlparser::ast::Expr::IsNotFalse(inner) => {
            rewrite_pg_overlap_expr(inner);
        }
        _ => {}
    }
}

fn rewrite_pg_overlap_function_args(func: &mut sqlparser::ast::Function) {
    if let sqlparser::ast::FunctionArguments::List(list) = &mut func.args {
        for arg in list.args.iter_mut() {
            if let sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(
                expr,
            )) = arg
            {
                rewrite_pg_overlap_expr(expr);
            }
        }
    }
}

/// Minimal Martin/PostGIS compatibility: PostgreSQL lets `ST_AsMVT(tile, ...,
/// 'geom')` pass a whole subquery row (`tile`) to the aggregate. DataFusion has
/// no record pseudo-type, but Martin's generated query always includes the MVT
/// geometry column name as the 4th argument. Rewrite the record form to the
/// geometry-only form that QuackGIS implements: `ST_AsMVT(tile.geom)`.
fn rewrite_postgis_st_asmvt_record_arg(func: &mut sqlparser::ast::Function) {
    let Some(name) = func.name.0.last() else {
        return;
    };
    if !name.to_string().eq_ignore_ascii_case("st_asmvt") {
        return;
    }

    let sqlparser::ast::FunctionArguments::List(list) = &mut func.args else {
        return;
    };
    if list.args.len() < 4 {
        return;
    }

    let alias = match &list.args[0] {
        sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(
            sqlparser::ast::Expr::Identifier(ident),
        )) => ident.value.clone(),
        _ => return,
    };
    let geom_col = match &list.args[3] {
        sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(
            sqlparser::ast::Expr::Value(value),
        )) => match &value.value {
            sqlparser::ast::Value::SingleQuotedString(s)
            | sqlparser::ast::Value::DoubleQuotedString(s) => s.clone(),
            _ => return,
        },
        _ => return,
    };

    list.args = vec![sqlparser::ast::FunctionArg::Unnamed(
        sqlparser::ast::FunctionArgExpr::Expr(sqlparser::ast::Expr::CompoundIdentifier(vec![
            sqlparser::ast::Ident::new(alias),
            sqlparser::ast::Ident::new(geom_col),
        ])),
    )];
}
