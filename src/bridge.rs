// SPDX-License-Identifier: Apache-2.0
//
//! # DuckDB-chunk ⇄ Arrow bridge to Apache SedonaDB's DataFusion UDFs
//!
//! The rest of this extension reimplements the `ST_*` surface using the same
//! `geo` / `wkb` crates Apache SedonaDB builds on. This module makes the
//! superset *literal*: it links the real [`sedona_functions`] crate and invokes
//! its own DataFusion scalar UDF kernels directly from a DuckDB callback.
//!
//! The bridge is three steps, identical to how DataFusion itself would call
//! these UDFs:
//!
//! 1. **Read** each DuckDB input column into an Arrow [`ColumnarValue`] (a BLOB
//!    column of ISO-WKB becomes an Arrow `BinaryArray`; DOUBLE → `Float64Array`;
//!    INTEGER → `Int32Array`; VARCHAR → `StringArray`). NULLs are preserved.
//! 2. **Invoke** the [`SedonaScalarUDF`] via its `ScalarUDFImpl::invoke_with_args`
//!    with a [`ScalarFunctionArgs`] built from the inputs plus their
//!    [`SedonaType`]s (geometry inputs are tagged `SedonaType::Wkb` so the
//!    kernel's argument matcher selects the geometry implementation).
//! 3. **Write** the returned [`ColumnarValue`] back into the DuckDB output
//!    vector, dispatching on the Arrow `DataType` (WKB `Binary` → BLOB,
//!    `Float64` → DOUBLE, `Boolean` → BOOLEAN, …).
//!
//! Geometry round-trips as WKB bytes the whole way — exactly the extension's
//! existing BLOB convention — so SedonaDB-backed functions are interchangeable
//! with the reimplemented ones.
//!
//! The registered SQL names are prefixed `sedona_` so they sit alongside the
//! existing reimplementation rather than shadowing it (see `registry.rs`).

#![allow(clippy::not_unsafe_ptr_arg_deref, clippy::missing_safety_doc)]

use std::sync::{Arc, OnceLock};

use arrow_array::{
    builder::BinaryBuilder, Array, ArrayRef, BinaryArray, BinaryViewArray, BooleanArray,
    Float32Array, Float64Array, Int16Array, Int32Array, Int64Array, Int8Array, LargeStringArray,
    StringArray, StringViewArray, StructArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow_schema::{DataType, Field, FieldRef};
use datafusion_common::config::ConfigOptions;
use datafusion_common::error::Result as DfResult;
use datafusion_common::ScalarValue;
use datafusion_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl};
use libduckdb_sys::{duckdb_data_chunk, duckdb_vector};
use quack_rs::data_chunk::DataChunk;
use quack_rs::vector::{VectorReader, VectorWriter};
use sedona_expr::function_set::FunctionSet;
use sedona_expr::scalar_udf::SedonaScalarUDF;
use sedona_functions::register::default_function_set;
use sedona_schema::datatypes::{Edges, SedonaType};

use crate::dispatch::{read_blob, read_f64, read_i32};

/// Planar WKB geometry with no CRS — the [`SedonaType`] this extension's BLOB
/// columns carry (ISO-WKB, SRID-less). Cloning a `SedonaType` is cheap (it is a
/// small enum whose `Crs` payload is a boxed `Option`).
fn wkb_geometry() -> SedonaType {
    // Mirrors `sedona_schema::datatypes::WKB_GEOMETRY` (a `pub const` we cannot
    // take by reference across an `fn` boundary cheaply; rebuild it instead).
    SedonaType::Wkb(Edges::Planar, None)
}

/// Lazily build the real Apache SedonaDB function catalog once, then hand out
/// `&'static` references to it. Built on first use from `register_all`.
fn sedona_set() -> &'static FunctionSet {
    static SET: OnceLock<FunctionSet> = OnceLock::new();
    SET.get_or_init(default_function_set)
}

/// Look up a SedonaDB scalar UDF by its SedonaDB name (e.g. `"st_envelope"`).
///
/// Returns `None` if no such UDF exists in the catalog. This must **not** panic:
/// the lookup runs inside a per-batch DuckDB callback, and the release profile
/// uses `panic = "abort"`, so a renamed/removed SedonaDB UDF (or a registration
/// typo) would otherwise abort the DuckDB *process*. Callers fail closed to
/// NULL instead.
fn try_udf(name: &str) -> Option<&'static SedonaScalarUDF> {
    sedona_set().scalar_udf(name)
}

// ---------------------------------------------------------------------------
// Step 1 — DuckDB column → Arrow array
// ---------------------------------------------------------------------------

/// Read a BLOB (ISO-WKB) DuckDB column into an Arrow `BinaryArray`.
///
/// EWKB SRID tags are stripped here — SedonaDB kernels use the same `wkb`
/// crate reader we do, which rejects the PostGIS SRID flag. Callers that
/// propagate SRIDs peek the tags separately via [`peek_srids`].
fn read_blob_array(col: &VectorReader, nrows: usize) -> BinaryArray {
    let mut b = BinaryBuilder::new();
    for row in 0..nrows {
        match read_blob(col, row) {
            Some(bytes) => b.append_value(crate::geometry::strip_ewkb_srid(bytes)),
            None => b.append_null(),
        }
    }
    b.finish()
}

/// Per-row EWKB SRID tags of a BLOB column (0 = untagged). Used to re-tag
/// geometry outputs so bridge-routed functions propagate SRIDs like PostGIS.
fn peek_srids(col: &VectorReader, nrows: usize) -> Vec<i32> {
    (0..nrows)
        .map(|row| {
            read_blob(col, row)
                .and_then(crate::geometry::peek_ewkb_srid)
                .unwrap_or(0)
        })
        .collect()
}

/// Read a DOUBLE DuckDB column into an Arrow `Float64Array`.
fn read_f64_array(col: &VectorReader, nrows: usize) -> Float64Array {
    (0..nrows).map(|row| read_f64(col, row)).collect()
}

/// Read an INTEGER DuckDB column into an Arrow `Int32Array`.
fn read_i32_array(col: &VectorReader, nrows: usize) -> Int32Array {
    (0..nrows).map(|row| read_i32(col, row)).collect()
}

/// Read a VARCHAR DuckDB column into an Arrow `StringArray`. The builder copies
/// the string bytes into Arrow's own buffers, so the returned array owns its
/// data independent of the DuckDB chunk.
fn read_string_array(col: &VectorReader, nrows: usize) -> StringArray {
    let mut vals: Vec<Option<&str>> = Vec::with_capacity(nrows);
    for row in 0..nrows {
        // SAFETY: `row` < `nrows` == `col.row_count()` for all call sites.
        if unsafe { col.is_valid(row) } {
            // SAFETY: column is a VARCHAR column; row is valid.
            vals.push(Some(unsafe { col.read_str(row) }));
        } else {
            vals.push(None);
        }
    }
    StringArray::from(vals)
}

// --- Constant-scalar detection ------------------------------------------------
//
// Some SedonaDB kernels (e.g. `ST_PointN`, `ST_InteriorRingN`) require a
// non-geometry argument to be a `ColumnarValue::Scalar` — they cast it to a
// concrete scalar type and match on the `Scalar` arm, returning NULL (or an
// error) when it arrives as an `Array`. DuckDB usually folds such arguments
// (`ST_PointN(geom, 2)`) into a constant vector. When the whole column is one
// repeated value, we emit a `Scalar` so those kernels select their
// implementation; otherwise we emit an `Array`. This mirrors DataFusion's own
// constant handling.

/// If every value of an `Int32Array` is identical, return it as a scalar.
fn i32_scalar(arr: &Int32Array) -> Option<ScalarValue> {
    if arr.is_empty() {
        return None;
    }
    let first = if arr.is_null(0) { None } else { Some(arr.value(0)) };
    (1..arr.len())
        .all(|i| {
            let cur = if arr.is_null(i) { None } else { Some(arr.value(i)) };
            cur == first
        })
        .then(|| ScalarValue::Int32(first))
}

/// If every value of a `Float64Array` is identical, return it as a scalar.
fn f64_scalar(arr: &Float64Array) -> Option<ScalarValue> {
    if arr.is_empty() {
        return None;
    }
    let first = if arr.is_null(0) { None } else { Some(arr.value(0)) };
    (1..arr.len())
        .all(|i| {
            let cur = if arr.is_null(i) { None } else { Some(arr.value(i)) };
            cur == first
        })
        .then(|| ScalarValue::Float64(first))
}

// ---------------------------------------------------------------------------
// Step 2 — invoke a SedonaDB UDF over Arrow inputs
// ---------------------------------------------------------------------------

/// Invoke a SedonaDB scalar UDF over the given Arrow inputs, returning its
/// [`ColumnarValue`] result. `arg_types` carries the [`SedonaType`] for each
/// input so SedonaDB's kernel matcher (geometry vs geography vs scalar) picks
/// the right implementation; `nrows` is the batch length. Returns `None` if the
/// UDF is absent from the catalog (callers write all-NULL).
fn invoke(
    udf: Option<&SedonaScalarUDF>,
    inputs: Vec<ColumnarValue>,
    arg_types: &[SedonaType],
    nrows: usize,
) -> Option<ColumnarValue> {
    let udf = udf?;
    // Build `arg_fields` the way SedonaDB expects: each field carries the geoarrow
    // extension metadata that `SedonaType::from_storage_field` reads back inside
    // `invoke_with_args` to reconstruct the `SedonaType`. A plain `DataType::Binary`
    // field would be parsed as `SedonaType::Arrow(Binary)` (no geometry) and fail
    // kernel matching, so we always go through `to_storage_field`.
    let arg_fields: Vec<FieldRef> = arg_types
        .iter()
        .map(|st| {
            Arc::new(st.to_storage_field("item", true).expect("SedonaType -> Field"))
        })
        .collect();

    // SedonaDB's `invoke_with_args` recomputes the return type internally (it
    // does not consult `return_field`), so a placeholder field is sufficient.
    let return_field: FieldRef = Arc::new(Field::new("item", DataType::Null, true));

    let args = ScalarFunctionArgs {
        args: inputs,
        arg_fields,
        number_rows: nrows,
        return_field,
        config_options: Arc::new(ConfigOptions::default()),
    };

    // `panic = "abort"` (see Cargo.toml profile) means a panic here aborts the
    // DuckDB process, so we only surface `Result::Err` — every kernel error
    // becomes a NULL result for the whole batch (set by the caller).
    match udf.invoke_with_args(args) {
        DfResult::Ok(out) => Some(out),
        DfResult::Err(e) => {
            eprintln!("sedonadb bridge: {udf_name} invoke failed: {e}", udf_name = udf.name());
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Step 3 — Arrow result → DuckDB output vector
// ---------------------------------------------------------------------------

/// Downcast a primitive integer array of any width and write its values as
/// DuckDB INTEGER (i32), casting each non-null value through `$cast`.
macro_rules! write_int {
    ($arr:expr, $ty:ty, $nrows:expr, $writer:expr, $cast:expr) => {{
        let a = $arr.as_any().downcast_ref::<$ty>().unwrap();
        for row in 0..$nrows {
            if a.is_null(row) {
                unsafe { $writer.set_null(row) };
            } else {
                unsafe { $writer.write_i32(row, $cast(a.value(row))) };
            }
        }
    }};
}

/// Write a SedonaDB [`ColumnarValue`] result into the DuckDB output vector,
/// broadcasting a scalar across `nrows` and dispatching on the Arrow
/// `DataType`. Unrecognized result types write all-NULL (fail closed).
fn write_back(cv: ColumnarValue, writer: &mut VectorWriter, nrows: usize) {
    write_back_with_srids(cv, writer, nrows, None)
}

/// Like [`write_back`], but re-tags BLOB (geometry) outputs with the per-row
/// EWKB SRIDs peeked from the function's first geometry argument (0 = leave
/// untagged). Non-blob results ignore `srids`.
fn write_back_with_srids(
    cv: ColumnarValue,
    writer: &mut VectorWriter,
    nrows: usize,
    srids: Option<&[i32]>,
) {
    // Normalize a scalar result to a full-length array so the write loop is
    // uniform. `to_array_of_size` repeats the scalar `nrows` times; it returns a
    // `Result` (degenerate scalars can fail), so we fall back to all-NULL.
    let arr: ArrayRef = match cv {
        ColumnarValue::Array(a) => a,
        ColumnarValue::Scalar(s) => match s.to_array_of_size(nrows) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("sedonadb bridge: scalar broadcast failed: {e}");
                for row in 0..nrows {
                    unsafe { writer.set_null(row) };
                }
                return;
            }
        },
    };

    let srid_at = |row: usize| srids.map_or(0, |s| s.get(row).copied().unwrap_or(0));

    match arr.data_type() {
        DataType::Binary => {
            let a = arr.as_any().downcast_ref::<BinaryArray>().unwrap();
            for row in 0..nrows {
                if a.is_null(row) {
                    unsafe { writer.set_null(row) };
                } else if srid_at(row) > 0 {
                    let tagged = crate::geometry::tag_ewkb_srid(a.value(row), srid_at(row));
                    unsafe { writer.write_blob(row, &tagged) };
                } else {
                    unsafe { writer.write_blob(row, a.value(row)) };
                }
            }
        }
        DataType::BinaryView => {
            let a = arr.as_any().downcast_ref::<BinaryViewArray>().unwrap();
            for row in 0..nrows {
                if a.is_null(row) {
                    unsafe { writer.set_null(row) };
                } else if srid_at(row) > 0 {
                    let tagged = crate::geometry::tag_ewkb_srid(a.value(row), srid_at(row));
                    unsafe { writer.write_blob(row, &tagged) };
                } else {
                    unsafe { writer.write_blob(row, a.value(row)) };
                }
            }
        }
        DataType::Float64 => {
            let a = arr
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap();
            for row in 0..nrows {
                if a.is_null(row) {
                    unsafe { writer.set_null(row) };
                } else {
                    unsafe { writer.write_f64(row, a.value(row)) };
                }
            }
        }
        DataType::Float32 => {
            let a = arr.as_any().downcast_ref::<Float32Array>().unwrap();
            for row in 0..nrows {
                if a.is_null(row) {
                    unsafe { writer.set_null(row) };
                } else {
                    unsafe { writer.write_f64(row, a.value(row) as f64) };
                }
            }
        }
        // Every integer width SedonaDB emits (dimensions/counters). DuckDB's
        // writer only has INTEGER (i32); saturating cast is safe for any
        // realistic dimension index / vertex count.
        DataType::Int8 => write_int!(arr, Int8Array, nrows, writer, |v: i8| v as i32),
        DataType::Int16 => write_int!(arr, Int16Array, nrows, writer, |v: i16| v as i32),
        DataType::Int32 => write_int!(arr, Int32Array, nrows, writer, |v: i32| v),
        DataType::Int64 => write_int!(arr, Int64Array, nrows, writer, |v: i64| v as i32),
        DataType::UInt8 => write_int!(arr, UInt8Array, nrows, writer, |v: u8| v as i32),
        DataType::UInt16 => write_int!(arr, UInt16Array, nrows, writer, |v: u16| v as i32),
        DataType::UInt32 => write_int!(arr, UInt32Array, nrows, writer, |v: u32| v as i32),
        DataType::UInt64 => write_int!(arr, UInt64Array, nrows, writer, |v: u64| v as i32),
        DataType::Boolean => {
            let a = arr.as_any().downcast_ref::<BooleanArray>().unwrap();
            for row in 0..nrows {
                if a.is_null(row) {
                    unsafe { writer.set_null(row) };
                } else {
                    unsafe { writer.write_bool(row, a.value(row)) };
                }
            }
        }
        DataType::Utf8 => {
            let a = arr.as_any().downcast_ref::<StringArray>().unwrap();
            for row in 0..nrows {
                if a.is_null(row) {
                    unsafe { writer.set_null(row) };
                } else {
                    unsafe { writer.write_varchar(row, a.value(row)) };
                }
            }
        }
        DataType::Utf8View => {
            let a = arr.as_any().downcast_ref::<StringViewArray>().unwrap();
            for row in 0..nrows {
                if a.is_null(row) {
                    unsafe { writer.set_null(row) };
                } else {
                    unsafe { writer.write_varchar(row, a.value(row)) };
                }
            }
        }
        DataType::LargeUtf8 => {
            let a = arr.as_any().downcast_ref::<LargeStringArray>().unwrap();
            for row in 0..nrows {
                if a.is_null(row) {
                    unsafe { writer.set_null(row) };
                } else {
                    unsafe { writer.write_varchar(row, a.value(row)) };
                }
            }
        }
        // SedonaDB item-crs geometry: `struct<item: WKB, crs: Utf8View>`. The
        // extension's type model is plain SRID-less WKB (its own `ST_SetSRID`
        // is a no-op stub), so we unwrap the geometry and emit it as a BLOB,
        // dropping the CRS sidecar. This keeps item-crs-returning UDFs
        // (ST_GeomFromEWKT, ST_SetSRID, ST_Transform, …) usable through the
        // bridge at the extension's native fidelity level. Full CRS round-trip
        // would require a DuckDB struct type model throughout (out of scope).
        DataType::Struct(_) => {
            let s = arr.as_any().downcast_ref::<StructArray>().unwrap();
            if s.num_columns() >= 1 {
                // The first column is always the geometry `item` (WKB Binary).
                let item = s.column(0);
                for row in 0..nrows {
                    if s.is_null(row) {
                        unsafe { writer.set_null(row) };
                    } else if let Some(b) = item.as_any().downcast_ref::<BinaryArray>() {
                        if b.is_null(row) {
                            unsafe { writer.set_null(row) };
                        } else {
                            unsafe { writer.write_blob(row, b.value(row)) };
                        }
                    } else if let Some(b) = item.as_any().downcast_ref::<BinaryViewArray>() {
                        if b.is_null(row) {
                            unsafe { writer.set_null(row) };
                        } else {
                            unsafe { writer.write_blob(row, b.value(row)) };
                        }
                    } else {
                        unsafe { writer.set_null(row) };
                    }
                }
            } else {
                for row in 0..nrows {
                    unsafe { writer.set_null(row) };
                }
            }
        }
        other => {
            // Unsupported SedonaDB result type (e.g. List): fail closed to NULL
            // rather than risk a wrong write.
            eprintln!("sedonadb bridge: unsupported result type {other:?}; writing NULLs");
            for row in 0..nrows {
                unsafe { writer.set_null(row) };
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shape executors — one per (arity, return type). The registry mints a unique
// DuckDB callback per registration that forwards here with the SedonaDB UDF
// name as a `&'static str`.
// ---------------------------------------------------------------------------

/// `(geom) → geom` (e.g. `st_envelope`, `st_reverse`).
pub fn unary_blob_to_blob(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = reader.row_count();
    let srids = peek_srids(&reader, nrows);
    let arg = ColumnarValue::Array(Arc::new(read_blob_array(&reader, nrows)));
    let udf = try_udf(name);
    let stypes = [wkb_geometry()];
    let out = invoke(udf, vec![arg], &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back_with_srids(cv, &mut writer, nrows, Some(&srids)),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(binary WKB) → geom` — WKB constructors (`ST_GeomFromWKB`, `ST_GeomFromEWKB`).
/// Unlike `unary_blob_to_blob`, the input is typed as raw `Binary` (not geometry)
/// because these kernels match `is_binary()` and reject a geometry-typed arg.
pub fn binary_to_blob(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = reader.row_count();
    let arg = ColumnarValue::Array(Arc::new(read_blob_array(&reader, nrows)));
    let udf = try_udf(name);
    let stypes = [SedonaType::Arrow(DataType::Binary)];
    let out = invoke(udf, vec![arg], &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(geom) → INTEGER` (e.g. `st_dimension`, `st_numpoints`).
pub fn unary_blob_to_int(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = reader.row_count();
    let arg = ColumnarValue::Array(Arc::new(read_blob_array(&reader, nrows)));
    let udf = try_udf(name);
    let out = invoke(udf, vec![arg], &[wkb_geometry()], nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(geom) → BOOLEAN` (e.g. `st_isempty`, `st_isclosed`).
pub fn unary_blob_to_bool(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = reader.row_count();
    let arg = ColumnarValue::Array(Arc::new(read_blob_array(&reader, nrows)));
    let udf = try_udf(name);
    let out = invoke(udf, vec![arg], &[wkb_geometry()], nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(geom) → VARCHAR` (e.g. `st_astext`, `st_geometrytype`).
pub fn unary_blob_to_varchar(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = reader.row_count();
    let arg = ColumnarValue::Array(Arc::new(read_blob_array(&reader, nrows)));
    let udf = try_udf(name);
    let out = invoke(udf, vec![arg], &[wkb_geometry()], nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(geom) → DOUBLE` (e.g. `st_x`, `st_y`, `st_xmin`, ... ordinate accessors).
pub fn unary_blob_to_double(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = reader.row_count();
    let arg = ColumnarValue::Array(Arc::new(read_blob_array(&reader, nrows)));
    let udf = try_udf(name);
    let out = invoke(udf, vec![arg], &[wkb_geometry()], nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(VARCHAR) → geom` constructor (e.g. `st_geomfromewkt`).
pub fn varchar_to_blob(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = reader.row_count();
    let arg = ColumnarValue::Array(Arc::new(read_string_array(&reader, nrows)));
    let udf = try_udf(name);
    let out = invoke(udf, vec![arg], &[SedonaType::Arrow(DataType::Utf8)], nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(geom, INTEGER) → geom` (e.g. `st_setsrid`).
pub fn blob_int_to_blob(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let idx = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let nrows = geom.row_count();
    let arg0 = ColumnarValue::Array(Arc::new(read_blob_array(&geom, nrows)));
    // Emit a Scalar when the index column is a constant broadcast so SedonaDB
    // kernels that match on `ColumnarValue::Scalar` (ST_PointN, ...) select
    // their implementation instead of returning NULL.
    let idx_arr = Arc::new(read_i32_array(&idx, nrows));
    let arg1 = match i32_scalar(&idx_arr) {
        Some(s) => ColumnarValue::Scalar(s),
        None => ColumnarValue::Array(idx_arr),
    };
    let udf = try_udf(name);
    let stypes = [wkb_geometry(), SedonaType::Arrow(DataType::Int32)];
    let out = invoke(udf, vec![arg0, arg1], &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(geom, DOUBLE) → geom` (e.g. `st_translate` is 3-arg; this shape covers
/// the common 2-arg transforms like `st_segmentize`).
pub fn blob_double_to_blob(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let val = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let nrows = geom.row_count();
    let srids = peek_srids(&geom, nrows);
    let arg0 = ColumnarValue::Array(Arc::new(read_blob_array(&geom, nrows)));
    let val_arr = Arc::new(read_f64_array(&val, nrows));
    let arg1 = match f64_scalar(&val_arr) {
        Some(s) => ColumnarValue::Scalar(s),
        None => ColumnarValue::Array(val_arr),
    };
    let udf = try_udf(name);
    let stypes = [wkb_geometry(), SedonaType::Arrow(DataType::Float64)];
    let out = invoke(udf, vec![arg0, arg1], &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back_with_srids(cv, &mut writer, nrows, Some(&srids)),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(DOUBLE, DOUBLE) → geom` constructor (`ST_Point`).
pub fn doubles2_to_blob(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let xc = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let yc = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let nrows = xc.row_count();
    let xa = Arc::new(read_f64_array(&xc, nrows));
    let ya = Arc::new(read_f64_array(&yc, nrows));
    let arg0 = match f64_scalar(&xa) {
        Some(s) => ColumnarValue::Scalar(s),
        None => ColumnarValue::Array(xa),
    };
    let arg1 = match f64_scalar(&ya) {
        Some(s) => ColumnarValue::Scalar(s),
        None => ColumnarValue::Array(ya),
    };
    let udf = try_udf(name);
    let stypes = [SedonaType::Arrow(DataType::Float64), SedonaType::Arrow(DataType::Float64)];
    let out = invoke(udf, vec![arg0, arg1], &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// Reads `ncols` Float64 columns from the chunk, emitting a `Scalar` when the
/// column is a constant broadcast (so SedonaDB kernels that match on
/// `ColumnarValue::Scalar` select the right arm). Shared by the Z/M point
/// constructors (`ST_PointZ` = 3, `ST_PointM` = 3, `ST_PointZM` = 4).
fn f64_args(input: duckdb_data_chunk, ncols: usize) -> (Vec<ColumnarValue>, Vec<SedonaType>, usize) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let first = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = first.row_count();
    let mut args = Vec::with_capacity(ncols);
    let mut stypes = vec![SedonaType::Arrow(DataType::Float64); ncols];
    for col in 0..ncols {
        let reader = unsafe { VectorReader::new(chunk.as_raw(), col) };
        let arr = Arc::new(read_f64_array(&reader, nrows));
        let cv = match f64_scalar(&arr) {
            Some(s) => ColumnarValue::Scalar(s),
            None => ColumnarValue::Array(arr),
        };
        args.push(cv);
    }
    let _ = &mut stypes; // keep stypes aligned with ncols
    (args, stypes, nrows)
}

/// `(DOUBLE, DOUBLE, DOUBLE) → geom` (`ST_PointZ`, `ST_PointM`).
pub fn doubles3_to_blob(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let (args, stypes, nrows) = f64_args(input, 3);
    let udf = try_udf(name);
    let out = invoke(udf, args, &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(DOUBLE, DOUBLE, DOUBLE, DOUBLE) → geom` (`ST_PointZM`).
pub fn doubles4_to_blob(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let (args, stypes, nrows) = f64_args(input, 4);
    let udf = try_udf(name);
    let out = invoke(udf, args, &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(geom, DOUBLE, DOUBLE) → geom` (`ST_Translate`, `ST_Scale`, `ST_LineSubstring`).
pub fn blob_double2_to_blob(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let a = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let b = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let nrows = geom.row_count();
    let srids = peek_srids(&geom, nrows);
    let arg0 = ColumnarValue::Array(Arc::new(read_blob_array(&geom, nrows)));
    let aa = Arc::new(read_f64_array(&a, nrows));
    let ba = Arc::new(read_f64_array(&b, nrows));
    let arg1 = match f64_scalar(&aa) {
        Some(s) => ColumnarValue::Scalar(s),
        None => ColumnarValue::Array(aa),
    };
    let arg2 = match f64_scalar(&ba) {
        Some(s) => ColumnarValue::Scalar(s),
        None => ColumnarValue::Array(ba),
    };
    let udf = try_udf(name);
    let stypes = [
        wkb_geometry(),
        SedonaType::Arrow(DataType::Float64),
        SedonaType::Arrow(DataType::Float64),
    ];
    let out = invoke(udf, vec![arg0, arg1, arg2], &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back_with_srids(cv, &mut writer, nrows, Some(&srids)),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(geom, DOUBLE×6) → geom` (`ST_Affine` 2D: a,b,d,e,xOff,yOff).
pub fn blob_double6_to_blob(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = geom.row_count();
    let srids = peek_srids(&geom, nrows);
    let arg0 = ColumnarValue::Array(Arc::new(read_blob_array(&geom, nrows)));
    let mut args = vec![arg0];
    let mut stypes = vec![wkb_geometry()];
    for col in 1..=6 {
        let reader = unsafe { VectorReader::new(chunk.as_raw(), col) };
        let arr = Arc::new(read_f64_array(&reader, nrows));
        let cv = match f64_scalar(&arr) {
            Some(s) => ColumnarValue::Scalar(s),
            None => ColumnarValue::Array(arr),
        };
        args.push(cv);
        stypes.push(SedonaType::Arrow(DataType::Float64));
    }
    let udf = try_udf(name);
    let out = invoke(udf, args, &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back_with_srids(cv, &mut writer, nrows, Some(&srids)),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(geom, geom) → geom` (`ST_MakeLine`).
pub fn blob_blob_to_blob(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let a = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let b = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let nrows = a.row_count();
    let srids = peek_srids(&a, nrows);
    let arg0 = ColumnarValue::Array(Arc::new(read_blob_array(&a, nrows)));
    let arg1 = ColumnarValue::Array(Arc::new(read_blob_array(&b, nrows)));
    let udf = try_udf(name);
    let stypes = [wkb_geometry(), wkb_geometry()];
    let out = invoke(udf, vec![arg0, arg1], &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back_with_srids(cv, &mut writer, nrows, Some(&srids)),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(BLOB) → BLOB` with a constant DOUBLE injected as the second arg.
/// Used for 1-arg overloads like `ST_Force3D(geom)` → `ST_Force3D(geom, 0.0)`.
pub fn blob_to_blob_with_default_double(
    name: &'static str,
    default_val: f64,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = geom.row_count();
    let srids = peek_srids(&geom, nrows);
    let arg0 = ColumnarValue::Array(Arc::new(read_blob_array(&geom, nrows)));
    let arg1 = ColumnarValue::Scalar(ScalarValue::Float64(Some(default_val)));
    let udf = try_udf(name);
    let stypes = [wkb_geometry(), SedonaType::Arrow(DataType::Float64)];
    let out = invoke(udf, vec![arg0, arg1], &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back_with_srids(cv, &mut writer, nrows, Some(&srids)),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(BLOB) → BLOB` with two constant DOUBLEs injected as args.
/// Used for `ST_Force4D(geom)` → `ST_Force4D(geom, 0.0, 0.0)`.
pub fn blob_to_blob_with_default_doubles(
    name: &'static str,
    default1: f64,
    default2: f64,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = geom.row_count();
    let srids = peek_srids(&geom, nrows);
    let arg0 = ColumnarValue::Array(Arc::new(read_blob_array(&geom, nrows)));
    let arg1 = ColumnarValue::Scalar(ScalarValue::Float64(Some(default1)));
    let arg2 = ColumnarValue::Scalar(ScalarValue::Float64(Some(default2)));
    let udf = try_udf(name);
    let stypes = [
        wkb_geometry(),
        SedonaType::Arrow(DataType::Float64),
        SedonaType::Arrow(DataType::Float64),
    ];
    let out = invoke(udf, vec![arg0, arg1, arg2], &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back_with_srids(cv, &mut writer, nrows, Some(&srids)),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// `(geom, geom) → DOUBLE` (`ST_Azimuth`).
pub fn blob_blob_to_double(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let a = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let b = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let nrows = a.row_count();
    let arg0 = ColumnarValue::Array(Arc::new(read_blob_array(&a, nrows)));
    let arg1 = ColumnarValue::Array(Arc::new(read_blob_array(&b, nrows)));
    let udf = try_udf(name);
    let stypes = [wkb_geometry(), wkb_geometry()];
    let out = invoke(udf, vec![arg0, arg1], &stypes, nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_back(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

// --- CRS sidecar access -----------------------------------------------------
//
// SedonaDB CRS-producing UDFs (`ST_SetSRID`, `ST_GeomFromEWKT`, ...) return an
// item-crs struct `struct<item: WKB, crs: Utf8View>`. The default executors
// above unwrap the `item` to a plain BLOB (Phase 3, the extension's native
// SRID-less model). These `_crs` variants instead extract the `crs` sidecar as
// a VARCHAR, so callers can read the CRS metadata SedonaDB computed without a
// full DuckDB struct type model. Pair with the matching item-returning UDF.

/// Extract the CRS string from an item-crs struct returned by a `(geom) →
/// item-crs` UDF (`ST_SRID` semantics via the literal kernel).
pub fn unary_blob_extract_crs(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = reader.row_count();
    let arg = ColumnarValue::Array(Arc::new(read_blob_array(&reader, nrows)));
    let out = invoke(try_udf(name), vec![arg], &[wkb_geometry()], nrows);
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_crs(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// Extract the CRS string from an item-crs struct returned by a `(VARCHAR) →
/// item-crs` constructor (e.g. `ST_GeomFromEWKT`, which carries the SRID in the
/// `SRID=...;` prefix). Returns NULL when the input has no CRS.
pub fn varchar_extract_crs(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let nrows = reader.row_count();
    let arg = ColumnarValue::Array(Arc::new(read_string_array(&reader, nrows)));
    let out = invoke(
        try_udf(name),
        vec![arg],
        &[SedonaType::Arrow(DataType::Utf8)],
        nrows,
    );
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_crs(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// Extract the CRS string from an item-crs struct returned by a
/// `(geom, INTEGER) → item-crs` UDF (e.g. `ST_SetSRID`).
pub fn blob_int_extract_crs(name: &'static str, input: duckdb_data_chunk, output: duckdb_vector) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let idx = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let nrows = geom.row_count();
    let arg0 = ColumnarValue::Array(Arc::new(read_blob_array(&geom, nrows)));
    let idx_arr = Arc::new(read_i32_array(&idx, nrows));
    let arg1 = match i32_scalar(&idx_arr) {
        Some(s) => ColumnarValue::Scalar(s),
        None => ColumnarValue::Array(idx_arr),
    };
    let out = invoke(
        try_udf(name),
        vec![arg0, arg1],
        &[wkb_geometry(), SedonaType::Arrow(DataType::Int32)],
        nrows,
    );
    let mut writer = unsafe { VectorWriter::new(output) };
    match out {
        Some(cv) => write_crs(cv, &mut writer, nrows),
        None => for row in 0..nrows {
            unsafe { writer.set_null(row) };
        },
    }
}

/// Write the `crs` column of a SedonaDB item-crs struct result as a VARCHAR.
/// Non-struct results (plain WKB) have no CRS → NULL.
fn write_crs(cv: ColumnarValue, writer: &mut VectorWriter, nrows: usize) {
    let arr = match cv {
        ColumnarValue::Array(a) => a,
        ColumnarValue::Scalar(s) => match s.to_array_of_size(nrows) {
            Ok(a) => a,
            Err(_) => {
                for row in 0..nrows {
                    unsafe { writer.set_null(row) };
                }
                return;
            }
        },
    };
    let Some(s) = arr.as_any().downcast_ref::<StructArray>() else {
        for row in 0..nrows {
            unsafe { writer.set_null(row) };
        }
        return;
    };
    // item-crs layout: column 0 = item (WKB), column 1 = crs (Utf8View).
    let crs = s.column(1);
    let mut write_str = |row: usize, val: Option<&str>| match val {
        Some(v) => unsafe { writer.write_varchar(row, v) },
        None => unsafe { writer.set_null(row) },
    };
    if let Some(v) = crs.as_any().downcast_ref::<StringViewArray>() {
        for row in 0..nrows {
            write_str(row, if v.is_null(row) { None } else { Some(v.value(row)) });
        }
    } else if let Some(v) = crs.as_any().downcast_ref::<StringArray>() {
        for row in 0..nrows {
            write_str(row, if v.is_null(row) { None } else { Some(v.value(row)) });
        }
    } else if let Some(v) = crs.as_any().downcast_ref::<LargeStringArray>() {
        for row in 0..nrows {
            write_str(row, if v.is_null(row) { None } else { Some(v.value(row)) });
        }
    } else {
        for row in 0..nrows {
            unsafe { writer.set_null(row) };
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — exercise the *literal* SedonaDB invoke path at the Rust level,
// independent of DuckDB (which needs the full GDAL build to load). These prove
// the real `sedona-functions` kernels run through our Arrow bridge.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry;
    use geo_types::{Geometry, Point};

    /// Build ISO-WKB bytes for a 2D point.
    fn point_wkb(x: f64, y: f64) -> Vec<u8> {
        geometry::to_wkb(&Geometry::Point(Point::new(x, y))).unwrap()
    }

    /// Materialize a result `ColumnarValue` to a full-length `ArrayRef`.
    fn to_array(cv: ColumnarValue) -> ArrayRef {
        match cv {
            ColumnarValue::Array(a) => a,
            ColumnarValue::Scalar(s) => s.to_array_of_size(1).unwrap(),
        }
    }

    /// `ST_Dimension` of a POINT is 0, computed by SedonaDB's own kernel
    /// (returns `Int8`). This is the canonical "the literal path works" check.
    #[test]
    fn literal_sedona_st_dimension() {
        let bytes = point_wkb(1.0, 2.0);
        let arr: ArrayRef = Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())]));
        let input = ColumnarValue::Array(arr);

        let out = invoke(try_udf("st_dimension"), vec![input], &[wkb_geometry()], 1)
            .expect("SedonaDB st_dimension must invoke");

        let arr = to_array(out);
        let ints = arr
            .as_any()
            .downcast_ref::<Int8Array>()
            .expect("st_dimension returns Int8");
        assert_eq!(ints.value(0), 0, "POINT dimension is 0");
    }

    /// `ST_AsText` round-trips WKB → WKT through SedonaDB's writer.
    #[test]
    fn literal_sedona_st_astext() {
        let bytes = point_wkb(1.0, 2.0);
        let arr: ArrayRef = Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())]));

        let out = invoke(
            try_udf("st_astext"),
            vec![ColumnarValue::Array(arr)],
            &[wkb_geometry()],
            1,
        )
        .expect("SedonaDB st_astext must invoke");

        let result = to_array(out);
        let s = result
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("st_astext returns Utf8");
        assert!(
            s.value(0).starts_with("POINT"),
            "expected WKT for POINT, got {:?}",
            s.value(0)
        );
    }

    /// `ST_IsEmpty` of a real point is `false`, via SedonaDB's kernel.
    #[test]
    fn literal_sedona_st_isempty() {
        let bytes = point_wkb(1.0, 2.0);
        let arr: ArrayRef = Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())]));

        let out = invoke(
            try_udf("st_isempty"),
            vec![ColumnarValue::Array(arr)],
            &[wkb_geometry()],
            1,
        )
        .expect("SedonaDB st_isempty must invoke");

        let result = to_array(out);
        let b = result
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("st_isempty returns Boolean");
        assert!(!b.value(0), "POINT(1 2) is not empty");
    }

    /// The default function set really is Apache SedonaDB's own catalog, and it
    /// contains the functions we registered through the bridge.
    #[test]
    fn sedona_default_function_set_is_populated() {
        let set = sedona_set();
        for name in [
            "st_dimension",
            "st_astext",
            "st_isempty",
            "st_envelope",
            "st_geometrytype",
        ] {
            assert!(set.scalar_udf(name).is_some(), "missing {name}");
        }
        assert!(set.scalar_udfs().count() >= 60, "expected a large catalog");
    }
}
