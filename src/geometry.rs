// SPDX-License-Identifier: Apache-2.0
//
// WKB <-> geo_types geometry conversion.
//
// This is the trust boundary where DuckDB BLOB values (ISO Well-Known Binary)
// are parsed into owned, owned-able `geo_types::Geometry<f64>` values that the
// `geo` crate's algorithms operate on, and where results are serialized back
// to WKB. We use exactly the same crates Apache SedonaDB uses internally
// (`wkb`, `geo-traits`, `geo-types`), so behaviour stays compatible.
//
// The `geo-traits -> geo-types` converter below is adapted from Apache
// SedonaDB's `rust/sedona-geo/src/to_geo.rs` (Apache-2.0). It is deliberately
// limited: geometries that `geo-types` cannot represent (`POINT EMPTY`, a
// `MULTIPOINT` with an `EMPTY` child, and arbitrarily-nested geometry
// collections) are reported as errors and the calling executor emits NULL.

use geo_traits::to_geo::{
    ToGeoLineString, ToGeoMultiLineString, ToGeoMultiPoint, ToGeoMultiPolygon, ToGeoPoint,
    ToGeoPolygon,
};
use geo_traits::{GeometryCollectionTrait, GeometryTrait, GeometryType};
use geo_types::Geometry;
use wkb::reader::read_wkb;
use wkb::Endianness;
use wkb::writer::{write_geometry, WriteOptions};

/// Concrete owned geometry type used throughout the extension.
pub type Geom = Geometry<f64>;

/// A cheap, descriptive error used to drive NULL-on-failure semantics in the
/// dispatch layer without pulling in `thiserror` for the hot path.
#[derive(Debug)]
pub struct GeometryError(pub &'static str);

impl core::fmt::Display for GeometryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for GeometryError {}

/// Parse a WKB (or EWKB) byte slice into an owned [`Geom`].
///
/// Returns an error for truncated / malformed WKB or for geometries that
/// `geo-types` cannot represent (see the module docs).
///
/// **EWKB tolerance:** EWKB is WKB with an optional SRID flag (`0x20000000`) in
/// the type word and, when set, a 4-byte SRID after the type word. The `wkb`
/// crate reader does not understand that flag, so this entry point strips it
/// from the *top-level* geometry before parsing (children never carry it). This
/// keeps every ST_* function robust to EWKB input (which DuckDB's own `spatial`
/// extension produces) without changing behaviour for plain WKB. The SRID
/// itself is discarded: this extension carries no SRID in its `Geom` type.
pub fn from_wkb(bytes: &[u8]) -> Result<Geom, GeometryError> {
    let normalized = strip_ewkb_srid(bytes);
    let wkb = read_wkb(&normalized).map_err(|_| GeometryError("invalid or truncated WKB"))?;
    to_geometry(&wkb).ok_or(GeometryError(
        "unsupported geometry (e.g. POINT EMPTY or nested GEOMETRYCOLLECTION)",
    ))
}

/// If `bytes` is little- or big-endian EWKB whose top-level geometry type
/// carries the SRID flag, return an owned slice with the flag cleared and the
/// 4 SRID bytes removed. Otherwise return a borrow of the input unchanged.
/// Owned output is dropped by the caller, so there is no leak.
fn strip_ewkb_srid(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    use std::borrow::Cow;
    // EWKB layout: [endian:1][type:4][optional srid:4][coords...].
    if bytes.len() < 5 {
        return Cow::Borrowed(bytes);
    }
    let little = bytes[0] == 1;
    let type_bytes: [u8; 4] = match bytes[1..5].try_into() {
        Ok(b) => b,
        Err(_) => return Cow::Borrowed(bytes),
    };
    let type_word = if little {
        u32::from_le_bytes(type_bytes)
    } else {
        u32::from_be_bytes(type_bytes)
    };
    const SRID_FLAG: u32 = 0x2000_0000;
    if type_word & SRID_FLAG == 0 {
        return Cow::Borrowed(bytes);
    }
    // Strip the flag and drop the 4 SRID bytes after the type word.
    let cleared = type_word & !SRID_FLAG;
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len().saturating_sub(4));
    out.push(bytes[0]);
    out.extend_from_slice(&if little {
        cleared.to_le_bytes()
    } else {
        cleared.to_be_bytes()
    });
    if bytes.len() >= 9 {
        // bytes[5..9] is the SRID we drop; bytes[9..] is the coordinate payload.
        out.extend_from_slice(&bytes[9..]);
    }
    Cow::Owned(out)
}

/// Serialize a [`Geom`] back to little-endian ISO WKB.
pub fn to_wkb(geom: &Geom) -> Result<Vec<u8>, GeometryError> {
    let mut buf = Vec::new();
    write_geometry(
        &mut buf,
        geom,
        &WriteOptions {
            endianness: Endianness::LittleEndian,
        },
    )
    .map_err(|_| GeometryError("failed to serialize WKB"))?;
    Ok(buf)
}

/// Convert any `geo-traits` geometry into an owned [`Geometry`].
///
/// Mirrors SedonaDB's `to_geometry`. Geometry collections are flattened one
/// level deep to stay compatible with the `geo-types` representation.
fn to_geometry(g: &impl GeometryTrait<T = f64>) -> Option<Geometry> {
    match g.as_type() {
        GeometryType::Point(geom) => geom.try_to_point().map(Geometry::Point),
        GeometryType::LineString(geom) => Some(Geometry::LineString(geom.to_line_string())),
        GeometryType::Polygon(geom) => Some(Geometry::Polygon(geom.to_polygon())),
        GeometryType::MultiPoint(geom) => geom.try_to_multi_point().map(Geometry::MultiPoint),
        GeometryType::MultiLineString(geom) => Some(Geometry::MultiLineString(geom.to_multi_line_string())),
        GeometryType::MultiPolygon(geom) => Some(Geometry::MultiPolygon(geom.to_multi_polygon())),
        GeometryType::GeometryCollection(geom) => geometry_collection_to_geometry(geom),
        _ => None,
    }
}

fn geometry_collection_to_geometry<GC: GeometryCollectionTrait<T = f64>>(
    geom: &GC,
) -> Option<Geometry> {
    let geometries: Vec<Geometry> = geom
        .geometries()
        .filter_map(|child| match child.as_type() {
            GeometryType::Point(g) => g.try_to_point().map(Geometry::Point),
            GeometryType::LineString(g) => Some(Geometry::LineString(g.to_line_string())),
            GeometryType::Polygon(g) => Some(Geometry::Polygon(g.to_polygon())),
            GeometryType::MultiPoint(g) => g.try_to_multi_point().map(Geometry::MultiPoint),
            GeometryType::MultiLineString(g) => Some(Geometry::MultiLineString(g.to_multi_line_string())),
            GeometryType::MultiPolygon(g) => Some(Geometry::MultiPolygon(g.to_multi_polygon())),
            GeometryType::GeometryCollection(g) => geometry_collection_to_geometry(g),
            _ => None,
        })
        .collect();

    // If any child conversion failed, surface the whole collection as None so
    // the caller emits NULL rather than silently dropping geometries.
    if geometries.len() != geom.num_geometries() {
        return None;
    }

    Some(Geometry::GeometryCollection(geo_types::GeometryCollection(
        geometries,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    // WKB for POINT (1 2), little-endian.
    const POINT_WKB: [u8; 21] = [
        0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x40,
    ];

    #[test]
    fn roundtrip_point() {
        let g = from_wkb(&POINT_WKB).expect("parse POINT");
        match g {
            Geometry::Point(p) => {
                assert!((p.x() - 1.0).abs() < f64::EPSILON);
                assert!((p.y() - 2.0).abs() < f64::EPSILON);
            }
            other => panic!("expected Point, got {other:?}"),
        }

        let bytes = to_wkb(&g).expect("serialize POINT");
        assert_eq!(bytes, POINT_WKB);
    }

    #[test]
    fn rejects_garbage() {
        assert!(from_wkb(&[0u8; 3]).is_err());
        assert!(from_wkb(&[]).is_err());
    }

    // EWKB for POINT(1 2) with SRID=4326, little-endian. Type word = 0x2000_0001.
    const POINT_EWKB_SRID: [u8; 25] = [
        0x01,                                   // little-endian
        0x01, 0x00, 0x00, 0x20,                 // type = 0x20000001 (Point + SRID flag)
        0xe0, 0x01, 0x00, 0x00,                 // SRID = 4326 (LE)
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, // x = 1.0
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, // y = 2.0
    ];

    #[test]
    fn parses_ewkb_with_srid() {
        // The extension is SRID-less in Geom, but must accept EWKB input by
        // stripping the top-level SRID flag + value.
        let g = from_wkb(&POINT_EWKB_SRID).expect("parse EWKB POINT");
        match g {
            Geometry::Point(p) => {
                assert!((p.x() - 1.0).abs() < f64::EPSILON);
                assert!((p.y() - 2.0).abs() < f64::EPSILON);
            }
            other => panic!("expected Point, got {other:?}"),
        }
    }

    #[test]
    fn plain_wkb_unchanged_by_ewkb_strip() {
        // Plain WKB (no SRID flag) must round-trip identically.
        let g = from_wkb(&POINT_WKB).expect("parse POINT");
        let bytes = to_wkb(&g).expect("serialize POINT");
        assert_eq!(bytes, POINT_WKB);
    }
}
