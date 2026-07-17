use std::str::FromStr;
use std::sync::Arc;

use arrow::{array::*, datatypes::*};
use bytes::{BufMut, BytesMut};
use chrono::NaiveTime;
use chrono::{NaiveDate, NaiveDateTime};
use pg_interval::Interval as PgInterval;
use pgwire::api::results::{CopyEncoder, DataRowEncoder, FieldInfo};
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use pgwire::messages::copy::CopyData;
use pgwire::messages::data::DataRow;
use pgwire::types::ToSqlText;
use pgwire::types::format::FormatOptions;
use postgres_types::{IsNull, Json, ToSql, Type};
use rust_decimal::Decimal;
use timezone::Tz;

use crate::datatypes::{GEOGRAPHY_OID, GEOMETRY_OID};
use crate::error::ToSqlError;
use crate::list_encoder::encode_list;
use crate::struct_encoder::encode_struct;

fn is_geometry_wire_type(ty: &Type) -> bool {
    matches!(ty.oid(), GEOMETRY_OID | GEOGRAPHY_OID)
}

fn is_reg_oid_type(ty: &Type) -> bool {
    matches!(
        *ty,
        Type::REGCLASS | Type::REGTYPE | Type::REGNAMESPACE | Type::REGROLE
    )
}

#[derive(Debug)]
struct PgRegOid(u32);

impl ToSql for PgRegOid {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        out.put_u32(self.0);
        Ok(IsNull::No)
    }

    fn accepts(ty: &Type) -> bool {
        is_reg_oid_type(ty)
    }

    postgres_types::to_sql_checked!();
}

impl ToSqlText for PgRegOid {
    fn to_sql_text(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
        _format_options: &FormatOptions,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        out.extend_from_slice(self.0.to_string().as_bytes());
        Ok(IsNull::No)
    }
}

#[derive(Debug)]
struct PgSpatialWkb<'a>(Option<&'a [u8]>);

impl ToSql for PgSpatialWkb<'_> {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        let Some(value) = self.0 else {
            return Ok(IsNull::Yes);
        };
        out.extend_from_slice(value);
        Ok(IsNull::No)
    }

    fn accepts(ty: &Type) -> bool {
        is_geometry_wire_type(ty)
    }

    postgres_types::to_sql_checked!();
}

impl ToSqlText for PgSpatialWkb<'_> {
    fn to_sql_text(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
        _format_options: &FormatOptions,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        let Some(value) = self.0 else {
            return Ok(IsNull::Yes);
        };
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        out.reserve(value.len().saturating_mul(2));
        for byte in value {
            out.extend_from_slice(&[HEX[(byte >> 4) as usize], HEX[(byte & 0x0f) as usize]]);
        }
        Ok(IsNull::No)
    }
}

fn encode_binary_as_geometry(
    encoder: &mut impl Encoder,
    value: Option<&[u8]>,
    pg_field: &FieldInfo,
) -> PgWireResult<()> {
    encoder.encode_field(&PgSpatialWkb(value), pg_field)
}

fn encode_binary_as_bytea(
    encoder: &mut impl Encoder,
    value: Option<&[u8]>,
    pg_field: &FieldInfo,
) -> PgWireResult<()> {
    encoder.encode_field_with_type(&value, &Type::BYTEA, pg_field)
}

fn pg_char_from_str(value: Option<&str>) -> Option<i8> {
    value.map(|value| value.as_bytes().first().copied().unwrap_or_default() as i8)
}

#[derive(Debug)]
struct PgChar(Option<i8>);

impl ToSql for PgChar {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        let Some(value) = self.0 else {
            return Ok(IsNull::Yes);
        };
        out.extend_from_slice(&[value as u8]);
        Ok(IsNull::No)
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::CHAR
    }

    postgres_types::to_sql_checked!();
}

impl ToSqlText for PgChar {
    fn to_sql_text(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
        _format_options: &FormatOptions,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        let Some(value) = self.0 else {
            return Ok(IsNull::Yes);
        };
        if value != 0 {
            out.extend_from_slice(&[value as u8]);
        }
        Ok(IsNull::No)
    }
}

pub trait Encoder {
    type Item;

    fn encode_field<T>(&mut self, value: &T, pg_field: &FieldInfo) -> PgWireResult<()>
    where
        T: ToSql + ToSqlText + Sized;

    fn encode_field_with_type<T>(
        &mut self,
        value: &T,
        data_type: &Type,
        pg_field: &FieldInfo,
    ) -> PgWireResult<()>
    where
        T: ToSql + ToSqlText + Sized,
    {
        let _ = data_type;
        self.encode_field(value, pg_field)
    }

    fn take_row(&mut self) -> Self::Item;
}

impl Encoder for DataRowEncoder {
    type Item = DataRow;

    fn encode_field<T>(&mut self, value: &T, pg_field: &FieldInfo) -> PgWireResult<()>
    where
        T: ToSql + ToSqlText + Sized,
    {
        self.encode_field_with_type_and_format(
            value,
            pg_field.datatype(),
            pg_field.format(),
            pg_field.format_options(),
        )
    }

    fn encode_field_with_type<T>(
        &mut self,
        value: &T,
        data_type: &Type,
        pg_field: &FieldInfo,
    ) -> PgWireResult<()>
    where
        T: ToSql + ToSqlText + Sized,
    {
        self.encode_field_with_type_and_format(
            value,
            data_type,
            pg_field.format(),
            pg_field.format_options(),
        )
    }

    fn take_row(&mut self) -> Self::Item {
        self.take_row()
    }
}

impl Encoder for CopyEncoder {
    type Item = CopyData;

    fn encode_field<T>(&mut self, value: &T, _pg_field: &FieldInfo) -> PgWireResult<()>
    where
        T: ToSql + ToSqlText + Sized,
    {
        self.encode_field(value)
    }

    fn take_row(&mut self) -> Self::Item {
        self.take_copy()
    }
}

fn get_bool_value(arr: &Arc<dyn Array>, idx: usize) -> Option<bool> {
    (!arr.is_null(idx)).then(|| {
        arr.as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap()
            .value(idx)
    })
}

macro_rules! get_primitive_value {
    ($name:ident, $t:ty, $pt:ty) => {
        fn $name(arr: &Arc<dyn Array>, idx: usize) -> Option<$pt> {
            (!arr.is_null(idx)).then(|| {
                arr.as_any()
                    .downcast_ref::<PrimitiveArray<$t>>()
                    .unwrap()
                    .value(idx)
            })
        }
    };
}

get_primitive_value!(get_i8_value, Int8Type, i8);
get_primitive_value!(get_i16_value, Int16Type, i16);
get_primitive_value!(get_i32_value, Int32Type, i32);
get_primitive_value!(get_i64_value, Int64Type, i64);
get_primitive_value!(get_u8_value, UInt8Type, u8);
get_primitive_value!(get_u16_value, UInt16Type, u16);
get_primitive_value!(get_u32_value, UInt32Type, u32);
get_primitive_value!(get_u64_value, UInt64Type, u64);

fn get_u64_as_decimal_value(arr: &Arc<dyn Array>, idx: usize) -> Option<Decimal> {
    get_u64_value(arr, idx).map(Decimal::from)
}
fn get_f16_value(arr: &Arc<dyn Array>, idx: usize) -> Option<f32> {
    (!arr.is_null(idx)).then(|| {
        arr.as_any()
            .downcast_ref::<Float16Array>()
            .expect("Arrow field and array type must agree")
            .value(idx)
            .to_f32()
    })
}
get_primitive_value!(get_f32_value, Float32Type, f32);
get_primitive_value!(get_f64_value, Float64Type, f64);

fn get_utf8_view_value(arr: &Arc<dyn Array>, idx: usize) -> Option<&str> {
    (!arr.is_null(idx)).then(|| {
        arr.as_any()
            .downcast_ref::<StringViewArray>()
            .unwrap()
            .value(idx)
    })
}

fn get_binary_view_value(arr: &Arc<dyn Array>, idx: usize) -> Option<&[u8]> {
    (!arr.is_null(idx)).then(|| {
        arr.as_any()
            .downcast_ref::<BinaryViewArray>()
            .unwrap()
            .value(idx)
    })
}

fn get_utf8_value(arr: &Arc<dyn Array>, idx: usize) -> Option<&str> {
    (!arr.is_null(idx)).then(|| {
        arr.as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(idx)
    })
}

fn get_large_utf8_value(arr: &Arc<dyn Array>, idx: usize) -> Option<&str> {
    (!arr.is_null(idx)).then(|| {
        arr.as_any()
            .downcast_ref::<LargeStringArray>()
            .unwrap()
            .value(idx)
    })
}

fn get_binary_value(arr: &Arc<dyn Array>, idx: usize) -> Option<&[u8]> {
    (!arr.is_null(idx)).then(|| {
        arr.as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .value(idx)
    })
}

fn get_large_binary_value(arr: &Arc<dyn Array>, idx: usize) -> Option<&[u8]> {
    (!arr.is_null(idx)).then(|| {
        arr.as_any()
            .downcast_ref::<LargeBinaryArray>()
            .unwrap()
            .value(idx)
    })
}

fn get_fixed_size_binary_value(arr: &Arc<dyn Array>, idx: usize) -> Option<&[u8]> {
    (!arr.is_null(idx)).then(|| {
        arr.as_any()
            .downcast_ref::<FixedSizeBinaryArray>()
            .expect("Arrow field and array type must agree")
            .value(idx)
    })
}

fn parse_json_value(value: Option<&str>) -> PgWireResult<Option<Json<serde_json::Value>>> {
    value
        .map(|value| {
            serde_json::from_str(value).map(Json).map_err(|error| {
                PgWireError::UserError(Box::new(ErrorInfo::new(
                    "ERROR".to_owned(),
                    "22P02".to_owned(),
                    format!("invalid JSON value: {error}"),
                )))
            })
        })
        .transpose()
}

fn get_date32_value(arr: &Arc<dyn Array>, idx: usize) -> Option<NaiveDate> {
    if arr.is_null(idx) {
        return None;
    }
    arr.as_any()
        .downcast_ref::<Date32Array>()
        .unwrap()
        .value_as_date(idx)
}

fn get_date64_value(arr: &Arc<dyn Array>, idx: usize) -> Option<NaiveDate> {
    if arr.is_null(idx) {
        return None;
    }
    arr.as_any()
        .downcast_ref::<Date64Array>()
        .unwrap()
        .value_as_date(idx)
}

fn get_time32_second_value(arr: &Arc<dyn Array>, idx: usize) -> Option<NaiveTime> {
    if arr.is_null(idx) {
        return None;
    }
    arr.as_any()
        .downcast_ref::<Time32SecondArray>()
        .unwrap()
        .value_as_time(idx)
}

fn get_time32_millisecond_value(arr: &Arc<dyn Array>, idx: usize) -> Option<NaiveTime> {
    if arr.is_null(idx) {
        return None;
    }
    arr.as_any()
        .downcast_ref::<Time32MillisecondArray>()
        .unwrap()
        .value_as_time(idx)
}

fn get_time64_microsecond_value(arr: &Arc<dyn Array>, idx: usize) -> Option<NaiveTime> {
    if arr.is_null(idx) {
        return None;
    }
    arr.as_any()
        .downcast_ref::<Time64MicrosecondArray>()
        .unwrap()
        .value_as_time(idx)
}
fn get_time64_nanosecond_value(arr: &Arc<dyn Array>, idx: usize) -> Option<NaiveTime> {
    if arr.is_null(idx) {
        return None;
    }
    arr.as_any()
        .downcast_ref::<Time64NanosecondArray>()
        .unwrap()
        .value_as_time(idx)
}

fn get_numeric_128_value(
    arr: &Arc<dyn Array>,
    idx: usize,
    scale: u32,
) -> PgWireResult<Option<Decimal>> {
    if arr.is_null(idx) {
        return Ok(None);
    }

    let array = arr.as_any().downcast_ref::<Decimal128Array>().unwrap();
    let value = array.value(idx);
    Decimal::try_from_i128_with_scale(value, scale)
        .map_err(|e| {
            let error_code = match e {
                rust_decimal::Error::ExceedsMaximumPossibleValue => {
                    "22003" // numeric_value_out_of_range
                }
                rust_decimal::Error::LessThanMinimumPossibleValue => {
                    "22003" // numeric_value_out_of_range
                }
                rust_decimal::Error::ScaleExceedsMaximumPrecision(scale) => {
                    return PgWireError::UserError(Box::new(ErrorInfo::new(
                        "ERROR".to_string(),
                        "22003".to_string(),
                        format!("Scale {scale} exceeds maximum precision for numeric type"),
                    )));
                }
                _ => "22003", // generic numeric_value_out_of_range
            };
            PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".to_string(),
                error_code.to_string(),
                format!("Numeric value conversion failed: {e}"),
            )))
        })
        .map(Some)
}

pub fn encode_value<T: Encoder>(
    encoder: &mut T,
    arr: &Arc<dyn Array>,
    idx: usize,
    arrow_field: &Field,
    pg_field: &FieldInfo,
) -> PgWireResult<()> {
    let arrow_type = arrow_field.data_type();

    match arrow_type {
        DataType::Null => encoder.encode_field(&None::<i8>, pg_field)?,
        DataType::Boolean => encoder.encode_field(&get_bool_value(arr, idx), pg_field)?,
        DataType::Int8 => encoder.encode_field(&get_i8_value(arr, idx), pg_field)?,
        DataType::Int16 => encoder.encode_field(&get_i16_value(arr, idx), pg_field)?,
        DataType::Int32 if is_reg_oid_type(pg_field.datatype()) => encoder.encode_field(
            &get_i32_value(arr, idx).map(|value| PgRegOid(value as u32)),
            pg_field,
        )?,
        DataType::Int32 if *pg_field.datatype() == Type::OID => {
            encoder.encode_field(&get_i32_value(arr, idx).map(|value| value as u32), pg_field)?
        }
        DataType::Int32 => encoder.encode_field(&get_i32_value(arr, idx), pg_field)?,
        DataType::Int64 => encoder.encode_field(&get_i64_value(arr, idx), pg_field)?,
        DataType::UInt8 => {
            encoder.encode_field(&(get_u8_value(arr, idx).map(|x| x as i16)), pg_field)?
        }
        DataType::UInt16 => {
            encoder.encode_field(&(get_u16_value(arr, idx).map(|x| x as i32)), pg_field)?
        }
        DataType::UInt32 if is_reg_oid_type(pg_field.datatype()) => {
            encoder.encode_field(&get_u32_value(arr, idx).map(PgRegOid), pg_field)?
        }
        DataType::UInt32 if *pg_field.datatype() == Type::OID => {
            encoder.encode_field(&get_u32_value(arr, idx), pg_field)?
        }
        DataType::UInt32 => {
            encoder.encode_field(&get_u32_value(arr, idx).map(|x| x as i64), pg_field)?
        }
        DataType::UInt64 => encoder.encode_field(&get_u64_as_decimal_value(arr, idx), pg_field)?,
        DataType::Float16 => encoder.encode_field(&get_f16_value(arr, idx), pg_field)?,
        DataType::Float32 => encoder.encode_field(&get_f32_value(arr, idx), pg_field)?,
        DataType::Float64 => encoder.encode_field(&get_f64_value(arr, idx), pg_field)?,
        DataType::Decimal128(_, s) => {
            encoder.encode_field(&get_numeric_128_value(arr, idx, *s as u32)?, pg_field)?
        }
        DataType::Utf8 if *pg_field.datatype() == Type::CHAR => encoder.encode_field(
            &PgChar(pg_char_from_str(get_utf8_value(arr, idx))),
            pg_field,
        )?,
        DataType::Utf8
            if *pg_field.datatype() == Type::JSONB || *pg_field.datatype() == Type::JSON =>
        {
            encoder.encode_field(&parse_json_value(get_utf8_value(arr, idx))?, pg_field)?
        }
        DataType::Utf8 => encoder.encode_field(&get_utf8_value(arr, idx), pg_field)?,
        DataType::Utf8View if *pg_field.datatype() == Type::CHAR => encoder.encode_field(
            &PgChar(pg_char_from_str(get_utf8_view_value(arr, idx))),
            pg_field,
        )?,
        DataType::Utf8View
            if *pg_field.datatype() == Type::JSONB || *pg_field.datatype() == Type::JSON =>
        {
            encoder.encode_field(&parse_json_value(get_utf8_view_value(arr, idx))?, pg_field)?
        }
        DataType::Utf8View => encoder.encode_field(&get_utf8_view_value(arr, idx), pg_field)?,
        DataType::BinaryView if is_geometry_wire_type(pg_field.datatype()) => {
            encode_binary_as_geometry(encoder, get_binary_view_value(arr, idx), pg_field)?
        }
        DataType::BinaryView => encoder.encode_field(&get_binary_view_value(arr, idx), pg_field)?,
        DataType::LargeUtf8 if *pg_field.datatype() == Type::CHAR => {
            encoder.encode_field(&pg_char_from_str(get_large_utf8_value(arr, idx)), pg_field)?
        }
        DataType::LargeUtf8
            if *pg_field.datatype() == Type::JSONB || *pg_field.datatype() == Type::JSON =>
        {
            encoder.encode_field(&parse_json_value(get_large_utf8_value(arr, idx))?, pg_field)?
        }
        DataType::LargeUtf8 => encoder.encode_field(&get_large_utf8_value(arr, idx), pg_field)?,
        DataType::Binary if is_geometry_wire_type(pg_field.datatype()) => {
            encode_binary_as_geometry(encoder, get_binary_value(arr, idx), pg_field)?
        }
        DataType::Binary => encoder.encode_field(&get_binary_value(arr, idx), pg_field)?,
        DataType::LargeBinary if is_geometry_wire_type(pg_field.datatype()) => {
            encode_binary_as_geometry(encoder, get_large_binary_value(arr, idx), pg_field)?
        }
        DataType::LargeBinary => {
            encoder.encode_field(&get_large_binary_value(arr, idx), pg_field)?
        }
        DataType::FixedSizeBinary(_) => {
            encode_binary_as_bytea(encoder, get_fixed_size_binary_value(arr, idx), pg_field)?
        }
        DataType::Date32 => encoder.encode_field(&get_date32_value(arr, idx), pg_field)?,
        DataType::Date64 => encoder.encode_field(&get_date64_value(arr, idx), pg_field)?,
        DataType::Time32(unit) => match unit {
            TimeUnit::Second => {
                encoder.encode_field(&get_time32_second_value(arr, idx), pg_field)?
            }
            TimeUnit::Millisecond => {
                encoder.encode_field(&get_time32_millisecond_value(arr, idx), pg_field)?
            }
            unsupported => {
                return Err(PgWireError::ApiError(ToSqlError::from(format!(
                    "Unsupported Time32 unit {unsupported:?}"
                ))));
            }
        },
        DataType::Time64(unit) => match unit {
            TimeUnit::Microsecond => {
                encoder.encode_field(&get_time64_microsecond_value(arr, idx), pg_field)?
            }
            TimeUnit::Nanosecond => {
                encoder.encode_field(&get_time64_nanosecond_value(arr, idx), pg_field)?
            }
            unsupported => {
                return Err(PgWireError::ApiError(ToSqlError::from(format!(
                    "Unsupported Time64 unit {unsupported:?}"
                ))));
            }
        },
        DataType::Timestamp(unit, timezone) => match unit {
            TimeUnit::Second => {
                if arr.is_null(idx) {
                    return encoder.encode_field(&None::<NaiveDateTime>, pg_field);
                }
                let ts_array = arr.as_any().downcast_ref::<TimestampSecondArray>().unwrap();
                if let Some(tz) = timezone {
                    let tz = Tz::from_str(tz.as_ref()).map_err(ToSqlError::from)?;
                    let value = ts_array
                        .value_as_datetime_with_tz(idx, tz)
                        .map(|d| d.fixed_offset());

                    encoder.encode_field(&value, pg_field)?;
                } else {
                    let value = ts_array.value_as_datetime(idx);
                    encoder.encode_field(&value, pg_field)?;
                }
            }
            TimeUnit::Millisecond => {
                if arr.is_null(idx) {
                    return encoder.encode_field(&None::<NaiveDateTime>, pg_field);
                }
                let ts_array = arr
                    .as_any()
                    .downcast_ref::<TimestampMillisecondArray>()
                    .unwrap();
                if let Some(tz) = timezone {
                    let tz = Tz::from_str(tz.as_ref()).map_err(ToSqlError::from)?;
                    let value = ts_array
                        .value_as_datetime_with_tz(idx, tz)
                        .map(|d| d.fixed_offset());
                    encoder.encode_field(&value, pg_field)?;
                } else {
                    let value = ts_array.value_as_datetime(idx);
                    encoder.encode_field(&value, pg_field)?;
                }
            }
            TimeUnit::Microsecond => {
                if arr.is_null(idx) {
                    return encoder.encode_field(&None::<NaiveDateTime>, pg_field);
                }
                let ts_array = arr
                    .as_any()
                    .downcast_ref::<TimestampMicrosecondArray>()
                    .unwrap();
                if let Some(tz) = timezone {
                    let tz = Tz::from_str(tz.as_ref()).map_err(ToSqlError::from)?;
                    let value = ts_array
                        .value_as_datetime_with_tz(idx, tz)
                        .map(|d| d.fixed_offset());
                    encoder.encode_field(&value, pg_field)?;
                } else {
                    let value = ts_array.value_as_datetime(idx);
                    encoder.encode_field(&value, pg_field)?;
                }
            }
            TimeUnit::Nanosecond => {
                if arr.is_null(idx) {
                    return encoder.encode_field(&None::<NaiveDateTime>, pg_field);
                }
                let ts_array = arr
                    .as_any()
                    .downcast_ref::<TimestampNanosecondArray>()
                    .unwrap();
                if let Some(tz) = timezone {
                    let tz = Tz::from_str(tz.as_ref()).map_err(ToSqlError::from)?;
                    let value = ts_array
                        .value_as_datetime_with_tz(idx, tz)
                        .map(|d| d.fixed_offset());
                    encoder.encode_field(&value, pg_field)?;
                } else {
                    let value = ts_array.value_as_datetime(idx);
                    encoder.encode_field(&value, pg_field)?;
                }
            }
        },
        DataType::Interval(interval_unit) => match interval_unit {
            IntervalUnit::YearMonth => {
                if arr.is_null(idx) {
                    return encoder.encode_field(&None::<PgInterval>, pg_field);
                }
                let interval_array = arr
                    .as_any()
                    .downcast_ref::<IntervalYearMonthArray>()
                    .unwrap();
                let months = IntervalYearMonthType::to_months(interval_array.value(idx));
                encoder.encode_field(&PgInterval::new(months, 0, 0), pg_field)?;
            }
            IntervalUnit::DayTime => {
                if arr.is_null(idx) {
                    return encoder.encode_field(&None::<PgInterval>, pg_field);
                }
                let interval_array = arr.as_any().downcast_ref::<IntervalDayTimeArray>().unwrap();
                let (days, millis) = IntervalDayTimeType::to_parts(interval_array.value(idx));
                encoder
                    .encode_field(&PgInterval::new(0, days, millis as i64 * 1000i64), pg_field)?;
            }
            IntervalUnit::MonthDayNano => {
                if arr.is_null(idx) {
                    return encoder.encode_field(&None::<PgInterval>, pg_field);
                }
                let interval_array = arr
                    .as_any()
                    .downcast_ref::<IntervalMonthDayNanoArray>()
                    .unwrap();
                let (months, days, nanoseconds) =
                    IntervalMonthDayNanoType::to_parts(interval_array.value(idx));

                encoder.encode_field(
                    &PgInterval::new(months, days, nanoseconds / 1000i64),
                    pg_field,
                )?;
            }
        },
        DataType::Duration(unit) => match unit {
            TimeUnit::Second => {
                if arr.is_null(idx) {
                    return encoder.encode_field(&None::<PgInterval>, pg_field);
                }
                let duration_array = arr.as_any().downcast_ref::<DurationSecondArray>().unwrap();
                let microseconds = duration_array.value(idx) * 1_000_000i64;
                encoder.encode_field(&PgInterval::new(0, 0, microseconds), pg_field)?;
            }
            TimeUnit::Millisecond => {
                if arr.is_null(idx) {
                    return encoder.encode_field(&None::<PgInterval>, pg_field);
                }
                let duration_array = arr
                    .as_any()
                    .downcast_ref::<DurationMillisecondArray>()
                    .unwrap();
                let microseconds = duration_array.value(idx) * 1_000i64;
                encoder.encode_field(&PgInterval::new(0, 0, microseconds), pg_field)?;
            }
            TimeUnit::Microsecond => {
                if arr.is_null(idx) {
                    return encoder.encode_field(&None::<PgInterval>, pg_field);
                }
                let duration_array = arr
                    .as_any()
                    .downcast_ref::<DurationMicrosecondArray>()
                    .unwrap();
                let microseconds = duration_array.value(idx);
                encoder.encode_field(&PgInterval::new(0, 0, microseconds), pg_field)?;
            }
            TimeUnit::Nanosecond => {
                if arr.is_null(idx) {
                    return encoder.encode_field(&None::<PgInterval>, pg_field);
                }
                let duration_array = arr
                    .as_any()
                    .downcast_ref::<DurationNanosecondArray>()
                    .unwrap();
                let microseconds = duration_array.value(idx) / 1_000i64;
                encoder.encode_field(&PgInterval::new(0, 0, microseconds), pg_field)?;
            }
        },
        DataType::List(_) => {
            if arr.is_null(idx) {
                return encoder.encode_field(&None::<&[i8]>, pg_field);
            }
            let array = arr
                .as_any()
                .downcast_ref::<ListArray>()
                .expect("Arrow field and array type must agree")
                .value(idx);
            encode_list(encoder, array, pg_field)?
        }
        DataType::LargeList(_) => {
            if arr.is_null(idx) {
                return encoder.encode_field(&None::<&[i8]>, pg_field);
            }
            let array = arr
                .as_any()
                .downcast_ref::<LargeListArray>()
                .expect("Arrow field and array type must agree")
                .value(idx);
            encode_list(encoder, array, pg_field)?
        }
        DataType::Struct(arrow_fields) => encode_struct(encoder, arr, idx, arrow_fields, pg_field)?,
        DataType::Dictionary(_, value_type) => {
            if arr.is_null(idx) {
                return encoder.encode_field(&None::<i8>, pg_field);
            }
            // Get the dictionary values and the mapped row index
            macro_rules! get_dict_values_and_index {
                ($key_type:ty) => {
                    arr.as_any()
                        .downcast_ref::<DictionaryArray<$key_type>>()
                        .map(|dict| (dict.values(), dict.keys().value(idx) as usize))
                };
            }

            // Try to extract values using different key types
            let (values, idx) = get_dict_values_and_index!(Int8Type)
                .or_else(|| get_dict_values_and_index!(Int16Type))
                .or_else(|| get_dict_values_and_index!(Int32Type))
                .or_else(|| get_dict_values_and_index!(Int64Type))
                .or_else(|| get_dict_values_and_index!(UInt8Type))
                .or_else(|| get_dict_values_and_index!(UInt16Type))
                .or_else(|| get_dict_values_and_index!(UInt32Type))
                .or_else(|| get_dict_values_and_index!(UInt64Type))
                .ok_or_else(|| {
                    ToSqlError::from(format!(
                        "Unsupported dictionary key type for value type {value_type}"
                    ))
                })?;

            let inner_arrow_field = Field::new(pg_field.name(), *value_type.clone(), true);

            encode_value(encoder, values, idx, &inner_arrow_field, pg_field)?
        }
        _ => {
            return Err(PgWireError::ApiError(ToSqlError::from(format!(
                "Unsupported Datatype {} and array {:?}",
                arr.data_type(),
                &arr
            ))));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use arrow::buffer::NullBuffer;
    use bytes::BytesMut;
    use pgwire::{api::results::FieldFormat, types::format::FormatOptions};
    use postgres_types::Type;
    use proptest::prelude::*;

    use super::*;

    #[derive(Default)]
    struct TextCaptureEncoder {
        encoded: Vec<Vec<u8>>,
    }

    impl Encoder for TextCaptureEncoder {
        type Item = Vec<Vec<u8>>;

        fn encode_field<T>(&mut self, value: &T, pg_field: &FieldInfo) -> PgWireResult<()>
        where
            T: ToSql + ToSqlText + Sized,
        {
            self.encode_field_with_type(value, pg_field.datatype(), pg_field)
        }

        fn encode_field_with_type<T>(
            &mut self,
            value: &T,
            data_type: &Type,
            _pg_field: &FieldInfo,
        ) -> PgWireResult<()>
        where
            T: ToSql + ToSqlText + Sized,
        {
            let mut bytes = BytesMut::new();
            value
                .to_sql_text(data_type, &mut bytes, &FormatOptions::default())
                .map_err(PgWireError::ApiError)?;
            self.encoded.push(bytes.to_vec());
            Ok(())
        }

        fn take_row(&mut self) -> Self::Item {
            std::mem::take(&mut self.encoded)
        }
    }

    proptest! {
        #[test]
        fn geometry_sentinel_encodes_generated_wkb_as_bare_hex_text(
            values in prop::collection::vec(prop::option::of(prop::collection::vec(any::<u8>(), 0..128)), 0..64)
        ) {
            let array: Arc<dyn Array> = Arc::new(BinaryArray::from_iter(
                values.iter().map(|value| value.as_deref()),
            ));
            let arrow_field = Field::new("geom_wkb", DataType::Binary, true);
            let geometry = FieldInfo::new(
                "geom_wkb".to_owned(),
                None,
                None,
                crate::datatypes::geometry_pg_type(),
                FieldFormat::Text,
            );
            for (index, value) in values.iter().enumerate() {
                let mut geometry_encoder = TextCaptureEncoder::default();
                encode_value(&mut geometry_encoder, &array, index, &arrow_field, &geometry)?;
                let expected = value
                    .as_ref()
                    .map(|value| {
                        value
                            .iter()
                            .flat_map(|byte| {
                                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                                [HEX[(byte >> 4) as usize], HEX[(byte & 0x0f) as usize]]
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                prop_assert_eq!(&geometry_encoder.encoded, &vec![expected]);
            }
        }

        #[test]
        fn generated_fixed_binary_values_encode_without_panics(
            values in prop::collection::vec(prop::option::of(any::<[u8; 4]>()), 0..64)
        ) {
            let array: Arc<dyn Array> = Arc::new(
                FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                    values.iter().map(|value| value.as_ref().map(|value| value.as_slice())),
                    4,
                ).expect("valid generated fixed binary"),
            );
            let arrow_field = Field::new("payload", DataType::FixedSizeBinary(4), true);
            let pg_field = FieldInfo::new(
                "payload".to_owned(), None, None, Type::BYTEA, FieldFormat::Text,
            );
            for index in 0..values.len() {
                let mut encoder = TextCaptureEncoder::default();
                encode_value(&mut encoder, &array, index, &arrow_field, &pg_field)?;
                prop_assert_eq!(encoder.encoded.len(), 1);
            }
        }
    }

    #[test]
    fn spatial_wkb_uses_bare_hex_text_and_raw_binary() {
        let value = PgSpatialWkb(Some(&[0x01, 0xab, 0xff]));
        let geometry = crate::datatypes::geometry_pg_type();

        let mut text = BytesMut::new();
        let text_null = value
            .to_sql_text(&geometry, &mut text, &FormatOptions::default())
            .expect("spatial text");
        assert!(matches!(text_null, IsNull::No));
        assert_eq!(text.as_ref(), b"01ABFF");

        let mut binary = BytesMut::new();
        let binary_null = value
            .to_sql(&geometry, &mut binary)
            .expect("spatial binary");
        assert!(matches!(binary_null, IsNull::No));
        assert_eq!(binary.as_ref(), &[0x01, 0xab, 0xff]);

        let mut null = BytesMut::new();
        let null_status = PgSpatialWkb(None)
            .to_sql_text(&geometry, &mut null, &FormatOptions::default())
            .expect("null spatial text");
        assert!(matches!(null_status, IsNull::Yes));
        assert!(null.is_empty());
    }

    #[test]
    fn empty_postgresql_char_is_nul_not_null() {
        assert_eq!(pg_char_from_str(None), None);
        assert_eq!(pg_char_from_str(Some("")), Some(0));
        assert_eq!(pg_char_from_str(Some("p")), Some(b'p' as i8));

        let mut text = BytesMut::new();
        assert!(matches!(
            PgChar(Some(b'b' as i8))
                .to_sql_text(&Type::CHAR, &mut text, &FormatOptions::default())
                .expect("PostgreSQL char text"),
            IsNull::No
        ));
        assert_eq!(text.as_ref(), b"b");

        let mut empty = BytesMut::new();
        assert!(matches!(
            PgChar(Some(0))
                .to_sql_text(&Type::CHAR, &mut empty, &FormatOptions::default())
                .expect("empty PostgreSQL char text"),
            IsNull::No
        ));
        assert!(empty.is_empty());
    }

    #[test]
    fn invalid_json_fails_instead_of_becoming_json_null() {
        let array: Arc<dyn Array> = Arc::new(StringArray::from(vec![Some("{not json")]));
        let arrow_field = Field::new("properties", DataType::Utf8, true);
        let pg_field = FieldInfo::new(
            "properties".to_owned(),
            None,
            None,
            Type::JSONB,
            FieldFormat::Text,
        );
        let mut encoder = TextCaptureEncoder::default();
        let error = encode_value(&mut encoder, &array, 0, &arrow_field, &pg_field)
            .expect_err("invalid JSON must fail closed");
        assert!(matches!(error, PgWireError::UserError(_)));
        assert!(encoder.encoded.is_empty());
    }

    #[test]
    fn advertised_float16_and_uint32_oid_values_encode() {
        let float_array: Arc<dyn Array> = Arc::new(Float16Array::from(vec![
            Some(half::f16::from_f32(1.5)),
            None,
        ]));
        let float_field = Field::new("value", DataType::Float16, true);
        let float_pg = FieldInfo::new(
            "value".to_owned(),
            None,
            None,
            Type::FLOAT4,
            FieldFormat::Text,
        );
        for index in 0..float_array.len() {
            let mut encoder = TextCaptureEncoder::default();
            encode_value(&mut encoder, &float_array, index, &float_field, &float_pg)
                .expect("Float16 advertised as FLOAT4 must encode");
            assert_eq!(encoder.encoded.len(), 1);
        }

        let oid_array: Arc<dyn Array> = Arc::new(UInt32Array::from(vec![Some(u32::MAX), None]));
        let oid_field = Field::new("oid", DataType::UInt32, true);
        let oid_pg = FieldInfo::new("oid".to_owned(), None, None, Type::OID, FieldFormat::Text);
        for index in 0..oid_array.len() {
            let mut encoder = TextCaptureEncoder::default();
            encode_value(&mut encoder, &oid_array, index, &oid_field, &oid_pg)
                .expect("UInt32 OID alias must encode as OID");
            assert_eq!(encoder.encoded.len(), 1);
        }
        for pg_type in [
            Type::REGCLASS,
            Type::REGTYPE,
            Type::REGNAMESPACE,
            Type::REGROLE,
        ] {
            let pg_field = FieldInfo::new(
                "registered_oid".to_owned(),
                None,
                None,
                pg_type,
                FieldFormat::Text,
            );
            let mut encoder = TextCaptureEncoder::default();
            encode_value(&mut encoder, &oid_array, 0, &oid_field, &pg_field)
                .expect("registered OID type must encode");
            assert_eq!(encoder.encoded.len(), 1);
        }
    }

    #[test]
    fn null_intervals_emit_one_null_field() {
        let array: Arc<dyn Array> = Arc::new(IntervalYearMonthArray::from(vec![None]));
        let arrow_field = Field::new(
            "interval_value",
            DataType::Interval(IntervalUnit::YearMonth),
            true,
        );
        let pg_field = FieldInfo::new(
            "interval_value".to_owned(),
            None,
            None,
            Type::INTERVAL,
            FieldFormat::Text,
        );
        let mut encoder = TextCaptureEncoder::default();
        encode_value(&mut encoder, &array, 0, &arrow_field, &pg_field).expect("null interval");
        assert_eq!(encoder.encoded.len(), 1);
    }

    #[test]
    fn encodes_dictionary_array() {
        #[derive(Default)]
        struct MockEncoder {
            encoded_value: String,
        }

        impl Encoder for MockEncoder {
            type Item = String;

            fn encode_field<T>(&mut self, value: &T, pg_field: &FieldInfo) -> PgWireResult<()>
            where
                T: ToSql + ToSqlText + Sized,
            {
                let mut bytes = BytesMut::new();
                let _sql_text =
                    value.to_sql_text(pg_field.datatype(), &mut bytes, &FormatOptions::default());
                let string = String::from_utf8(bytes.to_vec());
                self.encoded_value = string.unwrap();
                Ok(())
            }

            fn take_row(&mut self) -> Self::Item {
                std::mem::take(&mut self.encoded_value)
            }
        }

        let val = "~!@&$[]()@@!!";
        let value = StringArray::from_iter_values([val]);
        let keys = Int8Array::from_iter_values([0, 0, 0, 0]);
        let dict_arr: Arc<dyn Array> =
            Arc::new(DictionaryArray::<Int8Type>::try_new(keys, Arc::new(value)).unwrap());

        let mut encoder = MockEncoder::default();

        let arrow_field = Field::new(
            "x",
            DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Utf8)),
            true,
        );
        let pg_field = FieldInfo::new("x".to_string(), None, None, Type::TEXT, FieldFormat::Text);
        let result = encode_value(&mut encoder, &dict_arr, 2, &arrow_field, &pg_field);

        assert!(result.is_ok());

        assert!(encoder.encoded_value == val);
    }

    #[test]
    fn encode_struct_null_emits_field() {
        // Regression test: encode_struct must call encoder.encode_field for
        // NULL struct values so a NULL indicator is written to the DataRow.
        // Previously it returned Ok(()) without encoding, corrupting the
        // column count.

        #[derive(Default)]
        struct CountingEncoder {
            call_count: usize,
        }

        impl Encoder for CountingEncoder {
            type Item = ();

            fn encode_field<T>(&mut self, _value: &T, _pg_field: &FieldInfo) -> PgWireResult<()>
            where
                T: ToSql + ToSqlText + Sized,
            {
                self.call_count += 1;
                Ok(())
            }

            fn take_row(&mut self) -> Self::Item {}
        }

        let fields = vec![
            Arc::new(Field::new("a", DataType::Utf8, true)),
            Arc::new(Field::new("b", DataType::Utf8, true)),
        ];
        let a = Arc::new(StringArray::from(vec![Some("hello"), Some("x")])) as Arc<dyn Array>;
        let b = Arc::new(StringArray::from(vec![Some("world"), Some("y")])) as Arc<dyn Array>;

        // Row 0: non-null struct, Row 1: null struct
        let null_buffer = NullBuffer::from(vec![true, false]);
        let struct_arr: Arc<dyn Array> = Arc::new(
            StructArray::try_new(fields.clone().into(), vec![a, b], Some(null_buffer)).unwrap(),
        );

        let arrow_field = Field::new("s", DataType::Struct(fields.into()), true);
        let pg_field = FieldInfo::new("s".to_string(), None, None, Type::TEXT, FieldFormat::Text);

        // Encode the NULL row (index 1).
        let mut encoder = CountingEncoder::default();
        let result = encode_value(&mut encoder, &struct_arr, 1, &arrow_field, &pg_field);
        assert!(result.is_ok());
        assert_eq!(
            encoder.call_count, 1,
            "encode_field must be called exactly once for a NULL struct to emit a NULL indicator"
        );
    }

    #[test]
    fn nested_struct_encoding_propagates_errors_without_panicking() {
        let timezone = Arc::<str>::from("Not/A-Timezone");
        let timestamp_field = Arc::new(Field::new(
            "observed_at",
            DataType::Timestamp(TimeUnit::Second, Some(Arc::clone(&timezone))),
            true,
        ));
        let timestamp: Arc<dyn Array> =
            Arc::new(TimestampSecondArray::from(vec![Some(0)]).with_timezone(timezone));
        let struct_array: Arc<dyn Array> = Arc::new(
            StructArray::try_new(
                vec![Arc::clone(&timestamp_field)].into(),
                vec![timestamp],
                None,
            )
            .expect("struct array"),
        );
        let struct_field = Field::new(
            "value",
            DataType::Struct(vec![timestamp_field].into()),
            false,
        );
        let pg_field = FieldInfo::new(
            "value".to_owned(),
            None,
            None,
            Type::RECORD,
            FieldFormat::Text,
        );
        let mut encoder = TextCaptureEncoder::default();

        let error = encode_value(&mut encoder, &struct_array, 0, &struct_field, &pg_field)
            .expect_err("invalid nested timezone must fail closed");
        assert!(matches!(error, PgWireError::ApiError(_)));
        assert!(encoder.encoded.is_empty());
    }

    #[test]
    fn test_get_time32_second_value() {
        let array = Time32SecondArray::from_iter_values([3723_i32]);
        let array: Arc<dyn Array> = Arc::new(array);
        let value = get_time32_second_value(&array, 0);
        assert_eq!(value, NaiveTime::from_hms_opt(1, 2, 3));
    }

    #[test]
    fn test_get_time32_millisecond_value() {
        let array = Time32MillisecondArray::from_iter_values([3723001_i32]);
        let array: Arc<dyn Array> = Arc::new(array);
        let value = get_time32_millisecond_value(&array, 0);
        assert_eq!(value, NaiveTime::from_hms_milli_opt(1, 2, 3, 1));
    }

    #[test]
    fn test_get_time64_microsecond_value() {
        let array = Time64MicrosecondArray::from_iter_values([3723001001_i64]);
        let array: Arc<dyn Array> = Arc::new(array);
        let value = get_time64_microsecond_value(&array, 0);
        assert_eq!(value, NaiveTime::from_hms_micro_opt(1, 2, 3, 1001));
    }

    #[test]
    fn test_get_time64_nanosecond_value() {
        let array = Time64NanosecondArray::from_iter_values([3723001001001_i64]);
        let array: Arc<dyn Array> = Arc::new(array);
        let value = get_time64_nanosecond_value(&array, 0);
        assert_eq!(value, NaiveTime::from_hms_nano_opt(1, 2, 3, 1001001));
    }
}
