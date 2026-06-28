.mode list
-- Port family: overlay operations
-- Tests that PostGIS boolean set operations port directly.

-- PG: SELECT ST_Area(ST_Intersection(
-- PG:   ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
-- PG:   ST_GeomFromText('POLYGON((2 2,6 2,6 6,2 6,2 2))')));
-- Expected: 4  (2×2 overlap)
SELECT CASE WHEN abs(st_area(st_intersection(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POLYGON((2 2,6 2,6 6,2 6,2 2))'))) - 4.0) < 0.001
THEN 'PASS overlay_intersection_area' ELSE 'FAIL overlay_intersection_area' END;

-- PG: SELECT ST_Area(ST_Union(
-- PG:   ST_GeomFromText('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
-- PG:   ST_GeomFromText('POLYGON((1 1,3 1,3 3,1 3,1 1))')));
-- Expected: 7  (4 + 4 - 1 overlap)
SELECT CASE WHEN abs(st_area(st_union(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'))) - 7.0) < 0.001
THEN 'PASS overlay_union_area' ELSE 'FAIL overlay_union_area' END;

-- PG: SELECT ST_Area(ST_Difference(
-- PG:   ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
-- PG:   ST_GeomFromText('POLYGON((2 2,6 2,6 6,2 6,2 2))')));
-- Expected: 12  (16 - 4 clipped corner)
SELECT CASE WHEN abs(st_area(st_difference(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POLYGON((2 2,6 2,6 6,2 6,2 2))'))) - 12.0) < 0.001
THEN 'PASS overlay_difference_area' ELSE 'FAIL overlay_difference_area' END;

-- PG: SELECT ST_Area(ST_SymDifference(
-- PG:   ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
-- PG:   ST_GeomFromText('POLYGON((2 2,6 2,6 6,2 6,2 2))')));
-- Expected: 24  (SymDiff = (A−B) + (B−A) = 12 + 12 = 24)
SELECT CASE WHEN abs(st_area(st_symdifference(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POLYGON((2 2,6 2,6 6,2 6,2 2))'))) - 24.0) < 0.001
THEN 'PASS overlay_symdifference_area' ELSE 'FAIL overlay_symdifference_area' END;

-- PG: SELECT ST_IsEmpty(ST_Intersection(
-- PG:   ST_GeomFromText('POLYGON((0 0,1 0,1 1,0 1,0 0))'),
-- PG:   ST_GeomFromText('POLYGON((10 10,11 10,11 11,10 11,10 10))')));
-- Expected: t  (disjoint → empty or NULL depending on version)
-- PostGIS returns GEOMETRYCOLLECTION EMPTY for disjoint intersection.
SELECT CASE WHEN st_intersection(
    st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'),
    st_geomfromtext('POLYGON((10 10,11 10,11 11,10 11,10 10))')) IS NULL
              OR
              st_isempty(st_intersection(
    st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'),
    st_geomfromtext('POLYGON((10 10,11 10,11 11,10 11,10 10))'))) = true
THEN 'PASS overlay_disjoint' ELSE 'FAIL overlay_disjoint' END;
