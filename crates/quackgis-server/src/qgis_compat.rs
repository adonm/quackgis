// SPDX-License-Identifier: Apache-2.0
//! QGIS-specific catalog compatibility shims found by client-trace probing.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::common::ParamValues;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion::sql::sqlparser::ast::Statement;
use datafusion_postgres::hooks::{HookClient, QueryHook};
use datafusion_postgres::pgwire::api::ClientInfo;
use datafusion_postgres::pgwire::api::Type;
use datafusion_postgres::pgwire::api::results::{
    DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response,
};
use datafusion_postgres::pgwire::error::PgWireResult;

#[derive(Debug)]
pub struct QgisCatalogHook;

#[async_trait]
impl QueryHook for QgisCatalogHook {
    async fn handle_simple_query(
        &self,
        statement: &Statement,
        _session_context: &SessionContext,
        _client: &mut dyn HookClient,
    ) -> Option<PgWireResult<Response>> {
        qgis_geometry_typname_response(statement)
    }

    async fn handle_extended_parse_query(
        &self,
        _statement: &Statement,
        _session_context: &SessionContext,
        _client: &(dyn ClientInfo + Send + Sync),
    ) -> Option<PgWireResult<LogicalPlan>> {
        None
    }

    async fn handle_extended_query(
        &self,
        statement: &Statement,
        _logical_plan: &LogicalPlan,
        _params: &ParamValues,
        _session_context: &SessionContext,
        _client: &mut dyn HookClient,
    ) -> Option<PgWireResult<Response>> {
        qgis_geometry_typname_response(statement)
    }
}

fn qgis_geometry_typname_response(statement: &Statement) -> Option<PgWireResult<Response>> {
    let sql = statement.to_string().to_lowercase();
    if sql.contains("qgis_editor_widget_styles") && sql.contains("exists") {
        return Some(single_bool_row("exists", false).map(Response::Query));
    }
    if sql.contains("pg_description") && sql.contains("regclass") {
        return Some(empty_response("description", Type::VARCHAR).map(Response::Query));
    }
    if sql.contains("pg_inherits") && sql.contains("inhparent") {
        return Some(single_i64_row("count", 0).map(Response::Query));
    }
    if sql.contains("pg_index") && sql.contains("indrelid") {
        return Some(empty_response("indexrelid", Type::INT4).map(Response::Query));
    }
    if sql.contains("relkind") && sql.contains("pg_class") && sql.contains("regclass") {
        return Some(single_text_row("relkind", "r").map(Response::Query));
    }
    if sql.contains("pg_attribute") && sql.contains("regclass") && sql.contains("attidentity") {
        return Some(empty_response("attidentity", Type::VARCHAR).map(Response::Query));
    }
    if sql.contains("pg_attribute") && sql.contains("regclass") && sql.contains("attname") {
        return Some(empty_response("attname", Type::VARCHAR).map(Response::Query));
    }

    if !(sql.contains("pg_attribute")
        && sql.contains("pg_type")
        && sql.contains("t.typname")
        && sql.contains("a.attname = 'geom'"))
    {
        return None;
    }

    Some(single_text_row("typname", "geometry").map(Response::Query))
}

fn empty_response(name: &str, ty: Type) -> PgWireResult<QueryResponse> {
    let fields = vec![FieldInfo::new(
        name.to_string(),
        None,
        None,
        ty,
        FieldFormat::Text,
    )];
    let row_stream = futures::stream::empty();
    Ok(QueryResponse::new(Arc::new(fields), Box::pin(row_stream)))
}

fn single_bool_row(name: &str, value: bool) -> PgWireResult<QueryResponse> {
    let fields = vec![FieldInfo::new(
        name.to_string(),
        None,
        None,
        Type::BOOL,
        FieldFormat::Text,
    )];
    let mut encoder = DataRowEncoder::new(Arc::new(fields.clone()));
    encoder.encode_field(&Some(value))?;
    let row = Ok(encoder.take_row());
    let row_stream = futures::stream::once(async move { row });
    Ok(QueryResponse::new(Arc::new(fields), Box::pin(row_stream)))
}

fn single_i64_row(name: &str, value: i64) -> PgWireResult<QueryResponse> {
    let fields = vec![FieldInfo::new(
        name.to_string(),
        None,
        None,
        Type::INT8,
        FieldFormat::Text,
    )];
    let mut encoder = DataRowEncoder::new(Arc::new(fields.clone()));
    encoder.encode_field(&Some(value))?;
    let row = Ok(encoder.take_row());
    let row_stream = futures::stream::once(async move { row });
    Ok(QueryResponse::new(Arc::new(fields), Box::pin(row_stream)))
}

fn single_text_row(name: &str, value: &str) -> PgWireResult<QueryResponse> {
    let fields = vec![FieldInfo::new(
        name.to_string(),
        None,
        None,
        Type::VARCHAR,
        FieldFormat::Text,
    )];
    let mut encoder = DataRowEncoder::new(Arc::new(fields.clone()));
    encoder.encode_field(&Some(value))?;
    let row = Ok(encoder.take_row());
    let row_stream = futures::stream::once(async move { row });
    Ok(QueryResponse::new(Arc::new(fields), Box::pin(row_stream)))
}
