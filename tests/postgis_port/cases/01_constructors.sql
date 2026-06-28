.mode list
-- Port family: constructors
-- Tests that common PostGIS constructor SQL ports directly.

-- PG: SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'));
-- Expected: POINT(1 2)
-- Rewrite: none (identical namespace)
SELECT CASE WHEN st_astext(st_geomfromtext('POINT(1 2)')) = 'POINT(1 2)'
THEN 'PASS constructors_geomfromtext' ELSE 'FAIL constructors_geomfromtext' END;

-- PG: SELECT ST_AsText(ST_Point(3, 4));
-- Expected: POINT(3 4)
SELECT CASE WHEN st_astext(st_point(3.0, 4.0)) = 'POINT(3 4)'
THEN 'PASS constructors_point' ELSE 'FAIL constructors_point' END;

-- PG: SELECT ST_AsText(ST_MakeEnvelope(0, 0, 2, 2));
-- Expected: POLYGON((0 0,2 0,2 2,0 2,0 0))
SELECT CASE WHEN st_astext(st_makeenvelope(0, 0, 2, 2)) LIKE 'POLYGON((0 0,2 0,2 2,0 2,0 0))'
THEN 'PASS constructors_makeenvelope' ELSE 'FAIL constructors_makeenvelope' END;

-- PG: SELECT ST_AsText(ST_MakeLine(ST_Point(0,0), ST_Point(1,1)));
-- Expected: LINESTRING(0 0,1 1)
SELECT CASE WHEN st_astext(st_makeline(st_point(0.0, 0.0), st_point(1.0, 1.0))) LIKE 'LINESTRING(0 0,1 1)'
THEN 'PASS constructors_makeline' ELSE 'FAIL constructors_makeline' END;

-- PG: SELECT ST_AsText(ST_MakePolygon(ST_GeomFromText('LINESTRING(0 0,1 0,1 1,0 0)')));
-- Expected: POLYGON((0 0,1 0,1 1,0 0))
SELECT CASE WHEN st_astext(st_makepolygon(st_geomfromtext('LINESTRING(0 0,1 0,1 1,0 0)'))) LIKE 'POLYGON((0 0,1 0,1 1,0 0))'
THEN 'PASS constructors_makepolygon' ELSE 'FAIL constructors_makepolygon' END;

-- PG: SELECT ST_AsText(ST_Buffer(ST_Point(0,0), 1);
-- Expected: POLYGON(...)  — area ≈ π (3.14159…)
-- Rewrite: none
SELECT CASE WHEN abs(st_area(st_buffer(st_point(0.0, 0.0), 1.0)) - 3.14159) < 0.05
THEN 'PASS constructors_buffer_area' ELSE 'FAIL constructors_buffer_area' END;

-- PG: SELECT ST_GeometryType(ST_GeomFromText('LINESTRING(0 0, 1 1)'));
-- Expected: ST_LineString
SELECT CASE WHEN st_geometrytype(st_geomfromtext('LINESTRING(0 0, 1 1)')) = 'ST_LineString'
THEN 'PASS constructors_geometrytype' ELSE 'FAIL constructors_geometrytype' END;

-- PG: SELECT ST_AsText(ST_GeomFromEWKT('SRID=4326;POINT(1 2)'));
-- Expected: POINT(1 2)  (SRID not in WKB)
SELECT CASE WHEN st_astext(st_geomfromewkt('SRID=4326;POINT(1 2)')) = 'POINT(1 2)'
THEN 'PASS constructors_ewkt' ELSE 'FAIL constructors_ewkt' END;
