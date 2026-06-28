-- SPDX-License-Identifier: Apache-2.0
-- Month 2 reference fixtures: high-fidelity capability tests for ST_Subdivide,
-- ST_IntersectionAgg, ST_Value, raster clipping, and spheroid documentation.
--
-- Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < tests/reference/m2_fixtures.sql
.bail off
.mode list

-- ======================================================================
-- 1. ST_Subdivide edge cases
-- ======================================================================

-- Point (already small) → returned unchanged.
SELECT CASE WHEN st_geometrytype(st_subdivide(st_geomfromtext('POINT(1 2)'), 255)) = 'ST_Point'
            THEN 'PASS subdivide point passthrough' ELSE 'FAIL subdivide point passthrough' END;

-- LineString with 4 points, max_vertices=2 → GeometryCollection of 2 lines.
SELECT CASE WHEN st_numgeometries(st_subdivide(st_geomfromtext('LINESTRING(0 0,1 1,2 2,3 3)'), 2)) = 2
            THEN 'PASS subdivide line split' ELSE 'FAIL subdivide line split' END;

-- LineString with 4 points, max_vertices=255 → returned unchanged.
SELECT CASE WHEN st_geometrytype(st_subdivide(st_geomfromtext('LINESTRING(0 0,1 1,2 2,3 3)'), 255)) = 'ST_LineString'
            THEN 'PASS subdivide line passthrough' ELSE 'FAIL subdivide line passthrough' END;

-- Polygon subdivision: large polygon should produce a GeometryCollection.
SELECT CASE WHEN st_geometrytype(st_subdivide(
                st_geomfromtext('POLYGON((0 0,100 0,100 100,0 100,0 0),
                              (1 1,2 1,2 2,1 2,1 1),(5 5,6 5,6 6,5 6,5 5),
                              (10 10,11 10,11 11,10 11,10 10))'),
                8)) = 'ST_GeometryCollection'
            THEN 'PASS subdivide polygon' ELSE 'FAIL subdivide polygon' END;

-- Empty geometry → unchanged.
SELECT CASE WHEN st_isempty(st_subdivide(st_geomfromtext('POINT EMPTY'), 10))
            THEN 'PASS subdivide empty' ELSE 'FAIL subdivide empty' END;

-- ======================================================================
-- 2. ST_IntersectionAgg edge cases
-- ======================================================================

-- Two overlapping polygons → non-empty intersection.
SELECT CASE WHEN st_area(st_intersection_agg(g)) > 0
            THEN 'PASS intersection_agg overlap' ELSE 'FAIL intersection_agg overlap' END
FROM (SELECT st_geomfromtext(c) AS g FROM (VALUES
    ('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    ('POLYGON((2 0,6 0,6 4,2 4,2 0))')
) AS t(c));

-- Two disjoint polygons → NULL (PostGIS semantics: disjoint → empty → NULL).
SELECT CASE WHEN st_intersection_agg(g) IS NULL
            THEN 'PASS intersection_agg disjoint null' ELSE 'FAIL intersection_agg disjoint null' END
FROM (SELECT st_geomfromtext(c) AS g FROM (VALUES
    ('POLYGON((0 0,1 0,1 1,0 1,0 0))'),
    ('POLYGON((10 10,11 10,11 11,10 11,10 10))')
) AS t(c));

-- Empty input table → NULL.
SELECT CASE WHEN st_intersection_agg(g) IS NULL
            THEN 'PASS intersection_agg empty table' ELSE 'FAIL intersection_agg empty table' END
FROM (SELECT st_geomfromtext('POINT EMPTY') AS g WHERE 1=0) AS sub;

-- Three overlapping polygons → cascaded intersection still valid.
SELECT CASE WHEN st_isvalid(st_intersection_agg(g))
            THEN 'PASS intersection_agg three valid' ELSE 'FAIL intersection_agg three valid' END
FROM (SELECT st_geomfromtext(c) AS g FROM (VALUES
    ('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    ('POLYGON((2 0,6 0,6 4,2 4,2 0))'),
    ('POLYGON((1 1,5 1,5 3,1 3,1 1))')
) AS t(c));

-- ======================================================================
-- 3. ST_Value — point sampling
-- ======================================================================
-- Test raster: 4×3 pixels, origin (0,3), pixel size 1×-1.
-- Forward: x = col, y = 3 - row.
-- Pixel values: row-major 1..12 (nodata=-9999).

-- Pixel (0,0) = value 1 at geographic (0.5, 2.5).
SELECT CASE WHEN st_value('tests/data/test_raster.asc', 1, 0.5, 2.5) = 1.0
            THEN 'PASS st_value pixel 0,0' ELSE 'FAIL st_value pixel 0,0' END;

-- Pixel (3,2) = value 12 at geographic (3.5, 0.5).
SELECT CASE WHEN st_value('tests/data/test_raster.asc', 1, 3.5, 0.5) = 12.0
            THEN 'PASS st_value pixel 3,2' ELSE 'FAIL st_value pixel 3,2' END;

-- Pixel (1,1) = value 6 at geographic (1.5, 1.5).
SELECT CASE WHEN st_value('tests/data/test_raster.asc', 1, 1.5, 1.5) = 6.0
            THEN 'PASS st_value pixel 1,1' ELSE 'FAIL st_value pixel 1,1' END;

-- Out-of-bounds (x < 0) → NULL.
SELECT CASE WHEN st_value('tests/data/test_raster.asc', 1, -1.0, 2.5) IS NULL
            THEN 'PASS st_value out of bounds' ELSE 'FAIL st_value out of bounds' END;

-- Out-of-bounds (y > 3) → NULL.
SELECT CASE WHEN st_value('tests/data/test_raster.asc', 1, 0.5, 5.0) IS NULL
            THEN 'PASS st_value y out of bounds' ELSE 'FAIL st_value y out of bounds' END;

-- NULL propagation.
SELECT CASE WHEN st_value(NULL, 1, 0.5, 2.5) IS NULL
            THEN 'PASS st_value null path' ELSE 'FAIL st_value null path' END;

-- ======================================================================
-- 4. Raster clipping workflow (DuckDB-native)
-- ======================================================================
-- Select pixels within a bounding box using st_pixeldata + st_raster_transform.
-- This is the DuckDB-native equivalent of ST_Clip.

-- All pixels within the box x∈[1,3], y∈[0,2] should be a subset of 12 pixels.
WITH transform AS (
    SELECT origin_x, origin_y, pixel_w, pixel_h, row_rot, col_rot
    FROM st_raster_transform('tests/data/test_raster.asc')
),
pixels AS (
    SELECT p.row, p.col, p.value,
           t.origin_x + p.col * t.pixel_w + p.row * t.row_rot AS x,
           t.origin_y + p.col * t.col_rot + p.row * t.pixel_h AS y
    FROM st_pixeldata('tests/data/test_raster.asc', 1) p
    CROSS JOIN transform t
)
SELECT CASE WHEN (SELECT count(*) FROM pixels WHERE x BETWEEN 1 AND 3 AND y BETWEEN 0 AND 2 AND value IS NOT NULL) = 6
            THEN 'PASS raster clip pixel count' ELSE 'FAIL raster clip pixel count'
            || ': got ' || (SELECT count(*) FROM pixels WHERE x BETWEEN 1 AND 3 AND y BETWEEN 0 AND 2 AND value IS NOT NULL) END;

-- The clipped pixels should have values in [2, 11].
WITH transform AS (
    SELECT origin_x, origin_y, pixel_w, pixel_h, row_rot, col_rot
    FROM st_raster_transform('tests/data/test_raster.asc')
),
clipped AS (
    SELECT p.value
    FROM st_pixeldata('tests/data/test_raster.asc', 1) p
    CROSS JOIN transform t
    WHERE t.origin_x + p.col * t.pixel_w + p.row * t.row_rot BETWEEN 1 AND 3
      AND t.origin_y + p.col * t.col_rot + p.row * t.pixel_h BETWEEN 0 AND 2
      AND p.value IS NOT NULL
)
SELECT CASE WHEN (SELECT min(value) FROM clipped) >= 6.0
              AND (SELECT max(value) FROM clipped) <= 12.0
            THEN 'PASS raster clip value range' ELSE 'FAIL raster clip value range' END;

-- ======================================================================
-- 5. Spheroid geodesic stability (antipodal-safe)
-- ======================================================================
-- Antipodal points: distance should be approximately half the Earth's
-- circumference (~20003.93 km via WGS84 spheroid).
SELECT CASE WHEN abs(st_distancespheroid(st_point(0, 0), st_point(180, 0)) - 20003931.4586) < 100.0
            THEN 'PASS spheroid antipodal' ELSE 'FAIL spheroid antipodal' END;

-- London → Paris spheroid distance should be ~343 km.
SELECT CASE WHEN abs(st_distancespheroid(st_point(-0.1278, 51.5074), st_point(2.3522, 48.8566)) - 343483.0) < 500.0
            THEN 'PASS spheroid london paris' ELSE 'FAIL spheroid london paris' END;

-- Spheroid vs sphere: spheroid should be slightly different (WGS84 ellipsoid).
SELECT CASE WHEN abs(st_distancespheroid(st_point(0, 0), st_point(1, 0))
                    - st_distancesphere(st_point(0, 0), st_point(1, 0))) < 1000.0
              AND abs(st_distancespheroid(st_point(0, 0), st_point(1, 0))
                     - st_distancesphere(st_point(0, 0), st_point(1, 0))) > 0.0
            THEN 'PASS spheroid vs sphere distinct' ELSE 'FAIL spheroid vs sphere distinct' END;

-- ======================================================================
-- 6. Raster map algebra — reclassification via st_pixeldata
-- ======================================================================
SELECT CASE WHEN (SELECT count(*) FROM (
    SELECT CASE WHEN value > 8 THEN 'high'
                WHEN value > 4 THEN 'mid'
                ELSE 'low' END AS class
    FROM st_pixeldata('tests/data/test_raster.asc', 1)
    WHERE value IS NOT NULL
) WHERE class = 'high') = 4  -- values 9,10,11,12
            THEN 'PASS reclass high count' ELSE 'FAIL reclass high count' END;
