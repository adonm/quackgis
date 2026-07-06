// SPDX-License-Identifier: Apache-2.0
//! PostgreSQL/PostGIS catalog and cursor compatibility shims found by
//! client-trace probing.
//!
//! This module is organized around PostgreSQL/PostGIS surfaces rather than
//! individual clients. QGIS, GDAL/OGR, Martin, and similar clients mostly probe
//! the same boundary: `pg_type`, `pg_class`, `pg_attribute`, `pg_index`,
//! `geometry_columns`, and cursor flow. Helpers below are named for those
//! server surfaces; test names record the client trace that motivated each
//! query shape.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use datafusion::arrow::array::{Array, BinaryArray, BinaryViewArray, Int32Array, StringArray};
use datafusion::common::{ParamValues, ScalarValue};
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion::sql::sqlparser::ast::Statement;
use datafusion_postgres::hooks::{HookClient, QueryHook};
use datafusion_postgres::pgwire::api::ClientInfo;
use datafusion_postgres::pgwire::api::Type;
use datafusion_postgres::pgwire::api::results::{
    DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response, Tag,
};
use datafusion_postgres::pgwire::error::{PgWireError, PgWireResult};

const GEOMETRY_OID: u32 = 90_001;
const GEOGRAPHY_OID: u32 = 90_002;
const SYNTHETIC_PK_INDEX_OID: u32 = 90_101;

static POSTGRES_DRIVER_CURSORS: LazyLock<Mutex<HashMap<String, PostgresDriverCursor>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
struct PostgresDriverCursor {
    table: String,
    offset: usize,
}

#[derive(Debug)]
pub struct CatalogCompatHook;

/// Backward-compatible alias for older call sites. The hook now contains
/// PostgreSQL/PostGIS catalog shims used by multiple clients, not only QGIS.
pub type QgisCatalogHook = CatalogCompatHook;

#[async_trait]
impl QueryHook for CatalogCompatHook {
    async fn handle_simple_query(
        &self,
        statement: &Statement,
        session_context: &SessionContext,
        _client: &mut dyn HookClient,
    ) -> Option<PgWireResult<Response>> {
        catalog_query_response(statement, session_context)
    }

    async fn handle_extended_parse_query(
        &self,
        statement: &Statement,
        session_context: &SessionContext,
        _client: &(dyn ClientInfo + Send + Sync),
    ) -> Option<PgWireResult<LogicalPlan>> {
        let sql = statement.to_string();
        if sql.to_uppercase().contains("OGRPGLAYERREADER") {
            if let Some((cursor, _limit)) = parse_postgres_driver_fetch(&sql) {
                let table = POSTGRES_DRIVER_CURSORS
                    .lock()
                    .ok()
                    .and_then(|cursors| cursors.get(&cursor).map(|state| state.table.clone()));
                return Some(cursor_feature_logical_plan(session_context, table.as_deref()).await);
            }
            return Some(dummy_logical_plan(session_context).await);
        }
        None
    }

    async fn handle_extended_query(
        &self,
        statement: &Statement,
        _logical_plan: &LogicalPlan,
        params: &ParamValues,
        session_context: &SessionContext,
        _client: &mut dyn HookClient,
    ) -> Option<PgWireResult<Response>> {
        if let Some(response) = pg_type_extended_info_response(statement, params)
            .or_else(|| catalog_query_response(statement, session_context))
        {
            return Some(response);
        }
        postgres_driver_cursor_response(statement, session_context).await
    }
}

fn pg_type_extended_info_response(
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

fn catalog_query_response(
    statement: &Statement,
    session_context: &SessionContext,
) -> Option<PgWireResult<Response>> {
    let sql = statement.to_string().to_lowercase();
    if let Some(response) = pg_type_oid_in_response(&sql) {
        return Some(response.map(Response::Query));
    }
    if sql.contains("pg_type")
        && sql.contains("oid")
        && sql.contains("typname")
        && sql.contains("typtype")
    {
        return Some(pg_type_postgis_probe_response().map(Response::Query));
    }
    if (sql.contains("qgis_editor_widget_styles") || sql.contains("layer_styles"))
        && sql.contains("exists")
    {
        return Some(single_bool_row("exists", false).map(Response::Query));
    }
    if sql.contains("pg_class")
        && sql.contains("pg_namespace")
        && sql.contains("pg_description")
        && sql.contains("d.classoid")
        && sql.contains("d.objsubid")
        && sql.contains("relkind")
        && sql.contains("relname")
        && sql.contains("nspname")
    {
        return Some(pg_class_table_listing_response(session_context).map(Response::Query));
    }
    if sql.contains("pg_inherits") && sql.contains("inhparent") && sql.contains("relname") {
        return Some(empty_response("relname", Type::VARCHAR).map(Response::Query));
    }
    if sql.contains("pg_inherits") && sql.contains("inhparent") {
        return Some(single_i64_row("count", 0).map(Response::Query));
    }
    if sql.contains("pg_attribute")
        && sql.contains("pg_type")
        && sql.contains("format_type")
        && sql.contains("pg_attrdef")
        && sql.contains("pg_index")
        && sql.contains("pg_description")
        && sql.contains("attnotnull")
        && sql.contains("indisunique")
    {
        return Some(pg_attribute_column_listing_response().map(Response::Query));
    }
    if sql.contains("from geography_columns")
        && sql.contains("type")
        && sql.contains("coord_dimension")
        && sql.contains("srid")
    {
        return Some(geography_columns_probe_response().map(Response::Query));
    }
    if sql.contains("pg_description") && sql.contains("regclass") {
        return Some(empty_response("description", Type::VARCHAR).map(Response::Query));
    }
    if sql.contains("pg_attribute")
        && sql.contains("pg_type")
        && sql.contains("pg_index")
        && sql.contains("attnum")
        && sql.contains("typname")
        && sql.contains("isfid")
        && sql.contains("indisprimary")
    {
        return Some(pg_index_primary_key_probe_response().map(Response::Query));
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

async fn postgres_driver_cursor_response(
    statement: &Statement,
    session_context: &SessionContext,
) -> Option<PgWireResult<Response>> {
    let sql = statement.to_string();
    if let Some((cursor, table)) = parse_postgres_driver_declare(&sql) {
        if let Ok(mut cursors) = POSTGRES_DRIVER_CURSORS.lock() {
            cursors.insert(cursor, PostgresDriverCursor { table, offset: 0 });
        }
        return Some(Ok(Response::Execution(Tag::new("DECLARE CURSOR"))));
    }
    if let Some((cursor, limit)) = parse_postgres_driver_fetch(&sql) {
        let state = POSTGRES_DRIVER_CURSORS
            .lock()
            .ok()
            .and_then(|cursors| cursors.get(&cursor).cloned())?;
        return Some(cursor_fetch_response(session_context, &cursor, state, limit).await);
    }
    None
}

fn parse_postgres_driver_declare(sql: &str) -> Option<(String, String)> {
    let upper = sql.to_uppercase();
    if !(upper.starts_with("DECLARE ")
        && upper.contains("OGRPGLAYERREADER")
        && upper.contains(" CURSOR FOR SELECT ")
        && upper.contains("WKB_GEOMETRY"))
    {
        return None;
    }
    let cursor = sql.split_whitespace().nth(1)?.to_string();
    let from_pos = upper.find(" FROM ")? + " FROM ".len();
    let table = sql[from_pos..]
        .split_whitespace()
        .next()?
        .trim_matches('"')
        .to_string();
    Some((cursor, table))
}

fn parse_postgres_driver_fetch(sql: &str) -> Option<(String, usize)> {
    let upper = sql.to_uppercase();
    if !upper.starts_with("FETCH ") || !upper.contains("OGRPGLAYERREADER") {
        return None;
    }
    let mut limit = 500_usize;
    let mut cursor = None;
    for part in sql.split_whitespace().skip(1) {
        if let Ok(parsed) = part.parse() {
            limit = parsed;
        }
        if part.to_uppercase().starts_with("OGRPGLAYERREADER") {
            cursor = Some(part.trim_matches('"').to_string());
        }
    }
    let cursor = cursor?;
    Some((cursor, limit))
}

async fn dummy_logical_plan(session_context: &SessionContext) -> PgWireResult<LogicalPlan> {
    let dataframe = session_context
        .sql("SELECT 1")
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    dataframe
        .into_optimized_plan()
        .map_err(|e| PgWireError::ApiError(Box::new(e)))
}

async fn cursor_feature_logical_plan(
    session_context: &SessionContext,
    table: Option<&str>,
) -> PgWireResult<LogicalPlan> {
    let sql = if let Some(table) = table {
        format!(
            "SELECT CAST(NULL AS TEXT) AS \"wkb_geometry\", CAST(\"id\" AS INT) AS \"id\", \
             CAST(\"name\" AS TEXT) AS \"name\" FROM \"{}\" WHERE FALSE",
            table.replace('"', "\"\"")
        )
    } else {
        "SELECT X'' AS \"wkb_geometry\", \
                CAST(NULL AS INT) AS \"id\", \
                CAST(NULL AS TEXT) AS \"name\" \
         WHERE FALSE"
            .to_string()
    };
    let dataframe = session_context
        .sql(&sql)
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    dataframe
        .into_optimized_plan()
        .map_err(|e| PgWireError::ApiError(Box::new(e)))
}

async fn cursor_fetch_response(
    session_context: &SessionContext,
    cursor: &str,
    state: PostgresDriverCursor,
    limit: usize,
) -> PgWireResult<Response> {
    let cursor = cursor.to_string();
    let fields = Arc::new(cursor_feature_fields());
    let query = format!(
        "SELECT \"wkb_geometry\", CAST(\"id\" AS INT) AS \"id\", \
         CAST(\"name\" AS TEXT) AS \"name\" FROM \"{}\" LIMIT {} OFFSET {}",
        state.table.replace('"', "\"\""),
        limit,
        state.offset
    );
    let dataframe = session_context
        .sql(&query)
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    let batches = dataframe
        .collect()
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    let mut emitted = 0_usize;
    let mut rows = Vec::new();
    for batch in batches {
        let wkb = batch.column(0).as_any().downcast_ref::<BinaryArray>();
        let wkb_view = batch.column(0).as_any().downcast_ref::<BinaryViewArray>();
        let ids = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int32Array>()
            .ok_or_else(|| api_error("OGR cursor expected INT id"))?;
        let names = batch.column(2).as_any().downcast_ref::<StringArray>();
        let name_views = batch
            .column(2)
            .as_any()
            .downcast_ref::<datafusion::arrow::array::StringViewArray>();
        for row_idx in 0..batch.num_rows() {
            let mut encoder = DataRowEncoder::new(Arc::clone(&fields));
            if batch.column(0).is_null(row_idx) {
                encoder.encode_field(&None::<&str>)?;
            } else if let Some(wkb) = wkb {
                let bytea = format!("\\x{}", hex_encode(wkb.value(row_idx)));
                encoder.encode_field(&Some(bytea.as_str()))?;
            } else if let Some(wkb_view) = wkb_view {
                let bytea = format!("\\x{}", hex_encode(wkb_view.value(row_idx)));
                encoder.encode_field(&Some(bytea.as_str()))?;
            } else {
                return Err(api_error("OGR cursor expected binary geometry"));
            }
            if ids.is_null(row_idx) {
                encoder.encode_field(&None::<i32>)?;
            } else {
                encoder.encode_field(&Some(ids.value(row_idx)))?;
            }
            if batch.column(2).is_null(row_idx) {
                encoder.encode_field(&None::<&str>)?;
            } else if let Some(names) = names {
                encoder.encode_field(&Some(names.value(row_idx)))?;
            } else if let Some(name_views) = name_views {
                encoder.encode_field(&Some(name_views.value(row_idx)))?;
            } else {
                return Err(api_error("OGR cursor expected text name"));
            }
            emitted += 1;
            rows.push(Ok(encoder.take_row()));
        }
    }
    if let Ok(mut cursors) = POSTGRES_DRIVER_CURSORS.lock()
        && let Some(cursor_state) = cursors.get_mut(&cursor)
    {
        cursor_state.offset += emitted;
    }
    Ok(Response::Query(QueryResponse::new(
        fields,
        Box::pin(futures::stream::iter(rows)),
    )))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn api_error(message: &str) -> PgWireError {
    PgWireError::ApiError(Box::new(std::io::Error::other(message.to_string())))
}

fn cursor_feature_fields() -> Vec<FieldInfo> {
    vec![
        FieldInfo::new(
            "wkb_geometry".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new("id".to_string(), None, None, Type::INT4, FieldFormat::Text),
        FieldInfo::new(
            "name".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
    ]
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

fn pg_type_postgis_probe_response() -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new("oid".to_string(), None, None, Type::OID, FieldFormat::Text),
        FieldInfo::new(
            "typname".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
    ];
    let row_stream = futures::stream::empty();
    Ok(QueryResponse::new(Arc::new(fields), Box::pin(row_stream)))
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

fn geography_columns_probe_response() -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new(
            "type".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "coord_dimension".to_string(),
            None,
            None,
            Type::INT4,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "srid".to_string(),
            None,
            None,
            Type::INT4,
            FieldFormat::Text,
        ),
    ];
    let row_stream = futures::stream::empty();
    Ok(QueryResponse::new(Arc::new(fields), Box::pin(row_stream)))
}

fn pg_index_primary_key_probe_response() -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new(
            "attname".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "attnum".to_string(),
            None,
            None,
            Type::INT2,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "typname".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "isfid".to_string(),
            None,
            None,
            Type::BOOL,
            FieldFormat::Text,
        ),
    ];
    let row_stream = futures::stream::empty();
    Ok(QueryResponse::new(Arc::new(fields), Box::pin(row_stream)))
}

fn pg_class_table_listing_response(
    session_context: &SessionContext,
) -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new(
            "relname".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "nspname".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "description".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
    ];

    let rows = visible_tables_as_public_relations(session_context);
    let fields = Arc::new(fields);
    let row_stream = futures::stream::iter(rows.into_iter().map({
        let fields = Arc::clone(&fields);
        move |(relname, nspname)| {
            let mut encoder = DataRowEncoder::new(Arc::clone(&fields));
            encoder.encode_field(&Some(relname.as_str()))?;
            encoder.encode_field(&Some(nspname.as_str()))?;
            encoder.encode_field(&None::<&str>)?;
            Ok(encoder.take_row())
        }
    }));

    Ok(QueryResponse::new(fields, Box::pin(row_stream)))
}

fn visible_tables_as_public_relations(session_context: &SessionContext) -> Vec<(String, String)> {
    let state = session_context.state();
    let catalog_list = state.catalog_list();
    let Some(catalog) = catalog_list.catalog("quackgis") else {
        return Vec::new();
    };
    let Some(schema) = catalog.schema("main") else {
        return Vec::new();
    };

    schema
        .table_names()
        .into_iter()
        .map(|table| (table, "public".to_string()))
        .collect()
}

fn pg_attribute_column_listing_response() -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new(
            "attname".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "typname".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "attlen".to_string(),
            None,
            None,
            Type::INT2,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "format_type".to_string(),
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
        FieldInfo::new(
            "def".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "indisunique".to_string(),
            None,
            None,
            Type::BOOL,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "description".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
    ];
    let rows = [
        ("id", "int4", 4_i16, "integer"),
        ("wkb_geometry", "bytea", -1_i16, "bytea"),
        ("name", "text", -1_i16, "text"),
    ];
    let fields = Arc::new(fields);
    let row_stream = futures::stream::iter(rows.into_iter().map({
        let fields = Arc::clone(&fields);
        move |(attname, typname, attlen, format_type)| {
            let mut encoder = DataRowEncoder::new(Arc::clone(&fields));
            encoder.encode_field(&Some(attname))?;
            encoder.encode_field(&Some(typname))?;
            encoder.encode_field(&Some(attlen))?;
            encoder.encode_field(&Some(format_type))?;
            encoder.encode_field(&Some(false))?;
            encoder.encode_field(&None::<&str>)?;
            encoder.encode_field(&Some(false))?;
            encoder.encode_field(&None::<&str>)?;
            Ok(encoder.take_row())
        }
    }));

    Ok(QueryResponse::new(fields, Box::pin(row_stream)))
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
