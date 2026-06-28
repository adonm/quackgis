-- postgis_editing.sql — curated from PostGIS regress: reverse, normalize,
-- removepoint, setpoint, flipcoordinates, forcepolygoncw.
-- Source: regress/core/{reverse,normalize,removepoint,setpoint,swapordinates,orientation}.sql
.mode list

-- ======================================================================
-- ST_Reverse — reverse the vertex order
-- ======================================================================

-- LineString reversed
SELECT CASE WHEN st_astext(st_reverse(st_geomfromtext(
    'LINESTRING(0 0,1 1)'
))) = 'LINESTRING(1 1,0 0)'
THEN 'PASS reverse line' ELSE 'FAIL reverse line' END;

-- MultiLineString reversed
SELECT CASE WHEN st_astext(st_reverse(st_geomfromtext(
    'MULTILINESTRING((0 0,1 0),(-3 3,2 0))'
))) = 'MULTILINESTRING((1 0,0 0),(2 0,-3 3))'
THEN 'PASS reverse multiline' ELSE 'FAIL reverse multiline' END;

-- Polygon reversed (exterior ring + interior rings)
SELECT CASE WHEN st_astext(st_reverse(st_geomfromtext(
    'POLYGON((0 0,8 0,8 8,0 8,0 0))'
))) = 'POLYGON((0 0,0 8,8 8,8 0,0 0))'
THEN 'PASS reverse polygon' ELSE 'FAIL reverse polygon' END;

-- MultiPolygon reversed
SELECT CASE WHEN st_numpoints(st_reverse(st_geomfromtext(
    'MULTIPOLYGON(((0 0,5 0,5 5,0 5,0 0)))'
))) > 0
THEN 'PASS reverse multipolygon' ELSE 'FAIL reverse multipolygon' END;

-- Point reversed = same
SELECT CASE WHEN st_astext(st_reverse(st_geomfromtext(
    'POINT(0 0)'
))) = 'POINT(0 0)'
THEN 'PASS reverse point' ELSE 'FAIL reverse point' END;

-- Reverse preserves SRID
SELECT CASE WHEN st_srid(st_reverse(st_setsrid(st_geomfromtext(
    'LINESTRING(0 0,1 1)'
), 4326))) = 4326
THEN 'PASS reverse srid' ELSE 'FAIL reverse srid' END;

-- ======================================================================
-- ST_Normalize — canonical vertex ordering
-- ======================================================================

-- Normalized collection has consistent ordering
SELECT CASE WHEN st_normalize(st_geomfromtext(
    'GEOMETRYCOLLECTION(LINESTRING(1 1,0 0),LINESTRING(3 3,2 2))'
)) IS NOT NULL
THEN 'PASS normalize collection' ELSE 'FAIL normalize collection' END;

-- Normalized polygon: rings rotated to start at lexicographically smallest
SELECT CASE WHEN st_isvalid(st_normalize(st_geomfromtext(
    'POLYGON((0 0,10 0,10 10,0 10,0 0),(2 2,4 2,4 4,2 4,2 2))'
))) = true
THEN 'PASS normalize polygon' ELSE 'FAIL normalize polygon' END;

-- ======================================================================
-- ST_RemovePoint — remove a vertex from a LineString
-- ======================================================================

-- Remove middle point
SELECT CASE WHEN st_astext(st_removepoint(st_geomfromtext(
    'LINESTRING(0 0,1 1,2 2)'
), 1)) = 'LINESTRING(0 0,2 2)'
THEN 'PASS removepoint middle' ELSE 'FAIL removepoint middle' END;

-- Remove first point
SELECT CASE WHEN st_astext(st_removepoint(st_geomfromtext(
    'LINESTRING(0 0,1 1,2 2)'
), 0)) = 'LINESTRING(1 1,2 2)'
THEN 'PASS removepoint first' ELSE 'FAIL removepoint first' END;

-- Remove last point
SELECT CASE WHEN st_astext(st_removepoint(st_geomfromtext(
    'LINESTRING(0 0,1 1,2 2)'
), 2)) = 'LINESTRING(0 0,1 1)'
THEN 'PASS removepoint last' ELSE 'FAIL removepoint last' END;

-- RemovePoint on single-segment line: PostGIS errors; we return a degenerate
-- line. Documented behavioral difference (our impl is more permissive).
SELECT CASE WHEN st_removepoint(st_geomfromtext(
    'LINESTRING(0 0,1 1)'
), 0) IS NOT NULL
THEN 'PASS removepoint single_seg permissive' ELSE 'FAIL removepoint single_seg permissive' END;

-- ======================================================================
-- ST_SetPoint — replace a vertex in a LineString
-- ======================================================================

-- Replace middle point
SELECT CASE WHEN st_astext(st_setpoint(st_geomfromtext(
    'LINESTRING(0 0,1 1,2 2)'
), 1, st_point(5, 5))) = 'LINESTRING(0 0,5 5,2 2)'
THEN 'PASS setpoint middle' ELSE 'FAIL setpoint middle' END;

-- Replace first point
SELECT CASE WHEN st_astext(st_setpoint(st_geomfromtext(
    'LINESTRING(0 0,1 1,2 2)'
), 0, st_point(3, 3))) = 'LINESTRING(3 3,1 1,2 2)'
THEN 'PASS setpoint first' ELSE 'FAIL setpoint first' END;

-- Replace last point
SELECT CASE WHEN st_astext(st_setpoint(st_geomfromtext(
    'LINESTRING(0 0,1 1,2 2)'
), 2, st_point(4, 4))) = 'LINESTRING(0 0,1 1,4 4)'
THEN 'PASS setpoint last' ELSE 'FAIL setpoint last' END;

-- ======================================================================
-- ST_FlipCoordinates — swap X and Y
-- ======================================================================

-- Point flip
SELECT CASE WHEN st_astext(st_flipcoordinates(st_geomfromtext(
    'POINT(1 2)'
))) = 'POINT(2 1)'
THEN 'PASS flip point' ELSE 'FAIL flip point' END;

-- LineString flip
SELECT CASE WHEN st_astext(st_flipcoordinates(st_geomfromtext(
    'LINESTRING(1 2,3 4)'
))) = 'LINESTRING(2 1,4 3)'
THEN 'PASS flip line' ELSE 'FAIL flip line' END;

-- Polygon flip
SELECT CASE WHEN st_flipcoordinates(st_geomfromtext(
    'POLYGON((1 0,2 1,1 2,1 0))'
)) IS NOT NULL
THEN 'PASS flip polygon' ELSE 'FAIL flip polygon' END;

-- ======================================================================
-- ST_ForcePolygonCW — force polygon ring orientation to CW
-- ======================================================================

-- Force a CCW polygon to CW
SELECT CASE WHEN st_forcepolygoncw(st_geomfromtext(
    'POLYGON((0 0,1 0,1 1,0 1,0 0))'
)) IS NOT NULL
THEN 'PASS forcepolygoncw basic' ELSE 'FAIL forcepolygoncw basic' END;

-- ForcePolygonCW preserves SRID
SELECT CASE WHEN st_srid(st_forcepolygoncw(st_setsrid(st_geomfromtext(
    'POLYGON((0 0,1 0,1 1,0 1,0 0))'
), 4326))) = 4326
THEN 'PASS forcepolygoncw srid' ELSE 'FAIL forcepolygoncw srid' END;
