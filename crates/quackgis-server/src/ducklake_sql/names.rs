// SPDX-License-Identifier: Apache-2.0
//! Table-name and target-shape helpers for DuckLake SQL routing.

use datafusion::sql::sqlparser::ast::{
    Delete, FromTable, ObjectName, Query, SetExpr, TableFactor, TableObject, TableWithJoins,
};

use crate::context::DUCKLAKE_CATALOG;

pub(super) fn table_name_parts(name: &ObjectName) -> Option<(String, String)> {
    let parts: Vec<String> = name
        .0
        .iter()
        .map(|p| p.to_string().trim_matches('"').to_string())
        .collect();
    match parts.as_slice() {
        [catalog, schema, table] if catalog == DUCKLAKE_CATALOG && is_ducklake_schema(schema) => {
            Some(("main".to_string(), table.clone()))
        }
        [schema, table] if is_ducklake_schema(schema) => Some(("main".to_string(), table.clone())),
        [table] => Some(("main".to_string(), table.clone())),
        _ => None,
    }
}

fn is_ducklake_schema(schema: &str) -> bool {
    schema.eq_ignore_ascii_case("main") || schema.eq_ignore_ascii_case("public")
}

pub(super) fn insert_target_parts(table: &TableObject) -> Option<(String, String)> {
    match table {
        TableObject::TableName(name) => table_name_parts(name),
        _ => None,
    }
}

pub(super) fn insert_source_is_values(query: &Query) -> bool {
    matches!(query.body.as_ref(), SetExpr::Values(_))
}

pub(super) fn delete_target_parts(delete: &Delete) -> Option<(String, String)> {
    let from = match &delete.from {
        FromTable::WithFromKeyword(t) | FromTable::WithoutKeyword(t) => t,
    };
    if from.len() != 1 || delete.using.is_some() || !delete.tables.is_empty() {
        return None;
    }
    table_factor_parts(&from[0].relation)
}

pub(super) fn update_target_parts(table: &TableWithJoins) -> Option<(String, String)> {
    if !table.joins.is_empty() {
        return None;
    }
    table_factor_parts(&table.relation)
}

fn table_factor_parts(f: &TableFactor) -> Option<(String, String)> {
    match f {
        TableFactor::Table { name, .. } => table_name_parts(name),
        _ => None,
    }
}

pub(super) fn object_name_last(name: &ObjectName) -> Option<String> {
    name.0
        .last()
        .map(|p| p.to_string().trim_matches('"').to_string())
}

pub(super) fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub(super) fn ducklake_table_ref(schema: &str, table: &str) -> String {
    format!("{DUCKLAKE_CATALOG}.{}.", quote_ident(schema)) + &quote_ident(table)
}

pub(super) fn public_table_ref(table: &str) -> String {
    format!("public.{}", quote_ident(table))
}
