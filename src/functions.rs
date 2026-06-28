// SPDX-License-Identifier: Apache-2.0
//
// Spatial function implementations.
//
// Every function here is a *plain* `fn` with one of the shapes expected by the
// generic executors in `dispatch.rs`:
//
//   fn(&Geom) -> Option<Geom>            unary, geometry -> geometry
//   fn(&Geom, &Geom) -> Option<Geom>     binary, geometry -> geometry
//   fn(&Geom, &Geom) -> Option<bool>     binary predicate
//   fn(&Geom) -> Option<f64>             geometry -> DOUBLE
//   fn(&Geom) -> Option<String>          geometry -> VARCHAR
//   fn(&Geom) -> Option<i32>             geometry -> INTEGER
//
// The algorithms come from the `geo` crate — the same library Apache SedonaDB
// uses for these operations (`sedona-geo` wraps `geo` as its DataFusion UDFs).
// `None` means "undefined for this input" and propagates to SQL `NULL`.

use geo::algorithm::bool_ops::BooleanOps;
use geo::prelude::*;
#[allow(deprecated)]
use geo::EuclideanDistance;
use geo::Validation;
use geo_types::{Geometry, MultiPolygon, Point};

use crate::geometry::Geom;

/// Reduce a geometry to its areal (polygonal) part as a `MultiPolygon`.
///
/// `geo`'s `BooleanOps` are only implemented for `Polygon` / `MultiPolygon`,
/// so `ST_Intersection` / `ST_Union` operate on the polygonal part of each
/// input. Non-areal geometries contribute an empty `MultiPolygon`, which means
/// they yield an empty result — the standard OGC behaviour for boolean ops on
/// non-polygonal inputs.
fn to_multi_polygon(g: &Geom) -> MultiPolygon {
    let polys: Vec<geo_types::Polygon> = match g {
        Geometry::Polygon(p) => vec![p.clone()],
        Geometry::MultiPolygon(mp) => mp.0.clone(),
        Geometry::GeometryCollection(c) => {
            let mut polys = Vec::new();
            for item in c.iter() {
                match item {
                    Geometry::Polygon(p) => polys.push(p.clone()),
                    Geometry::MultiPolygon(mp) => polys.extend(mp.0.iter().cloned()),
                    _ => {}
                }
            }
            polys
        }
        _ => Vec::new(),
    };
    MultiPolygon::new(polys)
}

// ----- unary: geometry -> geometry --------------------------------------

/// `ST_ConvexHull(geom)` — convex hull of the geometry's coordinates.
pub fn convex_hull(g: &Geom) -> Option<Geom> {
    match g {
        Geometry::Point(_) => Some(g.clone()),
        _ => Some(g.convex_hull().into()),
    }
}

/// `ST_Envelope(geom)` — minimum bounding rectangle as a Polygon
/// (or NULL when the input is degenerate).
pub fn envelope(g: &Geom) -> Option<Geom> {
    let rect = g.bounding_rect()?;
    Some(Geometry::Polygon(rect.to_polygon()))
}

/// `ST_Centroid(geom)` — planar centroid, or NULL if undefined.
pub fn centroid(g: &Geom) -> Option<Geom> {
    let point: Point = g.centroid()?;
    Some(Geometry::Point(point))
}

// ----- set operations ---------------------------------------------------

/// `ST_MakeValid(geom)` — repair an invalid geometry.
///
/// `geo`'s DE-9IM `relate` (used by `ST_Within`/`Contains`/`Covers`/…) and its
/// boolean ops can stack-overflow / misbehave on degenerate (self-intersecting,
/// over-complex) polygons — which real-world datasets (e.g. Overture admin
/// boundaries in SpatialBench) do contain. The classic repair is `buffer(0)`,
/// which rebuilds topology with even-odd fill and drops the self-intersections.
/// For already-valid input this is a no-op clone; for non-areal input (points /
/// lines) we return the value unchanged (those rarely go invalid).
pub fn make_valid(g: &Geom) -> Option<Geom> {
    use geo::Validation;
    if g.is_valid() {
        return Some(g.clone());
    }
    match g {
        Geometry::Polygon(_) | Geometry::MultiPolygon(_) => {
            // buffer(0) -> cleaned MultiPolygon (even-odd fill).
            Some(Geometry::MultiPolygon(g.buffer(0.0)))
        }
        // Lines/points/collections: structural validity issues are uncommon
        // here; return as-is rather than risk losing detail.
        _ => Some(g.clone()),
    }
}

/// Borrow an input geometry, repairing it only if it is invalid. Used to guard
/// every relate-based predicate and boolean op so invalid real-world polygons
/// don't crash the extension. Cheap on the hot path: `is_valid` is a structural
/// check, and valid geometries (the vast majority) are borrowed, not copied.
fn ensure_valid<'a>(g: &'a Geom) -> std::borrow::Cow<'a, Geom> {
    use geo::Validation;
    if g.is_valid() {
        std::borrow::Cow::Borrowed(g)
    } else {
        std::borrow::Cow::Owned(make_valid(g).unwrap_or_else(|| g.clone()))
    }
}

/// `ST_Intersection(a, b)`.
pub fn intersection(a: &Geom, b: &Geom) -> Option<Geom> {
    let av = ensure_valid(a);
    let bv = ensure_valid(b);
    Some(Geometry::MultiPolygon(
        to_multi_polygon(&av).intersection(&to_multi_polygon(&bv)),
    ))
}

/// `ST_Union(a, b)`.
pub fn union(a: &Geom, b: &Geom) -> Option<Geom> {
    let av = ensure_valid(a);
    let bv = ensure_valid(b);
    Some(Geometry::MultiPolygon(
        to_multi_polygon(&av).union(&to_multi_polygon(&bv)),
    ))
}

// ----- binary: geometry, geometry -> boolean ----------------------------

/// `ST_Intersects(a, b)`. (Uses `geo`'s sweep-line `Intersects`, which is
/// robust on invalid input — no `make_valid` needed.)
pub fn intersects(a: &Geom, b: &Geom) -> Option<bool> {
    Some(a.intersects(b))
}

/// `ST_Contains(a, b)` — a fully contains b. Point operand uses our own robust
/// ray-cast PIP (see `point_in_geometry`); the general case falls back to geo's
/// `Contains` guarded by `ensure_valid`.
pub fn contains(a: &Geom, b: &Geom) -> Option<bool> {
    match b {
        Geometry::Point(p) => Some(point_in_geometry(p, a)),
        _ => {
            use geo::Contains;
            let av = ensure_valid(a);
            let bv = ensure_valid(b);
            Some((&*av).contains(&*bv))
        }
    }
}

/// `ST_Within(a, b)` — a is fully contained by b. Point operand uses our own
/// robust ray-cast PIP (the SpatialBench join shape: trip point within a zone
/// polygon). General case falls back to geo's `Contains` guarded by
/// `ensure_valid`.
pub fn within(a: &Geom, b: &Geom) -> Option<bool> {
    match a {
        Geometry::Point(p) => Some(point_in_geometry(p, b)),
        _ => {
            use geo::Contains;
            let av = ensure_valid(a);
            let bv = ensure_valid(b);
            Some((&*bv).contains(&*av))
        }
    }
}

/// Robust point-in-geometry test via even-odd ray casting.
///
/// We implement this ourselves rather than calling `geo`'s `Contains<Point>`
/// because geo's point-in-polygon path stack-overflows on the over-complex
/// (100k+ vertex) and self-intersecting polygons found in real datasets such
/// as Overture admin boundaries. PNPOLY ray casting is iterative O(n), cannot
/// recurse, and yields a well-defined "is the point in the filled area" answer
/// even for self-intersecting rings.
fn point_in_geometry(p: &geo_types::Point<f64>, g: &Geom) -> bool {
    let (x, y) = (p.x(), p.y());
    match g {
        Geometry::Polygon(poly) => point_in_polygon(x, y, poly),
        Geometry::MultiPolygon(mp) => mp.0.iter().any(|poly| point_in_polygon(x, y, poly)),
        Geometry::GeometryCollection(c) => c.0.iter().any(|item| point_in_geometry(p, item)),
        Geometry::LineString(ls) => ls.0.iter().any(|c| c.x == x && c.y == y),
        Geometry::Line(l) => {
            // point-on-segment
            let same_side = (x - l.start.x) * (l.end.y - l.start.y)
                == (y - l.start.y) * (l.end.x - l.start.x);
            same_side
                && (x >= f64::min(l.start.x, l.end.x) && x <= f64::max(l.start.x, l.end.x))
                && (y >= f64::min(l.start.y, l.end.y) && y <= f64::max(l.start.y, l.end.y))
        }
        Geometry::Point(other) => other.x() == x && other.y() == y,
        _ => false,
    }
}

/// PNPOLY even-odd ray cast: inside the exterior ring and outside every hole.
fn point_in_polygon(x: f64, y: f64, poly: &geo_types::Polygon<f64>) -> bool {
    let inside_ring = |ring: &geo_types::LineString<f64>| {
        let pts = &ring.0;
        let n = pts.len();
        if n < 3 {
            return false;
        }
        let mut c = false;
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = (pts[i].x, pts[i].y);
            let (xj, yj) = (pts[j].x, pts[j].y);
            if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi) {
                c = !c;
            }
            j = i;
        }
        c
    };
    let in_exterior = inside_ring(poly.exterior());
    poly.interiors().iter().fold(in_exterior, |acc, hole| acc && !inside_ring(hole))
}

/// `ST_Disjoint(a, b)` — negation of ST_Intersects.
pub fn disjoint(a: &Geom, b: &Geom) -> Option<bool> {
    Some(!a.intersects(b))
}

// ----- unary: geometry -> DOUBLE ----------------------------------------

/// `ST_Area(geom)` — unsigned planar area. Zero for non-areal geometries.
pub fn area(g: &Geom) -> Option<f64> {
    Some(g.unsigned_area())
}

/// `ST_X(point)` — x ordinate of a Point, else NULL.
pub fn x(g: &Geom) -> Option<f64> {
    match g {
        Geometry::Point(p) => Some(p.x()),
        _ => None,
    }
}

/// `ST_Y(point)` — y ordinate of a Point, else NULL.
pub fn y(g: &Geom) -> Option<f64> {
    match g {
        Geometry::Point(p) => Some(p.y()),
        _ => None,
    }
}

// ----- unary: geometry -> VARCHAR ---------------------------------------

/// `ST_GeometryType(geom)` — OGC style type name (e.g. `ST_Point`).
pub fn geometry_type(g: &Geom) -> Option<String> {
    let name = match g {
        Geometry::Point(_) => "ST_Point",
        Geometry::Line(_) => "ST_LineString",
        Geometry::LineString(_) => "ST_LineString",
        Geometry::Polygon(_) => "ST_Polygon",
        Geometry::MultiPoint(_) => "ST_MultiPoint",
        Geometry::MultiLineString(_) => "ST_MultiLineString",
        Geometry::MultiPolygon(_) => "ST_MultiPolygon",
        Geometry::GeometryCollection(_) => "ST_GeometryCollection",
        Geometry::Rect(_) => "ST_Polygon",
        Geometry::Triangle(_) => "ST_Polygon",
    };
    Some(name.to_string())
}

// ----- unary: geometry -> INTEGER ---------------------------------------

/// `ST_Dimension(geom)` — inherent dimension (Point=0, Line=1, Polygon=2, ...).
pub fn dimension(g: &Geom) -> Option<i32> {
    let dim = match g {
        Geometry::Point(_) | Geometry::MultiPoint(_) => 0,
        Geometry::Line(_) | Geometry::LineString(_) | Geometry::MultiLineString(_) => 1,
        Geometry::Polygon(_) | Geometry::MultiPolygon(_) | Geometry::Rect(_) | Geometry::Triangle(_) => 2,
        Geometry::GeometryCollection(c) => c.iter().map(|item| dimension(item).unwrap_or(0)).max().unwrap_or(0),
    };
    Some(dim)
}

// ----- constructors & I/O -----------------------------------------------

/// `ST_GeomFromText(wkt)` — parse Well-Known Text into a geometry.
pub fn geom_from_text(s: &str) -> Option<Geom> {
    use std::str::FromStr;
    let parsed = wkt::Wkt::<f64>::from_str(s).ok()?;
    // `Wkt::to_geometry` yields a `geo_types::Geometry`; map Option/Result
    // shapes defensively without depending on which the crate returns.
    geom_from_wkt(parsed)
}

#[allow(clippy::needless_pass_by_value)]
fn geom_from_wkt(parsed: wkt::Wkt<f64>) -> Option<Geom> {
    use std::convert::TryInto;
    // Prefer the explicit TryFrom<Wkt> for Geometry when available; fall back
    // to the inherent `to_geometry` accessor used by upstream SedonaDB.
    TryInto::<Geom>::try_into(parsed).ok()
}

/// `ST_AsText(geom)` — serialize a geometry to Well-Known Text.
pub fn as_text(g: &Geom) -> Option<String> {
    let mut out = String::new();
    wkt::to_wkt::write_geometry(&mut out, g).ok()?;
    Some(out)
}

/// `ST_Point(x, y)` — construct a 2D point.
pub fn point(x: f64, y: f64) -> Option<Geom> {
    Some(Geometry::Point(geo_types::Point::new(x, y)))
}

/// `ST_GeomFromWKB(blob)` — parse + re-serialize WKB (validates and normalizes).
pub fn geom_from_wkb(g: &Geom) -> Option<Geom> {
    Some(g.clone())
}

// ----- measurements -----------------------------------------------------

/// `ST_Length(geom)` — planar length of linear geometries (0 for points/areas).
#[allow(deprecated)]
pub fn length(g: &Geom) -> Option<f64> {
    use geo::EuclideanLength;
    fn ring_len(p: &geo_types::Polygon<f64>) -> f64 {
        use geo::EuclideanLength as _;
        p.exterior().euclidean_length()
            + p.interiors().iter().map(|r| r.euclidean_length()).sum::<f64>()
    }
    Some(match g {
        Geometry::Line(l) => l.euclidean_length(),
        Geometry::LineString(ls) => ls.euclidean_length(),
        Geometry::MultiLineString(mls) => mls.euclidean_length(),
        Geometry::Polygon(p) => ring_len(p),
        Geometry::MultiPolygon(mp) => mp.0.iter().map(ring_len).sum::<f64>(),
        Geometry::GeometryCollection(c) => c.iter().filter_map(length).sum(),
        _ => 0.0,
    })
}

/// `ST_Distance(a, b)` — planar Euclidean distance between geometries.
#[allow(deprecated)]
pub fn distance(a: &Geom, b: &Geom) -> Option<f64> {
    Some(a.euclidean_distance(b))
}

/// `ST_DWithin(a, b, distance)` — true when the planar distance from `a` to
/// `b` is `<= distance`.
#[allow(deprecated)]
pub fn dwithin(a: &Geom, b: &Geom, distance: f64) -> Option<bool> {
    Some(a.euclidean_distance(b) <= distance)
}

// ----- transforms -------------------------------------------------------

/// `ST_Buffer(geom, radius)` — polygon buffer at `radius`.
pub fn buffer(g: &Geom, radius: f64) -> Option<Geom> {
    Some(Geometry::MultiPolygon(g.buffer(radius)))
}

/// `ST_Simplify(geom, epsilon)` — Ramer-Douglas-Peucker simplification.
pub fn simplify(g: &Geom, epsilon: f64) -> Option<Geom> {
    use geo::Simplify as _;
    Some(match g {
        Geometry::LineString(ls) => Geometry::LineString(ls.simplify(epsilon)),
        Geometry::MultiLineString(mls) => Geometry::MultiLineString(mls.simplify(epsilon)),
        Geometry::Polygon(p) => Geometry::Polygon(p.simplify(epsilon)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(mp.simplify(epsilon)),
        Geometry::GeometryCollection(c) => {
            let items: Vec<_> = c.iter().filter_map(|item| simplify(item, epsilon)).collect();
            Geometry::GeometryCollection(geo_types::GeometryCollection(items))
        }
        other => other.clone(),
    })
}

// ----- set operations ---------------------------------------------------

/// `ST_Difference(a, b)`. Guards via `ensure_valid`.
pub fn difference(a: &Geom, b: &Geom) -> Option<Geom> {
    let av = ensure_valid(a);
    let bv = ensure_valid(b);
    Some(Geometry::MultiPolygon(
        to_multi_polygon(&av).difference(&to_multi_polygon(&bv)),
    ))
}

/// `ST_SymDifference(a, b)`. Guards via `ensure_valid`.
pub fn sym_difference(a: &Geom, b: &Geom) -> Option<Geom> {
    let av = ensure_valid(a);
    let bv = ensure_valid(b);
    Some(Geometry::MultiPolygon(
        to_multi_polygon(&av).xor(&to_multi_polygon(&bv)),
    ))
}

/// `ST_MakeLine(a, b)` — line string through the point coordinates of `a`
/// then `b`. Used by SpatialBench Q7 to build a trip segment from pickup/dropoff.
pub fn make_line(a: &Geom, b: &Geom) -> Option<Geom> {
    let pa = point_coord(a)?;
    let pb = point_coord(b)?;
    Some(Geometry::LineString(geo_types::LineString::from(vec![pa, pb])))
}

/// Extract the (x, y) of a Point geometry, or `None` for non-points.
fn point_coord(g: &Geom) -> Option<geo_types::Coord<f64>> {
    match g {
        Geometry::Point(p) => Some(p.0),
        _ => None,
    }
}

// ----- validity & shape -------------------------------------------------

/// `ST_IsValid(geom)` — passes `geo`'s structural validation.
pub fn is_valid(g: &Geom) -> Option<bool> {
    Some(g.is_valid())
}

/// `ST_IsEmpty(geom)` — true for geometries with no coordinate content.
pub fn is_empty(g: &Geom) -> Option<bool> {
    Some(match g {
        Geometry::Point(_) | Geometry::Line(_) | Geometry::Rect(_) | Geometry::Triangle(_) => false,
        Geometry::MultiPoint(mp) => mp.0.is_empty(),
        Geometry::LineString(ls) => ls.0.is_empty(),
        Geometry::MultiLineString(mls) => mls.0.is_empty(),
        Geometry::Polygon(p) => p.exterior().0.is_empty(),
        Geometry::MultiPolygon(mp) => mp.0.is_empty(),
        Geometry::GeometryCollection(c) => c.0.is_empty(),
    })
}

/// `ST_NumPoints(geom)` — total vertex count across the geometry.
pub fn num_points(g: &Geom) -> Option<i32> {
    let n: usize = match g {
        Geometry::Point(_) => 1,
        Geometry::MultiPoint(mp) => mp.0.len(),
        Geometry::Line(_) => 2,
        Geometry::LineString(ls) => ls.0.len(),
        Geometry::MultiLineString(mls) => mls.0.iter().map(|ls| ls.0.len()).sum(),
        Geometry::Polygon(p) => {
            p.exterior().0.len() + p.interiors().iter().map(|r| r.0.len()).sum::<usize>()
        }
        Geometry::MultiPolygon(mp) => mp
            .0
            .iter()
            .map(|p| p.exterior().0.len() + p.interiors().iter().map(|r| r.0.len()).sum::<usize>())
            .sum(),
        Geometry::Rect(_) => 5,
        Geometry::Triangle(_) => 4,
        Geometry::GeometryCollection(c) => c.0.iter().map(|item| num_points(item).unwrap_or(0) as usize).sum(),
    };
    n.try_into().ok()
}

// ----- bounding-box accessors (used for join prefiltering) ---------------

/// `ST_XMin(geom)`.
pub fn xmin(g: &Geom) -> Option<f64> {
    g.bounding_rect().map(|r| r.min().x)
}
/// `ST_XMax(geom)`.
pub fn xmax(g: &Geom) -> Option<f64> {
    g.bounding_rect().map(|r| r.max().x)
}
/// `ST_YMin(geom)`.
pub fn ymin(g: &Geom) -> Option<f64> {
    g.bounding_rect().map(|r| r.min().y)
}
/// `ST_YMax(geom)`.
pub fn ymax(g: &Geom) -> Option<f64> {
    g.bounding_rect().map(|r| r.max().y)
}

// ----- DE-9IM predicates (via geo::Relate) -------------------------------
// All route through `geo`'s geomgraph `relate`, which can stack-overflow on
// invalid polygons — so every one guards both inputs with `ensure_valid`.

/// `ST_Equals(a, b)` — topological equality.
pub fn equals(a: &Geom, b: &Geom) -> Option<bool> {
    use geo::Relate;
    let av = ensure_valid(a);
    let bv = ensure_valid(b);
    Some((&*av).relate(&*bv).is_equal_topo())
}
/// `ST_Touches(a, b)`.
pub fn touches(a: &Geom, b: &Geom) -> Option<bool> {
    use geo::Relate;
    let av = ensure_valid(a);
    let bv = ensure_valid(b);
    Some((&*av).relate(&*bv).is_touches())
}
/// `ST_Crosses(a, b)`.
pub fn crosses(a: &Geom, b: &Geom) -> Option<bool> {
    use geo::Relate;
    let av = ensure_valid(a);
    let bv = ensure_valid(b);
    Some((&*av).relate(&*bv).is_crosses())
}
/// `ST_Overlaps(a, b)`.
pub fn overlaps(a: &Geom, b: &Geom) -> Option<bool> {
    use geo::Relate;
    let av = ensure_valid(a);
    let bv = ensure_valid(b);
    Some((&*av).relate(&*bv).is_overlaps())
}
/// `ST_Covers(a, b)`.
pub fn covers(a: &Geom, b: &Geom) -> Option<bool> {
    use geo::Relate;
    let av = ensure_valid(a);
    let bv = ensure_valid(b);
    Some((&*av).relate(&*bv).is_covers())
}
/// `ST_CoveredBy(a, b)`.
pub fn covered_by(a: &Geom, b: &Geom) -> Option<bool> {
    use geo::Relate;
    let av = ensure_valid(a);
    let bv = ensure_valid(b);
    Some((&*av).relate(&*bv).is_coveredby())
}

// ----- structural accessors ----------------------------------------------

/// `ST_NumGeometries(geom)`.
pub fn num_geometries(g: &Geom) -> Option<i32> {
    let n: usize = match g {
        Geometry::MultiPoint(mp) => mp.0.len(),
        Geometry::MultiLineString(mls) => mls.0.len(),
        Geometry::MultiPolygon(mp) => mp.0.len(),
        Geometry::GeometryCollection(c) => c.0.len(),
        _ => 1,
    };
    n.try_into().ok()
}

/// `ST_NumInteriorRings(geom)`.
pub fn num_interior_rings(g: &Geom) -> Option<i32> {
    match g {
        Geometry::Polygon(p) => p.interiors().len().try_into().ok(),
        Geometry::MultiPolygon(mp) => mp
            .0
            .iter()
            .map(|p| p.interiors().len())
            .sum::<usize>()
            .try_into()
            .ok(),
        _ => Some(0),
    }
}

/// `ST_ExteriorRing(geom)` — polygon's exterior ring as a LineString.
pub fn exterior_ring(g: &Geom) -> Option<Geom> {
    match g {
        Geometry::Polygon(p) => Some(Geometry::LineString(p.exterior().clone())),
        _ => None,
    }
}

/// `ST_StartPoint(geom)` — first vertex of a LineString.
pub fn start_point(g: &Geom) -> Option<Geom> {
    match g {
        Geometry::LineString(ls) => ls.0.first().copied().map(|c| Geometry::Point(c.into())),
        _ => None,
    }
}

/// `ST_EndPoint(geom)` — last vertex of a LineString.
pub fn end_point(g: &Geom) -> Option<Geom> {
    match g {
        Geometry::LineString(ls) => ls.0.last().copied().map(|c| Geometry::Point(c.into())),
        _ => None,
    }
}

/// `ST_IsClosed(geom)`.
pub fn is_closed(g: &Geom) -> Option<bool> {
    Some(match g {
        Geometry::LineString(ls) => ls.0.first().is_some_and(|f| ls.0.last().is_some_and(|l| f == l)),
        Geometry::MultiLineString(mls) => mls.0.iter().all(|ls| {
            ls.0.first().is_some_and(|f| ls.0.last().is_some_and(|l| f == l))
        }),
        Geometry::Polygon(_) | Geometry::MultiPolygon(_) => true,
        _ => false,
    })
}

/// `ST_CoordDim(geom)` — this extension handles 2D WKB.
pub fn coord_dim(_g: &Geom) -> Option<i32> {
    Some(2)
}

// ----- more measurements -------------------------------------------------

/// `ST_Perimeter(geom)`.
#[allow(deprecated)]
pub fn perimeter(g: &Geom) -> Option<f64> {
    fn ring_perim(p: &geo_types::Polygon<f64>) -> f64 {
        use geo::EuclideanLength as _;
        p.exterior().euclidean_length()
            + p.interiors().iter().map(|r| r.euclidean_length()).sum::<f64>()
    }
    Some(match g {
        Geometry::Polygon(p) => ring_perim(p),
        Geometry::MultiPolygon(mp) => mp.0.iter().map(ring_perim).sum::<f64>(),
        _ => 0.0,
    })
}

/// `ST_Azimuth(a, b)` — planar bearing (radians, clockwise from +Y).
pub fn azimuth(a: &Geom, b: &Geom) -> Option<f64> {
    let pa = point_coord(a)?;
    let pb = point_coord(b)?;
    Some((pb.x - pa.x).atan2(pb.y - pa.y).rem_euclid(2.0 * std::f64::consts::PI))
}

// ----- more transforms ---------------------------------------------------

/// `ST_PointOnSurface(geom)`.
pub fn point_on_surface(g: &Geom) -> Option<Geom> {
    use geo::InteriorPoint;
    g.interior_point().map(Geometry::Point)
}

/// `ST_Rotate(geom, angle)` — rotate about centroid by `angle` radians.
pub fn rotate(g: &Geom, angle: f64) -> Option<Geom> {
    use geo::{Centroid, Rotate};
    let c = g.centroid()?;
    Some(g.rotate_around_point(angle, c))
}

/// `ST_SimplifyVW(geom, epsilon)`.
pub fn simplify_vw(g: &Geom, epsilon: f64) -> Option<Geom> {
    use geo::SimplifyVw as _;
    Some(match g {
        Geometry::LineString(ls) => Geometry::LineString(ls.simplify_vw(epsilon)),
        Geometry::MultiLineString(mls) => Geometry::MultiLineString(mls.simplify_vw(epsilon)),
        Geometry::Polygon(p) => Geometry::Polygon(p.simplify_vw(epsilon)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(mp.simplify_vw(epsilon)),
        other => other.clone(),
    })
}

/// `ST_Translate(geom, dx, dy)`.
pub fn translate(g: &Geom, dx: f64, dy: f64) -> Option<Geom> {
    use geo::Translate;
    Some(g.translate(dx, dy))
}

/// `ST_Scale(geom, xfac, yfac)`.
pub fn scale(g: &Geom, xfac: f64, yfac: f64) -> Option<Geom> {
    use geo::Scale;
    Some(g.scale_xy(xfac, yfac))
}

// ----- I/O ----------------------------------------------------------------

/// `ST_AsBinary(geom)` — ISO-WKB bytes.
pub fn as_binary(g: &Geom) -> Option<Vec<u8>> {
    crate::geometry::to_wkb(g).ok()
}

// ----- 2D / Z / M stubs (this extension handles 2D WKB only) -------------

/// `ST_Force2D(geom)` — drop Z/M (no-op here; we are already 2D).
pub fn force_2d(g: &Geom) -> Option<Geom> {
    Some(g.clone())
}
/// `ST_HasZ(geom)` — false (2D only).
pub fn has_z(_g: &Geom) -> Option<bool> {
    Some(false)
}
/// `ST_HasM(geom)` — false (2D only).
pub fn has_m(_g: &Geom) -> Option<bool> {
    Some(false)
}
/// `ST_ZMflag(geom)` — 0 (2D only).
pub fn zm_flag(_g: &Geom) -> Option<i32> {
    Some(0)
}
/// `ST_Z(geom)` — NULL (2D only).
pub fn z(_g: &Geom) -> Option<f64> {
    None
}
/// `ST_M(geom)` — NULL (2D only).
pub fn m(_g: &Geom) -> Option<f64> {
    None
}
/// `ST_IsCollection(geom)`.
pub fn is_collection(g: &Geom) -> Option<bool> {
    Some(matches!(g, Geometry::MultiPoint(_) | Geometry::MultiLineString(_) | Geometry::MultiPolygon(_) | Geometry::GeometryCollection(_)))
}

// ----- structural accessors (indexed) ------------------------------------

/// `ST_GeometryN(geom, n)` — the n-th geometry of a collection (1-indexed).
pub fn geometry_n(g: &Geom, n: i32) -> Option<Geom> {
    let i = usize::try_from(n.checked_sub(1)?).ok()?;
    match g {
        Geometry::MultiPoint(mp) => mp.0.get(i).cloned().map(Geometry::Point),
        Geometry::MultiLineString(mls) => mls.0.get(i).cloned().map(Geometry::LineString),
        Geometry::MultiPolygon(mp) => mp.0.get(i).cloned().map(Geometry::Polygon),
        Geometry::GeometryCollection(c) => c.0.get(i).cloned(),
        _ => None,
    }
}

/// `ST_PointN(geom, n)` — the n-th vertex of a LineString (1-indexed).
pub fn point_n(g: &Geom, n: i32) -> Option<Geom> {
    let i = usize::try_from(n.checked_sub(1)?).ok()?;
    match g {
        Geometry::LineString(ls) => ls.0.get(i).copied().map(|c| Geometry::Point(c.into())),
        _ => None,
    }
}

/// `ST_InteriorRingN(geom, n)` — the n-th hole of a Polygon (1-indexed).
pub fn interior_ring_n(g: &Geom, n: i32) -> Option<Geom> {
    let i = usize::try_from(n.checked_sub(1)?).ok()?;
    match g {
        Geometry::Polygon(p) => p.interiors().get(i).cloned().map(Geometry::LineString),
        _ => None,
    }
}

// ----- more editing transforms -------------------------------------------

/// `ST_Reverse(geom)` — reverse vertex order.
pub fn reverse_geom(g: &Geom) -> Option<Geom> {
    Some(match g {
        Geometry::LineString(ls) => {
            let mut pts = ls.0.clone();
            pts.reverse();
            Geometry::LineString(geo_types::LineString(pts))
        }
        Geometry::MultiLineString(mls) => {
            Geometry::MultiLineString(geo_types::MultiLineString(mls.0.iter().map(|ls| { let mut p = ls.0.clone(); p.reverse(); geo_types::LineString(p) }).collect()))
        }
        Geometry::Polygon(p) => {
            let mut ext = p.exterior().0.clone();
            ext.reverse();
            let ints: Vec<_> = p.interiors().iter().map(|r| { let mut q = r.0.clone(); q.reverse(); geo_types::LineString(q) }).collect();
            Geometry::Polygon(geo_types::Polygon::new(geo_types::LineString(ext), ints))
        }
        other => other.clone(),
    })
}

/// `ST_FlipCoordinates(geom)` — swap X and Y.
pub fn flip_coordinates(g: &Geom) -> Option<Geom> {
    use geo::MapCoords;
    Some(g.map_coords(|c| geo_types::Coord { x: c.y, y: c.x }))
}

/// `ST_RemoveRepeatedPoints(geom)` — drop consecutive duplicate vertices.
pub fn remove_repeated_points(g: &Geom) -> Option<Geom> {
    use geo::RemoveRepeatedPoints;
    Some(match g {
        Geometry::LineString(ls) => Geometry::LineString(ls.remove_repeated_points()),
        Geometry::MultiLineString(mls) => Geometry::MultiLineString(mls.remove_repeated_points()),
        Geometry::Polygon(p) => Geometry::Polygon(p.remove_repeated_points()),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(mp.remove_repeated_points()),
        other => other.clone(),
    })
}

/// `ST_LineInterpolatePoint(geom, fraction)` — point at `fraction` along a LineString.
pub fn line_interpolate_point(g: &Geom, fraction: f64) -> Option<Geom> {
    use geo::LineInterpolatePoint;
    match g {
        Geometry::LineString(ls) => ls.line_interpolate_point(fraction).map(Geometry::Point),
        _ => None,
    }
}

/// `ST_ConcaveHull(geom, concavity)` — dispatches by variant (geo's ConcaveHull
/// is not implemented for the Geometry enum directly).
pub fn concave_hull(g: &Geom, concavity: f64) -> Option<Geom> {
    use geo::ConcaveHull;
    Some(Geometry::Polygon(match g {
        Geometry::Polygon(p) => p.concave_hull(concavity),
        Geometry::MultiPolygon(mp) => mp.concave_hull(concavity),
        Geometry::MultiPoint(mp) => mp.concave_hull(concavity),
        Geometry::LineString(ls) => ls.concave_hull(concavity),
        Geometry::MultiLineString(mls) => mls.concave_hull(concavity),
        _ => return None,
    }))
}

/// `ST_OrientedEnvelope(geom)` — minimum-area rotated bounding rectangle.
pub fn oriented_envelope(g: &Geom) -> Option<Geom> {
    use geo::MinimumRotatedRect;
    Some(Geometry::Polygon(g.minimum_rotated_rect()?))
}

/// `ST_HausdorffDistance(a, b)`.
pub fn hausdorff_distance(a: &Geom, b: &Geom) -> Option<f64> {
    use geo::HausdorffDistance;
    Some(a.hausdorff_distance(b))
}

// ----- EWKT / SRID (SRID carried in text only; geometry is SRID-less) ----

/// `ST_AsEWKT(geom, srid)` — `SRID=<n>;<wkt>`.
pub fn as_ewkt(g: &Geom, srid: i32) -> Option<String> {
    Some(format!("SRID={srid};{}", as_text(g)?))
}

/// `ST_GeomFromEWKT(text)` — parse `SRID=<n>;<wkt>` (SRID discarded, 2D only).
pub fn geom_from_ewkt(s: &str) -> Option<Geom> {
    let wkt = if let Some(rest) = s.strip_prefix("SRID=") {
        rest.split_once(';').map(|(_, w)| w).unwrap_or(rest)
    } else {
        s
    };
    geom_from_text(wkt)
}

/// `ST_SetSRID(geom, srid)` — no-op tag (extension is SRID-less until PROJ/Tier 3).
pub fn set_srid(g: &Geom, _srid: i32) -> Option<Geom> {
    Some(g.clone())
}

/// `ST_SRID(geom)` — always 0 until CRS support lands.
pub fn srid(_g: &Geom) -> Option<i32> {
    Some(0)
}

// ----- more geometry processing -----------------------------------------

/// All coordinates of a geometry, flattened (manual; geo's `CoordsIter` isn't
/// implemented for the `Geometry` enum).
fn all_coords(g: &Geom) -> Vec<geo_types::Coord<f64>> {
    use geo_types::Coord;
    fn rec(g: &Geom, out: &mut Vec<Coord<f64>>) {
        match g {
            Geometry::Point(p) => out.push(p.0),
            Geometry::Line(l) => { out.push(l.start); out.push(l.end); }
            Geometry::LineString(ls) => out.extend(ls.0.iter().copied()),
            Geometry::Polygon(p) => {
                out.extend(p.exterior().0.iter().copied());
                for r in p.interiors() { out.extend(r.0.iter().copied()); }
            }
            Geometry::MultiPoint(mp) => out.extend(mp.0.iter().map(|p| p.0)),
            Geometry::MultiLineString(mls) => for ls in &mls.0 { out.extend(ls.0.iter().copied()) },
            Geometry::MultiPolygon(mp) => for p in &mp.0 {
                out.extend(p.exterior().0.iter().copied());
                for r in p.interiors() { out.extend(r.0.iter().copied()); }
            },
            Geometry::GeometryCollection(c) => for item in &c.0 { rec(item, out) },
            Geometry::Rect(r) => { out.push(r.min()); out.push(r.max()); }
            Geometry::Triangle(t) => out.extend(t.to_array().iter()),
        }
    }
    let mut out = Vec::new();
    rec(g, &mut out);
    out
}

/// `ST_Points(geom)` — every vertex as a MultiPoint.
pub fn points(g: &Geom) -> Option<Geom> {
    let pts: Vec<geo_types::Point<f64>> = all_coords(g).into_iter().map(geo_types::Point::from).collect();
    Some(Geometry::MultiPoint(geo_types::MultiPoint(pts)))
}

/// `ST_LineLocatePoint(line, point)` — fraction of `line` at the projection of `point`.
pub fn line_locate_point(g: &Geom, p: &Geom) -> Option<f64> {
    use geo::LineLocatePoint;
    match (g, p) {
        (Geometry::LineString(ls), Geometry::Point(pt)) => Some(ls.line_locate_point(pt)?),
        _ => None,
    }
}

/// `ST_FrechetDistance(a, b)` — discrete Fréchet distance of two LineStrings.
pub fn frechet_distance(a: &Geom, b: &Geom) -> Option<f64> {
    use geo::FrechetDistance;
    match (a, b) {
        (Geometry::LineString(la), Geometry::LineString(lb)) => Some(la.frechet_distance(lb)),
        _ => None,
    }
}

/// `ST_AsGeoJSON(geom)` — GeoJSON serialization of the geometry.
pub fn as_geojson(g: &Geom) -> Option<String> {
    // Manual GeoJSON serialization (avoids geojson crate API churn).
    let coord = |c: &geo_types::Coord<f64>| format!("[{},{}]", c.x, c.y);
    let ring = |ls: &geo_types::LineString<f64>| {
        format!("[{}]", ls.0.iter().map(coord).collect::<Vec<_>>().join(","))
    };
    let json = match g {
        Geometry::Point(p) => format!(r#"{{"type":"Point","coordinates":[{},{}]}}"#, p.x(), p.y()),
        Geometry::MultiPoint(mp) => format!(r#"{{"type":"MultiPoint","coordinates":[{}]}}"#,
            mp.0.iter().map(|p| format!("[{},{}]", p.x(), p.y())).collect::<Vec<_>>().join(",")),
        Geometry::LineString(ls) => format!(r#"{{"type":"LineString","coordinates":{}}}"#, ring(ls)),
        Geometry::MultiLineString(mls) => format!(r#"{{"type":"MultiLineString","coordinates":[{}]}}"#,
            mls.0.iter().map(ring).collect::<Vec<_>>().join(",")),
        Geometry::Polygon(p) => {
            let rings: Vec<String> = std::iter::once(p.exterior()).chain(p.interiors().iter()).map(ring).collect();
            format!(r#"{{"type":"Polygon","coordinates":[{}]}}"#, rings.join(","))
        }
        Geometry::MultiPolygon(mp) => {
            let polys: Vec<String> = mp.0.iter().map(|p| {
                let rings: Vec<String> = std::iter::once(p.exterior()).chain(p.interiors().iter()).map(ring).collect();
                format!("[{}]", rings.join(","))
            }).collect();
            format!(r#"{{"type":"MultiPolygon","coordinates":[{}]}}"#, polys.join(","))
        }
        _ => r#"{"type":"GeometryCollection","geometries":[]}"#.to_string(),
    };
    Some(json)
}

/// `ST_Project(geom, distance, azimuth)` — geographic destination point from a
/// point, distance in metres, and azimuth in degrees (clockwise from north).
#[allow(deprecated)]
pub fn project(g: &Geom, distance: f64, azimuth: f64) -> Option<Geom> {
    use geo::HaversineDestination;
    let p = match g { Geometry::Point(p) => *p, _ => return None };
    Some(Geometry::Point(p.haversine_destination(distance, azimuth)))
}

/// `ST_ForcePolygonCW(geom)` — force exterior ring CW, interiors CCW.
pub fn force_polygon_cw(g: &Geom) -> Option<Geom> {
    use geo::Orient;
    Some(match g {
        Geometry::Polygon(p) => Geometry::Polygon(p.orient(geo::algorithm::orient::Direction::Reversed)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(mp.orient(geo::algorithm::orient::Direction::Reversed)),
        other => other.clone(),
    })
}

/// `ST_SnapToGrid(geom, size)` — round every coordinate to the nearest `size` grid.
pub fn snap_to_grid(g: &Geom, size: f64) -> Option<Geom> {
    if size <= 0.0 { return Some(g.clone()); }
    let round = |v: f64| (v / size).round() * size;
    use geo::MapCoords;
    Some(g.map_coords(|c| geo_types::Coord { x: round(c.x), y: round(c.y) }))
}

/// `ST_Boundary(geom)` — topological boundary: polygon → MultiLineString of rings,
/// LineString → MultiPoint of endpoints (if open) or empty (if closed).
pub fn boundary(g: &Geom) -> Option<Geom> {
    Some(match g {
        Geometry::Polygon(p) => {
            let lines: Vec<geo_types::LineString> = std::iter::once(p.exterior().clone())
                .chain(p.interiors().iter().cloned())
                .collect();
            Geometry::MultiLineString(geo_types::MultiLineString(lines))
        }
        Geometry::MultiPolygon(mp) => {
            let lines: Vec<geo_types::LineString> = mp.0.iter().flat_map(|p| {
                std::iter::once(p.exterior().clone()).chain(p.interiors().iter().cloned())
            }).collect();
            Geometry::MultiLineString(geo_types::MultiLineString(lines))
        }
        Geometry::LineString(ls) => {
            if ls.0.len() >= 2 && ls.0.first() != ls.0.last() {
                Geometry::MultiPoint(geo_types::MultiPoint(vec![
                    geo_types::Point::from(ls.0[0]),
                    geo_types::Point::from(*ls.0.last().unwrap()),
                ]))
            } else {
                Geometry::GeometryCollection(geo_types::GeometryCollection(vec![]))
            }
        }
        _ => Geometry::GeometryCollection(geo_types::GeometryCollection(vec![])),
    })
}

/// `ST_IsRing(geom)` — true for a closed, simple LineString.  (Approximation:
/// checks `is_closed`; full simplicity needs geo's `is_simple` which is not on the
/// `Geometry` enum.)
pub fn is_ring(g: &Geom) -> Option<bool> {
    Some(matches!(g, Geometry::LineString(ls) if ls.0.len() >= 4 && ls.0.first().is_some_and(|f| ls.0.last().is_some_and(|l| f == l))))
}

/// `ST_ClosestPoint(geom, point)` — nearest point on `geom` to `point`.
pub fn closest_point(g: &Geom, p: &Geom) -> Option<Geom> {
    use geo::ClosestPoint;
    let pt = geo_types::Point::from(point_coord(p)?);
    let single = |c: geo::Closest<f64>| match c {
        geo::Closest::SinglePoint(p) | geo::Closest::Intersection(p) => Some(Geometry::Point(p)),
        _ => None,
    };
    match g {
        Geometry::LineString(ls) => single(ls.closest_point(&pt)),
        Geometry::Polygon(poly) => single(poly.closest_point(&pt)),
        Geometry::Line(l) => single(l.closest_point(&pt)),
        _ => None,
    }
}

/// `ST_DelaunayTriangles(geom)` — Delaunay triangulation of the vertex set
/// (via the `delaunator` crate).
pub fn delaunay_triangles(g: &Geom) -> Option<Geom> {
    let coords = all_coords(g);
    let pts: Vec<delaunator::Point> = coords.iter().map(|c| delaunator::Point { x: c.x, y: c.y }).collect();
    if pts.len() < 3 {
        return None;
    }
    let tri = delaunator::triangulate(&pts);
    let t = &tri.triangles;
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 < t.len() {
        let pa = &pts[t[i]];
        let pb = &pts[t[i + 1]];
        let pc = &pts[t[i + 2]];
        out.push(Geometry::Triangle(geo_types::Triangle(
            geo_types::Coord { x: pa.x, y: pa.y },
            geo_types::Coord { x: pb.x, y: pb.y },
            geo_types::Coord { x: pc.x, y: pc.y },
        )));
        i += 3;
    }
    Some(Geometry::GeometryCollection(geo_types::GeometryCollection(out)))
}

/// Circumcenter of three points (NaN-safe for collinear input).
fn circumcenter(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64) -> (f64, f64) {
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if d.abs() < 1e-20 {
        return (ax, ay);
    }
    let a2 = ax * ax + ay * ay;
    let b2 = bx * bx + by * by;
    let c2 = cx * cx + cy * cy;
    let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
    let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
    (ux, uy)
}

/// `ST_VoronoiLines(geom)` — the interior Voronoi diagram edges, derived as the
/// dual of the Delaunay triangulation (connect circumcenters of adjacent
/// triangles). Boundary edges (no adjacent triangle) are omitted. Returns a
/// GeometryCollection of 2-point LineStrings.
pub fn voronoi_lines(g: &Geom) -> Option<Geom> {
    let coords = all_coords(g);
    let pts: Vec<delaunator::Point> = coords.iter().map(|c| delaunator::Point { x: c.x, y: c.y }).collect();
    if pts.len() < 3 {
        return None;
    }
    let tri = delaunator::triangulate(&pts);
    let t = &tri.triangles;
    // Circumcenter of each triangle.
    let ccs: Vec<(f64, f64)> = (0..t.len())
        .step_by(3)
        .map(|i| {
            let a = &pts[t[i]];
            let b = &pts[t[i + 1]];
            let c = &pts[t[i + 2]];
            circumcenter(a.x, a.y, b.x, b.y, c.x, c.y)
        })
        .collect();
    // Connect circumcenters of triangles sharing an edge (use delaunator halfedges).
    let mut lines = Vec::new();
    for e in 0..t.len() {
        let opp = tri.halfedges[e];
        if opp != delaunator::EMPTY && e < opp {
            let t1 = e / 3;
            let t2 = opp / 3;
            let (x1, y1) = ccs[t1];
            let (x2, y2) = ccs[t2];
            lines.push(Geometry::LineString(geo_types::LineString::from(vec![
                (x1, y1),
                (x2, y2),
            ])));
        }
    }
    Some(Geometry::GeometryCollection(geo_types::GeometryCollection(lines)))
}

// ----- Geography (geodesic) variants (Tier 2) ---------------------------
// Coordinates interpreted as lon/lat. Distances/length in metres, area in m².
// Point-to-point / per-type only (documented); full geometry-geometry geodesic
// distance would need closest-point-on-sphere work.

/// `ST_DistanceSphere(a, b)` — great-circle distance between two points (metres).
#[allow(deprecated)]
pub fn distance_sphere(a: &Geom, b: &Geom) -> Option<f64> {
    use geo::HaversineDistance;
    let pa = geo_types::Point::from(point_coord(a)?);
    let pb = geo_types::Point::from(point_coord(b)?);
    Some(pa.haversine_distance(&pb))
}

/// `ST_DWithinSphere(a, b, metres)`.
#[allow(deprecated)]
pub fn dwithin_sphere(a: &Geom, b: &Geom, metres: f64) -> Option<bool> {
    Some(distance_sphere(a, b)? <= metres)
}

/// `ST_LengthSphere(geom)` — great-circle length of a (multi)linestring (metres).
#[allow(deprecated)]
pub fn length_sphere(g: &Geom) -> Option<f64> {
    use geo::HaversineLength;
    Some(match g {
        Geometry::LineString(ls) => ls.haversine_length(),
        Geometry::MultiLineString(mls) => mls.haversine_length(),
        _ => 0.0,
    })
}

/// `ST_AreaSphere(geom)` — geodesic area of a (multi)polygon (m²).
pub fn area_sphere(g: &Geom) -> Option<f64> {
    use geo::ChamberlainDuquetteArea;
    Some(match g {
        Geometry::Polygon(p) => p.chamberlain_duquette_unsigned_area(),
        Geometry::MultiPolygon(mp) => mp.chamberlain_duquette_unsigned_area(),
        _ => 0.0,
    })
}

// ----- CRS reprojection via PROJ (Tier 3a) ------------------------------
// Requires libproj at runtime. A thread-local cache of `proj::Proj` objects
// (keyed by (from_epsg, to_epsg)) avoids re-parsing the CRS per row, which is
// the expensive part — the per-coordinate transform is then cheap.

thread_local! {
    static PROJ_CACHE: std::cell::RefCell<std::collections::HashMap<(i32, i32), Option<proj::Proj>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// `ST_Transform(geom, from_srid, to_srid)` — reproject between EPSG codes.
pub fn transform(g: &Geom, from_srid: i32, to_srid: i32) -> Option<Geom> {
    use geo::MapCoords;
    PROJ_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let proj = cache
            .entry((from_srid, to_srid))
            .or_insert_with(|| {
                proj::Proj::new_known_crs(
                    &format!("EPSG:{from_srid}"),
                    &format!("EPSG:{to_srid}"),
                    None,
                )
                .ok()
            })
            .as_ref()?;
        Some(g.map_coords(|c| {
            proj.convert((c.x, c.y))
                .map(|(x, y)| geo_types::Coord { x, y })
                .unwrap_or(c)
        }))
    })
}

// =====================================================================
// Tier 1 / 1b parity batch
//
// These functions close the geometry-level gaps flagged in ROADMAP.md:
// affine & segmentize transforms, line substring / merge, collection
// editing, force-orientation/normalization, distance-based measurements,
// polygon triangulation, structural accessors, type predicates, and
// EWKB/hex I/O. Each follows the same `fn(&Geom[, &Geom][, scalar])* ->
// Option<...>` shape the generic executors expect.
// =====================================================================

// ----- editing transforms -----------------------------------------------

/// `ST_Affine(geom, a, b, d, e, xoff, yoff)` — 2D affine transform:
/// `x' = a*x + d*y + xoff`, `y' = b*x + e*y + yoff`.
///
/// Parameter names/order match PostGIS's 2D `ST_Affine` overload
/// (`ST_Affine(geom, a, b, d, e, xoff, yoff)`).
pub fn affine(g: &Geom, a: f64, b: f64, d: f64, e: f64, xoff: f64, yoff: f64) -> Option<Geom> {
    use geo::MapCoords;
    Some(g.map_coords(|c| geo_types::Coord {
        x: a * c.x + d * c.y + xoff,
        y: b * c.x + e * c.y + yoff,
    }))
}

/// `ST_Segmentize(geom, max_len)` — split any segment longer than `max_len` into
/// equal-length sub-segments so that no segment exceeds `max_len`. No-op when
/// `max_len <= 0`. Operates on every LineString / ring in the geometry.
pub fn segmentize(g: &Geom, max_len: f64) -> Option<Geom> {
    if max_len <= 0.0 {
        return Some(g.clone());
    }
    let seg = |ls: &geo_types::LineString<f64>| -> geo_types::LineString<f64> {
        geo_types::LineString(segmentize_ring(&ls.0, max_len))
    };
    Some(match g {
        Geometry::LineString(ls) => Geometry::LineString(seg(ls)),
        Geometry::MultiLineString(mls) => Geometry::MultiLineString(geo_types::MultiLineString(
            mls.0.iter().map(seg).collect(),
        )),
        Geometry::Polygon(p) => Geometry::Polygon(segmentize_polygon(p, max_len)),
        Geometry::MultiPolygon(mp) => Geometry::MultiPolygon(geo_types::MultiPolygon(
            mp.0.iter().map(|p| segmentize_polygon(p, max_len)).collect(),
        )),
        Geometry::GeometryCollection(c) => Geometry::GeometryCollection(geo_types::GeometryCollection(
            c.0.iter().filter_map(|item| segmentize(item, max_len)).collect(),
        )),
        other => other.clone(),
    })
}

fn segmentize_polygon(p: &geo_types::Polygon<f64>, max_len: f64) -> geo_types::Polygon<f64> {
    let ext = geo_types::LineString(segmentize_ring(&p.exterior().0, max_len));
    let ints: Vec<_> = p
        .interiors()
        .iter()
        .map(|r| geo_types::LineString(segmentize_ring(&r.0, max_len)))
        .collect();
    geo_types::Polygon::new(ext, ints)
}

/// Split a coordinate ring's segments so none exceeds `max_len`. Rings keep
/// their closed-ness (the last point repeats the first when the input did).
fn segmentize_ring(coords: &[geo_types::Coord<f64>], max_len: f64) -> Vec<geo_types::Coord<f64>> {
    if coords.len() < 2 {
        return coords.to_vec();
    }
    let mut out = Vec::with_capacity(coords.len() * 2);
    out.push(coords[0]);
    for w in coords.windows(2) {
        let (a, b) = (w[0], w[1]);
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = dx.hypot(dy);
        if len <= max_len {
            out.push(b);
            continue;
        }
        let n = (len / max_len).ceil() as usize;
        for i in 1..=n {
            let t = i as f64 / n as f64;
            out.push(geo_types::Coord { x: a.x + t * dx, y: a.y + t * dy });
        }
    }
    out
}

/// `ST_LineSubstring(geom, start_frac, end_frac)` — the substring of a
/// LineString between two fractional lengths. Fractions are clamped to [0,1];
/// returns NULL when `start_frac >= end_frac` or the input is not a LineString.
pub fn line_substring(g: &Geom, start_frac: f64, end_frac: f64) -> Option<Geom> {
    let ls = match g {
        Geometry::LineString(ls) => ls,
        _ => return None,
    };
    let d0 = start_frac.clamp(0.0, 1.0);
    let d1 = end_frac.clamp(0.0, 1.0);
    if d0 >= d1 {
        return None;
    }
    let total: f64 = ls.lines().map(|l| line_len(&l)).sum();
    if total <= 0.0 {
        return None;
    }
    let sub = substring_line(ls, d0 * total, d1 * total, total);
    Some(Geometry::LineString(sub))
}

/// Walk a LineString, emitting the portion between absolute lengths `from` and
/// `to` (measured along the line from its start). The two endpoints are
/// interpolated onto their containing segments.
fn substring_line(
    ls: &geo_types::LineString<f64>,
    from: f64,
    to: f64,
    total: f64,
) -> geo_types::LineString<f64> {
    let mut out: Vec<geo_types::Coord<f64>> = Vec::new();
    let mut acc = 0.0; // length consumed up to the start of the current segment
    let mut pushed_start = false;
    for line in ls.lines() {
        let seg_len = line_len(&line);
        let seg_end = acc + seg_len;
        if !pushed_start && seg_end >= from {
            let t = if seg_len > 0.0 { (from - acc) / seg_len } else { 0.0 };
            let t = t.clamp(0.0, 1.0);
            out.push(geo_types::Coord {
                x: line.start.x + t * (line.end.x - line.start.x),
                y: line.start.y + t * (line.end.y - line.start.y),
            });
            pushed_start = true;
        }
        if pushed_start {
            if seg_end < to {
                out.push(line.end);
            } else {
                let t = if seg_len > 0.0 { (to - acc) / seg_len } else { 0.0 };
                let t = t.clamp(0.0, 1.0);
                out.push(geo_types::Coord {
                    x: line.start.x + t * (line.end.x - line.start.x),
                    y: line.start.y + t * (line.end.y - line.start.y),
                });
                break;
            }
        }
        acc = seg_end;
    }
    let _ = total; // total carried only for symmetry / debugging.
    geo_types::LineString(out)
}

/// `ST_LineMerge(geom)` — join a MultiLineString into the fewest LineStrings
/// possible by chaining segments whose endpoints touch. Non-LinearString input
/// is returned unchanged.
pub fn line_merge(g: &Geom) -> Option<Geom> {
    let lines: Vec<geo_types::LineString<f64>> = match g {
        Geometry::LineString(_) => return Some(g.clone()),
        Geometry::MultiLineString(mls) => mls.0.clone(),
        Geometry::GeometryCollection(c) => {
            let mut out = Vec::new();
            for item in &c.0 {
                match item {
                    Geometry::LineString(ls) => out.push(ls.clone()),
                    Geometry::MultiLineString(mls) => out.extend(mls.0.iter().cloned()),
                    _ => {}
                }
            }
            out
        }
        _ => return Some(g.clone()),
    };
    let merged = merge_linestrings(lines);
    Some(Geometry::MultiLineString(geo_types::MultiLineString(merged)))
}

/// Greedy endpoint-chaining merge: repeatedly take an unused LineString, extend
/// its tail while another unused segment shares the endpoint (possibly
/// reversed), then extend its head the same way. O(n^2) but n is small.
fn merge_linestrings(lines: Vec<geo_types::LineString<f64>>) -> Vec<geo_types::LineString<f64>> {
    if lines.is_empty() {
        return lines;
    }
    let buffers: Vec<Vec<geo_types::Coord<f64>>> = lines.iter().map(|ls| ls.0.clone()).collect();
    let mut used = vec![false; buffers.len()];
    let mut out: Vec<geo_types::LineString<f64>> = Vec::new();
    for start in 0..buffers.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut cur = buffers[start].clone();
        loop {
            // Try to extend the tail with another unused chain.
            let tail = *cur.last().unwrap();
            let mut found = None;
            for (i, cand) in buffers.iter().enumerate() {
                if used[i] || cand.is_empty() {
                    continue;
                }
                let cfirst = cand[0];
                let clast = *cand.last().unwrap();
                let touches_first =
                    (cfirst.x - tail.x).abs() < 1e-12 && (cfirst.y - tail.y).abs() < 1e-12;
                let touches_last =
                    (clast.x - tail.x).abs() < 1e-12 && (clast.y - tail.y).abs() < 1e-12;
                if touches_first {
                    found = Some((i, false));
                    break;
                }
                if touches_last {
                    found = Some((i, true));
                    break;
                }
            }
            match found {
                Some((i, reverse)) => {
                    used[i] = true;
                    let mut ext = buffers[i].clone();
                    if reverse {
                        ext.reverse();
                    }
                    // Drop the shared junction vertex so it isn't duplicated.
                    cur.extend(ext.into_iter().skip(1));
                }
                None => break,
            }
        }
        out.push(geo_types::LineString(cur));
    }
    out
}

/// `ST_CollectionExtract(geom, dim)` — extract the geometries of one dimension
/// from a (multi/collection) geometry: `1`→MultiPoint, `2`→MultiLineString,
/// `3`→MultiPolygon. Non-matching members are dropped. Returns an empty
/// GEOMETRYCOLLECTION for an unknown `dim` or no matches.
pub fn collection_extract(g: &Geom, dim: i32) -> Option<Geom> {
    let (mut pts, mut lns, mut pls): (
        Vec<geo_types::Point<f64>>,
        Vec<geo_types::LineString<f64>>,
        Vec<geo_types::Polygon<f64>>,
    ) = (Vec::new(), Vec::new(), Vec::new());
    fn rec(
        g: &Geom,
        pts: &mut Vec<geo_types::Point<f64>>,
        lns: &mut Vec<geo_types::LineString<f64>>,
        pls: &mut Vec<geo_types::Polygon<f64>>,
    ) {
        match g {
            Geometry::Point(p) => pts.push(*p),
            Geometry::MultiPoint(mp) => pts.extend(mp.0.iter().copied()),
            Geometry::LineString(ls) => lns.push(ls.clone()),
            Geometry::MultiLineString(mls) => lns.extend(mls.0.iter().cloned()),
            Geometry::Polygon(p) => pls.push(p.clone()),
            Geometry::MultiPolygon(mp) => pls.extend(mp.0.iter().cloned()),
            Geometry::GeometryCollection(c) => {
                for item in &c.0 {
                    rec(item, pts, lns, pls);
                }
            }
            _ => {}
        }
    }
    rec(g, &mut pts, &mut lns, &mut pls);
    Some(match dim {
        1 => Geometry::MultiPoint(geo_types::MultiPoint(pts)),
        2 => Geometry::MultiLineString(geo_types::MultiLineString(lns)),
        3 => Geometry::MultiPolygon(geo_types::MultiPolygon(pls)),
        _ => Geometry::GeometryCollection(geo_types::GeometryCollection(vec![])),
    })
}

/// `ST_ForcePolygonCCW(geom)` — exterior rings CCW, interior rings CW.
pub fn force_polygon_ccw(g: &Geom) -> Option<Geom> {
    use geo::Orient;
    Some(match g {
        Geometry::Polygon(p) => Geometry::Polygon(p.orient(geo::algorithm::orient::Direction::Default)),
        Geometry::MultiPolygon(mp) => {
            Geometry::MultiPolygon(mp.orient(geo::algorithm::orient::Direction::Default))
        }
        other => other.clone(),
    })
}

/// `ST_ForceRHR(geom)` — force right-hand-rule orientation (exterior CW so the
/// filled area is to the right). Alias of `force_polygon_cw`.
pub fn force_rhr(g: &Geom) -> Option<Geom> {
    force_polygon_cw(g)
}

/// `ST_ForceCollection(geom)` — wrap any geometry in a GeometryCollection.
pub fn force_collection(g: &Geom) -> Option<Geom> {
    match g {
        Geometry::GeometryCollection(_) => Some(g.clone()),
        other => Some(Geometry::GeometryCollection(geo_types::GeometryCollection(
            vec![other.clone()],
        ))),
    }
}

/// `ST_Multi(geom)` — promote a single geometry to its Multi form. Multi and
/// collection inputs pass through unchanged.
pub fn multi(g: &Geom) -> Option<Geom> {
    Some(match g {
        Geometry::Point(p) => Geometry::MultiPoint(geo_types::MultiPoint(vec![*p])),
        Geometry::LineString(ls) => {
            Geometry::MultiLineString(geo_types::MultiLineString(vec![ls.clone()]))
        }
        Geometry::Polygon(p) => Geometry::MultiPolygon(geo_types::MultiPolygon(vec![p.clone()])),
        other => other.clone(),
    })
}

/// `ST_Normalize(geom)` — arrange geometry canonically: rings rotated to start
/// at their lexicographically smallest coordinate, interior rings sorted,
/// multi-members sorted. Lets topologically-equal geometries compare equal.
pub fn normalize(g: &Geom) -> Option<Geom> {
    Some(match g {
        Geometry::LineString(ls) => Geometry::LineString(normalize_ring(ls.clone())),
        Geometry::Polygon(p) => Geometry::Polygon(normalize_polygon(p)),
        Geometry::MultiLineString(mls) => {
            let mut v: Vec<_> = mls.0.iter().cloned().map(normalize_ring).collect();
            v.sort_by(coords_cmp);
            Geometry::MultiLineString(geo_types::MultiLineString(v))
        }
        Geometry::MultiPolygon(mp) => {
            let mut v: Vec<_> = mp.0.iter().map(|p| normalize_polygon(p)).collect();
            v.sort_by(poly_cmp);
            Geometry::MultiPolygon(geo_types::MultiPolygon(v))
        }
        Geometry::MultiPoint(mp) => {
            let mut v = mp.0.clone();
            v.sort_by(|a, b| {
                a.x().partial_cmp(&b.x())
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.y().partial_cmp(&b.y()).unwrap_or(std::cmp::Ordering::Equal))
            });
            Geometry::MultiPoint(geo_types::MultiPoint(v))
        }
        other => other.clone(),
    })
}

fn normalize_ring(mut ls: geo_types::LineString<f64>) -> geo_types::LineString<f64> {
    if ls.0.len() >= 2 && ls.0.first() == ls.0.last() {
        // Closed ring: drop the repeated closing vertex, rotate, re-close.
        let closing = ls.0.pop();
        let n = ls.0.len();
        if n > 0 {
            let mut start = 0;
            for i in 1..n {
                if coords_lt(&ls.0[i], &ls.0[start]) {
                    start = i;
                }
            }
            ls.0.rotate_left(start);
        }
        if let Some(c) = closing {
            ls.0.push(c);
        }
    }
    ls
}
fn normalize_polygon(p: &geo_types::Polygon<f64>) -> geo_types::Polygon<f64> {
    let ext = normalize_ring(p.exterior().clone());
    let mut ints: Vec<_> = p.interiors().iter().cloned().map(normalize_ring).collect();
    ints.sort_by(coords_cmp);
    geo_types::Polygon::new(ext, ints)
}
/// Lexicographic ordering of two LineStrings' coordinate sequences. (`Coord`
/// is not `Ord` because `f64` isn't, so we compare component-wise via
/// `partial_cmp`, treating NaN as less-than-everything.)
fn coords_cmp(
    a: &geo_types::LineString<f64>,
    b: &geo_types::LineString<f64>,
) -> std::cmp::Ordering {
    coord_slice_cmp(&a.0, &b.0)
}
fn poly_cmp(a: &geo_types::Polygon<f64>, b: &geo_types::Polygon<f64>) -> std::cmp::Ordering {
    coord_slice_cmp(&a.exterior().0, &b.exterior().0)
}
fn coord_slice_cmp(
    a: &[geo_types::Coord<f64>],
    b: &[geo_types::Coord<f64>],
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let len = a.len().min(b.len());
    for i in 0..len {
        match a[i]
            .x
            .partial_cmp(&b[i].x)
            .unwrap_or(Ordering::Equal)
            .then(a[i].y.partial_cmp(&b[i].y).unwrap_or(Ordering::Equal))
        {
            Ordering::Equal => continue,
            ord => return ord,
        }
    }
    a.len().cmp(&b.len())
}
fn coords_lt(a: &geo_types::Coord<f64>, b: &geo_types::Coord<f64>) -> bool {
    a.x < b.x || (a.x == b.x && a.y < b.y)
}

// ----- measurements & processing (Tier 1b) ------------------------------

/// Length of a single 2D segment.
fn line_len(l: &geo_types::Line<f64>) -> f64 {
    let dx = l.end.x - l.start.x;
    let dy = l.end.y - l.start.y;
    dx.hypot(dy)
}

/// All vertices of a geometry, flattened (same as `all_coords` but kept
/// explicit here for the distance-based measurements).
fn vertices(g: &Geom) -> Vec<geo_types::Coord<f64>> {
    all_coords(g)
}

/// `ST_MaxDistance(a, b)` — greatest distance between any vertex of `a` and any
/// vertex of `b`. Returns 0.0 when either side has no vertices.
pub fn max_distance(a: &Geom, b: &Geom) -> Option<f64> {
    let va = vertices(a);
    let vb = vertices(b);
    if va.is_empty() || vb.is_empty() {
        return Some(0.0);
    }
    let mut best = 0.0_f64;
    for p in &va {
        for q in &vb {
            let d = (p.x - q.x).hypot(p.y - q.y);
            if d > best {
                best = d;
            }
        }
    }
    Some(best)
}

/// `ST_LongestLine(a, b)` — the 2-point LineString joining the pair of vertices
/// (one from each geometry) that realize `ST_MaxDistance`.
pub fn longest_line(a: &Geom, b: &Geom) -> Option<Geom> {
    let va = vertices(a);
    let vb = vertices(b);
    if va.is_empty() || vb.is_empty() {
        return None;
    }
    let mut best = 0.0_f64;
    let mut bestp = (va[0], vb[0]);
    for p in &va {
        for q in &vb {
            let d = (p.x - q.x).hypot(p.y - q.y);
            if d > best {
                best = d;
                bestp = (*p, *q);
            }
        }
    }
    Some(Geometry::LineString(geo_types::LineString::from(vec![
        (bestp.0.x, bestp.0.y),
        (bestp.1.x, bestp.1.y),
    ])))
}

/// `ST_ShortestLine(a, b)` — the 2-point LineString joining the closest points
/// of `a` and `b`, considering vertex-vertex, vertex-segment, and
/// segment-vertex distances (so the endpoint may lie mid-segment).
pub fn shortest_line(a: &Geom, b: &Geom) -> Option<Geom> {
    let va = vertices(a);
    let vb = vertices(b);
    let sa = segments(a);
    let sb = segments(b);
    if va.is_empty() && sa.is_empty() || vb.is_empty() && sb.is_empty() {
        return None;
    }
    let mut best = f64::INFINITY;
    let zero = geo_types::Coord { x: 0.0, y: 0.0 };
    let mut bestpair = (
        va.first().copied().unwrap_or(zero),
        vb.first().copied().unwrap_or(zero),
    );
    let consider = |p: geo_types::Coord<f64>, q: geo_types::Coord<f64>,
                    best: &mut f64,
                    bestpair: &mut (geo_types::Coord<f64>, geo_types::Coord<f64>)| {
        let d = (p.x - q.x).hypot(p.y - q.y);
        if d < *best {
            *best = d;
            *bestpair = (p, q);
        }
    };
    // vertex(a) vs vertex(b)
    for p in &va {
        for q in &vb {
            consider(*p, *q, &mut best, &mut bestpair);
        }
    }
    // vertex(a) vs segment(b), and segment(a) vs vertex(b)
    for p in &va {
        for s in &sb {
            let q = closest_on_segment(p, s);
            consider(*p, q, &mut best, &mut bestpair);
        }
    }
    for s in &sa {
        for q in &vb {
            let p = closest_on_segment(q, s);
            consider(p, *q, &mut best, &mut bestpair);
        }
    }
    Some(Geometry::LineString(geo_types::LineString::from(vec![
        (bestpair.0.x, bestpair.0.y),
        (bestpair.1.x, bestpair.1.y),
    ])))
}

/// All (directed) segments of a geometry as start/end coordinate pairs.
fn segments(g: &Geom) -> Vec<(geo_types::Coord<f64>, geo_types::Coord<f64>)> {
    let mut out = Vec::new();
    let from_ring = |coords: &[geo_types::Coord<f64>], out: &mut Vec<_>| {
        for w in coords.windows(2) {
            out.push((w[0], w[1]));
        }
    };
    match g {
        Geometry::Line(l) => out.push((l.start, l.end)),
        Geometry::LineString(ls) => from_ring(&ls.0, &mut out),
        Geometry::MultiLineString(mls) => for ls in &mls.0 {
            from_ring(&ls.0, &mut out)
        },
        Geometry::Polygon(p) => {
            from_ring(&p.exterior().0, &mut out);
            for r in p.interiors() {
                from_ring(&r.0, &mut out);
            }
        }
        Geometry::MultiPolygon(mp) => for p in &mp.0 {
            from_ring(&p.exterior().0, &mut out);
            for r in p.interiors() {
                from_ring(&r.0, &mut out);
            }
        },
        Geometry::GeometryCollection(c) => for item in &c.0 {
            out.extend(segments(item));
        },
        _ => {}
    }
    out
}

/// Closest point on segment `s` to point `p`.
fn closest_on_segment(
    p: &geo_types::Coord<f64>,
    s: &(geo_types::Coord<f64>, geo_types::Coord<f64>),
) -> geo_types::Coord<f64> {
    let (a, b) = s;
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return *a;
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    geo_types::Coord {
        x: a.x + t * dx,
        y: a.y + t * dy,
    }
}

/// `ST_TriangulatePolygon(geom)` — a triangulation of a polygon's area. We
/// Delaunay-triangulate the vertex set (via `delaunator`) and keep the triangles
/// whose centroid is inside the polygon (inside the exterior ring and outside
/// every hole). For convex polygons this is exact; for concile / holed input it
/// is a Delaunay-based approximation of constrained triangulation, documented
/// in ROADMAP.md.
pub fn triangulate_polygon(g: &Geom) -> Option<Geom> {
    let polys: Vec<&geo_types::Polygon<f64>> = match g {
        Geometry::Polygon(p) => vec![p],
        Geometry::MultiPolygon(mp) => mp.0.iter().collect(),
        _ => return None,
    };
    if polys.is_empty() {
        return None;
    }
    let mut out: Vec<Geometry> = Vec::new();
    for poly in &polys {
        for tri in delaunay_interior(*poly) {
            out.push(Geometry::Polygon(tri));
        }
    }
    Some(Geometry::GeometryCollection(geo_types::GeometryCollection(out)))
}

/// Delaunay triangulation of a single polygon's vertices, filtered to triangles
/// whose centroid lies inside the polygon.
fn delaunay_interior(poly: &geo_types::Polygon<f64>) -> Vec<geo_types::Polygon<f64>> {
    let mut coords = Vec::new();
    coords.extend(poly.exterior().0.iter().copied());
    for r in poly.interiors() {
        coords.extend(r.0.iter().copied());
    }
    // Drop exact duplicate vertices (polygon rings repeat their closing vertex,
    // and `delaunator` misbehaves on coincident input points).
    if coords.len() >= 2 {
        let last = *coords.last().unwrap();
        if coords[0].x == last.x && coords[0].y == last.y {
            coords.pop();
        }
    }
    coords.dedup_by(|a, b| a.x == b.x && a.y == b.y);
    if coords.len() < 3 {
        return Vec::new();
    }
    let pts: Vec<delaunator::Point> = coords
        .iter()
        .map(|c| delaunator::Point { x: c.x, y: c.y })
        .collect();
    let tri = delaunator::triangulate(&pts);
    let t = &tri.triangles;
    let to_tri = |i: usize| {
        let a = coords[t[i]];
        let b = coords[t[i + 1]];
        let c = coords[t[i + 2]];
        // Order so the triangle is closed and CCW.
        let ring = geo_types::LineString::from(vec![
            (a.x, a.y),
            (b.x, b.y),
            (c.x, c.y),
            (a.x, a.y),
        ]);
        geo_types::Polygon::new(ring, vec![])
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 < t.len() {
        let triangle = to_tri(i);
        let cx = (coords[t[i]].x + coords[t[i + 1]].x + coords[t[i + 2]].x) / 3.0;
        let cy = (coords[t[i]].y + coords[t[i + 1]].y + coords[t[i + 2]].y) / 3.0;
        if point_in_polygon(cx, cy, poly) {
            out.push(triangle);
        }
        i += 3;
    }
    out
}

// ----- accessors / type predicates / I/O --------------------------------

/// `ST_NRings(geom)` — total number of rings (exterior + interior) across all
/// polygons in the geometry.
pub fn n_rings(g: &Geom) -> Option<i32> {
    let rec = |g: &Geom| -> usize {
        match g {
            Geometry::Polygon(p) => 1 + p.interiors().len(),
            Geometry::MultiPolygon(mp) => {
                mp.0.iter().map(|p| 1 + p.interiors().len()).sum()
            }
            Geometry::GeometryCollection(c) => c.0.iter().map(|item| n_rings(item).unwrap_or(0) as usize).sum(),
            _ => 0,
        }
    };
    rec(g).try_into().ok()
}

/// `ST_OrderingEquals(a, b)` — structural equality: same type, same coordinate
/// order. Implemented as byte-equal canonical WKB, which our serializer
/// guarantees is canonical per-geometry.
pub fn ordering_equals(a: &Geom, b: &Geom) -> Option<bool> {
    let ka = crate::geometry::to_wkb(a).ok()?;
    let kb = crate::geometry::to_wkb(b).ok()?;
    Some(ka == kb)
}

/// `ST_IsPoint(geom)`.
pub fn is_point(g: &Geom) -> Option<bool> {
    Some(matches!(g, Geometry::Point(_) | Geometry::MultiPoint(_)))
}
/// `ST_IsLineString(geom)`.
pub fn is_linestring(g: &Geom) -> Option<bool> {
    Some(matches!(g, Geometry::LineString(_) | Geometry::Line(_) | Geometry::MultiLineString(_)))
}
/// `ST_IsPolygon(geom)`.
pub fn is_polygon(g: &Geom) -> Option<bool> {
    Some(matches!(g, Geometry::Polygon(_) | Geometry::MultiPolygon(_)))
}

/// `ST_AsHexEWKB(geom)` — uppercase hex of the geometry's WKB (SRID-less, so
/// EWKB == WKB here).
pub fn as_hex_ewkb(g: &Geom) -> Option<String> {
    let bytes = crate::geometry::to_wkb(g).ok()?;
    let mut s = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        s.push_str(&format!("{:02X}", byte));
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{from_wkb, to_wkb};

    fn wkt_point(x: f64, y: f64) -> Geom {
        Geometry::Point(geo_types::Point::new(x, y))
    }

    fn wkb_of(g: &Geom) -> Vec<u8> {
        to_wkb(g).expect("serialize")
    }

    #[test]
    fn convex_hull_of_line_is_line() {
        // A two-point line's convex hull is a degenerate (collapsed) polygon.
        let ls = Geometry::LineString(geo_types::LineString::from(vec![(0.0, 0.0), (4.0, 4.0)]));
        let out = convex_hull(&ls).expect("hull");
        // Result is representable and round-trips.
        assert!(to_wkb(&out).is_ok());
    }

    #[test]
    fn envelope_of_polygon() {
        let poly = Geometry::Polygon(
            geo_types::Polygon::new(
                geo_types::LineString::from(vec![(1.0, 1.0), (5.0, 1.0), (5.0, 4.0), (1.0, 4.0), (1.0, 1.0)]),
                vec![],
            ),
        );
        let env = envelope(&poly).expect("envelope");
        match env {
            Geometry::Polygon(p) => {
                let bbox = p.bounding_rect().unwrap();
                assert_eq!(bbox.min().x, 1.0);
                assert_eq!(bbox.max().x, 5.0);
            }
            _ => panic!("expected polygon envelope"),
        }
    }

    #[test]
    fn centroid_and_area() {
        let poly = Geometry::Polygon(
            geo_types::Polygon::new(
                geo_types::LineString::from(vec![(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0), (0.0, 0.0)]),
                vec![],
            ),
        );
        let c = centroid(&poly).expect("centroid");
        match c {
            Geometry::Point(p) => {
                assert!((p.x() - 2.0).abs() < 1e-9);
                assert!((p.y() - 2.0).abs() < 1e-9);
            }
            _ => panic!("expected point centroid"),
        }
        assert!((area(&poly).unwrap() - 16.0).abs() < 1e-9);
    }

    #[test]
    fn predicates_on_overlapping_squares() {
        let a = Geometry::Polygon(geo_types::Polygon::new(
            geo_types::LineString::from(vec![(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0), (0.0, 0.0)]),
            vec![],
        ));
        let b = Geometry::Polygon(geo_types::Polygon::new(
            geo_types::LineString::from(vec![(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0), (1.0, 1.0)]),
            vec![],
        ));
        let inner = wkt_point(1.5, 1.5);

        assert_eq!(intersects(&a, &b), Some(true));
        assert_eq!(disjoint(&a, &b), Some(false));
        assert_eq!(contains(&a, &inner), Some(true));
        assert_eq!(within(&inner, &a), Some(true));
        assert_eq!(contains(&a, &b), Some(false));
    }

    #[test]
    fn dimension_and_type_names() {
        assert_eq!(dimension(&wkt_point(0.0, 0.0)), Some(0));
        assert_eq!(geometry_type(&wkt_point(0.0, 0.0)).as_deref(), Some("ST_Point"));
    }

    #[test]
    fn full_pipeline_through_wkb() {
        // Exercise from_wkb -> convex_hull -> to_wkb, the exact path the
        // dispatch layer takes for every ST_* call.
        let tri = Geometry::Polygon(geo_types::Polygon::new(
            geo_types::LineString::from(vec![(0.0, 0.0), (4.0, 0.0), (2.0, 4.0), (0.0, 0.0)]),
            vec![],
        ));
        let bytes = wkb_of(&tri);
        let parsed = from_wkb(&bytes).expect("parse");
        let hull = convex_hull(&parsed).expect("hull");
        let out = to_wkb(&hull).expect("serialize");
        // A convex hull is itself a valid geometry that re-parses.
        assert!(from_wkb(&out).is_ok());
    }

    // ---- dispatch-path isolation: mirrors str_to_geom → unary_geom_varchar ----
    fn dispatch_roundtrip(wkt_in: &str) -> Option<String> {
        let geom = geom_from_text(wkt_in)?;
        let bytes = crate::geometry::to_wkb(&geom).ok()?;
        let reparsed = crate::geometry::from_wkb(&bytes).ok()?;
        as_text(&reparsed)
    }

    #[test]
    fn isolate_value_dependent_bug() {
        // These are the exact cases that failed (NULL) under DuckDB. If they
        // pass here, the bug is in the FFI dispatch layer, not the geometry layer.
        for (wkt, label) in [
            ("POINT(1 2)", "POINT(1 2)"),
            ("POINT(3 4)", "POINT(3 4)"),
            ("POINT(0 0)", "POINT(0 0)"),
            ("POINT(1 0)", "POINT(1 0)"),
            ("POINT(2 0)", "POINT(2 0)"),
            ("POINT(3 0)", "POINT(3 0)"),
            ("POINT(4 0)", "POINT(4 0)"),
            ("POLYGON((0 0,1 0,1 1,0 1,0 0))", "POLYGON(1x1)"),
            ("POLYGON((0 0,4 0,4 4,0 4,0 0))", "POLYGON(4x4)"),
            ("LINESTRING(0 0, 3 4)", "LINESTRING"),
        ] {
            let got = dispatch_roundtrip(wkt);
            assert!(got.is_some(), "ROUNDTRIP FAILED (pure rust) for {label}: {wkt} -> {got:?}");
            println!("{label}: {got:?}");
        }
    }

    // ---- Tier 1 / 1b batch ----------------------------------------------

    fn rect_poly(x0: f64, y0: f64, x1: f64, y1: f64) -> Geom {
        Geometry::Polygon(geo_types::Polygon::new(
            geo_types::LineString::from(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]),
            vec![],
        ))
    }
    fn ls(coords: &[(f64, f64)]) -> Geom {
        Geometry::LineString(geo_types::LineString::from(coords.to_vec()))
    }

    #[test]
    fn affine_acts_as_translate_with_identity_matrix() {
        // a=1, b=0, d=0, e=1, xoff=5, yoff=5  ==> pure translate (5,5).
        let out = affine(&wkt_point(1.0, 2.0), 1.0, 0.0, 0.0, 1.0, 5.0, 5.0).unwrap();
        match out {
            Geometry::Point(p) => {
                assert!((p.x() - 6.0).abs() < 1e-9);
                assert!((p.y() - 7.0).abs() < 1e-9);
            }
            other => panic!("expected point, got {other:?}"),
        }
    }

    #[test]
    fn affine_scales_with_2x_3x_matrix() {
        // a=2, b=0, d=0, e=3, xoff=0, yoff=0  ==> scale (2,3).
        let out = affine(&wkt_point(1.0, 1.0), 2.0, 0.0, 0.0, 3.0, 0.0, 0.0).unwrap();
        match out {
            Geometry::Point(p) => {
                assert!((p.x() - 2.0).abs() < 1e-9);
                assert!((p.y() - 3.0).abs() < 1e-9);
            }
            other => panic!("expected point, got {other:?}"),
        }
    }

    #[test]
    fn segmentize_splits_long_segments() {
        let out = segmentize(&ls(&[(0.0, 0.0), (10.0, 0.0)]), 4.0).unwrap();
        let n = num_points(&out).unwrap();
        // 10/4 = 2.5 -> ceil = 3 sub-segments -> 4 vertices for an open line.
        assert_eq!(n, 4, "segmentize should produce 4 vertices");
    }

    #[test]
    fn segmentize_noop_for_short_segments() {
        let out = segmentize(&ls(&[(0.0, 0.0), (1.0, 0.0)]), 4.0).unwrap();
        assert_eq!(num_points(&out).unwrap(), 2);
    }

    #[test]
    fn line_substring_middle_half() {
        let out = line_substring(&ls(&[(0.0, 0.0), (10.0, 0.0)]), 0.25, 0.75).unwrap();
        match out {
            Geometry::LineString(ls) => {
                let xs: Vec<f64> = ls.0.iter().map(|c| c.x).collect();
                assert!((xs.first().unwrap() - 2.5).abs() < 1e-9);
                assert!((xs.last().unwrap() - 7.5).abs() < 1e-9);
            }
            other => panic!("expected linestring, got {other:?}"),
        }
    }

    #[test]
    fn line_substring_returns_none_for_inverted_fraction() {
        assert!(line_substring(&ls(&[(0.0, 0.0), (10.0, 0.0)]), 0.75, 0.25).is_none());
    }

    #[test]
    fn line_merge_chains_touching_lines() {
        let mls = Geometry::MultiLineString(geo_types::MultiLineString(vec![
            geo_types::LineString::from(vec![(0.0, 0.0), (1.0, 0.0)]),
            geo_types::LineString::from(vec![(1.0, 0.0), (2.0, 0.0)]),
        ]));
        let out = line_merge(&mls).unwrap();
        match out {
            Geometry::MultiLineString(mls) => {
                assert_eq!(mls.0.len(), 1, "merged into a single linestring");
                assert_eq!(mls.0[0].0.len(), 3);
            }
            other => panic!("expected multilinestring, got {other:?}"),
        }
    }

    #[test]
    fn collection_extract_polygons() {
        let gc = Geometry::GeometryCollection(geo_types::GeometryCollection(vec![
            rect_poly(0.0, 0.0, 1.0, 1.0),
            ls(&[(0.0, 0.0), (1.0, 1.0)]),
            wkt_point(2.0, 2.0),
        ]));
        let out = collection_extract(&gc, 3).unwrap();
        match out {
            Geometry::MultiPolygon(mp) => assert_eq!(mp.0.len(), 1),
            other => panic!("expected multipolygon, got {other:?}"),
        }
    }

    #[test]
    fn force_collection_wraps_single_geom() {
        let out = force_collection(&wkt_point(1.0, 2.0)).unwrap();
        match out {
            Geometry::GeometryCollection(c) => assert_eq!(c.0.len(), 1),
            other => panic!("expected collection, got {other:?}"),
        }
    }

    #[test]
    fn multi_promotes_single_to_multi() {
        let out = multi(&wkt_point(1.0, 2.0)).unwrap();
        assert!(matches!(out, Geometry::MultiPoint(_)));
        let out2 = multi(&rect_poly(0.0, 0.0, 1.0, 1.0)).unwrap();
        assert!(matches!(out2, Geometry::MultiPolygon(_)));
    }

    #[test]
    fn normalize_rotates_closed_ring_to_smallest_vertex() {
        // Polygon ring not starting at the smallest coord normalizes so its
        // exterior begins at the lex-min vertex.
        let p = rect_poly(2.0, 0.0, 4.0, 2.0);
        let out = normalize(&p).unwrap();
        match out {
            Geometry::Polygon(poly) => {
                let first = poly.exterior().0[0];
                // The lex-min vertex of this ring is (0,0)-relative... here (2,0).
                assert!((first.x - 2.0).abs() < 1e-9 && (first.y - 0.0).abs() < 1e-9);
            }
            other => panic!("expected polygon, got {other:?}"),
        }
    }

    #[test]
    fn max_distance_between_two_squares() {
        let a = rect_poly(0.0, 0.0, 1.0, 1.0);
        let b = rect_poly(4.0, 4.0, 5.0, 5.0);
        let d = max_distance(&a, &b).unwrap();
        // furthest pair: (0,0) and (5,5) -> sqrt(50)
        assert!((d - 50.0_f64.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn longest_line_realizes_max_distance() {
        let a = rect_poly(0.0, 0.0, 1.0, 1.0);
        let b = rect_poly(4.0, 4.0, 5.0, 5.0);
        let line = longest_line(&a, &b).unwrap();
        match line {
            Geometry::LineString(ls) => {
                let dx = ls.0[1].x - ls.0[0].x;
                let dy = ls.0[1].y - ls.0[0].y;
                assert!((dx.hypot(dy) - 50.0_f64.sqrt()).abs() < 1e-9);
            }
            other => panic!("expected linestring, got {other:?}"),
        }
    }

    #[test]
    fn shortest_line_between_point_and_segment() {
        // Point straight above the midpoint of a horizontal segment: shortest
        // line should land at (1,0) and have length 1.
        let p = wkt_point(1.0, 1.0);
        let s = ls(&[(0.0, 0.0), (2.0, 0.0)]);
        let line = shortest_line(&p, &s).unwrap();
        match line {
            Geometry::LineString(ls) => {
                assert!((ls.0[1].x - 1.0).abs() < 1e-9);
                assert!((ls.0[1].y - 0.0).abs() < 1e-9);
            }
            other => panic!("expected linestring, got {other:?}"),
        }
    }

    #[test]
    fn triangulate_polygon_covers_area() {
        let p = rect_poly(0.0, 0.0, 2.0, 2.0);
        let out = triangulate_polygon(&p).unwrap();
        // Two triangles whose total area == the rectangle area (4).
        let mut total = 0.0_f64;
        if let Geometry::GeometryCollection(c) = &out {
            for item in &c.0 {
                if let Geometry::Polygon(t) = item {
                    total += t.unsigned_area();
                }
            }
        }
        assert!((total - 4.0).abs() < 1e-9, "triangulation covers the area");
    }

    #[test]
    fn n_rings_counts_exterior_and_holes() {
        let p = Geometry::Polygon(geo_types::Polygon::new(
            geo_types::LineString::from(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)]),
            vec![geo_types::LineString::from(vec![
                (2.0, 2.0), (4.0, 2.0), (4.0, 4.0), (2.0, 4.0), (2.0, 2.0),
            ])],
        ));
        assert_eq!(n_rings(&p).unwrap(), 2);
        // A bare polygon: 1 exterior + 1 hole = 2 rings.
    }

    #[test]
    fn ordering_equals_byte_equal_wkb() {
        let a = wkt_point(1.0, 2.0);
        let b = wkt_point(1.0, 2.0);
        assert_eq!(ordering_equals(&a, &b), Some(true));
        let c = wkt_point(1.0, 3.0);
        assert_eq!(ordering_equals(&a, &c), Some(false));
    }

    #[test]
    fn type_predicates() {
        assert_eq!(is_point(&wkt_point(0.0, 0.0)), Some(true));
        assert_eq!(is_linestring(&wkt_point(0.0, 0.0)), Some(false));
        assert_eq!(is_polygon(&rect_poly(0.0, 0.0, 1.0, 1.0)), Some(true));
        assert_eq!(is_polygon(&wkt_point(0.0, 0.0)), Some(false));
    }

    #[test]
    fn as_hex_ewkb_is_uppercase_hex() {
        let g = wkt_point(1.0, 2.0);
        let wkb = crate::geometry::to_wkb(&g).unwrap();
        let h = as_hex_ewkb(&g).unwrap();
        assert_eq!(h.len(), wkb.len() * 2);
        // Starts with the LE marker byte 01.
        assert!(h.starts_with("01"));
        // Every char is a hex digit; the encoding is uppercase (A-F where used).
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(h.chars().all(|c| !c.is_ascii_lowercase()));
    }
}
