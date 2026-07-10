# Benchmarks

## QuackGIS LayoutBench

LayoutBench is the synthetic validation suite for DuckLake spatial-temporal
layout and the maintained high-QPS reader gates. It is deterministic by seed and scale
factor so CI, local dev, nightly, and manual stress runs exercise the same
distributions at different sizes.

Benchmark priorities follow the project direction:

1. selective large-table spatial query throughput and pruning;
2. high-QPS parallel pgwire readers over one shared DuckLake catalog/data prefix;
3. DuckDB-style OLAP fanout queries over columnar spatial data: grouped stats,
   primitive calculations, projection/filter/aggregate pushdown, and candidate
   narrowing before exact SedonaDB predicates;
4. COPY/parallel ingest behavior and DuckLake snapshot conflict semantics;
5. compaction effectiveness for fragmented append layouts;
6. compatibility smoke stability for GIS tools.

`sf0` now exists as a Rust integration oracle:

```sh
just layoutbench-sf0
```

For local, pgwire-level smoke/benchmark runs against an already-running server:

```sh
just server
just layoutbench-local sf0 3 generated insert
just layoutbench-local local-r22k8 5 generated copy
just layoutbench-local local-r22k8 3 shuffled insert true   # before/after compaction
just layoutbench-local-smoke
just benchmark-profile-check                       # exact 100M contract, no load
just kind-layoutbench-catalog-measure             # preseeded exact 100M catalog-call gate
python3 scripts/layoutbench_catalog_report.py \
  --profile benchmarks/profiles/layoutbench-regional-r100m-v1.json \
  --log .tmp/layoutbench-regional/catalog.log \
  --out .tmp/layoutbench-regional/metrics.json
```

The local runner can vary ingest order (`generated`, `shuffled`, `layout`), load
method (`insert`, `copy`), transaction grouping, and query variants. It seeds
`public.layoutbench_local_*` tables, prints row counts,
`total/base/candidate/exact` pruning metrics, repeated-query timings, and
`EXPLAIN ANALYZE` scan metrics (`bytes_scanned`, row-group/file-range pruning,
Parquet pushdown rows, and whether the hidden `_qg_*` predicate reached the
physical plan). The smoke recipe starts a temporary server and runs the sf0
runner once.

PostgreSQL/shared-catalog reads expose the process counter
`quackgis_catalog_read_provider_calls_total`. One increment is one delegated
`MetadataProvider` method, including an error. SQLite, writes, pool setup, pgwire,
and object-store IO are unmetered. This boundary does not count physical PostgreSQL
network roundtrips because SQLx connection and statement-cache behavior is below
it. Take before/after samples from the exact process or pod; a load-balanced or
aggregated scrape cannot be used for query deltas. Catalog refreshes remain a
distinct counter.

Each `layoutbench_catalog` phase record includes the serving process/pod id and
raw counter start/end. The report rejects resets, mismatched deltas, impossible
zero-call phases, invalid profile composition, and missing numeric run identity or
RFC3339 start time before replacing an evidence artifact.

The warm selective execution oracle is 3 schema-preflight calls plus 4 calls to
build a fresh scan plan. Extended execution deliberately replans instead of using
the parse-time plan so prepared statements observe new snapshots. Unit tests pin
the 2-call schema-only table lookup and wrapper arithmetic. The report parser
enforces the committed warm `7/query`, `1680/240`, cold `<=13`, direct `4`, and
refresh `0` provider-call budgets. Exact phase deltas are not claimed locally
because SQLite is intentionally unmetered; they remain a Kind PostgreSQL
measurement. `just kind-layoutbench-catalog-measure` now owns the bounded Kind
measurement path against a preseeded exact profile: it scales `lake` to one pod,
enables the metrics endpoint, runs cold/direct/warm phases through one process,
writes `.tmp/layoutbench-regional/catalog.log`, and renders the budgeted
`.tmp/compatibility/metrics.json`. Use `just kind-layoutbench-catalog-seed` only
with `LAYOUTBENCH_ALLOW_EXACT_R100M=true`, enough disk/time, and the committed
profile; the actual 100M execution evidence remains open until those artifacts are
published for a source SHA.

For Alpha shared-storage evidence, use the Kind lake profile instead of the
SQLite/local profile:

```sh
just kind-write-smoke   # parallel ingest plus DuckLake snapshot conflict/retry
just kind-qps-smoke     # high-QPS selective spatial reads over PostgreSQL/S3
just kind-olap-smoke    # grouped OLAP fanout, pushdown/pruning, exact recheck
```

The writer gate now prints `write_conflict conflict_observed=True` before
`write_ok True`, proving a stale transactional replace fails closed and a fresh
retry preserves the concurrent row.

The QPS and OLAP gates now fail if selective scans exceed configured
bytes-scanned ceilings (`QPS_MAX_BYTES_SCANNED`, `OLAP_MAX_BYTES_SCANNED`), and
QPS also enforces a file-group ceiling (`QPS_MAX_FILE_GROUPS`). That turns the
printed pruning metrics into enforced regression budgets.

### Current local-r22k8 iteration notes (2026-07-08)

The `layoutbench-local-r22k8-v1` profile is deliberately moderate (`factor=100`:
10,800 aerial rows,
9,600 CAD rows, 2,400 asset rows) so developer laptops can iterate quickly. It
is not the regional scale contract; it is a fast lever-finding loop.

Run shape:

```sh
# Fresh temp catalog per case; default row-group cap is 512 rows.
cargo run -p quackgis-server --example layoutbench -- \
  --scale local-r22k8 --query-iters 3 --load-method copy \
  --ingest-order generated --compare-variants
```

Key measurements from the legacy `.tmp/layoutbench-sf1/current` artifact (the
active profile name is `layoutbench-local-r22k8-v1`):

| Case | Seed time | Aerial avg | CAD avg | Assets avg | Row-group pruning |
|---|---:|---:|---:|---:|---|
| `insert`, generated, autocommit | 16.05 s | 62.0 ms | 60.4 ms | 37.7 ms | good: 22/1, 20/1, 5/1 |
| `insert`, shuffled, autocommit | 16.02 s | 89.9 ms | 88.0 ms | 43.7 ms | poor: 22/18, 20/19, 5/5 |
| `insert`, generated, transaction | 13.99 s | 37.8 ms | 33.9 ms | 31.7 ms | good, one file/range per table |
| `insert`, shuffled, transaction | 14.10 s | 38.7 ms | 33.8 ms | 30.6 ms | good, shuffled order neutralized |
| `copy`, generated, autocommit | 0.90 s | 35.4 ms | 33.1 ms | 31.5 ms | good: 22/1, 19/1, 5/1 |
| `copy`, shuffled, autocommit | 0.91 s | 38.5 ms | 35.3 ms | 32.1 ms | good, shuffled order neutralized |
| `copy`, generated, transaction | 0.88 s | 35.6 ms | 34.4 ms | 30.4 ms | good, one file/range per table |
| `insert`, shuffled, compacted | +0.52 s compact | 33.4 ms | 32.8 ms | 29.5 ms | repaired: 22/1, 19/1, 5/1; one file/range |

Interpretation:

1. **Load protocol dominates ingest.** COPY is about 18× faster than batched
   INSERT VALUES at this scale (≈0.9 s vs ≈16 s). OGR/GDAL `PG_USE_COPY=YES` is
   therefore a core architecture path, not an optional optimization.
2. **Write grouping dominates scan stability.** Autocommit INSERT writes many
   small append snapshots/files; transaction grouping or COPY produces one or two
   files/ranges per table and faster query times.
3. **Sort granularity matters.** Sorting each small INSERT batch helps only when
   client order is already layout-local. Shuffled autocommit INSERT destroys
   row-group locality because each append is sorted in isolation. Bulk COPY or
   transaction-staged writes sort the whole table/table delta and make client row
   order mostly irrelevant.
4. **Current pruning is row-group driven.** File/range pruning is not selective in
   these runs (`files_ranges` usually all matched). DuckLake/DataFusion row-group
   stats are the effective skip surface today; future compaction/file-level layout
   work should target file/range pruning.
5. **Exact predicates remain cheap enough after bbox pruning.** The `bbox_only`
   variants are slightly faster, but `bbox_exact` preserves correctness with
   modest overhead. Hidden bbox predicates are essential for CAD/assets: CAD
   `internal_exact` scanned every row group (`19/19`) and ~996 bytes vs bbox+exact
   scanning one row group and ~74 bytes.

Row-group sweep using COPY/generated:

| `QUACKGIS_DUCKLAKE_ROW_GROUP_ROWS` | Aerial avg / groups | CAD avg / groups | Assets avg / groups | Notes |
|---:|---|---|---|---|
| 128 | 47.7 ms / 85→1 | 45.6 ms / 75→1 | 32.7 ms / 19→1 | too many row groups; metadata overhead dominates |
| 256 | 41.3 ms / 43→1 | 42.2 ms / 38→1 | 34.9 ms / 10→1 | better, still more overhead than 512 |
| 512 (default) | 35.8 ms / 22→1 | 37.5 ms / 19→1 | 31.9 ms / 5→1 | best current local balance |
| 1024 | 36.4 ms / 11→1 | 35.7 ms / 10→1 | 34.2 ms / 3→1 | close to 512; scans more bytes |
| 2048 | 33.3 ms / 6→1 | 35.4 ms / 5→1 | 34.7 ms / 2→1 | fewer groups, weaker fine pruning |
| disabled (`0`) | 42.7 ms / 1→1 | 53.0 ms / 1→1 | 35.1 ms / 1→1 | no row-group pruning; poor CAD shape |

Architecture decisions from this pass:

- Keep the default local row-group cap at 512 rows until larger/nightly data says
  otherwise. It gives stable pruning without excessive row-group overhead.
- Prefer COPY for bulk ingest and document INSERT VALUES as a compatibility path.
- Keep sorting by hidden layout keys in the DuckLake write path, but treat it as a
  bulk-write/compaction primitive: it must run over whole COPY batches,
  transaction-staged table deltas, or compaction units, not just isolated small
  appends.
- Subsequent gates implemented bucket-local compaction, Kind parallel-reader QPS,
  and grouped OLAP fanout. Those are now the baseline, not pending benchmark work.
- Execute the committed `layoutbench-regional-r100m-v1` contract in Kind, then on
  managed services and copied real data; add measured metadata-scan, conflict,
  wire-roundtrip, and cost budgets beside its instrumented provider-call budget.

The first implemented compaction surface is explicit and table-scoped:

```sql
CALL quackgis_compact_table('public.layoutbench_local_aerial_frames');
```

It reads the DuckLake table, recomputes/projects hidden layout columns, sorts by
the layout key, and rewrites one replacement snapshot. On the shuffled INSERT
`layoutbench-local-r22k8-v1`
case it took about 0.52 s for all three benchmark tables and repaired the bad
layout:

| Label | Before compact | After compact |
|---|---|---|
| aerial | 88.3 ms, row groups 22/18/4, files 23/23/0 | 33.4 ms, row groups 22/1/21, files 1/1/0 |
| CAD | 84.2 ms, row groups 20/19/1, files 21/21/0 | 32.8 ms, row groups 19/1/18, files 1/1/0 |
| assets | 45.0 ms, row groups 5/5/0, files 6/6/0 | 29.5 ms, row groups 5/1/4, files 1/1/0 |

This validates the compaction lever. Whole-table compaction remains available;
bucket-scoped compaction now uses the native delete+pending-append mutation path
when row-lineage planning succeeds. The next benchmark step is to compare
file-group/bytes-scanned improvements on fragmented per-bucket append workloads.

Current pinned `sf0` counts:

```text
layoutbench_sf0 aerial=18 cad=12 assets=18 control=7
layoutbench_sf0_pruning aerial=108/30/18/18 cad=96/24/12/12 assets=24/20/18/18 false_positive=3/3/2/1
```

The test creates the planned table families, verifies `_qg_*` layout projection,
checks bbox-prefiltered query shapes return the same counts as exact SedonaDB
predicates, and pins the `total/base/candidate/exact` row counts for the current
sf0 pruning windows. The false-positive case proves bbox pruning can over-select
while exact SedonaDB evaluation still returns the correct result. Larger scales
remain planned generator/benchmark work.

| Scale | Purpose |
|---|---|
| `sf0` | CI oracle: implemented; small enough to compare prefiltered results against exact SedonaDB predicates |
| `layoutbench-local-r22k8-v1` (runner argument `local-r22k8`) | current fast local trend run: exactly 22,800 mixed rows |
| future `layoutbench-city-r10m-*` | city-scale client/pruning/compaction gate: exact profile required |
| `layoutbench-regional-r100m-v1` | defined routine regional contract: exactly 100M rows; execution pending |
| future `layoutbench-stress-r1b-*` | scheduled regional/national stress: exact profile required |
| future `layoutbench-inventory-b10tb-*` | manual object/catalog stress: exact profile required |

Synthetic tables:

- `layoutbench_aerial_frames`: overlapping drone/aerial photo footprints along
  flight strips with capture time, camera, GSD, altitude, mission id, and CRS
  metadata.
- `layoutbench_cad_objects`: local-coordinate architectural/site features with
  floor/level, object type, source id, Z range, transform/tolerance sidecars, and
  millimetre-scale detail near large project-grid offsets.
- `layoutbench_assets`: footprint rows for COPC/LAZ/E57 point clouds,
  COG/GeoTIFF rasters, 3D Tiles/glTF meshes, and IFC/CityGML/LandXML/DXF-derived
  layers.
- `layoutbench_control_points`: multi-epoch survey/control points with known
  synthetic drift, vertical datum metadata, and expected residual thresholds.

Gate queries:

1. tile/time aerial window;
2. mission strip crossing many spatial buckets;
3. CAD viewport by floor/level in local coordinates;
4. asset discovery by footprint/time/resolution/accuracy;
5. coordinate-drift residual check;
6. `sf0` exact-vs-pruned equality oracle;
7. OLAP fanout: grouped spatial/attribute stats + calculated filters + exact
   recheck;
8. append-small-files → compact-by-bucket → unchanged results + improved skip
   ratio.

Record: ingest rows/sec, file/row-group sizes, DuckLake metadata rows, max open
partitions per writer, partition/file/row-group skip ratios, bytes scanned,
exact-predicate candidate false-positive ratio, query time, compaction time, and
coordinate residual error.

See `docs/DUCKLAKE_SPATIAL_LAYOUT.md` for the type/fidelity model and layout
details.

## Historical SpatialBench archive (retired v0.1 architecture)

The remainder of this file records the retired DuckDB-extension benchmark path.
It is historical comparison data, not a current QuackGIS release gate. Current
benchmark contracts live above and in `docs/ANALYTICS_BENCHMARKS.md`.

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
