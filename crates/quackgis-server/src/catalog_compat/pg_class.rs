// SPDX-License-Identifier: Apache-2.0
//! `pg_class` relation listing and synthetic OID compatibility.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion_postgres::pgwire::api::Type;
use datafusion_postgres::pgwire::api::results::{
    DataRowEncoder, FieldFormat, FieldInfo, QueryResponse,
};
use datafusion_postgres::pgwire::error::{PgWireError, PgWireResult};

use super::encoding::encode_i32_field;
use super::sql_parse::parse_single_quoted_literal;

static SYNTHETIC_TABLE_OIDS: LazyLock<Mutex<HashMap<u32, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) async fn ogr_oid_lookup_logical_plan(
    sql: &str,
    session_context: &SessionContext,
) -> PgWireResult<LogicalPlan> {
    let select_list = selected_oid_columns(sql)
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

pub(super) fn oid_lookup_response(
    sql: &str,
    field_format: FieldFormat,
) -> PgWireResult<QueryResponse> {
    let table = parse_relname_filter(sql).unwrap_or_else(|| "points".to_string());
    let oid = synthetic_table_oid_for_table(&table);
    if let Ok(mut tables) = SYNTHETIC_TABLE_OIDS.lock() {
        tables.insert(oid, table.clone());
    }
    let selected = selected_oid_columns(sql);
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

fn selected_oid_columns(sql: &str) -> Vec<PgClassOidColumn> {
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

fn parse_relname_filter(sql: &str) -> Option<String> {
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

pub(super) fn lookup_synthetic_table_oid(table_oid: u32) -> Option<String> {
    SYNTHETIC_TABLE_OIDS
        .lock()
        .ok()
        .and_then(|tables| tables.get(&table_oid).cloned())
}

pub(super) fn parse_regclass_table(sql: &str) -> Option<String> {
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

pub(super) fn synthetic_table_oid_for_table(table: &str) -> u32 {
    let hash = table.bytes().fold(0_u32, |acc, b| {
        acc.wrapping_mul(33).wrapping_add(u32::from(b))
    });
    70_000 + (hash % 20_000)
}

pub(super) fn table_name_for_synthetic_oid(
    session_context: &SessionContext,
    oid: u32,
) -> Option<String> {
    visible_tables_as_public_relations(session_context)
        .into_iter()
        .map(|(table, _schema)| table)
        .find(|table| synthetic_table_oid_for_table(table) == oid)
}

pub(super) fn table_listing_response(
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

pub(super) fn visible_tables_as_public_relations(
    session_context: &SessionContext,
) -> Vec<(String, String)> {
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
