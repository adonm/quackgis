.mode list
-- Milestone 11: Sedona-style adaptive partitioning.
-- Tests the sort-then-pack approach: compute a cell histogram at fine zoom,
-- sort cells by quadkey (preserving spatial locality), and cut partitions at
-- target row boundaries. The result is plain data — no hidden index files.

-- =====================================================================
-- 1. Helper function basic correctness
-- =====================================================================

-- st_estimate_partition_count: 1B rows, 200 bytes/row, 256MB target → 746
SELECT CASE WHEN st_estimate_partition_count(1000000000, 200, 268435456) = 746
THEN 'PASS est_part_1b' ELSE 'FAIL est_part_1b' END;

-- Small dataset fits in one object
SELECT CASE WHEN st_estimate_partition_count(1000, 100, 268435456) = 1
THEN 'PASS est_part_small' ELSE 'FAIL est_part_small' END;

-- Invalid inputs → NULL
SELECT CASE WHEN st_estimate_partition_count(0, 100, 268435456) IS NULL
THEN 'PASS est_part_invalid' ELSE 'FAIL est_part_invalid' END;

-- st_recommend_zoom: 746 partitions → zoom 5 (4^5=1024 > 746)
SELECT CASE WHEN st_recommend_zoom(746) = 5
THEN 'PASS rec_zoom_746' ELSE 'FAIL rec_zoom_746' END;

-- 1 partition → zoom 0
SELECT CASE WHEN st_recommend_zoom(1) = 0
THEN 'PASS rec_zoom_1' ELSE 'FAIL rec_zoom_1' END;

-- Invalid → NULL
SELECT CASE WHEN st_recommend_zoom(0) IS NULL
THEN 'PASS rec_zoom_invalid' ELSE 'FAIL rec_zoom_invalid' END;

-- =====================================================================
-- 2. Generate skewed spatial data
-- =====================================================================

-- 1000 points: 90% clustered near NYC, 10% spread across the globe
CREATE TEMP TABLE skewed_data AS
SELECT
    i AS id,
    CASE
        WHEN i < 900 THEN st_point(-74.0 + (i % 30)::double / 100.0,
                                    40.7 + (i % 20)::double / 100.0)
        ELSE st_point((i % 360)::double - 180.0, (i % 170)::double - 85.0)
    END AS geom
FROM range(0, 1000) t(i);

-- Verify data was created
SELECT CASE WHEN count(*) = 1000
THEN 'PASS skewed_data_created' ELSE 'FAIL skewed_data_created' END
FROM skewed_data;

-- =====================================================================
-- 3. Fixed-grid partitioning (baseline for comparison)
-- =====================================================================

CREATE TEMP TABLE fixed_grid AS
SELECT
    st_quadkey(geom, 4) AS cell,
    count(*) AS row_count
FROM skewed_data
GROUP BY cell
ORDER BY cell;

-- With fixed grid, the NYC cluster creates a hot cell
SELECT CASE WHEN max(row_count) > 400
THEN 'PASS fixed_grid_hot_spot' ELSE 'FAIL fixed_grid_hot_spot' END
FROM fixed_grid;

-- =====================================================================
-- 4. Adaptive partitioning (sort-then-pack)
-- =====================================================================

-- Step A: Compute cell histogram at fine zoom (12 = ~0.088° cells)
CREATE TEMP TABLE cell_hist AS
SELECT
    st_quadkey(geom, 12) AS cell,
    count(*) AS row_count
FROM skewed_data
GROUP BY cell;

-- Step B: Sort cells lexicographically by quadkey (preserves locality),
--         compute cumulative rows, and assign partition IDs at target boundaries.
--         Target: 200 rows per adaptive partition.
CREATE TEMP TABLE adaptive_spec AS
WITH sorted AS (
    SELECT
        cell,
        row_count,
        sum(row_count) OVER (ORDER BY cell ROWS UNBOUNDED PRECEDING) AS cum_rows,
        200 AS target_per_partition
    FROM cell_hist
)
SELECT
    floor((cum_rows - 1) / target_per_partition)::int AS partition_id,
    min(cell) AS cell_min,
    max(cell) AS cell_max,
    sum(row_count) AS total_rows
FROM sorted
GROUP BY floor((cum_rows - 1) / target_per_partition)
ORDER BY partition_id;

-- Verify the adaptive spec has multiple partitions
SELECT CASE WHEN count(*) > 1
THEN 'PASS adaptive_has_partitions' ELSE 'FAIL adaptive_has_partitions' END
FROM adaptive_spec;

-- Verify all rows are accounted for
SELECT CASE WHEN sum(total_rows) = 1000
THEN 'PASS adaptive_total_rows' ELSE 'FAIL adaptive_total_rows' END
FROM adaptive_spec;

-- Verify partitions are contiguous quadkey ranges (cell_min <= cell_max)
SELECT CASE WHEN count(*) = 0
THEN 'PASS adaptive_contiguous' ELSE 'FAIL adaptive_contiguous' END
FROM adaptive_spec
WHERE cell_min > cell_max;

-- =====================================================================
-- 5. Adaptive is more balanced than fixed grid
-- =====================================================================

-- Fixed grid max partition size vs adaptive max partition size
-- Adaptive should have smaller max partition because it packs by target size
SELECT CASE WHEN
    (SELECT max(row_count) FROM fixed_grid) >
    (SELECT max(total_rows) FROM adaptive_spec)
THEN 'PASS adaptive_better_balance' ELSE 'FAIL adaptive_better_balance' END;

-- =====================================================================
-- 6. Assign geometries to adaptive partitions
-- =====================================================================

-- Build a direct cell-to-partition lookup (simpler than range join)
CREATE TEMP TABLE cell_to_partition AS
WITH sorted AS (
    SELECT
        cell,
        row_count,
        sum(row_count) OVER (ORDER BY cell ROWS UNBOUNDED PRECEDING) AS cum_rows,
        200 AS target_per_partition
    FROM cell_hist
)
SELECT cell, floor((cum_rows - 1) / target_per_partition)::int AS partition_id
FROM sorted;

-- Assign each geometry to its partition via the lookup
CREATE TEMP TABLE assigned AS
SELECT
    d.id,
    d.geom,
    st_quadkey(d.geom, 12) AS cell,
    c.partition_id
FROM skewed_data d
JOIN cell_to_partition c ON c.cell = st_quadkey(d.geom, 12);

-- Every geometry gets a partition assignment
SELECT CASE WHEN count(CASE WHEN partition_id IS NULL THEN 1 END) = 0
THEN 'PASS assign_all_have_partition' ELSE 'FAIL assign_all_have_partition' END
FROM assigned;

-- No duplicate or lost rows (each geometry assigned exactly once)
SELECT CASE WHEN count(*) = 1000
THEN 'PASS assign_no_duplicates' ELSE 'FAIL assign_no_duplicates' END
FROM assigned;

-- =====================================================================
-- 7. Query correctness: adaptive assignment doesn't change results
-- =====================================================================

-- Exact query (no partition pruning) on original table vs assigned table
SELECT CASE WHEN
    (SELECT count(*) FROM skewed_data
     WHERE st_distance(geom, st_point(-74.0, 40.7)) < 0.5)
    =
    (SELECT count(*) FROM assigned
     WHERE st_distance(geom, st_point(-74.0, 40.7)) < 0.5)
THEN 'PASS adaptive_query_correct' ELSE 'FAIL adaptive_query_correct' END;

-- =====================================================================
-- 8. Determinism: partition spec is reproducible
-- =====================================================================

CREATE TEMP TABLE adaptive_spec_2 AS
WITH sorted AS (
    SELECT
        cell,
        row_count,
        sum(row_count) OVER (ORDER BY cell ROWS UNBOUNDED PRECEDING) AS cum_rows,
        200 AS target_per_partition
    FROM cell_hist
)
SELECT
    floor((cum_rows - 1) / target_per_partition)::int AS partition_id,
    min(cell) AS cell_min,
    max(cell) AS cell_max,
    sum(row_count) AS total_rows
FROM sorted
GROUP BY floor((cum_rows - 1) / target_per_partition)
ORDER BY partition_id;

SELECT CASE WHEN count(*) = 0
THEN 'PASS adaptive_deterministic' ELSE 'FAIL adaptive_deterministic' END
FROM (
    SELECT a.partition_id, a.cell_min, a.cell_max, a.total_rows
    FROM adaptive_spec a
    JOIN adaptive_spec_2 b USING (partition_id)
    WHERE a.cell_min != b.cell_min OR a.cell_max != b.cell_max OR a.total_rows != b.total_rows
);
