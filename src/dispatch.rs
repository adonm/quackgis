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

use geo::algorithm::bool_ops::BooleanOps;
use geo_types::{Geometry, MultiPolygon};
use libduckdb_sys::{
    duckdb_aggregate_state, duckdb_data_chunk, duckdb_function_info, duckdb_vector, idx_t,
};
use quack_rs::aggregate::{AggregateState, FfiState};
use quack_rs::data_chunk::DataChunk;
use quack_rs::vector::{VectorReader, VectorWriter};

use crate::geometry::{self, Geom};
use crate::functions::to_multi_polygon;

/// Read & parse the geometry at `row` of a BLOB column.
///
/// The `VectorReader::read_blob` we delegate to is the binary-safe reader from
/// our vendored `quack-rs` (upstream's routed BLOBs through a UTF-8-validating
/// string reader and returned empty bytes for non-UTF-8 WKB — the bug that
/// originally forced this extension to hand-roll a `BlobCol` reader; that
/// workaround is gone now that the fix lives at the source).
fn read_geom(col: &VectorReader, row: usize) -> Option<Geom> {
    // SAFETY: callers loop `row` over `col.row_count()`, so `row` is in bounds.
    if !unsafe { col.is_valid(row) } {
        return None;
    }
    // SAFETY: as above; the column is a BLOB (ISO-WKB) column.
    let bytes = unsafe { col.read_blob(row) };
    geometry::from_wkb(bytes).ok()
}

/// Read the raw bytes at `row` of a BLOB column (validity-checked), without
/// parsing. Used by the aggregate `update` callbacks, which need the raw WKB.
pub(crate) fn read_blob(col: &VectorReader, row: usize) -> Option<&[u8]> {
    // SAFETY: callers loop `row` over `col.row_count()`.
    if !unsafe { col.is_valid(row) } {
        return None;
    }
    // SAFETY: as above; column is a BLOB column.
    Some(unsafe { col.read_blob(row) })
}

/// Read a geometry plus its EWKB SRID tag (0 when untagged). The tag is what
/// the dispatch layer propagates onto geometry outputs (PostGIS SRID
/// semantics); the parsed [`Geom`] itself is SRID-less.
fn read_geom_srid(col: &VectorReader, row: usize) -> Option<(Geom, i32)> {
    let bytes = read_blob(col, row)?;
    let srid = geometry::peek_ewkb_srid(bytes).unwrap_or(0);
    let g = geometry::from_wkb(bytes).ok()?;
    Some((g, srid))
}

/// Serialize `g` and write it at `row`, tagging the blob with `srid` when > 0.
/// This is the single write path that gives every geometry-producing local
/// function PostGIS SRID propagation (output carries the input's SRID).
fn write_geom_srid(writer: &mut VectorWriter, row: usize, g: Option<Geom>, srid: i32) {
    match g.and_then(|out| geometry::to_wkb(&out).ok()) {
        // SAFETY (both arms): `row` < row_count; output is a BLOB vector.
        Some(bytes) if srid > 0 => {
            let tagged = geometry::tag_ewkb_srid(&bytes, srid);
            unsafe { writer.write_blob(row, &tagged) }
        }
        Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
        None => unsafe { writer.set_null(row) },
    }
}

/// Vectorized executor for `(geometry, INTEGER) -> geometry` accessors
/// (`ST_GeometryN`, `ST_PointN`, `ST_InteriorRingN`).
pub fn geom_int_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, i32) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let idx = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.row_count() {
        let (Some((g, srid)), Some(i)) = (read_geom_srid(&geom, row), read_i32(&idx, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        write_geom_srid(&mut writer, row, f(&g, i), srid);
    }
}

/// Vectorized executor for `(geometry, INTEGER, geometry) -> geometry` (`ST_SetPoint`).
pub fn geom_int_geom_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, i32, &Geom) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom0 = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let idx = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let geom1 = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom0.row_count() {
        let (Some((g0, srid)), Some(i), Some(g1)) =
            (read_geom_srid(&geom0, row), read_i32(&idx, row), read_geom(&geom1, row))
        else {
            unsafe { writer.set_null(row) };
            continue;
        };
        write_geom_srid(&mut writer, row, f(&g0, i, &g1), srid);
    }
}

/// Vectorized executor for `(geometry, INTEGER) -> VARCHAR` (`ST_AsEWKT`).
pub fn geom_int_to_varchar<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, i32) -> Option<String>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let idx = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.row_count() {
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
/// (`ST_Transform(geom, from_srid, to_srid)`). The output is tagged with the
/// *second* integer (the destination SRID), not the input's tag — a reprojected
/// geometry is in the destination CRS.
pub fn geom_int2_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, i32, i32) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let a = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let b = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.row_count() {
        let (Some(g), Some(from), Some(to)) =
            (read_geom(&geom, row), read_i32(&a, row), read_i32(&b, row))
        else {
            unsafe { writer.set_null(row) };
            continue;
        };
        write_geom_srid(&mut writer, row, f(&g, from, to), to);
    }
}

/// Read an INTEGER at `row`, or `None` if NULL.
pub(crate) fn read_i32(reader: &VectorReader, row: usize) -> Option<i32> {
    if !unsafe { reader.is_valid(row) } {
        return None;
    }
    Some(unsafe { reader.read_i32(row) })
}

/// Read a BIGINT at `row`, or `None` if NULL.
pub(crate) fn read_i64(reader: &VectorReader, row: usize) -> Option<i64> {
    if !unsafe { reader.is_valid(row) } {
        return None;
    }
    Some(unsafe { reader.read_i64(row) })
}

/// Read a DOUBLE at `row`, or `None` if NULL.
pub(crate) fn read_f64(reader: &VectorReader, row: usize) -> Option<f64> {
    // SAFETY: `row` < row_count in all loops below.
    if !unsafe { reader.is_valid(row) } {
        return None;
    }
    Some(unsafe { reader.read_f64(row) })
}

/// Read a VARCHAR (`&str`) at `row`, or `None` if NULL.
pub(crate) fn read_varchar(reader: &VectorReader, row: usize) -> Option<&str> {
    // SAFETY: `row` < row_count in all loops below; column is a VARCHAR column.
    if !unsafe { reader.is_valid(row) } {
        return None;
    }
    Some(unsafe { reader.read_str(row) })
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
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
        let Some((g, srid)) = read_geom_srid(&reader, row) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        write_geom_srid(&mut writer, row, f(&g), srid);
    }
}

/// Vectorized executor for raw-WKB functions (`Fn(&[u8]) -> Option<Vec<u8>>`).
/// Used by the GEOS backend (`ST_Node`, `ST_Polygonize`, ...) which reads and
/// writes ISO WKB directly without a `geo_types` round-trip. GEOS's WKB reader
/// understands EWKB natively; we re-tag its (plain-WKB) output ourselves.
pub fn unary_wkb<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&[u8]) -> Option<Vec<u8>>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };
    for row in 0..reader.row_count() {
        match read_blob(&reader, row) {
            Some(wkb) => {
                let srid = geometry::peek_ewkb_srid(wkb).unwrap_or(0);
                match f(wkb) {
                    Some(out) if srid > 0 => {
                        let tagged = geometry::tag_ewkb_srid(&out, srid);
                        unsafe { writer.write_blob(row, &tagged) }
                    }
                    Some(out) => unsafe { writer.write_blob(row, &out) },
                    None => unsafe { writer.set_null(row) },
                }
            }
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for binary raw-WKB + double functions
/// (`Fn(&[u8], &[u8], f64) -> Option<Vec<u8>>`).
/// Used by the GEOS backend for `ST_Snap(geom, geom, tolerance)`.
pub fn binary_wkb_double<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&[u8], &[u8], f64) -> Option<Vec<u8>>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let left = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let right = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let dbl = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };
    for row in 0..left.row_count() {
        let (Some(a), Some(b), Some(tol)) = (read_blob(&left, row), read_blob(&right, row), read_f64(&dbl, row))
        else {
            unsafe { writer.set_null(row) };
            continue;
        };
        let srid = geometry::peek_ewkb_srid(a).unwrap_or(0);
        match f(a, b, tol) {
            Some(out) if srid > 0 => {
                let tagged = geometry::tag_ewkb_srid(&out, srid);
                unsafe { writer.write_blob(row, &tagged) }
            }
            Some(out) => unsafe { writer.write_blob(row, &out) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for binary geometry-producing functions
/// (`ST_Intersection`, `ST_Union`, ...). The output carries the first
/// argument's SRID tag (PostGIS requires matching SRIDs and propagates them).
pub fn binary_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, &Geom) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let left = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let right = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.row_count() {
        let (Some((a, srid)), Some(b)) = (read_geom_srid(&left, row), read_geom(&right, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        write_geom_srid(&mut writer, row, f(&a, &b), srid);
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
    let left = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let right = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.row_count() {
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

/// Vectorized executor for `(geometry, geometry) -> VARCHAR`
/// (`ST_Relate` DE-9IM matrix output).
pub fn binary_geom_varchar<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, &Geom) -> Option<String>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let left = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let right = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.row_count() {
        let (Some(a), Some(b)) = (read_geom(&left, row), read_geom(&right, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&a, &b) {
            Some(v) => unsafe { writer.write_varchar(row, &v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(geometry, geometry, VARCHAR) -> BOOLEAN`
/// (`ST_Relate(a, b, pattern)`).
pub fn geom_geom_str_predicate<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, &Geom, &str) -> Option<bool>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let left = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let right = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let pat = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.row_count() {
        let (Some(a), Some(b), Some(p)) = (
            read_geom(&left, row),
            read_geom(&right, row),
            read_varchar(&pat, row),
        ) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&a, &b, p) {
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
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
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
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
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
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
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
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
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

/// Vectorized executor for raw `VARCHAR -> BLOB` constructors that manage
/// their own serialization/tagging (`ST_GeomFromEWKT` with SRID preservation).
pub fn str_to_blob<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&str) -> Option<Vec<u8>>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
        if !unsafe { reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let s = unsafe { reader.read_str(row) };
        match f(s) {
            Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(VARCHAR, INTEGER) -> BLOB` constructors
/// (`ST_GeomFromText(wkt, srid)`).
pub fn str_int_to_blob<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&str, i32) -> Option<Vec<u8>>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let idx = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
        let Some(i) = read_i32(&idx, row) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        if !unsafe { reader.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let s = unsafe { reader.read_str(row) };
        match f(s, i) {
            Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for raw `(BLOB, INTEGER) -> BLOB` functions that manage
/// their own tagging (`ST_SetSRID`, `ST_Transform(geom, to_srid)`,
/// `ST_GeomFromWKB(wkb, srid)`). No automatic SRID propagation: the function
/// itself decides the output tag.
pub fn wkb_int_to_blob<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&[u8], i32) -> Option<Vec<u8>>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let blob = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let idx = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..blob.row_count() {
        let (Some(bytes), Some(i)) = (read_blob(&blob, row), read_i32(&idx, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(bytes, i) {
            Some(out) => unsafe { writer.write_blob(row, &out) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for raw `BLOB -> INTEGER` accessors (`ST_SRID`).
pub fn wkb_to_int<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&[u8]) -> Option<i32>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
        match read_blob(&reader, row).and_then(&f) {
            Some(v) => unsafe { writer.write_i32(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for raw `BLOB -> VARCHAR` accessors that need the tag
/// bytes (`ST_AsEWKT(geom)` reading the EWKB SRID).
pub fn wkb_to_varchar<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&[u8]) -> Option<String>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let reader = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..reader.row_count() {
        match read_blob(&reader, row).and_then(&f) {
            Some(v) => unsafe { writer.write_varchar(row, &v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(geometry, VARCHAR) -> DOUBLE` measurements
/// (`ST_LengthSpheroid(geom, spheroid)`).
pub fn geom_str_to_double<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, &str) -> Option<f64>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let txt = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.row_count() {
        let (Some(g), Some(s)) = (read_geom(&geom, row), read_varchar(&txt, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&g, s) {
            Some(v) => unsafe { writer.write_f64(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(geometry, geometry, VARCHAR) -> DOUBLE`
/// measurements (`ST_DistanceSpheroid(a, b, spheroid)`).
pub fn geom_geom_str_to_double<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, &Geom, &str) -> Option<f64>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let left = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let right = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let txt = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.row_count() {
        let (Some(a), Some(b), Some(s)) =
            (read_geom(&left, row), read_geom(&right, row), read_varchar(&txt, row))
        else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&a, &b, s) {
            Some(v) => unsafe { writer.write_f64(row, v) },
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
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let scalar = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.row_count() {
        let (Some((g, srid)), Some(v)) = (read_geom_srid(&geom, row), read_f64(&scalar, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        write_geom_srid(&mut writer, row, f(&g, v), srid);
    }
}

/// Vectorized executor for `(geometry, DOUBLE, DOUBLE) -> geometry` transforms
/// (`ST_Translate`, `ST_Scale`).
pub fn geom_double2_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, f64, f64) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let s1 = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let s2 = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.row_count() {
        let (Some((g, srid)), Some(a), Some(b)) =
            (read_geom_srid(&geom, row), read_f64(&s1, row), read_f64(&s2, row))
        else {
            unsafe { writer.set_null(row) };
            continue;
        };
        write_geom_srid(&mut writer, row, f(&g, a, b), srid);
    }
}

/// Vectorized executor for `(geometry, DOUBLE×6) -> geometry` transforms
/// (`ST_Affine` 2D: a, b, d, e, xoff, yoff).
pub fn geom_double6_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, f64, f64, f64, f64, f64, f64) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let s = [
        unsafe { VectorReader::new(chunk.as_raw(), 1) },
        unsafe { VectorReader::new(chunk.as_raw(), 2) },
        unsafe { VectorReader::new(chunk.as_raw(), 3) },
        unsafe { VectorReader::new(chunk.as_raw(), 4) },
        unsafe { VectorReader::new(chunk.as_raw(), 5) },
        unsafe { VectorReader::new(chunk.as_raw(), 6) },
    ];
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..geom.row_count() {
        let Some((g, srid)) = read_geom_srid(&geom, row) else {
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
        write_geom_srid(&mut writer, row, f(&g, a, b, c, d, e, ff), srid);
    }
}

/// Vectorized executor for `(DOUBLE×4) -> geometry` constructors
/// (`ST_MakeEnvelope`).
pub fn doubles4_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(f64, f64, f64, f64) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let s = [
        unsafe { VectorReader::new(chunk.as_raw(), 0) },
        unsafe { VectorReader::new(chunk.as_raw(), 1) },
        unsafe { VectorReader::new(chunk.as_raw(), 2) },
        unsafe { VectorReader::new(chunk.as_raw(), 3) },
    ];
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..s[0].row_count() {
        let vals: Option<[f64; 4]> = s
            .iter()
            .map(|r| read_f64(r, row))
            .collect::<Option<Vec<_>>>()
            .map(|v| [v[0], v[1], v[2], v[3]]);
        let Some([a, b, c, d]) = vals else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(a, b, c, d).and_then(|g| geometry::to_wkb(&g).ok()) {
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
    let left = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let right = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.row_count() {
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
    let left = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let right = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let scalar = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.row_count() {
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

/// Vectorized executor for `(geometry, geometry, DOUBLE) -> geometry`
/// (`ST_Snap`).
pub fn geom_geom_double_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, &Geom, f64) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let left = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let right = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let scalar = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };

    for row in 0..left.row_count() {
        let (Some((a, srid)), Some(b)) = (read_geom_srid(&left, row), read_geom(&right, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        let Some(t) = read_f64(&scalar, row) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        write_geom_srid(&mut writer, row, f(&a, &b, t), srid);
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
    let col = unsafe { VectorReader::new(input, 0) };
    for row in 0..col.row_count() {
        let Some(bytes) = read_blob(&col, row) else { continue };
        let Ok(g) = geometry::from_wkb(bytes) else { continue };
        // SAFETY: `states` has one entry per input row. With ORDER BY, some
        // state slots may be uninitialized by DuckDB (inner = null); skip
        // them rather than crashing. Use the rewriter to pre-sort via subquery.
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
    let col = unsafe { VectorReader::new(input, 0) };
    for row in 0..col.row_count() {
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
    let col = unsafe { VectorReader::new(input, 0) };
    for row in 0..col.row_count() {
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

// ---------------------------------------------------------------------------
// ST_Intersection aggregate (cascaded polygonal intersection)
// ---------------------------------------------------------------------------

/// Per-group state for `ST_Intersection` aggregate: the running intersection
/// as an optional `MultiPolygon`. `None` means "no geometries seen yet"
/// (the identity for intersection is the universe — the first geometry
/// initializes, subsequent geometries are intersected in).
#[derive(Default)]
pub struct IntersectionAggState {
    pub mp: Option<MultiPolygon>,
}
impl AggregateState for IntersectionAggState {}

#[inline]
fn expand_intersection(state: &mut IntersectionAggState, g: &Geom) {
    let incoming = to_multi_polygon(g);
    match &mut state.mp {
        None => state.mp = Some(incoming),
        Some(current) => {
            if current.0.is_empty() {
                // Keep current empty — intersection with anything is empty.
            } else if incoming.0.is_empty() {
                // Incoming is empty → intersection is empty.
                current.0.clear();
            } else {
                let new_mp = current.intersection(&incoming);
                *current = new_mp;
            }
        }
    }
}

pub unsafe extern "C" fn intersection_state_size(info: duckdb_function_info) -> idx_t {
    unsafe { FfiState::<IntersectionAggState>::size_callback(info) }
}

pub unsafe extern "C" fn intersection_state_init(info: duckdb_function_info, state: duckdb_aggregate_state) {
    unsafe { FfiState::<IntersectionAggState>::init_callback(info, state) };
}

pub unsafe extern "C" fn intersection_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let col = unsafe { VectorReader::new(input, 0) };
    for row in 0..col.row_count() {
        let Some(bytes) = read_blob(&col, row) else { continue };
        let Ok(g) = geometry::from_wkb(bytes) else { continue };
        let state_ptr = unsafe { *states.add(row) };
        if let Some(st) = unsafe { FfiState::<IntersectionAggState>::with_state_mut(state_ptr) } {
            expand_intersection(st, &g);
        }
    }
}

pub unsafe extern "C" fn intersection_combine(
    _info: duckdb_function_info,
    source: *mut duckdb_aggregate_state,
    target: *mut duckdb_aggregate_state,
    count: idx_t,
) {
    for i in 0..count as usize {
        let src_ptr = unsafe { *source.add(i) };
        let tgt_ptr = unsafe { *target.add(i) };
        let src = unsafe { FfiState::<IntersectionAggState>::with_state(src_ptr) };
        let tgt = unsafe { FfiState::<IntersectionAggState>::with_state_mut(tgt_ptr) };
        if let (Some(s), Some(t)) = (src, tgt) {
            match (&s.mp, &mut t.mp) {
                (None, _) => {} // source has no data; skip
                (Some(src_mp), None) => t.mp = Some(src_mp.clone()),
                (Some(src_mp), Some(tgt_mp)) => {
                    if src_mp.0.is_empty() || tgt_mp.0.is_empty() {
                        tgt_mp.0.clear();
                    } else {
                        let new_mp = tgt_mp.intersection(src_mp);
                        *tgt_mp = new_mp;
                    }
                }
            }
        }
    }
}

pub unsafe extern "C" fn intersection_finalize(
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
        match unsafe { FfiState::<IntersectionAggState>::with_state(state_ptr) } {
            Some(st) => match &st.mp {
                Some(mp) if !mp.0.is_empty() => {
                    let g = Geometry::MultiPolygon(mp.clone());
                    match geometry::to_wkb(&g) {
                        Ok(b) => unsafe { writer.write_blob(out_row, &b) },
                        Err(_) => unsafe { writer.set_null(out_row) },
                    }
                }
                Some(_) => {
                    // Empty intersection → NULL (matches PostGIS behaviour for
                    // disjoint inputs).
                    unsafe { writer.set_null(out_row) };
                }
                None => unsafe { writer.set_null(out_row) },
            },
            None => unsafe { writer.set_null(out_row) },
        }
    }
}

pub unsafe extern "C" fn intersection_destroy(states: *mut duckdb_aggregate_state, count: idx_t) {
    unsafe { FfiState::<IntersectionAggState>::destroy_callback(states, count) };
}

// ---------------------------------------------------------------------------
// ST_MakeLine aggregate (points → LineString)
// ---------------------------------------------------------------------------

/// Per-group state for `ST_MakeLine`: the accumulated vertex sequence. Points
/// contribute their single coordinate; LineStrings contribute all of theirs.
/// Non-point/linestring inputs are ignored (PostGIS only combines points/lines).
#[derive(Default)]
pub struct MakeLineAggState {
    pub coords: Vec<geo_types::Coord<f64>>,
}
impl AggregateState for MakeLineAggState {}

/// Append the coordinates of a point/linestring-bearing geometry to `coords`.
fn append_line_coords(g: &Geom, coords: &mut Vec<geo_types::Coord<f64>>) {
    match g {
        Geometry::Point(p) => coords.push(p.0),
        Geometry::MultiPoint(mp) => coords.extend(mp.0.iter().map(|p| p.0)),
        Geometry::LineString(ls) => coords.extend(ls.0.iter().copied()),
        Geometry::MultiLineString(mls) => {
            for ls in &mls.0 {
                coords.extend(ls.0.iter().copied());
            }
        }
        Geometry::GeometryCollection(c) => {
            for item in &c.0 {
                append_line_coords(item, coords);
            }
        }
        _ => {}
    }
}

pub unsafe extern "C" fn make_line_state_size(info: duckdb_function_info) -> idx_t {
    unsafe { FfiState::<MakeLineAggState>::size_callback(info) }
}
pub unsafe extern "C" fn make_line_state_init(info: duckdb_function_info, state: duckdb_aggregate_state) {
    unsafe { FfiState::<MakeLineAggState>::init_callback(info, state) };
}
pub unsafe extern "C" fn make_line_update(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    states: *mut duckdb_aggregate_state,
) {
    let col = unsafe { VectorReader::new(input, 0) };
    for row in 0..col.row_count() {
        let Some(g) = read_geom(&col, row) else { continue };
        // SAFETY: `states` has one entry per input row.
        let state_ptr = unsafe { *states.add(row) };
        let Some(st) = (unsafe { FfiState::<MakeLineAggState>::with_state_mut(state_ptr) }) else { continue };
        append_line_coords(&g, &mut st.coords);
    }
}
pub unsafe extern "C" fn make_line_combine(
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
                FfiState::<MakeLineAggState>::with_state(src_ptr),
                FfiState::<MakeLineAggState>::with_state_mut(tgt_ptr),
            )
        };
        if let (Some(s), Some(t)) = (src, tgt) {
            t.coords.extend(s.coords.iter().copied());
        }
    }
}
pub unsafe extern "C" fn make_line_finalize(
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
        match unsafe { FfiState::<MakeLineAggState>::with_state(state_ptr) } {
            Some(st) if st.coords.len() >= 2 => {
                let ls = Geometry::LineString(geo_types::LineString(st.coords.clone()));
                match geometry::to_wkb(&ls) {
                    Ok(b) => unsafe { writer.write_blob(out_row, &b) },
                    Err(_) => unsafe { writer.set_null(out_row) },
                }
            }
            _ => unsafe { writer.set_null(out_row) },
        }
    }
}

/// Vectorized executor for `(geometry, INTEGER) -> BIGINT` (Hilbert/Morton keys).
pub fn geom_int_to_i64<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(&Geom, i32) -> Option<i64>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let geom = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let bits = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let mut writer = unsafe { VectorWriter::new(output) };
    for row in 0..geom.row_count() {
        let (Some(g), Some(b)) = (read_geom(&geom, row), read_i32(&bits, row)) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(&g, b) {
            Some(v) => unsafe { writer.write_i64(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(INTEGER, INTEGER, INTEGER) -> geometry` (TileEnvelope).
pub fn int3_to_geom<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(i32, i32, i32) -> Option<Geom>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let c0 = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let c1 = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let c2 = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };
    for row in 0..c0.row_count() {
        let (Some(z), Some(x), Some(y)) =
            (read_i32(&c0, row), read_i32(&c1, row), read_i32(&c2, row))
        else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(z, x, y).and_then(|g| geometry::to_wkb(&g).ok()) {
            Some(bytes) => unsafe { writer.write_blob(row, &bytes) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(BIGINT, INT, BIGINT) -> INT` (partition estimation).
pub fn i64_i32_i64_to_i32<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(i64, i32, i64) -> Option<i32>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let c0 = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let c1 = unsafe { VectorReader::new(chunk.as_raw(), 1) };
    let c2 = unsafe { VectorReader::new(chunk.as_raw(), 2) };
    let mut writer = unsafe { VectorWriter::new(output) };
    for row in 0..c0.row_count() {
        let (Some(total), Some(avg), Some(tgt)) = (
            read_i64(&c0, row),
            read_i32(&c1, row),
            read_i64(&c2, row),
        ) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match f(total, avg, tgt) {
            Some(v) => unsafe { writer.write_i32(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

/// Vectorized executor for `(INT) -> INT` (recommend zoom).
pub fn i32_to_i32<F>(input: duckdb_data_chunk, output: duckdb_vector, f: F)
where
    F: Fn(i32) -> Option<i32>,
{
    let chunk = unsafe { DataChunk::from_raw(input) };
    let c0 = unsafe { VectorReader::new(chunk.as_raw(), 0) };
    let mut writer = unsafe { VectorWriter::new(output) };
    for row in 0..c0.row_count() {
        match read_i32(&c0, row).and_then(&f) {
            Some(v) => unsafe { writer.write_i32(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}

pub unsafe extern "C" fn make_line_destroy(states: *mut duckdb_aggregate_state, count: idx_t) {
    unsafe { FfiState::<MakeLineAggState>::destroy_callback(states, count) };
}
