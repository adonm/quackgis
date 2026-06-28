# SpatialBench benchmarks (local DuckLake)

Runs the [Apache SpatialBench](https://github.com/apache/sedona-spatialbench)
queries against the **sedonadb** extension over a **local DuckLake** (DuckDB file
as catalog, local folder for Parquet data).

## Benchmark suites

| Script | What it measures |
|--------|-----------------|
| `run.sh` / `run_queries.sh` | SpatialBench end-to-end (Q1–Q7, FN_dist/FN_area) over 600k trips / 20k buildings |
| `bridge.sql` | Literal SedonaDB bridge overhead vs local reimplementation (1M points) |
| `backends.sql` | GEOS topology, spheroid geodesics, raster streaming, bridge overhead (10k–100k rows) |
| `perf_budget.sql` | Full performance budget: bridge, GEOS, spheroid, raster, local pipeline, aggregates, table functions (10k–100k rows) |

## QA role

Apache SpatialBench is the **heavy workload tier** for this extension. It is not
the primary semantic oracle — exact compatibility belongs in focused SQL
fixtures under `tests/reference/`. SpatialBench answers a different release QA
question: do realistic spatial scans/joins finish, return stable row counts, and
stay within broad performance budgets on real geometry distributions?

Use SpatialBench:

- before releases and after hard backend routing changes (GEOS/local/SedonaDB);
- to catch robustness failures on invalid, complex, or very large Overture
  polygons;
- to track spatial join ergonomics (`bbox` prefilter + exact predicate) and
  throughput regressions;
- as a manual/nightly gate, not as a required per-commit test.

Snapshot every release run with: extension commit, DuckDB version, hardware,
data scale, adapted/skipped queries, result row counts, and wall-clock timings.

Run `backends.sql`:

```sh
LD_LIBRARY_PATH="$(brew --prefix gdal)/lib" \
  duckdb -unsigned -cmd "LOAD 'build/dev/sedonadb.duckdb_extension';" \
  < benchmarks/backends.sql
```

## Reproduce

```sh
# one end-to-end driver (builds, packages, generates data, sets up lake):
./benchmarks/run.sh
# per-query timing (cleaner: each query in its own process):
./benchmarks/run_queries.sh
```

Data setup:

- `trip` SF 0.1 (**600,000** trips), `building` SF 1 (**20,000** polygons) generated
  locally with `spatialbench-cli`.
- **`zone` is cached**: SpatialBench zone generation is slow (≈156k complex
  Overture polygons even at SF 0.1, ≈1.4 GB). `benchmarks/run.sh` downloads one
  pre-generated partition (~26k zones, 222 MB) from the
  [`apache-sedona/spatialbench` Hugging Face dataset](https://huggingface.co/datasets/apache-sedona/spatialbench/tree/main/v0.1.0/sf0.1/zone)
  into `build/spatialbench-sf0.1/zone/` and reuses it. Override by setting
  `SB_ZONE_PARQUET` or by generating with
  `spatialbench-cli -s 0.1 --tables zone`.

## Adaptations vs the canonical queries

The reference "DuckDB" dialect wraps every geometry column in
`ST_GeomFromWKB(...)` because the DuckDB `spatial` extension uses a `GEOMETRY`
type. sedonadb's `ST_*` consume ISO-WKB `BLOB` natively, so the wrappers are
dropped. Literals use `ST_GeomFromText` cast to `BLOB` (DuckDB 1.5 also ships a
`GEOMETRY`-returning `ST_GeomFromWKB`, so the cast disambiguates our overload).

**Spatial joins use a bounding-box prefilter.** With no spatial index, a naive
`trip ⋈ zone` is a nested-loop cross join (600k × 26k ≈ 1.5 × 10¹⁰ calls) and
does not finish. We materialize four bbox columns (`st_xmin/xmax/ymin/ymax`)
once per table and join on overlapping ranges (which DuckDB plans with its
inequality/IEJoin), then apply the exact predicate only on the surviving
candidate pairs. This is the pragmatic version of the "spatial-join table
function" from the project brief; a true R-tree index remains future work.

| Query | Workload | Rows touched | Result | Time |
|-------|----------|-------------:|--------|-----:|
| Q1 | trips within 50 km of Sedona (`ST_DWithin`+`ST_Distance`, scan) | 600,000 | 6 | 0.07 s |
| Q2 | trips in Coconino County (`ST_Intersects`, 1 zone) | 600,000 | 0 | 0.08 s |
| Q4 | high-tip trips → pickup zone (`ST_Within`, bbox-prefiltered join) | 1,000 × 26k | 0 | 0.12 s |
| Q5 | convex hull of collected dropoffs (`ST_Collect` + `ST_ConvexHull` + `ST_Area`) | 3 | 8.0 | 0.01 s |
| Q7 | geometric length of every trip (`ST_MakeLine` + `ST_Length`) | 600,000 | 0.03519° | 0.11 s |
| Q8 | pickups near buildings, 500 m (`ST_DWithin`, bbox-prefiltered join) | 600k × 20k | 63 buildings / 80 pickups | 0.20 s |
| Q9 | building overlap IoU pairs (`ST_Intersects`, self-join) | 20k × 20k | 37 | 1.29 s |
| Q10 | trips per pickup zone (`ST_Within`, bbox-prefiltered join) | 600k × 26k | 2,184 zones / 59k trips | 3.57 s |

Per-function throughput over the full 600k-row `trip` table (point geometries):

| Function(s) | Workload | Time |
|-------------|----------|-----:|
| `ST_X` + `ST_Y` | scan | 0.02 s |
| `ST_Distance(pickup, dropoff)` | scan | 0.02 s |
| `ST_MakeLine` + `ST_Length` | scan | 0.06 s |
| `ST_Area` over 20k polygons | scan | 0.007 s |

## Robustness hardening done (option B from the roadmap)

Real-world polygons (Overture admin boundaries here) include **invalid
(self-intersecting)** and **over-complex (up to ~133k vertices in one zone)**
polygons. `geo` 0.31's `relate` and point-in-polygon paths crash on these.
Three layered fixes were added so the extension degrades gracefully instead of
segfaulting:

1. **Public GEOS `ST_MakeValid(geom)` + an internal `ensure_valid` guard.** Every
   relate-based predicate (`ST_Within/Contains/Covers/CoveredBy/Equals/Touches/
   Crosses/Overlaps`) and every boolean op (`ST_Intersection/Union/Difference/
   SymDifference`) validates inputs and avoids fabricating results for invalid
   geometry. The public `ST_MakeValid` route now uses GEOS `make_valid` (the same
   canonical engine PostGIS uses); the local guard remains a cheap repair/fallback
   around `geo` operations. Valid inputs take the cheap fast path (`is_valid` +
   borrow, no copy).
2. **Custom ray-cast point-in-polygon for `ST_Within`/`ST_Contains`.** When one
   operand is a point (the SpatialBench join shape), we run PNPOLY even-odd ray
   casting ourselves instead of `geo`'s `Contains<Point>`. It is iterative O(n),
   cannot stack-overflow, and gives a well-defined answer even for
   self-intersecting rings — so the 133k-vertex zone no longer crashes
   `ST_Within` (0.9 s for one PIP, correct result). This is *also* faster than
   going through `geo`'s geomgraph.
3. `ST_Intersects` already uses `geo`'s sweep-line path, which is robust to
   invalid input — no change needed.

## Known limitations

- **Non-flat vector encodings are fixed and pinned by tests.** Ordered/limited,
  filtered, projected, and constant geometry vectors now feed scalar `ST_*` and
  `sedona_*` callbacks without segfaulting (`tests/vector_encodings.sql`).
- **General (non-point) `ST_Within/Contains/Covers/...` on a 100k+ vertex
  polygon** can still overflow `geo`'s geomgraph `relate`. The point case is
  covered by fix #2 above; the general case is the upstream `geo` bug below.
- **`ST_IsValid` is O(n²)** on very complex polygons — fine for normal data but
  slow on the 133k-vertex zone. It runs only inside `ensure_valid` (so the cost
  is paid once per invalid geometry, not per row).

## Where the upstream `geo` fix lives

The crashes that motivated fix #2 are in the `geo` crate (v0.31), not this
extension:

- **DE-9IM relate / geomgraph** — `geo-0.31.0/src/algorithm/relate/`:
  - `relate_operation.rs` + `geomgraph/` (`edge_end_bundle.rs`,
    `edge_end_bundle_star.rs`, `node.rs`, `geometry_graph.rs`) build a planar
    subdivision graph via recursion; on a self-intersecting or pathologically
    complex polygon this overflows the default ~8 MB worker stack.
  - This path backs `Relate`, hence `ST_Equals/Touches/Crosses/Overlaps/Covers/
    CoverededBy` and the non-point branches of `ST_Within/Contains`.
  - Fix direction: make the geomgraph traversal iterative / bounded, or cap
    recursion depth and fall back. Upstream issue to file against
    `georust/geo`.
- **Point-in-polygon** — `geo-0.31.0/src/algorithm/contains/polygon.rs`
  (`Contains<Point> for Polygon`) delegates to the winding/ray logic in
  `winding_order` / kernels; on the 133k-vertex Overture polygon it also
  mis-behaves. Fix #2 sidesteps it entirely with our own PNPOLY, but the
  upstream impl should be hardened too.

These map onto the brief's dependency table: until `geo` is fixed upstream, a
`cargo update` would not remove the need for fixes #1–#2; afterwards, the
guards here remain as cheap belt-and-suspenders.

## Files

| File | Purpose |
|------|---------|
| `setup_lake.sql` | (re)create the DuckLake + ingest SpatialBench parquet |
| `spatialbench_lake.sql` | safe subset of queries (scans, no joins) |
| `spatialbench_full.sql` | full query set incl. bbox-prefiltered joins |
| `run_queries.sh` | per-query timing (one process each) |
| `run.sh` | end-to-end: build → package → generate/cache data → lake → queries |
| `bridge.sql` | local `st_*` vs literal SedonaDB `sedona_*` overhead (1M points) |

## Literal SedonaDB bridge overhead (`bridge.sql`)

Compares the local `st_*` reimplementation against the literal Apache SedonaDB
kernel (`sedona_*`) over 1,000,000 points, wall-clock seconds (DuckDB
1.5.4, `.timer on`). Both paths share the same vectorized DuckDB chunking; the
delta is the DuckDB-chunk ⇄ Arrow bridge cost (per-chunk array build +
`invoke_with_args` + write-back).

| Operation | local `st_*` (s) | literal `sedona_*` (s) |
|-----------|------------------|------------------------|
| `ST_Dimension`     | 0.197 | 0.188 |
| `ST_XMin`          | 0.208 | 0.258 |
| `ST_AsText`        | 0.355 | 0.324 |
| `ST_Segmentize`    | 0.491 | 0.436 |

**Finding: the bridge overhead is negligible** — within run-to-run noise, and
the literal SedonaDB path is competitive with (often faster than) the local
reimplementation on the heavier kernels (`ST_AsText`, `ST_Segmentize`). No
allocation-tuning is warranted: the per-chunk Arrow array build is amortized
across DuckDB's standard 2048-row chunks, and SedonaDB's own WKB iteration is
already vectorized. Reproduce with:
`LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < benchmarks/bridge.sql`.
