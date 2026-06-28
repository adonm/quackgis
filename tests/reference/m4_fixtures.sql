-- SPDX-License-Identifier: Apache-2.0
-- Month 4 reference fixtures: fidelity harness for routed st_* == sedona_st_*,
-- ST_Polygon constructor, typed WKT constructor validation, and edge cases.
--
-- Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < tests/reference/m4_fixtures.sql
.bail off
.mode list

-- ======================================================================
-- 1. Parity: routed st_* == literal sedona_st_* (newly routed batch)
-- ======================================================================
-- Every function routed in Month 4 must produce identical output to its
-- literal SedonaDB twin on valid geometry inputs.

-- ST_Point
SELECT CASE WHEN st_astext(st_point(1, 2)) = sedona_st_astext(sedona_st_point(1, 2))
            THEN 'PASS parity st_point' ELSE 'FAIL parity st_point' END;

-- ST_MakePoint (alias)
SELECT CASE WHEN st_astext(st_makepoint(3, 4)) = sedona_st_astext(sedona_st_point(3, 4))
            THEN 'PASS parity st_makepoint' ELSE 'FAIL parity st_makepoint' END;

-- ST_MakeLine
SELECT CASE WHEN st_astext(st_makeline(st_point(0,0), st_point(1,1))) =
                 sedona_st_astext(sedona_st_makeline(st_point(0,0), st_point(1,1)))
            THEN 'PASS parity st_makeline' ELSE 'FAIL parity st_makeline' END;

-- ST_Azimuth
SELECT CASE WHEN abs(st_azimuth(st_point(0,0), st_point(0,1)) -
                     sedona_st_azimuth(st_point(0,0), st_point(0,1))) < 1e-12
            THEN 'PASS parity st_azimuth' ELSE 'FAIL parity st_azimuth' END;

-- ST_Affine (2D scale by 2)
SELECT CASE WHEN st_astext(st_affine(st_point(1,1), 2,0,0,2,0,0)) =
                 sedona_st_astext(sedona_st_affine(st_point(1,1), 2,0,0,2,0,0))
            THEN 'PASS parity st_affine' ELSE 'FAIL parity st_affine' END;

-- ST_Rotate (90 degrees)
SELECT CASE WHEN st_astext(st_rotate(st_geomfromtext('POINT(1 0)'), 1.5707963267948966)) =
                 sedona_st_astext(sedona_st_rotate(st_geomfromtext('POINT(1 0)'), 1.5707963267948966))
            THEN 'PASS parity st_rotate' ELSE 'FAIL parity st_rotate' END;

-- ST_Translate
SELECT CASE WHEN st_astext(st_translate(st_point(0,0), 5, 5)) =
                 sedona_st_astext(sedona_st_translate(st_point(0,0), 5, 5))
            THEN 'PASS parity st_translate' ELSE 'FAIL parity st_translate' END;

-- ST_Scale
SELECT CASE WHEN st_astext(st_scale(st_point(2,3), 2, 2)) =
                 sedona_st_astext(sedona_st_scale(st_point(2,3), 2, 2))
            THEN 'PASS parity st_scale' ELSE 'FAIL parity st_scale' END;

-- ST_LineFromText (valid)
SELECT CASE WHEN st_astext(st_linefromtext('LINESTRING(0 0,1 1)')) =
                 sedona_st_astext(sedona_st_linefromtext('LINESTRING(0 0,1 1)'))
            THEN 'PASS parity st_linefromtext' ELSE 'FAIL parity st_linefromtext' END;

-- ST_PointFromText (valid)
SELECT CASE WHEN st_astext(st_pointfromtext('POINT(1 2)')) =
                 sedona_st_astext(sedona_st_pointfromtext('POINT(1 2)'))
            THEN 'PASS parity st_pointfromtext' ELSE 'FAIL parity st_pointfromtext' END;

-- ST_PolygonFromText (valid)
SELECT CASE WHEN st_astext(st_polygonfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))')) =
                 sedona_st_astext(sedona_st_polygonfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'))
            THEN 'PASS parity st_polygonfromtext' ELSE 'FAIL parity st_polygonfromtext' END;

-- ======================================================================
-- 2. ST_Polygon constructor
-- ======================================================================

-- Valid: closed linestring → polygon
SELECT CASE WHEN st_astext(st_polygon(st_geomfromtext('LINESTRING(0 0,4 0,4 4,0 4,0 0)'), 4326))
                 = 'POLYGON((0 0,4 0,4 4,0 4,0 0))'
            THEN 'PASS st_polygon valid' ELSE 'FAIL st_polygon valid' END;

-- Area of constructed polygon
SELECT CASE WHEN abs(st_area(st_polygon(st_geomfromtext('LINESTRING(0 0,2 0,2 2,0 2,0 0)'), 0)) - 4.0) < 1e-9
            THEN 'PASS st_polygon area' ELSE 'FAIL st_polygon area' END;

-- Unclosed linestring → NULL
SELECT CASE WHEN st_polygon(st_geomfromtext('LINESTRING(0 0,1 0,1 1)'), 0) IS NULL
            THEN 'PASS st_polygon unclosed null' ELSE 'FAIL st_polygon unclosed null' END;

-- Non-linestring input → NULL
SELECT CASE WHEN st_polygon(st_geomfromtext('POINT(1 2)'), 0) IS NULL
            THEN 'PASS st_polygon non_line null' ELSE 'FAIL st_polygon non_line null' END;

-- NULL input → NULL
SELECT CASE WHEN st_polygon(NULL, 4326) IS NULL
            THEN 'PASS st_polygon null input' ELSE 'FAIL st_polygon null input' END;

-- ======================================================================
-- 3. Typed WKT constructor validation (PostGIS fidelity)
-- ======================================================================
-- Routed to SedonaDB typed kernels: mismatched WKT type returns NULL.

-- ST_LineFromText with POINT WKT → NULL
SELECT CASE WHEN st_linefromtext('POINT(1 2)') IS NULL
            THEN 'PASS linefromtext type reject' ELSE 'FAIL linefromtext type reject' END;

-- ST_PointFromText with LINESTRING WKT → NULL
SELECT CASE WHEN st_pointfromtext('LINESTRING(0 0,1 1)') IS NULL
            THEN 'PASS pointfromtext type reject' ELSE 'FAIL pointfromtext type reject' END;

-- ST_PolygonFromText with POINT WKT → NULL
SELECT CASE WHEN st_polygonfromtext('POINT(1 2)') IS NULL
            THEN 'PASS polygonfromtext type reject' ELSE 'FAIL polygonfromtext type reject' END;

-- Valid typed constructors still work
SELECT CASE WHEN st_geometrytype(st_linefromtext('LINESTRING(0 0,1 1)')) = 'ST_LineString'
            THEN 'PASS linefromtext valid type' ELSE 'FAIL linefromtext valid type' END;

SELECT CASE WHEN st_geometrytype(st_pointfromtext('POINT(1 2)')) = 'ST_Point'
            THEN 'PASS pointfromtext valid type' ELSE 'FAIL pointfromtext valid type' END;

SELECT CASE WHEN st_geometrytype(st_polygonfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))')) = 'ST_Polygon'
            THEN 'PASS polygonfromtext valid type' ELSE 'FAIL polygonfromtext valid type' END;

-- ======================================================================
-- 4. NULL propagation for routed functions
-- ======================================================================

SELECT CASE WHEN st_point(NULL, NULL) IS NULL
            THEN 'PASS null st_point' ELSE 'FAIL null st_point' END;

SELECT CASE WHEN st_makeline(NULL, st_point(1,1)) IS NULL
            THEN 'PASS null st_makeline' ELSE 'FAIL null st_makeline' END;

SELECT CASE WHEN st_azimuth(NULL, st_point(0,1)) IS NULL
            THEN 'PASS null st_azimuth' ELSE 'FAIL null st_azimuth' END;

SELECT CASE WHEN st_rotate(NULL, 1.0) IS NULL
            THEN 'PASS null st_rotate' ELSE 'FAIL null st_rotate' END;

SELECT CASE WHEN st_translate(NULL, 1.0, 2.0) IS NULL
            THEN 'PASS null st_translate' ELSE 'FAIL null st_translate' END;

SELECT CASE WHEN st_affine(NULL, 1.0,0.0,0.0,1.0,0.0,0.0) IS NULL
            THEN 'PASS null st_affine' ELSE 'FAIL null st_affine' END;

-- ======================================================================
-- 5. Routed functions on collections and Z-dimension geometry
-- ======================================================================

-- ST_Affine on a MultiPoint
SELECT CASE WHEN abs(st_area(st_envelope(st_affine(
                 st_geomfromtext('MULTIPOINT((0 0),(1 1),(2 2))'),
                 2,0,0,2,0,0))) - 16.0) < 1e-9
            THEN 'PASS affine multipoint' ELSE 'FAIL affine multipoint' END;

-- ST_Translate on a LineString
SELECT CASE WHEN st_astext(st_translate(
                 st_geomfromtext('LINESTRING(0 0,1 1)'), 10, 20))
                 = 'LINESTRING(10 20,11 21)'
            THEN 'PASS translate linestring' ELSE 'FAIL translate linestring' END;

-- ST_Scale on a Polygon
SELECT CASE WHEN abs(st_area(st_scale(
                 st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'),
                 3, 3)) - 9.0) < 1e-9
            THEN 'PASS scale polygon' ELSE 'FAIL scale polygon' END;

-- ======================================================================
-- 6. Routed transforms: functional correctness vs known values
-- ======================================================================

-- ST_Rotate 90° of (1,0) about origin → (0,1)
SELECT CASE WHEN abs(st_x(st_rotate(st_geomfromtext('POINT(1 0)'), 1.5707963267948966))) < 1e-9
            AND abs(st_y(st_rotate(st_geomfromtext('POINT(1 0)'), 1.5707963267948966)) - 1.0) < 1e-9
            THEN 'PASS rotate 90deg' ELSE 'FAIL rotate 90deg' END;

-- ST_Azimuth east → π/2
SELECT CASE WHEN abs(st_azimuth(st_point(0,0), st_point(1,0)) - 1.5707963267948966) < 1e-12
            THEN 'PASS azimuth east' ELSE 'FAIL azimuth east' END;

-- ST_Azimuth north → 0
SELECT CASE WHEN abs(st_azimuth(st_point(0,0), st_point(0,1))) < 1e-12
            THEN 'PASS azimuth north' ELSE 'FAIL azimuth north' END;

-- ST_Affine identity
SELECT CASE WHEN st_astext(st_affine(st_point(3,4), 1,0,0,1,0,0)) = 'POINT(3 4)'
            THEN 'PASS affine identity' ELSE 'FAIL affine identity' END;

-- ST_Affine translation only
SELECT CASE WHEN st_astext(st_affine(st_point(0,0), 1,0,0,1,5,10)) = 'POINT(5 10)'
            THEN 'PASS affine translate' ELSE 'FAIL affine translate' END;
