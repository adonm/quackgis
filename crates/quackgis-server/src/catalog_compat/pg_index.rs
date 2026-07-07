// SPDX-License-Identifier: Apache-2.0
//! `pg_index` primary-key metadata compatibility.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use datafusion::prelude::SessionContext;
use datafusion_postgres::pgwire::api::Type;
use datafusion_postgres::pgwire::api::results::{FieldFormat, FieldInfo, QueryResponse};
use datafusion_postgres::pgwire::error::PgWireResult;

use super::SYNTHETIC_ROWID_COLUMN;
use super::encoding::{single_attname_attnotnull_row, single_oid_row, single_text_row};
use super::pg_class;
use super::sql_parse::parse_first_u32;

const SYNTHETIC_PK_INDEX_OID: u32 = 90_101;

static SYNTHETIC_INDEXES: LazyLock<Mutex<HashMap<u32, SyntheticIndex>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
pub(super) struct SyntheticIndex {
    pub(super) table: String,
    pub(super) key_column: String,
    pub(super) key_attnum: i16,
}

pub(super) fn primary_key_probe_response() -> PgWireResult<QueryResponse> {
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
    Ok(QueryResponse::new(
        std::sync::Arc::new(fields),
        Box::pin(row_stream),
    ))
}

pub(super) async fn for_table_response(
    sql: &str,
    session_context: &SessionContext,
) -> PgWireResult<QueryResponse> {
    let table = pg_class::parse_regclass_table(sql).unwrap_or_else(|| "points".to_string());
    let index = synthetic_index_for_table(session_context, &table).await;
    let index_oid = synthetic_index_oid_for_table(&index.table);
    if let Ok(mut indexes) = SYNTHETIC_INDEXES.lock() {
        indexes.insert(index_oid, index);
    }
    single_oid_row("indexrelid", index_oid)
}

pub(super) fn indkey_response(sql: &str) -> PgWireResult<QueryResponse> {
    let index = parse_indexrelid(sql).and_then(lookup_synthetic_index);
    let attnum = index.map(|idx| idx.key_attnum).unwrap_or(1);
    single_text_row("indkey", &attnum.to_string())
}

pub(super) fn key_column_response(sql: &str) -> PgWireResult<QueryResponse> {
    let index = parse_indexrelid(sql).and_then(lookup_synthetic_index);
    let key_column = index
        .map(|idx| idx.key_column)
        .unwrap_or_else(|| "id".to_string());
    single_attname_attnotnull_row(&key_column, true)
}

pub(super) fn get_indexdef_response(sql: &str) -> PgWireResult<QueryResponse> {
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

pub(super) async fn synthetic_index_for_table(
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

fn parse_indexrelid(sql: &str) -> Option<u32> {
    let start = sql.find("indexrelid")? + "indexrelid".len();
    parse_first_u32(&sql[start..])
}

fn parse_pg_get_indexdef_oid(sql: &str) -> Option<u32> {
    let start = sql.find("pg_get_indexdef")? + "pg_get_indexdef".len();
    parse_first_u32(&sql[start..])
}
