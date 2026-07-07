// SPDX-License-Identifier: Apache-2.0
//! Extended-query parameter helpers for catalog compatibility shims.

use datafusion::common::{ParamValues, ScalarValue};

pub(super) fn first_oid_param(params: &ParamValues) -> Option<u32> {
    let value = match params {
        ParamValues::List(values) => values.first()?.value.clone(),
        ParamValues::Map(values) => values.get("$1").or_else(|| values.get("1"))?.value.clone(),
    };
    match value {
        ScalarValue::UInt32(Some(value)) => Some(value),
        ScalarValue::Int32(Some(value)) if value >= 0 => Some(value as u32),
        ScalarValue::Int64(Some(value)) if value >= 0 => u32::try_from(value).ok(),
        _ => None,
    }
}

pub(super) fn last_string_param(params: &ParamValues) -> Option<String> {
    match params {
        ParamValues::List(values) => values
            .iter()
            .rev()
            .find_map(|value| scalar_string(&value.value)),
        ParamValues::Map(values) => (1..=values.len()).rev().find_map(|idx| {
            values
                .get(&format!("${idx}"))
                .or_else(|| values.get(&idx.to_string()))
                .and_then(|value| scalar_string(&value.value))
        }),
    }
}

pub(super) fn string_param(params: &ParamValues, idx: usize) -> Option<String> {
    match params {
        ParamValues::List(values) => values
            .get(idx.checked_sub(1)?)
            .and_then(|value| scalar_string(&value.value)),
        ParamValues::Map(values) => values
            .get(&format!("${idx}"))
            .or_else(|| values.get(&idx.to_string()))
            .and_then(|value| scalar_string(&value.value)),
    }
}

fn scalar_string(value: &ScalarValue) -> Option<String> {
    match value {
        ScalarValue::Utf8(Some(value)) => Some(value.clone()),
        ScalarValue::LargeUtf8(Some(value)) => Some(value.clone()),
        ScalarValue::Utf8View(Some(value)) => Some(value.clone()),
        _ => None,
    }
}
