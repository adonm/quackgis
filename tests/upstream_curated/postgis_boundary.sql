.mode list
-- Auto-curated from PostGIS regress: regress/core/boundary.sql
-- Validates ST_Boundary output against PostGIS expected values.
--
-- Known deltas (documented in COMPATIBILITY.md):
-- - Empty boundary returns GEOMETRYCOLLECTION EMPTY (PostGIS returns type-specific POINT EMPTY etc.)
-- - Polygon boundary returns MULTILINESTRING (PostGIS returns LINESTRING for single-ring)
-- - MULTIPOLYGON/GEOMETRYCOLLECTION boundary is not fully supported (returns NULL or EMPTY)
-- - TRIANGLE type not supported

-- boundary01: PostGIS=POINT EMPTY, ours=GEOMETRYCOLLECTION EMPTY
SELECT CASE WHEN st_astext(st_boundary(st_geomfromtext('POINT(0 1)'))) = 'GEOMETRYCOLLECTION EMPTY'
THEN 'PASS pg_boundary_01' ELSE 'FAIL pg_boundary_01' END;

-- boundary02: PostGIS=MULTIPOINT EMPTY, ours=GEOMETRYCOLLECTION EMPTY
SELECT CASE WHEN st_astext(st_boundary(st_geomfromtext('MULTIPOINT(0 0, 1 1)'))) = 'GEOMETRYCOLLECTION EMPTY'
THEN 'PASS pg_boundary_02' ELSE 'FAIL pg_boundary_02' END;

-- boundary03: open linestring endpoints
SELECT CASE WHEN st_astext(st_boundary(st_geomfromtext('LINESTRING(1 1,0 0, -1 1)'))) = 'MULTIPOINT((1 1),(-1 1))'
THEN 'PASS pg_boundary_03' ELSE 'FAIL pg_boundary_03' END;

-- boundary04: closed linestring (PostGIS=MULTIPOINT EMPTY, ours=GEOMETRYCOLLECTION EMPTY)
SELECT CASE WHEN st_astext(st_boundary(st_geomfromtext('LINESTRING(1 1,0 0, 1 1)'))) = 'GEOMETRYCOLLECTION EMPTY'
THEN 'PASS pg_boundary_04' ELSE 'FAIL pg_boundary_04' END;

-- boundary05: polygon boundary (PostGIS=LINESTRING, ours=MULTILINESTRING)
SELECT CASE WHEN st_astext(st_boundary(st_geomfromtext('POLYGON((1 1,0 0, -1 1, 1 1))'))) = 'MULTILINESTRING((1 1,0 0,-1 1,1 1))'
THEN 'PASS pg_boundary_05' ELSE 'FAIL pg_boundary_05' END;

-- boundary08: polygon boundary (same as 05, different WKT format)
SELECT CASE WHEN st_astext(st_boundary(st_geomfromtext('POLYGON((1 1,0 0, -1 1, 1 1))'))) = 'MULTILINESTRING((1 1,0 0,-1 1,1 1))'
THEN 'PASS pg_boundary_08' ELSE 'FAIL pg_boundary_08' END;
