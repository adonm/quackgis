.mode list
-- Port family: DE-9IM predicates
-- Tests that common PostGIS predicate SQL ports directly.

-- PG: SELECT ST_Intersects(ST_GeomFromText('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
-- PG:                        ST_GeomFromText('POLYGON((1 1,3 1,3 3,1 3,1 1))'));
-- Expected: t
SELECT CASE WHEN st_intersects(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))')) = true
THEN 'PASS predicates_intersects' ELSE 'FAIL predicates_intersects' END;

-- PG: SELECT ST_Contains(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
-- PG:                    ST_Point(1, 1));
-- Expected: t
SELECT CASE WHEN st_contains(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_point(1.0, 1.0)) = true
THEN 'PASS predicates_contains' ELSE 'FAIL predicates_contains' END;

-- PG: SELECT ST_Contains(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
-- PG:                    ST_Point(0, 0));
-- Expected: f  (boundary point is not "contained" per PostGIS DE-9IM)
-- M22: boundary delta RETIRED — st_contains now matches PostGIS exactly.
SELECT CASE WHEN NOT st_contains(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_point(0.0, 0.0))
THEN 'PASS predicates_contains_boundary' ELSE 'FAIL predicates_contains_boundary' END;

-- PG: SELECT ST_Within(ST_Point(1, 1), ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'));
-- Expected: t
SELECT CASE WHEN st_within(
    st_point(1.0, 1.0),
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))')) = true
THEN 'PASS predicates_within' ELSE 'FAIL predicates_within' END;

-- PG: SELECT ST_Disjoint(ST_Point(0, 0), ST_Point(10, 10));
-- Expected: t
SELECT CASE WHEN st_disjoint(st_point(0.0, 0.0), st_point(10.0, 10.0)) = true
THEN 'PASS predicates_disjoint' ELSE 'FAIL predicates_disjoint' END;

-- PG: SELECT ST_Touches(ST_GeomFromText('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
-- PG:                   ST_GeomFromText('POLYGON((2 0,4 0,4 2,2 2,2 0))'));
-- Expected: t  (share edge (2,0)-(2,2))
SELECT CASE WHEN st_touches(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((2 0,4 0,4 2,2 2,2 0))')) = true
THEN 'PASS predicates_touches' ELSE 'FAIL predicates_touches' END;

-- PG: SELECT ST_Crosses(ST_GeomFromText('LINESTRING(0 0, 4 4)'),
-- PG:                   ST_GeomFromText('LINESTRING(0 4, 4 0)'));
-- Expected: t
SELECT CASE WHEN st_crosses(
    st_geomfromtext('LINESTRING(0 0, 4 4)'),
    st_geomfromtext('LINESTRING(0 4, 4 0)')) = true
THEN 'PASS predicates_crosses' ELSE 'FAIL predicates_crosses' END;

-- PG: SELECT ST_Overlaps(ST_GeomFromText('POLYGON((0 0,3 0,3 3,0 3,0 0))'),
-- PG:                      ST_GeomFromText('POLYGON((2 2,5 2,5 5,2 5,2 2))'));
-- Expected: t  (partial overlap, neither contains the other)
SELECT CASE WHEN st_overlaps(
    st_geomfromtext('POLYGON((0 0,3 0,3 3,0 3,0 0))'),
    st_geomfromtext('POLYGON((2 2,5 2,5 5,2 5,2 2))')) = true
THEN 'PASS predicates_overlaps' ELSE 'FAIL predicates_overlaps' END;

-- PG: SELECT ST_Equals(ST_GeomFromText('LINESTRING(0 0, 1 1)'),
-- PG:                  ST_GeomFromText('LINESTRING(1 1, 0 0)'));
-- Expected: t  (same set of points)
SELECT CASE WHEN st_equals(
    st_geomfromtext('LINESTRING(0 0, 1 1)'),
    st_geomfromtext('LINESTRING(1 1, 0 0)')) = true
THEN 'PASS predicates_equals' ELSE 'FAIL predicates_equals' END;

-- PG: SELECT ST_DWithin(ST_Point(0,0), ST_Point(3,4), 6);
-- Expected: t  (distance = 5 ≤ 6)
SELECT CASE WHEN st_dwithin(st_point(0.0, 0.0), st_point(3.0, 4.0), 6.0) = true
THEN 'PASS predicates_dwithin' ELSE 'FAIL predicates_dwithin' END;

-- PG: SELECT ST_Covers(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
-- PG:                   ST_Point(0, 0));
-- Expected: t  (boundary IS covered, unlike Contains)
SELECT CASE WHEN st_covers(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_point(0.0, 0.0)) = true
THEN 'PASS predicates_covers_boundary' ELSE 'FAIL predicates_covers_boundary' END;
