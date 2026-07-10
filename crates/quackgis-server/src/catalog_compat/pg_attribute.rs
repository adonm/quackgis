// SPDX-License-Identifier: Apache-2.0
//! `pg_attribute` column metadata compatibility.

use std::sync::Arc;

use datafusion::arrow::datatypes::{DataType, Field};
use datafusion::prelude::SessionContext;
use datafusion_postgres::arrow_pg::datatypes::{SpatialFamily, classify_spatial_field};
use datafusion_postgres::pgwire::api::Type;
use datafusion_postgres::pgwire::api::results::{
    DataRowEncoder, FieldFormat, FieldInfo, QueryResponse,
};
use datafusion_postgres::pgwire::error::PgWireResult;

use super::pg_class;
use super::sql_parse::parse_first_u32;

pub(super) async fn column_listing_response(
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
    if let Some(table_name) = pg_class::parse_regclass_table(sql) {
        return Some(table_name);
    }
    let table_oid = parse_attrelid(sql)?;
    pg_class::lookup_synthetic_table_oid(table_oid)
        .or_else(|| pg_class::table_name_for_synthetic_oid(session_context, table_oid))
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
            .filter(|field| !crate::ducklake_sql::layout::is_layout_column(field.name()))
            .map(|field| {
                let (typname, attlen, format_type) = ogr_type_for_arrow_field(field);
                ogr_attribute_row(field.name(), typname, attlen, format_type)
            })
            .collect(),
    )
}

fn ogr_type_for_arrow_field(field: &Field) -> (&'static str, i16, &'static str) {
    match classify_spatial_field(field) {
        Some(SpatialFamily::Geometry) => return ("geometry", -1, "geometry"),
        Some(SpatialFamily::Geography) => return ("geography", -1, "geography"),
        None => {}
    }
    match field.data_type() {
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
