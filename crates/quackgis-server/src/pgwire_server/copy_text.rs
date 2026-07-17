// SPDX-License-Identifier: Apache-2.0
//! Incremental, bounded PostgreSQL text COPY decoding.

use std::borrow::Cow;
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float32Array, Float64Array, Int16Array,
    Int32Array, Int64Array, RecordBatch, TimestampMicrosecondArray,
    builder::{BinaryBuilder, StringBuilder},
};
use arrow_pg::datatypes::classify_spatial_field;
use arrow_schema::{DataType, Field, SchemaRef, TimeUnit};
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

#[derive(Clone, Copy)]
struct FieldRange {
    start: u32,
    end: u32,
}

impl FieldRange {
    const NULL: Self = Self {
        start: u32::MAX,
        end: u32::MAX,
    };

    fn new(start: usize, end: usize) -> Self {
        Self {
            start: u32::try_from(start).expect("COPY batch data is bounded below 4 GiB"),
            end: u32::try_from(end).expect("COPY batch data is bounded below 4 GiB"),
        }
    }

    fn is_null(self) -> bool {
        self.start == u32::MAX
    }

    fn len(self) -> usize {
        (self.end - self.start) as usize
    }
}

struct CopyRows {
    data: Vec<u8>,
    fields: Vec<FieldRange>,
    row_ends: Vec<usize>,
    columns: usize,
}

impl CopyRows {
    fn new(columns: usize) -> Self {
        Self {
            data: Vec::new(),
            fields: Vec::new(),
            row_ends: vec![0],
            columns,
        }
    }

    fn len(&self) -> usize {
        self.row_ends.len() - 1
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn value(&self, row: usize, column: usize) -> Option<&[u8]> {
        let range = self.fields[row * self.columns + column];
        (!range.is_null()).then(|| &self.data[range.start as usize..range.end as usize])
    }

    fn split_off(&mut self, row: usize) -> Self {
        let byte_offset = self.row_ends[row];
        let data = self.data.split_off(byte_offset);
        let mut fields = self.fields.split_off(row * self.columns);
        let field_offset = u32::try_from(byte_offset).expect("COPY batch offset");
        for range in fields.iter_mut().filter(|range| !range.is_null()) {
            range.start -= field_offset;
            range.end -= field_offset;
        }
        let mut row_ends = self.row_ends.split_off(row);
        for end in &mut row_ends {
            *end -= byte_offset;
        }
        self.row_ends.push(byte_offset);
        Self {
            data,
            fields,
            row_ends,
            columns: self.columns,
        }
    }

    #[cfg(test)]
    fn from_owned(rows: Vec<Vec<Option<Vec<u8>>>>) -> Self {
        let columns = rows.first().map_or(0, Vec::len);
        let mut output = Self::new(columns);
        for row in rows {
            assert_eq!(row.len(), columns);
            for value in row {
                output.fields.push(match value {
                    Some(value) => {
                        let start = output.data.len();
                        output.data.extend_from_slice(&value);
                        FieldRange::new(start, output.data.len())
                    }
                    None => FieldRange::NULL,
                });
            }
            output.row_ends.push(output.data.len());
        }
        output
    }
}

pub struct CopyTextDecoder {
    schema: SchemaRef,
    limits: CopyBatchLimits,
    rows: CopyRows,
    field_start: usize,
    row_fields: usize,
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
        let columns = schema.fields().len();
        Ok(Self {
            schema,
            limits,
            rows: CopyRows::new(columns),
            field_start: 0,
            row_fields: 0,
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
        let mut offset = 0;
        while offset < data.len() {
            if self.terminated {
                return Err(copy_error("22P04", "COPY data follows the end marker"));
            }
            let remaining = &data[offset..];
            let Some(delimiter_offset) = memchr::memchr2(b'\t', b'\n', remaining) else {
                self.extend_field(remaining)?;
                break;
            };
            self.extend_field(&remaining[..delimiter_offset])?;
            self.add_row_bytes(1)?;
            match remaining[delimiter_offset] {
                b'\t' => self.finish_field()?,
                b'\n' => {
                    if self.rows.data.len() > self.field_start
                        && self.rows.data.last() == Some(&b'\r')
                    {
                        self.rows.data.pop();
                    }
                    if self.row_fields == 0 && self.rows.data[self.field_start..] == *br"\." {
                        self.rows.data.truncate(self.field_start);
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
                _ => unreachable!("memchr2 returns only configured delimiters"),
            }
            offset += delimiter_offset + 1;
        }
        Ok(batches)
    }

    fn extend_field(&mut self, data: &[u8]) -> Result<(), CopyDecodeError> {
        self.add_row_bytes(data.len())?;
        self.rows.data.extend_from_slice(data);
        Ok(())
    }

    fn add_row_bytes(&mut self, bytes: usize) -> Result<(), CopyDecodeError> {
        self.row_bytes = self.row_bytes.saturating_add(bytes);
        if self.row_bytes > self.limits.max_row_bytes {
            return Err(copy_error(
                "54000",
                &format!(
                    "COPY row {} exceeds the configured byte limit",
                    self.row_number
                ),
            ));
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<Vec<RecordBatch>, CopyDecodeError> {
        if !self.terminated && (self.rows.data.len() > self.field_start || self.row_fields > 0) {
            self.finish_row()?;
        }
        self.flush()
    }

    fn finish_field(&mut self) -> Result<(), CopyDecodeError> {
        if self.row_fields >= self.schema.fields().len() {
            return Err(copy_error(
                "22P04",
                &format!("COPY row {} has too many columns", self.row_number),
            ));
        }
        let value = if self.rows.data[self.field_start..] == *br"\N" {
            self.rows.data.truncate(self.field_start);
            FieldRange::NULL
        } else {
            FieldRange::new(self.field_start, self.rows.data.len())
        };
        self.rows.fields.push(value);
        self.field_start = self.rows.data.len();
        self.row_fields += 1;
        Ok(())
    }

    fn finish_row(&mut self) -> Result<(), CopyDecodeError> {
        self.finish_field()?;
        if self.row_fields != self.schema.fields().len() {
            return Err(copy_error(
                "22P04",
                &format!(
                    "COPY row {} has {} columns; expected {}",
                    self.row_number,
                    self.row_fields,
                    self.schema.fields().len()
                ),
            ));
        }
        self.batch_bytes = self.batch_bytes.saturating_add(self.row_bytes);
        self.row_bytes = 0;
        self.row_number += 1;
        self.rows.row_ends.push(self.rows.data.len());
        self.field_start = self.rows.data.len();
        self.row_fields = 0;
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
        self.field_start = 0;
        bounded_batches(
            Arc::clone(&self.schema),
            std::mem::replace(&mut self.rows, CopyRows::new(self.schema.fields().len())),
            self.limits.max_bytes,
        )
    }
}

fn bounded_batches(
    schema: SchemaRef,
    rows: CopyRows,
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

fn build_batch(schema: SchemaRef, rows: &CopyRows) -> Result<RecordBatch, CopyDecodeError> {
    let arrays = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(column, field)| copy_array(field, rows, column))
        .collect::<Result<Vec<_>, _>>()?;
    RecordBatch::try_new(schema, arrays)
        .map_err(|error| copy_error("22000", &format!("building COPY Arrow batch: {error}")))
}

fn copy_array(field: &Field, rows: &CopyRows, column: usize) -> Result<ArrayRef, CopyDecodeError> {
    match field.data_type() {
        DataType::Boolean => typed_values(rows, column, parse_bool)
            .map(|values| Arc::new(BooleanArray::from(values)) as ArrayRef),
        DataType::Int16 => typed_values(rows, column, |value| {
            parse_integer(value, "Int16").and_then(|value| {
                i16::try_from(value).map_err(|_| copy_error("22P02", "invalid COPY Int16 value"))
            })
        })
        .map(|values| Arc::new(Int16Array::from(values)) as ArrayRef),
        DataType::Int32 => typed_values(rows, column, |value| {
            parse_integer(value, "Int32").and_then(|value| {
                i32::try_from(value).map_err(|_| copy_error("22P02", "invalid COPY Int32 value"))
            })
        })
        .map(|values| Arc::new(Int32Array::from(values)) as ArrayRef),
        DataType::Int64 => typed_values(rows, column, |value| parse_integer(value, "Int64"))
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
            let value_bytes = rows
                .fields
                .iter()
                .skip(column)
                .step_by(rows.columns)
                .filter(|range| !range.is_null())
                .map(|range| range.len())
                .sum();
            let mut builder = StringBuilder::with_capacity(rows.len(), value_bytes);
            for row in 0..rows.len() {
                match rows.value(row, column) {
                    Some(raw) => {
                        let value = decode_copy_field(raw, false)?;
                        builder.append_value(field_text(&value)?);
                    }
                    None => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        DataType::Binary => {
            let value_bytes = rows
                .fields
                .iter()
                .skip(column)
                .step_by(rows.columns)
                .filter(|range| !range.is_null())
                .map(|range| range.len() / 2)
                .sum();
            let mut builder = BinaryBuilder::with_capacity(rows.len(), value_bytes);
            let mut decoded = Vec::new();
            let allow_postgis_hex = classify_spatial_field(field).is_some();
            for row in 0..rows.len() {
                match rows.value(row, column) {
                    Some(raw) => {
                        let value = decode_copy_field(raw, true)?;
                        parse_hex_into(&value, &mut decoded, allow_postgis_hex)?;
                        builder.append_value(&decoded);
                    }
                    None => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        unsupported => Err(copy_error(
            "0A000",
            &format!("unsupported DuckDB COPY Arrow type {unsupported}"),
        )),
    }
}

fn typed_values<T>(
    rows: &CopyRows,
    column: usize,
    parse: impl Fn(&[u8]) -> Result<T, CopyDecodeError>,
) -> Result<Vec<Option<T>>, CopyDecodeError> {
    (0..rows.len())
        .map(|row| {
            rows.value(row, column)
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

fn parse_integer(value: &[u8], label: &str) -> Result<i64, CopyDecodeError> {
    let (negative, digits) = match value.first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(copy_error("22P02", &format!("invalid COPY {label} value")));
    }
    let mut parsed = 0_i64;
    for digit in digits {
        parsed = if negative {
            parsed
                .checked_mul(10)
                .and_then(|value| value.checked_sub(i64::from(digit - b'0')))
        } else {
            parsed
                .checked_mul(10)
                .and_then(|value| value.checked_add(i64::from(digit - b'0')))
        }
        .ok_or_else(|| copy_error("22P02", &format!("invalid COPY {label} value")))?;
    }
    Ok(parsed)
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

fn parse_hex_into(
    value: &[u8],
    output: &mut Vec<u8>,
    allow_postgis_hex: bool,
) -> Result<(), CopyDecodeError> {
    let hex = match value.strip_prefix(br"\x") {
        Some(hex) => hex,
        None if allow_postgis_hex => value,
        None => {
            return Err(copy_error(
                "22P02",
                "COPY binary value must use \\x hex format",
            ));
        }
    };
    if !hex.len().is_multiple_of(2) {
        return Err(copy_error(
            "22P02",
            "COPY binary value contains invalid hex",
        ));
    }
    output.clear();
    output.reserve(hex.len() / 2);
    for pair in hex.chunks_exact(2) {
        let high = hex_nibble(pair[0])
            .ok_or_else(|| copy_error("22P02", "COPY binary value contains invalid hex"))?;
        let low = hex_nibble(pair[1])
            .ok_or_else(|| copy_error("22P02", "COPY binary value contains invalid hex"))?;
        output.push((high << 4) | low);
    }
    Ok(())
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn field_text(value: &[u8]) -> Result<&str, CopyDecodeError> {
    std::str::from_utf8(value).map_err(|_| copy_error("22021", "COPY text must be valid UTF-8"))
}

fn decode_copy_field(
    raw: &[u8],
    preserve_single_hex: bool,
) -> Result<Cow<'_, [u8]>, CopyDecodeError> {
    // Keep the legacy single-backslash bytea form intact. Standard COPY clients
    // send `\\x...`, which the general escape decoder below turns into `\x...`.
    if preserve_single_hex && raw.starts_with(br"\x") && raw[2..].iter().all(u8::is_ascii_hexdigit)
    {
        return Ok(Cow::Borrowed(raw));
    }
    if !raw.contains(&b'\\') {
        return Ok(Cow::Borrowed(raw));
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
    Ok(Cow::Owned(decoded))
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
    use arrow_array::{Array, BinaryArray, Int32Array, StringArray};
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
    fn accepts_postgis_hex_only_for_spatial_binary_fields() {
        let spatial_schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("geom_wkb", DataType::Binary, true),
        ]));
        let mut spatial = CopyTextDecoder::new(
            spatial_schema,
            CopyBatchLimits {
                max_rows: 8,
                max_bytes: 65_536,
                max_row_bytes: 256,
            },
        )
        .expect("spatial decoder");
        assert!(
            spatial
                .push(b"1\t0101000000000000000000F03F0000000000000040\n")
                .expect("GDAL PostGIS COPY geometry")
                .is_empty()
        );
        let batches = spatial.finish().expect("finish spatial COPY");
        let geometry = batches[0]
            .column(1)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("geometry");
        assert_eq!(geometry.value(0).len(), 21);

        let mut ordinary = CopyTextDecoder::new(
            schema(),
            CopyBatchLimits {
                max_rows: 8,
                max_bytes: 65_536,
                max_row_bytes: 256,
            },
        )
        .expect("ordinary decoder");
        assert!(
            ordinary
                .push(b"1\tvalue\t00ff\n")
                .expect("buffer ordinary binary")
                .is_empty()
        );
        let error = ordinary
            .finish()
            .expect_err("ordinary binary must retain bytea syntax");
        assert_eq!(error.sqlstate, "22P02");
    }

    #[test]
    fn contiguous_rows_preserve_empty_null_and_crlf_fields() {
        assert_eq!(std::mem::size_of::<FieldRange>(), 8);
        let mut decoder = CopyTextDecoder::new(
            schema(),
            CopyBatchLimits {
                max_rows: 8,
                max_bytes: 65_536,
                max_row_bytes: 256,
            },
        )
        .expect("decoder");
        assert!(
            decoder
                .push(b"1\t\t\\N\r\n2\tname\t\\x00\n")
                .expect("empty/null/CRLF rows")
                .is_empty()
        );
        let batches = decoder.finish().expect("finish rows");
        let batch = &batches[0];
        let names = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("names");
        assert_eq!(names.value(0), "");
        assert_eq!(names.value(1), "name");
        let payloads = batch
            .column(2)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("payloads");
        assert!(payloads.is_null(0));
        assert_eq!(payloads.value(1), &[0]);
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
            decode_copy_field(br"\x41", false)
                .expect("text hex escape")
                .as_ref(),
            b"A"
        );
        assert_eq!(
            decode_copy_field(br"\x41", true)
                .expect("legacy binary hex")
                .as_ref(),
            br"\x41"
        );
        assert_eq!(
            parse_integer(b"-9223372036854775808", "Int64").unwrap(),
            i64::MIN
        );
        assert_eq!(parse_integer(b"+42", "Int64").unwrap(), 42);
        assert!(parse_integer(b"9223372036854775808", "Int64").is_err());
        assert!(parse_integer(b"12x", "Int64").is_err());
    }

    #[test]
    fn recursively_splits_batches_to_the_arrow_byte_limit() {
        let rows = CopyRows::from_owned(
            (0..8)
                .map(|id| vec![Some(id.to_string().into_bytes()), Some(vec![b'x'; 40])])
                .collect(),
        );
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
        ]));
        let one_row = build_batch(
            Arc::clone(&schema),
            &CopyRows::from_owned(vec![vec![Some(b"0".to_vec()), Some(vec![b'x'; 40])]]),
        )
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
