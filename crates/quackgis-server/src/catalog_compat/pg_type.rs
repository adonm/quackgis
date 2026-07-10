// SPDX-License-Identifier: Apache-2.0
//! `pg_type` and PostGIS type metadata compatibility.

use std::sync::Arc;

use datafusion::arrow::datatypes::{DataType, Field};
use datafusion::common::ParamValues;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion::sql::sqlparser::ast::Statement;
use datafusion_postgres::arrow_pg::datatypes::{
    GEOGRAPHY_OID, GEOMETRY_OID, SpatialFamily, classify_spatial_field,
};
use datafusion_postgres::hooks::HookClient;
use datafusion_postgres::pgwire::api::Type;
use datafusion_postgres::pgwire::api::results::{
    DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response,
};
use datafusion_postgres::pgwire::error::{PgWireError, PgWireResult};

use super::encoding::{
    current_portal_result_format, encode_bool_field, encode_char_field, encode_u32_field,
};
use super::params::first_oid_param;
use super::surfaces::{is_pgjdbc_typeinfo_name_query, is_pgjdbc_typeinfo_sqltype_query};

pub(super) fn extended_info_response(
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

pub(super) async fn pgjdbc_typeinfo_sqltype_logical_plan(
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

pub(super) async fn pgjdbc_typeinfo_name_logical_plan(
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

pub(super) async fn oid_typname_logical_plan(
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

pub(super) fn oid_in_response(sql: &str) -> Option<PgWireResult<QueryResponse>> {
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

pub(super) fn oid_typname_probe_response(sql: &str) -> PgWireResult<QueryResponse> {
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

pub(super) struct PgTypeInfo {
    pub(super) oid: u32,
    pub(super) attlen: i16,
}

pub(super) fn for_arrow_field(field: &Field) -> PgTypeInfo {
    match classify_spatial_field(field) {
        Some(SpatialFamily::Geometry) => {
            return PgTypeInfo {
                oid: GEOMETRY_OID,
                attlen: -1,
            };
        }
        Some(SpatialFamily::Geography) => {
            return PgTypeInfo {
                oid: GEOGRAPHY_OID,
                attlen: -1,
            };
        }
        None => {}
    }

    match field.data_type() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion_postgres::arrow_pg::datatypes::with_spatial_family_metadata;

    #[test]
    fn pgjdbc_field_mapping_prefers_explicit_family_and_rejects_text_name_fallback() {
        let location = with_spatial_family_metadata(
            Field::new("location", DataType::Binary, true),
            Some(SpatialFamily::Geometry),
        );
        let earth = with_spatial_family_metadata(
            Field::new("earth", DataType::Binary, true),
            Some(SpatialFamily::Geography),
        );
        let text_geom = Field::new("geom", DataType::Utf8, true);

        assert_eq!(for_arrow_field(&location).oid, GEOMETRY_OID);
        assert_eq!(for_arrow_field(&earth).oid, GEOGRAPHY_OID);
        assert_eq!(for_arrow_field(&text_geom).oid, Type::TEXT.oid());
    }
}
