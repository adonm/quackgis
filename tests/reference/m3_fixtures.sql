-- SPDX-License-Identifier: Apache-2.0
-- Month 3 reference fixtures: spatial join ergonomics, table functions,
-- aggregate completeness, and maintenance verification (routed == literal).
--
-- Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < tests/reference/m3_fixtures.sql
.bail off
.mode list

-- ======================================================================
-- 1. Spatial join: bbox prefilter + exact predicate
-- ======================================================================
-- Verify the canonical bbox-prefilter join pattern produces correct results.
WITH a(id, geom) AS (
    SELECT * FROM (VALUES
        (1, st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))')),
        (2, st_geomfromtext('POLYGON((10 10,12 10,12 12,10 12,10 10))')),
        (3, st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'))
    )
),
b(id, geom) AS (
    SELECT * FROM (VALUES
        (10, st_geomfromtext('POLYGON((1 1,2 1,2 2,1 2,1 1))')),
        (20, st_geomfromtext('POLYGON((11 11,13 11,13 13,11 13,11 11))'))
    )
),
a_bbox AS (
    SELECT *, st_xmin(geom) AS ax_min, st_xmax(geom) AS ax_max,
              st_ymin(geom) AS ay_min, st_ymax(geom) AS ay_max FROM a
),
b_bbox AS (
    SELECT *, st_xmin(geom) AS bx_min, st_xmax(geom) AS bx_max,
              st_ymin(geom) AS by_min, st_ymax(geom) AS by_max FROM b
)
SELECT CASE WHEN (
    SELECT count(*) FROM a_bbox JOIN b_bbox
      ON ax_min <= bx_max AND ax_max >= bx_min
     AND ay_min <= by_max AND ay_max >= by_min
    WHERE st_intersects(a_bbox.geom, b_bbox.geom)
) = 3  -- a1↔b10, a2↔b20, a3↔b10
            THEN 'PASS bbox join count' ELSE 'FAIL bbox join count' END;

-- ======================================================================
-- 2. Table functions: ST_Dump, ST_DumpPoints, ST_DumpSegments
-- ======================================================================

-- ST_Dump: multi-geometry → one row per atomic geometry.
SELECT CASE WHEN (SELECT count(*) FROM st_dump(
    st_geomfromtext('MULTIPOINT(1 2,3 4,5 6)'))) = 3
            THEN 'PASS dump multipoint' ELSE 'FAIL dump multipoint' END;

-- ST_DumpPoints: polygon → one row per vertex.
SELECT CASE WHEN (SELECT count(*) FROM st_dumppoints(
    st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'))) = 5
            THEN 'PASS dumppoints polygon' ELSE 'FAIL dumppoints polygon' END;

-- ST_DumpSegments: linestring → one row per edge.
SELECT CASE WHEN (SELECT count(*) FROM st_dumpsegments(
    st_geomfromtext('LINESTRING(0 0,1 1,2 2,3 3)'))) = 3
            THEN 'PASS dumpsegments line' ELSE 'FAIL dumpsegments line' END;

-- ======================================================================
-- 3. Aggregates: collect, union_agg, envelope_agg, makeline_agg, intersection_agg
-- ======================================================================

-- ST_Collect: 3 points → GeometryCollection of 3.
SELECT CASE WHEN st_numgeometries(st_collect(g)) = 3
            THEN 'PASS collect 3 points' ELSE 'FAIL collect 3 points' END
FROM (SELECT st_point(i, i) AS g FROM range(0, 3) t(i));

-- ST_Union_Agg: 4 adjacent squares → one polygon.
SELECT CASE WHEN st_area(st_union_agg(g)) = 4.0
            THEN 'PASS union_agg 4 squares' ELSE 'FAIL union_agg 4 squares' END
FROM (SELECT st_makeenvelope(i, 0, i+1, 1) AS g FROM range(0, 4) t(i));

-- ST_Envelope_Agg: 3 points at (0,0),(2,2),(4,4) → bbox area = 16.
SELECT CASE WHEN abs(st_area(st_envelope_agg(g)) - 16.0) < 1e-9
            THEN 'PASS envelope_agg 3 points' ELSE 'FAIL envelope_agg 3 points' END
FROM (SELECT st_point(i*2, i*2) AS g FROM range(0, 3) t(i));

-- ST_MakeLine_Agg: 3 points → LineString with 3 vertices.
SELECT CASE WHEN st_numpoints(st_makeline_agg(g)) = 3
            THEN 'PASS makeline_agg 3 points' ELSE 'FAIL makeline_agg 3 points' END
FROM (SELECT st_point(i, 0) AS g FROM range(0, 3) t(i));

-- ST_Intersection_Agg: 3 overlapping polygons → valid intersection.
SELECT CASE WHEN st_isvalid(st_intersection_agg(g))
            THEN 'PASS intersection_agg valid' ELSE 'FAIL intersection_agg valid' END
FROM (SELECT st_makeenvelope(i, 0, i+3, 3) AS g FROM range(0, 3) t(i));

-- ======================================================================
-- 4. CRS transform (PROJ) stability
-- ======================================================================
-- EPSG:4326 → 3857: London should be near (−14227, 6711542).
SELECT CASE WHEN abs(st_x(st_transform(st_geomfromtext('POINT(-0.1278 51.5074)'), 4326, 3857))
                     - (-14227.16)) < 1.0
            THEN 'PASS proj london webmercator' ELSE 'FAIL proj london webmercator' END;

-- Round-trip: 4326 → 3857 → 4326 should be within 1e-6 degrees.
WITH p AS (SELECT st_geomfromtext('POINT(2.3522 48.8566)') AS g)
SELECT CASE WHEN abs(st_x(st_transform(st_transform(g, 4326, 3857), 3857, 4326))
                     - st_x(g)) < 1e-6
            THEN 'PASS proj roundtrip' ELSE 'FAIL proj roundtrip' END FROM p;

-- ======================================================================
-- 5. Raster: ST_Value + st_raster_transform integration
-- ======================================================================
-- ST_Value at pixel center should match st_pixeldata value.
SELECT CASE WHEN st_value('tests/data/test_raster.asc', 1, 1.5, 1.5) = 6.0
            THEN 'PASS st_value integration' ELSE 'FAIL st_value integration' END;

-- st_raster_transform returns correct spatial bounds.
SELECT CASE WHEN (SELECT xmin FROM st_raster_transform('tests/data/test_raster.asc')) = 0.0
              AND (SELECT ymax FROM st_raster_transform('tests/data/test_raster.asc')) = 3.0
            THEN 'PASS raster_transform bounds' ELSE 'FAIL raster_transform bounds' END;

-- ======================================================================
-- 6. Maintenance: routed st_* match literal sedona_st_* kernel
-- ======================================================================
-- Verify that routed functions produce identical results to their literal twins.
WITH corpus(g) AS (
    SELECT st_geomfromtext('POINT(1 2)')
    UNION ALL SELECT st_geomfromtext('LINESTRING(0 0,1 1,2 2)')
    UNION ALL SELECT st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))')
)
SELECT CASE WHEN (SELECT count(*) FROM corpus WHERE
        st_dimension(g) IS DISTINCT FROM sedona_st_dimension(g)
     OR st_isempty(g) IS DISTINCT FROM sedona_st_isempty(g)
     OR st_isclosed(g) IS DISTINCT FROM sedona_st_isclosed(g)
     OR abs(st_xmin(g) - sedona_st_xmin(g)) > 1e-9
     OR abs(st_xmax(g) - sedona_st_xmax(g)) > 1e-9
    ) = 0
            THEN 'PASS routed == literal' ELSE 'FAIL routed != literal' END;

-- ======================================================================
-- 7. Force-dimension family (Month 1 addition) — regression
-- ======================================================================
SELECT CASE WHEN sedona_st_hasz(st_force3d(st_geomfromtext('POINT(1 2)'), 0.0))
            THEN 'PASS force3d regression' ELSE 'FAIL force3d regression' END;

SELECT CASE WHEN sedona_st_hasm(st_force4d(st_geomfromtext('POINT(1 2)'), 0.0, 0.0))
            THEN 'PASS force4d regression' ELSE 'FAIL force4d regression' END;

-- ======================================================================
-- 8. NULL safety across function families
-- ======================================================================
SELECT CASE WHEN st_buffer(NULL, 1.0) IS NULL
              AND st_simplify(NULL, 1.0) IS NULL
              AND st_centroid(NULL) IS NULL
              AND st_convexhull(NULL) IS NULL
            THEN 'PASS null safety' ELSE 'FAIL null safety' END;
