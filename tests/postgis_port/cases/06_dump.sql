.mode list
-- Port family: dump and set-returning functions
-- Tests that PostGIS dump-family SQL ports (table-function syntax varies).

-- PG: SELECT path[1], ST_AsText(geom) FROM ST_Dump(ST_GeomFromText('MULTIPOINT(0 0, 1 1, 2 2)'));
-- Expected: 3 rows: {1} POINT(0 0), {2} POINT(1 1), {3} POINT(2 2)
-- Rewrite: FROM st_dump(...) instead of lateral cross join
SELECT CASE WHEN count(*) = 3
THEN 'PASS dump_multipoint_count' ELSE 'FAIL dump_multipoint_count' END
FROM st_dump(st_geomfromtext('MULTIPOINT(0 0, 1 1, 2 2)'));

-- PG: SELECT ST_AsText(geom) FROM ST_DumpPoints(ST_GeomFromText('LINESTRING(0 0, 1 1, 2 2)'));
-- Expected: 3 rows
SELECT CASE WHEN count(*) = 3
THEN 'PASS dumppoints_linestring' ELSE 'FAIL dumppoints_linestring' END
FROM st_dumppoints(st_geomfromtext('LINESTRING(0 0, 1 1, 2 2)'));

-- PG: SELECT ST_AsText(geom) FROM ST_DumpPoints(ST_GeomFromText('POLYGON((0 0,2 0,2 2,0 2,0 0))'));
-- Expected: 5 rows (including closing point)
SELECT CASE WHEN count(*) = 5
THEN 'PASS dumppoints_polygon' ELSE 'FAIL dumppoints_polygon' END
FROM st_dumppoints(st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'));

-- PG: SELECT count(*) FROM ST_DumpRings(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))'));
-- Expected: 2 (1 exterior + 1 interior)
SELECT CASE WHEN count(*) = 2
THEN 'PASS dumprings_polygon_with_hole' ELSE 'FAIL dumprings_polygon_with_hole' END
FROM st_dumprings(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))'));

-- PG: SELECT count(*) FROM ST_DumpSegments(ST_GeomFromText('LINESTRING(0 0, 1 1, 2 2, 3 3)'));
-- Expected: 3 segments
SELECT CASE WHEN count(*) = 3
THEN 'PASS dumpsegments_linestring' ELSE 'FAIL dumpsegments_linestring' END
FROM st_dumpsegments(st_geomfromtext('LINESTRING(0 0, 1 1, 2 2, 3 3)'));
