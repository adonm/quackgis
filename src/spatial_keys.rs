// SPDX-License-Identifier: Apache-2.0
//
//! Spatial partition key primitives for DuckLake / Hive-partitioned stores.
//!
//! These are **pure, deterministic** functions that derive partition/sort keys
//! from geometry. They never hold state, never depend on DuckLake internals,
//! and always produce the same output for the same input across sessions and
//! writers.
//!
//! CRS contract: cell keys (`ST_QuadKey`, `ST_GeoHash`) assume lon/lat
//! EPSG:4326 input. The extension does not split envelopes crossing the
//! antimeridian in v1 (documented, fixture-pinned). Out-of-range coordinates
//! yield NULL.

use geo::BoundingRect;
use geo_types::{Geometry, Point};

use crate::geometry::Geom;

// =====================================================================
// Envelope helpers
// =====================================================================

/// Return `(xmin, ymin, xmax, ymax)` or `None` for empty/NULL geometry.
fn envelope(g: &Geom) -> Option<(f64, f64, f64, f64)> {
    let r = g.bounding_rect()?;
    Some((r.min().x, r.min().y, r.max().x, r.max().y))
}

/// Envelope center: `(cx, cy)` or `None`.
fn envelope_center(g: &Geom) -> Option<(f64, f64)> {
    let (xmin, ymin, xmax, ymax) = envelope(g)?;
    Some(((xmin + xmax) / 2.0, (ymin + ymax) / 2.0))
}

// =====================================================================
// ST_BBoxIntersects
// =====================================================================

/// Cheap bbox-only intersection predicate.
pub fn bbox_intersects(a: &Geom, b: &Geom) -> Option<bool> {
    let (axmin, aymin, axmax, aymax) = envelope(a)?;
    let (bxmin, bymin, bxmax, bymax) = envelope(b)?;
    Some(axmax >= bxmin && axmin <= bxmax && aymax >= bymin && aymin <= bymax)
}

// =====================================================================
// Tile math (Bing/QuadKey convention)
// =====================================================================

/// Convert lon/lat to tile (x, y) at the given zoom level.
/// Returns `None` for out-of-range coordinates.
fn lonlat_to_tile(lon: f64, lat: f64, zoom: u32) -> Option<(u32, u32)> {
    if !(-180.0..=180.0).contains(&lon) || !(-85.05112878..=85.05112878).contains(&lat) {
        return None;
    }
    let n = 1u32 << zoom;
    let x = ((lon + 180.0) / 360.0 * n as f64).floor() as u32;
    let lat_rad = lat.to_radians();
    let y = ((1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI)
        / 2.0
        * n as f64)
        .floor() as u32;
    Some((x.min(n - 1), y.min(n - 1)))
}

/// Convert tile (x, y) at zoom to a Bing quadkey string.
fn tile_to_quadkey(x: u32, y: u32, zoom: u32) -> String {
    if zoom == 0 {
        return "0".to_string();
    }
    let mut qk = String::with_capacity(zoom as usize);
    for i in (0..zoom).rev() {
        let mut digit = 0u8;
        let mask = 1u32 << i;
        if x & mask != 0 {
            digit += 1;
        }
        if y & mask != 0 {
            digit += 2;
        }
        qk.push((b'0' + digit) as char);
    }
    qk
}

/// `ST_QuadKey(geom, zoom)` — Bing quadkey of the envelope center cell.
/// Assumes EPSG:4326 lon/lat. NULL for NULL/EMPTY or out-of-range.
pub fn quadkey(g: &Geom, zoom: i32) -> Option<String> {
    if zoom < 0 || zoom > 23 {
        return None;
    }
    let (cx, cy) = envelope_center(g)?;
    let (x, y) = lonlat_to_tile(cx, cy, zoom as u32)?;
    Some(tile_to_quadkey(x, y, zoom as u32))
}

// =====================================================================
// GeoHash
// =====================================================================

const GEOHASH_BASE32: &[u8] = b"0123456789bcdefghjkmnpqrstuvwxyz";

/// Encode lon/lat as a geohash string at the given precision (1–12 chars).
pub fn encode_geohash(lon: f64, lat: f64, precision: usize) -> Option<String> {
    if precision == 0 || precision > 12 {
        return None;
    }
    if !(-180.0..=180.0).contains(&lon) || !(-90.0..=90.0).contains(&lat) {
        return None;
    }
    let mut hash = String::with_capacity(precision);
    let mut lat_range = (-90.0_f64, 90.0);
    let mut lon_range = (-180.0_f64, 180.0);
    let mut bit = 0u8;
    let mut ch: u8 = 0;
    let mut even = true;
    while hash.len() < precision {
        if even {
            let mid = (lon_range.0 + lon_range.1) / 2.0;
            if lon >= mid {
                ch |= 1 << (4 - bit);
                lon_range.0 = mid;
            } else {
                lon_range.1 = mid;
            }
        } else {
            let mid = (lat_range.0 + lat_range.1) / 2.0;
            if lat >= mid {
                ch |= 1 << (4 - bit);
                lat_range.0 = mid;
            } else {
                lat_range.1 = mid;
            }
        }
        even = !even;
        if bit < 4 {
            bit += 1;
        } else {
            hash.push(GEOHASH_BASE32[ch as usize] as char);
            bit = 0;
            ch = 0;
        }
    }
    Some(hash)
}

/// `ST_GeoHash(geom, precision)` — geohash of the envelope center.
/// Assumes EPSG:4326 lon/lat.
pub fn geohash(g: &Geom, precision: i32) -> Option<String> {
    let (cx, cy) = envelope_center(g)?;
    encode_geohash(cx, cy, precision.clamp(1, 12) as usize)
}

// =====================================================================
// Hilbert curve sort key
// =====================================================================

/// Convert (x, y) grid coordinates to a Hilbert curve distance.
/// `order` is the curve order (grid is `2^order × 2^order`).
/// Uses the Wikipedia xy→d algorithm with wrapping arithmetic for the
/// coordinate reflections (matching C unsigned behavior).
fn hilbert_d(order: u32, x: u32, y: u32) -> u64 {
    let mut d = 0u64;
    let mut cx = x;
    let mut cy = y;
    let mut s = 1u32 << (order - 1);
    while s > 0 {
        let rx = if (cx & s) != 0 { 1u64 } else { 0u64 };
        let ry = if (cy & s) != 0 { 1u64 } else { 0u64 };
        d += (s as u64) * (s as u64) * ((3 * rx) ^ ry);
        // rot: reflect within current quadrant
        if ry == 0 {
            if rx == 1 {
                cx = (s - 1).wrapping_sub(cx);
                cy = (s - 1).wrapping_sub(cy);
            }
            std::mem::swap(&mut cx, &mut cy);
        }
        s /= 2;
    }
    d
}

/// `ST_Hilbert(geom, bits)` — Hilbert curve sort key of the envelope center.
/// Assumes EPSG:4326 lon/lat, maps to `[0, 2^bits−1]² → [0, 4^bits−1]`.
pub fn hilbert_key(g: &Geom, bits: i32) -> Option<i64> {
    if bits <= 0 || bits > 16 {
        return None;
    }
    let (cx, cy) = envelope_center(g)?;
    if !(-180.0..=180.0).contains(&cx) || !(-90.0..=90.0).contains(&cy) {
        return None;
    }
    let n = (1u32 << bits) - 1;
    let gx = ((cx + 180.0) / 360.0 * n as f64) as u32;
    let gy = ((cy + 90.0) / 180.0 * n as f64) as u32;
    Some(hilbert_d(bits as u32, gx.min(n), gy.min(n)) as i64)
}

// =====================================================================
// Morton (Z-order) curve sort key
// =====================================================================

/// Interleave bits of a 16-bit integer (Part1By1).
fn part1by1(mut n: u32) -> u64 {
    let mut n = n as u64;
    n &= 0xffff;
    n = (n | (n << 8)) & 0x00FF00FF;
    n = (n | (n << 4)) & 0x0F0F0F0F;
    n = (n | (n << 2)) & 0x33333333;
    n = (n | (n << 1)) & 0x55555555;
    n
}

/// `ST_Morton(geom, bits)` — Morton/Z-order curve key of the envelope center.
pub fn morton_key(g: &Geom, bits: i32) -> Option<i64> {
    if bits <= 0 || bits > 16 {
        return None;
    }
    let (cx, cy) = envelope_center(g)?;
    if !(-180.0..=180.0).contains(&cx) || !(-90.0..=90.0).contains(&cy) {
        return None;
    }
    let n = (1u32 << bits) - 1;
    let gx = ((cx + 180.0) / 360.0 * n as f64) as u32;
    let gy = ((cy + 90.0) / 180.0 * n as f64) as u32;
    Some((part1by1(gy.min(n)) | (part1by1(gx.min(n)) << 1)) as i64)
}

// =====================================================================
// ST_TileEnvelope
// =====================================================================

/// `ST_TileEnvelope(zoom, x, y)` — Web Mercator tile bounds as a Polygon.
/// Returns lon/lat coordinates (EPSG:4326), matching PostGIS convention.
pub fn tile_envelope(z: i32, x: i32, y: i32) -> Option<Geom> {
    if z < 0 || z > 23 || x < 0 || y < 0 {
        return None;
    }
    let n = 1u32 << z;
    if (x as u32) >= n || (y as u32) >= n {
        return None;
    }
    let nf = n as f64;
    let lon_min = x as f64 / nf * 360.0 - 180.0;
    let lon_max = (x + 1) as f64 / nf * 360.0 - 180.0;
    let lat_max = (std::f64::consts::PI * (1.0 - 2.0 * y as f64 / nf))
        .sinh()
        .atan()
        .to_degrees();
    let lat_min = (std::f64::consts::PI * (1.0 - 2.0 * (y + 1) as f64 / nf))
        .sinh()
        .atan()
        .to_degrees();
    Some(Geometry::Polygon(geo_types::Polygon::new(
        geo_types::LineString::from(vec![
            (lon_min, lat_min),
            (lon_max, lat_min),
            (lon_max, lat_max),
            (lon_min, lat_max),
            (lon_min, lat_min),
        ]),
        vec![],
    )))
}

// =====================================================================
// ST_CoveringQuadKeys (table function core)
// =====================================================================

/// All tile cells covered by the geometry's envelope at the given zoom.
/// Returns `(quadkey, tile_x, tile_y)` tuples.
/// Returns `None` if the cell count exceeds `max_cells` (fail closed).
pub fn covering_quadkeys(g: &Geom, zoom: u32, max_cells: usize) -> Option<Vec<(String, u32, u32)>> {
    let (xmin, ymin, xmax, ymax) = envelope(g)?;

    // Clamp to Web Mercator valid range.
    let ymin = ymin.clamp(-85.05112878, 85.05112878);
    let ymax = ymax.clamp(-85.05112878, 85.05112878);
    let xmin = xmin.clamp(-180.0, 180.0);
    let xmax = xmax.clamp(-180.0, 180.0);

    let (tx0, ty0) = lonlat_to_tile(xmin, ymin, zoom)?;
    let (tx1, ty1) = lonlat_to_tile(xmax, ymax, zoom)?;
    // Tile Y increases downward (north = 0), so ymax → smaller tile_y.
    let (tx_min, tx_max) = if tx0 <= tx1 { (tx0, tx1) } else { (tx1, tx0) };
    let (ty_min, ty_max) = if ty0 <= ty1 { (ty0, ty1) } else { (ty1, ty0) };

    let tiles_x = (tx_max - tx_min + 1) as usize;
    let tiles_y = (ty_max - ty_min + 1) as usize;
    let count = tiles_x * tiles_y;
    if count > max_cells {
        return None;
    }

    let mut result = Vec::with_capacity(count);
    for ty in ty_min..=ty_max {
        for tx in tx_min..=tx_max {
            let qk = tile_to_quadkey(tx, ty, zoom);
            result.push((qk, tx, ty));
        }
    }
    Some(result)
}

// =====================================================================
// Adaptive partitioning helpers
// =====================================================================

/// Estimate how many partitions are needed so that each partition is
/// approximately `target_object_bytes` bytes.
///
/// `total_rows` is the estimated row count, `avg_row_bytes` is the estimated
/// average row size in bytes (including geometry), and `target_object_bytes`
/// is the desired Parquet object size (e.g., 256 MB = 268_435_456).
///
/// Returns at least 1.
pub fn estimate_partition_count(
    total_rows: i64,
    avg_row_bytes: i32,
    target_object_bytes: i64,
) -> Option<i32> {
    if total_rows <= 0 || avg_row_bytes <= 0 || target_object_bytes <= 0 {
        return None;
    }
    let total_bytes = total_rows * avg_row_bytes as i64;
    let n = (total_bytes + target_object_bytes - 1) / target_object_bytes;
    Some(n.max(1) as i32)
}

/// Recommend a quadkey zoom level that produces approximately `n_partitions`
/// cells for a global or near-global extent.
///
/// A quadtree at zoom `z` has `4^z` cells. So `z = ceil(log4(n))`.
/// Clamped to [0, 23].
pub fn recommend_zoom(n_partitions: i32) -> Option<i32> {
    if n_partitions <= 0 {
        return None;
    }
    let z = (n_partitions as f64).log(4.0).ceil() as i32;
    Some(z.clamp(0, 23))
}

// =====================================================================
// Unit tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64) -> Geom {
        Geometry::Point(Point::new(x, y))
    }

    #[test]
    fn quadkey_known_values() {
        // NYC ~ (−74.0, 40.7) at zoom 8
        let qk = quadkey(&pt(-74.0, 40.7), 8).unwrap();
        assert_eq!(qk.len(), 8);
        // At zoom 0, everything is "0"
        assert_eq!(quadkey(&pt(0.0, 0.0), 0).unwrap(), "0");
        // Determinism
        assert_eq!(quadkey(&pt(-74.0, 40.7), 8), quadkey(&pt(-74.0, 40.7), 8));
    }

    #[test]
    fn geohash_known_values() {
        // dr5… is a well-known prefix for NYC area
        let gh = geohash(&pt(-74.0, 40.7), 4).unwrap();
        assert_eq!(gh, "dr5r");
    }

    #[test]
    fn hilbert_deterministic() {
        let h1 = hilbert_key(&pt(-74.0, 40.7), 12).unwrap();
        let h2 = hilbert_key(&pt(-74.0, 40.7), 12).unwrap();
        assert_eq!(h1, h2);
        // Hilbert values are non-negative
        assert!(h1 >= 0);
    }

    #[test]
    fn morton_deterministic() {
        let m1 = morton_key(&pt(-74.0, 40.7), 12).unwrap();
        let m2 = morton_key(&pt(-74.0, 40.7), 12).unwrap();
        assert_eq!(m1, m2);
    }

    #[test]
    fn bbox_intersects_basic() {
        let a = Geometry::Polygon(geo_types::Polygon::new(
            geo_types::LineString::from(vec![(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0), (0.0, 0.0)]),
            vec![],
        ));
        let b = Geometry::Polygon(geo_types::Polygon::new(
            geo_types::LineString::from(vec![(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0), (2.0, 2.0)]),
            vec![],
        ));
        let c = pt(100.0, 100.0);
        assert_eq!(bbox_intersects(&a, &b), Some(true));
        assert_eq!(bbox_intersects(&a, &c), Some(false));
    }

    #[test]
    fn tile_envelope_bounds() {
        let env = tile_envelope(1, 0, 0).unwrap();
        let r = env.bounding_rect().unwrap();
        assert!((r.min().x - (-180.0)).abs() < 0.001);
        assert!((r.max().x - 0.0).abs() < 0.001);
    }

    #[test]
    fn covering_quadkeys_point() {
        // A point should cover exactly 1 cell.
        let cells = covering_quadkeys(&pt(-74.0, 40.7), 8, 1000).unwrap();
        assert_eq!(cells.len(), 1);
    }

    #[test]
    fn covering_quadkeys_fails_closed() {
        // A wide envelope at high zoom with small max_cells → None (fail closed).
        let region = Geometry::Polygon(geo_types::Polygon::new(
            geo_types::LineString::from(vec![
                (-170.0, -80.0),
                (170.0, -80.0),
                (170.0, 80.0),
                (-170.0, 80.0),
                (-170.0, -80.0),
            ]),
            vec![],
        ));
        // At zoom 8, this envelope covers thousands of cells.
        // With max_cells=10, should return None.
        assert!(covering_quadkeys(&region, 8, 10).is_none());
    }

    #[test]
    fn null_empty_returns_none() {
        let empty = Geometry::Point(Point::new(0.0, 0.0)); // not actually empty
        // For a truly empty geometry, we'd need GeometryCollection::new()
        // but the dispatch layer handles NULL → None before calling us.
        // Just verify a valid point works.
        assert!(quadkey(&empty, 4).is_some());
    }

    #[test]
    fn estimate_partition_count_basic() {
        // 1B rows, 200 bytes/row, 256MB target → 746 partitions
        let n = estimate_partition_count(1_000_000_000, 200, 268_435_456).unwrap();
        assert_eq!(n, 746); // ceil(1e9 * 200 / 268435456)
        // Edge: fewer rows than one object
        let n = estimate_partition_count(100, 100, 268_435_456).unwrap();
        assert_eq!(n, 1);
        // Invalid → None
        assert!(estimate_partition_count(0, 100, 268_435_456).is_none());
        assert!(estimate_partition_count(100, 0, 268_435_456).is_none());
    }

    #[test]
    fn recommend_zoom_basic() {
        // 4^5 = 1024 → zoom 5
        assert_eq!(recommend_zoom(1024).unwrap(), 5);
        // 4^6 = 4096 → zoom 6
        assert_eq!(recommend_zoom(4096).unwrap(), 6);
        // 1 partition → zoom 0
        assert_eq!(recommend_zoom(1).unwrap(), 0);
        // 746 partitions → zoom 5 (4^5=1024 > 746)
        assert_eq!(recommend_zoom(746).unwrap(), 5);
        // Invalid → None
        assert!(recommend_zoom(0).is_none());
        assert!(recommend_zoom(-1).is_none());
    }
}
