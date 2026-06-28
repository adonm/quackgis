// SPDX-License-Identifier: Apache-2.0
//
//! Raster support via GDAL (Tier 4). Requires libgdal 3.x (vendored+patched
//! `gdal` crate; see vendor/gdal). Provides:
//!   * `st_raster_info(path)` — per-band metadata (band, width, height, dtype,
//!     nodata, GeoTransform origin/pixel size).
//!   * `st_raster_stats(path, band)` — ST_SummaryStats: min/max/mean/std/count
//!     over a band's non-nodata pixels (read as its native GDAL type).
//!   * `st_pixeldata(path, band)` — stream all pixels of a band as (row, col,
//!     value) rows. This is the foundation for map algebra: users apply SQL
//!     expressions (WHERE, CASE, arithmetic) directly on the pixel table rather
//!     than through a custom expression parser. The GDAL boundary stays narrow
//!     (one band read → DuckDB rows); the algebra is DuckDB-native SQL.

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

// ===== st_pixeldata(path, band) ==========================================
// Streams all pixels of a band as (row, col, value) rows. Nodata pixels are
// emitted as NULL (so SQL predicates naturally skip them). Map algebra is then
// DuckDB-native SQL, e.g.:
//   SELECT avg(value) FROM st_pixeldata('r.tif', 1) WHERE value > 100;
//   SELECT row, col, CASE WHEN value > 50 THEN 1 ELSE 0 END FROM st_pixeldata(...);

fn read_band_pixels(path: &str, band_no: usize) -> Result<(Vec<Option<f64>>, usize, usize), String> {
    let ds = Dataset::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let band = ds.rasterband(band_no).map_err(|e| format!("band {band_no}: {e}"))?;
    let (w, h) = band.size();
    let nodata = band.no_data_value();
    let to_opts = |v: f64| if !v.is_finite() || matches!(nodata, Some(nd) if nd == v) { None } else { Some(v) };
    let pixels: Vec<Option<f64>> = match band.band_type() {
        GdalDataType::UInt8 => band.read_band_as::<u8>().map_err(|e| e.to_string())?.data().iter().map(|&v| to_opts(v as f64)).collect(),
        GdalDataType::Int16 => band.read_band_as::<i16>().map_err(|e| e.to_string())?.data().iter().map(|&v| to_opts(v as f64)).collect(),
        GdalDataType::UInt16 => band.read_band_as::<u16>().map_err(|e| e.to_string())?.data().iter().map(|&v| to_opts(v as f64)).collect(),
        GdalDataType::Int32 => band.read_band_as::<i32>().map_err(|e| e.to_string())?.data().iter().map(|&v| to_opts(v as f64)).collect(),
        GdalDataType::UInt32 => band.read_band_as::<u32>().map_err(|e| e.to_string())?.data().iter().map(|&v| to_opts(v as f64)).collect(),
        GdalDataType::Float32 => band.read_band_as::<f32>().map_err(|e| e.to_string())?.data().iter().map(|&v| to_opts(v as f64)).collect(),
        GdalDataType::Float64 => band.read_band_as::<f64>().map_err(|e| e.to_string())?.data().iter().map(|&v| to_opts(v)).collect(),
        other => return Err(format!("unsupported GDAL dtype: {other:?}")),
    };
    Ok((pixels, w, h))
}

pub struct PixelDataBind { pixels: Vec<Option<f64>>, width: usize }
pub struct PixelDataScan { cursor: usize }

pub unsafe extern "C" fn pixeldata_bind(info: duckdb_bind_info) {
    let bi = unsafe { BindInfo::new(info) };
    let path = unsafe { bi.get_parameter_value(0) }.as_str().unwrap_or_default();
    let band_no = unsafe { bi.get_parameter_value(1) }.as_i32().max(1) as usize;
    let (pixels, width, _height) = match read_band_pixels(&path, band_no) {
        Ok(r) => r,
        Err(e) => { bi.set_error(&e); return; }
    };
    bi.add_result_column("row", TypeId::Integer)
        .add_result_column("col", TypeId::Integer)
        .add_result_column("value", TypeId::Double)
        .set_cardinality(pixels.len() as u64, true);
    unsafe { FfiBindData::<PixelDataBind>::set(info, PixelDataBind { pixels, width }) };
}

pub unsafe extern "C" fn pixeldata_init(info: duckdb_init_info) {
    unsafe { InitInfo::new(info).set_max_threads(1) };
    unsafe { FfiInitData::<PixelDataScan>::set(info, PixelDataScan { cursor: 0 }) };
}

pub unsafe extern "C" fn pixeldata_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let cap = unsafe { duckdb_vector_size() } as usize;
    let Some(data) = (unsafe { FfiBindData::<PixelDataBind>::get_from_function(info) }) else { unsafe { chunk.set_size(0) }; return; };
    let Some(state) = (unsafe { FfiInitData::<PixelDataScan>::get_mut(info) }) else { unsafe { chunk.set_size(0) }; return; };
    let batch = data.pixels.len().saturating_sub(state.cursor).min(cap);
    if batch == 0 { unsafe { chunk.set_size(0) }; return; }
    let mut c0 = unsafe { chunk.writer(0) };
    let mut c1 = unsafe { chunk.writer(1) };
    let mut c2 = unsafe { chunk.writer(2) };
    for i in 0..batch {
        let idx = state.cursor + i;
        let row = (idx / data.width) as i32;
        let col = (idx % data.width) as i32;
        unsafe { c0.write_i32(i, row) };
        unsafe { c1.write_i32(i, col) };
        match data.pixels[idx] {
            Some(v) => unsafe { c2.write_f64(i, v) },
            None => unsafe { c2.set_null(i) },
        }
    }
    state.cursor += batch;
    unsafe { chunk.set_size(batch) };
    drop((c0, c1, c2));
}

// ===== st_raster_transform(path) ========================================
// Returns the GeoTransform (6 params) + computed spatial bounds in one row.
// This lets users convert between pixel (row,col) and geographic (x,y)
// coordinates entirely in SQL:
//   x = origin_x + col * pixel_w + row * row_rot
//   y = origin_y + col * col_rot + row * pixel_h

pub struct RasterTransformBind {
    origin_x: f64, origin_y: f64,
    pixel_w: f64, pixel_h: f64,
    row_rot: f64, col_rot: f64,
    xmin: f64, ymin: f64, xmax: f64, ymax: f64,
}

pub unsafe extern "C" fn raster_transform_bind(info: duckdb_bind_info) {
    let bi = unsafe { BindInfo::new(info) };
    let path = unsafe { bi.get_parameter_value(0) }.as_str().unwrap_or_default();
    let ds = match Dataset::open(&path) {
        Ok(ds) => ds,
        Err(e) => { bi.set_error(&format!("open {path}: {e}")); return; }
    };
    let gt = ds.geo_transform().unwrap_or_else(|_| [0.0, 1.0, 0.0, 0.0, 0.0, -1.0]);
    let (w, h) = ds.raster_size();
    // Compute the four corner coordinates to derive bounds.
    let corners = [
        (gt[0], gt[3]),                                              // top-left
        (gt[0] + w as f64 * gt[1], gt[3] + w as f64 * gt[4]),        // top-right
        (gt[0] + h as f64 * gt[2], gt[3] + h as f64 * gt[5]),        // bottom-left
        (gt[0] + w as f64 * gt[1] + h as f64 * gt[2],                // bottom-right
         gt[3] + w as f64 * gt[4] + h as f64 * gt[5]),
    ];
    let (xmin, xmax) = corners.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), &(x, _)| (mn.min(x), mx.max(x)));
    let (ymin, ymax) = corners.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), &(_, y)| (mn.min(y), mx.max(y)));
    bi.add_result_column("origin_x", TypeId::Double)
        .add_result_column("origin_y", TypeId::Double)
        .add_result_column("pixel_w", TypeId::Double)
        .add_result_column("pixel_h", TypeId::Double)
        .add_result_column("row_rot", TypeId::Double)
        .add_result_column("col_rot", TypeId::Double)
        .add_result_column("xmin", TypeId::Double)
        .add_result_column("ymin", TypeId::Double)
        .add_result_column("xmax", TypeId::Double)
        .add_result_column("ymax", TypeId::Double)
        .set_cardinality(1, true);
    unsafe {
        FfiBindData::<RasterTransformBind>::set(info, RasterTransformBind {
            origin_x: gt[0], origin_y: gt[3],
            pixel_w: gt[1], pixel_h: gt[5],
            row_rot: gt[2], col_rot: gt[4],
            xmin, ymin, xmax, ymax,
        })
    };
}

pub unsafe extern "C" fn raster_transform_init(info: duckdb_init_info) {
    unsafe { InitInfo::new(info).set_max_threads(1) };
    unsafe { FfiInitData::<bool>::set(info, false) };
}

pub unsafe extern "C" fn raster_transform_scan(info: duckdb_function_info, output: duckdb_data_chunk) {
    let chunk = unsafe { DataChunk::from_raw(output) };
    let Some(emitted) = (unsafe { FfiInitData::<bool>::get_mut(info) }) else { unsafe { chunk.set_size(0) }; return; };
    if *emitted { unsafe { chunk.set_size(0) }; return; }
    *emitted = true;
    let Some(data) = (unsafe { FfiBindData::<RasterTransformBind>::get_from_function(info) }) else { unsafe { chunk.set_size(0) }; return; };
    let mut cols: [quack_rs::vector::VectorWriter; 10] = [
        unsafe { chunk.writer(0) }, unsafe { chunk.writer(1) }, unsafe { chunk.writer(2) },
        unsafe { chunk.writer(3) }, unsafe { chunk.writer(4) }, unsafe { chunk.writer(5) },
        unsafe { chunk.writer(6) }, unsafe { chunk.writer(7) }, unsafe { chunk.writer(8) },
        unsafe { chunk.writer(9) },
    ];
    unsafe { cols[0].write_f64(0, data.origin_x) };
    unsafe { cols[1].write_f64(0, data.origin_y) };
    unsafe { cols[2].write_f64(0, data.pixel_w) };
    unsafe { cols[3].write_f64(0, data.pixel_h) };
    unsafe { cols[4].write_f64(0, data.row_rot) };
    unsafe { cols[5].write_f64(0, data.col_rot) };
    unsafe { cols[6].write_f64(0, data.xmin) };
    unsafe { cols[7].write_f64(0, data.ymin) };
    unsafe { cols[8].write_f64(0, data.xmax) };
    unsafe { cols[9].write_f64(0, data.ymax) };
    unsafe { chunk.set_size(1) };
    drop(cols);
}

// ===== st_value(path, band, x, y) — point sampling scalar ==================
//
// PostGIS-compatible point sampling: given geographic coordinates (x, y),
// invert the raster's GeoTransform to find the pixel (col, row), then read
// that single pixel value. Returns NULL for out-of-bounds or nodata pixels.
//
// For bulk sampling (many points from one raster), prefer joining against
// st_pixeldata(path, band) — this scalar opens the dataset per call.

use libduckdb_sys::duckdb_vector;
use quack_rs::vector::{VectorReader, VectorWriter};

/// Invert the GeoTransform to convert geographic (x, y) → pixel (col, row).
/// Returns `None` if the transform is singular.
fn invert_geotransform(gt: &[f64; 6], x: f64, y: f64) -> Option<(i64, i64)> {
    let det = gt[1] * gt[5] - gt[2] * gt[4];
    if det.abs() < 1e-20 {
        return None;
    }
    let dx = x - gt[0];
    let dy = y - gt[3];
    let col = (gt[5] * dx - gt[2] * dy) / det;
    let row = (-gt[4] * dx + gt[1] * dy) / det;
    // Pixel index = floor of the fractional pixel coordinate.
    Some((col.floor() as i64, row.floor() as i64))
}

/// Sample a single pixel value at geographic coordinates (x, y).
///
/// Opens the raster, inverts the GeoTransform, reads the pixel at the
/// computed (col, row). Returns `None` for out-of-bounds, nodata, or I/O
/// errors.
fn sample_pixel(path: &str, band_no: usize, x: f64, y: f64) -> Option<f64> {
    let ds = Dataset::open(path).ok()?;
    let band = ds.rasterband(band_no).ok()?;
    let gt = ds.geo_transform().unwrap_or_else(|_| [0.0, 1.0, 0.0, 0.0, 0.0, -1.0]);
    let (col, row) = invert_geotransform(&gt, x, y)?;
    let (w, h) = band.size();
    if col < 0 || col >= w as i64 || row < 0 || row >= h as i64 {
        return None;
    }
    let idx = row as usize * w + col as usize;
    let nodata = band.no_data_value();
    let val = match band.band_type() {
        GdalDataType::UInt8 => band.read_band_as::<u8>().ok()?.data()[idx] as f64,
        GdalDataType::Int16 => band.read_band_as::<i16>().ok()?.data()[idx] as f64,
        GdalDataType::UInt16 => band.read_band_as::<u16>().ok()?.data()[idx] as f64,
        GdalDataType::Int32 => band.read_band_as::<i32>().ok()?.data()[idx] as f64,
        GdalDataType::UInt32 => band.read_band_as::<u32>().ok()?.data()[idx] as f64,
        GdalDataType::Float32 => band.read_band_as::<f32>().ok()?.data()[idx] as f64,
        GdalDataType::Float64 => band.read_band_as::<f64>().ok()?.data()[idx],
        _ => return None,
    };
    if !val.is_finite() || matches!(nodata, Some(nd) if nd == val) {
        return None;
    }
    Some(val)
}

/// DuckDB scalar callback for `st_value(path VARCHAR, band INTEGER, x DOUBLE, y
/// DOUBLE) → DOUBLE`.
pub unsafe extern "C" fn st_value_cb(
    _info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let chunk = unsafe { DataChunk::from_raw(input) };
    let nrows = chunk.size();
    let mut writer = unsafe { VectorWriter::new(output) };
    let col_path = unsafe { VectorReader::new(input, 0) };
    let col_band = unsafe { VectorReader::new(input, 1) };
    let col_x = unsafe { VectorReader::new(input, 2) };
    let col_y = unsafe { VectorReader::new(input, 3) };

    for row in 0..nrows {
        // NULL propagation: any NULL input → NULL output.
        if !unsafe { col_path.is_valid(row) } {
            unsafe { writer.set_null(row) };
            continue;
        }
        let path = unsafe { col_path.read_str(row) };
        let band = crate::dispatch::read_i32(&col_band, row).unwrap_or(1).max(1) as usize;
        let Some(x) = crate::dispatch::read_f64(&col_x, row) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        let Some(y) = crate::dispatch::read_f64(&col_y, row) else {
            unsafe { writer.set_null(row) };
            continue;
        };
        match sample_pixel(path, band, x, y) {
            Some(v) => unsafe { writer.write_f64(row, v) },
            None => unsafe { writer.set_null(row) },
        }
    }
}
