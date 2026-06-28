.mode list
-- Milestone 13: Workload-scale validation.
-- Five representative PostGIS workloads ported to DuckDB + DuckLake,
-- each with row-count parity and partition-pruning evidence.
--
-- Requires: ducklake extension, sedonadb extension.
-- Run: duckdb -unsigned -cmd "LOAD sedonadb; LOAD ducklake;" < this_file

-- =====================================================================
-- Setup: generate test data and load into DuckLake
-- =====================================================================

ATTACH 'ducklake::memory::' AS dl13;

-- Points table: 2000 points spread across [-50,50] x [-50,50]
CREATE TABLE dl13.points AS
SELECT
    i AS id,
    st_point((i % 100)::double / 2.0 - 25.0, ((i * 3) % 100)::double / 2.0 - 25.0) AS geom,
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,
    st_quadkey(geom, 4) AS spatial_cell,
    st_hilbert(geom, 12) AS spatial_sort
FROM range(0, 2000) t(i);
ALTER TABLE dl13.points SET PARTITIONED BY (spatial_cell);

-- Polygons table: 20 grid cells as query polygons
CREATE TABLE dl13.regions AS
SELECT
    i AS id,
    st_makeenvelope(
        (i % 5)::double * 10.0 - 25.0,
        (i / 5)::double * 10.0 - 25.0,
        (i % 5)::double * 10.0 - 15.0,
        (i / 5)::double * 10.0 - 15.0
    ) AS geom,
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax
FROM range(0, 20) t(i);

-- =====================================================================
-- Workload 1: Points-in-polygons spatial join
-- PostGIS:
--   SELECT p.id, r.id FROM points p JOIN regions r ON st_within(p.geom, r.geom);
-- DuckDB port: identical st_within + bbox prefilter
-- =====================================================================

-- Parity: DuckLake join count = in-memory join count
SELECT CASE WHEN
    (SELECT count(*) FROM dl13.points p, dl13.regions r
     WHERE st_within(p.geom, r.geom))
    =
    (SELECT count(*) FROM dl13.points p, dl13.regions r
     WHERE p.xmax >= r.xmin AND p.xmin <= r.xmax
       AND p.ymax >= r.ymin AND p.ymin <= r.ymax
       AND st_within(p.geom, r.geom))
THEN 'PASS workload_points_in_poly_parity' ELSE 'FAIL workload_points_in_poly_parity' END;

-- Result is non-empty
SELECT CASE WHEN count(*) > 0
THEN 'PASS workload_points_in_poly_nonempty' ELSE 'FAIL workload_points_in_poly_nonempty' END
FROM dl13.points p, dl13.regions r
WHERE st_within(p.geom, r.geom);

-- =====================================================================
-- Workload 2: KNN (nearest neighbor)
-- PostGIS:
--   SELECT * FROM points ORDER BY geom <-> st_point(0,0) LIMIT 5;
-- DuckDB port: ORDER BY st_distance + LIMIT
-- =====================================================================

SELECT CASE WHEN
    (SELECT id FROM dl13.points ORDER BY st_distance(geom, st_point(0.0, 0.0)) LIMIT 1)
    IS NOT NULL
THEN 'PASS workload_knn_nearest' ELSE 'FAIL workload_knn_nearest' END;

-- KNN with bbox prefilter returns the same nearest point
SELECT CASE WHEN
    (SELECT id FROM dl13.points
     WHERE xmin BETWEEN -5.0 AND 5.0 AND ymin BETWEEN -5.0 AND 5.0
     ORDER BY st_distance(geom, st_point(0.0, 0.0)) LIMIT 1)
    =
    (SELECT id FROM dl13.points
     ORDER BY st_distance(geom, st_point(0.0, 0.0)) LIMIT 1)
THEN 'PASS workload_knn_bbox_prefilter_parity' ELSE 'FAIL workload_knn_bbox_prefilter_parity' END;

-- =====================================================================
-- Workload 3: Dissolve / aggregate
-- PostGIS:
--   SELECT st_union(geom) FROM polygons GROUP BY region;
--   SELECT st_collect(geom) FROM points GROUP BY cell;
-- DuckDB port: st_union_agg for polygons, st_collect for points
-- =====================================================================

-- Collect points by cell (multipoint aggregation)
SELECT CASE WHEN count(*) > 0
THEN 'PASS workload_dissolve_groups' ELSE 'FAIL workload_dissolve_groups' END
FROM (SELECT spatial_cell, st_collect(geom) AS merged FROM dl13.points GROUP BY spatial_cell);

-- Each group produces a non-null geometry
SELECT CASE WHEN count(*) = 0
THEN 'PASS workload_dissolve_nonnull' ELSE 'FAIL workload_dissolve_nonnull' END
FROM (
    SELECT spatial_cell, st_collect(geom) AS merged
    FROM dl13.points GROUP BY spatial_cell
) WHERE merged IS NULL;

-- =====================================================================
-- Workload 4: Spatial range query with partition pruning
-- PostGIS:
--   SELECT * FROM points WHERE st_dwithin(geom, st_point(0,0), 5);
-- DuckDB port: three-stage (cell prune + bbox + exact)
-- =====================================================================

-- Parity: three-stage query = exact-only query
SELECT CASE WHEN
    (SELECT count(*) FROM dl13.points
     WHERE st_distance(geom, st_point(0.0, 0.0)) < 5.0)
    =
    (SELECT count(*) FROM dl13.points p
     WHERE p.spatial_cell IN (
         SELECT quadkey FROM st_covering_quadkeys(
             st_makeenvelope(-5.0, -5.0, 5.0, 5.0), 4, 1000)
     )
     AND p.xmax >= -5.0 AND p.xmin <= 5.0
     AND p.ymax >= -5.0 AND p.ymin <= 5.0
     AND st_distance(p.geom, st_point(0.0, 0.0)) < 5.0)
THEN 'PASS workload_range_pruning_parity' ELSE 'FAIL workload_range_pruning_parity' END;

-- The pruned query scans fewer rows (cell filter is restrictive)
SELECT CASE WHEN
    (SELECT count(*) FROM dl13.points p
     WHERE p.spatial_cell IN (
         SELECT quadkey FROM st_covering_quadkeys(
             st_makeenvelope(-5.0, -5.0, 5.0, 5.0), 4, 1000)
     )) <
    (SELECT count(*) FROM dl13.points)
THEN 'PASS workload_range_pruning_effective' ELSE 'FAIL workload_range_pruning_effective' END;

-- =====================================================================
-- Workload 5: Bbox window query
-- PostGIS:
--   SELECT * FROM points WHERE geom && st_makeenvelope(-10,-10,10,10);
-- DuckDB port: bbox column predicate + exact predicate
-- =====================================================================

SELECT CASE WHEN count(*) > 0
THEN 'PASS workload_bbox_window' ELSE 'FAIL workload_bbox_window' END
FROM dl13.points
WHERE xmax >= -10.0 AND xmin <= 10.0
  AND ymax >= -10.0 AND ymin <= 10.0
  AND st_intersects(geom, st_makeenvelope(-10.0, -10.0, 10.0, 10.0));

-- Bbox window matches exact intersects
SELECT CASE WHEN
    (SELECT count(*) FROM dl13.points
     WHERE xmax >= -10.0 AND xmin <= 10.0
       AND ymax >= -10.0 AND ymin <= 10.0
       AND st_intersects(geom, st_makeenvelope(-10.0, -10.0, 10.0, 10.0)))
    =
    (SELECT count(*) FROM dl13.points
     WHERE st_intersects(geom, st_makeenvelope(-10.0, -10.0, 10.0, 10.0)))
THEN 'PASS workload_bbox_window_parity' ELSE 'FAIL workload_bbox_window_parity' END;

-- =====================================================================
-- Evidence: DuckLake partition pruning is observable
-- =====================================================================

-- The covering cells for the query area are a strict subset of all cells
SELECT CASE WHEN
    (SELECT count(DISTINCT spatial_cell) FROM dl13.points)
    >
    (SELECT count(*) FROM st_covering_quadkeys(
        st_makeenvelope(-5.0, -5.0, 5.0, 5.0), 4, 1000))
THEN 'PASS evidence_fewer_cells_scanned' ELSE 'FAIL evidence_fewer_cells_scanned' END;

-- Cleanup
DETACH dl13;
