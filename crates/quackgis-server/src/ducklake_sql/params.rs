// SPDX-License-Identifier: Apache-2.0
//! Extended-protocol parameter literalization for DuckLake DML routing.

use anyhow::anyhow;
use datafusion::common::{ParamValues, ScalarValue};
use datafusion_postgres::pgwire::error::{PgWireError, PgWireResult};

use super::rewrites::{
    decode_pg_escape_bytea_body, hex_encode, repair_latin1_decoded_utf8_mojibake,
};

pub(super) fn inline_params_if_needed(
    sql: &str,
    params: Option<&ParamValues>,
) -> PgWireResult<String> {
    let Some(params) = params else {
        return Ok(sql.to_string());
    };
    inline_params(sql, params)
}

fn inline_params(sql: &str, params: &ParamValues) -> PgWireResult<String> {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                out.push('\'');
                i += 1;
                while i < bytes.len() {
                    out.push(bytes[i] as char);
                    if bytes[i] == b'\'' {
                        if bytes.get(i + 1) == Some(&b'\'') {
                            i += 1;
                            out.push('\'');
                        } else {
                            i += 1;
                            break;
                        }
                    }
                    i += 1;
                }
            }
            b'"' => {
                out.push('"');
                i += 1;
                while i < bytes.len() {
                    out.push(bytes[i] as char);
                    if bytes[i] == b'"' {
                        if bytes.get(i + 1) == Some(&b'"') {
                            i += 1;
                            out.push('"');
                        } else {
                            i += 1;
                            break;
                        }
                    }
                    i += 1;
                }
            }
            b'$' if bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                let start = i;
                i += 2;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let placeholder = &sql[start..i];
                out.push_str(&param_sql_literal(params, placeholder)?);
            }
            b => {
                out.push(b as char);
                i += 1;
            }
        }
    }
    Ok(out)
}

fn param_sql_literal(params: &ParamValues, placeholder: &str) -> PgWireResult<String> {
    let Ok(value) = params.get_placeholders_with_values(placeholder) else {
        // datafusion-postgres currently drops UNKNOWN-typed NULL parameters
        // during deserialization. QGIS sends the synthetic rowid as a NULL
        // placeholder on INSERT; keep the DML hook fail-closed for storage by
        // materializing that missing bind as SQL NULL and then ignoring the
        // synthetic rowid target column below.
        return Ok("NULL".to_string());
    };
    scalar_sql_literal(&value.value)
}

fn scalar_sql_literal(value: &ScalarValue) -> PgWireResult<String> {
    let literal = match value {
        ScalarValue::Null
        | ScalarValue::Boolean(None)
        | ScalarValue::Float16(None)
        | ScalarValue::Float32(None)
        | ScalarValue::Float64(None)
        | ScalarValue::Decimal32(None, _, _)
        | ScalarValue::Decimal64(None, _, _)
        | ScalarValue::Decimal128(None, _, _)
        | ScalarValue::Decimal256(None, _, _)
        | ScalarValue::Int8(None)
        | ScalarValue::Int16(None)
        | ScalarValue::Int32(None)
        | ScalarValue::Int64(None)
        | ScalarValue::UInt8(None)
        | ScalarValue::UInt16(None)
        | ScalarValue::UInt32(None)
        | ScalarValue::UInt64(None)
        | ScalarValue::Utf8(None)
        | ScalarValue::Utf8View(None)
        | ScalarValue::LargeUtf8(None)
        | ScalarValue::Binary(None)
        | ScalarValue::BinaryView(None)
        | ScalarValue::FixedSizeBinary(_, None)
        | ScalarValue::LargeBinary(None) => "NULL".to_string(),
        ScalarValue::Boolean(Some(value)) => value.to_string(),
        ScalarValue::Float16(Some(value)) => value.to_string(),
        ScalarValue::Float32(Some(value)) => value.to_string(),
        ScalarValue::Float64(Some(value)) => value.to_string(),
        ScalarValue::Decimal32(Some(value), _, scale) => decimal_literal(*value as i128, *scale),
        ScalarValue::Decimal64(Some(value), _, scale) => decimal_literal(*value as i128, *scale),
        ScalarValue::Decimal128(Some(value), _, scale) => decimal_literal(*value, *scale),
        ScalarValue::Decimal256(Some(_), _, _) => {
            return Err(user_error(anyhow!(
                "Decimal256 query parameters are not supported by DuckLake DML routing"
            )));
        }
        ScalarValue::Int8(Some(value)) => value.to_string(),
        ScalarValue::Int16(Some(value)) => value.to_string(),
        ScalarValue::Int32(Some(value)) => value.to_string(),
        ScalarValue::Int64(Some(value)) => value.to_string(),
        ScalarValue::UInt8(Some(value)) => value.to_string(),
        ScalarValue::UInt16(Some(value)) => value.to_string(),
        ScalarValue::UInt32(Some(value)) => value.to_string(),
        ScalarValue::UInt64(Some(value)) => value.to_string(),
        ScalarValue::Utf8(Some(value))
        | ScalarValue::Utf8View(Some(value))
        | ScalarValue::LargeUtf8(Some(value)) => string_or_bytea_literal(value),
        ScalarValue::Binary(Some(value))
        | ScalarValue::BinaryView(Some(value))
        | ScalarValue::FixedSizeBinary(_, Some(value))
        | ScalarValue::LargeBinary(Some(value)) => binary_literal(value),
        other => {
            return Err(user_error(anyhow!(
                "unsupported query parameter for DuckLake DML routing: {other:?}"
            )));
        }
    };
    Ok(literal)
}

fn string_or_bytea_literal(value: &str) -> String {
    if let Some(bytes) = decode_pg_escape_bytea_body(value) {
        return binary_literal(&bytes);
    }
    let repaired = repair_latin1_decoded_utf8_mojibake(value);
    let value = repaired.as_deref().unwrap_or(value);
    format!("'{}'", value.replace('\'', "''"))
}

fn binary_literal(value: &[u8]) -> String {
    format!("X'{}'", hex_encode(value))
}

fn decimal_literal(value: i128, scale: i8) -> String {
    if scale <= 0 {
        return value.to_string();
    }
    let scale = scale as usize;
    let sign = if value < 0 { "-" } else { "" };
    let digits = value.abs().to_string();
    if digits.len() <= scale {
        format!("{sign}0.{}{}", "0".repeat(scale - digits.len()), digits)
    } else {
        let split = digits.len() - scale;
        format!("{sign}{}.{}", &digits[..split], &digits[split..])
    }
}

fn user_error(err: anyhow::Error) -> PgWireError {
    PgWireError::UserError(Box::new(
        datafusion_postgres::pgwire::error::ErrorInfo::new(
            "ERROR".to_string(),
            "22023".to_string(),
            err.to_string(),
        ),
    ))
}
