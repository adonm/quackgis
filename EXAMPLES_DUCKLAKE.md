# Spatial DuckLake recipes

Copy-paste recipes for storing, partitioning, and querying spatial data in
[DuckLake](https://ducklake.select/) using the `sedonadb` extension.

See [ARCHITECTURE.md](./ARCHITECTURE.md) §L3–L4 for the design rationale and
verified DuckLake constraints.

## Prerequisites

```sql
LOAD sedonadb;
LOAD ducklake;

-- Optional: helper macros that make the patterns below shorter.
-- DuckDB CLI:
-- .read sql/ducklake_spatial_macros.sql
```

The macro pack is optional. It creates thin SQL wrappers such as
`sedona_layout_cell(geom, 6)`, `sedona_covering_cells_bbox(...)`, and
`sedona_bbox_overlaps(...)`. It does **not** create any extension-owned state.

## 1. Create a DuckLake with spatial layout

```sql
-- Create a DuckLake catalog (local file + local data path)
ATTACH 'ducklake:my_spatial.ducklake' AS mylake
    (DATA_PATH 'my_spatial_files/');

-- Create a table with materialized layout columns
CREATE TABLE mylake.parcels AS
SELECT
    *,                                          -- your attributes
    geom,                                       -- WKB BLOB
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,   -- zone-map columns
    st_quadkey(geom, 6)  AS spatial_cell,           -- partition key
    st_hilbert(geom, 12) AS spatial_sort            -- clustering key
FROM raw_parcels
ORDER BY spatial_sort;                               -- cluster files spatially

-- Partition by spatial cell (or bucket for high cardinality)
ALTER TABLE mylake.parcels SET PARTITIONED BY (spatial_cell);
-- For high-cardinality cells: bucket instead
-- ALTER TABLE mylake.parcels SET PARTITIONED BY (bucket(64, spatial_cell));
```

With the optional macros:

```sql
CREATE TABLE mylake.parcels AS
SELECT
    *, geom,
    sedona_layout_xmin(geom) AS xmin, sedona_layout_ymin(geom) AS ymin,
    sedona_layout_xmax(geom) AS xmax, sedona_layout_ymax(geom) AS ymax,
    sedona_layout_cell(geom, 6) AS spatial_cell,
    sedona_layout_sort(geom, 12) AS spatial_sort
FROM raw_parcels
ORDER BY spatial_sort;
```

**Why every column matters:**

| Column | Purpose | DuckLake feature used |
|---|---|---|
| `xmin/ymin/xmax/ymax` | File-level zone-map pruning | Per-file min/max stats |
| `spatial_cell` | Partition pruning | `PARTITIONED BY` or `bucket(N, …)` |
| `spatial_sort` | Spatially clustered files | `ORDER BY` at write time |

## 2. Append new data

```sql
-- Single writer: INSERT with materialized columns
INSERT INTO mylake.parcels
SELECT
    id, geom, attrs...,
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,
    st_quadkey(geom, 6)  AS spatial_cell,
    st_hilbert(geom, 12) AS spatial_sort
FROM new_batch;
```

Multiple writers can append to the same DuckLake table simultaneously —
DuckLake's catalog handles commit atomicity. The partition-key functions are
pure and deterministic, so every writer assigns the same cell to the same
geometry.

## 3. Spatial range query (three-stage pattern)

```sql
-- Query: find all parcels within 5 degrees of (0, 0)
WITH cells AS (
    -- Stage 1: compute covering cells for the QUERY AREA (not just the point)
    SELECT quadkey
    FROM st_covering_quadkeys(
        st_makeenvelope(-5.0, -5.0, 5.0, 5.0),  -- query area bbox
        6,                                        -- same zoom as partition key
        1000                                      -- max_cells guard
    )
)
SELECT p.*
FROM mylake.parcels p
WHERE p.spatial_cell IN (SELECT quadkey FROM cells)       -- 1. partition prune
  AND p.xmax >= -5.0 AND p.xmin <= 5.0
  AND p.ymax >= -5.0 AND p.ymin <= 5.0                     -- 2. zone-map prune
  AND st_distance(p.geom, st_point(0.0, 0.0)) < 5.0;      -- 3. exact predicate
```

With helper macros:

```sql
WITH cells AS (
    SELECT quadkey FROM sedona_covering_cells_bbox(-5.0, -5.0, 5.0, 5.0, 6, 1000)
)
SELECT p.*
FROM mylake.parcels p
WHERE p.spatial_cell IN (SELECT quadkey FROM cells)
  AND sedona_bbox_overlaps(p.xmin, p.ymin, p.xmax, p.ymax, -5.0, -5.0, 5.0, 5.0)
  AND st_distance(p.geom, st_point(0.0, 0.0)) < 5.0;
```

**Critical:** the covering cells must cover the **query area** (including the
distance threshold), not just the query point. A common mistake is to compute
covering cells for the query point alone — this silently drops matching rows
whose cell assignment is different from the query point's cell.

Stages 1–2 are performance filters. Stage 3 alone defines correctness. Dropping
stages 1–2 changes speed, never results.

## 4. Spatial join

```sql
-- Join two spatial DuckLake tables using the three-stage pattern
WITH join_cells AS (
    SELECT DISTINCT q.quadkey
    FROM query_polygons qp,
         st_covering_quadkeys(qp.geom, 6, 1000) q
)
SELECT p.*, qp.name
FROM mylake.parcels p
JOIN query_polygons qp ON true
WHERE p.spatial_cell IN (SELECT quadkey FROM join_cells)
  AND p.xmax >= st_xmin(qp.geom) AND p.xmin <= st_xmax(qp.geom)
  AND p.ymax >= st_ymin(qp.geom) AND p.ymin <= st_ymax(qp.geom)
  AND st_intersects(p.geom, qp.geom);
```

## 5. KNN (nearest neighbor)

```sql
-- Find 5 nearest parcels to a point
SELECT *
FROM mylake.parcels p
ORDER BY st_distance(p.geom, st_point(10.0, 20.0))
LIMIT 5;

-- With bbox prefilter for large tables
SELECT *
FROM mylake.parcels p
WHERE p.xmin BETWEEN 9.0 AND 11.0
  AND p.ymin BETWEEN 19.0 AND 21.0
ORDER BY st_distance(p.geom, st_point(10.0, 20.0))
LIMIT 5;
```

## 6. Partition evolution

DuckLake keeps old files under their old partitioning. New writes use the new
partition key. Queries stay correct because the exact predicate (stage 3) always
runs.

```sql
-- Start with zoom 6
ALTER TABLE mylake.parcels SET PARTITIONED BY (spatial_cell);

-- Later, switch to bucketed layout
ALTER TABLE mylake.parcels RESET PARTITIONED BY;
ALTER TABLE mylake.parcels SET PARTITIONED BY (bucket(128, spatial_cell));

-- Old files stay at zoom-6 partitioning; new files use bucketed layout.
-- Queries work correctly across both.
```

## 7. Time travel

```sql
-- Query at the original snapshot
SELECT count(*) FROM mylake.parcels AT (VERSION => 1);

-- List all snapshots
FROM mylake.snapshots();
```

## 8. Multi-writer append

DuckLake's catalog handles commit atomicity and conflict resolution. Multiple
DuckDB processes can attach to the same DuckLake and append independently:

```sql
-- Writer 1 (process A)
ATTACH 'ducklake:shared.ducklake' AS dl (DATA_PATH 'shared_data/');
INSERT INTO dl.parcels SELECT ..., st_quadkey(geom, 6) AS spatial_cell, ...;

-- Writer 2 (process B, different machine or session)
ATTACH 'ducklake:shared.ducklake' AS dl (DATA_PATH 'shared_data/');
INSERT INTO dl.parcels SELECT ..., st_quadkey(geom, 6) AS spatial_cell, ...;
```

**What the extension guarantees:** partition-key functions (`st_quadkey`,
`st_geohash`, `st_hilbert`) are pure and deterministic — same input always
produces the same key, regardless of which writer computes it. No shared state,
no coordination needed.

**What DuckLake guarantees:** catalog commit atomicity, conflict detection, and
snapshot isolation. File names are UUIDs (no collisions). Each commit is
all-or-nothing at the catalog level.

**Catalog choice matters:**

| Catalog | Writer concurrency |
|---|---|
| DuckDB file (`*.ducklake`) | Serialized (file lock; one writer at a time) |
| PostgreSQL / MySQL | Concurrent (row-level catalog locks) |

Both are safe — the difference is throughput under contention, not correctness.
File-catalog serialization is fine for batch jobs that naturally take turns.

**Failure modes:**
- Commit conflict: DuckLake rejects the later commit; the writer retries.
  No data corruption, no partial files.
- Partition evolution: old files keep their old partitioning; new writes use
  the new key. Queries stay correct across mixed layouts (exact predicate
  always runs).

## Cardinality guidance

For PB-scale / trillion-row ceilings, optimize for **physical object size** and
query pruning, not rows-per-partition. A good lakehouse target is **100 MB–1 GB
per Parquet object** after compression, with 256 MB–512 MB as a practical default
starting range. Row counts per object vary too much with schema width and
geometry complexity to be a safe planning unit.

| Scale | Initial zoom | Partition strategy | Object target |
|---|---|---|---|
| Small / dev | 2–4 | identity | convenience, not performance |
| 10 GB–1 TB | 4–6 | identity or `bucket(16, spatial_cell)` | 128–512 MB |
| 1–100 TB | 5–7 | `bucket(64, spatial_cell)` or adaptive spec | 256 MB–1 GB |
| 100 TB–PB+ | adaptive | adaptive KDB/quadtree + bucket | 512 MB–1 GB |

**Rules of thumb:**
- Target **100 MB–1 GB per object**, not a row count. Smaller objects create
  metadata/listing overhead; larger objects reduce pruning and parallelism.
- Use `bucket(N, spatial_cell)` or adaptive partition specs when raw
  `spatial_cell` cardinality would create tiny objects.
- Keep the layout deterministic so many writers can append independently.
- `ORDER BY st_hilbert(geom, bits)` at write time is **always beneficial**, even
  without partitioning — it clusters nearby geometries in the same Parquet row
  groups, which tightens zone-map stats.
- At PB/trillion-row scale, fixed zoom is a bootstrap. Adaptive partitioning
  (Milestone 11) is the intended path for skewed global datasets.

## Common mistakes

1. **Covering cells for the point, not the area.** Always cover the full query
   area bbox.
2. **Dropping the exact predicate.** Cell/bbox filters are approximations.
   `st_intersects` / `st_distance` is correctness.
3. **Too-fine partitioning.** Zoom 12 produces millions of cells; use zoom 6–8
   or bucket.
4. **Forgetting `ORDER BY spatial_sort` at write time.** Without spatial
   clustering, bbox zone-map pruning is ineffective.
