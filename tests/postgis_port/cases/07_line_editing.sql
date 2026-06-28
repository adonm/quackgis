.mode list
-- Port family: line editing and linear referencing
-- Tests that PostGIS line-editing SQL ports directly.

-- PG: SELECT ST_AsText(ST_LineSubstring(ST_GeomFromText('LINESTRING(0 0, 10 0)'), 0.25, 0.75));
-- Expected: LINESTRING(2.5 0,7.5 0)
SELECT CASE WHEN st_astext(st_linesubstring(
    st_geomfromtext('LINESTRING(0 0, 10 0)'), 0.25, 0.75)) LIKE 'LINESTRING(2.5 0,7.5 0)'
THEN 'PASS line_linesubstring' ELSE 'FAIL line_linesubstring' END;

-- PG: SELECT ST_AsText(ST_LineInterpolatePoint(ST_GeomFromText('LINESTRING(0 0, 10 0)'), 0.5));
-- Expected: POINT(5 0)
SELECT CASE WHEN st_astext(st_lineinterpolatepoint(
    st_geomfromtext('LINESTRING(0 0, 10 0)'), 0.5)) LIKE 'POINT(5 0)'
THEN 'PASS line_interpolate' ELSE 'FAIL line_interpolate' END;

-- PG: SELECT ST_LineLocatePoint(ST_GeomFromText('LINESTRING(0 0, 10 0)'), ST_Point(3, 0));
-- Expected: 0.3
SELECT CASE WHEN abs(st_linelocatepoint(
    st_geomfromtext('LINESTRING(0 0, 10 0)'), st_point(3.0, 0.0)) - 0.3) < 0.001
THEN 'PASS line_locate' ELSE 'FAIL line_locate' END;

-- PG: SELECT ST_AsText(ST_SetPoint(ST_GeomFromText('LINESTRING(0 0, 1 1, 2 2)'), 0, ST_Point(5 5)));
-- Expected: LINESTRING(5 5,1 1,2 2)
SELECT CASE WHEN st_astext(st_setpoint(
    st_geomfromtext('LINESTRING(0 0, 1 1, 2 2)'), 0, st_point(5.0, 5.0))) LIKE 'LINESTRING(5 5,1 1,2 2)'
THEN 'PASS line_setpoint' ELSE 'FAIL line_setpoint' END;

-- PG: SELECT ST_AsText(ST_AddPoint(ST_GeomFromText('LINESTRING(0 0, 1 1)'), ST_Point(2 2)));
-- Expected: LINESTRING(0 0,1 1,2 2)
SELECT CASE WHEN st_astext(st_addpoint(
    st_geomfromtext('LINESTRING(0 0, 1 1)'), st_point(2.0, 2.0))) LIKE 'LINESTRING(0 0,1 1,2 2)'
THEN 'PASS line_addpoint' ELSE 'FAIL line_addpoint' END;

-- PG: SELECT ST_AsText(ST_RemovePoint(ST_GeomFromText('LINESTRING(0 0, 1 1, 2 2)'), 1));
-- Expected: LINESTRING(0 0,2 2)
SELECT CASE WHEN st_astext(st_removepoint(
    st_geomfromtext('LINESTRING(0 0, 1 1, 2 2)'), 1)) LIKE 'LINESTRING(0 0,2 2)'
THEN 'PASS line_removepoint' ELSE 'FAIL line_removepoint' END;

-- PG: SELECT ST_AsText(ST_Translate(ST_Point(1, 2), 3, 4));
-- Expected: POINT(4 6)
SELECT CASE WHEN st_astext(st_translate(st_point(1.0, 2.0), 3.0, 4.0)) LIKE 'POINT(4 6)'
THEN 'PASS line_translate' ELSE 'FAIL line_translate' END;

-- PG: SELECT ST_AsText(ST_Simplify(ST_GeomFromText('LINESTRING(0 0, 1 0.01, 2 0)'), 0.1));
-- Expected: LINESTRING(0 0,2 0)  (middle point removed by RDP)
SELECT CASE WHEN st_astext(st_simplify(
    st_geomfromtext('LINESTRING(0 0, 1 0.01, 2 0)'), 0.1)) LIKE 'LINESTRING(0 0,2 0)'
THEN 'PASS line_simplify' ELSE 'FAIL line_simplify' END;
