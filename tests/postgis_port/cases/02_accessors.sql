.mode list
-- Port family: accessors
-- Tests that common PostGIS accessor SQL ports directly.

-- PG: SELECT ST_NumPoints(ST_GeomFromText('LINESTRING(0 0, 1 1, 2 2)'));
-- Expected: 3
SELECT CASE WHEN st_numpoints(st_geomfromtext('LINESTRING(0 0, 1 1, 2 2)')) = 3
THEN 'PASS accessors_numpoints' ELSE 'FAIL accessors_numpoints' END;

-- PG: SELECT ST_X(ST_Point(1.5, 2.5));
-- Expected: 1.5
SELECT CASE WHEN st_x(st_point(1.5, 2.5)) = 1.5
THEN 'PASS accessors_x' ELSE 'FAIL accessors_x' END;

-- PG: SELECT ST_Y(ST_Point(1.5, 2.5));
-- Expected: 2.5
SELECT CASE WHEN st_y(st_point(1.5, 2.5)) = 2.5
THEN 'PASS accessors_y' ELSE 'FAIL accessors_y' END;

-- PG: SELECT ST_Area(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'));
-- Expected: 16
SELECT CASE WHEN st_area(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))')) = 16.0
THEN 'PASS accessors_area' ELSE 'FAIL accessors_area' END;

-- PG: SELECT ST_Length(ST_GeomFromText('LINESTRING(0 0, 3 4)'));
-- Expected: 5
SELECT CASE WHEN st_length(st_geomfromtext('LINESTRING(0 0, 3 4)')) = 5.0
THEN 'PASS accessors_length' ELSE 'FAIL accessors_length' END;

-- PG: SELECT ST_Perimeter(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))'));
-- Expected: 16
SELECT CASE WHEN st_perimeter(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))')) = 16.0
THEN 'PASS accessors_perimeter' ELSE 'FAIL accessors_perimeter' END;

-- PG: SELECT ST_Dimension(ST_GeomFromText('POLYGON((0 0,1 0,1 1,0 0))'));
-- Expected: 2
SELECT CASE WHEN st_dimension(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))')) = 2
THEN 'PASS accessors_dimension' ELSE 'FAIL accessors_dimension' END;

-- PG: SELECT ST_IsEmpty(ST_GeomFromText('POINT EMPTY'));
-- Expected: t
SELECT CASE WHEN st_isempty(st_geomfromtext('POINT EMPTY')) = true
THEN 'PASS accessors_isempty' ELSE 'FAIL accessors_isempty' END;

-- PG: SELECT ST_NumInteriorRings(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))'));
-- Expected: 1
SELECT CASE WHEN st_numinteriorrings(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))')) = 1
THEN 'PASS accessors_numinteriorrings' ELSE 'FAIL accessors_numinteriorrings' END;

-- PG: SELECT ST_XMin(ST_GeomFromText('POLYGON((1 2,5 2,5 6,1 6,1 2))'));
-- Expected: 1
SELECT CASE WHEN st_xmin(st_geomfromtext('POLYGON((1 2,5 2,5 6,1 6,1 2))')) = 1.0
THEN 'PASS accessors_xmin' ELSE 'FAIL accessors_xmin' END;

-- PG: SELECT ST_XMax(ST_GeomFromText('POLYGON((1 2,5 2,5 6,1 6,1 2))'));
-- Expected: 5
SELECT CASE WHEN st_xmax(st_geomfromtext('POLYGON((1 2,5 2,5 6,1 6,1 2))')) = 5.0
THEN 'PASS accessors_xmax' ELSE 'FAIL accessors_xmax' END;
