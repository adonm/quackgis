// SPDX-License-Identifier: Apache-2.0
//
//! Set-returning "dump" table functions — the one FFI shape the extension did
//! not yet have.
//!
//!   * `st_dump(geom)`        → `(path VARCHAR, geom BLOB)` — one row per
//!     *atomic* geometry (collections / multis exploded), PostGIS path layout.
//!   * `st_dumppoints(geom)`  → `(path VARCHAR, npt BIGINT, geom BLOB)` — one
//!     row per vertex, as a `POINT`.
//!   * `st_dumpsegments(geom)` → `(path VARCHAR, geom BLOB)` — one row per
//!     edge, as a 2-point `LINESTRING`.
//!
//! The geometry-exploding core is plain Rust (no FFI), so it is unit-tested
//! directly. The `bind` callback reads the BLOB geometry argument via the
//! `Value::as_blob`, runs the core, and stages `(path, wkb)` rows that
//! `init`/`scan` stream back to DuckDB.
//!
//! All three scan callbacks are **parallel-safe** via an `AtomicUsize` cursor
//! that atomically claims the next `vector_size`-row batch.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use geo_types::Geometry;
use libduckdb_sys::{
    duckdb_bind_info, duckdb_data_chunk, duckdb_function_info, duckdb_init_info,
};
use quack_rs::data_chunk::DataChunk;
use quack_rs::table::{BindInfo, FfiBindData, FfiInitData, InitInfo};
use quack_rs::types::TypeId;
use quack_rs::value::Value;

use crate::geometry::{self, Geom};

// =====================================================================
// Pure-Rust dump cores (unit-tested).
// =====================================================================

/// Format a PostGIS-style path: `{1,2,3}`.
fn format_path(path: &[i32]) -> String {
    let mut s = String::from("{");
    for (i, n) in path.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&n.to_string());
    }
    s.push('}');
    s
}

/// Whether a geometry is "atomic" for ST_Dump (not a multi/collection).
fn is_atomic(g: &Geom) -> bool {
    !matches!(
        g,
        Geometry::MultiPoint(_)
            | Geometry::MultiLineString(_)
            | Geometry::MultiPolygon(_)
            | Geometry::GeometryCollection(_)
    )
}

/// `ST_Dump` — explode into atomic geometries with PostGIS path semantics.
///
/// A bare atomic geometry yields a single row at path `{1}`. A multi/collection
/// yields one row per *direct* atomic child at `{1}`,`{2}`,…; nesting adds a
/// path element per level (so a collection inside a collection produces
/// `{1,1}`,`{1,2}`,…).
pub fn dump(geom: &Geom) -> Vec<(Vec<i32>, Geom)> {
    let mut out = Vec::new();
    if is_atomic(geom) {
        out.push((vec![1], geom.clone()));
    } else {
        for (i, child) in children(geom).into_iter().enumerate() {
            dump_into(&child, vec![i as i32 + 1], &mut out);
        }
    }
    out
}

fn dump_into(geom: &Geom, path: Vec<i32>, out: &mut Vec<(Vec<i32>, Geom)>) {
    if is_atomic(geom) {
        out.push((path, geom.clone()));
    } else {
        for (i, child) in children(geom).into_iter().enumerate() {
            let mut p = path.clone();
            p.push(i as i32 + 1);
            dump_into(&child, p, out);
        }
    }
}

/// The direct children of a multi/collection geometry.
fn children(g: &Geom) -> Vec<Geom> {
    match g {
        Geometry::MultiPoint(mp) => mp.0.iter().copied().map(Geometry::Point).collect(),
        Geometry::MultiLineString(mls) => mls.0.iter().cloned().map(Geometry::LineString).collect(),
        Geometry::MultiPolygon(mp) => mp.0.iter().cloned().map(Geometry::Polygon).collect(),
        Geometry::GeometryCollection(c) => c.0.clone(),
        _ => Vec::new(),
    }
}

/// `ST_DumpPoints` — one row per vertex (as a `POINT`), with a path that
/// navigates to the containing ring/linestring and the 1-based vertex index.
/// `npt` is the 1-based absolute vertex sequence number across the geometry.
pub fn dump_points(geom: &Geom) -> Vec<(Vec<i32>, i64, Geom)> {
    let mut out = Vec::new();
    let mut npt = 0_i64;
    dump_points_into(geom, &mut Vec::new(), &mut npt, &mut out);
    out
}

fn dump_points_into(
    geom: &Geom,
    prefix: &mut Vec<i32>,
    npt: &mut i64,
    out: &mut Vec<(Vec<i32>, i64, Geom)>,
) {
    match geom {
        Geometry::Point(p) => {
            prefix.push(1);
            *npt += 1;
            out.push((prefix.clone(), *npt, Geometry::Point(*p)));
            prefix.pop();
        }
        Geometry::LineString(ls) => {
            emit_ring(&ls.0, prefix, npt, out);
        }
        Geometry::Polygon(p) => {
            for (ri, ring) in std::iter::once(p.exterior())
                .chain(p.interiors().iter())
                .enumerate()
            {
                prefix.push(ri as i32 + 1);
                emit_ring(&ring.0, prefix, npt, out);
                prefix.pop();
            }
        }
        Geometry::MultiPoint(mp) => {
            for (i, p) in mp.0.iter().enumerate() {
                prefix.push(i as i32 + 1);
                dump_points_into(&Geometry::Point(*p), prefix, npt, out);
                prefix.pop();
            }
        }
        Geometry::MultiLineString(mls) => {
            for (i, ls) in mls.0.iter().enumerate() {
                prefix.push(i as i32 + 1);
                dump_points_into(&Geometry::LineString(ls.clone()), prefix, npt, out);
                prefix.pop();
            }
        }
        Geometry::MultiPolygon(mp) => {
            for (i, p) in mp.0.iter().enumerate() {
                prefix.push(i as i32 + 1);
                dump_points_into(&Geometry::Polygon(p.clone()), prefix, npt, out);
                prefix.pop();
            }
        }
        Geometry::GeometryCollection(c) => {
            for (i, item) in c.0.iter().enumerate() {
                prefix.push(i as i32 + 1);
                dump_points_into(item, prefix, npt, out);
                prefix.pop();
            }
        }
        Geometry::Line(l) => {
            prefix.push(1);
            emit_ring(&[l.start, l.end], prefix, npt, out);
            prefix.pop();
        }
        Geometry::Rect(r) => {
            prefix.push(1);
            emit_ring(
                &[r.min(), r.max()],
                prefix,
                npt,
                out,
            );
            prefix.pop();
        }
        Geometry::Triangle(t) => {
            prefix.push(1);
            let v = t.to_array();
            emit_ring(&v, prefix, npt, out);
            prefix.pop();
        }
    }
}

/// Emit one row per vertex of a coordinate sequence, vertex index 1-based.
fn emit_ring(
    coords: &[geo_types::Coord<f64>],
    prefix: &mut Vec<i32>,
    npt: &mut i64,
    out: &mut Vec<(Vec<i32>, i64, Geom)>,
) {
    for (vi, c) in coords.iter().enumerate() {
        prefix.push(vi as i32 + 1);
        *npt += 1;
        out.push((
            prefix.clone(),
            *npt,
            Geometry::Point(geo_types::Point::from(*c)),
        ));
        prefix.pop();
    }
}

/// `ST_DumpSegments` — one row per edge (as a 2-point `LINESTRING`), with a
/// path navigating to the containing ring/linestring and the 1-based edge index.
pub fn dump_segments(geom: &Geom) -> Vec<(Vec<i32>, Geom)> {
    let mut out = Vec::new();
    dump_segments_into(geom, &mut Vec::new(), &mut out);
    out
}

fn dump_segments_into(geom: &Geom, prefix: &mut Vec<i32>, out: &mut Vec<(Vec<i32>, Geom)>) {
    match geom {
        Geometry::LineString(ls) => emit_segments(&ls.0, prefix, out),
        Geometry::Polygon(p) => {
            for (ri, ring) in std::iter::once(p.exterior())
                .chain(p.interiors().iter())
                .enumerate()
            {
                prefix.push(ri as i32 + 1);
                emit_segments(&ring.0, prefix, out);
                prefix.pop();
            }
        }
        Geometry::MultiLineString(mls) => {
            for (i, ls) in mls.0.iter().enumerate() {
                prefix.push(i as i32 + 1);
                dump_segments_into(&Geometry::LineString(ls.clone()), prefix, out);
                prefix.pop();
            }
        }
        Geometry::MultiPolygon(mp) => {
            for (i, p) in mp.0.iter().enumerate() {
                prefix.push(i as i32 + 1);
                dump_segments_into(&Geometry::Polygon(p.clone()), prefix, out);
                prefix.pop();
            }
        }
        Geometry::GeometryCollection(c) => {
            for (i, item) in c.0.iter().enumerate() {
                prefix.push(i as i32 + 1);
                dump_segments_into(item, prefix, out);
                prefix.pop();
            }
        }
        Geometry::MultiPoint(_) | Geometry::Point(_) => { /* no edges */ }
        Geometry::Line(l) => {
            prefix.push(1);
            push_segment(l.start, l.end, prefix, out);
            prefix.pop();
        }
        Geometry::Rect(r) => {
            prefix.push(1);
            let lo = r.min();
            let hi = r.max();
            // Four boundary edges of the rectangle.
            for (a, b) in [
                (lo, geo_types::Coord { x: hi.x, y: lo.y }),
                (geo_types::Coord { x: hi.x, y: lo.y }, hi),
                (hi, geo_types::Coord { x: lo.x, y: hi.y }),
                (geo_types::Coord { x: lo.x, y: hi.y }, lo),
            ] {
                push_segment(a, b, prefix, out);
            }
            prefix.pop();
        }
        Geometry::Triangle(t) => {
            prefix.push(1);
            let v = t.to_array();
            let n = v.len();
            for i in 0..n {
                let a = v[i];
                let b = v[(i + 1) % n];
                push_segment(a, b, prefix, out);
            }
            prefix.pop();
        }
    }
}

fn emit_segments(
    coords: &[geo_types::Coord<f64>],
    prefix: &mut Vec<i32>,
    out: &mut Vec<(Vec<i32>, Geom)>,
) {
    for w in coords.windows(2) {
        push_segment(w[0], w[1], prefix, out);
    }
}

fn push_segment(
    a: geo_types::Coord<f64>,
    b: geo_types::Coord<f64>,
    prefix: &mut Vec<i32>,
    out: &mut Vec<(Vec<i32>, Geom)>,
) {
    // Edge index = current count of segments under this prefix + 1.
    let idx = out.iter().filter(|(p, _)| p.starts_with(prefix.as_slice())).count() as i32 + 1;
    let mut path = prefix.clone();
    path.push(idx);
    out.push((
        path,
        Geometry::LineString(geo_types::LineString::from(vec![
            (a.x, a.y),
            (b.x, b.y),
        ])),
    ));
}

// =====================================================================
// FFI bind / init / scan.
// =====================================================================

/// Read & parse the geometry BLOB passed as positional parameter `idx`.
/// Reports a SQL error on NULL / unparseable input.
unsafe fn read_geom_param(bi: &BindInfo, idx: u64, fn_name: &str) -> Option<Geom> {
    let val = unsafe { bi.get_parameter_value(idx) };
    if val.is_null() {
        bi.set_error(&format!("{fn_name}: NULL geometry"));
        return None;
    }
    let bytes = match val.as_blob() {
        Ok(b) => b,
        Err(e) => {
            bi.set_error(&format!("{fn_name}: {e}"));
            return None;
        }
    };
    match geometry::from_wkb(&bytes) {
        Ok(g) => Some(g),
        Err(e) => {
            bi.set_error(&format!("{fn_name}: {e}"));
            None
        }
    }
}

/// Shared scan state: an atomic cursor into an Arc-wrapped rows slice.
/// Each DuckDB worker thread atomically claims the next `vector_size`-row
/// batch — lock-free, no local-init needed.
struct ScanCursor {
    cursor: AtomicUsize,
}

// ----- ST_Dump -----------------------------------------------------------

pub struct DumpBind {
    rows: Arc<[(String, Vec<u8>)]>,
}

pub unsafe extern "C" fn dump_bind(info: duckdb_bind_info) {
    let bi = unsafe { BindInfo::new(info) };
    let Some(g) = (unsafe { read_geom_param(&bi, 0, "st_dump") }) else {
        return;
    };
    let mut rows = Vec::new();
    for (path, geom) in dump(&g) {
        let wkb = geometry::to_wkb(&geom).unwrap_or_default();
        rows.push((format_path(&path), wkb));
    }
    bi.add_result_column("path", TypeId::Varchar)
        .add_result_column("geom", TypeId::Blob)
        .set_cardinality(rows.len() as u64, true);
    unsafe { FfiBindData::<DumpBind>::set(info, DumpBind { rows: Arc::from(rows) }) };
}

pub unsafe extern "C" fn dump_init(info: duckdb_init_info) {
    unsafe { FfiInitData::<ScanCursor>::set(info, ScanCursor { cursor: AtomicUsize::new(0) }) };
}

pub unsafe extern "C" fn dump_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let cap = unsafe { libduckdb_sys::duckdb_vector_size() } as usize;
    let Some(data) = (unsafe { FfiBindData::<DumpBind>::get_from_function(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let Some(state) = (unsafe { FfiInitData::<ScanCursor>::get(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let start = state.cursor.fetch_add(cap, Ordering::Relaxed);
    if start >= data.rows.len() {
        unsafe { chunk.set_size(0) };
        return;
    }
    let batch = (data.rows.len() - start).min(cap);
    let mut w0 = unsafe { chunk.writer(0) };
    let mut w1 = unsafe { chunk.writer(1) };
    for i in 0..batch {
        let (path, geom) = &data.rows[start + i];
        unsafe { w0.write_varchar(i, path) };
        unsafe { w1.write_blob(i, geom) };
    }
    unsafe { chunk.set_size(batch) };
    drop((w0, w1));
}

// ----- ST_DumpPoints -----------------------------------------------------

pub struct DumpPointsBind {
    rows: Arc<[(String, i64, Vec<u8>)]>,
}

pub unsafe extern "C" fn dump_points_bind(info: duckdb_bind_info) {
    let bi = unsafe { BindInfo::new(info) };
    let Some(g) = (unsafe { read_geom_param(&bi, 0, "st_dumppoints") }) else {
        return;
    };
    let mut rows = Vec::new();
    for (path, npt, geom) in dump_points(&g) {
        let wkb = geometry::to_wkb(&geom).unwrap_or_default();
        rows.push((format_path(&path), npt, wkb));
    }
    bi.add_result_column("path", TypeId::Varchar)
        .add_result_column("npt", TypeId::BigInt)
        .add_result_column("geom", TypeId::Blob)
        .set_cardinality(rows.len() as u64, true);
    unsafe { FfiBindData::<DumpPointsBind>::set(info, DumpPointsBind { rows: Arc::from(rows) }) };
}

pub unsafe extern "C" fn dump_points_init(info: duckdb_init_info) {
    unsafe { FfiInitData::<ScanCursor>::set(info, ScanCursor { cursor: AtomicUsize::new(0) }) };
}

pub unsafe extern "C" fn dump_points_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let cap = unsafe { libduckdb_sys::duckdb_vector_size() } as usize;
    let Some(data) = (unsafe { FfiBindData::<DumpPointsBind>::get_from_function(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let Some(state) = (unsafe { FfiInitData::<ScanCursor>::get(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let start = state.cursor.fetch_add(cap, Ordering::Relaxed);
    if start >= data.rows.len() {
        unsafe { chunk.set_size(0) };
        return;
    }
    let batch = (data.rows.len() - start).min(cap);
    let mut c0 = unsafe { chunk.writer(0) };
    let mut c1 = unsafe { chunk.writer(1) };
    let mut c2 = unsafe { chunk.writer(2) };
    for i in 0..batch {
        let (path, npt, geom) = &data.rows[start + i];
        unsafe { c0.write_varchar(i, path) };
        unsafe { c1.write_i64(i, *npt) };
        unsafe { c2.write_blob(i, geom) };
    }
    unsafe { chunk.set_size(batch) };
    drop((c0, c1, c2));
}

// ----- ST_DumpSegments ---------------------------------------------------

pub struct DumpSegmentsBind {
    rows: Arc<[(String, Vec<u8>)]>,
}

pub unsafe extern "C" fn dump_segments_bind(info: duckdb_bind_info) {
    let bi = unsafe { BindInfo::new(info) };
    let Some(g) = (unsafe { read_geom_param(&bi, 0, "st_dumpsegments") }) else {
        return;
    };
    let mut rows = Vec::new();
    for (path, geom) in dump_segments(&g) {
        let wkb = geometry::to_wkb(&geom).unwrap_or_default();
        rows.push((format_path(&path), wkb));
    }
    bi.add_result_column("path", TypeId::Varchar)
        .add_result_column("geom", TypeId::Blob)
        .set_cardinality(rows.len() as u64, true);
    unsafe { FfiBindData::<DumpSegmentsBind>::set(info, DumpSegmentsBind { rows: Arc::from(rows) }) };
}

pub unsafe extern "C" fn dump_segments_init(info: duckdb_init_info) {
    unsafe { FfiInitData::<ScanCursor>::set(info, ScanCursor { cursor: AtomicUsize::new(0) }) };
}

pub unsafe extern "C" fn dump_segments_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let cap = unsafe { libduckdb_sys::duckdb_vector_size() } as usize;
    let Some(data) = (unsafe { FfiBindData::<DumpSegmentsBind>::get_from_function(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let Some(state) = (unsafe { FfiInitData::<ScanCursor>::get(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let start = state.cursor.fetch_add(cap, Ordering::Relaxed);
    if start >= data.rows.len() {
        unsafe { chunk.set_size(0) };
        return;
    }
    let batch = (data.rows.len() - start).min(cap);
    let mut c0 = unsafe { chunk.writer(0) };
    let mut c1 = unsafe { chunk.writer(1) };
    for i in 0..batch {
        let (path, geom) = &data.rows[start + i];
        unsafe { c0.write_varchar(i, path) };
        unsafe { c1.write_blob(i, geom) };
    }
    unsafe { chunk.set_size(batch) };
    drop((c0, c1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::geom_from_text as parse;

    fn g(wkt: &str) -> Geom {
        parse(wkt).expect("parse")
    }

    #[test]
    fn dump_atomic_polygon_is_single_row_path_1() {
        let p = g("POLYGON((0 0,1 0,1 1,0 0))");
        let rows = dump(&p);
        assert_eq!(rows.len(), 1);
        assert_eq!(format_path(&rows[0].0), "{1}");
    }

    #[test]
    fn dump_multipolygon_explodes_to_direct_children() {
        let mp = g("MULTIPOLYGON(((0 0,1 0,1 1,0 0)),((2 2,3 2,3 3,2 2)))");
        let rows = dump(&mp);
        assert_eq!(rows.len(), 2);
        assert_eq!(format_path(&rows[0].0), "{1}");
        assert_eq!(format_path(&rows[1].0), "{2}");
    }

    #[test]
    fn dump_nested_collection_accumulates_path() {
        let gc = g("GEOMETRYCOLLECTION(MULTIPOINT(1 2,3 4))");
        let rows = dump(&gc);
        assert_eq!(rows.len(), 2);
        assert_eq!(format_path(&rows[0].0), "{1,1}");
        assert_eq!(format_path(&rows[1].0), "{1,2}");
    }

    #[test]
    fn dump_mixed_collection() {
        // Matches the PostGIS reference example.
        let gc = g("GEOMETRYCOLLECTION(POINT(1 2),MULTILINESTRING((3 4,5 6),(7 8,9 10)),POLYGON((0 0,0 1,1 1,0 0)))");
        let rows = dump(&gc);
        let paths: Vec<String> = rows.iter().map(|(p, _)| format_path(p)).collect();
        assert_eq!(
            paths,
            vec!["{1}".to_string(), "{2,1}".to_string(), "{2,2}".to_string(), "{3}".to_string()]
        );
    }

    #[test]
    fn dump_points_linestring() {
        let ls = g("LINESTRING(0 0,1 1,2 2)");
        let rows = dump_points(&ls);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].1, 1);
        assert_eq!(rows[2].1, 3);
        assert_eq!(format_path(&rows[1].0), "{2}");
    }

    #[test]
    fn dump_points_polygon_exterior_then_holes() {
        let p = g("POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))");
        let rows = dump_points(&p);
        // 5 exterior + 5 hole = 10 vertices.
        assert_eq!(rows.len(), 10);
        assert_eq!(format_path(&rows[0].0), "{1,1}");
        assert_eq!(format_path(&rows[4].0), "{1,5}");
        assert_eq!(format_path(&rows[5].0), "{2,1}");
    }

    #[test]
    fn dump_points_point_is_path_1() {
        let p = g("POINT(1 2)");
        let rows = dump_points(&p);
        assert_eq!(rows.len(), 1);
        assert_eq!(format_path(&rows[0].0), "{1}");
    }

    #[test]
    fn dump_segments_linestring() {
        let ls = g("LINESTRING(0 0,1 1,2 2)");
        let rows = dump_segments(&ls);
        assert_eq!(rows.len(), 2);
        assert_eq!(format_path(&rows[0].0), "{1}");
        assert_eq!(format_path(&rows[1].0), "{2}");
    }

    #[test]
    fn dump_segments_polygon_has_ring_paths() {
        let p = g("POLYGON((0 0,1 0,1 1,0 0))");
        let rows = dump_segments(&p);
        // Closed ring of 4 vertices → 3 edges.
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|(path, _)| path.starts_with(&[1])));
    }
}

// =====================================================================
// ST_DumpRings
// =====================================================================

/// Extract rings from a polygon-containing geometry.
/// Returns `(path, LineString)` pairs:
///   - Exterior ring at path `{0}` (or `{poly_idx, 0}` for MultiPolygon)
///   - Interior rings (holes) at paths `{1}`, `{2}`, ...
pub fn dump_rings(geom: &Geom) -> Vec<(Vec<i32>, Geom)> {
    let mut out = Vec::new();
    dump_rings_into(geom, &mut Vec::new(), &mut out);
    out
}

fn dump_rings_into(geom: &Geom, prefix: &mut Vec<i32>, out: &mut Vec<(Vec<i32>, Geom)>) {
    match geom {
        Geometry::Polygon(p) => {
            // Exterior ring → path suffix {0}.
            prefix.push(0);
            out.push((prefix.clone(), Geometry::LineString(p.exterior().clone())));
            prefix.pop();
            // Interior rings → path suffix {1}, {2}, ...
            for (i, ring) in p.interiors().iter().enumerate() {
                prefix.push((i + 1) as i32);
                out.push((prefix.clone(), Geometry::LineString(ring.clone())));
                prefix.pop();
            }
        }
        Geometry::MultiPolygon(mp) => {
            for (i, p) in mp.0.iter().enumerate() {
                prefix.push((i + 1) as i32);
                dump_rings_into(&Geometry::Polygon(p.clone()), prefix, out);
                prefix.pop();
            }
        }
        Geometry::GeometryCollection(c) => {
            for (i, g) in c.iter().enumerate() {
                prefix.push((i + 1) as i32);
                dump_rings_into(g, prefix, out);
                prefix.pop();
            }
        }
        // Non-polygonal geometry → no rings.
        _ => {}
    }
}

// ----- ST_DumpRings FFI -------------------------------------------------

pub struct DumpRingsBind {
    rows: Arc<[(String, Vec<u8>)]>,
}

pub unsafe extern "C" fn dump_rings_bind(info: duckdb_bind_info) {
    let bi = unsafe { BindInfo::new(info) };
    let Some(g) = (unsafe { read_geom_param(&bi, 0, "st_dumprings") }) else {
        return;
    };
    let mut rows = Vec::new();
    for (path, geom) in dump_rings(&g) {
        let wkb = geometry::to_wkb(&geom).unwrap_or_default();
        rows.push((format_path(&path), wkb));
    }
    bi.add_result_column("path", TypeId::Varchar)
        .add_result_column("geom", TypeId::Blob)
        .set_cardinality(rows.len() as u64, true);
    unsafe { FfiBindData::<DumpRingsBind>::set(info, DumpRingsBind { rows: Arc::from(rows) }) };
}

pub unsafe extern "C" fn dump_rings_init(info: duckdb_init_info) {
    unsafe { FfiInitData::<ScanCursor>::set(info, ScanCursor { cursor: AtomicUsize::new(0) }) };
}

pub unsafe extern "C" fn dump_rings_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let cap = unsafe { libduckdb_sys::duckdb_vector_size() } as usize;
    let Some(data) = (unsafe { FfiBindData::<DumpRingsBind>::get_from_function(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let Some(state) = (unsafe { FfiInitData::<ScanCursor>::get(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let start = state.cursor.fetch_add(cap, Ordering::Relaxed);
    if start >= data.rows.len() {
        unsafe { chunk.set_size(0) };
        return;
    }
    let batch = (data.rows.len() - start).min(cap);
    let mut w0 = unsafe { chunk.writer(0) };
    let mut w1 = unsafe { chunk.writer(1) };
    for i in 0..batch {
        let (path, geom) = &data.rows[start + i];
        unsafe { w0.write_varchar(i, path) };
        unsafe { w1.write_blob(i, geom) };
    }
    unsafe { chunk.set_size(batch) };
    drop((w0, w1));
}

// =====================================================================
// ST_IsValidDetail
// =====================================================================

/// Compute validity detail for a geometry.
/// Returns `(valid, reason, location_geom)` matching PostGIS `ST_IsValidDetail`.
/// For valid geometry: `(true, "Valid Geometry", EMPTY POINT)`.
/// For invalid geometry: `(false, reason_string, problem_location_point)`.
pub fn valid_detail(geom: &Geom) -> (bool, String, Geom) {
    use geo::Validation;
    let errs = geom.validation_errors();
    if errs.is_empty() {
        (
            true,
            "Valid Geometry".to_string(),
            Geometry::Point(geo_types::Point::new(0.0, 0.0)),
        )
    } else {
        let reason = errs
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        // geo::ValidationError doesn't carry an explicit coordinate in all
        // cases; we return a NaN point as the "location" placeholder.
        (false, reason, Geometry::Point(geo_types::Point::new(f64::NAN, f64::NAN)))
    }
}

pub struct ValidDetailBind {
    rows: Arc<[(bool, String, Vec<u8>)]>,
}

pub unsafe extern "C" fn valid_detail_bind(info: duckdb_bind_info) {
    let bi = unsafe { BindInfo::new(info) };
    let Some(g) = (unsafe { read_geom_param(&bi, 0, "st_isvaliddetail") }) else {
        return;
    };
    let (valid, reason, loc_geom) = valid_detail(&g);
    let wkb = geometry::to_wkb(&loc_geom).unwrap_or_default();
    let rows = vec![(valid, reason, wkb)];
    bi.add_result_column("valid", TypeId::Boolean)
        .add_result_column("reason", TypeId::Varchar)
        .add_result_column("geom", TypeId::Blob)
        .set_cardinality(rows.len() as u64, true);
    unsafe {
        FfiBindData::<ValidDetailBind>::set(info, ValidDetailBind {
            rows: Arc::from(rows),
        })
    };
}

pub unsafe extern "C" fn valid_detail_init(info: duckdb_init_info) {
    unsafe { FfiInitData::<ScanCursor>::set(info, ScanCursor { cursor: AtomicUsize::new(0) }) };
}

pub unsafe extern "C" fn valid_detail_scan(
    info: duckdb_function_info,
    output: duckdb_data_chunk,
) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let cap = unsafe { libduckdb_sys::duckdb_vector_size() } as usize;
    let Some(data) = (unsafe { FfiBindData::<ValidDetailBind>::get_from_function(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let Some(state) = (unsafe { FfiInitData::<ScanCursor>::get(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let start = state.cursor.fetch_add(cap, Ordering::Relaxed);
    if start >= data.rows.len() {
        unsafe { chunk.set_size(0) };
        return;
    }
    let batch = (data.rows.len() - start).min(cap);
    let mut w0 = unsafe { chunk.writer(0) };
    let mut w1 = unsafe { chunk.writer(1) };
    let mut w2 = unsafe { chunk.writer(2) };
    for i in 0..batch {
        let (valid, reason, geom) = &data.rows[start + i];
        unsafe { w0.write_bool(i, *valid) };
        unsafe { w1.write_varchar(i, reason) };
        unsafe { w2.write_blob(i, geom) };
    }
    unsafe { chunk.set_size(batch) };
    drop((w0, w1, w2));
}

// =====================================================================
// ST_CoveringQuadKeys
// =====================================================================

pub struct CoveringQuadKeysBind {
    rows: Arc<[(String, i32, i32)]>,
}

pub unsafe extern "C" fn covering_quadkeys_bind(info: duckdb_bind_info) {
    let bi = unsafe { BindInfo::new(info) };
    let Some(g) = (unsafe { read_geom_param(&bi, 0, "st_covering_quadkeys") }) else {
        return;
    };
    let zoom = unsafe { bi.get_parameter_value(1) }.as_i32();
    let max_cells_raw = unsafe { bi.get_parameter_value(2) }.as_i32();
    let max_cells = if max_cells_raw > 0 {
        max_cells_raw as usize
    } else {
        1000
    };
    if zoom < 0 || zoom > 23 {
        return;
    }
    match crate::spatial_keys::covering_quadkeys(&g, zoom as u32, max_cells) {
        Some(cells) => {
            let rows: Vec<(String, i32, i32)> = cells
                .into_iter()
                .map(|(qk, tx, ty)| (qk, tx as i32, ty as i32))
                .collect();
            bi.add_result_column("quadkey", TypeId::Varchar)
                .add_result_column("tile_x", TypeId::Integer)
                .add_result_column("tile_y", TypeId::Integer)
                .set_cardinality(rows.len() as u64, true);
            unsafe {
                FfiBindData::<CoveringQuadKeysBind>::set(info, CoveringQuadKeysBind {
                    rows: Arc::from(rows),
                })
            };
        }
        None => {
            // Fail closed: too many cells for the given max_cells.
            bi.add_result_column("quadkey", TypeId::Varchar)
                .add_result_column("tile_x", TypeId::Integer)
                .add_result_column("tile_y", TypeId::Integer)
                .set_cardinality(0, true);
            unsafe {
                FfiBindData::<CoveringQuadKeysBind>::set(info, CoveringQuadKeysBind {
                    rows: Arc::from(Vec::new()),
                })
            };
        }
    }
}

pub unsafe extern "C" fn covering_quadkeys_init(info: duckdb_init_info) {
    unsafe { FfiInitData::<ScanCursor>::set(info, ScanCursor { cursor: AtomicUsize::new(0) }) };
}

pub unsafe extern "C" fn covering_quadkeys_scan(
    info: duckdb_function_info,
    output: duckdb_data_chunk,
) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let cap = unsafe { libduckdb_sys::duckdb_vector_size() } as usize;
    let Some(data) = (unsafe { FfiBindData::<CoveringQuadKeysBind>::get_from_function(info) })
    else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let Some(state) = (unsafe { FfiInitData::<ScanCursor>::get(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let start = state.cursor.fetch_add(cap, Ordering::Relaxed);
    if start >= data.rows.len() {
        unsafe { chunk.set_size(0) };
        return;
    }
    let batch = (data.rows.len() - start).min(cap);
    let mut w0 = unsafe { chunk.writer(0) };
    let mut w1 = unsafe { chunk.writer(1) };
    let mut w2 = unsafe { chunk.writer(2) };
    for i in 0..batch {
        let (qk, tx, ty) = &data.rows[start + i];
        unsafe { w0.write_varchar(i, qk) };
        unsafe { w1.write_i32(i, *tx) };
        unsafe { w2.write_i32(i, *ty) };
    }
    unsafe { chunk.set_size(batch) };
    drop((w0, w1, w2));
}