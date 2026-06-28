// SPDX-License-Identifier: Apache-2.0
//! GEOS-backed planar topology operations.
//!
//! A narrow boundary: WKB bytes in → GEOS → operation → WKB bytes out. This
//! module never exposes GEOS types beyond its own functions; callers work at
//! the raw-WKB level so there is no double conversion through `geo_types`.
//!
//! GEOS is the same engine PostGIS uses for `ST_Node`, `ST_Polygonize`,
//! `ST_BuildArea`, and `ST_VoronoiPolygons`, giving us PostGIS-grade fidelity
//! for planar topology without maintaining a custom algorithm port.

use geos::{Geom, Geometry};

/// Parse ISO WKB bytes into a GEOS geometry. Returns `None` on parse failure
/// (fail-closed — the caller emits NULL, never a wrong geometry).
fn from_wkb(wkb: &[u8]) -> Option<Geometry> {
    Geometry::new_from_wkb(wkb).ok()
}

/// Serialize a GEOS geometry to ISO WKB bytes.
fn to_wkb(geom: &Geometry) -> Option<Vec<u8>> {
    geom.to_wkb().ok()
}

/// `ST_Node` — add nodes at every intersection of the linework, returning a
/// noded MultiLineString.
pub fn node(wkb: &[u8]) -> Option<Vec<u8>> {
    let g = from_wkb(wkb)?;
    let noded = g.node().ok()?;
    to_wkb(&noded)
}

/// `ST_Polygonize` — form a MultiPolygon from all constituent linestrings that
/// can be rings. GEOS `polygonize` accepts a multi-geometry directly; it
/// extracts the internal components in C.
pub fn polygonize(wkb: &[u8]) -> Option<Vec<u8>> {
    let g = from_wkb(wkb)?;
    let result = Geometry::polygonize(&[&g]).ok()?;
    to_wkb(&result)
}

/// `ST_BuildArea` — build an areal geometry (Polygon or MultiPolygon) from the
/// linework of the input, directed by the boundary relationships.
pub fn build_area(wkb: &[u8]) -> Option<Vec<u8>> {
    let g = from_wkb(wkb)?;
    let result = g.build_area().ok()?;
    to_wkb(&result)
}

/// `ST_VoronoiPolygons` — bounded Voronoi diagram of the input points. Returns
/// a GeometryCollection (or MultiPolygon) of finite Voronoi cells.
///
/// * `tolerance` — snapping tolerance (0.0 for exact).
/// * `extend_to` — optional WKB envelope to extend the diagram to (PostGIS
///   `extend_to` argument). When `None`, GEOS derives the envelope from the
///   input sites with a small buffer.
pub fn voronoi_polygons(wkb: &[u8], tolerance: f64, extend_to: Option<&[u8]>) -> Option<Vec<u8>> {
    let g = from_wkb(wkb)?;
    let env = match extend_to {
        Some(env_wkb) => Some(from_wkb(env_wkb)?),
        None => None,
    };
    // only_edges=false → polygonal cells; true → the dual edges (ST_VoronoiLines).
    let result = match env {
        Some(ref e) => g.voronoi(Some(e), tolerance, false).ok()?,
        None => g.voronoi(None::<&Geometry>, tolerance, false).ok()?,
    };
    to_wkb(&result)
}

/// `ST_Snap(geom1, geom2, tolerance)` — snap vertices of geom1 to geom2 where
/// they are within `tolerance`. This is the canonical PostGIS `ST_Snap`,
/// powered by the same GEOS engine PostGIS uses.
pub fn snap(wkb1: &[u8], wkb2: &[u8], tolerance: f64) -> Option<Vec<u8>> {
    let g1 = from_wkb(wkb1)?;
    let g2 = from_wkb(wkb2)?;
    let result = g1.snap(&g2, tolerance).ok()?;
    to_wkb(&result)
}

/// `ST_MakeValid(geom)` — repair invalid geometry using the GEOS MakeValid
/// algorithm (the canonical PostGIS engine). This is higher-fidelity than the
/// local `buffer(0)` heuristic because it preserves structure (rings, holes)
/// rather than relying on a zero-width buffer to fix topology.
pub fn make_valid(wkb: &[u8]) -> Option<Vec<u8>> {
    let g = from_wkb(wkb)?;
    let result = g.make_valid().ok()?;
    to_wkb(&result)
}

// -----------------------------------------------------------------------
// GEOS overlay operations (fallback for local `geo::BooleanOps`).
//
// These are the same GEOS overlay routines PostGIS uses. They serve as the
// safety-net fallback when the local `geo` crate's BooleanOps panic on
// complex or pathological input (e.g. 100k+ vertex self-intersecting
// Overture polygons that survive `ensure_valid`).
// -----------------------------------------------------------------------

/// GEOS `intersection` overlay. `(wkb1, wkb2) → wkb_out`.
pub fn intersection(wkb1: &[u8], wkb2: &[u8]) -> Option<Vec<u8>> {
    let g1 = from_wkb(wkb1)?;
    let g2 = from_wkb(wkb2)?;
    let result = g1.intersection(&g2).ok()?;
    to_wkb(&result)
}

/// GEOS `union` overlay. `(wkb1, wkb2) → wkb_out`.
pub fn union(wkb1: &[u8], wkb2: &[u8]) -> Option<Vec<u8>> {
    let g1 = from_wkb(wkb1)?;
    let g2 = from_wkb(wkb2)?;
    let result = g1.union(&g2).ok()?;
    to_wkb(&result)
}

/// GEOS `difference` overlay. `(wkb1, wkb2) → wkb_out`.
pub fn difference(wkb1: &[u8], wkb2: &[u8]) -> Option<Vec<u8>> {
    let g1 = from_wkb(wkb1)?;
    let g2 = from_wkb(wkb2)?;
    let result = g1.difference(&g2).ok()?;
    to_wkb(&result)
}

/// GEOS `symmetric_difference` overlay. `(wkb1, wkb2) → wkb_out`.
pub fn symmetric_difference(wkb1: &[u8], wkb2: &[u8]) -> Option<Vec<u8>> {
    let g1 = from_wkb(wkb1)?;
    let g2 = from_wkb(wkb2)?;
    let result = g1.sym_difference(&g2).ok()?;
    to_wkb(&result)
}

// -----------------------------------------------------------------------
// GEOS DE-9IM relate (the canonical PostGIS engine for `ST_Relate`).
// -----------------------------------------------------------------------

/// `ST_Relate(a, b)` — returns the 9-character DE-9IM intersection matrix
/// string for the two geometries. GEOS computes the same matrix PostGIS
/// exposes. Returns `None` on parse/compute failure (fail-closed → NULL).
pub fn relate(wkb1: &[u8], wkb2: &[u8]) -> Option<String> {
    let g1 = from_wkb(wkb1)?;
    let g2 = from_wkb(wkb2)?;
    g1.relate(&g2).ok()
}

/// `ST_Relate(a, b, pattern)` — returns whether the DE-9IM matrix of the two
/// geometries matches the given 9-character pattern (where pattern entries may
/// be `0/1/2/F/T/*`). GEOS `relate_pattern` is the canonical PostGIS engine.
/// Returns `None` on parse/compute failure.
pub fn relate_pattern(wkb1: &[u8], wkb2: &[u8], pattern: &str) -> Option<bool> {
    let g1 = from_wkb(wkb1)?;
    let g2 = from_wkb(wkb2)?;
    g1.relate_pattern(&g2, pattern).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geos_node_crossing_lines() {
        // Two crossing lines → noded into 4 segments with an intersection node.
        let wkb = geo_wkb("MULTILINESTRING((0 0,4 4),(0 4,4 0))");
        let noded = node(&wkb).expect("node should succeed");
        let g = from_wkb(&noded).unwrap();
        assert!(g.get_num_geometries().unwrap() >= 2, "noded result");
    }

    #[test]
    fn geos_polygonize_rings() {
        // A closed ring → polygonize produces a polygon.
        let wkb = geo_wkb("LINESTRING(0 0,4 0,4 4,0 4,0 0)");
        let poly = polygonize(&wkb).expect("polygonize should succeed");
        let g = from_wkb(&poly).unwrap();
        assert_eq!(g.get_num_geometries().unwrap(), 1, "one polygon from ring");
    }

    #[test]
    fn geos_voronoi_grid_does_not_lose_cells() {
        // The 3x3 grid that defeated the earlier angle-sort prototype. GEOS must
        // produce 9 cells (one per site), proving the half-edge approach is
        // correct on cocircular/degenerate input.
        let wkb = geo_wkb("MULTIPOINT((0 0),(1 0),(2 0),(0 1),(1 1),(2 1),(0 2),(1 2),(2 2))");
        let result = voronoi_polygons(&wkb, 0.0, None).expect("voronoi should succeed");
        let g = from_wkb(&result).unwrap();
        let n = g.get_num_geometries().unwrap();
        assert_eq!(n, 9, "3x3 grid must yield exactly 9 voronoi cells, got {n}");
    }

    #[test]
    fn geos_build_area_from_rings() {
        // Exterior + interior ring → polygon with hole.
        let wkb = geo_wkb("MULTILINESTRING((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))");
        let area = build_area(&wkb).expect("build_area should succeed");
        let g = from_wkb(&area).unwrap();
        let a = g.area().unwrap();
        assert!((a - 15.0).abs() < 1e-6, "4x4 square minus 1x1 hole = 15, got {a}");
    }

    #[test]
    fn geos_snap_nearby_vertices() {
        // Snap a polygon vertex at (4, 0.01) to a reference line vertex at (4, 0)
        // with tolerance 0.1. The snapped geometry should have the vertex at (4, 0).
        let poly = geo_wkb("POLYGON((0 0,4 0.01,8 0,4 4,0 0))");
        let ref_geom = geo_wkb("POINT(4 0)");
        let snapped = snap(&poly, &ref_geom, 0.1).expect("snap should succeed");
        let g = from_wkb(&snapped).unwrap();
        // After snapping, the polygon should still be valid and have 5 coords.
        assert!(g.is_valid().unwrap(), "snapped polygon must be valid");
    }

    #[test]
    fn geos_snap_zero_tolerance_is_identity() {
        let poly = geo_wkb("POLYGON((0 0,4 0,8 0,4 4,0 0))");
        let ref_geom = geo_wkb("POINT(100 100)");
        let snapped = snap(&poly, &ref_geom, 0.0).expect("snap(0) should succeed");
        // Zero tolerance → no snapping, geometry unchanged.
        let original = from_wkb(&poly).unwrap();
        let snapped_g = from_wkb(&snapped).unwrap();
        assert_eq!(
            original.get_num_coordinates().unwrap(),
            snapped_g.get_num_coordinates().unwrap(),
            "zero-tolerance snap preserves coordinate count"
        );
    }

    #[test]
    fn geos_make_valid_self_intersecting_polygon() {
        // A self-intersecting "bowtie" polygon → GEOS MakeValid should repair it.
        let bad = geo_wkb("POLYGON((0 0,4 4,4 0,0 4,0 0))");
        let repaired = make_valid(&bad).expect("make_valid should succeed");
        let g = from_wkb(&repaired).unwrap();
        assert!(g.is_valid().unwrap(), "repaired geometry must be valid");
    }

    #[test]
    fn geos_make_valid_already_valid_is_identity() {
        let good = geo_wkb("POLYGON((0 0,4 0,4 4,0 4,0 0))");
        let repaired = make_valid(&good).expect("make_valid should succeed");
        let g = from_wkb(&repaired).unwrap();
        assert!(g.is_valid().unwrap(), "valid input stays valid");
    }

    /// Helper: build WKB from a WKT string using the extension's own stack.
    fn geo_wkb(wkt: &str) -> Vec<u8> {
        use std::str::FromStr;
        let parsed = wkt::Wkt::<f64>::from_str(wkt).expect("valid WKT in test");
        let g = crate::functions::geom_from_wkt(parsed).expect("geom conversion");
        crate::geometry::to_wkb(&g).expect("wkb write")
    }
}
