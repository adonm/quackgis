-- postgis_predicates.sql - curated from PostGIS regress: OGC predicates.
-- Tests spatial relationship functions using PostGIS test geometries.
-- Source: regress/core/regress_ogc.sql patterns + regress.sql
.mode list

-- ======================================================================
-- Setup test geometries
-- ======================================================================
-- Two overlapping squares
-- a: POLYGON((0 0,10 0,10 10,0 10,0 0))
-- b: POLYGON((5 5,15 5,15 15,5 15,5 5))

-- ======================================================================
-- ST_Intersects - any spatial intersection
-- ======================================================================
SELECT CASE WHEN st_intersects(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POLYGON((5 5,15 5,15 15,5 15,5 5))')
) = true
THEN 'PASS intersects overlap' ELSE 'FAIL intersects overlap' END;

SELECT CASE WHEN st_intersects(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POLYGON((20 0,30 0,30 10,20 10,20 0))')
) = false
THEN 'PASS intersects_disjoint' ELSE 'FAIL intersects_disjoint' END;

-- ======================================================================
-- ST_Contains - a contains b if b is entirely within a
-- ======================================================================
SELECT CASE WHEN st_contains(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_point(5, 5)
) = true
THEN 'PASS contains_point' ELSE 'FAIL contains_point' END;

SELECT CASE WHEN st_contains(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_point(15, 5)
) = false
THEN 'PASS contains_outside' ELSE 'FAIL contains_outside' END;

-- Boundary points: PostGIS ST_Contains returns false for boundary-only contact
-- (containsProperly returns true for interior-only)
SELECT CASE WHEN st_contains(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_point(0, 0)
) = false
THEN 'PASS contains_boundary' ELSE 'FAIL contains_boundary' END;

-- ======================================================================
-- ST_Within - inverse of contains
-- ======================================================================
SELECT CASE WHEN st_within(
    st_point(5, 5),
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))')
) = true
THEN 'PASS within_inside' ELSE 'FAIL within_inside' END;

SELECT CASE WHEN st_within(
    st_point(0, 0),
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))')
) = false
THEN 'PASS within_boundary' ELSE 'FAIL within_boundary' END;

-- ======================================================================
-- ST_Covers - a covers b if no point of b is outside a (includes boundary)
-- ======================================================================
SELECT CASE WHEN st_covers(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_point(0, 0)
) = true
THEN 'PASS covers_boundary' ELSE 'FAIL covers_boundary' END;

SELECT CASE WHEN st_covers(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_point(-1, 5)
) = false
THEN 'PASS covers_outside' ELSE 'FAIL covers_outside' END;

-- ======================================================================
-- ST_CoveredBy - inverse of covers
-- ======================================================================
SELECT CASE WHEN st_coveredby(
    st_point(0, 0),
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))')
) = true
THEN 'PASS coveredby_boundary' ELSE 'FAIL coveredby_boundary' END;

-- ======================================================================
-- ST_Disjoint - no spatial intersection
-- ======================================================================
SELECT CASE WHEN st_disjoint(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POLYGON((20 0,30 0,30 10,20 10,20 0))')
) = true
THEN 'PASS disjoint_true' ELSE 'FAIL disjoint_true' END;

SELECT CASE WHEN st_disjoint(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POLYGON((5 5,15 5,15 15,5 15,5 5))')
) = false
THEN 'PASS disjoint_overlap' ELSE 'FAIL disjoint_overlap' END;

-- ======================================================================
-- ST_Touches - boundaries touch but interiors don't intersect
-- ======================================================================
SELECT CASE WHEN st_touches(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POLYGON((10 0,20 0,20 10,10 10,10 0))')
) = true
THEN 'PASS touches_edge' ELSE 'FAIL touches_edge' END;

SELECT CASE WHEN st_touches(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POLYGON((5 5,15 5,15 15,5 15,5 5))')
) = false
THEN 'PASS touches_overlap' ELSE 'FAIL touches_overlap' END;

-- ======================================================================
-- ST_Crosses - geometries cross (line through polygon, etc.)
-- ======================================================================
SELECT CASE WHEN st_crosses(
    st_geomfromtext('LINESTRING(0 5,20 5)'),
    st_geomfromtext('POLYGON((5 0,15 0,15 10,5 10,5 0))')
) = true
THEN 'PASS crosses line_poly' ELSE 'FAIL crosses line_poly' END;

SELECT CASE WHEN st_crosses(
    st_geomfromtext('LINESTRING(0 0,1 1)'),
    st_geomfromtext('POLYGON((20 0,30 0,30 10,20 10,20 0))')
) = false
THEN 'PASS crosses_disjoint' ELSE 'FAIL crosses_disjoint' END;

-- ======================================================================
-- ST_Overlaps - same dimension, partially overlapping
-- ======================================================================
SELECT CASE WHEN st_overlaps(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POLYGON((5 5,15 5,15 15,5 15,5 5))')
) = true
THEN 'PASS overlaps_partial' ELSE 'FAIL overlaps_partial' END;

-- ======================================================================
-- ST_Equals - same set of points (regardless of vertex order)
-- ======================================================================
SELECT CASE WHEN st_equals(
    st_geomfromtext('LINESTRING(0 0,1 1)'),
    st_geomfromtext('LINESTRING(1 1,0 0)')  -- reversed
) = true
THEN 'PASS equals reversed' ELSE 'FAIL equals reversed' END;

SELECT CASE WHEN st_equals(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POLYGON((0 0,0 10,10 10,10 0,0 0))')  -- same square, CW ring
) = true
THEN 'PASS equals different_direction' ELSE 'FAIL equals different_direction' END;

SELECT CASE WHEN st_equals(
    st_point(1, 2),
    st_point(1, 3)
) = false
THEN 'PASS equals different' ELSE 'FAIL equals different' END;

-- ======================================================================
-- ST_OrderingEquals - exact binary equality (same WKB)
-- ======================================================================
SELECT CASE WHEN st_orderingequals(
    st_geomfromtext('POINT(1 2)'),
    st_geomfromtext('POINT(1 2)')
) = true
THEN 'PASS ordering_equals same' ELSE 'FAIL ordering_equals same' END;

SELECT CASE WHEN st_orderingequals(
    st_geomfromtext('LINESTRING(0 0,1 1)'),
    st_geomfromtext('LINESTRING(1 1,0 0)')  -- reversed: different ordering
) = false
THEN 'PASS ordering_equals reversed' ELSE 'FAIL ordering_equals reversed' END;

-- ======================================================================
-- ST_IsValid - geometry validity
-- ======================================================================
SELECT CASE WHEN st_isvalid(st_geomfromtext(
    'POLYGON((0 0,10 0,10 10,0 10,0 0))'
)) = true
THEN 'PASS isvalid valid' ELSE 'FAIL isvalid valid' END;

SELECT CASE WHEN st_isvalid(st_geomfromtext(
    'POLYGON((0 0,10 0,10 10,0 10,0 0),(5 5,15 5,15 15,5 15,5 5))'  -- hole outside shell
)) = false
THEN 'PASS isvalid hole_outside' ELSE 'FAIL isvalid hole_outside' END;

-- ======================================================================
-- ST_Relate - already covered by 40 tests in postgis_relate.sql.
-- Verify st_relate_pattern for a standard contains case.
-- ======================================================================
SELECT CASE WHEN st_relate(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POINT(5 5)')
) LIKE '0%FF%212'
            OR st_relate(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POINT(5 5)')
) LIKE '0F%F%FF2'
THEN 'PASS relate contains' ELSE 'FAIL relate contains' END;
