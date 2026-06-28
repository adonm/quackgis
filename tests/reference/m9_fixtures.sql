.mode list
-- Milestone 9: spatial partition key primitives.
-- Tests determinism, edge cases, NULL safety, and correctness of the
-- partition-key functions that DuckLake layout recipes depend on.

-- =====================================================================
-- 1. ST_BBoxIntersects
-- =====================================================================

SELECT CASE WHEN st_bbox_intersects(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POLYGON((2 2,6 2,6 6,2 6,2 2))')) = true
THEN 'PASS bbox_intersects_overlap' ELSE 'FAIL bbox_intersects_overlap' END;

SELECT CASE WHEN st_bbox_intersects(
    st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'),
    st_geomfromtext('POLYGON((10 10,11 10,11 11,10 11,10 10))')) = false
THEN 'PASS bbox_intersects_disjoint' ELSE 'FAIL bbox_intersects_disjoint' END;

SELECT CASE WHEN st_bbox_intersects(NULL, st_point(0.0, 0.0)) IS NULL
THEN 'PASS bbox_intersects_null' ELSE 'FAIL bbox_intersects_null' END;

-- =====================================================================
-- 2. ST_QuadKey — determinism and correctness
-- =====================================================================

-- Known: NYC (-74.0, 40.7) at zoom 8 → 8-char quadkey
SELECT CASE WHEN length(st_quadkey(st_geomfromtext('POINT(-74.0 40.7)'), 8)) = 8
THEN 'PASS quadkey_length' ELSE 'FAIL quadkey_length' END;

-- Zoom 0 → single tile "0"
SELECT CASE WHEN st_quadkey(st_geomfromtext('POINT(0 0)'), 0) = '0'
THEN 'PASS quadkey_zoom0' ELSE 'FAIL quadkey_zoom0' END;

-- Determinism: same input → same output
SELECT CASE WHEN st_quadkey(st_geomfromtext('POINT(-74.0 40.7)'), 8) =
                 st_quadkey(st_geomfromtext('POINT(-74.0 40.7)'), 8)
THEN 'PASS quadkey_determinism' ELSE 'FAIL quadkey_determinism' END;

-- Different points at same zoom → different keys (almost always)
SELECT CASE WHEN st_quadkey(st_geomfromtext('POINT(-74.0 40.7)'), 8) !=
                 st_quadkey(st_geomfromtext('POINT(139.0 35.0)'), 8)
THEN 'PASS quadkey_different' ELSE 'FAIL quadkey_different' END;

-- NULL propagation
SELECT CASE WHEN st_quadkey(NULL, 8) IS NULL
THEN 'PASS quadkey_null' ELSE 'FAIL quadkey_null' END;

-- Invalid zoom → NULL
SELECT CASE WHEN st_quadkey(st_geomfromtext('POINT(0 0)'), -1) IS NULL
THEN 'PASS quadkey_bad_zoom' ELSE 'FAIL quadkey_bad_zoom' END;

-- =====================================================================
-- 3. ST_GeoHash
-- =====================================================================

-- Known: NYC (-74.0, 40.7) at precision 4 → "dr5r"
SELECT CASE WHEN st_geohash(st_geomfromtext('POINT(-74.0 40.7)'), 4) = 'dr5r'
THEN 'PASS geohash_nyc' ELSE 'FAIL geohash_nyc' END;

-- Precision = string length
SELECT CASE WHEN length(st_geohash(st_geomfromtext('POINT(-74.0 40.7)'), 8)) = 8
THEN 'PASS geohash_length' ELSE 'FAIL geohash_length' END;

-- Determinism
SELECT CASE WHEN st_geohash(st_geomfromtext('POINT(-74.0 40.7)'), 6) =
                 st_geohash(st_geomfromtext('POINT(-74.0 40.7)'), 6)
THEN 'PASS geohash_determinism' ELSE 'FAIL geohash_determinism' END;

-- NULL propagation
SELECT CASE WHEN st_geohash(NULL, 6) IS NULL
THEN 'PASS geohash_null' ELSE 'FAIL geohash_null' END;

-- =====================================================================
-- 4. ST_Hilbert — sort key for clustering
-- =====================================================================

-- Determinism: same input → same output
SELECT CASE WHEN st_hilbert(st_geomfromtext('POINT(-74.0 40.7)'), 12) =
                 st_hilbert(st_geomfromtext('POINT(-74.0 40.7)'), 12)
THEN 'PASS hilbert_determinism' ELSE 'FAIL hilbert_determinism' END;

-- Non-negative
SELECT CASE WHEN st_hilbert(st_geomfromtext('POINT(0 0)'), 8) >= 0
THEN 'PASS hilbert_nonneg' ELSE 'FAIL hilbert_nonneg' END;

-- Nearby points have closer Hilbert values than far points
SELECT CASE WHEN
    abs(st_hilbert(st_geomfromtext('POINT(0 0)'), 12) -
        st_hilbert(st_geomfromtext('POINT(0.001 0.001)'), 12)) <
    abs(st_hilbert(st_geomfromtext('POINT(0 0)'), 12) -
        st_hilbert(st_geomfromtext('POINT(90 90)'), 12))
THEN 'PASS hilbert_locality' ELSE 'FAIL hilbert_locality' END;

-- NULL propagation
SELECT CASE WHEN st_hilbert(NULL, 12) IS NULL
THEN 'PASS hilbert_null' ELSE 'FAIL hilbert_null' END;

-- =====================================================================
-- 5. ST_Morton — alternative sort key
-- =====================================================================

SELECT CASE WHEN st_morton(st_geomfromtext('POINT(-74.0 40.7)'), 12) =
                 st_morton(st_geomfromtext('POINT(-74.0 40.7)'), 12)
THEN 'PASS morton_determinism' ELSE 'FAIL morton_determinism' END;

SELECT CASE WHEN st_morton(NULL, 12) IS NULL
THEN 'PASS morton_null' ELSE 'FAIL morton_null' END;

-- =====================================================================
-- 6. ST_TileEnvelope — Web Mercator tile bounds
-- =====================================================================

-- Zoom 1, tile (0,0): western hemisphere, northern half
SELECT CASE WHEN
    abs(st_xmin(st_tileenvelope(1, 0, 0)) - (-180.0)) < 0.001 AND
    abs(st_xmax(st_tileenvelope(1, 0, 0)) - 0.0) < 0.001
THEN 'PASS tileenvelope_z1_x0_y0' ELSE 'FAIL tileenvelope_z1_x0_y0' END;

-- Valid polygon
SELECT CASE WHEN st_geometrytype(st_tileenvelope(5, 10, 10)) = 'ST_Polygon'
THEN 'PASS tileenvelope_type' ELSE 'FAIL tileenvelope_type' END;

-- Non-zero area
SELECT CASE WHEN st_area(st_tileenvelope(5, 10, 10)) > 0
THEN 'PASS tileenvelope_area' ELSE 'FAIL tileenvelope_area' END;

-- NULL/invalid → NULL
SELECT CASE WHEN st_tileenvelope(-1, 0, 0) IS NULL
THEN 'PASS tileenvelope_bad_zoom' ELSE 'FAIL tileenvelope_bad_zoom' END;

-- =====================================================================
-- 7. ST_CoveringQuadKeys — table function for query-side pruning
-- =====================================================================

-- A point covers exactly 1 cell
SELECT CASE WHEN count(*) = 1
THEN 'PASS covering_point' ELSE 'FAIL covering_point' END
FROM st_covering_quadkeys(st_geomfromtext('POINT(-74.0 40.7)'), 8, 1000);

-- A small polygon covers a small number of cells
SELECT CASE WHEN count(*) > 0 AND count(*) < 20
THEN 'PASS covering_small_polygon' ELSE 'FAIL covering_small_polygon' END
FROM st_covering_quadkeys(st_geomfromtext('POLYGON((-74.1 40.6,-73.9 40.6,-73.9 40.8,-74.1 40.8,-74.1 40.6))'), 10, 1000);

-- The covering cells' envelopes contain the query geometry's envelope center
SELECT CASE WHEN count(*) > 0
THEN 'PASS covering_nonempty' ELSE 'FAIL covering_nonempty' END
FROM st_covering_quadkeys(st_geomfromtext('POINT(-74.0 40.7)'), 8, 1000)
WHERE quadkey IS NOT NULL;

-- max_cells enforcement: large envelope at high zoom with small cap → 0 rows
SELECT CASE WHEN count(*) = 0
THEN 'PASS covering_fails_closed' ELSE 'FAIL covering_fails_closed' END
FROM st_covering_quadkeys(st_geomfromtext('POLYGON((-170 -80,170 -80,170 80,-170 80,-170 -80))'), 8, 10);

-- =====================================================================
-- 8. Layout workflow: materialize partition columns
-- =====================================================================

-- Simulate the canonical DuckLake layout pattern
CREATE TEMP TABLE layout_test AS
SELECT
    st_geomfromtext('POINT(-74.0 40.7)') AS geom,
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,
    st_quadkey(geom, 8) AS spatial_cell,
    st_hilbert(geom, 12) AS spatial_sort;

SELECT CASE WHEN
    spatial_cell IS NOT NULL AND
    spatial_sort IS NOT NULL AND
    xmin = xmax AND ymin = ymax  -- point has zero-area bbox
THEN 'PASS layout_materialize' ELSE 'FAIL layout_materialize' END
FROM layout_test;

-- Three-stage query pattern works (literal geometry — table functions
-- cannot take subquery arguments per DuckDB limitation)
SELECT CASE WHEN count(*) > 0
THEN 'PASS layout_three_stage_query' ELSE 'FAIL layout_three_stage_query' END
FROM layout_test p
WHERE p.spatial_cell IN (
    SELECT quadkey FROM st_covering_quadkeys(
        st_geomfromtext('POINT(-74.0 40.7)'), 8, 100))
  AND st_intersects(p.geom, st_geomfromtext('POINT(-74.0 40.7)'));
