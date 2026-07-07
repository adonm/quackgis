// SPDX-License-Identifier: Apache-2.0
//! pgjdbc/GeoServer metadata compatibility.

use std::sync::Arc;

use datafusion::common::ParamValues;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion::sql::sqlparser::ast::Statement;
use datafusion_postgres::hooks::HookClient;
use datafusion_postgres::pgwire::api::Type;
use datafusion_postgres::pgwire::api::results::{
    DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response,
};
use datafusion_postgres::pgwire::error::{PgWireError, PgWireResult};

use super::encoding::{
    current_portal_result_format, encode_bool_field, encode_i16_field, encode_i32_field,
    encode_numeric_i64_field, encode_u32_field,
};
use super::params::{last_string_param, string_param};
use super::surfaces::{is_pgjdbc_columns_query, is_pgjdbc_primary_keys_query};
use super::{pg_class, pg_index, pg_type};

pub(super) async fn primary_keys_logical_plan(
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

fn text_param_or_null(idx: usize, param_count: usize) -> String {
    if idx <= param_count {
        format!("CAST(${idx} AS TEXT)")
    } else {
        "CAST(NULL AS TEXT)".to_string()
    }
}

pub(super) fn table_listing_response(
    session_context: &SessionContext,
) -> PgWireResult<QueryResponse> {
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

    let rows = pg_class::visible_tables_as_public_relations(session_context);
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

pub(super) async fn columns_extended_response(
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
        columns_response(
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

pub(super) async fn primary_keys_extended_response(
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
        primary_keys_response(
            session_context,
            table_filter.as_deref(),
            current_portal_result_format(client),
        )
        .await
        .map(Response::Query),
    )
}

pub(super) async fn primary_keys_response(
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
    for (table, schema) in pg_class::visible_tables_as_public_relations(session_context) {
        if table_filter.is_some_and(|filter| !table.eq_ignore_ascii_case(filter)) {
            continue;
        }
        let index = pg_index::synthetic_index_for_table(session_context, &table).await;
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

pub(super) async fn columns_response(
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
            let mut visible_idx = 0_i64;
            for field in table.schema().fields() {
                if crate::ducklake_sql::layout::is_layout_column(field.name()) {
                    continue;
                }
                visible_idx += 1;
                if !like_filter_matches(column_filter, field.name()) {
                    continue;
                }
                let type_info = pg_type::for_arrow_field(field.name(), field.data_type());
                rows.push(PgJdbcColumnRow {
                    schema: "public".to_string(),
                    table: table_name.clone(),
                    column: field.name().clone(),
                    type_oid: type_info.oid,
                    attnotnull: !field.is_nullable(),
                    atttypmod: -1,
                    attlen: type_info.attlen,
                    typtypmod: -1,
                    attnum: visible_idx,
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
