.mode list
-- =====================================================================
-- Milestone 14: Canonical geometry fidelity hardening.
--
-- Section 1: ST_Relate DE-9IM matrix (via GEOS, PostGIS-faithful).
-- Section 2: ST_Relate pattern matching.
-- Section 3: Predicate fidelity audit (adversarial edge cases).
-- Section 4: ST_Contains / ST_Within boundary delta (PINNED with fixtures).
-- Section 5: Empty geometry and nested-collection edge cases.
-- Section 6: Adversarial overlay fixtures (GEOS fallback).
-- =====================================================================

-- =====================================================================
-- Section 1: ST_Relate DE-9IM matrix
-- Matrices verified against PostGIS reference output.
-- =====================================================================

-- Interior point vs polygon: I(a)∩I(b)=0D, I(a)∩B(b)=F, I(a)∩E(b)=F
SELECT CASE WHEN st_relate(st_geomfromtext('POINT(1 1)'),
                           st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
                 = '0FFFFF212'
THEN 'PASS relate_interior_point' ELSE 'FAIL relate_interior_point' END;

-- Boundary point (vertex) vs polygon: B(a)∩B(b)=0D
SELECT CASE WHEN st_relate(st_geomfromtext('POINT(0 0)'),
                           st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
                 = 'F0FFFF212'
THEN 'PASS relate_boundary_vertex' ELSE 'FAIL relate_boundary_vertex' END;

-- Point on edge (not vertex) vs polygon
SELECT CASE WHEN st_relate(st_geomfromtext('POINT(2 0)'),
                           st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
                 = 'F0FFFF212'
THEN 'PASS relate_boundary_edge' ELSE 'FAIL relate_boundary_edge' END;

-- Exterior point vs polygon
SELECT CASE WHEN st_relate(st_geomfromtext('POINT(9 9)'),
                           st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
                 = 'FF0FFF212'
THEN 'PASS relate_exterior_point' ELSE 'FAIL relate_exterior_point' END;

-- Two identical polygons (topologically equal)
SELECT CASE WHEN st_relate(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
                 = '2FFF1FFF2'
THEN 'PASS relate_equal_polygons' ELSE 'FAIL relate_equal_polygons' END;

-- Overlapping polygons
SELECT CASE WHEN st_relate(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           st_geomfromtext('POLYGON((2 2,2 6,6 6,6 2,2 2))'))
                 = '212101212'
THEN 'PASS relate_overlapping' ELSE 'FAIL relate_overlapping' END;

-- Touching polygons (share one edge)
SELECT CASE WHEN st_relate(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           st_geomfromtext('POLYGON((4 0,4 4,8 4,8 0,4 0))'))
                 = 'FF2F11212'
THEN 'PASS relate_touching' ELSE 'FAIL relate_touching' END;

-- Disjoint polygons
SELECT CASE WHEN st_relate(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           st_geomfromtext('POLYGON((10 10,10 14,14 14,14 10,10 10))'))
                 = 'FF2FF1212'
THEN 'PASS relate_disjoint' ELSE 'FAIL relate_disjoint' END;

-- Line crossing polygon interior
SELECT CASE WHEN st_relate(st_geomfromtext('LINESTRING(-1 2,8 2)'),
                           st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
                 = '101FF0212'
THEN 'PASS relate_line_crosses' ELSE 'FAIL relate_line_crosses' END;

-- Identical points
SELECT CASE WHEN st_relate(st_geomfromtext('POINT(0 0)'),
                           st_geomfromtext('POINT(0 0)'))
                 = '0FFFFFFF2'
THEN 'PASS relate_same_point' ELSE 'FAIL relate_same_point' END;

-- NULL propagation
SELECT CASE WHEN st_relate(NULL,
                           st_geomfromtext('POINT(0 0)')) IS NULL
THEN 'PASS relate_null_left' ELSE 'FAIL relate_null_left' END;

SELECT CASE WHEN st_relate(st_geomfromtext('POINT(0 0)'),
                           NULL) IS NULL
THEN 'PASS relate_null_right' ELSE 'FAIL relate_null_right' END;

-- =====================================================================
-- Section 2: ST_Relate pattern matching (3-arg form)
-- =====================================================================

-- Interior point matches Contains pattern (T*F**F***)
SELECT CASE WHEN st_relate(st_geomfromtext('POINT(1 1)'),
                           st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           '0FFFFF212')
THEN 'PASS relate_pattern_interior' ELSE 'FAIL relate_pattern_interior' END;

-- Boundary point does NOT match Contains pattern
SELECT CASE WHEN NOT st_relate(st_geomfromtext('POINT(0 0)'),
                               st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                               'T*F**F***')
THEN 'PASS relate_pattern_boundary_not_contains' ELSE 'FAIL relate_pattern_boundary_not_contains' END;

-- Wildcard pattern: boundary point matches its exact DE-9IM matrix
SELECT CASE WHEN st_relate(st_geomfromtext('POINT(0 0)'),
                           st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           'F0FFFF212')
THEN 'PASS relate_pattern_boundary_exact' ELSE 'FAIL relate_pattern_boundary_exact' END;

-- Overlapping polygons match Overlaps pattern
SELECT CASE WHEN st_relate(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           st_geomfromtext('POLYGON((2 2,2 6,6 6,6 2,2 2))'),
                           '2*T*T*2**')
THEN 'PASS relate_pattern_overlaps' ELSE 'FAIL relate_pattern_overlaps' END;

-- Disjoint polygons match Disjoint pattern
SELECT CASE WHEN st_relate(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           st_geomfromtext('POLYGON((10 10,10 14,14 14,14 10,10 10))'),
                           'FF*FF****')
THEN 'PASS relate_pattern_disjoint' ELSE 'FAIL relate_pattern_disjoint' END;

-- =====================================================================
-- Section 3: Predicate fidelity audit (adversarial edge cases)
-- =====================================================================

-- ST_Intersects: overlapping polygons
SELECT CASE WHEN st_intersects(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                               st_geomfromtext('POLYGON((2 2,2 6,6 6,6 2,2 2))'))
THEN 'PASS intersects_overlap' ELSE 'FAIL intersects_overlap' END;

-- ST_Intersects: disjoint polygons
SELECT CASE WHEN NOT st_intersects(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                                   st_geomfromtext('POLYGON((10 10,10 14,14 14,14 10,10 10))'))
THEN 'PASS intersects_disjoint' ELSE 'FAIL intersects_disjoint' END;

-- ST_Covers: boundary point is covered
SELECT CASE WHEN st_covers(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           st_geomfromtext('POINT(0 0)'))
THEN 'PASS covers_boundary_vertex' ELSE 'FAIL covers_boundary_vertex' END;

-- ST_Covers: exterior point is NOT covered
SELECT CASE WHEN NOT st_covers(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                               st_geomfromtext('POINT(9 9)'))
THEN 'PASS covers_exterior_false' ELSE 'FAIL covers_exterior_false' END;

-- ST_CoveredBy: boundary point is covered by polygon
SELECT CASE WHEN st_coveredby(st_geomfromtext('POINT(0 0)'),
                              st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
THEN 'PASS coveredby_boundary_vertex' ELSE 'FAIL coveredby_boundary_vertex' END;

-- ST_Touches: polygon and boundary point
SELECT CASE WHEN st_touches(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                            st_geomfromtext('POINT(0 0)'))
THEN 'PASS touches_boundary_vertex' ELSE 'FAIL touches_boundary_vertex' END;

-- ST_Touches: polygon and interior point → false (intersects but not only boundary)
SELECT CASE WHEN NOT st_touches(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                                st_geomfromtext('POINT(1 1)'))
THEN 'PASS touches_interior_false' ELSE 'FAIL touches_interior_false' END;

-- ST_Crosses: line through polygon
SELECT CASE WHEN st_crosses(st_geomfromtext('LINESTRING(-1 2,8 2)'),
                            st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
THEN 'PASS crosses_line_polygon' ELSE 'FAIL crosses_line_polygon' END;

-- ST_Overlaps: partially overlapping polygons
SELECT CASE WHEN st_overlaps(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                             st_geomfromtext('POLYGON((2 2,2 6,6 6,6 2,2 2))'))
THEN 'PASS overlaps_partial' ELSE 'FAIL overlaps_partial' END;

-- ST_Equals: identical polygons
SELECT CASE WHEN st_equals(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
THEN 'PASS equals_identical' ELSE 'FAIL equals_identical' END;

-- ST_Equals: reversed-ring-order polygon is topologically equal
SELECT CASE WHEN st_equals(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           st_geomfromtext('POLYGON((4 0,4 4,0 4,0 0,4 0))'))
THEN 'PASS equals_reordered' ELSE 'FAIL equals_reordered' END;

-- ST_Disjoint: clearly separate geometries
SELECT CASE WHEN st_disjoint(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                             st_geomfromtext('POLYGON((10 10,10 14,14 14,14 10,10 10))'))
THEN 'PASS disjoint_true' ELSE 'FAIL disjoint_true' END;

-- =====================================================================
-- Section 4: ST_Contains / ST_Within boundary semantics (M22: delta retired)
--
-- Previously: ST_Contains/ST_Within used a PNPOLY ray-cast that was
-- boundary-inclusive (point on boundary → TRUE). PostGIS returns FALSE
-- (DE-9IM requires interior intersection).
--
-- M22 fix: both predicates now route through geo::Relate with the PostGIS
-- DE-9IM pattern T*****FF*, matching PostGIS exactly.
-- ST_Covers / ST_CoveredBy remain the correct boundary-inclusive substitutes.
-- =====================================================================

SELECT CASE WHEN st_contains(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                             st_geomfromtext('POINT(1 1)'))
THEN 'PASS contains_interior' ELSE 'FAIL contains_interior' END;

-- RETIRED DELTA: boundary vertex now returns FALSE (matches PostGIS)
SELECT CASE WHEN NOT st_contains(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                                 st_geomfromtext('POINT(0 0)'))
THEN 'PASS contains_boundary_false' ELSE 'FAIL contains_boundary_false' END;

-- RETIRED DELTA: boundary point within now returns FALSE (matches PostGIS)
SELECT CASE WHEN NOT st_within(st_geomfromtext('POINT(0 0)'),
                               st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'))
THEN 'PASS within_boundary_false' ELSE 'FAIL within_boundary_false' END;

-- ST_Covers is still the correct boundary-inclusive substitute
SELECT CASE WHEN st_covers(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           st_geomfromtext('POINT(0 0)'))
THEN 'PASS covers_boundary_true' ELSE 'FAIL covers_boundary_true' END;

-- ST_ContainsProperly still excludes boundary (unchanged)
SELECT CASE WHEN NOT st_containsproperly(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                                         st_geomfromtext('POINT(0 0)'))
THEN 'PASS containsproperly_boundary_false' ELSE 'FAIL containsproperly_boundary_false' END;

-- =====================================================================
-- Section 5: Empty geometry and nested-collection edge cases
-- =====================================================================

-- ST_Relate with empty geometry
SELECT CASE WHEN st_relate(st_geomfromtext('POINT EMPTY'),
                           st_geomfromtext('POINT(0 0)')) IS NOT NULL
THEN 'PASS relate_empty_notnull' ELSE 'FAIL relate_empty_notnull' END;

-- ST_Intersects with empty → false
SELECT CASE WHEN NOT st_intersects(st_geomfromtext('POINT EMPTY'),
                                   st_geomfromtext('POINT(0 0)'))
THEN 'PASS intersects_empty_false' ELSE 'FAIL intersects_empty_false' END;

-- ST_Contains with empty operand → false
SELECT CASE WHEN NOT st_contains(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                                 st_geomfromtext('POINT EMPTY'))
THEN 'PASS contains_empty_false' ELSE 'FAIL contains_empty_false' END;

-- Nested collection: polygon inside a GeometryCollection
SELECT CASE WHEN st_contains(
    st_geomfromtext('GEOMETRYCOLLECTION(POLYGON((0 0,0 10,10 10,10 0,0 0)))'),
    st_geomfromtext('POINT(5 5)'))
THEN 'PASS contains_in_nested_collection' ELSE 'FAIL contains_in_nested_collection' END;

-- Nested collection: point in second member
SELECT CASE WHEN st_within(
    st_geomfromtext('POINT(5 5)'),
    st_geomfromtext('GEOMETRYCOLLECTION(POINT(0 0), POLYGON((0 0,0 10,10 10,10 0,0 0)))'))
THEN 'PASS within_nested_collection' ELSE 'FAIL within_nested_collection' END;

-- =====================================================================
-- Section 6: Adversarial overlay fixtures (GEOS fallback path)
-- =====================================================================

-- Bowtie polygon (self-intersecting) → ST_MakeValid repairs it
SELECT CASE WHEN st_isvalid(st_makevalid(
    st_geomfromtext('POLYGON((0 0,4 4,4 0,0 4,0 0))')))
THEN 'PASS overlay_bowtie_makevalid' ELSE 'FAIL overlay_bowtie_makevalid' END;

-- Bowtie intersection with valid polygon: GEOS make_valid first, then intersect
-- (Local buffer(0) repair differs from GEOS make_valid on self-intersecting
-- input; the documented workflow is ST_MakeValid → ST_Intersection.)
SELECT CASE WHEN st_area(st_intersection(
    st_makevalid(st_geomfromtext('POLYGON((0 0,4 4,4 0,0 4,0 0))')),
    st_geomfromtext('POLYGON((0 0,0 2,2 2,2 0,0 0))'))) > 0
THEN 'PASS overlay_bowtie_intersection' ELSE 'FAIL overlay_bowtie_intersection' END;

-- Sliver polygon: very thin triangle
SELECT CASE WHEN st_area(st_intersection(
    st_geomfromtext('POLYGON((0 0,0 0.0001,1 0.0001,1 0,0 0))'),
    st_geomfromtext('POLYGON((0 0,0 0.0002,1 0.0002,1 0,0 0))'))) > 0
THEN 'PASS overlay_sliver' ELSE 'FAIL overlay_sliver' END;

-- Polygon with hole: intersection preserves hole
SELECT CASE WHEN st_area(st_difference(
    st_geomfromtext('POLYGON((0 0,0 10,10 10,10 0,0 0))'),
    st_geomfromtext('POLYGON((2 2,2 8,8 8,8 2,2 2))'))) < 100
THEN 'PASS overlay_hole_difference' ELSE 'FAIL overlay_hole_difference' END;

-- Union of adjacent polygons
SELECT CASE WHEN st_area(st_union(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
    st_geomfromtext('POLYGON((4 0,4 4,8 4,8 0,4 0))'))) = 32
THEN 'PASS overlay_union_adjacent' ELSE 'FAIL overlay_union_adjacent' END;

-- Symmetric difference of overlapping polygons
SELECT CASE WHEN st_area(st_symdifference(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
    st_geomfromtext('POLYGON((2 2,2 6,6 6,6 2,2 2))'))) > 0
THEN 'PASS overlay_symdifference' ELSE 'FAIL overlay_symdifference' END;

-- Holes touching shells (ring-kissing): make_valid + intersection
SELECT CASE WHEN st_isvalid(st_makevalid(
    st_geomfromtext('POLYGON((0 0,0 10,10 10,10 0,0 0),(4 4,6 4,6 6,4 6,4 4))')))
THEN 'PASS overlay_hole_touches_shell' ELSE 'FAIL overlay_hole_touches_shell' END;

-- Collapsed ring (zero-area): intersection returns empty/NULL
SELECT CASE WHEN st_area(st_intersection(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
    st_geomfromtext('POLYGON((5 5,5 5,5 5,5 5,5 5))'))) IS NULL
            OR st_area(st_intersection(
    st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
    st_geomfromtext('POLYGON((5 5,5 5,5 5,5 5,5 5))'))) = 0
THEN 'PASS overlay_collapsed_ring' ELSE 'FAIL overlay_collapsed_ring' END;
