// SPDX-License-Identifier: Apache-2.0
//
//! `sedona_join` — an indexed spatial-join table function.
//!
//! Answers the brief's "spill table data to disk, let the extension handle the
//! join with an index" pattern, since DuckDB's C API exposes no join-planner or
//! GiST-style index operators. Usage:
//!
//! ```sql
//! COPY (SELECT id, geom FROM a) TO 'a.parquet';
//! COPY (SELECT id, geom FROM b) TO 'b.parquet';
//! SELECT * FROM sedona_join('a.parquet', 'b.parquet', 'intersects');
//! ```
//!
//! The function reads both Parquet files itself, builds an R*-tree (`rstar`)
//! over the right side, and for each left geometry streams the right geometries
//! whose bounding box intersects, then applies the exact predicate. Returns one
//! row per matching pair: `(a_row BIGINT, b_row BIGINT)` (0-indexed file rows).
//! The geometry column is taken to be the **last** BLOB column in each file.

use std::sync::Arc;

use arrow::array::{Array, BinaryArray, LargeBinaryArray};
use libduckdb_sys::{
    duckdb_bind_info, duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector_size,
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use quack_rs::data_chunk::DataChunk;
use quack_rs::table::{BindInfo, FfiBindData, FfiInitData, InitInfo};
use quack_rs::types::TypeId;

use crate::geometry::{from_wkb, Geom};

/// A geometry plus its original row index, indexable by `rstar`.
struct IndexedGeom {
    idx: usize,
    geom: Geom,
}

impl rstar::RTreeObject for IndexedGeom {
    type Envelope = rstar::AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        use geo::BoundingRect;
        match self.geom.bounding_rect() {
            Some(r) => rstar::AABB::from_corners([r.min().x, r.min().y], [r.max().x, r.max().y]),
            None => rstar::AABB::from_corners([0.0, 0.0], [0.0, 0.0]),
        }
    }
}

/// Read the last BLOB-typed column of a Parquet file into parsed geometries
/// (with their row index). Rows that fail to parse as WKB are skipped.
pub fn read_geoms(path: &str) -> Result<Vec<IndexedGeom>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| format!("parquet open {path}: {e}"))?
        .build()
        .map_err(|e| format!("parquet build: {e}"))?;

    let mut out = Vec::new();
    let mut row = 0usize;
    let mut blob_col: Option<usize> = None;
    for batch in reader {
        let batch = batch.map_err(|e| format!("parquet read: {e}"))?;
        if blob_col.is_none() {
            blob_col = (0..batch.num_columns()).rev().find(|&i| {
                let c = batch.column(i);
                c.as_any().downcast_ref::<BinaryArray>().is_some()
                    || c.as_any().downcast_ref::<LargeBinaryArray>().is_some()
            });
        }
        let Some(i) = blob_col else {
            row += batch.num_rows();
            continue;
        };
        let col = batch.column(i);
        let n = col.len();
        let take = |bytes: &[u8]| -> Option<Geom> {
            if bytes.is_empty() { None } else { from_wkb(bytes).ok() }
        };
        if let Some(b) = col.as_any().downcast_ref::<BinaryArray>() {
            for k in 0..n {
                if let Some(g) = take(b.value(k)) {
                    out.push(IndexedGeom { idx: row, geom: g });
                }
                row += 1;
            }
        } else if let Some(b) = col.as_any().downcast_ref::<LargeBinaryArray>() {
            for k in 0..n {
                if let Some(g) = take(b.value(k)) {
                    out.push(IndexedGeom { idx: row, geom: g });
                }
                row += 1;
            }
        } else {
            row += n;
        }
    }
    Ok(out)
}

/// Build the right-side R*-tree.
pub fn build_tree(right: Vec<IndexedGeom>) -> rstar::RTree<IndexedGeom> {
    rstar::RTree::<IndexedGeom>::bulk_load(right)
}

/// Find all (left_row, right_row) pairs whose geometries satisfy `predicate`.
/// Uses the R-tree for bbox prefiltering, then the exact predicate.
pub fn join_pairs(
    left: &[IndexedGeom],
    tree: &rstar::RTree<IndexedGeom>,
    predicate: &str,
) -> Vec<(i64, i64)> {
    use geo::BoundingRect;
    let mut pairs = Vec::new();
    for l in left {
        let aabb = match l.geom.bounding_rect() {
            Some(r) => rstar::AABB::from_corners([r.min().x, r.min().y], [r.max().x, r.max().y]),
            None => continue,
        };
        for cand in tree.locate_in_envelope_intersecting(aabb).collect::<Vec<_>>() {
            if matches_predicate(&l.geom, &cand.geom, predicate) {
                pairs.push((l.idx as i64, cand.idx as i64));
            }
        }
    }
    pairs
}

fn matches_predicate(a: &Geom, b: &Geom, predicate: &str) -> bool {
    match predicate.to_ascii_lowercase().as_str() {
        "intersects" => crate::functions::intersects(a, b).unwrap_or(false),
        "contains" => crate::functions::contains(a, b).unwrap_or(false),
        "within" => crate::functions::within(a, b).unwrap_or(false),
        "covers" => crate::functions::covers(a, b).unwrap_or(false),
        "disjoint" => crate::functions::disjoint(a, b).unwrap_or(false),
        "equals" => crate::functions::equals(a, b).unwrap_or(false),
        "touches" => crate::functions::touches(a, b).unwrap_or(false),
        "crosses" => crate::functions::crosses(a, b).unwrap_or(false),
        "overlaps" => crate::functions::overlaps(a, b).unwrap_or(false),
        "dwithin_0.0045" | "dwithin" => crate::functions::dwithin(a, b, 0.0045).unwrap_or(false),
        _ => false,
    }
}

// These re-exports keep the parquet RowFilter/RowSelector imports used if we
// later add predicate-pushdown to the parquet read; for now they document intent.
#[allow(dead_code)]
fn _suppress() {
    let _ = std::marker::PhantomData::<()>;
}

// ---------------------------------------------------------------------------
// Table-function lifecycle (bind / init / scan)
// ---------------------------------------------------------------------------

/// Bind data: the precomputed list of matching (left_row, right_row) pairs.
pub struct JoinBindData {
    pub pairs: Vec<(i64, i64)>,
}

/// Scan state: a cursor into `JoinBindData::pairs` (single-threaded scan).
pub struct JoinScanState {
    pub cursor: usize,
}

pub unsafe extern "C" fn join_bind(info: duckdb_bind_info) {
    let bi = unsafe { BindInfo::new(info) };
    let a_path = unsafe { bi.get_parameter_value(0) }.as_str().unwrap_or_default();
    let b_path = unsafe { bi.get_parameter_value(1) }.as_str().unwrap_or_default();
    let pred = unsafe { bi.get_parameter_value(2) }
        .as_str()
        .unwrap_or_else(|_| "intersects".to_string());

    let left = match read_geoms(&a_path) {
        Ok(v) => v,
        Err(e) => {
            bi.set_error(&e);
            return;
        }
    };
    let right = match read_geoms(&b_path) {
        Ok(v) => v,
        Err(e) => {
            bi.set_error(&e);
            return;
        }
    };
    let tree = build_tree(right);
    let pairs = join_pairs(&left, &tree, &pred);

    bi.add_result_column("a_row", TypeId::BigInt)
        .add_result_column("b_row", TypeId::BigInt)
        .set_cardinality(pairs.len() as u64, true);
    unsafe { FfiBindData::<JoinBindData>::set(info, JoinBindData { pairs }) };
}

pub unsafe extern "C" fn join_init(info: duckdb_init_info) {
    // Force single-threaded scan: the cursor pattern below is not partition-safe.
    unsafe {
        InitInfo::new(info).set_max_threads(1);
    }
    unsafe { FfiInitData::<JoinScanState>::set(info, JoinScanState { cursor: 0 }) };
}

pub unsafe extern "C" fn join_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let Some(data) = (unsafe { FfiBindData::<JoinBindData>::get_from_function(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let Some(state) = (unsafe { FfiInitData::<JoinScanState>::get_mut(info) }) else {
        unsafe { chunk.set_size(0) };
        return;
    };
    let vector_size = unsafe { duckdb_vector_size() } as usize;
    let remaining = data.pairs.len().saturating_sub(state.cursor);
    let batch = remaining.min(vector_size);
    let mut w0 = unsafe { chunk.writer(0) };
    let mut w1 = unsafe { chunk.writer(1) };
    for i in 0..batch {
        let (a, b) = data.pairs[state.cursor + i];
        unsafe { w0.write_i64(i, a) };
        unsafe { w1.write_i64(i, b) };
    }
    state.cursor += batch;
    unsafe { chunk.set_size(batch) };
}
