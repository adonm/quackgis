-- postgis_dump.sql - curated from PostGIS regress: dump, dumppoints,
-- dumpsegments, dumprings. Table functions (our syntax: FROM st_dump(g)).
-- Source: regress/core/{dump,dumppoints,dumpsegments}.sql
.mode list

-- ======================================================================
-- ST_Dump - decompose multi/part collections into individual geometries
-- ======================================================================

-- MULTIPOINT → 2 individual points
SELECT CASE WHEN (SELECT count(*) FROM st_dump(
    st_geomfromtext('MULTIPOINT((0 0),(1 1))')
)) = 2
THEN 'PASS dump multipoint' ELSE 'FAIL dump multipoint' END;

-- MULTIPOLYGON → 2 individual polygons
SELECT CASE WHEN (SELECT count(*) FROM st_dump(
    st_geomfromtext('MULTIPOLYGON(((0 0,1 0,1 1,0 0)),((2 2,3 2,3 3,2 2)))')
)) = 2
THEN 'PASS dump multipolygon' ELSE 'FAIL dump multipolygon' END;

-- Single geometry → 1 row
SELECT CASE WHEN (SELECT count(*) FROM st_dump(
    st_geomfromtext('POINT(0 0)')
)) = 1
THEN 'PASS dump single' ELSE 'FAIL dump single' END;

-- GEOMETRYCOLLECTION → N rows
SELECT CASE WHEN (SELECT count(*) FROM st_dump(
    st_geomfromtext('GEOMETRYCOLLECTION(POINT(1 1),LINESTRING(0 0,1 1))')
)) = 2
THEN 'PASS dump collection' ELSE 'FAIL dump collection' END;

-- Nested collection → flattened
SELECT CASE WHEN (SELECT count(*) FROM st_dump(
    st_geomfromtext('GEOMETRYCOLLECTION(GEOMETRYCOLLECTION(POINT(1 1),POINT(2 2)),POINT(3 3))')
)) >= 3
THEN 'PASS dump nested' ELSE 'FAIL dump nested' END;

-- ======================================================================
-- ST_DumpPoints - all vertices as individual points
-- ======================================================================

-- LineString → N points
SELECT CASE WHEN (SELECT count(*) FROM st_dumppoints(
    st_geomfromtext('LINESTRING(0 0,1 1,2 2,3 3)')
)) = 4
THEN 'PASS dumppoints line' ELSE 'FAIL dumppoints line' END;

-- Polygon → exterior ring vertices
SELECT CASE WHEN (SELECT count(*) FROM st_dumppoints(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0))')
)) = 5  -- closed ring: 4 unique + closing point
THEN 'PASS dumppoints polygon' ELSE 'FAIL dumppoints polygon' END;

-- Polygon with hole → exterior + interior
SELECT CASE WHEN (SELECT count(*) FROM st_dumppoints(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0),(2 2,4 2,4 4,2 4,2 2))')
)) = 10  -- 5 exterior + 5 interior
THEN 'PASS dumppoints polygon_hole' ELSE 'FAIL dumppoints polygon_hole' END;

-- MULTIPOINT → each point
SELECT CASE WHEN (SELECT count(*) FROM st_dumppoints(
    st_geomfromtext('MULTIPOINT((0 0),(1 1),(2 2))')
)) = 3
THEN 'PASS dumppoints multipoint' ELSE 'FAIL dumppoints multipoint' END;

-- Point → 1 vertex
SELECT CASE WHEN (SELECT count(*) FROM st_dumppoints(
    st_geomfromtext('POINT(0 0)')
)) = 1
THEN 'PASS dumppoints point' ELSE 'FAIL dumppoints point' END;

-- ======================================================================
-- ST_DumpSegments - individual line segments
-- ======================================================================

-- LineString with 3 points → 2 segments
SELECT CASE WHEN (SELECT count(*) FROM st_dumpsegments(
    st_geomfromtext('LINESTRING(0 0,1 1,2 2)')
)) = 2
THEN 'PASS dumpsegments line' ELSE 'FAIL dumpsegments line' END;

-- Polygon exterior ring → segments
SELECT CASE WHEN (SELECT count(*) FROM st_dumpsegments(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))')
)) >= 4
THEN 'PASS dumpsegments polygon' ELSE 'FAIL dumpsegments polygon' END;

-- Triangle (3 segments)
SELECT CASE WHEN (SELECT count(*) FROM st_dumpsegments(
    st_geomfromtext('LINESTRING(0 0,1 0,0.5 1,0 0)')
)) = 3
THEN 'PASS dumpsegments triangle' ELSE 'FAIL dumpsegments triangle' END;

-- ======================================================================
-- ST_DumpRings - individual rings of a polygon
-- ======================================================================

-- Polygon without holes → 1 ring
SELECT CASE WHEN (SELECT count(*) FROM st_dumprings(
    st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))')
)) = 1
THEN 'PASS dumprings no_holes' ELSE 'FAIL dumprings no_holes' END;

-- Polygon with 1 hole → 2 rings
SELECT CASE WHEN (SELECT count(*) FROM st_dumprings(
    st_geomfromtext('POLYGON((0 0,10 0,10 10,0 10,0 0),(2 2,4 2,4 4,2 4,2 2))')
)) = 2
THEN 'PASS dumprings one_hole' ELSE 'FAIL dumprings one_hole' END;

-- Polygon with 2 holes → 3 rings
SELECT CASE WHEN (SELECT count(*) FROM st_dumprings(
    st_geomfromtext('POLYGON((0 0,20 0,20 20,0 20,0 0),(2 2,4 2,4 4,2 4,2 2),(8 8,10 8,10 10,8 10,8 8))')
)) = 3
THEN 'PASS dumprings two_holes' ELSE 'FAIL dumprings two_holes' END;
