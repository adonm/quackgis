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
use datafusion::arrow::array::{
    Array, BinaryArray, BinaryViewArray, Int16Array, Int32Array, Int64Array, LargeBinaryArray,
    LargeStringArray, StringArray, UInt16Array, UInt32Array, UInt64Array,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::common::{DFSchema, ParamValues};
use datafusion::logical_expr::{Expr, LogicalPlan, Projection};
use datafusion::prelude::SessionContext;
use datafusion::sql::parser::Statement as DataFusionStatement;
use datafusion::sql::sqlparser::ast::Statement;
use datafusion_postgres::arrow_pg::datatypes::{encode_recordbatch, field_into_pg_type};
use datafusion_postgres::hooks::{HookClient, QueryHook};
use datafusion_postgres::pgwire::api::Type;
use datafusion_postgres::pgwire::api::results::{
    DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response, Tag,
};
use datafusion_postgres::pgwire::api::store::PortalStore;
use datafusion_postgres::pgwire::api::{ClientInfo, DEFAULT_NAME};
use datafusion_postgres::pgwire::error::{PgWireError, PgWireResult};
use futures::StreamExt;

mod params;
mod sql_parse;
mod surfaces;

use params::{first_oid_param, last_string_param, string_param};
use sql_parse::{
    count_positional_placeholders, escape_identifier, parse_first_u32, parse_single_quoted_literal,
    select_item_output_name, select_items, strip_trailing_semicolon,
};
use surfaces::{
    CatalogSurface, classify_catalog_surface, is_ogr_pg_class_oid_lookup, is_pgjdbc_columns_query,
    is_pgjdbc_primary_keys_query, is_pgjdbc_typeinfo_name_query, is_pgjdbc_typeinfo_sqltype_query,
};

const GEOMETRY_OID: u32 = 90_001;
const GEOGRAPHY_OID: u32 = 90_002;
const SYNTHETIC_PK_INDEX_OID: u32 = 90_101;
pub(crate) const SYNTHETIC_ROWID_COLUMN: &str = "_quackgis_rowid";

static POSTGRES_DRIVER_CURSORS: LazyLock<Mutex<HashMap<String, PostgresDriverCursor>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SYNTHETIC_INDEXES: LazyLock<Mutex<HashMap<u32, SyntheticIndex>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SYNTHETIC_TABLE_OIDS: LazyLock<Mutex<HashMap<u32, String>>> =
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

#[derive(Debug, Clone)]
struct SyntheticIndex {
    table: String,
    key_column: String,
    key_attnum: i16,
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
        catalog_query_response(statement, session_context).await
    }

    async fn handle_extended_parse_query(
        &self,
        statement: &Statement,
        session_context: &SessionContext,
        _client: &(dyn ClientInfo + Send + Sync),
    ) -> Option<PgWireResult<LogicalPlan>> {
        let sql = statement.to_string();
        let sql_lower = sql.to_lowercase();
        let param_count = count_positional_placeholders(&sql);
        if sql.to_uppercase().contains("OGRPGLAYERREADER") {
            if let Some((cursor, _limit)) = parse_postgres_driver_fetch(&sql) {
                let state = POSTGRES_DRIVER_CURSORS
                    .lock()
                    .ok()
                    .and_then(|cursors| cursors.get(&cursor).cloned());
                return Some(cursor_feature_logical_plan(session_context, state.as_ref()).await);
            }
            return Some(dummy_logical_plan(session_context).await);
        }
        if is_pgjdbc_typeinfo_sqltype_query(&sql_lower) {
            return Some(pgjdbc_typeinfo_sqltype_logical_plan(session_context, param_count).await);
        }
        if is_pgjdbc_typeinfo_name_query(&sql_lower) {
            return Some(pgjdbc_typeinfo_name_logical_plan(session_context, param_count).await);
        }
        if matches!(
            classify_catalog_surface(&sql_lower),
            Some(CatalogSurface::PgTypePostgisProbe)
        ) {
            return Some(pg_type_oid_typname_logical_plan(session_context, param_count).await);
        }
        if is_ogr_pg_class_oid_lookup(&sql_lower) {
            return Some(ogr_pg_class_oid_lookup_logical_plan(&sql, session_context).await);
        }
        if is_geotools_binary_geometry_query(&sql_lower) {
            return Some(
                geotools_binary_geometry_describe_plan(&sql, statement, session_context).await,
            );
        }
        if is_pgjdbc_primary_keys_query(&sql_lower) {
            return Some(
                pgjdbc_primary_keys_logical_plan(session_context, param_count.max(2)).await,
            );
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
        if is_geotools_binary_geometry_query(&statement.to_string().to_lowercase()) {
            return Some(
                geotools_st_asewkb_response(statement, params, session_context, _client).await,
            );
        }
        let sql = statement.to_string().to_lowercase();
        if is_ogr_pg_class_oid_lookup(&sql) {
            return Some(
                pg_class_oid_lookup_response(&sql, current_portal_result_format(_client))
                    .map(Response::Query),
            );
        }
        if let Some(response) =
            pgjdbc_primary_keys_extended_response(statement, params, session_context, _client).await
        {
            return Some(response);
        }
        if let Some(response) =
            pgjdbc_columns_extended_response(statement, params, session_context, _client).await
        {
            return Some(response);
        }
        if let Some(response) = pg_type_extended_info_response(statement, params, _client) {
            return Some(response);
        }
        if let Some(response) = catalog_query_response(statement, session_context).await {
            return Some(response);
        }
        postgres_driver_cursor_response(statement, session_context).await
    }
}

fn pg_type_extended_info_response(
    statement: &Statement,
    params: &ParamValues,
    client: &dyn HookClient,
) -> Option<PgWireResult<Response>> {
    let sql = statement.to_string().to_lowercase();
    let oid = first_oid_param(params)?;
    let field_format = current_portal_result_format(client);
    if is_pgjdbc_typeinfo_sqltype_query(&sql) {
        let (typname, oid) = custom_postgis_type(oid)?;
        return Some(pgjdbc_typeinfo_sqltype_row(typname, oid, field_format).map(Response::Query));
    }
    if is_pgjdbc_typeinfo_name_query(&sql) {
        let (typname, _oid) = custom_postgis_type(oid)?;
        return Some(pgjdbc_typeinfo_name_row(typname, field_format).map(Response::Query));
    }
    if !(sql.contains("pg_catalog.pg_type")
        && sql.contains("t.oid = $1")
        && sql.contains("t.typname")
        && sql.contains("t.typtype"))
    {
        return None;
    }

    let (typname, oid) = custom_postgis_type(oid)?;
    Some(typeinfo_row(typname, oid).map(Response::Query))
}

fn custom_postgis_type(oid: u32) -> Option<(&'static str, u32)> {
    match oid {
        GEOMETRY_OID => Some(("geometry", GEOMETRY_OID)),
        GEOGRAPHY_OID => Some(("geography", GEOGRAPHY_OID)),
        _ => None,
    }
}

fn pgjdbc_typeinfo_sqltype_row(
    typname: &str,
    oid: u32,
    field_format: FieldFormat,
) -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new("is_array".to_string(), None, None, Type::BOOL, field_format),
        FieldInfo::new("typtype".to_string(), None, None, Type::CHAR, field_format),
        FieldInfo::new(
            "typname".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new("oid".to_string(), None, None, Type::OID, field_format),
    ];
    let fields = Arc::new(fields);
    let typname = typname.to_string();
    let row_stream = futures::stream::once({
        let fields = Arc::clone(&fields);
        async move {
            let mut encoder = DataRowEncoder::new(fields);
            encode_bool_field(&mut encoder, false, field_format)?;
            encode_char_field(&mut encoder, b'b', field_format)?;
            encoder.encode_field(&Some(typname.as_str()))?;
            encode_u32_field(&mut encoder, oid, field_format)?;
            Ok(encoder.take_row())
        }
    });
    Ok(QueryResponse::new(fields, Box::pin(row_stream)))
}

fn pgjdbc_typeinfo_name_row(
    typname: &str,
    field_format: FieldFormat,
) -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new("?column?".to_string(), None, None, Type::BOOL, field_format),
        FieldInfo::new(
            "nspname".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new(
            "typname".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
    ];
    let fields = Arc::new(fields);
    let typname = typname.to_string();
    let row_stream = futures::stream::once({
        let fields = Arc::clone(&fields);
        async move {
            let mut encoder = DataRowEncoder::new(fields);
            encode_bool_field(&mut encoder, true, field_format)?;
            encoder.encode_field(&Some("public"))?;
            encoder.encode_field(&Some(typname.as_str()))?;
            Ok(encoder.take_row())
        }
    });
    Ok(QueryResponse::new(fields, Box::pin(row_stream)))
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

async fn catalog_query_response(
    statement: &Statement,
    session_context: &SessionContext,
) -> Option<PgWireResult<Response>> {
    let sql = statement.to_string().to_lowercase();
    if let Some(response) = pg_type_oid_in_response(&sql) {
        return Some(response.map(Response::Query));
    }

    let surface = classify_catalog_surface(&sql)?;
    match surface {
        CatalogSurface::PgTypePostgisProbe => {
            Some(pg_type_oid_typname_probe_response(&sql).map(Response::Query))
        }
        CatalogSurface::StyleTableExists => {
            Some(single_bool_row("exists", false).map(Response::Query))
        }
        CatalogSurface::PgJdbcTableListing => {
            Some(pgjdbc_table_listing_response(session_context).map(Response::Query))
        }
        CatalogSurface::PgJdbcPrimaryKeys => Some(
            pgjdbc_primary_keys_response(session_context, None, FieldFormat::Text)
                .await
                .map(Response::Query),
        ),
        CatalogSurface::PgJdbcColumns => Some(
            pgjdbc_columns_response(session_context, None, None, None, FieldFormat::Text)
                .await
                .map(Response::Query),
        ),
        CatalogSurface::PgClassTableListing => {
            Some(pg_class_table_listing_response(session_context).map(Response::Query))
        }
        CatalogSurface::PgClassOidLookup => {
            Some(pg_class_oid_lookup_response(&sql, FieldFormat::Text).map(Response::Query))
        }
        CatalogSurface::PgInheritsRelname => {
            Some(empty_response("relname", Type::VARCHAR).map(Response::Query))
        }
        CatalogSurface::PgInheritsCount => Some(single_i64_row("count", 0).map(Response::Query)),
        CatalogSurface::PgAttributeColumnListing => Some(
            pg_attribute_column_listing_response(&sql, session_context)
                .await
                .map(Response::Query),
        ),
        CatalogSurface::GeographyColumnsProbe => {
            Some(geography_columns_probe_response().map(Response::Query))
        }
        CatalogSurface::PgDescriptionRegclass => {
            Some(empty_response("description", Type::VARCHAR).map(Response::Query))
        }
        CatalogSurface::PgIndexPrimaryKeyProbe => {
            Some(pg_index_primary_key_probe_response().map(Response::Query))
        }
        CatalogSurface::PgIndexKeyColumn => {
            Some(pg_index_key_column_response(&sql).map(Response::Query))
        }
        CatalogSurface::PgIndexForTable => Some(
            pg_index_for_table_response(&sql, session_context)
                .await
                .map(Response::Query),
        ),
        CatalogSurface::PgIndexIndkey => Some(pg_index_indkey_response(&sql).map(Response::Query)),
        CatalogSurface::PgGetIndexdef => Some(pg_get_indexdef_response(&sql).map(Response::Query)),
        CatalogSurface::PgClassRelkindRegclass => {
            Some(single_text_row("relkind", "r").map(Response::Query))
        }
        CatalogSurface::PgAttributeRegclassIdentity => {
            Some(empty_response("attidentity", Type::VARCHAR).map(Response::Query))
        }
        CatalogSurface::PgAttributeRegclassName => {
            Some(empty_response("attname", Type::VARCHAR).map(Response::Query))
        }
        CatalogSurface::PgAttributeGeomTypeName => {
            Some(single_text_row("typname", "geometry").map(Response::Query))
        }
    }
}

async fn postgres_driver_cursor_response(
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

async fn dummy_logical_plan(session_context: &SessionContext) -> PgWireResult<LogicalPlan> {
    let dataframe = session_context
        .sql("SELECT 1")
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    dataframe
        .into_optimized_plan()
        .map_err(|e| PgWireError::ApiError(Box::new(e)))
}

fn is_geotools_binary_geometry_query(sql: &str) -> bool {
    sql.contains(" from ")
        && (sql.contains("st_asewkb")
            || sql.contains("st_asbinary")
            || sql.contains("st_force2d")
            || sql.contains("st_simplify")
            || sql.contains("st_curvetoline"))
}

async fn geotools_binary_geometry_describe_plan(
    sql: &str,
    statement: &Statement,
    session_context: &SessionContext,
) -> PgWireResult<LogicalPlan> {
    let actual_plan = session_context
        .state()
        .statement_to_plan(DataFusionStatement::Statement(Box::new(statement.clone())))
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    let expr = actual_plan
        .schema()
        .columns()
        .into_iter()
        .map(Expr::Column)
        .collect();
    let fields = geotools_st_asewkb_describe_fields(sql);
    let schema =
        DFSchema::try_from(Schema::new(fields)).map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    Projection::try_new_with_schema(expr, Arc::new(actual_plan), Arc::new(schema))
        .map(LogicalPlan::Projection)
        .map_err(|e| PgWireError::ApiError(Box::new(e)))
}

fn geotools_st_asewkb_describe_fields(sql: &str) -> Vec<Field> {
    select_items(sql)
        .into_iter()
        .map(|item| {
            let item_lower = item.to_lowercase();
            let name = select_item_output_name(&item).unwrap_or_else(|| {
                if item_lower.contains("st_asewkb") {
                    "geom".to_string()
                } else {
                    "?column?".to_string()
                }
            });
            let data_type = if is_geotools_binary_geometry_select_item(&item_lower, &name) {
                // `field_into_pg_type` maps Binary fields named `geom` to the
                // custom geometry OID by convention. Use FixedSizeBinary only
                // for the describe-time dummy plan so RowDescription stays
                // bytea for GeoTools' WKB-consuming geometry projections.
                DataType::FixedSizeBinary(1)
            } else if name.eq_ignore_ascii_case("id") {
                DataType::Int32
            } else if name.eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN) {
                DataType::Int64
            } else {
                DataType::Utf8
            };
            Field::new(name, data_type, true)
        })
        .collect()
}

fn is_geotools_binary_geometry_select_item(item_lower: &str, _output_name: &str) -> bool {
    if item_lower.contains("st_asbinary")
        || item_lower.contains("st_asewkb")
        || item_lower.contains("st_asmvt")
    {
        return true;
    }
    if item_lower.contains("st_astext")
        || item_lower.contains("st_extent")
        || item_lower.contains("st_hasarc")
    {
        return false;
    }
    item_lower.contains("st_force2d")
        || item_lower.contains("st_simplify")
        || item_lower.contains("st_curvetoline")
}

async fn geotools_st_asewkb_response(
    statement: &Statement,
    params: &ParamValues,
    session_context: &SessionContext,
    client: &dyn HookClient,
) -> PgWireResult<Response> {
    let plan = session_context
        .state()
        .statement_to_plan(DataFusionStatement::Statement(Box::new(statement.clone())))
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?
        .replace_params_with_values(params)
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    let plan = session_context
        .state()
        .optimize(&plan)
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    let dataframe = session_context
        .execute_logical_plan(plan)
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    let fields = Arc::new(
        dataframe
            .schema()
            .as_arrow()
            .fields()
            .iter()
            .enumerate()
            .map(|(idx, field)| {
                let pg_type = if field.name().eq_ignore_ascii_case("geom")
                    && matches!(
                        field.data_type(),
                        DataType::Binary | DataType::LargeBinary | DataType::BinaryView
                    ) {
                    Type::BYTEA
                } else {
                    field_into_pg_type(field)?
                };
                Ok(FieldInfo::new(
                    field.name().to_string(),
                    None,
                    None,
                    pg_type,
                    current_portal_field_format(client, idx),
                ))
            })
            .collect::<PgWireResult<Vec<_>>>()?,
    );
    let recordbatch_stream = dataframe
        .execute_stream()
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    let fields_ref = Arc::clone(&fields);
    let row_stream = recordbatch_stream
        .map(move |batch| {
            let row_stream: Box<dyn Iterator<Item = PgWireResult<_>> + Send + Sync> = match batch {
                Ok(batch) => Box::new(encode_recordbatch(Arc::clone(&fields_ref), batch)),
                Err(e) => Box::new(std::iter::once(Err(PgWireError::ApiError(e.into())))),
            };
            futures::stream::iter(row_stream)
        })
        .flatten();
    Ok(Response::Query(QueryResponse::new(fields, row_stream)))
}

async fn pgjdbc_primary_keys_logical_plan(
    session_context: &SessionContext,
    param_count: usize,
) -> PgWireResult<LogicalPlan> {
    let table_cat = text_param_or_null(1, param_count);
    let table_schem = text_param_or_null(2, param_count);
    let sql = format!(
        "SELECT {table_cat} AS \"TABLE_CAT\", \
                {table_schem} AS \"TABLE_SCHEM\", \
                CAST(NULL AS TEXT) AS \"TABLE_NAME\", \
                CAST(NULL AS TEXT) AS \"COLUMN_NAME\", \
                CAST(NULL AS INTEGER) AS \"KEY_SEQ\", \
                CAST(NULL AS TEXT) AS \"PK_NAME\""
    );
    let dataframe = session_context
        .sql(&sql)
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    dataframe
        .into_optimized_plan()
        .map_err(|e| PgWireError::ApiError(Box::new(e)))
}

async fn pgjdbc_typeinfo_sqltype_logical_plan(
    session_context: &SessionContext,
    param_count: usize,
) -> PgWireResult<LogicalPlan> {
    let oid = integer_param_or_null(1, param_count);
    let dataframe = session_context
        .sql(&format!(
            "SELECT CAST(NULL AS BOOLEAN) AS is_array, \
                    CAST(NULL AS TEXT) AS typtype, \
                    CAST(NULL AS TEXT) AS typname, \
                    {oid} AS oid"
        ))
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    dataframe
        .into_optimized_plan()
        .map_err(|e| PgWireError::ApiError(Box::new(e)))
}

async fn pgjdbc_typeinfo_name_logical_plan(
    session_context: &SessionContext,
    param_count: usize,
) -> PgWireResult<LogicalPlan> {
    let is_current_schema = if param_count >= 1 {
        "CAST($1 AS INTEGER) IS NULL".to_string()
    } else {
        "CAST(NULL AS BOOLEAN)".to_string()
    };
    let dataframe = session_context
        .sql(&format!(
            "SELECT {is_current_schema} AS \"?column?\", \
                    CAST(NULL AS TEXT) AS nspname, \
                    CAST(NULL AS TEXT) AS typname"
        ))
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    dataframe
        .into_optimized_plan()
        .map_err(|e| PgWireError::ApiError(Box::new(e)))
}

async fn pg_type_oid_typname_logical_plan(
    session_context: &SessionContext,
    param_count: usize,
) -> PgWireResult<LogicalPlan> {
    let oid = integer_param_or_null(1, param_count);
    let dataframe = session_context
        .sql(&format!(
            "SELECT {oid} AS oid, CAST(NULL AS TEXT) AS typname"
        ))
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    dataframe
        .into_optimized_plan()
        .map_err(|e| PgWireError::ApiError(Box::new(e)))
}

fn integer_param_or_null(idx: usize, param_count: usize) -> String {
    if idx <= param_count {
        format!("CAST(${idx} AS INTEGER)")
    } else {
        "CAST(NULL AS INTEGER)".to_string()
    }
}

async fn ogr_pg_class_oid_lookup_logical_plan(
    sql: &str,
    session_context: &SessionContext,
) -> PgWireResult<LogicalPlan> {
    let select_list = selected_pg_class_oid_columns(sql)
        .into_iter()
        .map(|column| match column {
            PgClassOidColumn::Oid => "CAST(NULL AS INTEGER) AS oid",
            PgClassOidColumn::Namespace => "CAST(NULL AS TEXT) AS nspname",
            PgClassOidColumn::Relname => "CAST(NULL AS TEXT) AS relname",
        })
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!("SELECT {select_list}");
    let dataframe = session_context
        .sql(&query)
        .await
        .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    dataframe
        .into_optimized_plan()
        .map_err(|e| PgWireError::ApiError(Box::new(e)))
}

fn text_param_or_null(idx: usize, param_count: usize) -> String {
    if idx <= param_count {
        format!("CAST(${idx} AS TEXT)")
    } else {
        "CAST(NULL AS TEXT)".to_string()
    }
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
    if let Some(values) = array
        .as_any()
        .downcast_ref::<datafusion::arrow::array::StringViewArray>()
    {
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
        (20_u32, "int8", 8_i16),
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

fn pg_type_oid_typname_probe_response(sql: &str) -> PgWireResult<QueryResponse> {
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
    let asks_for_spatial_types = (sql.matches("'bytea'").count() >= 2 && sql.contains("typtype"))
        || sql.contains("'geometry'")
        || sql.contains("'geography'");
    let rows = if asks_for_spatial_types {
        vec![(GEOMETRY_OID, "geometry"), (GEOGRAPHY_OID, "geography")]
    } else {
        [
            (17_u32, "bytea"),
            (20_u32, "int8"),
            (23_u32, "int4"),
            (25_u32, "text"),
        ]
        .into_iter()
        .filter(|(_oid, typname)| sql.contains(&format!("'{typname}'")))
        .collect::<Vec<_>>()
    };
    let fields = Arc::new(fields);
    let row_stream = futures::stream::iter(rows.into_iter().map({
        let fields = Arc::clone(&fields);
        move |(oid, typname)| {
            let mut encoder = DataRowEncoder::new(Arc::clone(&fields));
            encode_u32_field(&mut encoder, oid, FieldFormat::Text)?;
            encoder.encode_field(&Some(typname))?;
            Ok(encoder.take_row())
        }
    }));
    Ok(QueryResponse::new(fields, Box::pin(row_stream)))
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

async fn pg_index_for_table_response(
    sql: &str,
    session_context: &SessionContext,
) -> PgWireResult<QueryResponse> {
    let table = parse_regclass_table(sql).unwrap_or_else(|| "points".to_string());
    let index = synthetic_index_for_table(session_context, &table).await;
    let index_oid = synthetic_index_oid_for_table(&index.table);
    if let Ok(mut indexes) = SYNTHETIC_INDEXES.lock() {
        indexes.insert(index_oid, index);
    }
    single_oid_row("indexrelid", index_oid)
}

fn pg_index_indkey_response(sql: &str) -> PgWireResult<QueryResponse> {
    let index = parse_indexrelid(sql).and_then(lookup_synthetic_index);
    let attnum = index.map(|idx| idx.key_attnum).unwrap_or(1);
    single_text_row("indkey", &attnum.to_string())
}

fn pg_index_key_column_response(sql: &str) -> PgWireResult<QueryResponse> {
    let index = parse_indexrelid(sql).and_then(lookup_synthetic_index);
    let key_column = index
        .map(|idx| idx.key_column)
        .unwrap_or_else(|| "id".to_string());
    single_attname_attnotnull_row(&key_column, true)
}

fn pg_get_indexdef_response(sql: &str) -> PgWireResult<QueryResponse> {
    let index = parse_pg_get_indexdef_oid(sql)
        .and_then(lookup_synthetic_index)
        .unwrap_or_else(|| SyntheticIndex {
            table: "points".to_string(),
            key_column: "id".to_string(),
            key_attnum: 1,
        });
    let def = format!(
        "CREATE UNIQUE INDEX {}_pkey ON public.{} ({})",
        index.table, index.table, index.key_column
    );
    single_text_row("pg_get_indexdef", &def)
}

async fn synthetic_index_for_table(
    session_context: &SessionContext,
    table: &str,
) -> SyntheticIndex {
    let (key_column, key_attnum) = table_key_column(session_context, table)
        .await
        .unwrap_or_else(|| ("id".to_string(), 1));
    SyntheticIndex {
        table: table.to_string(),
        key_column,
        key_attnum,
    }
}

async fn table_key_column(
    session_context: &SessionContext,
    table_name: &str,
) -> Option<(String, i16)> {
    let state = session_context.state();
    let catalog_list = state.catalog_list();
    let catalog = catalog_list.catalog("quackgis")?;
    let schema = catalog.schema("main")?;
    let table = schema.table(table_name).await.ok().flatten()?;
    let schema = table.schema();
    if let Some(key) = schema
        .fields()
        .iter()
        .enumerate()
        .find(|(_, field)| field.name().eq_ignore_ascii_case("id"))
        .and_then(|(idx, field)| Some((field.name().clone(), i16::try_from(idx + 1).ok()?)))
    {
        return Some(key);
    }

    if let Some(key) = schema
        .fields()
        .iter()
        .enumerate()
        .find(|(_, field)| field.name().eq_ignore_ascii_case(SYNTHETIC_ROWID_COLUMN))
        .and_then(|(idx, field)| Some((field.name().clone(), i16::try_from(idx + 1).ok()?)))
    {
        return Some(key);
    }

    schema
        .fields()
        .iter()
        .any(|field| crate::geometry_columns::is_geometry_column_name(field.name()))
        .then_some((SYNTHETIC_ROWID_COLUMN.to_string(), 1))
}

fn lookup_synthetic_index(index_oid: u32) -> Option<SyntheticIndex> {
    if index_oid == SYNTHETIC_PK_INDEX_OID {
        return Some(SyntheticIndex {
            table: "points".to_string(),
            key_column: "id".to_string(),
            key_attnum: 1,
        });
    }
    SYNTHETIC_INDEXES
        .lock()
        .ok()
        .and_then(|indexes| indexes.get(&index_oid).cloned())
}

fn synthetic_index_oid_for_table(table: &str) -> u32 {
    if table.eq_ignore_ascii_case("points") {
        return SYNTHETIC_PK_INDEX_OID;
    }
    let hash = table.bytes().fold(0_u32, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(u32::from(b))
    });
    90_200 + (hash % 10_000)
}

fn synthetic_table_oid_for_table(table: &str) -> u32 {
    let hash = table.bytes().fold(0_u32, |acc, b| {
        acc.wrapping_mul(33).wrapping_add(u32::from(b))
    });
    70_000 + (hash % 20_000)
}

fn pg_class_oid_lookup_response(
    sql: &str,
    field_format: FieldFormat,
) -> PgWireResult<QueryResponse> {
    let table = parse_pg_class_relname_filter(sql).unwrap_or_else(|| "points".to_string());
    let oid = synthetic_table_oid_for_table(&table);
    if let Ok(mut tables) = SYNTHETIC_TABLE_OIDS.lock() {
        tables.insert(oid, table.clone());
    }
    let selected = selected_pg_class_oid_columns(sql);
    let fields = selected
        .iter()
        .map(|column| match column {
            PgClassOidColumn::Oid => {
                FieldInfo::new("oid".to_string(), None, None, Type::INT4, field_format)
            }
            PgClassOidColumn::Namespace => FieldInfo::new(
                "nspname".to_string(),
                None,
                None,
                Type::VARCHAR,
                field_format,
            ),
            PgClassOidColumn::Relname => FieldInfo::new(
                "relname".to_string(),
                None,
                None,
                Type::VARCHAR,
                field_format,
            ),
        })
        .collect::<Vec<_>>();
    let fields = Arc::new(fields);
    let row_stream = futures::stream::once({
        let fields = Arc::clone(&fields);
        let selected = selected.clone();
        async move {
            let mut encoder = DataRowEncoder::new(fields);
            for column in selected {
                match column {
                    PgClassOidColumn::Oid => {
                        encode_i32_field(&mut encoder, oid as i32, field_format)?
                    }
                    PgClassOidColumn::Namespace => encoder.encode_field(&Some("public"))?,
                    PgClassOidColumn::Relname => encoder.encode_field(&Some(table.as_str()))?,
                }
            }
            Ok(encoder.take_row())
        }
    });
    Ok(QueryResponse::new(fields, Box::pin(row_stream)))
}

#[derive(Clone, Copy)]
enum PgClassOidColumn {
    Oid,
    Namespace,
    Relname,
}

fn selected_pg_class_oid_columns(sql: &str) -> Vec<PgClassOidColumn> {
    let sql = sql.to_lowercase();
    let select_end = sql.find(" from ").unwrap_or(sql.len());
    let select_list = &sql[..select_end];
    let mut columns = Vec::new();
    if select_list.contains("c.oid") || select_list.contains(" oid") {
        columns.push(PgClassOidColumn::Oid);
    }
    if select_list.contains("n.nspname") || select_list.contains("nspname") {
        columns.push(PgClassOidColumn::Namespace);
    }
    if select_list.contains("c.relname") || select_list.contains("relname") {
        columns.push(PgClassOidColumn::Relname);
    }
    if columns.is_empty() {
        columns.push(PgClassOidColumn::Oid);
        columns.push(PgClassOidColumn::Relname);
    }
    columns
}

fn parse_pg_class_relname_filter(sql: &str) -> Option<String> {
    parse_regex_relname(sql)
        .or_else(|| parse_relname_equality(sql))
        .or_else(|| parse_regclass_table(sql))
}

fn parse_regex_relname(sql: &str) -> Option<String> {
    let marker = "^(";
    let start = sql.find(marker)? + marker.len();
    let end = sql[start..].find(")$")? + start;
    let table = sql[start..end].trim_matches('"');
    (!table.is_empty()).then(|| table.to_string())
}

fn parse_relname_equality(sql: &str) -> Option<String> {
    for marker in ["c.relname =", "c.relname=", "relname =", "relname="] {
        if let Some(start) = sql.find(marker) {
            let after_equals = sql[start + marker.len()..].trim_start();
            if let Some(value) = parse_single_quoted_literal(after_equals) {
                return Some(value.trim_matches('"').to_string());
            }
        }
    }
    None
}

fn lookup_synthetic_table_oid(table_oid: u32) -> Option<String> {
    SYNTHETIC_TABLE_OIDS
        .lock()
        .ok()
        .and_then(|tables| tables.get(&table_oid).cloned())
}

fn parse_regclass_table(sql: &str) -> Option<String> {
    let markers = ["'\"public\".\"", "'public.", "\"public\".\""];
    for marker in markers {
        if let Some(start) = sql.find(marker) {
            let table_start = start + marker.len();
            let end_marker = if marker.ends_with('"') { "\"" } else { "'" };
            let table_end = sql[table_start..].find(end_marker)? + table_start;
            let table = sql[table_start..table_end].trim_matches('"');
            if !table.is_empty() {
                return Some(table.to_string());
            }
        }
    }
    None
}

fn parse_indexrelid(sql: &str) -> Option<u32> {
    let start = sql.find("indexrelid")? + "indexrelid".len();
    parse_first_u32(&sql[start..])
}

fn parse_pg_get_indexdef_oid(sql: &str) -> Option<u32> {
    let start = sql.find("pg_get_indexdef")? + "pg_get_indexdef".len();
    parse_first_u32(&sql[start..])
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

fn pgjdbc_table_listing_response(session_context: &SessionContext) -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new(
            "TABLE_CAT".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "TABLE_SCHEM".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "TABLE_NAME".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "TABLE_TYPE".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "REMARKS".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "TYPE_CAT".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "TYPE_SCHEM".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "TYPE_NAME".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "SELF_REFERENCING_COL_NAME".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "REF_GENERATION".to_string(),
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
            encoder.encode_field(&Some("quackgis"))?;
            encoder.encode_field(&Some(nspname.as_str()))?;
            encoder.encode_field(&Some(relname.as_str()))?;
            encoder.encode_field(&Some("TABLE"))?;
            encoder.encode_field(&None::<&str>)?;
            encoder.encode_field(&Some(""))?;
            encoder.encode_field(&Some(""))?;
            encoder.encode_field(&Some(""))?;
            encoder.encode_field(&Some(""))?;
            encoder.encode_field(&Some(""))?;
            Ok(encoder.take_row())
        }
    }));

    Ok(QueryResponse::new(fields, Box::pin(row_stream)))
}

async fn pgjdbc_columns_extended_response(
    statement: &Statement,
    params: &ParamValues,
    session_context: &SessionContext,
    client: &dyn HookClient,
) -> Option<PgWireResult<Response>> {
    let sql = statement.to_string().to_lowercase();
    if !is_pgjdbc_columns_query(&sql) {
        return None;
    }
    let schema_filter = string_param(params, 1);
    let table_filter = string_param(params, 2);
    let column_filter = string_param(params, 3);
    Some(
        pgjdbc_columns_response(
            session_context,
            schema_filter.as_deref(),
            table_filter.as_deref(),
            column_filter.as_deref(),
            current_portal_result_format(client),
        )
        .await
        .map(Response::Query),
    )
}

async fn pgjdbc_primary_keys_extended_response(
    statement: &Statement,
    params: &ParamValues,
    session_context: &SessionContext,
    client: &dyn HookClient,
) -> Option<PgWireResult<Response>> {
    let sql = statement.to_string().to_lowercase();
    if !is_pgjdbc_primary_keys_query(&sql) {
        return None;
    }
    let table_filter = last_string_param(params);
    Some(
        pgjdbc_primary_keys_response(
            session_context,
            table_filter.as_deref(),
            current_portal_result_format(client),
        )
        .await
        .map(Response::Query),
    )
}

fn current_portal_result_format(client: &dyn HookClient) -> FieldFormat {
    current_portal_field_format(client, 0)
}

fn current_portal_field_format(client: &dyn HookClient, idx: usize) -> FieldFormat {
    client
        .portal_store()
        .get_portal(DEFAULT_NAME)
        .map(|portal| portal.result_column_format.format_for(idx))
        .unwrap_or(FieldFormat::Text)
}

async fn pgjdbc_primary_keys_response(
    session_context: &SessionContext,
    table_filter: Option<&str>,
    field_format: FieldFormat,
) -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new(
            "TABLE_CAT".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new(
            "TABLE_SCHEM".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new(
            "TABLE_NAME".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new(
            "COLUMN_NAME".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new("KEY_SEQ".to_string(), None, None, Type::INT4, field_format),
        FieldInfo::new(
            "PK_NAME".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
    ];

    let mut rows = Vec::new();
    for (table, schema) in visible_tables_as_public_relations(session_context) {
        if table_filter.is_some_and(|filter| !table.eq_ignore_ascii_case(filter)) {
            continue;
        }
        let index = synthetic_index_for_table(session_context, &table).await;
        rows.push((table, schema, index.key_column));
    }

    let fields = Arc::new(fields);
    let row_stream = futures::stream::iter(rows.into_iter().map({
        let fields = Arc::clone(&fields);
        move |(table, schema, column)| {
            let pk_name = format!("{table}_pkey");
            let mut encoder = DataRowEncoder::new(Arc::clone(&fields));
            encoder.encode_field(&Some("quackgis"))?;
            encoder.encode_field(&Some(schema.as_str()))?;
            encoder.encode_field(&Some(table.as_str()))?;
            encoder.encode_field(&Some(column.as_str()))?;
            encode_i32_field(&mut encoder, 1, field_format)?;
            encoder.encode_field(&Some(pk_name.as_str()))?;
            Ok(encoder.take_row())
        }
    }));

    Ok(QueryResponse::new(fields, Box::pin(row_stream)))
}

async fn pgjdbc_columns_response(
    session_context: &SessionContext,
    schema_filter: Option<&str>,
    table_filter: Option<&str>,
    column_filter: Option<&str>,
    field_format: FieldFormat,
) -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new(
            "current_database".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new(
            "nspname".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new(
            "relname".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new(
            "attname".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new("atttypid".to_string(), None, None, Type::OID, field_format),
        FieldInfo::new(
            "attnotnull".to_string(),
            None,
            None,
            Type::BOOL,
            field_format,
        ),
        FieldInfo::new(
            "atttypmod".to_string(),
            None,
            None,
            Type::INT4,
            field_format,
        ),
        FieldInfo::new("attlen".to_string(), None, None, Type::INT2, field_format),
        FieldInfo::new(
            "typtypmod".to_string(),
            None,
            None,
            Type::INT4,
            field_format,
        ),
        FieldInfo::new(
            "attnum".to_string(),
            None,
            None,
            Type::NUMERIC,
            field_format,
        ),
        FieldInfo::new(
            "attidentity".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new(
            "attgenerated".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new("adsrc".to_string(), None, None, Type::VARCHAR, field_format),
        FieldInfo::new(
            "description".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
        FieldInfo::new(
            "typbasetype".to_string(),
            None,
            None,
            Type::OID,
            field_format,
        ),
        FieldInfo::new(
            "typtype".to_string(),
            None,
            None,
            Type::VARCHAR,
            field_format,
        ),
    ];

    let mut rows = Vec::new();
    let state = session_context.state();
    let catalog_list = state.catalog_list();
    if let Some(catalog) = catalog_list.catalog("quackgis")
        && let Some(schema) = catalog.schema("main")
    {
        for table_name in schema.table_names() {
            if !like_filter_matches(schema_filter, "public")
                || !like_filter_matches(table_filter, &table_name)
            {
                continue;
            }
            let Some(table) = schema.table(&table_name).await.ok().flatten() else {
                continue;
            };
            for (idx, field) in table.schema().fields().iter().enumerate() {
                if !like_filter_matches(column_filter, field.name()) {
                    continue;
                }
                let type_info = pg_type_for_arrow_field(field.name(), field.data_type());
                rows.push(PgJdbcColumnRow {
                    schema: "public".to_string(),
                    table: table_name.clone(),
                    column: field.name().clone(),
                    type_oid: type_info.oid,
                    attnotnull: !field.is_nullable(),
                    atttypmod: -1,
                    attlen: type_info.attlen,
                    typtypmod: -1,
                    attnum: i64::try_from(idx + 1).unwrap_or(i64::MAX),
                    typbasetype: 0,
                    typtype: "b".to_string(),
                });
            }
        }
    }

    let fields = Arc::new(fields);
    let row_stream = futures::stream::iter(rows.into_iter().map({
        let fields = Arc::clone(&fields);
        move |row| {
            let mut encoder = DataRowEncoder::new(Arc::clone(&fields));
            encoder.encode_field(&Some("quackgis"))?;
            encoder.encode_field(&Some(row.schema.as_str()))?;
            encoder.encode_field(&Some(row.table.as_str()))?;
            encoder.encode_field(&Some(row.column.as_str()))?;
            encode_u32_field(&mut encoder, row.type_oid, field_format)?;
            encode_bool_field(&mut encoder, row.attnotnull, field_format)?;
            encode_i32_field(&mut encoder, row.atttypmod, field_format)?;
            encode_i16_field(&mut encoder, row.attlen, field_format)?;
            encode_i32_field(&mut encoder, row.typtypmod, field_format)?;
            encode_numeric_i64_field(&mut encoder, row.attnum, field_format)?;
            encoder.encode_field(&None::<&str>)?;
            encoder.encode_field(&None::<&str>)?;
            encoder.encode_field(&None::<&str>)?;
            encoder.encode_field(&None::<&str>)?;
            encode_u32_field(&mut encoder, row.typbasetype, field_format)?;
            encoder.encode_field(&Some(row.typtype.as_str()))?;
            Ok(encoder.take_row())
        }
    }));

    Ok(QueryResponse::new(fields, Box::pin(row_stream)))
}

struct PgJdbcColumnRow {
    schema: String,
    table: String,
    column: String,
    type_oid: u32,
    attnotnull: bool,
    atttypmod: i32,
    attlen: i16,
    typtypmod: i32,
    attnum: i64,
    typbasetype: u32,
    typtype: String,
}

struct PgTypeInfo {
    oid: u32,
    attlen: i16,
}

fn pg_type_for_arrow_field(column_name: &str, data_type: &DataType) -> PgTypeInfo {
    if crate::geometry_columns::is_geometry_column_name(column_name) {
        return PgTypeInfo {
            oid: GEOMETRY_OID,
            attlen: -1,
        };
    }

    match data_type {
        DataType::Boolean => PgTypeInfo { oid: 16, attlen: 1 },
        DataType::Int16 => PgTypeInfo { oid: 21, attlen: 2 },
        DataType::Int32 => PgTypeInfo { oid: 23, attlen: 4 },
        DataType::Int64 => PgTypeInfo { oid: 20, attlen: 8 },
        DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
            PgTypeInfo { oid: 20, attlen: 8 }
        }
        DataType::Float32 => PgTypeInfo {
            oid: 700,
            attlen: 4,
        },
        DataType::Float64 => PgTypeInfo {
            oid: 701,
            attlen: 8,
        },
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => PgTypeInfo {
            oid: 17,
            attlen: -1,
        },
        DataType::Date32 => PgTypeInfo {
            oid: 1082,
            attlen: 4,
        },
        DataType::Timestamp(_, _) => PgTypeInfo {
            oid: 1114,
            attlen: 8,
        },
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => PgTypeInfo {
            oid: 25,
            attlen: -1,
        },
        _ => PgTypeInfo {
            oid: 25,
            attlen: -1,
        },
    }
}

fn encode_i16_field(
    encoder: &mut DataRowEncoder,
    value: i16,
    field_format: FieldFormat,
) -> PgWireResult<()> {
    match field_format {
        FieldFormat::Text => {
            let text = value.to_string();
            encoder.encode_field(&Some(text.as_str()))
        }
        FieldFormat::Binary => encoder.encode_field(&Some(value)),
    }
}

fn encode_i32_field(
    encoder: &mut DataRowEncoder,
    value: i32,
    field_format: FieldFormat,
) -> PgWireResult<()> {
    match field_format {
        FieldFormat::Text => {
            let text = value.to_string();
            encoder.encode_field(&Some(text.as_str()))
        }
        FieldFormat::Binary => encoder.encode_field(&Some(value)),
    }
}

fn encode_u32_field(
    encoder: &mut DataRowEncoder,
    value: u32,
    field_format: FieldFormat,
) -> PgWireResult<()> {
    match field_format {
        FieldFormat::Text => {
            let text = value.to_string();
            encoder.encode_field(&Some(text.as_str()))
        }
        FieldFormat::Binary => encoder.encode_field(&Some(value)),
    }
}

fn encode_bool_field(
    encoder: &mut DataRowEncoder,
    value: bool,
    field_format: FieldFormat,
) -> PgWireResult<()> {
    match field_format {
        FieldFormat::Text => encoder.encode_field(&Some(if value { "t" } else { "f" })),
        FieldFormat::Binary => encoder.encode_field(&Some(value)),
    }
}

fn encode_char_field(
    encoder: &mut DataRowEncoder,
    value: u8,
    field_format: FieldFormat,
) -> PgWireResult<()> {
    match field_format {
        FieldFormat::Text => {
            let text = char::from(value).to_string();
            encoder.encode_field(&Some(text.as_str()))
        }
        FieldFormat::Binary => encoder.encode_field(&Some(value as i8)),
    }
}

fn encode_numeric_i64_field(
    encoder: &mut DataRowEncoder,
    value: i64,
    field_format: FieldFormat,
) -> PgWireResult<()> {
    match field_format {
        FieldFormat::Text => {
            let text = value.to_string();
            encoder.encode_field(&Some(text.as_str()))
        }
        FieldFormat::Binary => {
            let decimal = rust_decimal::Decimal::from(value);
            encoder.encode_field(&Some(decimal))
        }
    }
}

fn like_filter_matches(filter: Option<&str>, value: &str) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    sql_like_pattern_matches(filter, value)
}

#[derive(Debug, Clone, Copy)]
enum LikeToken {
    Literal(char),
    AnyOne,
    AnyMany,
}

fn sql_like_pattern_matches(pattern: &str, value: &str) -> bool {
    let mut tokens = Vec::new();
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '%' => tokens.push(LikeToken::AnyMany),
            '_' => tokens.push(LikeToken::AnyOne),
            '\\' => tokens.push(LikeToken::Literal(
                chars.next().unwrap_or('\\').to_ascii_lowercase(),
            )),
            ch => tokens.push(LikeToken::Literal(ch.to_ascii_lowercase())),
        }
    }

    let value: Vec<char> = value.chars().map(|ch| ch.to_ascii_lowercase()).collect();
    like_tokens_match(&tokens, 0, &value, 0)
}

fn like_tokens_match(
    tokens: &[LikeToken],
    token_idx: usize,
    value: &[char],
    value_idx: usize,
) -> bool {
    if token_idx == tokens.len() {
        return value_idx == value.len();
    }

    match tokens[token_idx] {
        LikeToken::Literal(ch) => {
            value.get(value_idx).is_some_and(|value_ch| *value_ch == ch)
                && like_tokens_match(tokens, token_idx + 1, value, value_idx + 1)
        }
        LikeToken::AnyOne => {
            value_idx < value.len()
                && like_tokens_match(tokens, token_idx + 1, value, value_idx + 1)
        }
        LikeToken::AnyMany => (value_idx..=value.len())
            .any(|next_value_idx| like_tokens_match(tokens, token_idx + 1, value, next_value_idx)),
    }
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

async fn pg_attribute_column_listing_response(
    sql: &str,
    session_context: &SessionContext,
) -> PgWireResult<QueryResponse> {
    let include_attgenerated = sql.contains("attgenerated");
    let mut fields = vec![
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
    if include_attgenerated {
        fields.push(FieldInfo::new(
            "attgenerated".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ));
    }
    let rows = ogr_attribute_rows(sql, session_context).await;
    let fields = Arc::new(fields);
    let row_stream = futures::stream::iter(rows.into_iter().map({
        let fields = Arc::clone(&fields);
        move |row| {
            let mut encoder = DataRowEncoder::new(Arc::clone(&fields));
            encoder.encode_field(&Some(row.attname.as_str()))?;
            encoder.encode_field(&Some(row.typname.as_str()))?;
            encoder.encode_field(&Some(row.attlen))?;
            encoder.encode_field(&Some(row.format_type.as_str()))?;
            encoder.encode_field(&Some(false))?;
            encoder.encode_field(&None::<&str>)?;
            encoder.encode_field(&Some(false))?;
            encoder.encode_field(&None::<&str>)?;
            if include_attgenerated {
                encoder.encode_field(&Some(""))?;
            }
            Ok(encoder.take_row())
        }
    }));

    Ok(QueryResponse::new(fields, Box::pin(row_stream)))
}

struct OgrAttributeRow {
    attname: String,
    typname: String,
    attlen: i16,
    format_type: String,
}

async fn ogr_attribute_rows(sql: &str, session_context: &SessionContext) -> Vec<OgrAttributeRow> {
    if let Some(table_name) = pg_attribute_table_name(sql, session_context)
        && let Some(rows) = schema_ogr_attribute_rows(session_context, &table_name).await
    {
        return rows;
    }
    vec![
        ogr_attribute_row("id", "int4", 4, "integer"),
        ogr_attribute_row("wkb_geometry", "bytea", -1, "bytea"),
        ogr_attribute_row("name", "text", -1, "text"),
    ]
}

fn pg_attribute_table_name(sql: &str, session_context: &SessionContext) -> Option<String> {
    if let Some(table_name) = parse_regclass_table(sql) {
        return Some(table_name);
    }
    let table_oid = parse_attrelid(sql)?;
    lookup_synthetic_table_oid(table_oid)
        .or_else(|| table_name_for_synthetic_oid(session_context, table_oid))
}

fn table_name_for_synthetic_oid(session_context: &SessionContext, oid: u32) -> Option<String> {
    visible_tables_as_public_relations(session_context)
        .into_iter()
        .map(|(table, _schema)| table)
        .find(|table| synthetic_table_oid_for_table(table) == oid)
}

async fn schema_ogr_attribute_rows(
    session_context: &SessionContext,
    table_name: &str,
) -> Option<Vec<OgrAttributeRow>> {
    let state = session_context.state();
    let catalog_list = state.catalog_list();
    let catalog = catalog_list.catalog("quackgis")?;
    let schema = catalog.schema("main")?;
    let table = schema.table(table_name).await.ok().flatten()?;
    Some(
        table
            .schema()
            .fields()
            .iter()
            .map(|field| {
                let (typname, attlen, format_type) = ogr_type_for_arrow_field(field.data_type());
                ogr_attribute_row(field.name(), typname, attlen, format_type)
            })
            .collect(),
    )
}

fn ogr_type_for_arrow_field(data_type: &DataType) -> (&'static str, i16, &'static str) {
    match data_type {
        DataType::Boolean => ("bool", 1, "boolean"),
        DataType::Int16 => ("int2", 2, "smallint"),
        DataType::Int32 => ("int4", 4, "integer"),
        DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => ("int8", 8, "bigint"),
        DataType::Float32 => ("float4", 4, "real"),
        DataType::Float64 => ("float8", 8, "double precision"),
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => ("bytea", -1, "bytea"),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => ("text", -1, "text"),
        _ => ("text", -1, "text"),
    }
}

fn ogr_attribute_row(
    attname: impl Into<String>,
    typname: impl Into<String>,
    attlen: i16,
    format_type: impl Into<String>,
) -> OgrAttributeRow {
    OgrAttributeRow {
        attname: attname.into(),
        typname: typname.into(),
        attlen,
        format_type: format_type.into(),
    }
}

fn parse_attrelid(sql: &str) -> Option<u32> {
    let start = sql.rfind("attrelid")? + "attrelid".len();
    parse_first_u32(&sql[start..])
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

fn single_attname_attnotnull_row(attname: &str, attnotnull: bool) -> PgWireResult<QueryResponse> {
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
    encoder.encode_field(&Some(attname))?;
    encoder.encode_field(&Some(attnotnull))?;
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
