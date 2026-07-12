// SPDX-License-Identifier: Apache-2.0
//! Incremental, bounded PostgreSQL text COPY decoding.

use std::sync::Arc;

use arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Decimal128Array, Float32Array, Float64Array,
    Int16Array, Int32Array, Int64Array, RecordBatch, StringArray, TimestampMicrosecondArray,
};
use arrow_schema::{DataType, SchemaRef, TimeUnit};
use chrono::{NaiveDate, NaiveDateTime};

pub const MAX_BATCHES_PER_COPY_CHUNK: usize = 128;

#[derive(Clone, Copy, Debug)]
pub struct CopyBatchLimits {
    pub max_rows: usize,
    pub max_bytes: usize,
    pub max_row_bytes: usize,
}

#[derive(Debug)]
pub struct CopyDecodeError {
    pub sqlstate: &'static str,
    pub message: String,
}

type CopyValue = Option<Vec<u8>>;
type CopyRow = Vec<CopyValue>;

pub struct CopyTextDecoder {
    schema: SchemaRef,
    limits: CopyBatchLimits,
    field: Vec<u8>,
    row: CopyRow,
    rows: Vec<CopyRow>,
    row_bytes: usize,
    batch_bytes: usize,
    row_number: usize,
    terminated: bool,
}

impl CopyTextDecoder {
    pub fn new(schema: SchemaRef, limits: CopyBatchLimits) -> Result<Self, CopyDecodeError> {
        if limits.max_rows == 0 || limits.max_bytes == 0 || limits.max_row_bytes == 0 {
            return Err(copy_error("22023", "COPY batch limits must be positive"));
        }
        Ok(Self {
            schema,
            limits,
            field: Vec::new(),
            row: Vec::new(),
            rows: Vec::new(),
            row_bytes: 0,
            batch_bytes: 0,
            row_number: 1,
            terminated: false,
        })
    }

    pub fn push(&mut self, data: &[u8]) -> Result<Vec<RecordBatch>, CopyDecodeError> {
        if self.terminated && !data.is_empty() {
            return Err(copy_error("22P04", "COPY data follows the end marker"));
        }
        let mut batches = Vec::new();
        for &byte in data {
            if self.terminated {
                return Err(copy_error("22P04", "COPY data follows the end marker"));
            }
            self.row_bytes = self.row_bytes.saturating_add(1);
            if self.row_bytes > self.limits.max_row_bytes {
                return Err(copy_error(
                    "54000",
                    &format!(
                        "COPY row {} exceeds the configured byte limit",
                        self.row_number
                    ),
                ));
            }
            match byte {
                b'\t' => self.finish_field()?,
                b'\n' => {
                    if self.field.last() == Some(&b'\r') {
                        self.field.pop();
                    }
                    if self.row.is_empty() && self.field == br"\." {
                        self.field.clear();
                        self.row_bytes = 0;
                        self.terminated = true;
                        continue;
                    }
                    self.finish_row()?;
                    if self.should_flush() {
                        batches.extend(self.flush()?);
                        if batches.len() > MAX_BATCHES_PER_COPY_CHUNK {
                            return Err(copy_error(
                                "54000",
                                "one COPY data chunk produces too many Arrow batches",
                            ));
                        }
                    }
                }
                _ => self.field.push(byte),
            }
        }
        Ok(batches)
    }

    pub fn finish(&mut self) -> Result<Vec<RecordBatch>, CopyDecodeError> {
        if !self.terminated && (!self.field.is_empty() || !self.row.is_empty()) {
            self.finish_row()?;
        }
        self.flush()
    }

    fn finish_field(&mut self) -> Result<(), CopyDecodeError> {
        if self.row.len() >= self.schema.fields().len() {
            return Err(copy_error(
                "22P04",
                &format!("COPY row {} has too many columns", self.row_number),
            ));
        }
        let field = std::mem::take(&mut self.field);
        let value = if field == br"\N" { None } else { Some(field) };
        self.row.push(value);
        Ok(())
    }

    fn finish_row(&mut self) -> Result<(), CopyDecodeError> {
        self.finish_field()?;
        if self.row.len() != self.schema.fields().len() {
            return Err(copy_error(
                "22P04",
                &format!(
                    "COPY row {} has {} columns; expected {}",
                    self.row_number,
                    self.row.len(),
                    self.schema.fields().len()
                ),
            ));
        }
        self.batch_bytes = self.batch_bytes.saturating_add(self.row_bytes);
        self.row_bytes = 0;
        self.row_number += 1;
        self.rows.push(std::mem::take(&mut self.row));
        Ok(())
    }

    fn should_flush(&self) -> bool {
        self.rows.len() >= self.limits.max_rows || self.batch_bytes >= self.limits.max_bytes
    }

    fn flush(&mut self) -> Result<Vec<RecordBatch>, CopyDecodeError> {
        if self.rows.is_empty() {
            return Ok(Vec::new());
        }
        self.batch_bytes = 0;
        bounded_batches(
            Arc::clone(&self.schema),
            std::mem::take(&mut self.rows),
            self.limits.max_bytes,
        )
    }
}

fn bounded_batches(
    schema: SchemaRef,
    rows: Vec<CopyRow>,
    max_bytes: usize,
) -> Result<Vec<RecordBatch>, CopyDecodeError> {
    let batch = build_batch(Arc::clone(&schema), &rows)?;
    if batch.get_array_memory_size() <= max_bytes {
        return Ok(vec![batch]);
    }
    if rows.len() == 1 {
        return Err(copy_error(
            "54000",
            "one COPY row exceeds the configured Arrow batch byte limit",
        ));
    }
    let mut left = rows;
    let right = left.split_off(left.len() / 2);
    let mut batches = bounded_batches(Arc::clone(&schema), left, max_bytes)?;
    batches.extend(bounded_batches(schema, right, max_bytes)?);
    Ok(batches)
}

fn build_batch(schema: SchemaRef, rows: &[CopyRow]) -> Result<RecordBatch, CopyDecodeError> {
    let arrays = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(column, field)| copy_array(field.data_type(), rows, column))
        .collect::<Result<Vec<_>, _>>()?;
    RecordBatch::try_new(schema, arrays)
        .map_err(|error| copy_error("22000", &format!("building COPY Arrow batch: {error}")))
}

fn copy_array(
    data_type: &DataType,
    rows: &[CopyRow],
    column: usize,
) -> Result<ArrayRef, CopyDecodeError> {
    match data_type {
        DataType::Boolean => typed_values(rows, column, parse_bool)
            .map(|values| Arc::new(BooleanArray::from(values)) as ArrayRef),
        DataType::Int16 => typed_values(rows, column, |value| parse(value, "Int16"))
            .map(|values| Arc::new(Int16Array::from(values)) as ArrayRef),
        DataType::Int32 => typed_values(rows, column, |value| parse(value, "Int32"))
            .map(|values| Arc::new(Int32Array::from(values)) as ArrayRef),
        DataType::Int64 => typed_values(rows, column, |value| parse(value, "Int64"))
            .map(|values| Arc::new(Int64Array::from(values)) as ArrayRef),
        DataType::Float32 => typed_values(rows, column, |value| parse(value, "Float32"))
            .map(|values| Arc::new(Float32Array::from(values)) as ArrayRef),
        DataType::Float64 => typed_values(rows, column, |value| parse(value, "Float64"))
            .map(|values| Arc::new(Float64Array::from(values)) as ArrayRef),
        DataType::Decimal128(precision, scale) => {
            let values = typed_values(rows, column, |value| {
                parse_decimal(value, *precision, *scale)
            })?;
            Decimal128Array::from(values)
                .with_precision_and_scale(*precision, *scale)
                .map(|array| Arc::new(array) as ArrayRef)
                .map_err(|error| copy_error("22003", &error.to_string()))
        }
        DataType::Date32 => typed_values(rows, column, parse_date)
            .map(|values| Arc::new(Date32Array::from(values)) as ArrayRef),
        DataType::Timestamp(TimeUnit::Microsecond, None) => {
            typed_values(rows, column, parse_timestamp)
                .map(|values| Arc::new(TimestampMicrosecondArray::from(values)) as ArrayRef)
        }
        DataType::Utf8 => {
            let values = rows
                .iter()
                .map(|row| {
                    row[column]
                        .as_deref()
                        .map(|value| {
                            decode_copy_field(value, false)
                                .and_then(|value| field_text(&value).map(str::to_owned))
                        })
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Arc::new(StringArray::from(values)) as ArrayRef)
        }
        DataType::Binary => {
            let values = rows
                .iter()
                .map(|row| {
                    row[column]
                        .as_deref()
                        .map(|value| {
                            decode_copy_field(value, true).and_then(|value| parse_hex(&value))
                        })
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Arc::new(BinaryArray::from_opt_vec(
                values.iter().map(|value| value.as_deref()).collect(),
            )) as ArrayRef)
        }
        unsupported => Err(copy_error(
            "0A000",
            &format!("unsupported DuckDB COPY Arrow type {unsupported}"),
        )),
    }
}

fn typed_values<T>(
    rows: &[CopyRow],
    column: usize,
    parse: impl Fn(&[u8]) -> Result<T, CopyDecodeError>,
) -> Result<Vec<Option<T>>, CopyDecodeError> {
    rows.iter()
        .map(|row| {
            row[column]
                .as_deref()
                .map(|value| decode_copy_field(value, false).and_then(|value| parse(&value)))
                .transpose()
        })
        .collect()
}

fn parse<T: std::str::FromStr>(value: &[u8], label: &str) -> Result<T, CopyDecodeError> {
    field_text(value)?
        .parse()
        .map_err(|_| copy_error("22P02", &format!("invalid COPY {label} value")))
}

fn parse_bool(value: &[u8]) -> Result<bool, CopyDecodeError> {
    match field_text(value)?.to_ascii_lowercase().as_str() {
        "t" | "true" | "1" | "y" | "yes" | "on" => Ok(true),
        "f" | "false" | "0" | "n" | "no" | "off" => Ok(false),
        _ => Err(copy_error("22P02", "invalid COPY Boolean value")),
    }
}

fn parse_decimal(value: &[u8], precision: u8, scale: i8) -> Result<i128, CopyDecodeError> {
    let decimal = field_text(value)?
        .parse::<rust_decimal::Decimal>()
        .map_err(|_| copy_error("22P02", "invalid COPY Decimal128 value"))?;
    let target_scale = u32::try_from(scale)
        .map_err(|_| copy_error("0A000", "negative Decimal128 COPY scale is unsupported"))?;
    let source_scale = decimal.scale();
    let mut mantissa = decimal.mantissa();
    if source_scale < target_scale {
        let factor = 10_i128
            .checked_pow(target_scale - source_scale)
            .ok_or_else(|| copy_error("22003", "COPY Decimal128 scale overflows"))?;
        mantissa = mantissa
            .checked_mul(factor)
            .ok_or_else(|| copy_error("22003", "COPY Decimal128 value overflows"))?;
    } else if source_scale > target_scale {
        let divisor = 10_i128
            .checked_pow(source_scale - target_scale)
            .ok_or_else(|| copy_error("22003", "COPY Decimal128 scale overflows"))?;
        if mantissa % divisor != 0 {
            return Err(copy_error(
                "22003",
                "COPY Decimal128 value exceeds the target scale",
            ));
        }
        mantissa /= divisor;
    }
    if mantissa.unsigned_abs().to_string().len() > usize::from(precision) {
        return Err(copy_error(
            "22003",
            "COPY Decimal128 value exceeds the target precision",
        ));
    }
    Ok(mantissa)
}

fn parse_date(value: &[u8]) -> Result<i32, CopyDecodeError> {
    let date = NaiveDate::parse_from_str(field_text(value)?, "%Y-%m-%d")
        .map_err(|_| copy_error("22007", "invalid COPY Date32 value"))?;
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid Unix epoch");
    i32::try_from(date.signed_duration_since(epoch).num_days())
        .map_err(|_| copy_error("22008", "COPY Date32 value is out of range"))
}

fn parse_timestamp(value: &[u8]) -> Result<i64, CopyDecodeError> {
    NaiveDateTime::parse_from_str(field_text(value)?, "%Y-%m-%d %H:%M:%S%.f")
        .map(|timestamp| timestamp.and_utc().timestamp_micros())
        .map_err(|_| copy_error("22007", "invalid COPY Timestamp value"))
}

fn parse_hex(value: &[u8]) -> Result<Vec<u8>, CopyDecodeError> {
    let hex = value
        .strip_prefix(br"\x")
        .ok_or_else(|| copy_error("22P02", "COPY binary value must use \\x hex format"))?;
    if !hex.len().is_multiple_of(2) || !hex.iter().all(u8::is_ascii_hexdigit) {
        return Err(copy_error(
            "22P02",
            "COPY binary value contains invalid hex",
        ));
    }
    hex.chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("ASCII hex pair");
            u8::from_str_radix(text, 16)
                .map_err(|_| copy_error("22P02", "COPY binary value contains invalid hex"))
        })
        .collect()
}

fn field_text(value: &[u8]) -> Result<&str, CopyDecodeError> {
    std::str::from_utf8(value).map_err(|_| copy_error("22021", "COPY text must be valid UTF-8"))
}

fn decode_copy_field(raw: &[u8], preserve_single_hex: bool) -> Result<Vec<u8>, CopyDecodeError> {
    // Keep the legacy single-backslash bytea form intact. Standard COPY clients
    // send `\\x...`, which the general escape decoder below turns into `\x...`.
    if preserve_single_hex && raw.starts_with(br"\x") && raw[2..].iter().all(u8::is_ascii_hexdigit)
    {
        return Ok(raw.to_vec());
    }
    let mut decoded = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        if raw[index] != b'\\' {
            decoded.push(raw[index]);
            index += 1;
            continue;
        }
        index += 1;
        let escaped = *raw
            .get(index)
            .ok_or_else(|| copy_error("22P04", "COPY field ends with a backslash escape"))?;
        match escaped {
            b'b' => decoded.push(8),
            b'f' => decoded.push(12),
            b'n' => decoded.push(b'\n'),
            b'r' => decoded.push(b'\r'),
            b't' => decoded.push(b'\t'),
            b'v' => decoded.push(11),
            b'\\' => decoded.push(b'\\'),
            b'x' => {
                let start = index + 1;
                let end = (start + 2).min(raw.len());
                let digits = raw[start..end]
                    .iter()
                    .take_while(|byte| byte.is_ascii_hexdigit())
                    .count();
                if digits == 0 {
                    return Err(copy_error("22P04", "COPY field has an invalid hex escape"));
                }
                let text = std::str::from_utf8(&raw[start..start + digits]).expect("ASCII hex");
                decoded.push(u8::from_str_radix(text, 16).expect("validated hex"));
                index += digits;
            }
            b'0'..=b'7' => {
                let start = index;
                let end = (start + 3).min(raw.len());
                let digits = raw[start..end]
                    .iter()
                    .take_while(|byte| matches!(byte, b'0'..=b'7'))
                    .count();
                let text = std::str::from_utf8(&raw[start..start + digits]).expect("ASCII octal");
                decoded.push(u8::from_str_radix(text, 8).expect("validated octal"));
                index += digits - 1;
            }
            other => decoded.push(other),
        }
        index += 1;
    }
    Ok(decoded)
}

fn copy_error(sqlstate: &'static str, message: &str) -> CopyDecodeError {
    CopyDecodeError {
        sqlstate,
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Array, Int32Array};
    use arrow_schema::{Field, Schema};

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("payload", DataType::Binary, true),
        ]))
    }

    #[test]
    fn decodes_chunks_escapes_nulls_and_binary() {
        let mut decoder = CopyTextDecoder::new(
            schema(),
            CopyBatchLimits {
                max_rows: 2,
                max_bytes: 65_536,
                max_row_bytes: 256,
            },
        )
        .expect("decoder");
        assert!(
            decoder
                .push(b"1\thello\\")
                .expect("partial escape")
                .is_empty()
        );
        let batches = decoder
            .push(b"tworld\t\\x00ff\n2\t\\N\t\\\\x0102\n")
            .expect("remaining chunks");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 2);
        let ids = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("ids");
        assert_eq!(ids.values(), &[1, 2]);
        let names = batches[0]
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("names");
        assert_eq!(names.value(0), "hello\tworld");
        assert!(names.is_null(1));
        let payload = batches[0]
            .column(2)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("payload");
        assert_eq!(payload.value(0), &[0, 255]);
        assert_eq!(payload.value(1), &[1, 2]);
    }

    #[test]
    fn enforces_row_and_arrow_batch_limits() {
        let mut decoder = CopyTextDecoder::new(
            schema(),
            CopyBatchLimits {
                max_rows: 1,
                max_bytes: 65_536,
                max_row_bytes: 16,
            },
        )
        .expect("decoder");
        let batches = decoder
            .push(b"1\ta\t\\x00\n2\tb\t\\x01\n")
            .expect("bounded rows");
        assert_eq!(batches.len(), 2);
        assert!(batches.iter().all(|batch| batch.num_rows() == 1));
        assert!(decoder.push(b"3\tthis-row-is-too-large").is_err());
        assert_eq!(
            decode_copy_field(br"\x41", false).expect("text hex escape"),
            b"A"
        );
        assert_eq!(
            decode_copy_field(br"\x41", true).expect("legacy binary hex"),
            br"\x41"
        );
    }

    #[test]
    fn recursively_splits_batches_to_the_arrow_byte_limit() {
        let rows = (0..8)
            .map(|id| vec![Some(id.to_string().into_bytes()), Some(vec![b'x'; 40])])
            .collect::<Vec<_>>();
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
        ]));
        let one_row = build_batch(Arc::clone(&schema), &rows[..1])
            .expect("one-row batch")
            .get_array_memory_size();
        let full = build_batch(Arc::clone(&schema), &rows)
            .expect("full batch")
            .get_array_memory_size();
        assert!(one_row < full);
        let max_bytes = one_row + 16;

        let batches = bounded_batches(schema, rows, max_bytes).expect("bounded batches");
        assert!(batches.len() > 1);
        assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 8);
        assert!(
            batches
                .iter()
                .all(|batch| batch.get_array_memory_size() <= max_bytes)
        );
    }

    #[test]
    fn end_marker_rejects_trailing_data_in_the_same_chunk() {
        let mut decoder = CopyTextDecoder::new(
            schema(),
            CopyBatchLimits {
                max_rows: 8,
                max_bytes: 65_536,
                max_row_bytes: 256,
            },
        )
        .expect("decoder");
        assert!(decoder.push(b"\\.\n1\ta\t\\x00\n").is_err());
    }
}
