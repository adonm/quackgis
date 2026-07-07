// SPDX-License-Identifier: Apache-2.0
//! GeoTools/GeoServer binary-geometry projection compatibility.

use std::sync::Arc;

use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::common::{DFSchema, ParamValues};
use datafusion::logical_expr::{Expr, LogicalPlan, Projection};
use datafusion::prelude::SessionContext;
use datafusion::sql::parser::Statement as DataFusionStatement;
use datafusion::sql::sqlparser::ast::Statement;
use datafusion_postgres::arrow_pg::datatypes::{encode_recordbatch, field_into_pg_type};
use datafusion_postgres::hooks::HookClient;
use datafusion_postgres::pgwire::api::Type;
use datafusion_postgres::pgwire::api::results::{FieldInfo, QueryResponse, Response};
use datafusion_postgres::pgwire::error::{PgWireError, PgWireResult};
use futures::StreamExt;

use super::SYNTHETIC_ROWID_COLUMN;
use super::encoding::current_portal_field_format;
use super::sql_parse::{select_item_output_name, select_items};

pub(super) fn is_binary_geometry_query(sql: &str) -> bool {
    sql.contains(" from ")
        && (sql.contains("st_asewkb")
            || sql.contains("st_asbinary")
            || sql.contains("st_force2d")
            || sql.contains("st_simplify")
            || sql.contains("st_curvetoline"))
}

pub(super) async fn binary_geometry_describe_plan(
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
    let fields = st_asewkb_describe_fields(sql);
    let schema =
        DFSchema::try_from(Schema::new(fields)).map_err(|e| PgWireError::ApiError(Box::new(e)))?;
    Projection::try_new_with_schema(expr, Arc::new(actual_plan), Arc::new(schema))
        .map(LogicalPlan::Projection)
        .map_err(|e| PgWireError::ApiError(Box::new(e)))
}

fn st_asewkb_describe_fields(sql: &str) -> Vec<Field> {
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
            let data_type = if is_binary_geometry_select_item(&item_lower, &name) {
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

fn is_binary_geometry_select_item(item_lower: &str, _output_name: &str) -> bool {
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

pub(super) async fn st_asewkb_response(
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
