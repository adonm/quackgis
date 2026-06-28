.mode list
-- Milestone 10: DuckLake spatial layout end-to-end test.
-- Requires: ducklake extension, sedonadb extension.
-- Run: duckdb -unsigned -cmd "LOAD sedonadb; LOAD ducklake;" < this_file
--
-- This test creates a real DuckLake catalog, writes spatial data with
-- materialized layout columns, partitions by spatial cell, and queries
-- using the canonical three-stage pattern.

-- Setup: clean and create DuckLake catalog
CREATE OR REPLACE TEMP TABLE _test_marker AS SELECT 1;
-- Use in-memory ducklake for CI portability
ATTACH 'ducklake::memory::' AS dl10;

-- Step 1: Create spatial table with materialized layout columns
CREATE TABLE dl10.points AS
SELECT
    i AS id,
    st_point(i::double / 10.0, (i % 100)::double / 10.0) AS geom,
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,
    st_quadkey(geom, 6) AS spatial_cell,
    st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 200) t(i);

-- Verify table was created with correct columns
SELECT CASE WHEN count(*) = 200
THEN 'PASS ducklake_create_table' ELSE 'FAIL ducklake_create_table' END
FROM dl10.points;

-- Step 2: Set partitioning
ALTER TABLE dl10.points SET PARTITIONED BY (spatial_cell);

-- Step 3: Append more data (triggers partitioned write)
INSERT INTO dl10.points
SELECT
    i + 200 AS id,
    st_point(20.0 + i::double / 10.0, 10.0 + (i % 50)::double / 10.0) AS geom,
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,
    st_quadkey(geom, 6) AS spatial_cell,
    st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 100) t(i);

-- Verify total rows
SELECT CASE WHEN count(*) = 300
THEN 'PASS ducklake_append' ELSE 'FAIL ducklake_append' END
FROM dl10.points;

-- Step 4: Multiple distinct partition cells exist
SELECT CASE WHEN count(DISTINCT spatial_cell) > 1
THEN 'PASS ducklake_multiple_cells' ELSE 'FAIL ducklake_multiple_cells' END
FROM dl10.points;

-- Step 5: Three-stage query — cell pruning + bbox + exact predicate
-- (Note: covering cells must cover the query AREA, not just the point.)
SELECT CASE WHEN count(*) > 0
THEN 'PASS ducklake_three_stage_query' ELSE 'FAIL ducklake_three_stage_query' END
FROM dl10.points p
WHERE p.spatial_cell IN (
    SELECT quadkey FROM st_covering_quadkeys(st_makeenvelope(3.0, 1.0, 7.0, 5.0), 6, 100)
)
  AND p.xmax >= 4.0 AND p.xmin <= 6.0
  AND p.ymax >= 2.0 AND p.ymin <= 4.0
  AND st_distance(p.geom, st_geomfromtext('POINT(5.0 3.0)')) < 2.0;

-- Step 6: Exact-only query (no pruning) returns the same rows
SELECT CASE WHEN count(*) > 0
THEN 'PASS ducklake_exact_query' ELSE 'FAIL ducklake_exact_query' END
FROM dl10.points p
WHERE st_distance(p.geom, st_geomfromtext('POINT(5.0 3.0)')) < 2.0;

-- Step 7: Cell-pruned query is a subset of exact query
-- (cell pruning should never miss matching rows)
SELECT CASE WHEN
    (SELECT count(*) FROM dl10.points p
     WHERE p.spatial_cell IN (
         SELECT quadkey FROM st_covering_quadkeys(st_makeenvelope(3.0, 1.0, 7.0, 5.0), 6, 100)
     )
     AND st_distance(p.geom, st_geomfromtext('POINT(5.0 3.0)')) < 2.0)
    =
    (SELECT count(*) FROM dl10.points p
     WHERE st_distance(p.geom, st_geomfromtext('POINT(5.0 3.0)')) < 2.0)
THEN 'PASS ducklake_pruning_correct' ELSE 'FAIL ducklake_pruning_correct' END;

-- Step 8: Partition evolution — change partition key
ALTER TABLE dl10.points RESET PARTITIONED BY;
ALTER TABLE dl10.points SET PARTITIONED BY (bucket(4, spatial_cell));

-- Insert more data under new partitioning
INSERT INTO dl10.points
SELECT
    i + 300 AS id,
    st_point(50.0, 50.0) AS geom,
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,
    st_quadkey(geom, 6) AS spatial_cell,
    st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 10) t(i);

-- Verify total rows after evolution
SELECT CASE WHEN count(*) = 310
THEN 'PASS ducklake_partition_evolution' ELSE 'FAIL ducklake_partition_evolution' END
FROM dl10.points;

-- Step 9: Time travel — query at original snapshot
SELECT CASE WHEN count(*) = 200
THEN 'PASS ducklake_time_travel' ELSE 'FAIL ducklake_time_travel' END
FROM dl10.points AT (VERSION => 1);

-- Cleanup
DETACH dl10;
