// SPDX-License-Identifier: Apache-2.0
//! pgwire row encoding helpers shared by catalog compatibility surfaces.

use std::sync::Arc;

use datafusion_postgres::hooks::HookClient;
use datafusion_postgres::pgwire::api::results::{
    DataRowEncoder, FieldFormat, FieldInfo, QueryResponse,
};
use datafusion_postgres::pgwire::api::store::PortalStore;
use datafusion_postgres::pgwire::api::{DEFAULT_NAME, Type};
use datafusion_postgres::pgwire::error::PgWireResult;

pub(super) fn current_portal_result_format(client: &dyn HookClient) -> FieldFormat {
    current_portal_field_format(client, 0)
}

pub(super) fn current_portal_field_format(client: &dyn HookClient, idx: usize) -> FieldFormat {
    client
        .portal_store()
        .get_portal(DEFAULT_NAME)
        .map(|portal| portal.result_column_format.format_for(idx))
        .unwrap_or(FieldFormat::Text)
}

pub(super) fn encode_i16_field(
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

pub(super) fn encode_i32_field(
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

pub(super) fn encode_u32_field(
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

pub(super) fn encode_bool_field(
    encoder: &mut DataRowEncoder,
    value: bool,
    field_format: FieldFormat,
) -> PgWireResult<()> {
    match field_format {
        FieldFormat::Text => encoder.encode_field(&Some(if value { "t" } else { "f" })),
        FieldFormat::Binary => encoder.encode_field(&Some(value)),
    }
}

pub(super) fn encode_char_field(
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

pub(super) fn encode_numeric_i64_field(
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

pub(super) fn empty_response(name: &str, ty: Type) -> PgWireResult<QueryResponse> {
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

pub(super) fn single_bool_row(name: &str, value: bool) -> PgWireResult<QueryResponse> {
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

pub(super) fn single_i64_row(name: &str, value: i64) -> PgWireResult<QueryResponse> {
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

pub(super) fn single_oid_row(name: &str, value: u32) -> PgWireResult<QueryResponse> {
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

pub(super) fn single_attname_attnotnull_row(
    attname: &str,
    attnotnull: bool,
) -> PgWireResult<QueryResponse> {
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

pub(super) fn single_text_row(name: &str, value: &str) -> PgWireResult<QueryResponse> {
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
