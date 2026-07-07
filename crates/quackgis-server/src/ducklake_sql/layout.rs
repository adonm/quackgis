// SPDX-License-Identifier: Apache-2.0
//! Internal DuckLake layout-column projection.
//!
//! Tables opt in by carrying `_qg_*` layout columns. The write path recomputes
//! those columns from WKB instead of trusting client-provided values.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use datafusion::arrow::array::{
    Array, ArrayRef, BinaryArray, BinaryViewArray, Float64Array, Int32Array, Int64Array,
    new_null_array,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use sedona_geometry::bounds::wkb_bounds_xy;
use sedona_geometry::interval::IntervalTrait;

pub(crate) const MINX: &str = "_qg_minx";
pub(crate) const MINY: &str = "_qg_miny";
pub(crate) const MAXX: &str = "_qg_maxx";
pub(crate) const MAXY: &str = "_qg_maxy";
pub(crate) const SPACE_BUCKET: &str = "_qg_space_bucket";
pub(crate) const SPACE_SORT: &str = "_qg_space_sort";
pub(crate) const TIME_BUCKET: &str = "_qg_time_bucket";

const SPACE_BUCKET_SIZE: f64 = 1024.0;

#[derive(Debug, Clone, Copy)]
struct Bbox {
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64,
}

#[derive(Debug, Clone, Copy)]
struct TimeColumn {
    index: usize,
    bucket_width: f64,
}

#[derive(Debug)]
struct LayoutValues {
    minx: Vec<Option<f64>>,
    miny: Vec<Option<f64>>,
    maxx: Vec<Option<f64>>,
    maxy: Vec<Option<f64>>,
    space_bucket: Vec<Option<i64>>,
    space_sort: Vec<Option<i64>>,
    time_bucket: Vec<Option<i64>>,
}

impl LayoutValues {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            minx: Vec::with_capacity(capacity),
            miny: Vec::with_capacity(capacity),
            maxx: Vec::with_capacity(capacity),
            maxy: Vec::with_capacity(capacity),
            space_bucket: Vec::with_capacity(capacity),
            space_sort: Vec::with_capacity(capacity),
            time_bucket: Vec::with_capacity(capacity),
        }
    }
}

pub(crate) fn is_layout_column(name: &str) -> bool {
    name.eq_ignore_ascii_case(MINX)
        || name.eq_ignore_ascii_case(MINY)
        || name.eq_ignore_ascii_case(MAXX)
        || name.eq_ignore_ascii_case(MAXY)
        || name.eq_ignore_ascii_case(SPACE_BUCKET)
        || name.eq_ignore_ascii_case(SPACE_SORT)
        || name.eq_ignore_ascii_case(TIME_BUCKET)
}

pub(super) fn ensure_columns_for_spatial_batches(
    batches: Vec<RecordBatch>,
) -> Result<Vec<RecordBatch>> {
    let Some(schema) = batches.first().map(|batch| batch.schema()) else {
        return Ok(batches);
    };
    let missing = missing_layout_fields(schema.as_ref());
    if missing.is_empty() || geometry_column_index(schema.as_ref()).is_none() {
        return Ok(batches);
    }

    let mut fields = schema
        .fields()
        .iter()
        .map(|field| field.as_ref().clone())
        .collect::<Vec<_>>();
    fields.extend(missing.iter().cloned());
    let output_schema = Arc::new(Schema::new(fields));

    batches
        .into_iter()
        .map(|batch| {
            let mut columns = batch.columns().to_vec();
            columns.extend(
                missing
                    .iter()
                    .map(|field| new_null_array(field.data_type(), batch.num_rows())),
            );
            RecordBatch::try_new(Arc::clone(&output_schema), columns)
                .map_err(|e| anyhow!("adding DuckLake layout columns: {e}"))
        })
        .collect()
}

pub(super) fn project_batches(batches: Vec<RecordBatch>) -> Result<Vec<RecordBatch>> {
    batches.into_iter().map(project_batch).collect()
}

fn missing_layout_fields(schema: &Schema) -> Vec<Field> {
    layout_fields()
        .into_iter()
        .filter(|layout_field| {
            !schema
                .fields()
                .iter()
                .any(|field| field.name().eq_ignore_ascii_case(layout_field.name()))
        })
        .collect()
}

fn layout_fields() -> Vec<Field> {
    vec![
        Field::new(MINX, DataType::Float64, true),
        Field::new(MINY, DataType::Float64, true),
        Field::new(MAXX, DataType::Float64, true),
        Field::new(MAXY, DataType::Float64, true),
        Field::new(TIME_BUCKET, DataType::Int64, true),
        Field::new(SPACE_BUCKET, DataType::Int64, true),
        Field::new(SPACE_SORT, DataType::Int64, true),
    ]
}

fn project_batch(batch: RecordBatch) -> Result<RecordBatch> {
    if !schema_has_layout_columns(batch.schema().as_ref()) {
        return Ok(batch);
    }

    let Some(geometry_idx) = geometry_column_index(batch.schema().as_ref()) else {
        return Ok(batch);
    };

    let time_column = time_column(batch.schema().as_ref());
    let values = layout_values(&batch, geometry_idx, time_column);
    let mut columns = Vec::with_capacity(batch.num_columns());
    let mut changed = false;

    for (field, column) in batch.schema().fields().iter().zip(batch.columns()) {
        if let Some(projected) = match_layout_array(field.name(), field.data_type(), &values)? {
            columns.push(projected);
            changed = true;
        } else {
            columns.push(Arc::clone(column));
        }
    }

    if !changed {
        return Ok(batch);
    }

    RecordBatch::try_new(batch.schema(), columns)
        .map_err(|e| anyhow!("projecting DuckLake layout columns: {e}"))
}

fn schema_has_layout_columns(schema: &Schema) -> bool {
    schema
        .fields()
        .iter()
        .any(|field| is_layout_column(field.name()))
}

fn geometry_column_index(schema: &Schema) -> Option<usize> {
    schema.fields().iter().position(|field| {
        is_binary_like(field.data_type())
            && (crate::geometry_columns::is_geometry_column_name(field.name())
                || field.name().eq_ignore_ascii_case("footprint"))
    })
}

fn is_binary_like(data_type: &DataType) -> bool {
    matches!(data_type, DataType::Binary | DataType::BinaryView)
}

fn time_column(schema: &Schema) -> Option<TimeColumn> {
    schema
        .fields()
        .iter()
        .enumerate()
        .find_map(|(index, field)| {
            if is_layout_column(field.name()) || !is_numeric_time_type(field.data_type()) {
                return None;
            }
            let name = field.name().to_ascii_lowercase();
            let bucket_width = if name.contains("minute") {
                Some(60.0)
            } else if name == "time"
                || name == "timestamp"
                || name == "datetime"
                || name.ends_with("_time")
                || name.ends_with("_timestamp")
                || name.ends_with("_at")
                || name.contains("epoch")
            {
                Some(1.0)
            } else {
                None
            }?;
            Some(TimeColumn {
                index,
                bucket_width,
            })
        })
}

fn is_numeric_time_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Int32 | DataType::Int64 | DataType::Float64
    )
}

fn layout_values(
    batch: &RecordBatch,
    geometry_idx: usize,
    time_column: Option<TimeColumn>,
) -> LayoutValues {
    let mut values = LayoutValues::with_capacity(batch.num_rows());
    let geometry = &batch.columns()[geometry_idx];

    for row in 0..batch.num_rows() {
        let bbox = wkb_value(geometry, row).and_then(bounds_for_wkb);
        values.minx.push(bbox.map(|bbox| bbox.minx));
        values.miny.push(bbox.map(|bbox| bbox.miny));
        values.maxx.push(bbox.map(|bbox| bbox.maxx));
        values.maxy.push(bbox.map(|bbox| bbox.maxy));
        values.space_bucket.push(bbox.map(space_bucket));
        values.space_sort.push(bbox.map(space_sort));
        values
            .time_bucket
            .push(time_bucket_for_row(batch, time_column, row));
    }

    values
}

fn wkb_value(array: &ArrayRef, row: usize) -> Option<&[u8]> {
    if array.is_null(row) {
        return None;
    }
    if let Some(binary) = array.as_any().downcast_ref::<BinaryArray>() {
        return Some(binary.value(row));
    }
    if let Some(binary_view) = array.as_any().downcast_ref::<BinaryViewArray>() {
        return Some(binary_view.value(row));
    }
    None
}

fn bounds_for_wkb(wkb: &[u8]) -> Option<Bbox> {
    let bbox = wkb_bounds_xy(wkb).ok()?;
    if bbox.is_empty() || bbox.x().is_wraparound() {
        return None;
    }
    let bbox = Bbox {
        minx: bbox.x().lo(),
        miny: bbox.y().lo(),
        maxx: bbox.x().hi(),
        maxy: bbox.y().hi(),
    };
    if [bbox.minx, bbox.miny, bbox.maxx, bbox.maxy]
        .iter()
        .all(|value| value.is_finite())
    {
        Some(bbox)
    } else {
        None
    }
}

fn time_bucket_for_row(
    batch: &RecordBatch,
    time_column: Option<TimeColumn>,
    row: usize,
) -> Option<i64> {
    let Some(time_column) = time_column else {
        return Some(0);
    };
    let value = numeric_value(&batch.columns()[time_column.index], row)?;
    if !value.is_finite() || time_column.bucket_width <= 0.0 {
        return None;
    }
    Some((value / time_column.bucket_width).floor() as i64)
}

fn numeric_value(array: &ArrayRef, row: usize) -> Option<f64> {
    if array.is_null(row) {
        return None;
    }
    if let Some(values) = array.as_any().downcast_ref::<Int32Array>() {
        return Some(values.value(row) as f64);
    }
    if let Some(values) = array.as_any().downcast_ref::<Int64Array>() {
        return Some(values.value(row) as f64);
    }
    if let Some(values) = array.as_any().downcast_ref::<Float64Array>() {
        return Some(values.value(row));
    }
    None
}

fn match_layout_array(
    name: &str,
    data_type: &DataType,
    values: &LayoutValues,
) -> Result<Option<ArrayRef>> {
    if name.eq_ignore_ascii_case(MINX) {
        return float_layout_array(name, data_type, &values.minx);
    }
    if name.eq_ignore_ascii_case(MINY) {
        return float_layout_array(name, data_type, &values.miny);
    }
    if name.eq_ignore_ascii_case(MAXX) {
        return float_layout_array(name, data_type, &values.maxx);
    }
    if name.eq_ignore_ascii_case(MAXY) {
        return float_layout_array(name, data_type, &values.maxy);
    }
    if name.eq_ignore_ascii_case(SPACE_BUCKET) {
        return int_layout_array(name, data_type, &values.space_bucket);
    }
    if name.eq_ignore_ascii_case(SPACE_SORT) {
        return int_layout_array(name, data_type, &values.space_sort);
    }
    if name.eq_ignore_ascii_case(TIME_BUCKET) {
        return int_layout_array(name, data_type, &values.time_bucket);
    }
    Ok(None)
}

fn float_layout_array(
    name: &str,
    data_type: &DataType,
    values: &[Option<f64>],
) -> Result<Option<ArrayRef>> {
    if data_type != &DataType::Float64 {
        return Err(anyhow!(
            "layout column {name} must be DOUBLE/Float64, got {data_type}"
        ));
    }
    Ok(Some(Arc::new(Float64Array::from(values.to_vec()))))
}

fn int_layout_array(
    name: &str,
    data_type: &DataType,
    values: &[Option<i64>],
) -> Result<Option<ArrayRef>> {
    if data_type != &DataType::Int64 {
        return Err(anyhow!(
            "layout column {name} must be BIGINT/Int64, got {data_type}"
        ));
    }
    Ok(Some(Arc::new(Int64Array::from(values.to_vec()))))
}

fn space_bucket(bbox: Bbox) -> i64 {
    let center_x = (bbox.minx + bbox.maxx) / 2.0;
    let center_y = (bbox.miny + bbox.maxy) / 2.0;
    morton_signed(
        quantized_coord(center_x, SPACE_BUCKET_SIZE),
        quantized_coord(center_y, SPACE_BUCKET_SIZE),
    )
}

fn space_sort(bbox: Bbox) -> i64 {
    let center_x = (bbox.minx + bbox.maxx) / 2.0;
    let center_y = (bbox.miny + bbox.maxy) / 2.0;
    morton_signed(
        quantized_coord(center_x, 1.0),
        quantized_coord(center_y, 1.0),
    )
}

fn quantized_coord(value: f64, cell_size: f64) -> i64 {
    if !value.is_finite() || cell_size <= 0.0 {
        return 0;
    }
    (value / cell_size).floor() as i64
}

fn morton_signed(x: i64, y: i64) -> i64 {
    let x = zigzag_i32(x) as u64;
    let y = zigzag_i32(y) as u64;
    (split_by_1(x) | (split_by_1(y) << 1)) as i64
}

fn zigzag_i32(value: i64) -> u32 {
    let value = value.clamp(i32::MIN as i64, i32::MAX as i64);
    ((value << 1) ^ (value >> 31)) as u32
}

fn split_by_1(mut value: u64) -> u64 {
    value &= 0x0000_0000_ffff_ffff;
    value = (value | (value << 16)) & 0x0000_ffff_0000_ffff;
    value = (value | (value << 8)) & 0x00ff_00ff_00ff_00ff;
    value = (value | (value << 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    value = (value | (value << 2)) & 0x3333_3333_3333_3333;
    (value | (value << 1)) & 0x5555_5555_5555_5555
}
