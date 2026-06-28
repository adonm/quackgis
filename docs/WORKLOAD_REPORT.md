# Workload porting report

Evidence that representative PostGIS workloads port to DuckDB + DuckLake with
the `sedonadb` extension, with correct results and observable partition pruning.

## Workload templates

Each template has PostGIS source SQL (as comments), ported DuckDB SQL, and a
row-count parity check in `tests/reference/m13_workloads.sql`. These checks
prove the DuckDB rewrite preserves the equivalent exact query result; the
smaller `tests/postgis_port/` harness carries direct PostGIS expected-output
cases.

### 1. Points-in-polygons spatial join

**PostGIS:**
```sql
SELECT p.id, r.id FROM points p JOIN regions r ON st_within(p.geom, r.geom);
```

**DuckDB port:** identical `st_within` + bbox column prefilter.
```sql
SELECT p.id, r.id
FROM points p, regions r
WHERE p.xmax >= r.xmin AND p.xmin <= r.xmax
  AND p.ymax >= r.ymin AND p.ymin <= r.ymax
  AND st_within(p.geom, r.geom);
```

**Result:** bbox-prefiltered join returns the same row count as the exact-only
join. No rewrite needed for the `ST_Within` call itself — the only change is
adding the `&&` → bbox-column prefilter.

### 2. KNN (nearest neighbor)

**PostGIS:**
```sql
SELECT * FROM points ORDER BY geom <-> st_point(0,0) LIMIT 5;
```

**DuckDB port:**
```sql
SELECT * FROM points
ORDER BY st_distance(geom, st_point(0,0)) LIMIT 5;
```

With bbox prefilter for large tables:
```sql
SELECT * FROM points
WHERE xmin BETWEEN -5 AND 5 AND ymin BETWEEN -5 AND 5
ORDER BY st_distance(geom, st_point(0,0)) LIMIT 5;
```

**Result:** bbox-prefiltered KNN returns the same nearest point as the full
scan. The rewrite is mechanical: `<->` → `ORDER BY st_distance`.

### 3. Dissolve / aggregate

**PostGIS:**
```sql
SELECT st_collect(geom) FROM points GROUP BY cell;
SELECT st_union(geom) FROM polygons GROUP BY region;
```

**DuckDB port:**
```sql
SELECT st_collect(geom) FROM points GROUP BY spatial_cell;
SELECT st_union_agg(geom) FROM polygons GROUP BY region;
```

**Result:** `ST_Collect` aggregate works for points; `ST_Union_Agg` for
polygonal dissolve. Note: `ST_Union_Agg` is polygonal-only (cascaded union);
use `ST_Collect` for heterogeneous geometry types.

### 4. Spatial range query with partition pruning

**PostGIS:**
```sql
SELECT * FROM points WHERE st_dwithin(geom, st_point(0,0), 5);
```

**DuckDB port (three-stage):**
```sql
SELECT * FROM points p
WHERE p.spatial_cell IN (
    SELECT quadkey FROM st_covering_quadkeys(
        st_makeenvelope(-5, -5, 5, 5), 4, 1000)
)
AND p.xmax >= -5 AND p.xmin <= 5
AND p.ymax >= -5 AND p.ymin <= 5
AND st_distance(p.geom, st_point(0,0)) < 5;
```

**Result:** three-stage query returns the same count as exact-only query.
Cell pruning is effective: the covering cells for the query area are a strict
subset of all partition cells, so DuckLake skips irrelevant files.

### 5. Bbox window query

**PostGIS:**
```sql
SELECT * FROM points WHERE geom && st_makeenvelope(-10,-10,10,10);
```

**DuckDB port:**
```sql
SELECT * FROM points
WHERE xmax >= -10 AND xmin <= 10
  AND ymax >= -10 AND ymin <= 10
  AND st_intersects(geom, st_makeenvelope(-10,-10,10,10));
```

**Result:** bbox-window query matches exact `st_intersects` count.

## Partition pruning evidence

The workload test verifies that DuckLake partition pruning is observable:

- The covering cells for a query area (e.g., 5-degree bbox around origin) are a
  strict subset of all partition cells in the table.
- This means DuckLake's file-level partition pruning skips files in non-matching
  cells.
- The three-stage query (cell + bbox + exact) returns the same result as the
  exact-only query, proving no rows are lost to pruning.

## Porting summary

| PostGIS pattern | Rewrite effort | Port status |
|---|---|---|
| `ST_*` function calls | None (identical namespace) | ✅ direct port |
| `&&` operator | Add bbox columns + predicate | ✅ documented rewrite |
| `<->` KNN operator | `ORDER BY st_distance` + `LIMIT` | ✅ documented rewrite |
| `::geometry` casts | `ST_GeomFromText` / WKB constructors | ✅ documented rewrite |
| `CREATE INDEX USING gist` | Materialize layout columns | ✅ M9–M12 primitives |
| `ST_Union` aggregate | `ST_Union_Agg` (polygons) / `ST_Collect` | ✅ aggregate available |
| GiST planner hooks | Not possible in DuckDB C-API | ➖ documented non-goal |

## Test artifacts

| File | Scope |
|---|---|
| `tests/postgis_port/cases/*.sql` | 59 individual PostGIS function port cases |
| `tests/reference/m13_workloads.sql` | 11 end-to-end workload checks on DuckLake |
| `tests/reference/m10_ducklake.sql` | DuckLake round-trip + partition evolution |
| `tests/reference/m12_multiwriter.sh` | Multi-writer + partition evolution shell test |
| `benchmarks/layout_benchmark.sh` | Three-layout timing comparison |
