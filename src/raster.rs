// SPDX-License-Identifier: Apache-2.0
//
//! Raster support via GDAL (Tier 4). Requires libgdal 3.x (vendored+patched
//! `gdal` crate; see vendor/gdal). Provides:
//!   * `st_raster_info(path)` — per-band metadata (band, width, height, dtype,
//!     nodata, GeoTransform origin/pixel size).
//!   * `st_raster_stats(path, band)` — ST_SummaryStats: min/max/mean/std/count
//!     over a band's non-nodata pixels (read as its native GDAL type).

use gdal::raster::GdalDataType;
use gdal::Dataset;
use libduckdb_sys::{
    duckdb_bind_info, duckdb_data_chunk, duckdb_function_info, duckdb_init_info, duckdb_vector_size,
};
use quack_rs::data_chunk::DataChunk;
use quack_rs::table::{BindInfo, FfiBindData, FfiInitData, InitInfo};
use quack_rs::types::TypeId;

struct BandInfoRow {
    band: i32,
    width: i64,
    height: i64,
    dtype: String,
    nodata: Option<f64>,
    origin_x: f64,
    origin_y: f64,
    pix_w: f64,
    pix_h: f64,
}

fn read_band_infos(path: &str) -> Result<Vec<BandInfoRow>, String> {
    let ds = Dataset::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let gt = ds.geo_transform().unwrap_or_else(|_| [0.0, 1.0, 0.0, 0.0, 0.0, -1.0]);
    let mut rows = Vec::new();
    for (i, band) in ds.rasterbands().enumerate() {
        let band = band.map_err(|e| format!("band {}: {e}", i + 1))?;
        let (w, h) = band.size();
        rows.push(BandInfoRow {
            band: (i + 1) as i32,
            width: w as i64,
            height: h as i64,
            dtype: format!("{:?}", band.band_type()),
            nodata: band.no_data_value(),
            origin_x: gt[0],
            origin_y: gt[3],
            pix_w: gt[1],
            pix_h: gt[5],
        });
    }
    Ok(rows)
}

fn band_stats(path: &str, band_no: usize) -> Result<(f64, f64, f64, f64, i64), String> {
    let ds = Dataset::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let band = ds.rasterband(band_no).map_err(|e| format!("band {band_no}: {e}"))?;
    let nodata = band.no_data_value();
    let acc = |vals: &[f64]| -> (f64, f64, f64, f64, i64) {
        let (mut mn, mut mx, mut sum, mut sq, mut n) = (f64::INFINITY, f64::NEG_INFINITY, 0.0, 0.0, 0i64);
        for &v in vals {
            if !v.is_finite() || matches!(nodata, Some(nd) if nd == v) { continue; }
            mn = mn.min(v); mx = mx.max(v); sum += v; sq += v * v; n += 1;
        }
        (mn, mx, sum, sq, n)
    };
    let read = |t: &str| -> Result<Vec<f64>, String> { Err(t.to_string()) };
    let _ = read;
    let (mn, mx, sum, sq, n) = match band.band_type() {
        GdalDataType::UInt8 => acc(&band.read_band_as::<u8>().map_err(|e| e.to_string())?.data().iter().map(|&v| v as f64).collect::<Vec<_>>()),
        GdalDataType::Int16 => acc(&band.read_band_as::<i16>().map_err(|e| e.to_string())?.data().iter().map(|&v| v as f64).collect::<Vec<_>>()),
        GdalDataType::UInt16 => acc(&band.read_band_as::<u16>().map_err(|e| e.to_string())?.data().iter().map(|&v| v as f64).collect::<Vec<_>>()),
        GdalDataType::Int32 => acc(&band.read_band_as::<i32>().map_err(|e| e.to_string())?.data().iter().map(|&v| v as f64).collect::<Vec<_>>()),
        GdalDataType::UInt32 => acc(&band.read_band_as::<u32>().map_err(|e| e.to_string())?.data().iter().map(|&v| v as f64).collect::<Vec<_>>()),
        GdalDataType::Float32 => acc(&band.read_band_as::<f32>().map_err(|e| e.to_string())?.data().iter().map(|&v| v as f64).collect::<Vec<_>>()),
        GdalDataType::Float64 => acc(band.read_band_as::<f64>().map_err(|e| e.to_string())?.data()),
        other => return Err(format!("unsupported GDAL dtype: {other:?}")),
    };
    let mean = if n > 0 { sum / n as f64 } else { f64::NAN };
    let std = if n > 0 { (sq / n as f64 - mean * mean).max(0.0).sqrt() } else { f64::NAN };
    Ok((mn, mx, mean, std, n))
}

// ===== st_raster_info(path) ==============================================

pub struct RasterInfoBind { rows: Vec<BandInfoRow> }
pub struct RasterInfoScan { cursor: usize }

pub unsafe extern "C" fn raster_info_bind(info: duckdb_bind_info) {
    let bi = unsafe { BindInfo::new(info) };
    let path = unsafe { bi.get_parameter_value(0) }.as_str().unwrap_or_default();
    let rows = match read_band_infos(&path) {
        Ok(r) => r,
        Err(e) => { bi.set_error(&e); return; }
    };
    bi.add_result_column("band", TypeId::Integer)
        .add_result_column("width", TypeId::BigInt)
        .add_result_column("height", TypeId::BigInt)
        .add_result_column("dtype", TypeId::Varchar)
        .add_result_column("nodata", TypeId::Double)
        .add_result_column("origin_x", TypeId::Double)
        .add_result_column("origin_y", TypeId::Double)
        .add_result_column("pix_w", TypeId::Double)
        .add_result_column("pix_h", TypeId::Double)
        .set_cardinality(rows.len() as u64, true);
    unsafe { FfiBindData::<RasterInfoBind>::set(info, RasterInfoBind { rows }) };
}
pub unsafe extern "C" fn raster_info_init(info: duckdb_init_info) {
    unsafe { InitInfo::new(info).set_max_threads(1) };
    unsafe { FfiInitData::<RasterInfoScan>::set(info, RasterInfoScan { cursor: 0 }) };
}
pub unsafe extern "C" fn raster_info_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let cap = unsafe { duckdb_vector_size() } as usize;
    let Some(data) = (unsafe { FfiBindData::<RasterInfoBind>::get_from_function(info) }) else { unsafe { chunk.set_size(0) }; return; };
    let Some(state) = (unsafe { FfiInitData::<RasterInfoScan>::get_mut(info) }) else { unsafe { chunk.set_size(0) }; return; };
    let batch = data.rows.len().saturating_sub(state.cursor).min(cap);
    if batch == 0 { unsafe { chunk.set_size(0) }; return; }
    let mut cols: [quack_rs::vector::VectorWriter; 9] = [
        unsafe { chunk.writer(0) }, unsafe { chunk.writer(1) }, unsafe { chunk.writer(2) },
        unsafe { chunk.writer(3) }, unsafe { chunk.writer(4) }, unsafe { chunk.writer(5) },
        unsafe { chunk.writer(6) }, unsafe { chunk.writer(7) }, unsafe { chunk.writer(8) },
    ];
    for i in 0..batch {
        let r = &data.rows[state.cursor + i];
        unsafe { cols[0].write_i32(i, r.band) };
        unsafe { cols[1].write_i64(i, r.width) };
        unsafe { cols[2].write_i64(i, r.height) };
        unsafe { cols[3].write_varchar(i, &r.dtype) };
        match r.nodata { Some(v) => unsafe { cols[4].write_f64(i, v) }, None => unsafe { cols[4].set_null(i) } }
        unsafe { cols[5].write_f64(i, r.origin_x) };
        unsafe { cols[6].write_f64(i, r.origin_y) };
        unsafe { cols[7].write_f64(i, r.pix_w) };
        unsafe { cols[8].write_f64(i, r.pix_h) };
    }
    state.cursor += batch;
    unsafe { chunk.set_size(batch) };
    // keep writers alive for the writes above
    drop(cols);
}

// ===== st_raster_stats(path, band) =======================================

pub struct RasterStatsBind { stats: Option<(f64, f64, f64, f64, i64)> }

pub unsafe extern "C" fn raster_stats_bind(info: duckdb_bind_info) {
    let bi = unsafe { BindInfo::new(info) };
    let path = unsafe { bi.get_parameter_value(0) }.as_str().unwrap_or_default();
    let band_no = unsafe { bi.get_parameter_value(1) }.as_i32().max(1) as usize;
    let stats = match band_stats(&path, band_no) {
        Ok(s) => Some(s),
        Err(e) => { bi.set_error(&e); return; }
    };
    bi.add_result_column("min", TypeId::Double)
        .add_result_column("max", TypeId::Double)
        .add_result_column("mean", TypeId::Double)
        .add_result_column("std", TypeId::Double)
        .add_result_column("count", TypeId::BigInt)
        .set_cardinality(1, true);
    unsafe { FfiBindData::<RasterStatsBind>::set(info, RasterStatsBind { stats }) };
}
pub unsafe extern "C" fn raster_stats_init(info: duckdb_init_info) {
    unsafe { InitInfo::new(info).set_max_threads(1) };
    unsafe { FfiInitData::<bool>::set(info, false) };
}
pub unsafe extern "C" fn raster_stats_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let Some(emitted) = (unsafe { FfiInitData::<bool>::get_mut(info) }) else { unsafe { chunk.set_size(0) }; return; };
    if *emitted { unsafe { chunk.set_size(0) }; return; }
    *emitted = true;
    let Some(data) = (unsafe { FfiBindData::<RasterStatsBind>::get_from_function(info) }) else { unsafe { chunk.set_size(0) }; return; };
    let (mn, mx, mean, std, count) = data.stats.unwrap_or((f64::NAN, f64::NAN, f64::NAN, f64::NAN, 0));
    let mut c0 = unsafe { chunk.writer(0) }; let mut c1 = unsafe { chunk.writer(1) };
    let mut c2 = unsafe { chunk.writer(2) }; let mut c3 = unsafe { chunk.writer(3) };
    let mut c4 = unsafe { chunk.writer(4) };
    unsafe { c0.write_f64(0, mn) }; unsafe { c1.write_f64(0, mx) };
    unsafe { c2.write_f64(0, mean) }; unsafe { c3.write_f64(0, std) };
    unsafe { c4.write_i64(0, count) };
    unsafe { chunk.set_size(1) };
    drop((c0, c1, c2, c3, c4));
}
