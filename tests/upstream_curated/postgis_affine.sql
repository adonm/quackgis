-- postgis_affine.sql - curated from PostGIS regress/core/affine.sql
-- Tests ST_Translate, ST_Scale, ST_Rotate, ST_Affine, ST_SnapToGrid.
-- Source: regress/core/affine.sql
.mode list

-- ======================================================================
-- ST_Translate - shift geometry by (dx, dy)
-- ======================================================================
SELECT CASE WHEN st_astext(st_translate(st_point(2, 3), 3, 9)) = 'POINT(5 12)' THEN 'PASS translate point' ELSE 'FAIL translate point' END;
SELECT CASE WHEN st_astext(st_translate(st_geomfromtext('LINESTRING(0 0,1 1)'), 5, 10)) = 'LINESTRING(5 10,6 11)' THEN 'PASS translate line' ELSE 'FAIL translate line' END;
-- Translate preserves SRID
SELECT CASE WHEN st_srid(st_translate(st_setsrid(st_point(0,0), 4326), 1, 2)) = 4326 THEN 'PASS translate srid' ELSE 'FAIL translate srid' END;
-- ======================================================================
-- ST_Scale - scale geometry by (sx, sy)
-- ======================================================================
SELECT CASE WHEN st_astext(st_scale(st_point(1, 1), 5, 5)) = 'POINT(5 5)' THEN 'PASS scale point' ELSE 'FAIL scale point' END;
SELECT CASE WHEN st_astext(st_scale(st_point(3, 2), 1, 1)) = 'POINT(3 2)' THEN 'PASS scale identity' ELSE 'FAIL scale identity' END;
SELECT CASE WHEN st_astext(st_scale(st_geomfromtext('LINESTRING(0 0,1 1)'), 2, 3)) = 'LINESTRING(0 0,2 3)' THEN 'PASS scale line' ELSE 'FAIL scale line' END;
-- ======================================================================
-- ST_Rotate - rotate around origin (2D)
-- ======================================================================
-- Rotate POINT(1,0) by pi/2 → POINT(0,1)
SELECT CASE WHEN abs(st_x(st_rotate(st_point(1, 0), 3.141592653589793/2))) < 1e-12 AND abs(st_y(st_rotate(st_point(1, 0), 3.141592653589793/2)) - 1.0) < 1e-12 THEN 'PASS rotate 90' ELSE 'FAIL rotate 90' END;
-- Rotate POINT(1,0) by pi → POINT(-1,0)
SELECT CASE WHEN abs(st_x(st_rotate(st_point(1, 0), 3.141592653589793)) + 1.0) < 1e-12 AND abs(st_y(st_rotate(st_point(1, 0), 3.141592653589793))) < 1e-12 THEN 'PASS rotate 180' ELSE 'FAIL rotate 180' END;
-- Rotate by 0 = identity
SELECT CASE WHEN st_astext(st_rotate(st_point(1, 2), 0)) = 'POINT(1 2)' THEN 'PASS rotate zero' ELSE 'FAIL rotate zero' END;
-- Rotate preserves SRID
SELECT CASE WHEN st_srid(st_rotate(st_setsrid(st_point(1,0), 4326), 1.0)) = 4326 THEN 'PASS rotate srid' ELSE 'FAIL rotate srid' END;
-- ======================================================================
-- ST_Affine - general 2D affine: x' = a*x+b*y+xoff, y' = d*x+e*y+yoff
-- ======================================================================
-- Identity affine (a=1, b=0, d=0, e=1, xoff=0, yoff=0)
SELECT CASE WHEN st_astext(st_affine(st_point(3, 4), 1, 0, 0, 1, 0, 0)) = 'POINT(3 4)' THEN 'PASS affine identity' ELSE 'FAIL affine identity' END;
-- Translation via affine (a=1, b=0, d=0, e=1, xoff=5, yoff=10)
SELECT CASE WHEN st_astext(st_affine(st_point(0, 0), 1, 0, 0, 1, 5, 10)) = 'POINT(5 10)' THEN 'PASS affine translate' ELSE 'FAIL affine translate' END;
-- Scale via affine (a=2, b=0, d=0, e=3, xoff=0, yoff=0)
SELECT CASE WHEN st_astext(st_affine(st_point(1, 1), 2, 0, 0, 3, 0, 0)) = 'POINT(2 3)' THEN 'PASS affine scale' ELSE 'FAIL affine scale' END;
-- ======================================================================
-- ST_SnapToGrid - round coordinates to grid
-- ======================================================================
SELECT CASE WHEN st_astext(st_snaptogrid(st_point(1.11111, 2.22222), 0.1)) = 'POINT(1.1 2.2)' THEN 'PASS snaptogrid 0.1' ELSE 'FAIL snaptogrid 0.1' END;
SELECT CASE WHEN st_astext(st_snaptogrid(st_point(1.11111, 2.22222), 1.0)) = 'POINT(1 2)' THEN 'PASS snaptogrid 1' ELSE 'FAIL snaptogrid 1' END;
-- SnapToGrid preserves SRID
SELECT CASE WHEN st_srid(st_snaptogrid(st_setsrid(st_point(1.1, 2.2), 4326), 1)) = 4326 THEN 'PASS snaptogrid srid' ELSE 'FAIL snaptogrid srid' END;
-- ======================================================================
