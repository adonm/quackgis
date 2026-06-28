// SPDX-License-Identifier: Apache-2.0
//
// Unified Vectorized Dispatch Pipeline.
//
// Instead of writing one FFI callback per SQL function, we provide a handful
// of *generic* executors — one per result shape — that each:
//
//   1. wrap the DuckDB input chunk / output vector,
//   2. read WKB blobs row by row,
//   3. apply a caller-supplied geometry function, and
//   4. write the result back into DuckDB's columnar vectors.
//
// The `registry` module then maps every SQL name to one of these executors via
// a declarative macro, so adding a function is a single line.
//
// All executors use `NullHandling::SpecialNullHandling` (see `registry.rs`),
// so they own NULL propagation: NULL in -> NULL out, and a geometry that fails
// to parse or compute also yields NULL.
//
// NOTE on blob reading: we deliberately bypass `quack_rs::VectorReader::read_blob`
// here. That helper routes through `read_duck_string`, which validates UTF-8
// and returns an *empty* slice for blobs that are not valid UTF-8. WKB is
// arbitrary binary, so that path silently corrupts ~half of all geometries
// (e.g. any coordinate whose IEEE-754 bytes form an invalid UTF-8 sequence).
// We parse the 16-byte `duckdb_string_t` ourselves to get the raw bytes.

use geo_types::{Geometry, MultiPolygon};
use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_data_chunk_get_size, duckdb_data_chunk_get_vector,
    duckdb_function_info, duckdb_vector, duckdb_vector_get_data, duckdb_vector_get_validity,
    duckdb_validity_row_is_valid, idx_t,
};
use quack_rs::aggregate::{AggregateState, FfiState};
use quack_rs::data_chunk::DataChunk;
use quack_rs::vector::{VectorReader, VectorWriter};

use crate::geometry::{self, Geom};

/// Size of a `duckdb_string_t` (inline + pointer union) in bytes.
const STRING_T_SIZE: usize = 16;
/// Strings ≤ this length are stored inline in the `duckdb_string_t`.
const STRING_T_INLINE_MAX: usize = 12;

/// Raw (non-UTF-8-validated) reader for a BLOB / VARCHAR column.
///
/// Holds a borrowed view of one column's data buffer + validity bitmap for the
/// lifetime of the enclosing data chunk.
struct BlobCol {
    data: *const u8,
    validity: *mut u64,
    n: usize,
}

impl BlobCol {
    /// # Safety
    /// `chunk` must be a valid `duckdb_data_chunk` for the lifetime of this
    /// reader; `col` must be a valid BLOB/VARCHAR column index.
    unsafe fn new(chunk: duckdb_data_chunk, col: usize) -> Self {
        let n = usize::try_from(unsafe { duckdb_data_chunk_get_size(chunk) }).unwrap_or(0);
        let vector = unsafe { duckdb_data_chunk_get_vector(chunk, col as idx_t) };
        let data = unsafe { duckdb_vector_get_data(vector) }.cast::<u8>();
        let validity = unsafe { duckdb_vector_get_validity(vector) };
        Self { data, validity, n }
    }

    /// Returns the raw bytes of the value at `row`, or `None` if the row is
    /// NULL or out of range.
    ///
    /// # Safety
    /// The reader must outlive the returned slice (it borrows from DuckDB's
    /// vector memory). `row` is bounds-checked internally.
    unsafe fn get(&self, row: usize) -> Option<&[u8]> {
        if row >= self.n {
            return None;
        }
        if !self.validity.is_null()
            && !unsafe { duckdb_validity_row_is_valid(self.validity, row as idx_t) }
        {
            return None;
        }
        // The duckdb_string_t for this row starts at `data + row * 16`.
        let st = unsafe { self.data.add(row * STRING_T_SIZE) };
        // SAFETY: st points to a valid 16-byte string_t.
        let raw: &[u8; STRING_T_SIZE] = unsafe { &*st.cast::<[u8; STRING_T_SIZE]>() };
        let len = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        if len <= STRING_T_INLINE_MAX {
            // Inline: the value lives in bytes [4..4+len].
            Some(&raw[4..4 + len])
        } else {
            // Pointer: bytes [8..16] hold the heap pointer.
            let pb: [u8; 8] = raw[8..16].try_into().ok()?;
            let ptr = u64::from_le_bytes(pb) as *const u8;
            if ptr.is_null() {
                return None;
            }
            // SAFETY: DuckDB allocated `len` bytes at `ptr`; valid for the
            // chunk's lifetime.
            Some(unsafe { core::slice::from_raw_parts(ptr, len) })
        }
    }
}

/// Read & parse the geometry at `row` of a BLOB column.
fn read_geom(col: &BlobCol, row: usize) -> Option<Geom> {
    let bytes = unsafe { col.get(row) }?;
    geometry::from_wkb(bytes).ok()
}

/// Vectorized executor for `(geometry, INTEGER) -> geometry` accessors
/// (`ST_GeometryN`, `ST_PointN`, `ST_InteriorRingN`).
pub fn geom_int_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, i32) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let idx = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.n {
        let (Some(g), Some(i)) = (read_geom(&geom, row), read_i32(&idx, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&g, i).and_then(|out| geometry::to_wkb(&out).ok()) {
            Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(geometry, INTEGER) -> VARCHAR` (`ST_AsEWKT`).
pub fn geom_int_to_varchar<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, i32) -> Option<String>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let idx = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.n {
        let (Some(g), Some(i)) = (read_geom(&geom, row), read_i32(&idx, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&g, i) {
            Some(v) => unsafe { writer.write_varchar(row, &v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(geometry, INTEGER, INTEGER) -> geometry` transforms
/// (`ST_Transform`).
pub fn geom_int2_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, i32, i32) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let a = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let b = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.n {
        let (Some(g), Some(from), Some(to)) =
            (read_geom(&geom, row), read_i32(&a, row), read_i32(&b, row))
        else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&g, from, to).and_then(|out| geometry::to_wkb(&out).ok()) {
            Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Read an INTEGER at `row`, or `None` if NULL.
fn read_i32(reader: &VectorReader, row: usize) -> Option<i32> {
    if !unsafe { reader.is_valid(row) } {
        return None;
    }
    Some(unsafe { reader.read_i32(row) })
}

/// Read a DOUBLE at `row`, or `None` if NULL.
fn read_f64(reader: &VectorReader, row: usize) -> Option<f64> {
    // SAFETY: `row` < row_count in all loops below.
    if !unsafe { reader.is_valid(row) } {
        return None;
    }
    Some(unsafe { reader.read_f64(row) })
}

// ---------------------------------------------------------------------------
// Geometry -> Geometry   (BLOB -> BLOB)
// ---------------------------------------------------------------------------

/// Vectorized executor for unary geometry-producing functions
/// (`ST_ConvHull`, `ST_Envelope`, `ST_Centroid`, ...).
pub fn unary_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom) -> Option<Geom>,
{
    // SAFETY: `input`/`output` are valid DuckDB handles handed to our callback.
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.n {
        let Some(g) = read_geom(&reader, row) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&g).and_then(|out| geometry::to_wkb(&out).ok()) {
            Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for binary geometry-producing functions
/// (`ST_Intersection`, `ST_Union`, ...).
pub fn binary_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, &Geom) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let left = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let right = unsafe { BlobCol::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.n {
        let (Some(a), Some(b)) = (read_geom(&left, row), read_geom(&right, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&a, &b).and_then(|out| geometry::to_wkb(&out).ok()) {
            Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

// ---------------------------------------------------------------------------
// Geometry x Geometry -> Boolean   (BLOB, BLOB -> BOOLEAN)
// ---------------------------------------------------------------------------

/// Vectorized executor for binary spatial predicates
/// (`ST_Intersects`, `ST_Contains`, `ST_Within`, `ST_Disjoint`).
pub fn binary_predicate<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, &Geom) -> Option<bool>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let left = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let right = unsafe { BlobCol::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.n {
        let (Some(a), Some(b)) = (read_geom(&left, row), read_geom(&right, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&a, &b) {
            Some(v) => unsafe { writer.write_bool(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

// ---------------------------------------------------------------------------
// Geometry -> scalar   (BLOB -> DOUBLE / VARCHAR / INTEGER / BOOLEAN)
// ---------------------------------------------------------------------------

/// Vectorized executor for unary scalar functions returning `DOUBLE`.
pub fn unary_geom_double<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom) -> Option<f64>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.n {
        match read_geom(&reader, row).and_then(|g| f(&g)) {
            Some(v) => unsafe { writer.write_f64(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for unary scalar functions returning `VARCHAR`.
pub fn unary_geom_varchar<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom) -> Option<String>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.n {
        match read_geom(&reader, row).and_then(|g| f(&g)) {
            Some(v) => unsafe { writer.write_varchar(row, &v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for unary scalar functions returning `INTEGER`.
pub fn unary_geom_int<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom) -> Option<i32>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.n {
        match read_geom(&reader, row).and_then(|g| f(&g)) {
            Some(v) => unsafe { writer.write_i32(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for unary `geometry -> BOOLEAN` predicates.
pub fn unary_geom_bool<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom) -> Option<bool>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.n {
        match read_geom(&reader, row).and_then(|g| f(&g)) {
            Some(v) => unsafe { writer.write_bool(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

// ---------------------------------------------------------------------------
// Constructors and mixed-type executors
// ---------------------------------------------------------------------------

/// Vectorized executor for WKT-string -> geometry constructors (`ST_GeomFromText`).
pub fn str_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&str) -> Option<Geom>,
{
    // VARCHAR input uses the UTF-8-safe reader (WKT is text).
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
        if !unsafe { reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let s = unsafe { reader.read_str(row) };
        match f(s).and_then(|g| geometry::to_wkb(&g).ok()) {
            Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(geometry, DOUBLE) -> geometry` transforms
/// (`ST_Buffer`, `ST_Simplify`).
pub fn geom_double_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, f64) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let scalar = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.n {
        let (Some(g), Some(v)) = (read_geom(&geom, row), read_f64(&scalar, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&g, v).and_then(|out| geometry::to_wkb(&out).ok()) {
            Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(geometry, DOUBLE, DOUBLE) -> geometry` transforms
/// (`ST_Translate`, `ST_Scale`).
pub fn geom_double2_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, f64, f64) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let s1 = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let s2 = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.n {
        let (Some(g), Some(a), Some(b)) =
            (read_geom(&geom, row), read_f64(&s1, row), read_f64(&s2, row))
        else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&g, a, b).and_then(|out| geometry::to_wkb(&out).ok()) {
            Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(geometry, DOUBLE×6) -> geometry` transforms
/// (`ST_Affine` 2D: a, b, d, e, xoff, yoff).
pub fn geom_double6_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, f64, f64, f64, f64, f64, f64) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let s = [
        unsafe { VectorReader::new(chunk.as_raw(), 1) },
        unsafe { VectorReader::new(chunk.as_raw(), 2) },
        unsafe { VectorReader::new(chunk.as_raw(), 3) },
        unsafe { VectorReader::new(chunk.as_raw(), 4) },
        unsafe { VectorReader::new(chunk.as_raw(), 5) },
        unsafe { VectorReader::new(chunk.as_raw(), 6) },
    ];
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.n {
        let Some(g) = read_geom(&geom, row) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        let vals: Option<[f64; 6]> = s
            .iter()
            .map(|r| read_f64(r, row))
            .collect::<Option<Vec<_>>>()
            .map(|v| [v[0], v[1], v[2], v[3], v[4], v[5]]);
        let Some([a, b, c, d, e, ff]) = vals else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&g, a, b, c, d, e, ff).and_then(|out| geometry::to_wkb(&out).ok()) {
            Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(DOUBLE, DOUBLE) -> geometry` constructors (`ST_Point`).
pub fn doubles2_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(f64, f64) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let x = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let y = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..x.row_count() {
        match (read_f64(&x, row), read_f64(&y, row)) {
            (Some(xv), Some(yv)) => match f(xv, yv).and_then(|g| geometry::to_wkb(&g).ok()) {
                Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
                None => unsafe { writer.set_null(row) },
            },
            _ => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(geometry, geometry) -> DOUBLE` measurements (`ST_Distance`).
pub fn binary_geom_double<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, &Geom) -> Option<f64>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let left = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let right = unsafe { BlobCol::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.n {
        let (Some(a), Some(b)) = (read_geom(&left, row), read_geom(&right, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&a, &b) {
            Some(v) => unsafe { writer.write_f64(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(geometry, geometry, DOUBLE) -> BOOLEAN` distance
/// predicates (`ST_DWithin`).
pub fn geom_geom_double_bool<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, &Geom, f64) -> Option<bool>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let left = unsafe { BlobCol::new(chunk.as_raw(), 0) };
    let right = unsafe { BlobCol::new(chunk.as_raw(), 1) };
    let scalar = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.n {
        let (Some(a), Some(b)) = (read_geom(&left, row), read_geom(&right, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        let Some(d) = read_f64(&scalar, row) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&a, &b, d) {
            Some(v) => unsafe { writer.write_bool(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Marker so the `#[allow(dead_code)]` on `Geometry` stays meaningful.
#[allow(dead_code)]
type _DocGeometry = Geometry<f64>;

// ---------------------------------------------------------------------------
// ST_Collect aggregate
// ---------------------------------------------------------------------------

/// Per-group state for `ST_Collect`: the accumulated geometries.
#[derive(Default)]
pub struct CollectState {
    pub geoms: Vec<Geom>,
}
impl AggregateState for CollectState {}

pub unsafe extern "C" fn collect_state_size(info: duckdb_function_info) -> idx_t {
    unsafe { FfiState::<CollectState>::size_callback(info) }
}

pub unsafe extern "C" fn collect_state_init(info: duckdb_function_info, state: duckdb_aggregate_state) {
    unsafe { FfiState::<CollectState>::init_callback(info, state) };
}

pub unsafe extern "C" fn collect_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let col = unsafe { BlobCol::new(input, 0) };
    for row in 0..col.n {
        let Some(bytes) = (unsafe { col.get(row) }) else { continue };
        let Ok(g) = geometry::from_wkb(bytes) else { continue };
        // SAFETY: `states` has one entry per input row.
        let state_ptr = unsafe { *states.add(row) };
        if let Some(st) = unsafe { FfiState::<CollectState>::with_state_mut(state_ptr) } {
            st.geoms.push(g);
        }
    }
}

pub unsafe extern "C" fn collect_combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        let src_ptr = unsafe { *source.add(i) };
        let tgt_ptr = unsafe { *target.add(i) };
        let src = unsafe { FfiState::<CollectState>::with_state(src_ptr) };
        let tgt = unsafe { FfiState::<CollectState>::with_state_mut(tgt_ptr) };
        if let (Some(s), Some(t)) = (src, tgt) {
            // Clone rather than move: `with_state` yields a shared reference.
            // Correctness > speed for an aggregate combiner; see quack-rs pitfall L1.
            t.geoms.extend(s.geoms.iter().cloned());
        }
    }
}

pub unsafe extern "C" fn collect_finalize(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    let mut writer = unsafe { VectorWriter::new(result) };
    for i in 0..count as usize {
        let out_row = offset as usize + i;
        let state_ptr = unsafe { *source.add(i) };
        match unsafe { FfiState::<CollectState>::with_state(state_ptr) } {
            Some(st) if !st.geoms.is_empty() => {
                let gc = Geometry::GeometryCollection(geo_types::GeometryCollection(st.geoms.clone()));
                match geometry::to_wkb(&gc) {
                    Ok(b) => unsafe { writer.write_blob(out_row, &b) },
                    Err(_) => unsafe { writer.set_null(out_row) },
                }
            }
            _ => unsafe { writer.set_null(out_row) },
        }
    }
}

pub unsafe extern "C" fn collect_destroy(states: *mut duckdb_aggregate_state, count: idx_t) {
    unsafe { FfiState::<CollectState>::destroy_callback(states, count) };
}

// ---------------------------------------------------------------------------
// ST_Envelope aggregate (union of bounding boxes)
// ---------------------------------------------------------------------------

/// Per-group state for `ST_Envelope` aggregate: an optional bbox.
#[derive(Default)]
pub struct EnvelopeAggState {
    pub min: Option<[f64; 2]>,
    pub max: Option<[f64; 2]>,
}
impl AggregateState for EnvelopeAggState {}

#[inline]
fn expand(state: &mut EnvelopeAggState, g: &Geom) {
    use geo::BoundingRect;
    if let Some(r) = g.bounding_rect() {
        let lo = [r.min().x, r.min().y];
        let hi = [r.max().x, r.max().y];
        match (state.min, state.max) {
            (None, None) => {
                state.min = Some(lo);
                state.max = Some(hi);
            }
            (Some(mut m), Some(mut x)) => {
                m[0] = m[0].min(lo[0]);
                m[1] = m[1].min(lo[1]);
                x[0] = x[0].max(hi[0]);
                x[1] = x[1].max(hi[1]);
                state.min = Some(m);
                state.max = Some(x);
            }
            _ => {}
        }
    }
}

pub unsafe extern "C" fn envelope_state_size(info: duckdb_function_info) -> idx_t {
    unsafe { FfiState::<EnvelopeAggState>::size_callback(info) }
}
pub unsafe extern "C" fn envelope_state_init(info: duckdb_function_info, state: duckdb_aggregate_state) {
    unsafe { FfiState::<EnvelopeAggState>::init_callback(info, state) };
}
pub unsafe extern "C" fn envelope_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let col = unsafe { BlobCol::new(input, 0) };
    for row in 0..col.n {
        let Some(g) = read_geom(&col, row) else { continue };
        let state_ptr = unsafe { *states.add(row) };
        if let Some(st) = unsafe { FfiState::<EnvelopeAggState>::with_state_mut(state_ptr) } {
            expand(st, &g);
        }
    }
}
pub unsafe extern "C" fn envelope_combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        let src_ptr = unsafe { *source.add(i) };
        let tgt_ptr = unsafe { *target.add(i) };
        let (src, tgt) = unsafe {
            (
                FfiState::<EnvelopeAggState>::with_state(src_ptr),
                FfiState::<EnvelopeAggState>::with_state_mut(tgt_ptr),
            )
        };
        if let (Some(s), Some(t)) = (src, tgt) {
            if let (Some(sm), Some(sx)) = (s.min, s.max) {
                let g = Geometry::Polygon(geo_types::Polygon::new(
                    geo_types::LineString::from(vec![
                        (sm[0], sm[1]),
                        (sx[0], sm[1]),
                        (sx[0], sx[1]),
                        (sm[0], sx[1]),
                        (sm[0], sm[1]),
                    ]),
                    vec![],
                ));
                expand(t, &g);
            }
        }
    }
}
pub unsafe extern "C" fn envelope_finalize(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    let mut writer = unsafe { VectorWriter::new(result) };
    for i in 0..count as usize {
        let out_row = offset as usize + i;
        let state_ptr = unsafe { *source.add(i) };
        match unsafe { FfiState::<EnvelopeAggState>::with_state(state_ptr) } {
            Some(st) if st.min.is_some() && st.max.is_some() => {
                let m = st.min.unwrap();
                let x = st.max.unwrap();
                let poly = Geometry::Polygon(geo_types::Polygon::new(
                    geo_types::LineString::from(vec![
                        (m[0], m[1]),
                        (x[0], m[1]),
                        (x[0], x[1]),
                        (m[0], x[1]),
                        (m[0], m[1]),
                    ]),
                    vec![],
                ));
                match geometry::to_wkb(&poly) {
                    Ok(b) => unsafe { writer.write_blob(out_row, &b) },
                    Err(_) => unsafe { writer.set_null(out_row) },
                }
            }
            _ => unsafe { writer.set_null(out_row) },
        }
    }
}
pub unsafe extern "C" fn envelope_destroy(states: *mut duckdb_aggregate_state, count: idx_t) {
    unsafe { FfiState::<EnvelopeAggState>::destroy_callback(states, count) };
}

// ---------------------------------------------------------------------------
// ST_Union aggregate (cascaded polygonal union)
// ---------------------------------------------------------------------------

/// Per-group state for `ST_Union`: the polygonal parts seen so far, unioned
/// pairwise as they arrive (cascaded). The result is a single MultiPolygon.
#[derive(Default)]
pub struct UnionAggState {
    /// Carried union of polygonal parts; `None` until the first polygonal geom.
    pub acc: Option<MultiPolygon>,
}
impl AggregateState for UnionAggState {}

/// Reduce a geometry to its polygonal part (for boolean union). Non-polygonal
/// geometries contribute nothing (consistent with the scalar `ST_Union`).
fn polygonal_part(g: &Geom) -> MultiPolygon {
    use geo_types::Polygon;
    fn collect(g: &Geom, polys: &mut Vec<Polygon>) {
        match g {
            Geometry::Polygon(p) => polys.push(p.clone()),
            Geometry::MultiPolygon(mp) => polys.extend(mp.0.iter().cloned()),
            Geometry::GeometryCollection(c) => {
                for item in &c.0 {
                    collect(item, polys);
                }
            }
            _ => {}
        }
    }
    let mut polys = Vec::new();
    collect(g, &mut polys);
    MultiPolygon::new(polys)
}

pub unsafe extern "C" fn union_state_size(info: duckdb_function_info) -> idx_t {
    unsafe { FfiState::<UnionAggState>::size_callback(info) }
}
pub unsafe extern "C" fn union_state_init(info: duckdb_function_info, state: duckdb_aggregate_state) {
    unsafe { FfiState::<UnionAggState>::init_callback(info, state) };
}
pub unsafe extern "C" fn union_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    use geo::algorithm::bool_ops::BooleanOps;
    let col = unsafe { BlobCol::new(input, 0) };
    for row in 0..col.n {
        let Some(g) = read_geom(&col, row) else { continue };
        // SAFETY: `states` has one entry per input row.
        let state_ptr = unsafe { *states.add(row) };
        let Some(st) = (unsafe { FfiState::<UnionAggState>::with_state_mut(state_ptr) }) else { continue };
        let part = polygonal_part(&g);
        if part.0.is_empty() {
            continue;
        }
        st.acc = Some(match st.acc.take() {
            Some(acc) => acc.union(&part),
            None => part,
        });
    }
}
pub unsafe extern "C" fn union_combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    use geo::algorithm::bool_ops::BooleanOps;
    for i in 0..count as usize {
        let src_ptr = unsafe { *source.add(i) };
        let tgt_ptr = unsafe { *target.add(i) };
        let (src, tgt) = unsafe {
            (
                FfiState::<UnionAggState>::with_state(src_ptr),
                FfiState::<UnionAggState>::with_state_mut(tgt_ptr),
            )
        };
        if let (Some(s), Some(t)) = (src, tgt) {
            match (s.acc.clone(), t.acc.take()) {
                (Some(ss), Some(ts)) => t.acc = Some(ts.union(&ss)),
                (Some(ss), None) => t.acc = Some(ss),
                (None, ts) => t.acc = ts,
            }
        }
    }
}
pub unsafe extern "C" fn union_finalize(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    result: duckdb_vector,
    count: idx_t,
    offset: idx_t,
) {
    let mut writer = unsafe { VectorWriter::new(result) };
    for i in 0..count as usize {
        let out_row = offset as usize + i;
        let state_ptr = unsafe { *source.add(i) };
        match unsafe { FfiState::<UnionAggState>::with_state(state_ptr) } {
            Some(st) if st.acc.is_some() => {
                let out = Geometry::MultiPolygon(st.acc.clone().unwrap());
                match geometry::to_wkb(&out) {
                    Ok(b) => unsafe { writer.write_blob(out_row, &b) },
                    Err(_) => unsafe { writer.set_null(out_row) },
                }
            }
            _ => unsafe { writer.set_null(out_row) },
        }
    }
}
pub unsafe extern "C" fn union_destroy(states: *mut duckdb_aggregate_state, count: idx_t) {
    unsafe { FfiState::<UnionAggState>::destroy_callback(states, count) };
}
