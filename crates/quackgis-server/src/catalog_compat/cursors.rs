// SPDX-License-Identifier: Apache-2.0
//! OGR PostgreSQL-driver cursor compatibility.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use datafusion::arrow::array::{
    Array, BinaryArray, BinaryViewArray, Int16Array, Int32Array, Int64Array, LargeBinaryArray,
    LargeStringArray, StringArray, StringViewArray, UInt16Array, UInt32Array, UInt64Array,
};
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion::sql::sqlparser::ast::Statement;
use datafusion_postgres::pgwire::api::Type;
use datafusion_postgres::pgwire::api::results::{
    DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response, Tag,
};
use datafusion_postgres::pgwire::error::{PgWireError, PgWireResult};

use super::SYNTHETIC_ROWID_COLUMN;
use super::sql_parse::{
    escape_identifier, select_item_output_name, select_items, strip_trailing_semicolon,
};

static POSTGRES_DRIVER_CURSORS: LazyLock<Mutex<HashMap<String, PostgresDriverCursor>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
struct PostgresDriverCursor {
    select_sql: String,
    columns: Vec<CursorColumn>,
    offset: usize,
}

#[derive(Debug, Clone)]
struct CursorColumn {
    name: String,
}

#[derive(Debug, Clone, Copy)]
enum CursorFieldKind {
    BinaryHexText,
    Text,
    Int32,
    Int64,
}

#[derive(Debug, Clone)]
struct CursorFieldSpec {
    name: String,
    kind: CursorFieldKind,
}

pub(super) async fn postgres_driver_fetch_logical_plan(
    sql: &str,
    session_context: &SessionContext,
) -> Option<PgWireResult<LogicalPlan>> {
    let (cursor, _limit) = parse_postgres_driver_fetch(sql)?;
    let state = POSTGRES_DRIVER_CURSORS
        .lock()
        .ok()
        .and_then(|cursors| cursors.get(&cursor).cloned());
    Some(cursor_feature_logical_plan(session_context, state.as_ref()).await)
}

pub(super) async fn postgres_driver_cursor_response(
    statement: &Statement,
    session_context: &SessionContext,
) -> Option<PgWireResult<Response>> {
    let sql = statement.to_string();
    if let Some((cursor, state)) = parse_postgres_driver_declare(&sql) {
        if let Ok(mut cursors) = POSTGRES_DRIVER_CURSORS.lock() {
            cursors.insert(cursor, state);
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

fn parse_postgres_driver_declare(sql: &str) -> Option<(String, PostgresDriverCursor)> {
    let upper = sql.to_uppercase();
    if !(upper.starts_with("DECLARE ")
        && upper.contains("OGRPGLAYERREADER")
        && upper.contains(" CURSOR FOR SELECT ")
        && upper.contains("WKB_GEOMETRY"))
    {
        return None;
    }
    let cursor = sql.split_whitespace().nth(1)?.trim_matches('"').to_string();
    let select_pos = upper.find(" CURSOR FOR ")? + " CURSOR FOR ".len();
    let select_sql = strip_trailing_semicolon(&sql[select_pos..]).to_string();
    let columns = cursor_columns_from_select(&select_sql);
    Some((
        cursor,
        PostgresDriverCursor {
            select_sql,
            columns,
            offset: 0,
        },
    ))
}

fn cursor_columns_from_select(select_sql: &str) -> Vec<CursorColumn> {
    let columns = select_items(select_sql)
        .into_iter()
        .filter_map(|expression| {
            let name = select_item_output_name(&expression)?;
            (name != "*").then_some(CursorColumn { name })
        })
        .collect::<Vec<_>>();

    if columns.is_empty() {
        return default_cursor_columns();
    }
    columns
}

fn default_cursor_columns() -> Vec<CursorColumn> {
    ["wkb_geometry", "id", "name"]
        .into_iter()
        .map(|name| CursorColumn {
            name: name.to_string(),
        })
        .collect()
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

pub(super) async fn dummy_logical_plan(
    session_context: &SessionContext,
) -> PgWireResult<LogicalPlan> {
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
    state: Option<&PostgresDriverCursor>,
) -> PgWireResult<LogicalPlan> {
    let columns = state
        .map(|state| state.columns.clone())
        .unwrap_or_else(default_cursor_columns);
    let specs = cursor_field_specs(&columns);
    let select_list = specs
        .iter()
        .map(|spec| {
            format!(
                "CAST(NULL AS {}) AS \"{}\"",
                spec.sql_type(),
                escape_identifier(&spec.name)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {select_list} WHERE FALSE");
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
    let specs = cursor_field_specs(&state.columns);
    let fields = Arc::new(cursor_feature_fields(&specs));
    let query = format!(
        "SELECT * FROM ({}) AS \"_quackgis_ogr_cursor\" LIMIT {} OFFSET {}",
        state.select_sql, limit, state.offset
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
        if batch.num_columns() != specs.len() {
            return Err(api_error("OGR cursor returned unexpected column count"));
        }
        for row_idx in 0..batch.num_rows() {
            let mut encoder = DataRowEncoder::new(Arc::clone(&fields));
            for (col_idx, spec) in specs.iter().enumerate() {
                encode_cursor_field(&mut encoder, batch.column(col_idx).as_ref(), row_idx, spec)?;
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

fn cursor_field_specs(columns: &[CursorColumn]) -> Vec<CursorFieldSpec> {
    columns
        .iter()
        .map(|column| CursorFieldSpec {
            name: column.name.clone(),
            kind: cursor_field_kind(&column.name),
        })
        .collect()
}

fn cursor_field_kind(name: &str) -> CursorFieldKind {
    if crate::geometry_columns::is_geometry_column_name(name) {
        CursorFieldKind::BinaryHexText
    } else if name.eq_ignore_ascii_case("id") {
        CursorFieldKind::Int32
    } else if name.eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN) {
        CursorFieldKind::Int64
    } else {
        CursorFieldKind::Text
    }
}

impl CursorFieldSpec {
    fn pg_type(&self) -> Type {
        match self.kind {
            CursorFieldKind::BinaryHexText | CursorFieldKind::Text => Type::VARCHAR,
            CursorFieldKind::Int32 => Type::INT4,
            CursorFieldKind::Int64 => Type::INT8,
        }
    }

    fn sql_type(&self) -> &'static str {
        match self.kind {
            CursorFieldKind::BinaryHexText | CursorFieldKind::Text => "TEXT",
            CursorFieldKind::Int32 => "INTEGER",
            CursorFieldKind::Int64 => "BIGINT",
        }
    }
}

fn encode_cursor_field(
    encoder: &mut DataRowEncoder,
    array: &dyn Array,
    row_idx: usize,
    spec: &CursorFieldSpec,
) -> PgWireResult<()> {
    if array.is_null(row_idx) {
        return encoder.encode_field(&None::<&str>);
    }

    match spec.kind {
        CursorFieldKind::BinaryHexText => encode_binary_hex_text(encoder, array, row_idx),
        CursorFieldKind::Text => encode_textish_field(encoder, array, row_idx),
        CursorFieldKind::Int32 => encode_i32ish_field(encoder, array, row_idx),
        CursorFieldKind::Int64 => encode_i64ish_field(encoder, array, row_idx),
    }
}

fn encode_binary_hex_text(
    encoder: &mut DataRowEncoder,
    array: &dyn Array,
    row_idx: usize,
) -> PgWireResult<()> {
    if let Some(values) = array.as_any().downcast_ref::<BinaryArray>() {
        let bytea = format!("\\x{}", hex_encode(values.value(row_idx)));
        return encoder.encode_field(&Some(bytea.as_str()));
    }
    if let Some(values) = array.as_any().downcast_ref::<BinaryViewArray>() {
        let bytea = format!("\\x{}", hex_encode(values.value(row_idx)));
        return encoder.encode_field(&Some(bytea.as_str()));
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        let bytea = format!("\\x{}", hex_encode(values.value(row_idx)));
        return encoder.encode_field(&Some(bytea.as_str()));
    }
    encode_textish_field(encoder, array, row_idx)
}

fn encode_textish_field(
    encoder: &mut DataRowEncoder,
    array: &dyn Array,
    row_idx: usize,
) -> PgWireResult<()> {
    if let Some(values) = array.as_any().downcast_ref::<StringArray>() {
        return encoder.encode_field(&Some(values.value(row_idx)));
    }
    if let Some(values) = array.as_any().downcast_ref::<StringViewArray>() {
        return encoder.encode_field(&Some(values.value(row_idx)));
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeStringArray>() {
        return encoder.encode_field(&Some(values.value(row_idx)));
    }
    if let Some(values) = array.as_any().downcast_ref::<BinaryArray>() {
        let bytea = format!("\\x{}", hex_encode(values.value(row_idx)));
        return encoder.encode_field(&Some(bytea.as_str()));
    }
    if let Some(values) = array.as_any().downcast_ref::<BinaryViewArray>() {
        let bytea = format!("\\x{}", hex_encode(values.value(row_idx)));
        return encoder.encode_field(&Some(bytea.as_str()));
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        let bytea = format!("\\x{}", hex_encode(values.value(row_idx)));
        return encoder.encode_field(&Some(bytea.as_str()));
    }

    let text =
        array_value_to_string(array, row_idx).map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    encoder.encode_field(&Some(text.as_str()))
}

fn encode_i32ish_field(
    encoder: &mut DataRowEncoder,
    array: &dyn Array,
    row_idx: usize,
) -> PgWireResult<()> {
    if let Some(values) = array.as_any().downcast_ref::<Int32Array>() {
        return encoder.encode_field(&Some(values.value(row_idx)));
    }
    if let Some(values) = array.as_any().downcast_ref::<UInt16Array>() {
        return encoder.encode_field(&Some(i32::from(values.value(row_idx))));
    }
    if let Some(values) = array.as_any().downcast_ref::<Int16Array>() {
        return encoder.encode_field(&Some(i32::from(values.value(row_idx))));
    }
    encode_textish_field(encoder, array, row_idx)
}

fn encode_i64ish_field(
    encoder: &mut DataRowEncoder,
    array: &dyn Array,
    row_idx: usize,
) -> PgWireResult<()> {
    if let Some(values) = array.as_any().downcast_ref::<Int64Array>() {
        return encoder.encode_field(&Some(values.value(row_idx)));
    }
    if let Some(values) = array.as_any().downcast_ref::<UInt32Array>() {
        return encoder.encode_field(&Some(i64::from(values.value(row_idx))));
    }
    if let Some(values) = array.as_any().downcast_ref::<UInt64Array>()
        && let Ok(value) = i64::try_from(values.value(row_idx))
    {
        return encoder.encode_field(&Some(value));
    }
    if let Some(values) = array.as_any().downcast_ref::<Int32Array>() {
        return encoder.encode_field(&Some(i64::from(values.value(row_idx))));
    }
    encode_textish_field(encoder, array, row_idx)
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

fn cursor_feature_fields(specs: &[CursorFieldSpec]) -> Vec<FieldInfo> {
    specs
        .iter()
        .map(|spec| {
            FieldInfo::new(
                spec.name.clone(),
                None,
                None,
                spec.pg_type(),
                FieldFormat::Text,
            )
        })
        .collect()
}
