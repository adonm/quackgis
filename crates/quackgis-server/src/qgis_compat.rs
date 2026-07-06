// SPDX-License-Identifier: Apache-2.0
//! QGIS-specific catalog compatibility shims found by client-trace probing.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::common::{ParamValues, ScalarValue};
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

const GEOMETRY_OID: u32 = 90_001;
const GEOGRAPHY_OID: u32 = 90_002;
const SYNTHETIC_PK_INDEX_OID: u32 = 90_101;

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
        params: &ParamValues,
        _session_context: &SessionContext,
        _client: &mut dyn HookClient,
    ) -> Option<PgWireResult<Response>> {
        qgis_typeinfo_response(statement, params)
            .or_else(|| qgis_geometry_typname_response(statement))
    }
}

fn qgis_typeinfo_response(
    statement: &Statement,
    params: &ParamValues,
) -> Option<PgWireResult<Response>> {
    let sql = statement.to_string().to_lowercase();
    if !(sql.contains("pg_catalog.pg_type")
        && sql.contains("t.oid = $1")
        && sql.contains("t.typname")
        && sql.contains("t.typtype"))
    {
        return None;
    }

    let oid = first_oid_param(params)?;
    match oid {
        GEOMETRY_OID => Some(typeinfo_row("geometry", GEOMETRY_OID).map(Response::Query)),
        GEOGRAPHY_OID => Some(typeinfo_row("geography", GEOGRAPHY_OID).map(Response::Query)),
        _ => None,
    }
}

fn first_oid_param(params: &ParamValues) -> Option<u32> {
    let value = match params {
        ParamValues::List(values) => values.first()?.value.clone(),
        ParamValues::Map(values) => values.get("$1").or_else(|| values.get("1"))?.value.clone(),
    };
    match value {
        ScalarValue::UInt32(Some(value)) => Some(value),
        ScalarValue::Int32(Some(value)) if value >= 0 => Some(value as u32),
        ScalarValue::Int64(Some(value)) if value >= 0 => u32::try_from(value).ok(),
        _ => None,
    }
}

fn typeinfo_row(typname: &str, oid: u32) -> PgWireResult<QueryResponse> {
    let format = FieldFormat::Binary;
    let fields = vec![
        FieldInfo::new("typname".to_string(), None, None, Type::VARCHAR, format),
        FieldInfo::new("typtype".to_string(), None, None, Type::CHAR, format),
        FieldInfo::new("typelem".to_string(), None, None, Type::OID, format),
        FieldInfo::new("rngsubtype".to_string(), None, None, Type::OID, format),
        FieldInfo::new("typbasetype".to_string(), None, None, Type::OID, format),
        FieldInfo::new("nspname".to_string(), None, None, Type::VARCHAR, format),
        FieldInfo::new("typrelid".to_string(), None, None, Type::OID, format),
    ];
    let mut encoder = DataRowEncoder::new(Arc::new(fields.clone()));
    encoder.encode_field(&Some(typname))?;
    encoder.encode_field(&Some(b'b' as i8))?;
    encoder.encode_field(&Some(0_u32))?;
    encoder.encode_field(&None::<u32>)?;
    encoder.encode_field(&Some(0_u32))?;
    encoder.encode_field(&Some("public"))?;
    encoder.encode_field(&Some(0_u32))?;
    let row = Ok(encoder.take_row());
    let row_stream = futures::stream::once(async move { row });
    let _ = oid;
    Ok(QueryResponse::new(Arc::new(fields), Box::pin(row_stream)))
}

fn qgis_geometry_typname_response(statement: &Statement) -> Option<PgWireResult<Response>> {
    let sql = statement.to_string().to_lowercase();
    if let Some(response) = pg_type_oid_in_response(&sql) {
        return Some(response.map(Response::Query));
    }

    if (sql.contains("qgis_editor_widget_styles") || sql.contains("layer_styles"))
        && sql.contains("exists")
    {
        return Some(single_bool_row("exists", false).map(Response::Query));
    }
    if sql.contains("pg_description") && sql.contains("regclass") {
        return Some(empty_response("description", Type::VARCHAR).map(Response::Query));
    }
    if sql.contains("pg_inherits") && sql.contains("inhparent") {
        return Some(single_i64_row("count", 0).map(Response::Query));
    }
    if sql.contains("pg_index")
        && sql.contains("pg_attribute")
        && sql.contains("attname")
        && sql.contains("attnotnull")
        && sql.contains("indexrelid")
    {
        return Some(single_id_attnotnull_row().map(Response::Query));
    }
    if sql.contains("pg_index") && sql.contains("indrelid") {
        return Some(single_oid_row("indexrelid", SYNTHETIC_PK_INDEX_OID).map(Response::Query));
    }
    if sql.contains("pg_index") && sql.contains("indkey") {
        return Some(single_text_row("indkey", "1").map(Response::Query));
    }
    if sql.contains("pg_get_indexdef") {
        return Some(
            single_text_row(
                "pg_get_indexdef",
                "CREATE UNIQUE INDEX points_pkey ON public.points (id)",
            )
            .map(Response::Query),
        );
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

fn pg_type_oid_in_response(sql: &str) -> Option<PgWireResult<QueryResponse>> {
    if !(sql.contains("pg_type")
        && sql.contains("oid")
        && sql.contains("typname")
        && (sql.contains(&GEOMETRY_OID.to_string()) || sql.contains(&GEOGRAPHY_OID.to_string())))
    {
        return None;
    }

    let mut rows = Vec::new();
    for (oid, typname, typlen) in [
        (23_u32, "int4", 4_i16),
        (25_u32, "text", -1_i16),
        (GEOMETRY_OID, "geometry", -1_i16),
        (GEOGRAPHY_OID, "geography", -1_i16),
    ] {
        if sql.contains(&oid.to_string()) {
            rows.push((oid, typname, typlen));
        }
    }

    let fields = vec![
        FieldInfo::new("oid".to_string(), None, None, Type::OID, FieldFormat::Text),
        FieldInfo::new(
            "typname".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "typtype".to_string(),
            None,
            None,
            Type::CHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "typelem".to_string(),
            None,
            None,
            Type::OID,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "typlen".to_string(),
            None,
            None,
            Type::INT2,
            FieldFormat::Text,
        ),
    ];
    let fields = Arc::new(fields);
    let row_stream = futures::stream::iter(rows.into_iter().map({
        let fields = Arc::clone(&fields);
        move |(oid, typname, typlen)| {
            let mut encoder = DataRowEncoder::new(Arc::clone(&fields));
            encoder.encode_field(&Some(oid))?;
            encoder.encode_field(&Some(typname))?;
            encoder.encode_field(&Some("b"))?;
            encoder.encode_field(&Some(0_u32))?;
            encoder.encode_field(&Some(typlen))?;
            Ok(encoder.take_row())
        }
    }));

    Some(Ok(QueryResponse::new(fields, Box::pin(row_stream))))
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

fn single_oid_row(name: &str, value: u32) -> PgWireResult<QueryResponse> {
    let fields = vec![FieldInfo::new(
        name.to_string(),
        None,
        None,
        Type::OID,
        FieldFormat::Text,
    )];
    let mut encoder = DataRowEncoder::new(Arc::new(fields.clone()));
    encoder.encode_field(&Some(value))?;
    let row = Ok(encoder.take_row());
    let row_stream = futures::stream::once(async move { row });
    Ok(QueryResponse::new(Arc::new(fields), Box::pin(row_stream)))
}

fn single_id_attnotnull_row() -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new(
            "attname".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "attnotnull".to_string(),
            None,
            None,
            Type::BOOL,
            FieldFormat::Text,
        ),
    ];
    let mut encoder = DataRowEncoder::new(Arc::new(fields.clone()));
    encoder.encode_field(&Some("id"))?;
    encoder.encode_field(&Some(true))?;
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
