# SpatialBench benchmarks (local DuckLake)

Runs the [Apache SpatialBench](https://github.com/apache/sedona-spatialbench)
queries against the **sedonadb** extension over a **local DuckLake** (DuckDB file
as catalog, local folder for Parquet data).

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

1. **`ST_MakeValid(geom)` + an internal `ensure_valid` guard.** Every
   relate-based predicate (`ST_Within/Contains/Covers/CoveredBy/Equals/Touches/
   Crosses/Overlaps`) and every boolean op (`ST_Intersection/Union/Difference/
   SymDifference`) now validates its inputs and, if invalid, repairs them with
   `buffer(0)` (even-odd topology rebuild) before calling into `geo`. Valid
   inputs take the cheap fast path (`is_valid` + borrow, no copy). This fixes
   the broad class of *invalid-polygon* errors across the whole catalog.
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

- **`ORDER BY ... LIMIT` on a geometry column that then feeds a scalar function
  segfaults.** An ordered/limited subquery yields a non-flat
  (sequence/dictionary) vector. The DuckDB **C API exposes no vector-encoding
  inspection** (only `duckdb_vector_get_data` / `get_validity` /
  `get_column_type`), so `BlobCol` cannot portably decode it and reads garbage
  → SIGSEGV. (This equally affects `quack-rs`'s own `VectorReader`.) Q4's
  canonical `ORDER BY t_tip DESC LIMIT 1000` form hits this; the benchmark uses
  the filter-equivalent (`WHERE t_tip > 40 LIMIT 1000`), which materializes a
  flat vector. **Workaround for users:** materialize the ordered geometry set
  into a temp/CTE table first, or filter instead of order. **Real fix:** the
  DuckDB C API would need to expose vector types (or a "flatten/fetch row"
  helper); tracked as the main portability gap.
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
