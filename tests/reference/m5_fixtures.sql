-- SPDX-License-Identifier: Apache-2.0
-- Month 5 reference fixtures: GEOS overlay fallback, ST_ContainsProperly,
-- ST_DumpRings, and adversarial invalid/polygonal inputs.
--
-- Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < tests/reference/m5_fixtures.sql
.bail off
.mode list

-- ======================================================================
-- 1. ST_ContainsProperly (DE-9IM T**FF*FF*)
-- ======================================================================

-- Interior point: TRUE
SELECT CASE WHEN st_containsproperly(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POINT(1 1)'))
            THEN 'PASS containsproperly interior' ELSE 'FAIL containsproperly interior' END;

-- Boundary point: FALSE (point is on the boundary)
SELECT CASE WHEN NOT st_containsproperly(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POINT(0 0)'))
            THEN 'PASS containsproperly boundary' ELSE 'FAIL containsproperly boundary' END;

-- Exterior point: FALSE
SELECT CASE WHEN NOT st_containsproperly(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POINT(10 10)'))
            THEN 'PASS containsproperly exterior' ELSE 'FAIL containsproperly exterior' END;

-- Interior polygon: TRUE
SELECT CASE WHEN st_containsproperly(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POLYGON((2 2,4 2,4 4,2 4,2 2))'))
            THEN 'PASS containsproperly interior poly' ELSE 'FAIL containsproperly interior poly' END;

-- Touching polygon: FALSE (shares boundary)
SELECT CASE WHEN NOT st_containsproperly(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))'),
    st_geomfromtext('POLYGON((0 0,5 0,5 5,0 5,0 0))'))
            THEN 'PASS containsproperly touching poly' ELSE 'FAIL containsproperly touching poly' END;

-- NULL propagation
SELECT CASE WHEN st_containsproperly(NULL, st_geomfromtext('POINT(1 1)')) IS NULL
            THEN 'PASS containsproperly null' ELSE 'FAIL containsproperly null' END;

-- ======================================================================
-- 2. ST_DumpRings
-- ======================================================================

-- Polygon with one hole: 2 rings (exterior + 1 interior)
SELECT CASE WHEN (SELECT count(*) FROM st_dumprings(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))'))) = 2
            THEN 'PASS dumprings poly+hole count' ELSE 'FAIL dumprings poly+hole count' END;

-- Exterior ring at path {0}
SELECT CASE WHEN (SELECT st_astext(geom) FROM st_dumprings(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))'))
    WHERE path = '{0}') = 'LINESTRING(0 0,4 0,4 4,0 4,0 0)'
            THEN 'PASS dumprings exterior' ELSE 'FAIL dumprings exterior' END;

-- Interior ring at path {1}
SELECT CASE WHEN (SELECT st_astext(geom) FROM st_dumprings(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))'))
    WHERE path = '{1}') = 'LINESTRING(1 1,2 1,2 2,1 2,1 1)'
            THEN 'PASS dumprings interior' ELSE 'FAIL dumprings interior' END;

-- Simple polygon (no holes): 1 ring
SELECT CASE WHEN (SELECT count(*) FROM st_dumprings(
    st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'))) = 1
            THEN 'PASS dumprings simple count' ELSE 'FAIL dumprings simple count' END;

-- MultiPolygon: each polygon contributes its rings
SELECT CASE WHEN (SELECT count(*) FROM st_dumprings(
    st_geomfromtext('MULTIPOLYGON(((0 0,1 0,1 1,0 1,0 0)),((2 2,3 2,3 3,2 3,2 2)))'))) = 2
            THEN 'PASS dumprings multipolygon' ELSE 'FAIL dumprings multipolygon' END;

-- Non-polygon: 0 rings
SELECT CASE WHEN (SELECT count(*) FROM st_dumprings(
    st_geomfromtext('POINT(1 2)'))) = 0
            THEN 'PASS dumprings non_polygon empty' ELSE 'FAIL dumprings non_polygon empty' END;

-- ======================================================================
-- 3. GEOS overlay fallback: invalid input that geo::BooleanOps panics on
-- ======================================================================

-- Self-intersecting bowtie polygon → make_valid first, then intersection
-- The local geo path would crash; ensure_valid + fallback handles it.
SELECT CASE WHEN abs(st_area(st_intersection(
    st_makevalid(st_geomfromtext('POLYGON((0 0,4 4,4 0,0 4,0 0))')),
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'))) - 8.0) < 0.01
            THEN 'PASS overlay fallback intersection' ELSE 'FAIL overlay fallback intersection' END;

-- Union of repaired bowtie + valid polygon
SELECT CASE WHEN st_area(st_union(
    st_makevalid(st_geomfromtext('POLYGON((0 0,4 4,4 0,0 4,0 0))')),
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'))) > 8.0
            THEN 'PASS overlay fallback union' ELSE 'FAIL overlay fallback union' END;

-- Difference should also work through the fallback
SELECT CASE WHEN st_area(st_difference(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_makevalid(st_geomfromtext('POLYGON((0 0,4 4,4 0,0 4,0 0))')))) > 0.0
            THEN 'PASS overlay fallback difference' ELSE 'FAIL overlay fallback difference' END;

-- SymDifference should work through the fallback
SELECT CASE WHEN st_area(st_symdifference(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_makevalid(st_geomfromtext('POLYGON((2 0,6 0,6 4,2 4,2 0))')))) > 0.0
            THEN 'PASS overlay fallback symdifference' ELSE 'FAIL overlay fallback symdifference' END;

-- ======================================================================
-- 4. Overlay on valid input still produces correct results
-- ======================================================================

-- Two overlapping squares: intersection area = 1
SELECT CASE WHEN abs(st_area(st_intersection(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'))) - 1.0) < 1e-9
            THEN 'PASS overlay valid intersection' ELSE 'FAIL overlay valid intersection' END;

-- Union of same: area = 7 (2×2 + 2×2 - 1×1 overlap)
SELECT CASE WHEN abs(st_area(st_union(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'))) - 7.0) < 1e-9
            THEN 'PASS overlay valid union' ELSE 'FAIL overlay valid union' END;

-- ======================================================================
-- 5. NULL propagation for overlay ops
-- ======================================================================

SELECT CASE WHEN st_intersection(NULL, st_geomfromtext('POINT(0 0)')) IS NULL
            THEN 'PASS null intersection' ELSE 'FAIL null intersection' END;

SELECT CASE WHEN st_union(NULL, NULL) IS NULL
            THEN 'PASS null union' ELSE 'FAIL null union' END;

SELECT CASE WHEN st_difference(st_geomfromtext('POINT(0 0)'), NULL) IS NULL
            THEN 'PASS null difference' ELSE 'FAIL null difference' END;

-- ======================================================================
-- 6. ST_ContainsProperly on collections
-- ======================================================================

-- MultiPolygon contains point properly
SELECT CASE WHEN st_containsproperly(
    st_geomfromtext('MULTIPOLYGON(((0 0,4 0,4 4,0 4,0 0)),((10 10,14 10,14 14,10 14,10 10)))'),
    st_geomfromtext('POINT(1 1)'))
            THEN 'PASS containsproperly multipoly' ELSE 'FAIL containsproperly multipoly' END;
